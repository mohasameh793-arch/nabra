//! The floating capsule at the edge of the screen. It docks at the middle of the left or right edge or at the
//! bottom middle (drag it there), and stays on the monitor the mouse is on.
//! It must never take focus: dictated text has to land in the app the user was typing in.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

use crate::App;

/// What the pill shows. Serialized to the pill page as `{ "view": "...", ... }` (+ `"dock"`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "lowercase")]
pub enum View {
    Idle,
    /// Mouse over the capsule: mic + notes buttons with shortcut tooltips.
    Hover { notes_on: bool, talk_key: &'static str, notes_key: &'static str },
    Notes { seconds: u64 },
    /// The capsule with a live voice meter. `command`: Right Alt is held (an instruction, not text).
    Listening { seconds: f32, level: f32, hands_free: bool, command: bool },
    /// The capsule with a spinner: transcribing a dictation.
    Busy,
    Working { label: String },
    Result { text: String, detail: String },
    Problem { message: String },
    /// A calendar meeting is starting: offer to take notes.
    Meeting { title: String },
    /// A new version is ready: offer to update.
    Update { version: String },
}

impl View {
    /// Window size in logical pixels for a pill docked at a side edge; content hugs the docked edge.
    fn size(&self, dock: &str) -> (f64, f64) {
        let (w, h) = match self {
            View::Idle => (24.0, 64.0),
            View::Listening { .. } | View::Busy => (44.0, 92.0),
            View::Hover { .. } => return if dock == "bottom" { (300.0, 190.0) } else { (330.0, 210.0) },
            View::Notes { .. } => return (170.0, 64.0),
            View::Working { .. } => return (300.0, 64.0),
            View::Result { .. } | View::Problem { .. } | View::Meeting { .. } | View::Update { .. } => return (420.0, 84.0),
        };
        if dock == "bottom" { (h, w) } else { (w, h) } // the capsule lies down at the bottom
    }
}

const MARGIN: f64 = 4.0;
const BOTTOM_MARGIN: f64 = 10.0;

/// The size of the last view shown, so the window can move (monitor change, docking) without re-rendering.
static LAST: Mutex<Option<serde_json::Value>> = Mutex::new(None);
static SIZE: Mutex<(f64, f64)> = Mutex::new((24.0, 64.0));
static MONITOR: Mutex<Option<(i32, i32)>> = Mutex::new(None);
static DRAGGING: AtomicBool = AtomicBool::new(false);

fn hwnd(app: &AppHandle) -> Option<HWND> {
    app.get_webview_window("pill")?.hwnd().ok().map(|h| HWND(h.0 as _))
}

pub fn init(app: &AppHandle) {
    if let Some(h) = hwnd(app) {
        unsafe {
            let style = GetWindowLongPtrW(h, GWL_EXSTYLE) | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize;
            SetWindowLongPtrW(h, GWL_EXSTYLE, style);
            let _ = ShowWindow(h, SW_SHOWNOACTIVATE); // Tauri's show() would activate (steal focus)
        }
    }
    follow_mouse(app.clone());
}

fn dock(app: &AppHandle) -> String {
    app.state::<App>().settings.lock().unwrap().pill_dock.clone()
}

/// The monitor under the mouse (falls back to the primary one).
fn monitor(app: &AppHandle) -> Option<tauri::Monitor> {
    let at = app.cursor_position().ok();
    at.and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten()).or_else(|| app.primary_monitor().ok().flatten())
}

/// Moves the window to its dock on the mouse's monitor, at `(lw, lh)` logical size.
fn place(app: &AppHandle, lw: f64, lh: f64) {
    let (Some(w), Some(m)) = (app.get_webview_window("pill"), monitor(app)) else { return };
    let scale = m.scale_factor();
    let area = m.work_area(); // the screen minus the taskbar
    let (ax, ay, aw, ah) = (area.position.x, area.position.y, area.size.width as i32, area.size.height as i32);
    let (pw, ph) = ((lw * scale) as i32, (lh * scale) as i32);
    let (x, y) = match dock(app).as_str() {
        "left" => (ax + (MARGIN * scale) as i32, ay + (ah - ph) / 2),
        "bottom" => (ax + (aw - pw) / 2, ay + ah - ph - (BOTTOM_MARGIN * scale) as i32),
        _ => (ax + aw - pw - (MARGIN * scale) as i32, ay + (ah - ph) / 2),
    };
    let _ = w.set_size(LogicalSize::new(lw, lh));
    let _ = w.set_position(PhysicalPosition::new(x, y));
    *SIZE.lock().unwrap() = (lw, lh);
    *MONITOR.lock().unwrap() = Some((m.position().x, m.position().y));
}

