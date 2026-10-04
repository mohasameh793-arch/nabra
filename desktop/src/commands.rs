//! Commands the hub and pill pages can call (window.__TAURI__.core.invoke).

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

use crate::dictation::Control;
use crate::keyboard::{talk_key_label, CATCH_UP_KEY_LABEL, COMMAND_KEY_LABEL, LISTEN_KEY_LABEL, NOTES_KEY_LABEL};
use crate::store::{Dictation, Line, Note, NoteCard, Pad, Settings, Snippet, Word};
use crate::{assets, autostart, calendar, keyboard, meeting, secrets, sidecar, sound, App, CALENDAR_SECRET};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

type Res<T> = Result<T, String>;

/// Runs blocking work (engine calls) off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Res<T> + Send + 'static) -> Res<T> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn boot(state: State<App>) -> Value {
    let settings = state.settings.lock().unwrap().clone(); // settings first, then meeting (same order everywhere)
    let meeting = state.meeting.lock().unwrap();
    json!({
        "settings": settings,
        "engine": *state.engine.lock().unwrap(),
        "meeting": meeting.as_ref().map(|m| json!({ "id": m.id, "elapsed": m.started.elapsed().as_secs_f32() })),
        "keys": { "talk": talk_key_label(), "notes": NOTES_KEY_LABEL, "command": COMMAND_KEY_LABEL, "listen": LISTEN_KEY_LABEL, "catch_up": CATCH_UP_KEY_LABEL },
        "talk_keys": crate::keyboard::TALK_KEYS.iter().map(|k| json!({ "id": k.0, "label": k.1 })).collect::<Vec<_>>(),
        "calendar_connected": secrets::get(CALENDAR_SECRET).is_some(),
        "setup_ready": assets::speech_ready(), // the AI model may still be downloading
        "autostart": autostart::enabled(),
        "version": env!("CARGO_PKG_VERSION"),
        "update": state.update.lock().unwrap().as_ref().map(|u| u.version.clone()),
    })
}

#[tauri::command]
fn save_settings(state: State<App>, settings: Settings) -> Res<Settings> {
    let mut current = state.settings.lock().unwrap();
    let mut next = settings;
    // Set by the app, not the settings screen (the hub may hold an older copy): keep the live values.
    next.pill_dock = current.pill_dock.clone();
    next.notes_consent = current.notes_consent;
    next.last_version = current.last_version.clone();
    state.store.save_settings(&next)?;
    crate::keyboard::set_talk_key(&next.talk_key);
    state.store.prune_history(&next.history_keep)?;
    *current = next.clone();
    Ok(next)
}

#[tauri::command]
fn set_autostart(on: bool) -> Res<bool> {
    autostart::set(on)?;
    Ok(autostart::enabled())
}

/// Settings → About → Check for updates. Installs right away if one exists (Nabra restarts).
#[tauri::command]
async fn check_updates(app: AppHandle) -> Res<crate::updater::Status> {
    crate::updater::check(&app).await
}

