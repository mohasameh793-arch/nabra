//! Updates from GitHub Releases. Every release is signed with the maintainer's private key; the public key in
//! tauri.conf.json makes the app refuse anything else. Nabra checks in the background and *offers* new
//! versions (pill + main window); nothing installs until the user clicks Update. Then it downloads with a
//! progress readout, installs, and the installer reopens the new version.

use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

use crate::dictation::Control;
use crate::App;

// latest.json is a tiny file on GitHub's CDN, so checking often costs nothing, and the pill offers each
// version only once.
const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Status {
    UpToDate { version: String },
    Available { version: String, notes: String },
}

fn idle(app: &AppHandle) -> bool {
    let state = app.state::<App>();
    !state.busy.load(Ordering::SeqCst) && state.meeting.lock().unwrap().is_none()
}

/// Asks GitHub for a newer signed release and remembers it. The pill offers each new version once.
pub async fn check(app: &AppHandle) -> Result<Status, String> {
    let current = app.package_info().version.to_string();
    let found = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Couldn't check for updates: {e}"))?;
    let Some(update) = found else { return Ok(Status::UpToDate { version: current }) };
    let (version, notes) = (update.version.clone(), update.body.clone().unwrap_or_default());
    let state = app.state::<App>();
    let is_new = state.update.lock().unwrap().as_ref().map_or(true, |u| u.version != version);
    *state.update.lock().unwrap() = Some(update);
    let _ = app.emit("update", json!({ "status": "available", "version": version, "notes": notes }));
    if is_new {
        state.tell(Control::UpdateAvailable(version.clone()));
    }
    Ok(Status::Available { version, notes })
}

/// The user clicked Update: download (showing progress), install. On Windows, install closes Nabra and the
/// installer reopens the new version.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let result = install_inner(app).await;
    if let Err(e) = &result {
        let _ = app.emit("update", json!({ "status": "failed", "message": e }));
        app.state::<App>().tell(Control::UpdateFailed(e.clone()));
    }
    result
}

async fn install_inner(app: &AppHandle) -> Result<(), String> {
    let pending = app.state::<App>().update.lock().unwrap().clone();
    let update = pending.ok_or("No update is waiting. Check for updates first.")?;
    if !idle(app) {
        return Err("Finish dictating or stop note taking first, then update.".into());
    }
    let version = update.version.clone();
    let tell = |label: String| app.state::<App>().tell(Control::Updating(label));
    let progress = |percent: u64| {
        let _ = app.emit("update", json!({ "status": "downloading", "version": version, "percent": percent }));
        tell(format!("Updating Nabra… {percent}%"));
    };
    progress(0);
    let (mut got, mut shown) = (0u64, 0u64);
    let bytes = update
        .download(
            |chunk, total| {
                got += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let percent = (got * 100 / total).min(100);
                    if percent != shown {
                        shown = percent;
                        progress(percent);
                    }
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Update download failed: {e}"))?;
    let _ = app.emit("update", json!({ "status": "installing", "version": version }));
    tell("Installing… Nabra will restart".into());
    sleep(Duration::from_millis(900)).await; // long enough to read before the window closes
    update.install(bytes).map_err(|e| format!("Update failed to install: {e}"))
}

async fn sleep(d: Duration) {
    // tauri's async runtime is tokio; avoid a direct tokio dependency by sleeping on a blocking thread.
    let _ = tauri::async_runtime::spawn_blocking(move || std::thread::sleep(d)).await;
}

/// Background loop: first check 20 s after start, then every 5 minutes (Settings → "Check for updates
/// automatically").
pub fn watch(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        sleep(FIRST_CHECK).await;
        loop {
            if app.state::<App>().settings.lock().unwrap().auto_update {
                if let Err(e) = check(&app).await {
                    eprintln!("updater: {e}"); // offline etc.: try again next round
                }
            }
            sleep(EVERY).await;
        }
    });
}
