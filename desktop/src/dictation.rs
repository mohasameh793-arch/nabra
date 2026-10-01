//! The dictation controller: one thread that owns the microphone while you dictate, talks to the engine,
//! types the result, records history, and decides what the pill shows.

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::keyboard::{self, Shortcut, NOTES_KEY_LABEL, TALK_KEY_LABEL};
use crate::pill::{self, View};
use crate::sound::{wav, Source, Tap};
use crate::store::{now_ms, Dictation};
use crate::{sidecar, App};

/// Inputs to the controller.
#[derive(Debug)]
pub enum Control {
    Key(Shortcut),
    MicClicked,
    NotesClicked,
    Hover(bool),
    /// Something outside changed (e.g. notes started from the hub): re-render the pill.
    Refresh,
}

const TOO_SHORT_S: f32 = 0.3;
const RESULT_FOR: Duration = Duration::from_millis(2600);
const PROBLEM_FOR: Duration = Duration::from_secs(5);

struct Take {
    tap: Tap,
    started: Instant,
    window: isize,
    app_name: String,
    hands_free: bool,
}

pub struct Controller {
    app: AppHandle,
    take: Option<Take>,
    hovering: bool,
    ready: bool,
    shown: Option<View>,
    hold_until: Option<Instant>, // a result/problem stays visible until then
}

impl Controller {
    pub fn new(app: AppHandle) -> Self {
        Self { app, take: None, hovering: false, ready: false, shown: None, hold_until: None }
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

    fn begin(&mut self, hands_free: bool) {
        if !self.ready {
            return self.problem("Still starting the speech engine…");
        }
        let mic = self.state().settings.lock().unwrap().microphone.clone();
        match Tap::open(Source::Mic, mic.as_deref()) {
            Ok(tap) => {
                self.take = Some(Take {
                    tap,
                    started: Instant::now(),
                    window: keyboard::focused_window(),
                    app_name: keyboard::focused_app(),
                    hands_free,
                });
                self.hold_until = None;
                self.render(View::Listening { seconds: 0.0, level: 0.0, hands_free });
            }
            Err(e) => self.problem(e),
        }
    }

    fn finish(&mut self) {
        let Some(take) = self.take.take() else { return };
        let samples = take.tap.take();
        let rate = take.tap.rate;
        drop(take.tap); // release the mic now
        let seconds = samples.len() as f32 / rate as f32;
        if seconds < TOO_SHORT_S {
            return self.rest();
        }
        self.render(View::Working { label: "Transcribing…".into() });
        let (langs, mode) = {
            let state = self.state();
            let s = state.settings.lock().unwrap();
            (s.langs(), s.mode.clone())
        };
        let result = match sidecar::dictate(&wav(&samples, rate), &langs, &mode) {
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
        self.flash(View::Result { text: result.text, detail }, RESULT_FOR);
    }

    fn toggle_notes(&mut self) {
        let outcome = crate::toggle_meeting(&self.app);
        if let Err(e) = outcome {
            self.problem(e);
        }
        self.rest();
    }

    pub fn run(mut self, inbox: Receiver<Control>) {
        self.render(View::Working { label: "Starting Nabra…".into() });
        let launched = {
            let state = self.state();
            let keep_clips = state.settings.lock().unwrap().keep_clips;
            sidecar::start(sidecar::Launch { dictionary: &state.store.dictionary_path(), keep_clips })
        };
        match launched.and_then(|_| sidecar::wait_ready(Duration::from_secs(240))) {
            Ok(health) => {
                *self.state().engine.lock().unwrap() = Some(health.clone());
                let _ = self.app.emit("engine", &health);
                self.ready = true;
                let gpu = if health.device == "cuda" { "GPU" } else { "CPU" };
                self.flash(View::Result { text: format!("Ready. Hold {TALK_KEY_LABEL} to dictate."), detail: gpu.into() }, RESULT_FOR);
            }
            Err(e) => self.problem(e),
        }

        loop {
            match inbox.recv_timeout(Duration::from_millis(50)) {
                Ok(Control::Key(Shortcut::TalkPressed)) if self.take.is_none() => self.begin(false),
                Ok(Control::Key(Shortcut::TalkReleased)) if self.take.as_ref().is_some_and(|t| !t.hands_free) => {
                    self.finish()
                }
                Ok(Control::MicClicked) => {
                    if self.take.is_some() {
                        self.finish()
                    } else {
                        self.begin(true)
                    }
                }
                Ok(Control::Key(Shortcut::NotesToggle)) | Ok(Control::NotesClicked) => self.toggle_notes(),
                Ok(Control::Hover(on)) => {
                    self.hovering = on;
                    if self.take.is_none() {
                        if on {
                            self.hold_until = None; // hovering dismisses a lingering result
                        }
                        self.rest();
                    }
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
