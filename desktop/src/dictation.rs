//! The dictation controller: one thread that owns the microphone while you speak, talks to the engine,
//! types or edits text, records history, and decides what the pill shows.
//!
//!   Right Ctrl (hold) / pill mic (toggle)  → dictate: text is typed where the cursor is
//!   Right Ctrl tapped twice                → hands-free dictation; one more tap types it
//!   Right Alt (hold)                       → command: "scratch that", "new line", a transform
//!                                            ("make it shorter") applied to the selection or the
//!                                            last dictation, or a question about past calls
//!   Ctrl+Alt+T (toggle)                    → transcribe what the PC is playing (shown, not typed or kept)
//!   Ctrl+Alt+U                             → during call notes: catch me up on the last 5 minutes

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY, VK_BACK, VK_LEFT, VK_RIGHT, VK_RMENU};

use crate::keyboard::{self, talk_key_label, Shortcut, COMMAND_KEY_LABEL, NOTES_KEY_LABEL};
use crate::pill::{self, View};
use crate::sound::{lift_quiet, wav, Source, Tap};
use crate::store::Dictation;
use crate::{sidecar, App};

/// Inputs to the controller.
#[derive(Debug)]
pub enum Control {
    Key(Shortcut),
    MicClicked,
    NotesClicked,
    Hover(bool),
    /// A calendar meeting is starting: offer to take notes.
    MeetingStarting(String),
    /// The user closed a result or meeting prompt on the pill.
    Dismiss,
    /// The updater found a new version: offer it on the pill.
    UpdateAvailable(String),
    /// Update progress ("Updating Nabra… 40%").
    Updating(String),
    UpdateFailed(String),
    /// First-run downloads finished: start the engine.
    SetupDone,
    /// Something outside changed (e.g. notes started from the hub): re-render the pill.
    Refresh,
    /// A finished background answer (catch-up, ask my meetings) to show on the pill.
    Reveal { title: String, text: String, note: Option<String>, t: f32 },
}

const TOO_SHORT_S: f32 = 0.3;
const RESULT_FOR: Duration = Duration::from_millis(2600);
const PROBLEM_FOR: Duration = Duration::from_secs(5);
const MEETING_PROMPT_FOR: Duration = Duration::from_secs(90);
const UPDATE_PROMPT_FOR: Duration = Duration::from_secs(120);
const UPDATING_FOR: Duration = Duration::from_secs(600);
const REVEAL_FOR: Duration = Duration::from_secs(60);
/// A forgotten hands-free dictation stops itself (and is transcribed) after this long.
const MAX_TAKE: Duration = Duration::from_secs(5 * 60);
/// Hands-free (double-tap, pill mic) dictation runs this long before it stops itself and types what was said.
const MAX_HANDS_FREE: Duration = Duration::from_secs(20 * 60);
/// A press of the talk key this short is a tap, not a dictation; two taps this close together start hands-free.
const TAP: Duration = Duration::from_millis(300);
const DOUBLE_TAP: Duration = Duration::from_millis(500);
const MAX_LISTEN: Duration = Duration::from_secs(10 * 60);
/// "scratch that" / "make it shorter" only reach back this far: after that the user has likely typed or moved
/// the cursor, and Backspace / Shift+Left would hit their own text.
const LAST_FOR: Duration = Duration::from_secs(30);

struct Take {
    tap: Tap,
    started: Instant,
    window: isize,
    app_name: String,
    app_kind: &'static str,
    hands_free: bool,
    command: bool,
    /// Recording what the PC plays (Ctrl+Alt+T), not the microphone.
    listen: bool,
}

/// What Nabra typed last, so "scratch that" / "make it shorter" can find it again.
struct Inserted {
    window: isize,
    text: String,
    at: Instant,
}

