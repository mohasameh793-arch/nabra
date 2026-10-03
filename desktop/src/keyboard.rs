//! Everything that touches the Windows keyboard: global shortcuts (low-level hook), typing text into
//! the focused app, the clipboard, and which app is in front.

use std::mem::size_of;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::OnceLock;

use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, GetMessageW, GetWindowTextW, GetWindowThreadProcessId, SetWindowsHookExW, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    TalkPressed,
    TalkReleased,
    NotesToggle,
    CommandPressed,
    CommandReleased,
    /// Right Alt was used as AltGr (another key pressed while held): not a voice command.
    CommandCancelled,
    /// The talk key was part of a shortcut (e.g. Right Ctrl + C): not a dictation.
    TalkCancelled,
    /// Ctrl+Alt+T: start/stop transcribing what the PC is playing.
    ListenToggle,
    /// Ctrl+Alt+U: "catch me up" on the last minutes of the call.
    CatchUp,
}

pub const NOTES_KEY_LABEL: &str = "Ctrl+Alt+N";
pub const LISTEN_KEY_LABEL: &str = "Ctrl+Alt+T";
pub const CATCH_UP_KEY_LABEL: &str = "Ctrl+Alt+U";
pub const COMMAND_KEY_LABEL: &str = "Right Alt";
const COMMAND_KEY: u32 = VK_RMENU.0 as u32;

/// Keys that can be the talk key: (setting id, label, virtual key, swallow it?). Keys with a side effect of
/// their own (Caps Lock, Insert…) are swallowed so holding them only dictates. "copilot" = the Copilot key (F23).
pub const TALK_KEYS: [(&str, &str, u32, bool); 7] = [
    ("right_ctrl", "Right Ctrl", VK_RCONTROL.0 as u32, false),
    ("right_shift", "Right Shift", 0xA1, false),
    ("caps_lock", "Caps Lock", 0x14, true),
    ("insert", "Insert", 0x2D, true),
    ("scroll_lock", "Scroll Lock", 0x91, true),
    ("pause", "Pause", 0x13, true),
    ("copilot", "Copilot key", 0x86, true),
];
static TALK_KEY: AtomicU32 = AtomicU32::new(0); // index into TALK_KEYS

/// Use the talk key chosen in Settings (unknown ids fall back to Right Ctrl).
pub fn set_talk_key(id: &str) {
    let i = TALK_KEYS.iter().position(|k| k.0 == id).unwrap_or(0);
    TALK_KEY.store(i as u32, Ordering::SeqCst);
}

pub fn talk_key_label() -> &'static str {
    TALK_KEYS[TALK_KEY.load(Ordering::SeqCst) as usize].1
}

/// Ctrl+Alt+<letter> shortcuts.
const COMBOS: [(u32, Shortcut); 3] =
    [(b'N' as u32, Shortcut::NotesToggle), (b'T' as u32, Shortcut::ListenToggle), (b'U' as u32, Shortcut::CatchUp)];
const MASK_KEY: VIRTUAL_KEY = VIRTUAL_KEY(0xE8); // unassigned: makes a lone Alt release not open app menus

static SINK: OnceLock<Sender<Shortcut>> = OnceLock::new();
static TALK_HELD: AtomicBool = AtomicBool::new(false);
static TALK_SPOILED: AtomicBool = AtomicBool::new(false);
/// The Ctrl+Alt+<letter> key currently held (0 = none), so auto-repeat fires a shortcut only once.
static COMBO_HELD: AtomicU32 = AtomicU32::new(0);
static COMMAND_HELD: AtomicBool = AtomicBool::new(false);
static COMMAND_SPOILED: AtomicBool = AtomicBool::new(false);

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
        if !synthetic && key.vkCode == COMMAND_KEY {
            if down && !COMMAND_HELD.swap(true, Ordering::SeqCst) {
                COMMAND_SPOILED.store(false, Ordering::SeqCst);
                fire = Some(Shortcut::CommandPressed);
            } else if up && COMMAND_HELD.swap(false, Ordering::SeqCst) {
                if !COMMAND_SPOILED.load(Ordering::SeqCst) {
                    // A lone Alt release would focus the app's menu bar; tap an unassigned key first.
                    let mask = [event(MASK_KEY, 0, false), event(MASK_KEY, 0, true)];
                    SendInput(&mask, size_of::<INPUT>() as i32);
                    fire = Some(Shortcut::CommandReleased);
                }
            }
        } else if !synthetic && down && COMMAND_HELD.load(Ordering::SeqCst) && !COMMAND_SPOILED.swap(true, Ordering::SeqCst) {
            fire = Some(Shortcut::CommandCancelled); // AltGr + key: the user is typing, not commanding
        }
        let (_, _, talk_vk, swallow_talk) = TALK_KEYS[TALK_KEY.load(Ordering::SeqCst) as usize];
        let combo = COMBOS.iter().find(|(vk, _)| *vk == key.vkCode).map(|(_, s)| *s);
        if !synthetic && key.vkCode == talk_vk {
            // Auto-repeat sends many downs; only the first press and the release matter.
            if down && !TALK_HELD.swap(true, Ordering::SeqCst) {
                TALK_SPOILED.store(false, Ordering::SeqCst);
                fire = Some(Shortcut::TalkPressed);
            } else if up && TALK_HELD.swap(false, Ordering::SeqCst) && !TALK_SPOILED.load(Ordering::SeqCst) {
                fire = Some(Shortcut::TalkReleased);
            }
            if swallow_talk {
                if let (Some(s), Some(tx)) = (fire, SINK.get()) {
                    let _ = tx.send(s);
                }
                return LRESULT(1); // e.g. Caps Lock: dictate, don't toggle caps
            }
        } else if !synthetic && down && TALK_HELD.load(Ordering::SeqCst) && !TALK_SPOILED.swap(true, Ordering::SeqCst) {
            fire = Some(Shortcut::TalkCancelled); // Right Ctrl + C etc.: a shortcut, not a dictation
        }
        // Ctrl+Alt+<letter>. Not with Right Alt held: that's AltGr typing a character (e.g. ń on Polish layouts).
        if let Some(shortcut) = combo.filter(|_| !synthetic) {
            if down && held(VK_CONTROL) && held(VK_MENU) && !held(VK_RMENU) {
                if COMBO_HELD.swap(key.vkCode, Ordering::SeqCst) != key.vkCode {
                    if let Some(tx) = SINK.get() {
                        let _ = tx.send(shortcut);
                    }
                }
                return LRESULT(1); // swallow it, so no letter lands in the focused app
            }
            if up && COMBO_HELD.compare_exchange(key.vkCode, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                return LRESULT(1);
            }
        }
        if let (Some(s), Some(tx)) = (fire, SINK.get()) {
            let _ = tx.send(s);
        }
    }
    // Modifier talk keys (Right Ctrl / Right Shift) are never swallowed: they keep working in every app.
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

