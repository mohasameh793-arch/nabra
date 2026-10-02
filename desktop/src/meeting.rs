//! Call notes. Computer audio (WASAPI loopback) is "them", the microphone is "you". Each side is cut into
//! phrases at pauses, transcribed in order, and streamed to the hub. While someone is still talking, a fast
//! "partial" pass shows their words live (GPU only); the finished phrase then replaces it. On stop the note
//! is saved; the summary is written when the user asks.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::sidecar;
use crate::sound::{wav, Source, Tap};
use crate::store::{now_ms, KnownVoice, Line, Note, Speaker};
use crate::App;

// ponytail: fixed energy gate (~-40 dBFS); switch to an adaptive noise floor if noisy rooms need it.
const SPEECH_RMS: f32 = 0.01;
const MIN_PHRASE_S: f32 = 1.5;
const PAUSE_S: f32 = 0.6;
const MAX_PHRASE_S: f32 = 12.0; // long monologues still finalize regularly (and partials stay short)
/// Live text: re-transcribe the phrase in progress this often.
const PARTIAL_EVERY: Duration = Duration::from_millis(700);
const MIN_PARTIAL_S: f32 = 0.8;
const ECHO_WINDOW_S: f32 = 30.0;

// ponytail: one fixed similarity bar (cosine, WeSpeaker ResNet34). Raise it if two people get merged, lower it
// if one person splits into two; per-call adaptive clustering if fixed bars prove too blunt.
pub const SAME_VOICE: f32 = 0.62;
/// A phrase too short for a voiceprint goes to whoever spoke last on that side, if they spoke this recently.
const SAME_TURN_S: f32 = 8.0;

pub fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

fn similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Tells the voices on the other side apart: named voices from earlier calls first, then voices heard in
/// this call, else a new "Speaker N".
pub struct Voices {
    known: Vec<KnownVoice>,
    last: Option<(String, f32)>, // (speaker id, end time) of the latest phrase
}

impl Voices {
    pub fn new(known: Vec<KnownVoice>) -> Self {
        Self { known, last: None }
    }

    /// Returns the speaker id for a phrase at `t`..`end`; `changed` is set when the speaker list changed.
    pub fn assign(&mut self, speakers: &mut Vec<Speaker>, voice: &[f32], t: f32, end: f32, changed: &mut bool) -> Option<String> {
        if voice.is_empty() {
            let id = self.last.as_ref().filter(|(_, at)| t - at < SAME_TURN_S).map(|(id, _)| id.clone());
            if let Some(id) = &id {
                self.last = Some((id.clone(), end));
            }
            return id;
        }
        let best_known = self
            .known
            .iter()
            .map(|k| (similarity(&k.voice, voice), k))
            .filter(|(s, _)| *s >= SAME_VOICE)
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let best_here = speakers
            .iter()
            .enumerate()
            .map(|(i, s)| (similarity(&s.voice, voice), i))
            .filter(|(s, _)| *s >= SAME_VOICE)
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let index = match (best_known, best_here) {
            // This call's own cluster wins when it's at least as close as the saved voice.
            (_, Some((here, i))) if best_known.map_or(true, |(k, _)| here >= k) => i,
            (Some((_, k)), _) => match speakers.iter().position(|s| s.name.eq_ignore_ascii_case(&k.name)) {
                Some(i) => i,
                None => {
                    speakers.push(Speaker { id: format!("s{}", speakers.len() + 1), name: k.name.clone(), voice: voice.to_vec(), phrases: 0 });
                    *changed = true;
                    speakers.len() - 1
                }
            },
            _ => {
                speakers.push(Speaker { id: format!("s{}", speakers.len() + 1), name: String::new(), voice: voice.to_vec(), phrases: 0 });
                *changed = true;
                speakers.len() - 1
            }
        };
        let s = &mut speakers[index];
        let n = s.phrases as f32;
        s.voice = unit(s.voice.iter().zip(voice).map(|(a, b)| a * n + b).collect());
        s.phrases += 1;
        self.last = Some((s.id.clone(), end));
        Some(s.id.clone())
    }
}

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

    /// The phrase still being spoken, for live text: (samples so far, start time) once there's speech.
    pub fn peek(&self) -> Option<(Vec<f32>, f32)> {
        (self.secs(self.voiced) >= 0.3 && self.secs(self.buf.len()) >= MIN_PARTIAL_S).then(|| (self.buf.clone(), self.start))
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
    pub note: Arc<Mutex<Note>>,
}