pub struct Controller {
    app: AppHandle,
    take: Option<Take>,
    last: Option<Inserted>,
    hovering: bool,
    ready: bool,
    shown: Option<View>,
    hold_until: Option<Instant>, // a result/problem/prompt stays visible until then
    /// Why the engine isn't ready (shown instead of "still starting" forever).
    engine_error: Option<String>,
    /// Prompts that arrived mid-dictation, shown once it ends (never silently dropped).
    deferred: Vec<Control>,
    /// Dictations this run that took too long on turbo (autotune); offered the light model once it reaches 3.
    slow_takes: u8,
    /// The Arabic dialect of the last dictation that had one (the pill's badge), e.g. "Gulf".
    heard: &'static str,
    /// When the talk key was last tapped (pressed and let go quickly): a second tap soon after starts hands-free.
    tapped: Option<Instant>,
}

fn dialect_label(code: &str) -> &'static str {
    match code {
        "gulf" => "Gulf",
        "egyptian" => "Egyptian",
        "levantine" => "Levantine",
        "msa" => "MSA",
        _ => "",
    }
}

/// Autotune: a dictation is slow when turning it into text takes over 3 s and over half as long as the speech.
fn too_slow(spoke_s: f32, took: Duration) -> bool {
    let took = took.as_secs_f32();
    took > 3.0 && took > spoke_s * 0.5
}
const SLOW_TAKES_BEFORE_OFFER: u8 = 3;

