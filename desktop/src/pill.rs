//! The floating capsule on the right edge of the screen.
//! It must never take focus: dictated text has to land in the app the user was typing in.

use serde::Serialize;
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, ShowWindow, GWL_EXSTYLE, SW_SHOWNOACTIVATE, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW,
};

/// What the pill shows. Serialized to the pill page as `{ "view": "...", ... }`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "lowercase")]
pub enum View {
    Idle,
    /// Mouse over the capsule: mic + notes buttons with shortcut tooltips.
    Hover { notes_on: bool, talk_key: &'static str, notes_key: &'static str },
    Notes { seconds: u64 },
    Listening { seconds: f32, level: f32, hands_free: bool },
    Working { label: String },
    Result { text: String, detail: String },
    Problem { message: String },
}

impl View {
    /// Window size in logical pixels; content is right-aligned inside it.
    fn size(&self) -> (f64, f64) {
        match self {
            View::Idle => (30.0, 92.0),
            View::Hover { .. } => (300.0, 150.0),
            View::Notes { .. } => (170.0, 64.0),
            View::Listening { .. } | View::Working { .. } => (300.0, 64.0),
            View::Result { .. } | View::Problem { .. } => (420.0, 84.0),
        }
    }
}

const MARGIN: f64 = 4.0;

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
}

/// Resize/reposition for `view` (anchored to the right edge, vertically centered) and render it.
pub fn show(app: &AppHandle, view: &View) {
    if let Some(w) = app.get_webview_window("pill") {
        if let Ok(Some(m)) = w.primary_monitor() {
            let (lw, lh) = view.size();
            let scale = m.scale_factor();
            let _ = w.set_size(LogicalSize::new(lw, lh));
            let (pos, size) = (m.position(), m.size());
            let x = pos.x + size.width as i32 - ((lw + MARGIN) * scale) as i32;
            let y = pos.y + (size.height as i32 - (lh * scale) as i32) / 2;
            let _ = w.set_position(PhysicalPosition::new(x, y));
        }
    }
    let _ = app.emit_to("pill", "pill", view);
}

/// Re-render without moving the window (timer ticks, level meter).
pub fn show_live(app: &AppHandle, view: &View) {
    let _ = app.emit_to("pill", "pill", view);
}
