//! Attendee names from the meeting app on screen (Zoom, Teams, or Google Meet / Zoom / Teams in a browser),
//! read through Windows UI Automation: the same interface screen readers use. Best-effort: it finds the names
//! the app exposes as list items, which is the participant list (Zoom/Teams "Participants", Meet "People").
//! Nothing is sent anywhere; names only label the call's speakers.

use std::collections::HashSet;

use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, TreeScope_Descendants, UIA_ButtonControlTypeId,
    UIA_ControlTypePropertyId, UIA_ListControlTypeId, UIA_ListItemControlTypeId,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};

const MAX_NAMES: usize = 30;
/// Only lists inside something named like the participant panel count (not the chat list, not the web page).
const PEOPLE_PANELS: [&str; 12] = [
    "participant", "people", "attendee", "in this meeting", "in the meeting", "in call", "roster", "المشارك",
    "الأشخاص", "الحضور", "الحاضرين", "في الاجتماع",
];
const BROWSERS: [&str; 6] = ["chrome.exe", "msedge.exe", "firefox.exe", "brave.exe", "opera.exe", "vivaldi.exe"];
const MEETING_APPS: [&str; 3] = ["zoom.exe", "ms-teams.exe", "teams.exe"];
/// List items in meeting apps that aren't people.
const NOT_PEOPLE: [&str; 24] = [
    "mute", "unmute", "chat", "participants", "people", "invite", "more", "video", "audio", "share", "share screen",
    "raise hand", "reactions", "leave", "end", "settings", "record", "apps", "whiteboard", "breakout rooms",
    "in this meeting", "waiting in lobby", "host", "everyone",
];

/// Names of people in the meeting(s) open on screen, without the user ("(me)", "(You)").
pub fn scan() -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else {
            return names;
        };
        for hwnd in meeting_windows() {
            names.extend(list_items(&uia, hwnd));
        }
    }
    clean(names)
}

/// The other people the meeting app shows as able to talk right now: unmuted, or marked as speaking. Polled about
/// once a second during call notes; a "They" phrase said while exactly one of them was unmuted is theirs.
/// Google Meet: a "Mute <name>'s microphone" button exists only for others, only while they're unmuted (seen in
/// a real call). Zoom/Teams: participant-list items that say "unmuted" or "speaking".
pub fn talking() -> Vec<String> {
    let mut names = Vec::new();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else {
            return names;
        };
        for hwnd in meeting_windows() {
            names.extend(talking_in(&uia, hwnd));
        }
    }
    let mut seen = HashSet::new();
    names.retain(|n: &String| !n.is_empty() && seen.insert(n.to_lowercase()));
    names
}

unsafe fn talking_in(uia: &IUIAutomation, hwnd: HWND) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(root) = uia.ElementFromHandle(hwnd) else { return out };
    let (Ok(is_button), Ok(is_item)) = (
        uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ButtonControlTypeId.0)),
        uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ListItemControlTypeId.0)),
    ) else {
        return out;
    };
    let Ok(either) = uia.CreateOrCondition(&is_button, &is_item) else { return out };
    let Ok(all) = root.FindAll(TreeScope_Descendants, &either) else { return out };
    for i in 0..all.Length().unwrap_or(0).min(600) {
        let Ok(name) = all.GetElement(i).and_then(|e| e.CurrentName()) else { continue };
        if let Some(n) = talking_name(&name.to_string()) {
            out.push(n);
        }
    }
    out
}

