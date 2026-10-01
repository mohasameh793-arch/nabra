//! Commands the hub and pill pages can call (window.__TAURI__.core.invoke).

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::dictation::Control;
use crate::keyboard::{COMMAND_KEY_LABEL, NOTES_KEY_LABEL, TALK_KEY_LABEL};
use crate::store::{Dictation, Line, Note, NoteCard, Pad, Settings, Snippet, Word};
use crate::{calendar, keyboard, meeting, secrets, sidecar, sound, App, CALENDAR_SECRET};

type Res<T> = Result<T, String>;

/// Runs blocking work (engine calls) off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn boot(state: State<App>) -> Value {
    let meeting = state.meeting.lock().unwrap();
    json!({
        "settings": *state.settings.lock().unwrap(),
        "engine": *state.engine.lock().unwrap(),
        "meeting": meeting.as_ref().map(|m| json!({ "id": m.id, "elapsed": m.started.elapsed().as_secs_f32() })),
        "keys": { "talk": TALK_KEY_LABEL, "notes": NOTES_KEY_LABEL, "command": COMMAND_KEY_LABEL },
        "calendar_connected": secrets::get(CALENDAR_SECRET).is_some(),
        "version": env!("CARGO_PKG_VERSION"),
    })
}

#[tauri::command]
fn save_settings(state: State<App>, settings: Settings) -> Res<()> {
    state.store.save_settings(&settings)?;
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

#[tauri::command]
fn microphones() -> Vec<String> {
    sound::microphones()
}

#[tauri::command]
fn history(state: State<App>) -> Vec<Dictation> {
    state.store.history()
}

#[tauri::command]
fn edit_dictation(state: State<App>, id: u64, flagged: Option<bool>, delete: bool) -> Res<()> {
    state.store.edit_dictation(id, flagged, delete)
}

#[tauri::command]
fn clear_history(state: State<App>) -> Res<()> {
    state.store.clear_history()
}

#[tauri::command]
fn words(state: State<App>) -> Vec<Word> {
    state.store.words()
}

#[tauri::command]
fn save_word(state: State<App>, word: Word) -> Res<Vec<Word>> {
    state.store.save_word(word) // the engine reloads dictionary.json on its next request
}

#[tauri::command]
fn delete_word(state: State<App>, id: u64) -> Res<Vec<Word>> {
    state.store.delete_word(id)
}

#[tauri::command]
fn snippets(state: State<App>) -> Vec<Snippet> {
    state.store.snippets()
}

#[tauri::command]
fn save_snippet(state: State<App>, snippet: Snippet) -> Res<Vec<Snippet>> {
    state.store.save_snippet(snippet) // the engine re-reads snippets.json on its next request
}

#[tauri::command]
fn delete_snippet(state: State<App>, id: u64) -> Res<Vec<Snippet>> {
    state.store.delete_snippet(id)
}

#[tauri::command]
fn pads(state: State<App>) -> Vec<Pad> {
    state.store.pads()
}

#[tauri::command]
fn save_pad(state: State<App>, pad: Pad) -> Res<Vec<Pad>> {
    if pad.body.len() > 1_000_000 {
        return Err("That pad is too large".into());
    }
    state.store.save_pad(pad)
}

#[tauri::command]
fn delete_pad(state: State<App>, id: u64) -> Res<Vec<Pad>> {
    state.store.delete_pad(id)
}

#[tauri::command]
fn calendar_events(state: State<App>) -> Vec<calendar::Event> {
    state.calendar.lock().unwrap().clone()
}

/// Checks the link works before storing it (in Windows Credential Manager, not settings.json).
#[tauri::command]
async fn connect_calendar(app: AppHandle, url: String) -> Res<usize> {
    let url = url.trim().to_string();
    blocking(move || {
        let ics = calendar::fetch(&url)?;
        if !ics.contains("BEGIN:VCALENDAR") {
            return Err("That link isn't an iCal calendar. Copy the 'secret address in iCal format'.".into());
        }
        secrets::set(CALENDAR_SECRET, &url)?;
        crate::refresh_calendar(&app)
    })
    .await
}

#[tauri::command]
async fn refresh_calendar(app: AppHandle) -> Res<usize> {
    blocking(move || crate::refresh_calendar(&app)).await
}

#[tauri::command]
fn disconnect_calendar(state: State<App>) {
    secrets::delete(CALENDAR_SECRET);
    state.calendar.lock().unwrap().clear();
}

/// How to connect an MCP client (Claude Code / Claude Desktop / others) to the read-only notes server.
#[tauri::command]
fn mcp_setup(state: State<App>) -> Value {
    let root = sidecar::root();
    let python = root.join(".venv").join("Scripts").join("python.exe").display().to_string();
    let script = root.join("engine").join("mcp_notes.py").display().to_string();
    let notes = state.store.dir.join("notes").display().to_string();
    json!({
        "claude_code": format!("claude mcp add nabra-notes -- \"{python}\" \"{script}\" --notes \"{notes}\""),
        "json": { "mcpServers": { "nabra-notes": { "command": python, "args": [script, "--notes", notes] } } },
    })
}

#[tauri::command]
fn notes(state: State<App>) -> Vec<NoteCard> {
    state.store.note_cards()
}

#[tauri::command]
fn note(state: State<App>, id: String) -> Res<Note> {
    state.store.note(&id)
}

#[tauri::command]
fn delete_note(state: State<App>, id: String) -> Res<()> {
    state.store.delete_note(&id)
}

#[tauri::command]
async fn summarize_note(app: AppHandle, id: String, language: Option<String>) -> Res<Note> {
    blocking(move || meeting::summarize(&app, &id, language)).await
}

#[tauri::command]
async fn ask_notes(app: AppHandle, question: String) -> Res<String> {
    if question.trim().is_empty() || question.len() > 1000 {
        return Err("Ask a question (up to 1000 characters)".into());
    }
    blocking(move || {
        let notes: Vec<Value> = app
            .state::<App>()
            .store
            .notes()
            .into_iter()
            .filter(|n| n.summary.is_some())
            .take(20)
            .map(|n| json!({ "title": n.title, "date": n.started_at, "summary": n.summary }))
            .collect();
        sidecar::ask(&question, &Value::Array(notes))
    })
    .await
}

#[tauri::command]
fn accept_notes_consent(state: State<App>) -> Res<()> {
    let mut s = state.settings.lock().unwrap().clone();
    s.notes_consent = true;
    state.store.save_settings(&s)?;
    *state.settings.lock().unwrap() = s;
    Ok(())
}

#[tauri::command]
fn toggle_meeting(app: AppHandle) -> Res<()> {
    crate::toggle_meeting(&app)
}

#[tauri::command]
fn live_lines(state: State<App>) -> Vec<Line> {
    state.meeting.lock().unwrap().as_ref().map(|m| m.lines()).unwrap_or_default()
}

#[tauri::command]
fn copy_text(text: String) -> Res<()> {
    keyboard::copy(&text)
}

#[tauri::command]
fn pill_hover(state: State<App>, on: bool) {
    state.tell(Control::Hover(on));
}

#[tauri::command]
fn pill_mic(state: State<App>) {
    state.tell(Control::MicClicked);
}

#[tauri::command]
fn pill_notes(state: State<App>) {
    state.tell(Control::NotesClicked);
}

#[tauri::command]
fn open_hub(app: AppHandle, page: Option<String>) {
    crate::open_hub(&app, page.as_deref());
}

#[tauri::command]
fn hide_hub(app: AppHandle) {
    if let Some(w) = app.get_webview_window("hub") {
        let _ = w.hide();
    }
}

pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        boot,
        save_settings,
        microphones,
        history,
        edit_dictation,
        clear_history,
        words,
        save_word,
        delete_word,
        snippets,
        save_snippet,
        delete_snippet,
        pads,
        save_pad,
        delete_pad,
        calendar_events,
        connect_calendar,
        refresh_calendar,
        disconnect_calendar,
        mcp_setup,
        notes,
        note,
        delete_note,
        summarize_note,
        ask_notes,
        accept_notes_consent,
        toggle_meeting,
        live_lines,
        copy_text,
        pill_hover,
        pill_mic,
        pill_notes,
        open_hub,
        hide_hub
    ]
}