/// Press a key (optionally with Ctrl/Shift held) `times` times.
pub fn press(vk: VIRTUAL_KEY, ctrl: bool, shift: bool, times: usize) {
    let mut inputs = Vec::new();
    for (on, mods) in [(ctrl, VK_CONTROL), (shift, VK_SHIFT)] {
        if on {
            inputs.push(event(mods, 0, false));
        }
    }
    for _ in 0..times {
        inputs.push(event(vk, 0, false));
        inputs.push(event(vk, 0, true));
    }
    for (on, mods) in [(shift, VK_SHIFT), (ctrl, VK_CONTROL)] {
        if on {
            inputs.push(event(mods, 0, true));
        }
    }
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
}

/// Characters as the user sees them (what Backspace / Shift+Left step over).
/// ponytail: counts Unicode scalars; combined emoji or Arabic with separate tashkeel may differ in some apps.
pub fn visible_len(text: &str) -> usize {
    text.replace("\r\n", "\n").chars().count()
}

pub fn read_clipboard() -> Option<String> {
    unsafe {
        OpenClipboard(Some(HWND::default())).ok()?;
        let text = (|| {
            let h = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(HGLOBAL(h.0));
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}

/// The selected text in the focused app (via Ctrl+C), or None. The clipboard is never emptied first: if
/// nothing is selected the app copies nothing and the clipboard (an image, files…) stays exactly as it was.
/// If something was copied, the user's previous text is put back.
// ponytail: a selection still replaces a copied image (as a manual Ctrl+C would); saving every clipboard format
// would fix that if it matters.
pub fn selected_text() -> Option<String> {
    let saved = read_clipboard();
    let before = unsafe { GetClipboardSequenceNumber() };
    press(VIRTUAL_KEY(b'C' as u16), true, false, 1);
    let mut changed = false;
    for _ in 0..15 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        if unsafe { GetClipboardSequenceNumber() } != before {
            changed = true;
            std::thread::sleep(std::time::Duration::from_millis(20)); // let the app finish writing
            break;
        }
    }
    if !changed {
        return None;
    }
    let got = read_clipboard().filter(|t| !t.trim().is_empty());
    if let Some(old) = saved {
        let _ = copy(&old);
    }
    got
}

// --- focused app ----------------------------------------------------------------------------

pub fn focused_title() -> String {
    unsafe {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(GetForegroundWindow(), &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

/// Which writing style applies: "personal" chats, "work" chats, "email", or "other".
pub fn app_kind(exe: &str, title: &str) -> &'static str {
    let exe = exe.to_ascii_lowercase();
    let title = title.to_lowercase();
    let any = |hay: &str, needles: &[&str]| needles.iter().any(|n| hay.contains(n));
    if any(&exe, &["outlook", "olk", "thunderbird", "hxoutlook"]) || any(&title, &["gmail", "outlook", "proton mail", "yahoo mail"]) {
        "email"
    } else if any(&exe, &["slack", "teams", "zoom"]) || any(&title, &["slack", "microsoft teams"]) {
        "work"
    } else if any(&exe, &["whatsapp", "telegram", "discord", "signal", "messenger", "instagram"])
        || any(&title, &["whatsapp", "messenger", "instagram", "telegram", "discord"])
    {
        "personal"
    } else {
        "other"
    }
}

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

    #[test]
    fn app_kinds() {
        assert_eq!(app_kind("WhatsApp", "WhatsApp"), "personal");
        assert_eq!(app_kind("chrome", "Inbox (3) - me@gmail.com - Gmail"), "email");
        assert_eq!(app_kind("slack", "general | Acme"), "work");
        assert_eq!(app_kind("Code", "main.rs - nabra"), "other");
        assert_eq!(visible_len("سلام\r\nok"), 7);
    }
}
