#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod assets;
mod attendees;
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
    /// A newer signed release found by the updater, waiting for the user to click Update.
    pub update: Mutex<Option<tauri_plugin_updater::Update>>,
    /// (version, when) the pill last offered an update, so a dismissed offer comes back a day later.
    pub offered: Mutex<Option<(String, std::time::Instant)>>,
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
    // Starting takes a moment (it opens two audio devices): without this, two quick toggles from the hotkey and
    // the window could both start a meeting, leaving one recording forever.
    static TOGGLING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = TOGGLING.lock().unwrap_or_else(|e| e.into_inner());
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
        // Who's invited: the calendar event happening now (started up to 15 min ago, or starting within 10).
        let now = store::now_ms() as i64;
        let event = state.calendar.lock().unwrap().iter()
            .find(|e| !e.all_day && e.start - 10 * 60_000 <= now && now <= e.end.max(e.start + 15 * 60_000))
            .cloned();
        let title = title.or_else(|| event.as_ref().map(|e| e.title.clone()));
        let attendees = event.map(|e| e.attendees).unwrap_or_default();
        let m = Meeting::start(app.clone(), langs, mic, title.clone(), attendees)?;
        let (id, scan_app) = (m.id.clone(), app.clone());
        // …and whoever the meeting app's participant list shows (Zoom / Teams / Meet). The window asks the user to
        // open that list; we keep looking for 10 minutes so names appear as soon as it's open, and newcomers too.
        std::thread::spawn(move || {
            for _ in 0..40 {
                let still_running = scan_app.state::<App>().meeting.lock().unwrap().as_ref().is_some_and(|m| m.id == id);
                if !still_running {
                    break;
                }
                meeting::add_attendees(&scan_app, &id, attendees::scan());
                std::thread::sleep(Duration::from_secs(15));
            }
        });
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
        let prompts = state.settings.lock().unwrap().meeting_prompts; // never hold two locks at once
        let wanted = prompts && state.meeting.lock().unwrap().is_none();
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

/// Appends a line to logs\nabra.log: startup steps and crashes, so "nothing happened" can be diagnosed.
/// (The engine and the local AI model write their own logs next to it.)
pub fn log(msg: impl AsRef<str>) {
    use std::io::Write;
    let dir = sidecar::logs_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("nabra.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
        let _ = std::fs::rename(&path, dir.join("nabra.old.log"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), msg.as_ref());
    }
}

/// `nabra.exe --notes` starts or stops call notes in the running Nabra (a shortcut, Stream Deck button or script).
/// The running copy waits on this named event; a second launch with --notes sets it and exits.
const NOTES_EVENT: windows::core::PCWSTR = windows::core::w!("Local\\Nabra.ToggleNotes");

fn signal_toggle_notes() {
    use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
    unsafe {
        match OpenEventW(EVENT_MODIFY_STATE, false, NOTES_EVENT) {
            Ok(event) => {
                let _ = SetEvent(event);
                let _ = windows::Win32::Foundation::CloseHandle(event);
            }
            Err(e) => log(format!("--notes: Nabra isn't listening ({e})")),
        }
    }
}

fn listen_for_toggle_notes(app: AppHandle) {
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject, INFINITE};
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    std::thread::spawn(move || unsafe {
        let Ok(event) = CreateEventW(None, false, false, NOTES_EVENT) else { return };
        while WaitForSingleObject(event, INFINITE) == WAIT_OBJECT_0 {
            log("--notes: toggling call notes");
            if let Err(e) = toggle_meeting(&app) {
                log(format!("--notes: {e}"));
            }
        }
    });
}

fn main() {
    std::panic::set_hook(Box::new(|info| log(format!("CRASH: {info}"))));
    if already_running() {
        if std::env::args().any(|a| a == "--notes") {
            signal_toggle_notes();
        } else {
            log("another Nabra is already running; this one exits");
        }
        return;
    }
    log(format!("Nabra {} starting", env!("CARGO_PKG_VERSION")));
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
                update: Mutex::new(None),
                offered: Mutex::new(None),
            });
            updater::watch(app.handle().clone());
            listen_for_toggle_notes(app.handle().clone());
            if std::env::args().any(|a| a == "--notes") {
                // Nabra wasn't open yet: start notes once the speech engine is up (it picks live mode on start).
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    for _ in 0..90 {
                        if app.state::<App>().engine.lock().unwrap().is_some() {
                            break;
                        }
                        std::thread::sleep(Duration::from_secs(1));
                    }
                    log("--notes: starting call notes");
                    if let Err(e) = toggle_meeting(&app) {
                        log(format!("--notes: {e}"));
                    }
                });
            }
            // Voice model for "who is speaking" in call notes (small; first run or after updating).
            std::thread::spawn(|| {
                if let Err(e) = assets::ensure_voice_model() {
                    eprintln!("voice model: {e}"); // offline: call notes still work, without speaker names
                }
            });
            let watcher = app.handle().clone();
            std::thread::spawn(move || watch_calendar(watcher));
            pill::init(app.handle());

            let (keys_tx, keys) = channel();
            let (talk_key, keep) = {
                let s = app.state::<App>().settings.lock().unwrap().clone();
                (s.talk_key, s.history_keep)
            };
            keyboard::set_talk_key(&talk_key);
            let _ = app.state::<App>().store.prune_history(&keep);
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
                .tooltip(format!("Nabra · hold {} to dictate", keyboard::talk_key_label()))
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