impl Controller {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app, take: None, last: None, hovering: false, ready: false, shown: None, hold_until: None, engine_error: None,
            deferred: Vec::new(), slow_takes: 0, heard: "", tapped: None,
        }
    }

    /// Turbo running on the processor, and the light model not chosen yet: the only setup autotune can speed up.
    fn on_turbo_cpu(&self) -> bool {
        use crate::assets::{prefers_light, Part};
        let cpu = self.state().engine.lock().unwrap().as_ref().is_some_and(|h| h.device != "cuda");
        cpu && !prefers_light() && Part::WhisperCpu.installed() && !Part::WhisperGpu.installed()
    }

    fn state(&self) -> tauri::State<'_, App> {
        self.app.state::<App>()
    }

    fn render(&mut self, view: View) {
        match &self.shown {
            Some(v) if *v == view => return,
            // Same kind of view (e.g. the notes timer ticking): update content, keep the window as is.
            Some(v) if std::mem::discriminant(v) == std::mem::discriminant(&view) => pill::show_live(&self.app, &view),
            _ => pill::show(&self.app, &view),
        }
        self.shown = Some(view);
    }

    fn flash(&mut self, view: View, for_: Duration) {
        self.render(view);
        self.hold_until = Some(Instant::now() + for_);
    }

    fn done(&mut self, text: impl Into<String>, detail: impl Into<String>) {
        self.flash(View::Result { text: text.into(), detail: detail.into() }, RESULT_FOR);
    }

    fn problem(&mut self, message: impl Into<String>) {
        self.flash(View::Problem { message: message.into() }, PROBLEM_FOR);
    }

    /// The resting view: hover buttons, notes timer, or the slim handle.
    fn rest(&mut self) {
        if self.hold_until.is_some_and(|t| Instant::now() < t) {
            return;
        }
        self.hold_until = None;
        let notes = self.state().meeting.lock().unwrap().as_ref().map(|m| m.started.elapsed().as_secs());
        let view = if self.hovering {
            View::Hover { notes_on: notes.is_some(), talk_key: talk_key_label(), notes_key: NOTES_KEY_LABEL, dialect: self.heard }
        } else if let Some(seconds) = notes {
            View::Notes { seconds }
        } else {
            View::Idle
        };
        self.render(view);
    }

    fn begin(&mut self, hands_free: bool, command: bool) {
        self.begin_take(hands_free, command, false);
    }

    fn begin_take(&mut self, hands_free: bool, command: bool, listen: bool) {
        if !self.ready {
            if self.engine_error.is_none() {
                return self.problem("Still starting the speech engine…");
            }
            // It failed to start earlier (driver reset, slow model load): try again rather than stay dead.
            self.render(View::Working { label: "Restarting the speech engine…".into() });
            return match self.start_engine() {
                Ok(_) => self.problem("The speech engine restarted. Please try again."),
                Err(e) => self.problem(e),
            };
        }
        let mic = self.state().settings.lock().unwrap().microphone.clone();
        let opened = if listen { Tap::open(Source::System, None) } else { Tap::open(Source::Mic, mic.as_deref()) };
        match opened {
            Ok(tap) => {
                let app_name = keyboard::focused_app();
                let app_kind = keyboard::app_kind(&app_name, &keyboard::focused_title());
                self.take = Some(Take {
                    tap,
                    started: Instant::now(),
                    window: keyboard::focused_window(),
                    app_name,
                    app_kind,
                    hands_free: hands_free || listen,
                    command,
                    listen,
                });
                self.state().busy.store(true, std::sync::atomic::Ordering::SeqCst); // no update mid-dictation
                self.hold_until = None;
                self.render(View::Listening { seconds: 0.0, level: 0.0, hands_free: hands_free || listen, command });
            }
            Err(e) => self.problem(e),
        }
    }

    /// Stops recording; returns the take and its WAV, or None if it was too short.
    fn stop(&mut self) -> Option<(Take, Vec<u8>, f32)> {
        let take = self.take.take()?;
        let mut samples = take.tap.take();
        let rate = take.tap.rate;
        let seconds = samples.len() as f32 / rate as f32;
        if seconds < TOO_SHORT_S {
            self.rest();
            return None;
        }
        lift_quiet(&mut samples); // whisper mode
        Some((take, wav(&samples, rate), seconds))
    }

    fn finish(&mut self) {
        let Some((take, audio, seconds)) = self.stop() else { return };
        if take.command {
            return self.command(take, &audio);
        }
        if take.listen {
            return self.heard(&audio);
        }
        self.render(View::Busy);
        let (langs, mode, style, dialect) = {
            let state = self.state();
            let s = state.settings.lock().unwrap();
            (s.langs(), s.mode.clone(), s.styles.for_kind(take.app_kind).to_string(), s.dialect.clone())
        };
        let asked = Instant::now();
        let Some(result) = self.call_engine(|| sidecar::dictate(&audio, &langs, &mode, &style, &dialect)) else { return };
        if let Some(d) = &result.dialect {
            self.heard = dialect_label(d);
        }
        if too_slow(seconds, asked.elapsed()) && self.on_turbo_cpu() {
            self.slow_takes = self.slow_takes.saturating_add(1);
            crate::log(format!("slow dictation: {seconds:.1} s of speech took {:.1} s", asked.elapsed().as_secs_f32()));
        }
        if result.text.trim().is_empty() {
            return self.problem("Didn't catch that. Try again.");
        }

        // Type into the app the user started in; if they switched away, use the clipboard instead.
        // Typed where the cursor is: nothing more to show (like Flow). Only a clipboard fallback is worth a note.
        // Only claim "it's on your clipboard" if it really is (a clipboard manager may be holding it).
        let clipboard = |why: String| match keyboard::copy(&result.text) {
            Ok(()) => format!("{why}, so it's on your clipboard"),
            Err(_) => format!("{why}. It's saved in your history in the Nabra window"),
        };
        let fallback = if keyboard::focused_window() != take.window {
            Some(clipboard("You switched windows".into()))
        } else if let Err(e) = keyboard::type_text(&result.text) {
            Some(clipboard(e))
        } else {
            self.last = Some(Inserted { window: take.window, text: result.text.clone(), at: Instant::now() });
            None
        };

        let entry = Dictation {
            id: crate::store::new_id(),
            app: take.app_name,
            words: result.text.split_whitespace().count() as u32,
            text: result.text.clone(),
            language: result.language,
            seconds,
            fixes: result.fixes,
            flagged: false,
        };
        let state = self.state();
        *state.last_text.lock().unwrap() = result.text.clone();
        let keep = state.settings.lock().unwrap().history_keep.clone();
        if keep != "off" {
            if let Err(e) = state.store.add_dictation(&entry) {
                eprintln!("history: {e}");
            }
        }
        let _ = self.app.emit("dictation", &entry);
        match fallback {
            Some(detail) => self.done(result.text, detail),
            None if (SLOW_TAKES_BEFORE_OFFER..u8::MAX).contains(&self.slow_takes) => {
                self.slow_takes = u8::MAX; // offered once per run; "Later" asks again next time Nabra starts
                self.flash(View::Slow, UPDATE_PROMPT_FOR);
            }
            None => {
                self.hold_until = None;
                self.rest();
            }
        }
    }

    /// Ctrl+Alt+T: what the PC was playing (a voice note, a video), transcribed in the speaker's own dialect.
    /// It's someone else's speech, so it's only shown (Copy if wanted): never typed, never kept in history.
    fn heard(&mut self, audio: &[u8]) {
        self.render(View::Busy);
        let langs = self.state().settings.lock().unwrap().langs();
        match self.call_engine(|| sidecar::note_chunk(audio, &langs, false, "")) {
            Some(text) if text.trim().is_empty() => self.problem("Didn't hear any speech playing."),
            Some(text) => self.flash(View::Reveal { title: "What was playing".into(), text, note: None, t: 0.0 }, REVEAL_FOR),
            None => {}
        }
    }

    /// Runs an engine request. If the engine is gone (it crashed, or a GPU error took it down), starts it again
    /// and repeats the request once with the same audio, so a long dictation isn't lost. On failure the problem
    /// is shown and None returned.
    fn call_engine<T>(&mut self, call: impl Fn() -> Result<T, String>) -> Option<T> {
        let mut result = call();
        if result.as_ref().is_err_and(|e| e == sidecar::ENGINE_DOWN) {
            crate::log("engine stopped answering: restarting it");
            self.render(View::Working { label: "Restarting the speech engine…".into() });
            result = self.start_engine().and_then(|_| call());
        }
        match result {
            Ok(v) => Some(v),
            Err(e) => {
                // e.g. llama-server died: refresh what the hub shows instead of claiming local AI is still on.
                if let Some(health) = self.ready.then(sidecar::health).flatten() {
                    self.engine_up(health);
                }
                self.problem(e);
                None
            }
        }
    }

    fn engine_up(&mut self, health: sidecar::Health) {
        let _ = self.app.emit("engine", &health);
        *self.state().engine.lock().unwrap() = Some(health);
    }

    /// Starts (or restarts) the engine and waits until it answers.
    fn start_engine(&mut self) -> Result<sidecar::Health, String> {
        self.ready = false;
        let launched = {
            let state = self.state();
            let keep_clips = state.settings.lock().unwrap().keep_clips;
            sidecar::start(
                &self.app,
                sidecar::Launch {
                    dictionary: &state.store.dictionary_path(),
                    snippets: &state.store.snippets_path(),
                    clips: &state.store.dir.join("clips"),
                    keep_clips,
                },
            )
        };
        let result = launched.and_then(|_| sidecar::wait_ready(Duration::from_secs(240)));
        match &result {
            Ok(health) => {
                self.engine_up(health.clone());
                self.ready = true;
                self.engine_error = None;
            }
            Err(e) => self.engine_error = Some(e.clone()),
        }
        result
    }

    /// Ctrl+Alt+U during call notes: the last few minutes in 3 bullets (worked out in the background).
    fn catch_up(&mut self) {
        if self.state().meeting.lock().unwrap().is_none() {
            return self.problem("Catch me up works during call notes (Ctrl+Alt+N).");
        }
        self.flash(View::Working { label: "Catching you up…".into() }, UPDATING_FOR);
        let app = self.app.clone();
        std::thread::spawn(move || {
            let reveal = match crate::meeting::catch_up(&app) {
                Ok(text) => Control::Reveal { title: "While you were away".into(), text, note: None, t: 0.0 },
                Err(e) => Control::Reveal { title: "Couldn't catch you up".into(), text: e, note: None, t: 0.0 },
            };
            app.state::<App>().tell(reveal);
        });
    }

    /// Right Alt + a question about past calls: search the notes, answer from them, point to the moment.
    fn ask(&mut self, question: String) {
        self.flash(View::Working { label: "Searching your meetings…".into() }, UPDATING_FOR);
        let app = self.app.clone();
        std::thread::spawn(move || {
            let reveal = match crate::meeting::ask(&app, &question) {
                Ok((text, note, t)) => Control::Reveal { title: question, text, note, t },
                Err(e) => Control::Reveal { title: question, text: e, note: None, t: 0.0 },
            };
            app.state::<App>().tell(reveal);
        });
    }

    /// Right Alt: a fixed edit ("scratch that", "new line") or a transform of the selection / last dictation.
    fn command(&mut self, take: Take, audio: &[u8]) {
        self.render(View::Working { label: "Listening to your command…".into() });
        let langs = self.state().settings.lock().unwrap().langs();
        let Some((instruction, action)) = self.call_engine(|| sidecar::instruction(audio, &langs)) else { return };
        if action == "ask" {
            return self.ask(instruction); // a question: no text to edit, any window is fine
        }
        if keyboard::focused_window() != take.window {
            return self.problem("You switched windows, so the command was skipped");
        }
        let last_here =
            self.last.as_ref().filter(|l| l.window == take.window && l.at.elapsed() < LAST_FOR).map(|l| l.text.clone());
        match action.as_str() {
            "none" => self.problem("Didn't catch the command. Try again."),
            "delete_last" => match last_here {
                Some(text) => {
                    keyboard::press(VK_BACK, false, false, keyboard::visible_len(&text));
                    self.last = None;
                    self.done("Deleted your last dictation", "")
                }
                None => self.problem("Nothing I typed here to delete"),
            },
            "new_line" | "new_paragraph" => {
                let breaks = if action == "new_line" { "\n" } else { "\n\n" };
                let _ = keyboard::type_text(breaks);
                self.done(if action == "new_line" { "New line" } else { "New paragraph" }, "")
            }
            "undo" => {
                keyboard::press(VIRTUAL_KEY(b'Z' as u16), true, false, 1);
                self.done("Undone", "")
            }
            "select_all" => {
                keyboard::press(VIRTUAL_KEY(b'A' as u16), true, false, 1);
                self.done("Selected all", "")
            }
            _ => self.transform(&take, &instruction, last_here),
        }
    }

    fn transform(&mut self, take: &Take, instruction: &str, last_here: Option<String>) {
        // Target: what the user selected; otherwise the last thing Nabra typed in this window.
        // ponytail: with nothing selected, some editors (VS Code) copy the whole line on Ctrl+C; that line
        // then becomes the target. Select text first for precise edits.
        let mut selected_back = false;
        let target = match keyboard::selected_text() {
            Some(sel) => sel,
            None => match last_here {
                Some(text) => {
                    keyboard::press(VK_LEFT, false, true, keyboard::visible_len(&text)); // select it back
                    selected_back = true;
                    text
                }
                None => return self.problem(format!("Select some text first, then hold {COMMAND_KEY_LABEL} and say what to do")),
            },
        };
        self.render(View::Working { label: format!("“{instruction}”") });
        match sidecar::transform(&target, instruction) {
            Ok(out) if !out.trim().is_empty() => {
                if let Err(e) = keyboard::type_text(&out) {
                    let _ = keyboard::copy(&out);
                    return self.problem(format!("{e}, so the result is on your clipboard"));
                }
                self.last = Some(Inserted { window: take.window, text: out.clone(), at: Instant::now() });
                *self.state().last_text.lock().unwrap() = out.clone();
                self.done(out, "Transformed")
            }
            failed => {
                if selected_back {
                    keyboard::press(VK_RIGHT, false, false, 1); // unselect it, so the next keystroke can't replace it
                }
                match failed {
                    Ok(_) => self.problem("The AI returned nothing. Your text is unchanged."),
                    Err(e) => self.problem(e),
                }
            }
        }
    }

    fn toggle_notes(&mut self) {
        if let Err(e) = crate::toggle_meeting(&self.app) {
            self.problem(e);
        }
        self.hold_until = None;
        self.rest();
    }

    pub fn run(mut self, inbox: Receiver<Control>) {
        if !crate::assets::speech_ready() {
            // First run: the models aren't downloaded yet. The hub's Setup page does that, then tells us.
            self.render(View::Working { label: "Finish setting up Nabra in its window".into() });
            crate::open_hub(&self.app, Some("setup"));
            loop {
                match inbox.recv() {
                    Ok(Control::SetupDone) => break,
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        }
        self.render(View::Working { label: "Starting Nabra…".into() });
        crate::log("starting the speech engine");
        let started = self.start_engine();
        crate::log(match &started {
            Ok(h) => format!("engine ready (device={}, llm={})", h.device, h.llm),
            Err(e) => format!("engine failed: {e}"),
        });
        match started {
            Ok(health) => {
                // cuda = NVIDIA; npu / gpu (Intel graphics) / cpu = what OpenVINO or the processor path runs on.
                let gpu = match health.device.as_str() {
                    "cuda" | "gpu" => "GPU",
                    "npu" => "NPU",
                    _ => "CPU",
                };
                let updated = self.state().updated_to.lock().unwrap().take();
                match updated {
                    Some(v) => self.done(format!("Nabra updated to {v}"), "Ready"),
                    None => self.done(format!("Ready. Hold {} to dictate.", talk_key_label()), gpu),
                }
            }
            Err(e) => self.problem(e),
        }

        loop {
            let t0 = Instant::now();
            // Updates wait while a dictation or voice command is in progress.
            self.state().busy.store(self.take.is_some(), std::sync::atomic::Ordering::SeqCst);
            // A prompt that arrived mid-dictation is shown now (it was never dropped).
            let next = if self.take.is_none() && !self.deferred.is_empty() {
                Ok(self.deferred.remove(0))
            } else {
                inbox.recv_timeout(Duration::from_millis(50))
            };
            match next {
                Ok(c @ (Control::MeetingStarting(_) | Control::UpdateAvailable(_) | Control::Reveal { .. })) if self.take.is_some() => {
                    self.deferred.push(c)
                }
                Ok(Control::Key(Shortcut::TalkCancelled)) if self.take.as_ref().is_some_and(|t| !t.hands_free && !t.command) => {
                    self.take = None; // Right Ctrl + C etc.: a shortcut, not a dictation
                    self.rest();
                }
                Ok(Control::Key(Shortcut::ListenToggle)) => match &self.take {
                    Some(t) if t.listen => self.finish(),
                    Some(_) => {}
                    None => self.begin_take(true, false, true),
                },
                Ok(Control::Key(Shortcut::CatchUp)) if self.take.is_none() => self.catch_up(),
                Ok(Control::Reveal { title, text, note, t }) => self.flash(View::Reveal { title, text, note, t }, REVEAL_FOR),
                // Double-tap Right Ctrl: hands-free. A tap while hands-free types what was said.
                Ok(Control::Key(Shortcut::TalkPressed)) if self.take.is_none() => {
                    let double = self.tapped.take().is_some_and(|t| t.elapsed() < DOUBLE_TAP);
                    self.begin(double, false)
                }
                Ok(Control::Key(Shortcut::TalkPressed)) if self.take.as_ref().is_some_and(|t| t.hands_free && !t.listen && !t.command) => {
                    self.finish()
                }
                Ok(Control::Key(Shortcut::TalkReleased)) if self.take.as_ref().is_some_and(|t| !t.hands_free && !t.command) => {
                    if self.take.as_ref().is_some_and(|t| t.started.elapsed() < TAP) {
                        self.take = None; // a tap, not a dictation: maybe the first of a double-tap
                        self.tapped = Some(Instant::now());
                        self.rest();
                    } else {
                        self.finish()
                    }
                }
                Ok(Control::Key(Shortcut::CommandPressed)) if self.take.is_none() => self.begin(false, true),
                Ok(Control::Key(Shortcut::CommandReleased)) if self.take.as_ref().is_some_and(|t| t.command) => self.finish(),
                Ok(Control::Key(Shortcut::CommandCancelled)) if self.take.as_ref().is_some_and(|t| t.command) => {
                    self.take = None; // AltGr typing, not a command: drop the recording silently
                    self.rest();
                }
                Ok(Control::MicClicked) => {
                    if self.take.is_some() {
                        self.finish()
                    } else {
                        self.begin(true, false)
                    }
                }
                // Not mid-take: it would draw over the Listening view and fight the take for the mic.
                Ok(Control::Key(Shortcut::NotesToggle)) | Ok(Control::NotesClicked) if self.take.is_none() => self.toggle_notes(),
                Ok(Control::MeetingStarting(title)) if self.take.is_none() => {
                    self.flash(View::Meeting { title }, MEETING_PROMPT_FOR)
                }
                Ok(Control::UpdateAvailable(version)) if self.take.is_none() => {
                    self.flash(View::Update { version }, UPDATE_PROMPT_FOR)
                }
                Ok(Control::Updating(label)) => self.flash(View::Working { label }, UPDATING_FOR),
                Ok(Control::UpdateFailed(message)) => self.problem(message),
                // More models arrived after Nabra started (the light model, or the AI model finishing): use them.
                Ok(Control::SetupDone) if self.take.is_none() => {
                    self.render(View::Working { label: "Loading the new model…".into() });
                    match self.start_engine() {
                        Ok(_) => self.done("Ready", "Using the new model"),
                        Err(e) => self.problem(e),
                    }
                }
                Ok(c @ Control::SetupDone) => self.deferred.push(c),
                Ok(Control::Hover(on)) => {
                    self.hovering = on;
                    if self.take.is_none() {
                        if on && !matches!(self.shown, Some(View::Meeting { .. } | View::Update { .. } | View::Slow | View::Working { .. })) {
                            self.hold_until = None; // hovering dismisses a lingering result (not a meeting prompt)
                        }
                        self.rest();
                    }
                }
                Ok(Control::Dismiss) => {
                    self.hold_until = None;
                    self.rest();
                }
                Ok(Control::Refresh) => {
                    if self.take.is_none() {
                        self.rest()
                    }
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => match &self.take {
                    Some(t) if t.started.elapsed() > if t.listen { MAX_LISTEN } else if t.hands_free { MAX_HANDS_FREE } else { MAX_TAKE } => self.finish(),
                    Some(t) if !t.hands_free && t.started.elapsed() > Duration::from_secs(1) && hold_key_up(t.command) => {
                        self.finish()
                    }
                    Some(t) => {
                        let view = View::Listening {
                            seconds: t.started.elapsed().as_secs_f32(),
                            level: t.tap.level(),
                            hands_free: t.hands_free,
                            command: t.command,
                        };
                        pill::show_live(&self.app, &view); // level updates skip the resize
                    }
                    None => self.rest(),
                },
                Err(RecvTimeoutError::Disconnected) => break,
            }
            // Clicks and key presses queued while Nabra was busy (transcribing, restarting the engine) would start
            // an unwanted recording if replayed now: drop them, keep everything else in order.
            if self.take.is_none() && t0.elapsed() > Duration::from_millis(300) {
                while let Ok(c) = inbox.try_recv() {
                    if !matches!(c, Control::MicClicked | Control::Key(Shortcut::TalkPressed | Shortcut::CommandPressed)) {
                        self.deferred.push(c);
                    }
                }
            }
        }
    }
}

/// The hold key is physically up though its release never arrived (Win+L, a UAC prompt, a hook timeout).
/// Swallowed talk keys (Caps Lock…) never show as held to other code, so those can't be checked this way.
fn hold_key_up(command: bool) -> bool {
    let (vk, swallowed) = if command {
        (VK_RMENU.0 as u32, false)
    } else {
        keyboard::TALK_KEYS.iter().find(|k| k.1 == talk_key_label()).map_or((0, true), |k| (k.2, k.3))
    };
    !swallowed && unsafe { GetAsyncKeyState(vk as i32) } >= 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autotune_flags_only_slow_dictations() {
        assert!(too_slow(5.0, Duration::from_secs(4))); // 5 s of speech, 4 s to get text: too slow
        assert!(!too_slow(5.0, Duration::from_secs(2))); // quick enough
        assert!(!too_slow(30.0, Duration::from_secs(8))); // a long dictation may take a while
        assert!(too_slow(1.0, Duration::from_millis(3500)));
    }
}
