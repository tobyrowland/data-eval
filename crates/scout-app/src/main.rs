//! Working Minds desktop shell.
//!
//! The interface is bundled HTML/CSS/JS served from the app itself; a strict
//! content security policy blocks every remote origin. All assessment work
//! runs in `scout-core` on a background thread. The window can call only
//! the commands below: there is no HTTP, shell or general filesystem access
//! from the interface.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use scout_core::report::render_text;
use scout_core::{build_data_map, extend_scan, ScanOptions};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, DragDropEvent, Emitter, Manager, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct AppState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
    inventory: Mutex<Option<scout_core::Inventory>>,
    last: Mutex<Option<(serde_json::Value, String)>>,
}

/// What the Add screen shows for one selection. Only metadata is read here.
#[derive(Serialize)]
struct Selection {
    path: String,
    name: String,
    kind: &'static str,
    bytes: u64,
    files: u64,
    /// True when the folder was too large to total quickly.
    approximate: bool,
}

const ARCHIVE_EXTS: &[&str] = &["zip", "tar", "gz", "tgz", "7z", "rar"];

fn describe(path: &Path) -> Option<Selection> {
    let md = std::fs::symlink_metadata(path).ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    if md.is_dir() {
        let (mut bytes, mut files, mut approximate) = (0u64, 0u64, false);
        let mut stack = vec![path.to_path_buf()];
        'walk: while let Some(dir) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            for e in rd.flatten() {
                if files >= 50_000 {
                    approximate = true;
                    break 'walk;
                }
                let Ok(m) = e.metadata() else { continue };
                if m.is_dir() {
                    stack.push(e.path());
                } else {
                    files += 1;
                    bytes += m.len();
                }
            }
        }
        return Some(Selection {
            path: path.display().to_string(),
            name,
            kind: "folder",
            bytes,
            files,
            approximate,
        });
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let kind = if ARCHIVE_EXTS.contains(&ext.as_str()) {
        "archive"
    } else {
        "file"
    };
    Some(Selection {
        path: path.display().to_string(),
        name,
        kind,
        bytes: md.len(),
        files: 1,
        approximate: false,
    })
}

/// Paths given on the command line (for example from "Open with").
#[tauri::command]
fn initial_paths() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-') && Path::new(a).exists())
        .collect()
}

#[tauri::command]
fn describe_paths(paths: Vec<String>) -> Vec<Selection> {
    paths
        .iter()
        .filter_map(|p| describe(Path::new(p)))
        .collect()
}

#[tauri::command]
async fn pick_files(app: AppHandle) -> Vec<String> {
    app.dialog()
        .file()
        .set_title("Choose exports to assess")
        .blocking_pick_files()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .map(|p| p.display().to_string())
        .collect()
}

#[tauri::command]
async fn pick_folder(app: AppHandle) -> Vec<String> {
    app.dialog()
        .file()
        .set_title("Choose a folder to assess")
        .blocking_pick_folders()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|f| f.into_path().ok())
        .map(|p| p.display().to_string())
        .collect()
}

/// Runs a scan on a background thread, either fresh or extending the
/// current assessment, and keeps the result for later additions.
async fn run_scan(
    app: AppHandle,
    state: &AppState,
    paths: Vec<String>,
    extend: bool,
) -> Result<serde_json::Value, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let prev = if extend {
        state.inventory.lock().unwrap().take()
    } else {
        None
    };
    let handle = app.clone();
    let inputs: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let opts = ScanOptions {
            cancel: Some(cancel),
            ..ScanOptions::default()
        };
        let mut last = Instant::now() - Duration::from_secs(1);
        let inventory = extend_scan(prev, &inputs, opts, &mut |p| {
            if last.elapsed() >= Duration::from_millis(80) {
                let _ = handle.emit("scout://progress", p);
                last = Instant::now();
            }
        });
        let _ = handle.emit("scout://stage", "building");
        let map = build_data_map(&inventory);
        let text = render_text(&map);
        (
            inventory,
            serde_json::to_value(&map).map_err(|e| e.to_string()),
            text,
        )
    })
    .await
    .map_err(|e| e.to_string())?;
    let (inventory, json, text) = result;
    let json = json?;
    remember_session(&app, &inventory);
    *state.inventory.lock().unwrap() = Some(inventory);
    *state.last.lock().unwrap() = Some((json.clone(), text));
    *state.cancel.lock().unwrap() = None;
    Ok(json)
}

#[tauri::command]
async fn assess(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    run_scan(app, &state, paths, false).await
}

