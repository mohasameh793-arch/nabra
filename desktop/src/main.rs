#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod autostart;
mod calendar;
mod commands;
mod dictation;
mod keyboard;
mod meeting;
mod pill;
mod secrets;
mod sidecar;
mod sound;
mod store;
mod updater;

use std::collections::HashSet;
use std::sync::mpsc::{channel, Sender};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;

use serde_json::json;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};

use dictation::{Control, Controller};
use meeting::Meeting;
use store::{Settings, Store};

/// Shared app state.
pub struct App {
    pub store: Store,
    pub settings: Mutex<Settings>,
    pub meeting: Mutex<Option<Meeting>>,
    pub engine: Mutex<Option<sidecar::Health>>,
    pub last_text: Mutex<String>,
    pub control: Mutex<Sender<Control>>,
    /// Upcoming events from the user's iCal link (empty if none connected).
    pub calendar: Mutex<Vec<calendar::Event>>,
    /// (title, when offered) of the meeting the pill just offered notes for.
    pub pending_title: Mutex<Option<(String, u64)>>,
    /// A dictation or voice command is in progress (updates wait for it).
    pub busy: AtomicBool,
    /// Set after an automatic update, to tell the user once.
    pub updated_to: Mutex<Option<String>>,
}

impl App {
    pub fn tell(&self, c: Control) {
        let _ = self.control.lock().unwrap().send(c);
    }
}

pub fn open_hub(app: &AppHandle, page: Option<&str>) {
    if let Some(w) = app.get_webview_window("hub") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
    if let Some(page) = page {
        let _ = app.emit_to("hub", "goto", page);
    }
}

/// The meeting window: about a quarter of the screen, on the right, above other windows.
pub fn show_meeting_window(app: &AppHandle) {
    let Some(w) = app.get_webview_window("meeting") else { return };
    if let Ok(Some(m)) = w.primary_monitor() {
        let (scale, size, pos) = (m.scale_factor(), m.size(), m.position());
        let width = (size.width as f64 / scale * 0.26).clamp(380.0, 520.0);
        let height = (size.height as f64 / scale * 0.62).clamp(460.0, 760.0);
        let _ = w.set_size(tauri::LogicalSize::new(width, height));
        let x = pos.x + size.width as i32 - ((width + 64.0) * scale) as i32; // leave room for the pill
        let y = pos.y + (size.height as i32 - (height * scale) as i32) / 2;
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = w.show();
    let _ = w.unminimize();
    let _ = w.set_focus();
}

/// Start or stop call notes (pill button, Ctrl+Alt+N, or the hub).
pub fn toggle_meeting(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<App>();
    let running = state.meeting.lock().unwrap().take();
    if let Some(m) = running {
        let _ = app.emit("meeting", json!({ "active": false, "id": m.id }));
        let app = app.clone();
        // Finishing waits for the last phrases to be transcribed; don't block the caller.
        std::thread::spawn(move || match m.finish(&app) {
            Ok(note) => {
                let _ = app.emit("note-saved", &note.id);
            }
            Err(e) => {
                let _ = app.emit("notes-problem", e);
            }
        });
    } else {
        let (consent, langs, mic) = {
            let s = state.settings.lock().unwrap();
            (s.notes_consent, s.langs(), s.microphone.clone())
        };
        if !consent {
            // First time: the hub explains that others should know they're being transcribed.
            open_hub(app, Some("notetaker"));
            let _ = app.emit_to("hub", "consent-needed", ());
            return Ok(());
        }
        // Use the offered meeting's name only if notes start within 15 minutes of the prompt.
        let title = state.pending_title.lock().unwrap().take()
            .filter(|(_, at)| store::now_ms().saturating_sub(*at) < 15 * 60_000)
            .map(|(t, _)| t);
        let m = Meeting::start(app.clone(), langs, mic, title.clone())?;
        let _ = app.emit("meeting", json!({ "active": true, "id": m.id, "elapsed": 0, "title": title }));
        *state.meeting.lock().unwrap() = Some(m);
        show_meeting_window(app);
    }
    state.tell(Control::Refresh);
    Ok(())
}

pub const CALENDAR_SECRET: &str = "calendar-ics";

/// Downloads the user's calendar (if connected) into App.calendar and tells the hub.
pub fn refresh_calendar(app: &AppHandle) -> Result<usize, String> {
    let Some(url) = secrets::get(CALENDAR_SECRET) else {
        app.state::<App>().calendar.lock().unwrap().clear();
        return Ok(0);
    };
    let events = calendar::upcoming(&calendar::fetch(&url)?, 7);
    let n = events.len();
    *app.state::<App>().calendar.lock().unwrap() = events;
    let _ = app.emit("calendar", n);
    Ok(n)
}

/// Every 30 s: offer notes when a meeting starts. Re-downloads the calendar every 10 minutes.
fn watch_calendar(app: AppHandle) {
    let mut prompted: HashSet<(String, i64)> = HashSet::new();
    let mut tick = 0u32;
    loop {
        if tick % 20 == 0 {
            if let Err(e) = refresh_calendar(&app) {
                eprintln!("calendar: {e}");
            }
        }
        tick += 1;
        let state = app.state::<App>();
        let wanted = state.settings.lock().unwrap().meeting_prompts && state.meeting.lock().unwrap().is_none();
        if wanted {
            let now = store::now_ms() as i64;
            let starting = state.calendar.lock().unwrap().iter()
                .find(|e| !e.all_day && e.start <= now + 60_000 && e.start >= now - 120_000 && !prompted.contains(&(e.title.clone(), e.start)))
                .cloned();
            if let Some(e) = starting {
                prompted.insert((e.title.clone(), e.start));
                *state.pending_title.lock().unwrap() = Some((e.title.clone(), store::now_ms()));
                state.tell(Control::MeetingStarting(e.title));
            }
        }
        std::thread::sleep(Duration::from_secs(30));
    }
}

/// True if another Nabra is already running for this user. Two copies would both listen to Right Ctrl and
/// type every dictation twice, so a second launch just exits.
fn already_running() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    unsafe {
        // Intentionally leaked: the mutex lives exactly as long as this process.
        let _mutex = CreateMutexW(None, true, w!("Local\\Nabra.SingleInstance"));
        GetLastError() == ERROR_ALREADY_EXISTS
    }
}

