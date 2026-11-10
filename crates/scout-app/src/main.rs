//! Working Minds desktop shell.
//!
//! The interface is bundled HTML/CSS/JS served from the app itself; a strict
//! content security policy blocks every remote origin. All assessment work
//! runs in `scout-core` on a background thread. The window can call only
//! the commands below: there is no HTTP, shell or general filesystem access
//! from the interface.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use scout_core::report::render_text;
use scout_core::{build_data_map, extend_scan, update_scan, ScanOptions};
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
    /// Items the owner removed from this assessment (paths), so they can be
    /// put back. Nothing on disk is touched.
    removed: Mutex<Vec<String>>,
    /// The latest training-value evaluation, if one was run.
    evaluation: Mutex<Option<serde_json::Value>>,
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
    exclude: Vec<String>,
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
            exclude,
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
    finish(&app, state, result)
}

/// Keeps a new assessment and its map, and remembers the session.
fn finish(
    app: &AppHandle,
    state: &AppState,
    result: (
        scout_core::Inventory,
        Result<serde_json::Value, String>,
        String,
    ),
) -> Result<serde_json::Value, String> {
    let (inventory, json, text) = result;
    *state.cancel.lock().unwrap() = None;
    let json = json?;
    // Items added back are no longer "removed".
    state.removed.lock().unwrap().retain(|r| {
        !inventory
            .collections
            .iter()
            .any(|c| c.source.display().to_string() == *r)
    });
    remember_session(app, &inventory, &state.removed.lock().unwrap());
    *state.inventory.lock().unwrap() = Some(inventory);
    // The data changed, so an earlier evaluation no longer applies.
    *state.evaluation.lock().unwrap() = None;
    *state.last.lock().unwrap() = Some((json.clone(), text));
    Ok(json)
}

/// Changes the current assessment on a background thread: drops `remove`,
/// reads `reread` again, with `exclude` as the new exclusion list.
async fn run_update(
    app: AppHandle,
    state: &AppState,
    remove: Vec<u32>,
    reread: Vec<u32>,
    exclude: Vec<String>,
) -> Result<serde_json::Value, String> {
    let prev = state
        .inventory
        .lock()
        .unwrap()
        .take()
        .ok_or("There is no assessment to change yet.")?;
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let opts = ScanOptions {
            cancel: Some(cancel),
            exclude,
            ..ScanOptions::default()
        };
        let mut last = Instant::now() - Duration::from_secs(1);
        let inventory = update_scan(prev, &remove, &reread, opts, &mut |p| {
            if last.elapsed() >= Duration::from_millis(80) {
                let _ = handle.emit("scout://progress", p);
                last = Instant::now();
            }
        });
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
    finish(&app, state, result)
}

fn collection_id(inv: &scout_core::Inventory, label: &str) -> Result<u32, String> {
    inv.collections
        .iter()
        .find(|c| c.label == label)
        .map(|c| c.id)
        .ok_or_else(|| format!("{label} is not in this assessment."))
}

/// Folders and files inside one item, for the owner's own screen only.
#[tauri::command]
fn list_files(
    state: State<'_, AppState>,
    label: String,
    prefix: Option<String>,
    service: Option<String>,
) -> Result<Vec<scout_core::browse::Entry>, String> {
    let guard = state.inventory.lock().unwrap();
    let inv = guard.as_ref().ok_or("There is no assessment yet.")?;
    let id = collection_id(inv, &label)?;
    Ok(scout_core::browse::children(
        inv,
        id,
        prefix.as_deref(),
        service.as_deref(),
    ))
}

/// Files in one item that link to a service, most links first.
#[tauri::command]
fn linking_files(
    state: State<'_, AppState>,
    label: String,
    service: String,
) -> Result<Vec<scout_core::browse::Entry>, String> {
    let guard = state.inventory.lock().unwrap();
    let inv = guard.as_ref().ok_or("There is no assessment yet.")?;
    let id = collection_id(inv, &label)?;
    Ok(scout_core::browse::linking_files(inv, id, &service, 500))
}

/// For every item: how many of its files link to each service.
#[tauri::command]
fn link_file_counts(
    state: State<'_, AppState>,
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<&'static str, u64>> {
    let guard = state.inventory.lock().unwrap();
    let Some(inv) = guard.as_ref() else {
        return Default::default();
    };
    inv.collections
        .iter()
        .map(|c| {
            (
                c.label.clone(),
                scout_core::browse::linking_file_counts(inv, c.id),
            )
        })
        .collect()
}

#[derive(Serialize)]
struct RemovedItem {
    path: String,
    name: String,
}

/// Items removed from this assessment that can be put back.
#[tauri::command]
fn removed_items(state: State<'_, AppState>) -> Vec<RemovedItem> {
    state
        .removed
        .lock()
        .unwrap()
        .iter()
        .filter(|p| Path::new(p).exists())
        .map(|p| RemovedItem {
            path: p.clone(),
            name: Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.clone()),
        })
        .collect()
}