fn payload(app: &AppHandle, view: &View) -> serde_json::Value {
    let mut v = serde_json::to_value(view).unwrap_or_default();
    v["dock"] = dock(app).into();
    v
}

/// Resize/reposition for `view` and render it.
pub fn show(app: &AppHandle, view: &View) {
    let (lw, lh) = view.size(&dock(app));
    if !DRAGGING.load(Ordering::SeqCst) {
        place(app, lw, lh);
    }
    let v = payload(app, view);
    *LAST.lock().unwrap() = Some(v.clone());
    let _ = app.emit_to("pill", "pill", v);
}

/// Re-render without moving the window (timer ticks, level meter).
pub fn show_live(app: &AppHandle, view: &View) {
    let _ = app.emit_to("pill", "pill", payload(app, view));
}

/// Like Wispr Flow, the pill goes to whichever screen the mouse is on.
fn follow_mouse(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(300));
        if DRAGGING.load(Ordering::SeqCst) {
            continue;
        }
        let Some(m) = monitor(&app) else { continue };
        let here = Some((m.position().x, m.position().y));
        if *MONITOR.lock().unwrap() != here {
            let (lw, lh) = *SIZE.lock().unwrap();
            place(&app, lw, lh);
        }
    });
}

/// Dragging: the pill follows the mouse; on release it snaps to the nearest of left-middle, bottom-middle and
/// right-middle on that screen, and remembers it.
pub fn drag(app: &AppHandle, on: bool) {
    if on {
        if DRAGGING.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = app.clone();
        std::thread::spawn(move || {
            while DRAGGING.load(Ordering::SeqCst) {
                if let (Some(w), Ok(p)) = (app.get_webview_window("pill"), app.cursor_position()) {
                    let size = w.outer_size().unwrap_or_default();
                    let _ = w.set_position(PhysicalPosition::new(
                        p.x as i32 - size.width as i32 / 2,
                        p.y as i32 - size.height as i32 / 2,
                    ));
                }
                std::thread::sleep(Duration::from_millis(12));
            }
        });
        return;
    }
    if !DRAGGING.swap(false, Ordering::SeqCst) {
        return;
    }
    let (Ok(p), Some(m)) = (app.cursor_position(), monitor(app)) else { return };
    let area = m.work_area();
    let (ax, ay, aw, ah) = (area.position.x as f64, area.position.y as f64, area.size.width as f64, area.size.height as f64);
    let spots = [("left", ax, ay + ah / 2.0), ("bottom", ax + aw / 2.0, ay + ah), ("right", ax + aw, ay + ah / 2.0)];
    let dist = |(x, y): (f64, f64)| (x - p.x).powi(2) + (y - p.y).powi(2);
    let nearest = spots.iter().min_by(|a, b| dist((a.1, a.2)).total_cmp(&dist((b.1, b.2)))).map(|s| s.0).unwrap_or("right");
    let state = app.state::<App>();
    {
        let mut s = state.settings.lock().unwrap();
        s.pill_dock = nearest.into();
        let _ = state.store.save_settings(&s);
    }
    // Same view, new dock: resize for the dock's orientation and re-render.
    let last = LAST.lock().unwrap().clone();
    if let Some(mut v) = last {
        v["dock"] = nearest.into();
        let (lw, lh) = size_of_payload(&v, nearest);
        place(app, lw, lh);
        let _ = app.emit_to("pill", "pill", v);
    }
}

/// Window size for an already-rendered payload (used after re-docking).
fn size_of_payload(v: &serde_json::Value, dock: &str) -> (f64, f64) {
    let view = match v["view"].as_str().unwrap_or("idle") {
        "hover" => View::Hover { notes_on: false, talk_key: "", notes_key: "" },
        "listening" => View::Listening { seconds: 0.0, level: 0.0, hands_free: false, command: false },
        "busy" => View::Busy,
        "notes" => View::Notes { seconds: 0 },
        "working" => View::Working { label: String::new() },
        "result" | "problem" | "meeting" | "update" => View::Problem { message: String::new() },
        _ => View::Idle,
    };
    view.size(dock)
}