#[tauri::command]
async fn install_update(app: AppHandle) -> Res<()> {
    crate::updater::install(&app).await
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

/// The Transforms page's "try it" box (the same AI the Right Alt command uses).
#[tauri::command]
async fn transform_text(text: String, instruction: String) -> Res<String> {
    if text.trim().is_empty() || instruction.trim().is_empty() || text.len() > 20_000 {
        return Err("Add some text and an instruction".into());
    }
    blocking(move || sidecar::transform(&text, &instruction)).await
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
/// It's the engine program in `--mcp` mode: bundled exe when installed, Python in a source checkout.
#[tauri::command]
fn mcp_setup(app: AppHandle, state: State<App>) -> Res<Value> {
    let (program, lead, _) = sidecar::engine_command(&app)?;
    let notes = state.store.dir.join("notes").display().to_string();
    let mut args: Vec<String> = lead;
    args.extend(["--mcp".into(), "--notes".into(), notes]);
    let quoted: Vec<String> = std::iter::once(program.display().to_string()).chain(args.clone()).map(|a| format!("\"{a}\"")).collect();
    Ok(json!({
        "claude_code": format!("claude mcp add -s user nabra-notes -- {}", quoted.join(" ")),
        "json": { "mcpServers": { "nabra-notes": { "command": program.display().to_string(), "args": args } } },
    }))
}

static SETTING_UP: AtomicBool = AtomicBool::new(false);

/// What this PC needs to download, and what's already there.
#[tauri::command]
fn setup_status() -> Value {
    let (vram, parts) = assets::plan();
    json!({
        "gpu_vram_mb": vram,
        "ready": parts.iter().all(|p| p.installed()),
        "speech_ready": assets::speech_ready(),
        "free_mb": assets::free_mb(),
        "running": SETTING_UP.load(Ordering::SeqCst),
        "folder": assets::dir().display().to_string(),
        "parts": parts.iter().map(|p| json!({ "id": p, "label": p.label(), "mb": p.approx_mb(), "installed": p.installed() })).collect::<Vec<_>>(),
    })
}

/// Downloads everything missing, emitting `setup-progress`; then starts Nabra. Safe to re-run: it resumes.
#[tauri::command]
async fn setup_run(app: AppHandle) -> Res<()> {
    if SETTING_UP.swap(true, Ordering::SeqCst) {
        return Err("Setup is already running".into());
    }
    let result = blocking({
        let app = app.clone();
        move || {
            let (_, parts) = assets::plan();
            let missing: Vec<_> = parts.into_iter().filter(|p| !p.installed()).collect();
            // Enough room? (+10% for unpacking.) If only speech fits, get speech now; the AI model is optional.
            let need = |ps: &[assets::Part]| ps.iter().map(|p| p.approx_mb()).sum::<u64>() * 11 / 10;
            let speech: Vec<_> = missing.iter().copied().filter(|p| p.is_speech()).collect();
            let free = assets::free_mb().unwrap_or(u64::MAX);
            let todo = if free >= need(&missing) {
                missing
            } else if free >= need(&speech) {
                let _ = app.emit("setup-note", format!(
                    "Not enough free space for the local AI model ({:.1} GB). Dictation and notes will work without AI cleanup and summaries.",
                    need(&missing[speech.len()..]) as f64 / 1024.0));
                speech
            } else {
                return Err(format!(
                    "Nabra needs {:.1} GB free on the drive of {} and there is {:.1} GB. Free some space and try again.",
                    need(&speech) as f64 / 1024.0, assets::dir().display(), free as f64 / 1024.0));
            };
            let mut told = false;
            for part in todo {
                if !told && assets::speech_ready() {
                    told = true; // speech is in: start dictating while the AI model downloads
                    app.state::<App>().tell(Control::SetupDone);
                }
                let _ = app.emit("setup-progress", json!({ "part": part, "done": 0, "total": 0, "stage": "Preparing" }));
                let files = assets::resolve(part)?;
                let total: u64 = files.iter().map(|f| f.size).sum();
                let mut before = 0u64;
                let mut last_emit = std::time::Instant::now();
                for f in &files {
                    assets::fetch(f, |done| {
                        if last_emit.elapsed().as_millis() >= 250 {
                            last_emit = std::time::Instant::now();
                            let _ = app.emit("setup-progress", json!({ "part": part, "done": before + done, "total": total, "stage": "Downloading" }));
                        }
                    })?;
                    before += f.size;
                }
                assets::mark_complete(part);
                let _ = app.emit("setup-progress", json!({ "part": part, "done": total, "total": total, "stage": "Done" }));
            }
            Ok(())
        }
    })
    .await;
    SETTING_UP.store(false, Ordering::SeqCst);
    result?;
    app.state::<App>().tell(Control::SetupDone);
    let _ = app.emit("setup-done", ());
    Ok(())
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
fn accept_notes_consent(state: State<App>) -> Res<()> {
    let mut s = state.settings.lock().unwrap().clone();
    s.notes_consent = true;
    state.store.save_settings(&s)?;
    *state.settings.lock().unwrap() = s;
    Ok(())
}

/// Start notes named after a calendar event (Notetaker → Today → Take notes).
#[tauri::command]
fn start_notes_for(app: AppHandle, state: State<App>, title: String) -> Res<()> {
    if state.meeting.lock().unwrap().is_some() {
        return Err("Already taking notes".into());
    }
    *state.pending_title.lock().unwrap() = Some((title.chars().take(120).collect(), crate::store::now_ms()));
    crate::toggle_meeting(&app)
}

#[tauri::command]
fn toggle_meeting(app: AppHandle) -> Res<()> {
    crate::toggle_meeting(&app)
}

/// Saves "My thoughts": into the live meeting while it records, else into the saved note.
#[tauri::command]
fn save_thoughts(state: State<App>, id: String, text: String) -> Res<()> {
    if text.len() > 200_000 {
        return Err("Your notes are too long to save".into());
    }
    if let Some(m) = state.meeting.lock().unwrap().as_ref().filter(|m| m.id == id) {
        m.set_thoughts(text);
        return Ok(());
    }
    let mut note = state.store.note(&id)?;
    note.thoughts = text;
    state.store.save_note(&note)
}

#[tauri::command]
fn live_thoughts(state: State<App>) -> String {
    state.meeting.lock().unwrap().as_ref().map(|m| m.thoughts()).unwrap_or_default()
}

#[tauri::command]
fn open_meeting_window(app: AppHandle) {
    crate::show_meeting_window(&app);
}

#[tauri::command]
fn close_meeting_window(app: AppHandle) {
    if let Some(w) = app.get_webview_window("meeting") {
        let _ = w.hide();
    }
}

#[tauri::command]
fn live_lines(state: State<App>) -> Vec<Line> {
    state.meeting.lock().unwrap().as_ref().map(|m| m.lines()).unwrap_or_default()
}

/// "Catch me up" from the meeting window: the last 5 minutes in 3 bullets.
#[tauri::command]
async fn catch_up(app: AppHandle) -> Res<String> {
    blocking(move || crate::meeting::catch_up(&app)).await
}

/// Speakers + attendee names of the live call or a saved note.
#[tauri::command]
fn note_speakers(state: State<App>, id: String) -> Res<Value> {
    if let Some(m) = state.meeting.lock().unwrap().as_ref().filter(|m| m.id == id) {
        return Ok(crate::meeting::speakers_json(&m.note.lock().unwrap()));
    }
    Ok(crate::meeting::speakers_json(&state.store.note(&id)?))
}

/// Name a voice ("Speaker 2" → "Zaid"); Nabra recognises it in later calls.
#[tauri::command]
fn name_speaker(app: AppHandle, id: String, speaker: String, name: String) -> Res<Value> {
    crate::meeting::name_speaker(&app, &id, &speaker, &name)
}

/// Add a name by hand, or (`scan`) look at the meeting app on screen again (e.g. after opening its participant list).
#[tauri::command]
async fn add_attendees(app: AppHandle, id: String, names: Vec<String>, scan: bool) -> Res<()> {
    blocking(move || {
        let mut names = names;
        if scan {
            names.extend(crate::attendees::scan());
        }
        crate::meeting::add_attendees(&app, &id, names);
        Ok(())
    })
    .await
}

#[tauri::command]
fn forget_voices(state: State<App>) -> Res<()> {
    state.store.forget_voices()
}

#[tauri::command]
fn copy_text(text: String) -> Res<()> {
    keyboard::copy(&text)
}

#[tauri::command]
fn pill_hover(state: State<App>, on: bool) {
    state.tell(Control::Hover(on));
}

/// The user is dragging the pill (`on`) or let go of it (snaps to left / bottom / right middle).
#[tauri::command]
fn pill_drag(app: AppHandle, on: bool) {
    crate::pill::drag(&app, on);
}

/// The pill's current view, for a pill page that just (re)loaded.
#[tauri::command]
fn pill_state() -> Option<serde_json::Value> {
    crate::pill::current()
}

#[tauri::command]
fn pill_dismiss(state: State<App>) {
    state.tell(Control::Dismiss);
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
        check_updates,
        install_update,
        note_speakers,
        catch_up,
        name_speaker,
        add_attendees,
        forget_voices,
        set_autostart,
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
        transform_text,
        calendar_events,
        connect_calendar,
        refresh_calendar,
        disconnect_calendar,
        mcp_setup,
        setup_status,
        setup_run,
        notes,
        note,
        delete_note,
        summarize_note,
        accept_notes_consent,
        toggle_meeting,
        start_notes_for,
        live_lines,
        save_thoughts,
        live_thoughts,
        open_meeting_window,
        close_meeting_window,
        copy_text,
        pill_hover,
        pill_drag,
        pill_mic,
        pill_dismiss,
        pill_notes,
        pill_state,
        open_hub,
        hide_hub
    ]
}