/// Takes whole items out of the assessment (and so out of the data pack).
#[tauri::command]
async fn remove_items(
    app: AppHandle,
    state: State<'_, AppState>,
    labels: Vec<String>,
) -> Result<serde_json::Value, String> {
    let (ids, paths, exclude) = {
        let guard = state.inventory.lock().unwrap();
        let inv = guard.as_ref().ok_or("There is no assessment yet.")?;
        let mut ids = Vec::new();
        let mut paths = Vec::new();
        for l in &labels {
            let id = collection_id(inv, l)?;
            ids.push(id);
            paths.push(inv.collections[id as usize].source.display().to_string());
        }
        (ids, paths, inv.exclude.clone())
    };
    {
        let mut removed = state.removed.lock().unwrap();
        for p in paths {
            if !removed.contains(&p) {
                removed.push(p);
            }
        }
    }
    run_update(app, &state, ids, Vec::new(), exclude).await
}

/// Puts a removed item back. Only that item is read.
#[tauri::command]
async fn restore_item(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<serde_json::Value, String> {
    let exclude = state
        .inventory
        .lock()
        .unwrap()
        .as_ref()
        .map(|i| i.exclude.clone())
        .unwrap_or_default();
    run_scan(app, &state, vec![path], true, exclude).await
}

/// Leaves files or folders inside items out. Only the items they are in are
/// read again.
#[tauri::command]
async fn exclude_paths(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    let (reread, exclude) = {
        let guard = state.inventory.lock().unwrap();
        let inv = guard.as_ref().ok_or("There is no assessment yet.")?;
        let mut exclude = inv.exclude.clone();
        let mut reread = Vec::new();
        for p in &paths {
            let p = p.trim_matches('/').to_string();
            reread.extend(inv.collections_for_path(&p));
            if !exclude.contains(&p) {
                exclude.push(p);
            }
        }
        reread.sort_unstable();
        reread.dedup();
        (reread, exclude)
    };
    run_update(app, &state, Vec::new(), reread, exclude).await
}

/// Puts excluded files or folders back.
#[tauri::command]
async fn restore_paths(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<serde_json::Value, String> {
    let (reread, exclude) = {
        let guard = state.inventory.lock().unwrap();
        let inv = guard.as_ref().ok_or("There is no assessment yet.")?;
        let wanted: Vec<String> = paths
            .iter()
            .map(|p| p.trim_matches('/').to_string())
            .collect();
        let exclude: Vec<String> = inv
            .exclude
            .iter()
            .filter(|e| !wanted.contains(&e.trim_matches('/').to_string()))
            .cloned()
            .collect();
        let mut reread: Vec<u32> = wanted
            .iter()
            .flat_map(|p| inv.collections_for_path(p))
            .collect();
        reread.sort_unstable();
        reread.dedup();
        (reread, exclude)
    };
    run_update(app, &state, Vec::new(), reread, exclude).await
}

#[tauri::command]
async fn assess(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
    exclude: Option<Vec<String>>,
) -> Result<serde_json::Value, String> {
    // A fresh assessment starts with nothing removed, unless it reopens the
    // last session (which passes its exclusions).
    if exclude.is_none() {
        state.removed.lock().unwrap().clear();
    }
    run_scan(app, &state, paths, false, exclude.unwrap_or_default()).await
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
    let exclude = state
        .inventory
        .lock()
        .unwrap()
        .as_ref()
        .map(|i| i.exclude.clone())
        .unwrap_or_default();
    run_scan(app, &state, paths, true, exclude).await
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

fn remember_session(app: &AppHandle, inv: &scout_core::Inventory, removed: &[String]) {
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
        serde_json::json!({
            "paths": paths,
            "exclude": inv.exclude,
            "removed": removed,
            "saved_unix": saved,
        })
        .to_string(),
    );
}

#[derive(Serialize)]
struct LastSession {
    paths: Vec<String>,
    /// Files and folders the owner left out inside those items.
    exclude: Vec<String>,
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
    let exclude = v["exclude"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let removed: Vec<String> = v["removed"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    *app.state::<AppState>().removed.lock().unwrap() = removed;
    Some(LastSession {
        exclude,
        missing: all.len() - paths.len(),
        paths,
        saved_unix: v["saved_unix"].as_u64().unwrap_or(0),
    })
}

// ------------------------------------------------------------ data pack
// Copies the selections into one folder with standard names. Local only.

#[tauri::command]
fn package_plan(state: State<'_, AppState>) -> Result<scout_core::package::Plan, String> {
    let guard = state.inventory.lock().unwrap();
    let inv = guard.as_ref().ok_or("Assess some data first.")?;
    Ok(scout_core::package::plan(inv))
}

#[tauri::command]
async fn pick_package_dest(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Choose where to create the data pack")
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command]
async fn build_package(
    app: AppHandle,
    state: State<'_, AppState>,
    dest: String,
) -> Result<scout_core::package::Summary, String> {
    let inv = state
        .inventory
        .lock()
        .unwrap()
        .take()
        .ok_or("Assess some data first.")?;
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let handle = app.clone();
    let today = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| ymd(d.as_secs() as i64))
        .unwrap_or_default();
    let (inv, summary) = tauri::async_runtime::spawn_blocking(move || {
        let plan = scout_core::package::plan(&inv);
        let mut last = Instant::now() - Duration::from_secs(1);
        let summary = scout_core::package::build(
            &inv,
            &plan,
            Path::new(&dest),
            &format!("Working Minds data pack {today}"),
            Some(&cancel),
            &mut |p| {
                if last.elapsed() >= Duration::from_millis(100) {
                    let _ = handle.emit("scout://package", p);
                    last = Instant::now();
                }
            },
        );
        (inv, summary)
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.inventory.lock().unwrap() = Some(inv);
    *state.cancel.lock().unwrap() = None;
    Ok(summary)
}

// ------------------------------------------------------------ source list
// The owner's edits to the data-source list (sources added or removed).
// Stored next to the session file; service names and labels only.

fn plan_file(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join("source-plan.json"))
}

fn read_plan(app: &AppHandle) -> serde_json::Value {
    plan_file(app)
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({ "added": [], "removed": [] }))
}

#[tauri::command]
fn get_source_plan(app: AppHandle) -> serde_json::Value {
    read_plan(&app)
}

#[tauri::command]
fn set_source_plan(app: AppHandle, plan: serde_json::Value) -> Result<(), String> {
    // Keep only the expected shape: lists of short strings or small objects.
    let ok = plan.get("added").is_some_and(|v| v.is_array())
        && plan.get("removed").is_some_and(|v| v.is_array());
    let text = plan.to_string();
    if !ok || text.len() > 64 * 1024 {
        return Err("Unexpected source list.".into());
    }
    let f = plan_file(&app).ok_or("No app data folder.")?;
    if let Some(dir) = f.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(f, text).map_err(|e| e.to_string())
}

#[tauri::command]
fn forget_session(app: AppHandle, state: State<'_, AppState>) {
    if let Some(f) = session_file(&app) {
        let _ = std::fs::remove_file(f);
    }
    if let Some(f) = plan_file(&app) {
        let _ = std::fs::remove_file(f);
    }
    *state.inventory.lock().unwrap() = None;
    *state.last.lock().unwrap() = None;
    state.removed.lock().unwrap().clear();
}

/// Reads every item again in full and evaluates its value as training data.
/// Takes a while on large data; progress is sent as `scout://evaluate`.
#[tauri::command]
async fn evaluate(app: AppHandle, state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let (inputs, labels, exclude) = {
        let guard = state.inventory.lock().unwrap();
        let inv = guard
            .as_ref()
            .ok_or("There is no assessment to evaluate yet.")?;
        (
            inv.collections
                .iter()
                .map(|c| c.source.clone())
                .collect::<Vec<_>>(),
            inv.collections
                .iter()
                .map(|c| c.label.clone())
                .collect::<Vec<_>>(),
            inv.exclude.clone(),
        )
    };
    let cancel = Arc::new(AtomicBool::new(false));
    *state.cancel.lock().unwrap() = Some(cancel.clone());
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut last = Instant::now() - Duration::from_secs(1);
        let ev = scout_core::evaluate::evaluate_inputs(
            inputs,
            labels,
            exclude,
            Some(cancel),
            &mut |p| {
                if last.elapsed() >= Duration::from_millis(150) {
                    let _ = handle.emit("scout://evaluate", p);
                    last = Instant::now();
                }
            },
        );
        serde_json::to_value(&ev).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.cancel.lock().unwrap() = None;
    let json = result?;
    *state.evaluation.lock().unwrap() = Some(json.clone());
    Ok(json)
}

/// The latest evaluation, if one was run in this session.
#[tauri::command]
fn evaluation_result(state: State<'_, AppState>) -> Option<serde_json::Value> {
    state.evaluation.lock().unwrap().clone()
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
    // Include the owner's edits to the data-source list.
    let mut json = json.clone();
    json["source_plan"] = read_plan(&app);
    if let Some(ev) = state.evaluation.lock().unwrap().as_ref() {
        json["training_value"] = ev.clone();
    }
    let pretty = serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?;
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
            get_source_plan,
            set_source_plan,
            package_plan,
            pick_package_dest,
            build_package,
            stop,
            evaluate,
            evaluation_result,
            list_files,
            linking_files,
            link_file_counts,
            removed_items,
            remove_items,
            restore_item,
            exclude_paths,
            restore_paths,
            save_map,
            reveal,
            open_download_page
        ])
        .run(tauri::generate_context!())
        .expect("Working Minds failed to start");
}