/// Adds more files or folders to the current assessment. Only the new
/// selections are read.
#[tauri::command]
async fn assess_more(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    if state.inventory.lock().unwrap().is_none() {
        return Err("There is no assessment to add to yet.".into());
    }
    run_scan(app, &state, paths, true).await
}

// ------------------------------------------------------------ last session
// Only the list of selected paths is remembered, in the app's private data
// folder on this computer, so the owner can reopen their map later. No file
// contents, counts or results are stored.

fn session_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("last-assessment.json"))
}

fn remember_session(app: &AppHandle, inv: &scout_core::Inventory) {
    let Some(f) = session_file(app) else { return };
    let paths: Vec<String> = inv
        .collections
        .iter()
        .map(|c| c.source.display().to_string())
        .collect();
    let saved = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Some(dir) = f.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(
        f,
        serde_json::json!({ "paths": paths, "saved_unix": saved }).to_string(),
    );
}

#[derive(Serialize)]
struct LastSession {
    paths: Vec<String>,
    missing: usize,
    saved_unix: u64,
}

#[tauri::command]
fn last_session(app: AppHandle) -> Option<LastSession> {
    let raw = std::fs::read(session_file(&app)?).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let all: Vec<String> = v["paths"]
        .as_array()?
        .iter()
        .filter_map(|p| p.as_str().map(String::from))
        .collect();
    let paths: Vec<String> = all
        .iter()
        .filter(|p| Path::new(p).exists())
        .cloned()
        .collect();
    if paths.is_empty() {
        return None;
    }
    Some(LastSession {
        missing: all.len() - paths.len(),
        paths,
        saved_unix: v["saved_unix"].as_u64().unwrap_or(0),
    })
}

#[tauri::command]
fn forget_session(app: AppHandle, state: State<'_, AppState>) {
    if let Some(f) = session_file(&app) {
        let _ = std::fs::remove_file(f);
    }
    *state.inventory.lock().unwrap() = None;
    *state.last.lock().unwrap() = None;
}

#[tauri::command]
fn stop(state: State<'_, AppState>) {
    if let Some(c) = state.cancel.lock().unwrap().as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

/// Saves the latest data map into Documents/Working Minds and returns the
/// saved text file's path.
#[tauri::command]
fn save_map(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let guard = state.last.lock().unwrap();
    let (json, text) = guard.as_ref().ok_or("There is no data map to save yet.")?;
    let base = match app.path().document_dir() {
        Ok(d) => d,
        Err(_) => {
            let home = app.path().home_dir().map_err(|e| e.to_string())?;
            let docs = home.join("Documents");
            if docs.is_dir() {
                docs
            } else {
                home
            }
        }
    };
    let dir = base.join("Working Minds");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let date = json["generated_unix"]
        .as_i64()
        .map(ymd)
        .unwrap_or_else(|| "today".into());
    let stem = format!("Your-Data-Map-{date}");
    let txt = dir.join(format!("{stem}.txt"));
    std::fs::write(&txt, text).map_err(|e| e.to_string())?;
    let pretty = serde_json::to_string_pretty(json).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{stem}.json")), pretty).map_err(|e| e.to_string())?;
    Ok(txt.display().to_string())
}

/// Where the companion Attachment Downloader is published. Opening it hands
/// the address to the user's own browser; this app makes no request itself.
const DOWNLOAD_PAGE: &str = "https://github.com/tobyrowland/data-eval/releases/tag/v0.1.0-preview";

#[tauri::command]
fn open_download_page() -> Result<(), String> {
    tauri_plugin_opener::open_url(DOWNLOAD_PAGE, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
}

fn ymd(secs: i64) -> String {
    let (y, m, d) = scout_core::analyze::slack::civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState::default())
        .on_window_event(|window, event| {
            if let WindowEvent::DragDrop(dd) = event {
                let _ = match dd {
                    DragDropEvent::Enter { .. } | DragDropEvent::Over { .. } => {
                        window.emit("scout://drag", "over")
                    }
                    DragDropEvent::Leave => window.emit("scout://drag", "leave"),
                    DragDropEvent::Drop { paths, .. } => {
                        let paths: Vec<String> =
                            paths.iter().map(|p| p.display().to_string()).collect();
                        window.emit("scout://drop", paths)
                    }
                    _ => Ok(()),
                };
            }
        })
        .invoke_handler(tauri::generate_handler![
            initial_paths,
            describe_paths,
            pick_files,
            pick_folder,
            assess,
            assess_more,
            last_session,
            forget_session,
            stop,
            save_map,
            reveal,
            open_download_page
        ])
        .run(tauri::generate_context!())
        .expect("Working Minds failed to start");
}