/// "Mute Ahmed's microphone" / "كتم صوت ميكروفون Ahmed" → Ahmed; "Zaid, Computer audio unmuted" → Zaid.
fn talking_name(label: &str) -> Option<String> {
    let label = strip_bidi(label);
    let lower = label.to_lowercase();
    let meet_en = label.get(..5).filter(|p| p.eq_ignore_ascii_case("mute ")).map(|_| &label[5..]).and_then(|rest| {
        rest.strip_suffix("'s microphone").or_else(|| rest.strip_suffix("’s microphone"))
    });
    let name = if let Some(rest) = label.strip_prefix("كتم صوت ميكروفون ") {
        rest.to_string()
    } else if let Some(rest) = meet_en {
        rest.to_string()
    } else if ["unmuted", "speaking", "is talking", "يتحدث", "غير مكتوم"].iter().any(|k| lower.contains(k))
        && !lower.contains("(me)") && !lower.contains("(you)") && !label.contains("(أنا)") && !label.contains("(أنت)")
    {
        label.split([',', '\n', '|', '(']).next().unwrap_or_default().to_string()
    } else {
        return None;
    };
    let name = name.trim().to_string();
    (2..=40).contains(&name.chars().count()).then_some(name)
}

/// Removes the invisible direction marks browsers put around names in Arabic pages (U+202A…U+202E, U+200E/F,
/// U+2066…U+2069), so a marked-up "Block guide" matches the plain name.
pub fn strip_bidi(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')).collect()
}

/// Visible top-level windows of Zoom/Teams, or browser tabs titled like a meeting.
fn meeting_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(hwnd: HWND, found: LPARAM) -> BOOL {
        let found = &mut *(found.0 as *mut Vec<HWND>);
        if IsWindowVisible(hwnd).as_bool() {
            let title = window_title(hwnd).to_lowercase();
            let exe = process_name(hwnd);
            let meeting_app = MEETING_APPS.contains(&exe.as_str());
            let meeting_tab = BROWSERS.contains(&exe.as_str())
                && (title.starts_with("meet -") || title.contains("google meet") || title.contains("zoom")
                    || title.contains("microsoft teams") || title.contains("| teams"));
            if (meeting_app && !title.is_empty()) || meeting_tab {
                found.push(hwnd);
            }
        }
        true.into()
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

unsafe fn window_title(hwnd: HWND) -> String {
    let mut buf = vec![0u16; GetWindowTextLengthW(hwnd) as usize + 1];
    let n = GetWindowTextW(hwnd, &mut buf);
    String::from_utf16_lossy(&buf[..n as usize])
}

unsafe fn process_name(hwnd: HWND) -> String {
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
    let _ = CloseHandle(process);
    if !ok {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().unwrap_or_default().to_lowercase()
}

/// The names in the window's participant list: list items of lists that sit in a panel named like
/// "Participants" / "People" / "In this meeting" (the list itself or up to 3 parents).
unsafe fn list_items(uia: &IUIAutomation, hwnd: HWND) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(root) = uia.ElementFromHandle(hwnd) else { return out };
    let (Ok(is_list), Ok(is_item)) = (
        uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ListControlTypeId.0)),
        uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ListItemControlTypeId.0)),
    ) else {
        return out;
    };
    let Ok(lists) = root.FindAll(TreeScope_Descendants, &is_list) else { return out };
    for i in 0..lists.Length().unwrap_or(0).min(50) {
        let Ok(list) = lists.GetElement(i) else { continue };
        if !in_people_panel(uia, &list) {
            continue;
        }
        let Ok(items) = list.FindAll(TreeScope_Descendants, &is_item) else { continue };
        for j in 0..items.Length().unwrap_or(0).min(300) {
            if let Ok(name) = items.GetElement(j).and_then(|e| e.CurrentName()) {
                out.push(name.to_string());
            }
        }
    }
    out
}

unsafe fn in_people_panel(uia: &IUIAutomation, list: &IUIAutomationElement) -> bool {
    let Ok(walker) = uia.ControlViewWalker() else { return false };
    let mut el = Some(list.clone());
    for _ in 0..4 {
        let Some(e) = el else { break };
        let name = e.CurrentName().map(|n| n.to_string().to_lowercase()).unwrap_or_default();
        if PEOPLE_PANELS.iter().any(|p| name.contains(p)) {
            return true;
        }
        el = walker.GetParentElement(&e).ok();
    }
    false
}