impl Meeting {
    /// `title`: the calendar event's name when notes were started from a meeting prompt.
    /// `attendees`: names from the calendar invite; the meeting window is scanned for more in the background.
    pub fn start(
        app: AppHandle,
        langs: String,
        mic: Option<String>,
        title: Option<String>,
        attendees: Vec<String>,
    ) -> Result<Meeting, String> {
        let started_at = now_ms();
        let note = Arc::new(Mutex::new(Note {
            id: format!("note-{started_at}"),
            started_at,
            title: title.unwrap_or_default(),
            attendees,
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (phrase_tx, phrase_rx) = channel::<Job>();
        // Live text costs a GPU decode every PARTIAL_EVERY per speaker: only worth it on a GPU.
        let live = app.state::<App>().engine.lock().unwrap().as_ref().is_some_and(|h| h.device == "cuda");
        let (ready_tx, ready_rx) = channel();

        let capture = {
            let (stop, app) = (stop.clone(), app.clone());
            std::thread::spawn(move || record(app, stop, mic, live, phrase_tx, ready_tx))
        };
        ready_rx.recv().map_err(|_| "Recording didn't start".to_string())??;

        let worker = {
            let note = note.clone();
            let mut voices = Voices::new(app.state::<App>().store.known_voices());
            std::thread::spawn(move || {
                let mut next_id = 0;
                while let Ok(first) = phrase_rx.recv() {
                    // Take everything queued; a partial is stale if a newer job for the same speaker is waiting,
                    // so live text never falls behind and finished lines are never delayed by it.
                    let batch: Vec<Job> = std::iter::once(first).chain(phrase_rx.try_iter()).collect();
                    for (i, job) in batch.iter().enumerate() {
                        if job.partial && batch[i + 1..].iter().any(|j| j.source == job.source) {
                            continue;
                        }
                        handle(&app, &note, &langs, job, &mut next_id, &mut voices);
                    }
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

/// What the windows need to label lines: [{id, name}] plus the attendee names to offer.
pub fn speakers_json(n: &Note) -> serde_json::Value {
    serde_json::json!({
        "id": n.id,
        "speakers": n.speakers.iter().map(|s| serde_json::json!({ "id": s.id, "name": s.name })).collect::<Vec<_>>(),
        "attendees": n.attendees,
    })
}

/// Name a voice (live or saved note) and remember it for future calls. Renaming to "" un-names it.
pub fn name_speaker(app: &AppHandle, note_id: &str, speaker: &str, name: &str) -> Result<serde_json::Value, String> {
    let name = name.trim().chars().take(60).collect::<String>();
    let state = app.state::<App>();
    let rename = |n: &mut Note| -> Option<(Vec<f32>, u32)> {
        let s = n.speakers.iter_mut().find(|s| s.id == speaker)?;
        s.name = name.clone();
        if !name.is_empty() && !n.attendees.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            n.attendees.push(name.clone());
        }
        Some((s.voice.clone(), s.phrases))
    };
    let live = state.meeting.lock().unwrap().as_ref().filter(|m| m.id == note_id).map(|m| m.note.clone());
    let (voice, json) = match live {
        Some(note) => {
            let mut n = note.lock().unwrap();
            let v = rename(&mut n).ok_or("Unknown speaker")?;
            (v, speakers_json(&n))
        }
        None => {
            let mut n = state.store.note(note_id)?;
            let v = rename(&mut n).ok_or("Unknown speaker")?;
            state.store.save_note(&n)?;
            (v, speakers_json(&n))
        }
    };
    if !name.is_empty() && !voice.0.is_empty() {
        state.store.remember_voice(&name, &voice.0, voice.1)?;
    }
    let _ = app.emit("note-speakers", &json);
    Ok(json)
}

/// Add names found after the call started (meeting window scan, or typed by the user).
pub fn add_attendees(app: &AppHandle, note_id: &str, names: Vec<String>) {
    let state = app.state::<App>();
    let live = state.meeting.lock().unwrap().as_ref().filter(|m| m.id == note_id).map(|m| m.note.clone());
    let Some(note) = live else { return };
    let mut n = note.lock().unwrap();
    for name in names {
        let name = name.trim().chars().take(60).collect::<String>();
        if !name.is_empty() && !n.attendees.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            n.attendees.push(name);
        }
    }
    let _ = app.emit("note-speakers", speakers_json(&n));
}

/// Summarize + title a saved note, store it, and tell the hub.
pub fn summarize(app: &AppHandle, id: &str, language: Option<String>) -> Result<Note, String> {
    let state = app.state::<App>();
    let mut note = state.store.note(id)?;
    let language = language.or_else(|| state.settings.lock().unwrap().summary_language.clone());
    let name_of = |l: &Line| l.speaker.as_ref().and_then(|id| note.speakers.iter().find(|s| &s.id == id)).map(|s| s.name.clone());
    let lines: Vec<serde_json::Value> = note
        .lines
        .iter()
        .map(|l| serde_json::json!({ "who": l.who, "text": l.text, "t": l.t, "name": name_of(l).unwrap_or_default() }))
        .collect();
    let lines = serde_json::Value::Array(lines);
    let (summary, title) = sidecar::summarize(&lines, language.as_deref())?;
    note.summary = Some(summary);
    if note.title.is_empty() && !title.is_empty() {
        note.title = title; // a calendar event's name wins over the AI's guess
    }
    state.store.save_note(&note)?;
    let _ = app.emit("note-updated", &note.id);
    Ok(note)
}

/// Audio to transcribe: a finished phrase, or (`partial`) the phrase someone is still saying.
pub struct Job {
    source: Source,
    samples: Vec<f32>,
    rate: u32,
    t: f32,
    partial: bool,
}

fn handle(app: &AppHandle, note: &Mutex<Note>, langs: &str, job: &Job, next_id: &mut u64, voices: &mut Voices) {
    let who = if job.source == Source::System { "them" } else { "you" };
    let text = match sidecar::note_chunk(&wav(&job.samples, job.rate), langs, job.partial) {
        Ok(text) => text,
        Err(e) => {
            if !job.partial {
                let _ = app.emit("notes-problem", e);
            }
            String::new()
        }
    };
    let near = |l: &&Line| (l.t - job.t).abs() < ECHO_WINDOW_S;
    let echo = |n: &Note| who == "you" && n.lines.iter().filter(near).any(|l| l.who == "them" && is_echo(&l.text, &text));
    if job.partial {
        // Live text for the phrase in progress; the hub/meeting window replace it when the line is final.
        if !text.is_empty() && !echo(&note.lock().unwrap()) {
            let _ = app.emit("note-partial", serde_json::json!({ "who": who, "t": job.t, "text": text }));
        }
        return;
    }
    // Who said it (other side only; the mic is always "you"). The voiceprint is computed on the CPU.
    let voice = if who == "them" && !text.is_empty() {
        sidecar::voice(&wav(&job.samples, job.rate)).unwrap_or_default()
    } else {
        Vec::new()
    };
    let line = {
        let mut n = note.lock().unwrap();
        if text.is_empty() || echo(&n) {
            None
        } else {
            if who == "them" {
                let echoes: Vec<u64> =
                    n.lines.iter().filter(near).filter(|l| l.who == "you" && is_echo(&l.text, &text)).map(|l| l.id).collect();
                n.lines.retain(|l| !echoes.contains(&l.id));
                for id in echoes {
                    let _ = app.emit("note-drop", id);
                }
            }
            let speaker = (who == "them").then(|| {
                let end = job.t + job.samples.len() as f32 / job.rate as f32;
                let mut changed = false;
                let id = voices.assign(&mut n.speakers, &voice, job.t, end, &mut changed);
                if changed {
                    let _ = app.emit("note-speakers", speakers_json(&n));
                }
                id
            }).flatten();
            let line = Line { id: *next_id, who: who.into(), text: text.clone(), t: job.t, speaker };
            *next_id += 1;
            n.lines.push(line.clone());
            n.lines.sort_by(|a, b| a.t.total_cmp(&b.t));
            Some(line)
        }
    };
    if let Some(line) = line {
        let _ = app.emit("note-line", line);
    }
    let _ = app.emit("note-partial", serde_json::json!({ "who": who, "t": job.t, "text": "" })); // phrase done
}

fn record(
    app: AppHandle,
    stop: Arc<AtomicBool>,
    mic: Option<String>,
    live: bool,
    out: Sender<Job>,
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
    let mut last_partial = Instant::now();
    let mut pump = |flush: bool| {
        let partial_due = live && !flush && last_partial.elapsed() >= PARTIAL_EVERY;
        if partial_due {
            last_partial = Instant::now();
        }
        for (tap, side) in taps.iter().zip(sides.iter_mut()) {
            side.feed(&tap.take());
            let rate = tap.rate;
            if let Some((samples, t)) = side.next(flush) {
                let _ = out.send(Job { source: side.source, samples, rate, t, partial: false });
            } else if partial_due {
                if let Some((samples, t)) = side.peek() {
                    let _ = out.send(Job { source: side.source, samples, rate, t, partial: true });
                }
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
    fn voices_are_told_apart_and_names_are_recognised() {
        let (zaid, ahmed, other) = (unit(vec![1.0, 0.1, 0.0]), unit(vec![0.0, 1.0, 0.1]), unit(vec![0.1, 0.0, 1.0]));
        let mut v = Voices::new(vec![KnownVoice { name: "Zaid".into(), voice: zaid.clone(), phrases: 3 }]);
        let (mut speakers, mut changed) = (Vec::new(), false);
        assert_eq!(v.assign(&mut speakers, &ahmed, 0.0, 2.0, &mut changed).as_deref(), Some("s1"));
        assert!(changed && speakers[0].name.is_empty(), "a new voice starts unnamed");
        assert_eq!(v.assign(&mut speakers, &zaid, 3.0, 5.0, &mut changed).as_deref(), Some("s2"));
        assert_eq!(speakers[1].name, "Zaid", "a saved voice is recognised by name");
        changed = false;
        assert_eq!(v.assign(&mut speakers, &unit(vec![0.05, 1.0, 0.12]), 6.0, 8.0, &mut changed).as_deref(), Some("s1"));
        assert!(!changed, "the same voice again is the same speaker");
        assert_eq!(v.assign(&mut speakers, &[], 9.0, 9.5, &mut changed).as_deref(), Some("s1"), "short reply: same turn");
        assert_eq!(v.assign(&mut speakers, &other, 10.0, 12.0, &mut changed).as_deref(), Some("s3"));
        assert_eq!(v.assign(&mut speakers, &[], 40.0, 40.5, &mut changed), None, "short phrase long after: unknown");
    }

    #[test]
    fn live_text_only_for_speech_in_progress() {
        let mut p = Phrases::new(Source::Mic, 1000);
        p.feed(&[0.0; 1500]);
        assert!(p.peek().is_none(), "silence has no live text");
        p.feed(&[0.3; 600]);
        let (so_far, at) = p.peek().expect("speaking: live text");
        assert_eq!((so_far.len(), at), (2100, 0.0));
        assert_eq!(p.buf.len(), 2100, "peeking keeps the phrase going");
    }

    #[test]
    fn echo_detection() {
        assert!(is_echo("OK, I'll send you the Stripe keys today.", "ok i'll send you the stripe keys today"));
        assert!(!is_echo("OK, I'll send you the keys.", "تمام، بضيف RTL للـ checkout"));
    }
}
