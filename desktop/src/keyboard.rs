//! Everything that touches the Windows keyboard: global shortcuts (low-level hook), typing text into
//! the focused app, the clipboard, and which app is in front.

use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_RCONTROL, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId, SetWindowsHookExW, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    TalkPressed,
    TalkReleased,
    NotesToggle,
}

pub const TALK_KEY_LABEL: &str = "Right Ctrl";
pub const NOTES_KEY_LABEL: &str = "Ctrl+Alt+N";
const TALK_KEY: u32 = VK_RCONTROL.0 as u32;
const NOTES_KEY: u32 = b'N' as u32;

static SINK: OnceLock<Sender<Shortcut>> = OnceLock::new();
static TALK_HELD: AtomicBool = AtomicBool::new(false);

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

unsafe extern "system" fn on_key(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let key = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let msg = wparam.0 as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
        let synthetic = key.flags.0 & LLKHF_INJECTED.0 != 0; // includes our own typing
        let mut fire = None;
        if !synthetic && key.vkCode == TALK_KEY {
            // Auto-repeat sends many downs; only the first press and the release matter.
            if down && !TALK_HELD.swap(true, Ordering::SeqCst) {
                fire = Some(Shortcut::TalkPressed);
            } else if up && TALK_HELD.swap(false, Ordering::SeqCst) {
                fire = Some(Shortcut::TalkReleased);
            }
        } else if !synthetic && down && key.vkCode == NOTES_KEY && held(VK_CONTROL) && held(VK_MENU) {
            if let Some(tx) = SINK.get() {
                let _ = tx.send(Shortcut::NotesToggle);
            }
            return LRESULT(1); // swallow it, so no "n" lands in the focused app
        }
        if let (Some(s), Some(tx)) = (fire, SINK.get()) {
            let _ = tx.send(s);
        }
    }
    // Right Ctrl itself is never swallowed: it keeps working in every app.
    CallNextHookEx(None, code, wparam, lparam)
}

/// Starts listening for the global shortcuts on a dedicated thread (hooks need a message loop).
pub fn listen(tx: Sender<Shortcut>) {
    let _ = SINK.set(tx);
    std::thread::spawn(|| unsafe {
        if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(on_key), None, 0) {
            eprintln!("keyboard hook: {e}");
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    });
}

// --- typing ---------------------------------------------------------------------------------

fn event(vk: VIRTUAL_KEY, unit: u16, release: bool) -> INPUT {
    let mut flags = if vk.0 == 0 { KEYEVENTF_UNICODE } else { KEYBD_EVENT_FLAGS(0) };
    if release {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: unit, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

fn keystrokes(text: &str) -> Vec<INPUT> {
    text.replace("\r\n", "\n")
        .encode_utf16()
        .flat_map(|u| {
            let (vk, unit) = if u == u16::from(b'\n') { (VK_RETURN, 0) } else { (VIRTUAL_KEY(0), u) };
            [event(vk, unit, false), event(vk, unit, true)]
        })
        .collect()
}

/// Types into whatever has focus. Windows silently drops input aimed at elevated (admin) windows.
pub fn type_text(text: &str) -> Result<(), String> {
    let inputs = keystrokes(text);
    let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) } as usize;
    if sent == inputs.len() {
        Ok(())
    } else {
        Err("That app blocked typing".into())
    }
}

pub fn copy(text: &str) -> Result<(), String> {
    let utf16: Vec<u16> = text.encode_utf16().chain([0]).collect();
    unsafe {
        OpenClipboard(Some(HWND::default())).map_err(|_| "Clipboard is busy".to_string())?;
        let done = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?;
            let mem = GlobalAlloc(GMEM_MOVEABLE, utf16.len() * 2).map_err(|e| e.to_string())?;
            let dst = GlobalLock(mem) as *mut u16;
            if dst.is_null() {
                return Err("GlobalLock failed".into());
            }
            std::ptr::copy_nonoverlapping(utf16.as_ptr(), dst, utf16.len());
            let _ = GlobalUnlock(mem);
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0))).map_err(|e| e.to_string())?;
            Ok(()) // the clipboard owns `mem` now
        })();
        let _ = CloseClipboard();
        done
    }
}

// --- focused app ----------------------------------------------------------------------------

/// Opaque id of the window that has focus (to detect the user switching away mid-transcription).
pub fn focused_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Executable name of the app in front, e.g. "Code" or "chrome". Empty if unknown.
pub fn focused_app() -> String {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(process);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arabic_newline_and_emoji_keystrokes() {
        let k = keystrokes("سلام\nOK");
        assert_eq!(k.len(), (4 + 1 + 2) * 2);
        assert_eq!(unsafe { k[8].Anonymous.ki.wVk }, VK_RETURN);
        assert_eq!(keystrokes("😀").len(), 4); // surrogate pair
        assert_eq!(keystrokes("a\r\nb").len(), 6);
    }
}
