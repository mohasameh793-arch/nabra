//! The dictation controller: one thread that owns the microphone while you speak, talks to the engine,
//! types or edits text, records history, and decides what the pill shows.
//!
//!   Right Ctrl (hold) / pill mic (toggle)  → dictate: text is typed where the cursor is
//!   Right Alt (hold)                       → command: "scratch that", "new line", or a transform
//!                                            ("make it shorter") applied to the selection or the
//!                                            last dictation

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_BACK, VK_LEFT};

use crate::keyboard::{self, Shortcut, COMMAND_KEY_LABEL, NOTES_KEY_LABEL, TALK_KEY_LABEL};
use crate::pill::{self, View};
use crate::sound::{lift_quiet, wav, Source, Tap};
use crate::store::{now_ms, Dictation};
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
}

const TOO_SHORT_S: f32 = 0.3;
const RESULT_FOR: Duration = Duration::from_millis(2600);
const PROBLEM_FOR: Duration = Duration::from_secs(5);
const MEETING_PROMPT_FOR: Duration = Duration::from_secs(90);
const UPDATE_PROMPT_FOR: Duration = Duration::from_secs(120);
const UPDATING_FOR: Duration = Duration::from_secs(600);

struct Take {
    tap: Tap,
    started: Instant,
    window: isize,
    app_name: String,
    app_kind: &'static str,
    hands_free: bool,
    command: bool,
}

/// What Nabra typed last, so "scratch that" / "make it shorter" can find it again.
struct Inserted {
    window: isize,
    text: String,
}

pub struct Controller {
    app: AppHandle,
    take: Option<Take>,
    last: Option<Inserted>,
    hovering: bool,
    ready: bool,
    shown: Option<View>,
    hold_until: Option<Instant>, // a result/problem/prompt stays visible until then
}

impl Controller {
    pub fn new(app: AppHandle) -> Self {
        Self { app, take: None, last: None, hovering: false, ready: false, shown: None, hold_until: None }
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
            View::Hover { notes_on: notes.is_some(), talk_key: TALK_KEY_LABEL, notes_key: NOTES_KEY_LABEL }
        } else if let Some(seconds) = notes {
            View::Notes { seconds }
        } else {
            View::Idle
        };
        self.render(view);
    }

    fn begin(&mut self, hands_free: bool, command: bool) {
        if !self.ready {
            return self.problem("Still starting the speech engine…");
        }
        let mic = self.state().settings.lock().unwrap().microphone.clone();
        match Tap::open(Source::Mic, mic.as_deref()) {
            Ok(tap) => {
                let app_name = keyboard::focused_app();
                let app_kind = keyboard::app_kind(&app_name, &keyboard::focused_title());
                self.take = Some(Take {
                    tap,
                    started: Instant::now(),
                    window: keyboard::focused_window(),
                    app_name,
                    app_kind,
                    hands_free,
                    command,
                });
                self.hold_until = None;
                self.render(View::Listening { seconds: 0.0, level: 0.0, hands_free, command });
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
        self.render(View::Working { label: "Transcribing…".into() });
        let (langs, mode, style) = {
            let state = self.state();
            let s = state.settings.lock().unwrap();
            (s.langs(), s.mode.clone(), s.styles.for_kind(take.app_kind).to_string())
        };
        let result = match sidecar::dictate(&audio, &langs, &mode, &style) {
            Ok(r) if r.text.trim().is_empty() => return self.problem("Didn't catch that. Try again."),
            Ok(r) => r,
            Err(e) => return self.problem(e),
        };

        // Type into the app the user started in; if they switched away, use the clipboard instead.
        let detail = if keyboard::focused_window() != take.window {
            let _ = keyboard::copy(&result.text);
            "You switched windows, so it's on your clipboard".to_string()
        } else if let Err(e) = keyboard::type_text(&result.text) {
            let _ = keyboard::copy(&result.text);
            format!("{e}, so it's on your clipboard")
        } else {
            self.last = Some(Inserted { window: take.window, text: result.text.clone() });
            format!("{:.1}s", result.ms as f32 / 1000.0)
        };

        let entry = Dictation {
            id: now_ms(),
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
        if let Err(e) = state.store.add_dictation(&entry) {
            eprintln!("history: {e}");
        }
        let _ = self.app.emit("dictation", &entry);
        self.done(result.text, detail);
    }

    /// Right Alt: a fixed edit ("scratch that", "new line") or a transform of the selection / last dictation.
    fn command(&mut self, take: Take, audio: &[u8]) {
        self.render(View::Working { label: "Listening to your command…".into() });
        let langs = self.state().settings.lock().unwrap().langs();
        let (instruction, action) = match sidecar::instruction(audio, &langs) {
            Ok(r) => r,
            Err(e) => return self.problem(e),
        };
        if keyboard::focused_window() != take.window {
            return self.problem("You switched windows, so the command was skipped");
        }
        let last_here = self.last.as_ref().filter(|l| l.window == take.window).map(|l| l.text.clone());
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
        let target = match keyboard::selected_text() {
            Some(sel) => sel,
            None => match last_here {
                Some(text) => {
                    keyboard::press(VK_LEFT, false, true, keyboard::visible_len(&text)); // select it back
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
                self.last = Some(Inserted { window: take.window, text: out.clone() });
                *self.state().last_text.lock().unwrap() = out.clone();
                self.done(out, "Transformed")
            }
            Ok(_) => self.problem("The AI returned nothing. Your text is unchanged."),
            Err(e) => self.problem(e),
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
        if !crate::assets::ready() {
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
        match launched.and_then(|_| sidecar::wait_ready(Duration::from_secs(240))) {
            Ok(health) => {
                *self.state().engine.lock().unwrap() = Some(health.clone());
                let _ = self.app.emit("engine", &health);
                self.ready = true;
                let gpu = if health.device == "cuda" { "GPU" } else { "CPU" };
                let updated = self.state().updated_to.lock().unwrap().take();
                match updated {
                    Some(v) => self.done(format!("Nabra updated to {v}"), "Ready"),
                    None => self.done(format!("Ready. Hold {TALK_KEY_LABEL} to dictate."), gpu),
                }
            }
            Err(e) => self.problem(e),
        }

        loop {
            // Updates wait while a dictation or voice command is in progress.
            self.state().busy.store(self.take.is_some(), std::sync::atomic::Ordering::SeqCst);
            match inbox.recv_timeout(Duration::from_millis(50)) {
                Ok(Control::Key(Shortcut::TalkPressed)) if self.take.is_none() => self.begin(false, false),
                Ok(Control::Key(Shortcut::TalkReleased)) if self.take.as_ref().is_some_and(|t| !t.hands_free && !t.command) => {
                    self.finish()
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
                Ok(Control::Key(Shortcut::NotesToggle)) | Ok(Control::NotesClicked) => self.toggle_notes(),
                Ok(Control::MeetingStarting(title)) if self.take.is_none() => {
                    self.flash(View::Meeting { title }, MEETING_PROMPT_FOR)
                }
                Ok(Control::UpdateAvailable(version)) if self.take.is_none() => {
                    self.flash(View::Update { version }, UPDATE_PROMPT_FOR)
                }
                Ok(Control::Updating(label)) => self.flash(View::Working { label }, UPDATING_FOR),
                Ok(Control::UpdateFailed(message)) => self.problem(message),
                Ok(Control::Hover(on)) => {
                    self.hovering = on;
                    if self.take.is_none() {
                        if on && !matches!(self.shown, Some(View::Meeting { .. } | View::Update { .. } | View::Working { .. })) {
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
        }
    }
}