fn main() {
    if already_running() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(commands::handler())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "hub" || window.label() == "meeting" {
                    api.prevent_close(); // closing the hub keeps Nabra running in the tray
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
            let store = Store::new(app.path().app_data_dir()?);
            let mut settings = store.settings();
            // First run after an automatic update: remember to say so once.
            let version = app.package_info().version.to_string();
            let updated_to = (!settings.last_version.is_empty() && settings.last_version != version).then(|| version.clone());
            if settings.last_version != version {
                settings.last_version = version;
                let _ = store.save_settings(&settings);
            }
            let (tx, inbox) = channel();
            app.manage(App {
                store,
                settings: Mutex::new(settings),
                meeting: Mutex::new(None),
                engine: Mutex::new(None),
                last_text: Mutex::new(String::new()),
                control: Mutex::new(tx.clone()),
                calendar: Mutex::new(Vec::new()),
                pending_title: Mutex::new(None),
                busy: AtomicBool::new(false),
                updated_to: Mutex::new(updated_to),
            });
            updater::watch(app.handle().clone());
            let watcher = app.handle().clone();
            std::thread::spawn(move || watch_calendar(watcher));
            pill::init(app.handle());

            let (keys_tx, keys) = channel();
            keyboard::listen(keys_tx);
            let to_controller = tx.clone();
            std::thread::spawn(move || {
                for k in keys {
                    let _ = to_controller.send(Control::Key(k));
                }
            });

            let open = MenuItem::with_id(app, "open", "Open Nabra", true, None::<&str>)?;
            let copy = MenuItem::with_id(app, "copy", "Copy last dictation", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Nabra", true, None::<&str>)?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().cloned().expect("bundle icon"))
                .tooltip(format!("Nabra · hold {} to dictate", keyboard::TALK_KEY_LABEL))
                .menu(&Menu::with_items(app, &[&open, &copy, &quit])?)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => open_hub(app, None),
                    "copy" => {
                        let text = app.state::<App>().last_text.lock().unwrap().clone();
                        let _ = keyboard::copy(&text);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            let controller = Controller::new(app.handle().clone());
            std::thread::spawn(move || controller.run(inbox));
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Nabra failed to start");
}
