//! DataPack Scout Attachment Downloader: desktop shell.
//!
//! Companion to DataPack Scout. Unlike the assessment app, this tool makes
//! network requests: only to Slack's file hosts, only when the user presses
//! Download, and only for links already inside their own export.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use scout_fetch::{default_out_dir, download, plan_from_export, Options, Plan, Summary};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, DragDropEvent, Emitter, State, WindowEvent};
use tauri_plugin_dialog::DialogExt;

#[derive(Default)]
struct AppState {
    plan: Mutex<Option<Plan>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

#[derive(Serialize)]
struct Inspection {
    export_path: String,
    export_name: String,
    files: u64,
    images: u64,
    documents: u64,
    known_bytes: u64,
    skipped_hidden: u64,
    skipped_external: u64,
    skipped_unusable: u64,
    default_out: String,
}

#[tauri::command]
fn initial_paths() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with('-') && Path::new(a).exists())
        .collect()
}

#[tauri::command]
async fn inspect(state: State<'_, AppState>, path: String) -> Result<Inspection, String> {
    let p = PathBuf::from(&path);
    let plan = tauri::async_runtime::spawn_blocking(move || plan_from_export(&p))
        .await
        .map_err(|e| e.to_string())??;
    let out = default_out_dir(Path::new(&path));
    let info = Inspection {
        export_path: path,
        export_name: plan.export_name.clone(),
        files: plan.files.len() as u64,
        images: plan.images(),
        documents: plan.files.len() as u64 - plan.images(),
        known_bytes: plan.known_bytes(),
        skipped_hidden: plan.skipped_hidden,
        skipped_external: plan.skipped_external,
        skipped_unusable: plan.skipped_unusable,
        default_out: out.display().to_string(),
    };
    *state.plan.lock().unwrap() = Some(plan);
    Ok(info)
}

#[tauri::command]
async fn pick_export(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Choose your Slack export")
        .add_filter("Slack export", &["zip"])
        .blocking_pick_file()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command]
async fn pick_export_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Choose your unzipped Slack export folder")
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command]
async fn pick_output(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Choose where to save the attachments")
        .blocking_pick_folder()
        .and_then(|f| f.into_path().ok())
        .map(|p| p.display().to_string())
}

#[tauri::command]
async fn start(app: AppHandle, state: State<'_, AppState>, out: String) -> Result<Summary, String> {
    let plan = state
        .plan
        .lock()
        .unwrap()
        .clone()
        .ok_or("Choose a Slack export first.")?;
    let mut opts = Options::new(PathBuf::from(out));
    *state.cancel.lock().unwrap() = Some(opts.cancel.clone());
    opts.workers = 4;
    let handle = app.clone();
    let summary = tauri::async_runtime::spawn_blocking(move || {
        download(&plan, &opts, &mut |p| {
            let _ = handle.emit("fetch://progress", p);
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    *state.cancel.lock().unwrap() = None;
    Ok(summary)
}

#[tauri::command]
fn stop(state: State<'_, AppState>) {
    if let Some(c) = state.cancel.lock().unwrap().as_ref() {
        c.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn reveal(path: String) -> Result<(), String> {
    tauri_plugin_opener::reveal_item_in_dir(path).map_err(|e| e.to_string())
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
                        window.emit("fetch://drag", "over")
                    }
                    DragDropEvent::Leave => window.emit("fetch://drag", "leave"),
                    DragDropEvent::Drop { paths, .. } => {
                        let paths: Vec<String> =
                            paths.iter().map(|p| p.display().to_string()).collect();
                        window.emit("fetch://drop", paths)
                    }
                    _ => Ok(()),
                };
            }
        })
        .invoke_handler(tauri::generate_handler![
            initial_paths,
            inspect,
            pick_export,
            pick_export_folder,
            pick_output,
            start,
            stop,
            reveal
        ])
        .run(tauri::generate_context!())
        .expect("Attachment Downloader failed to start");
}
