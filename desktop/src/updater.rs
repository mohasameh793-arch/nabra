//! Automatic updates from GitHub Releases. Every release is signed with the maintainer's private key; the
//! public key in tauri.conf.json makes the app refuse anything else. The update downloads in the background
//! and installs only when you're not dictating or taking notes; the installer reopens Nabra afterwards.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::App;

const FIRST_CHECK: Duration = Duration::from_secs(60);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    UpToDate { version: String },
    Installing { version: String },
}

fn idle(app: &AppHandle) -> bool {
    let state = app.state::<App>();
    !state.busy.load(Ordering::SeqCst) && state.meeting.lock().unwrap().is_none()
}

/// Checks GitHub; if there's a newer signed release, downloads it, waits until Nabra is idle, installs it.
/// On Windows the install step closes Nabra and the installer reopens the new version.
pub async fn run_once(app: &AppHandle) -> Result<Status, String> {
    let current = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;
    let Some(update) = update else { return Ok(Status::UpToDate { version: current }) };
    let version = update.version.clone();
    let _ = app.emit("update", serde_json::json!({ "status": "downloading", "version": version }));
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| format!("Update download failed: {e}"))?;
    while !idle(app) {
        tokio_sleep(Duration::from_secs(20)).await; // never interrupt a dictation or a meeting
    }
    let _ = app.emit("update", serde_json::json!({ "status": "installing", "version": version }));
    update.install(bytes).map_err(|e| format!("Update failed to install: {e}"))?;
    Ok(Status::Installing { version })
}

async fn tokio_sleep(d: Duration) {
    // tauri's async runtime is tokio; avoid a direct tokio dependency by sleeping on a blocking thread.
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

/// Background loop: first check a minute after start, then every 6 hours, if automatic updates are on.
pub fn watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio_sleep(FIRST_CHECK).await;
        loop {
            let enabled = app.state::<App>().settings.lock().unwrap().auto_update;
            if enabled {
                if let Err(e) = run_once(&app).await {
                    eprintln!("updater: {e}"); // offline etc.: try again next round
                }
            }
            tokio_sleep(EVERY).await;
        }
    });
}