/// "Ahmed Ali (Host), Muted, Video off" → "Ahmed Ali"; drops the user's own entry and UI items.
fn clean(raw: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for item in raw {
        let item = strip_bidi(&item);
        let lower = item.to_lowercase();
        if lower.contains("(me)") || lower.contains("(you)") || item.contains("(أنا)") {
            continue;
        }
        let first = item.split([',', '\n', '|']).next().unwrap_or_default();
        let name = first.split('(').next().unwrap_or_default().trim().trim_end_matches(['-', ':']).trim().to_string();
        let words = name.split_whitespace().count();
        let looks_like_name = (2..=40).contains(&name.chars().count())
            && (1..=4).contains(&words)
            && name.chars().any(char::is_alphabetic)
            && name.chars().filter(|c| c.is_ascii_digit()).count() <= 2
            && !NOT_PEOPLE.contains(&name.to_lowercase().as_str());
        if looks_like_name && seen.insert(name.to_lowercase()) {
            names.push(name);
            if names.len() == MAX_NAMES {
                break;
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn participant_list_items_become_names() {
        let raw = [
            "Ahmed Ali (Host), Muted",
            "Mohamed Sameh (me)",
            "Amjad, In call, Video off",
            "Zaid",
            "zaid",
            "Mute",
            "Participants (3)",
            "Raise hand",
            "زيد الحربي",
            "https://zoom.us/j/1234567890",
        ]
        .map(String::from)
        .to_vec();
        assert_eq!(clean(raw), ["Ahmed Ali", "Amjad", "Zaid", "زيد الحربي"]);
    }

    #[test]
    fn who_can_talk_from_real_labels() {
        // Labels from a real Google Meet call (Arabic UI, with the browser's invisible direction marks).
        assert_eq!(talking_name("كتم صوت ميكروفون \u{202a}Block guide\u{202c}\u{200f}").as_deref(), Some("Block guide"));
        assert_eq!(talking_name("لا يمكنك إعادة صوت شخص آخر"), None, "muted: no name");
        assert_eq!(talking_name("Mute Ahmed Ali's microphone").as_deref(), Some("Ahmed Ali"));
        assert_eq!(talking_name("Zaid, Computer audio unmuted, Video on").as_deref(), Some("Zaid"));
        assert_eq!(talking_name("Mohamed Sameh (me), Computer audio unmuted"), None, "never the user");
        assert_eq!(talking_name("Mute"), None);
        assert_eq!(talking_name("Mute all"), None);
        assert_eq!(clean(vec!["\u{202a}\u{202a}Block guide\u{202c}\u{200f}\u{202c}\u{200f}".into()]), ["Block guide"]);
    }
}

#[cfg(test)]
mod live {
    /// Manual check of the UI Automation plumbing on any open File Explorer window:
    /// `cargo test uia_reads_list_items -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn uia_reads_list_items() {
        use super::*;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let uia: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).unwrap();
            unsafe extern "system" fn explorer(hwnd: HWND, out: LPARAM) -> BOOL {
                let out = &mut *(out.0 as *mut Vec<HWND>);
                if IsWindowVisible(hwnd).as_bool() && process_name(hwnd) == "explorer.exe" && !window_title(hwnd).is_empty() {
                    out.push(hwnd);
                }
                true.into()
            }
            let mut wins: Vec<HWND> = Vec::new();
            let _ = EnumWindows(Some(explorer), LPARAM(&mut wins as *mut _ as isize));
            let items: Vec<String> = wins.iter().flat_map(|w| list_items(&uia, *w)).collect();
            // Explorer has no participant panel, so this checks the UI Automation plumbing runs (and finds nothing).
            println!("{} explorer windows, {} participant names: {:?}", wins.len(), items.len(), items.iter().take(8).collect::<Vec<_>>());
        }
    }
}
