//! Call notes. Computer audio (WASAPI loopback) is "them", the microphone is "you". Each side is cut into
//! phrases at pauses, transcribed in order, and streamed to the hub. On stop the note is saved and then
//! summarized + titled in the background.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::sidecar;
use crate::sound::{wav, Source, Tap};
use crate::store::{now_ms, Line, Note};
use crate::App;

// ponytail: fixed energy gate (~-40 dBFS); switch to an adaptive noise floor if noisy rooms need it.
const SPEECH_RMS: f32 = 0.01;
const MIN_PHRASE_S: f32 = 1.5;
const PAUSE_S: f32 = 0.6;
const MAX_PHRASE_S: f32 = 20.0;
const ECHO_WINDOW_S: f32 = 30.0;

/// Collects one source and hands out phrases ending at a pause.
pub struct Phrases {
    pub source: Source,
    rate: u32,
    buf: Vec<f32>,
    start: f32, // call time of buf[0], seconds
    voiced: usize,
}

impl Phrases {
    pub fn new(source: Source, rate: u32) -> Self {
        Self { source, rate, buf: Vec::new(), start: 0.0, voiced: 0 }
    }

    fn secs(&self, n: usize) -> f32 {
        n as f32 / self.rate as f32
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    pub fn feed(&mut self, samples: &[f32]) {
        for frame in samples.chunks((self.rate / 50) as usize) {
            if Self::rms(frame) > SPEECH_RMS {
                self.voiced += frame.len();
            }
        }
        self.buf.extend_from_slice(samples);
    }

    /// A finished phrase as (samples, start time), or None. `flush` ends whatever is buffered.
    pub fn next(&mut self, flush: bool) -> Option<(Vec<f32>, f32)> {
        let len = self.secs(self.buf.len());
        let tail = (PAUSE_S * self.rate as f32) as usize;
        let paused = self.buf.len() >= tail && Self::rms(&self.buf[self.buf.len() - tail..]) < SPEECH_RMS;
        if !(flush || len >= MAX_PHRASE_S || (len >= MIN_PHRASE_S && paused)) {
            return None;
        }
        let had_speech = self.secs(self.voiced) >= 0.3;
        let phrase = std::mem::take(&mut self.buf);
        let at = self.start;
        self.start += len;
        self.voiced = 0;
        had_speech.then_some((phrase, at)) // silence never reaches the GPU (and can't make Whisper hallucinate)
    }
}

fn words(s: &str) -> HashSet<String> {
    s.split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
        .filter(|w| !w.is_empty())
        .collect()
}

/// On speakers the mic also hears the other side; a "you" line that mostly repeats a "them" line is an echo.
pub fn is_echo(a: &str, b: &str) -> bool {
    let (a, b) = (words(a), words(b));
    let union = a.union(&b).count();
    union > 0 && a.intersection(&b).count() as f32 / union as f32 >= 0.6
}

pub struct Meeting {
    pub started: Instant,
    pub id: String,
    stop: Arc<AtomicBool>,
    capture: JoinHandle<Result<(), String>>,
    worker: JoinHandle<()>,
    note: Arc<Mutex<Note>>,
}

impl Meeting {
    /// `title`: the calendar event's name when notes were started from a meeting prompt.
    pub fn start(app: AppHandle, langs: String, mic: Option<String>, title: Option<String>) -> Result<Meeting, String> {
        let started_at = now_ms();
        let note = Arc::new(Mutex::new(Note {
            id: format!("note-{started_at}"),
            started_at,
            title: title.unwrap_or_default(),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (phrase_tx, phrase_rx) = channel::<(Source, Vec<f32>, u32, f32)>();
        let (ready_tx, ready_rx) = channel();

        let capture = {
            let (stop, app) = (stop.clone(), app.clone());
            std::thread::spawn(move || record(app, stop, mic, phrase_tx, ready_tx))
        };
        ready_rx.recv().map_err(|_| "Recording didn't start".to_string())??;

        let worker = {
            let note = note.clone();
            std::thread::spawn(move || {
                let mut next_id = 0;
                for (source, samples, rate, t) in phrase_rx {
                    let text = match sidecar::note_chunk(&wav(&samples, rate), &langs) {
                        Ok(text) if !text.is_empty() => text,
                        Ok(_) => continue,
                        Err(e) => {
                            let _ = app.emit("notes-problem", e);
                            continue;
                        }
                    };
                    let who = if source == Source::System { "them" } else { "you" };
                    let mut n = note.lock().unwrap();
                    let near = |l: &&Line| (l.t - t).abs() < ECHO_WINDOW_S;
                    if who == "you" && n.lines.iter().filter(near).any(|l| l.who == "them" && is_echo(&l.text, &text)) {
                        continue;
                    }
                    if who == "them" {
                        let echoes: Vec<u64> =
                            n.lines.iter().filter(near).filter(|l| l.who == "you" && is_echo(&l.text, &text)).map(|l| l.id).collect();
                        n.lines.retain(|l| !echoes.contains(&l.id));
                        for id in echoes {
                            let _ = app.emit("note-drop", id);
                        }
                    }
                    let line = Line { id: next_id, who: who.into(), text, t };
                    next_id += 1;
                    n.lines.push(line.clone());
                    n.lines.sort_by(|a, b| a.t.total_cmp(&b.t));
                    let _ = app.emit("note-line", line);
                }
            })
        };
        let id = note.lock().unwrap().id.clone();
        Ok(Meeting { started: Instant::now(), id, stop, capture, worker, note })
    }

    /// The transcript so far (the hub reopened mid-call).
    pub fn lines(&self) -> Vec<Line> {
        self.note.lock().unwrap().lines.clone()
    }

    /// The live note's "My thoughts" text (saved with the note when it stops).
    pub fn thoughts(&self) -> String {
        self.note.lock().unwrap().thoughts.clone()
    }

    pub fn set_thoughts(&self, text: String) {
        self.note.lock().unwrap().thoughts = text;
    }

    /// Stops recording, finishes the queued phrases, saves. The summary is written only when the user asks.
    pub fn finish(self, app: &AppHandle) -> Result<Note, String> {
        self.stop.store(true, Ordering::SeqCst);
        let recorded = self.capture.join().map_err(|_| "Recording thread crashed".to_string())?;
        let _ = self.worker.join(); // ends once the capture thread drops its sender
        let mut note = self.note.lock().unwrap().clone();
        note.seconds = self.started.elapsed().as_secs_f32();
        let state = app.state::<App>();
        state.store.save_note(&note)?;
        recorded?;
        Ok(note)
    }
}

/// Summarize + title a saved note, store it, and tell the hub.
pub fn summarize(app: &AppHandle, id: &str, language: Option<String>) -> Result<Note, String> {
    let state = app.state::<App>();
    let mut note = state.store.note(id)?;
    let language = language.or_else(|| state.settings.lock().unwrap().summary_language.clone());
    let lines = serde_json::to_value(&note.lines).map_err(|e| e.to_string())?;
    let (summary, title) = sidecar::summarize(&lines, language.as_deref())?;
    note.summary = Some(summary);
    if note.title.is_empty() && !title.is_empty() {
        note.title = title; // a calendar event's name wins over the AI's guess
    }
    state.store.save_note(&note)?;
    let _ = app.emit("note-updated", &note.id);
    Ok(note)
}

fn record(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    mic: Option<String>,
    out: Sender<(Source, Vec<f32>, u32, f32)>,
    ready: Sender<Result<(), String>>,
) -> Result<(), String> {
    // cpal streams aren't Send: they're opened, read and dropped on this thread only.
    let taps = match (Tap::open(Source::System, None), Tap::open(Source::Mic, mic.as_deref())) {
        (Ok(system), Ok(microphone)) => [system, microphone],
        (Err(e), _) | (_, Err(e)) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(()));
    let mut sides = [Phrases::new(Source::System, taps[0].rate), Phrases::new(Source::Mic, taps[1].rate)];
    let mut pump = |flush: bool| {
        for (tap, side) in taps.iter().zip(sides.iter_mut()) {
            side.feed(&tap.take());
            if let Some((samples, t)) = side.next(flush) {
                let _ = out.send((side.source, samples, tap.rate, t));
            }
        }
    };
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
        // Live sound meters for the meeting window: is it hearing the call, and you?
        let _ = app.emit("meeting-level", serde_json::json!({ "them": taps[0].level(), "you": taps[1].level() }));
        pump(false);
    }
    pump(true);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phrases_cut_at_pauses_and_drop_silence() {
        let mut p = Phrases::new(Source::Mic, 1000);
        p.feed(&[0.3; 2000]);
        assert!(p.next(false).is_none(), "still talking");
        p.feed(&[0.0; 700]);
        let (phrase, at) = p.next(false).expect("pause ends the phrase");
        assert_eq!((phrase.len(), at), (2700, 0.0));
        p.feed(&[0.0; 2000]);
        assert!(p.next(false).is_none(), "silence is never sent");
        assert!((p.start - 4.7).abs() < 1e-3);
        p.feed(&[0.3; 500]);
        assert!(p.next(true).is_some(), "flush sends the tail");
    }

    #[test]
    fn echo_detection() {
        assert!(is_echo("OK, I'll send you the Stripe keys today.", "ok i'll send you the stripe keys today"));
        assert!(!is_echo("OK, I'll send you the keys.", "تمام، بضيف RTL للـ checkout"));
    }
}
