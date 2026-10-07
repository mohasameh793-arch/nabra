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
// A slightly longer pause gives Whisper whole thoughts (more context, fewer cut words) instead of fragments.
const PAUSE_S: f32 = 0.8;
const MAX_PHRASE_S: f32 = 12.0; // long monologues still finalize regularly (and partials stay short)
/// Live text: re-transcribe the phrase in progress this often. Each pass redoes the whole phrase and both sides
/// can talk, so faster than this queued finished lines behind live ones (the call notes lagged).
const PARTIAL_EVERY: Duration = Duration::from_millis(1200);
const MIN_PARTIAL_S: f32 = 0.8;
// ponytail: an echo starts within a phrase cut (~2 s) of the line it echoes; a wider window deleted the user
// repeating something back. Compare phrase spans if real echoes still slip through.
const ECHO_WINDOW_S: f32 = 2.5;

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

/// What the meeting app showed, about once a second: (seconds into the call, others who were unmuted/speaking).
pub type Talking = Arc<Mutex<Vec<(f32, Vec<String>)>>>;
/// Votes before a voice gets a name from the meeting app (one stray unmute mustn't name the wrong person).
const NAME_VOTES: u32 = 2;

/// The one other person the meeting app showed as able to talk during most of `t`..`end`, if there was one.
fn sole_talker(samples: &[(f32, Vec<String>)], t: f32, end: f32) -> Option<String> {
    let during: Vec<&Vec<String>> = samples.iter().filter(|(at, _)| *at >= t - 1.0 && *at <= end + 1.0).map(|(_, n)| n).collect();
    let mut counts: std::collections::HashMap<&str, usize> = Default::default();
    for names in &during {
        if let [one] = names.as_slice() {
            *counts.entry(one.as_str()).or_default() += 1;
        }
    }
    counts.into_iter().max_by_key(|(_, c)| *c).filter(|(_, c)| *c * 2 >= during.len().max(1)).map(|(n, _)| n.to_string())
}

/// A name once it has enough votes and clearly leads them.
fn voted_name(votes: &std::collections::HashMap<String, u32>) -> Option<String> {
    let total: u32 = votes.values().sum();
    votes.iter().max_by_key(|(_, v)| **v).filter(|(_, v)| **v >= NAME_VOTES && **v * 10 >= total * 7).map(|(n, _)| n.clone())
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

    /// Loopback sends nothing while the call is silent: keep this side's clock at `now` (seconds since recording
    /// started) so "them" lines aren't stamped early, and a phrase cut off by silence still ends.
    pub fn pad_to(&mut self, now: f32) {
        let behind = now - self.start - self.secs(self.buf.len());
        if behind < 0.3 {
            return;
        }
        if self.buf.is_empty() {
            self.start += behind;
        } else {
            // Enough silence to end the phrase; the rest is skipped once it's sent (no huge buffer after a sleep).
            self.feed(&vec![0.0; (behind.min(PAUSE_S + 0.1) * self.rate as f32) as usize]);
        }
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
    // Short replies ("Okay", "Thank you") are said by both sides all the time: never call those an echo.
    union >= 4 && a.intersection(&b).count() as f32 / union as f32 >= 0.6
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

        // Who the meeting app shows as unmuted/speaking, about once a second (same clock as the recording).
        let talking: Talking = Default::default();
        {
            let (stop, talking, clock) = (stop.clone(), talking.clone(), Instant::now());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    let names = crate::attendees::talking();
                    let mut t = talking.lock().unwrap();
                    t.push((clock.elapsed().as_secs_f32(), names));
                    let excess = t.len().saturating_sub(1800); // the last ~30 minutes is plenty
                    t.drain(..excess);
                    drop(t);
                    std::thread::sleep(Duration::from_millis(900));
                }
            });
        }

        let worker = {
            let note = note.clone();
            let mut voices = Voices::new(app.state::<App>().store.known_voices());
            std::thread::spawn(move || {
                let mut next_id = 0;
                let mut votes: std::collections::HashMap<String, std::collections::HashMap<String, u32>> = Default::default();
                while let Ok(first) = phrase_rx.recv() {
                    // Take everything queued; a partial is stale if a newer job for the same speaker is waiting,
                    // so live text never falls behind and finished lines are never delayed by it.
                    let batch: Vec<Job> = std::iter::once(first).chain(phrase_rx.try_iter()).collect();
                    for (i, job) in batch.iter().enumerate() {
                        if job.partial && batch[i + 1..].iter().any(|j| j.source == job.source) {
                            continue;
                        }
                        handle(&app, &note, &langs, job, &mut next_id, &mut voices, &talking, &mut votes);
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
        let recorded = self.capture.join().unwrap_or_else(|_| Err("Recording thread crashed".to_string()));
        let _ = self.worker.join(); // ends once the capture thread drops its sender
        let mut note = self.note.lock().unwrap().clone();
        note.seconds = self.started.elapsed().as_secs_f32();
        let state = app.state::<App>();
        state.store.save_note(&note)?;
        recorded?;
        Ok(note)
    }
}

fn note_id(note: &Mutex<Note>) -> String {
    note.lock().unwrap().id.clone()
}

/// "Catch me up": the live call's last 5 minutes as 3 bullets.
pub fn catch_up(app: &AppHandle) -> Result<String, String> {
    const WINDOW_S: f32 = 5.0 * 60.0;
    let lines: Vec<serde_json::Value> = {
        let state = app.state::<App>();
        let meeting = state.meeting.lock().unwrap();
        let m = meeting.as_ref().ok_or("Call notes aren't running")?;
        let now = m.started.elapsed().as_secs_f32();
        let n = m.note.lock().unwrap();
        n.lines
            .iter()
            .filter(|l| l.t >= now - WINDOW_S)
            .map(|l| serde_json::json!({ "who": l.who, "text": l.text, "name": speaker_name(&n, l) }))
            .collect()
    };
    if lines.is_empty() {
        return Ok("Nothing was said in the last few minutes.".into());
    }
    let me = app.state::<App>().settings.lock().unwrap().name.clone();
    let text = sidecar::catch_up(&serde_json::Value::Array(lines), &me)?;
    Ok(if text.is_empty() { "Nothing important came up.".into() } else { text })
}

fn speaker_name(n: &Note, l: &Line) -> String {
    l.speaker.as_ref().and_then(|id| n.speakers.iter().find(|s| &s.id == id)).map(|s| s.name.clone()).unwrap_or_default()
}

/// Lowercase, no Arabic diacritics/tatweel, one form of alef/ya/ta marbuta: so "الإطلاق" finds "الاطلاق".
fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| !('\u{064B}'..='\u{0652}').contains(c) && *c != '\u{0640}')
        .map(|c| match c {
            'أ' | 'إ' | 'آ' => 'ا',
            'ى' => 'ي',
            'ة' => 'ه',
            _ => c,
        })
        .flat_map(char::to_lowercase)
        .collect()
}

/// Search words in a question: no stop words, no short words; Arabic "ال" dropped so both forms match.
fn search_words(question: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "what", "who", "when", "why", "how", "did", "does", "the", "about", "say", "said", "meeting", "meetings", "call",
        "calls", "was", "were", "and", "for", "with", "that", "this", "ask", "my", "notes", "last", "week", "today",
        "ايه", "اللي", "ماذا", "عن", "في", "على", "قال", "قالت", "ميتنج", "الميتنج", "اجتماع", "الاجتماع", "مين", "امتى",
    ];
    fold(question)
        .split(|c: char| !c.is_alphanumeric())
        .map(|w| w.strip_prefix("ال").filter(|r| r.chars().count() >= 3).unwrap_or(w).to_string())
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()))
        .collect()
}

/// "Ask my meetings": find the most relevant lines across saved notes, let the local AI answer from them, and
/// return (answer, note to open, time in that note).
pub fn ask(app: &AppHandle, question: &str) -> Result<(String, Option<String>, f32), String> {
    const SNIPPETS: usize = 12;
    let words = search_words(question);
    if words.is_empty() {
        return Err("Ask about something specific, for example: what did Zaid say about the launch?".into());
    }
    let notes = app.state::<App>().store.notes();
    let mut hits: Vec<(usize, &Note, &Line)> = Vec::new();
    for n in &notes {
        for l in &n.lines {
            let haystack = fold(&format!("{} {} {}", l.text, speaker_name(n, l), n.title));
            let score = words.iter().filter(|w| haystack.contains(w.as_str())).count();
            if score > 0 {
                hits.push((score, n, l));
            }
        }
    }
    if hits.is_empty() {
        return Ok(("I couldn't find that in your call notes.".into(), None, 0.0));
    }
    // Best matches first; among equals, the most recent call first.
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.started_at.cmp(&a.1.started_at)));
    hits.truncate(SNIPPETS);
    let date = |ms: u64| chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default();
    let snippets: Vec<serde_json::Value> = hits
        .iter()
        .enumerate()
        .map(|(i, (_, n, l))| {
            let who = if l.who == "you" { "Me".to_string() } else { Some(speaker_name(n, l)).filter(|s| !s.is_empty()).unwrap_or("Them".into()) };
            serde_json::json!({ "n": i + 1, "date": date(n.started_at), "title": n.title, "who": who, "text": l.text })
        })
        .collect();
    let answer = sidecar::ask(question, &serde_json::Value::Array(snippets))?;
    // Of the lines the answer cites, open the best-matching one (snippets are numbered best first).
    let cited = answer.split('[').skip(1).filter_map(|r| r.split(']').next()?.trim().parse::<usize>().ok()).filter(|n| *n >= 1).min();
    let (note, t) = cited
        .and_then(|n| hits.get(n.wrapping_sub(1)))
        .map(|(_, n, l)| (Some(n.id.clone()), l.t))
        .unwrap_or((None, 0.0));
    Ok((answer, note, t))
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
            let mut v = None;
            let n = state.store.update_note(note_id, |n| v = rename(n))?;
            (v.ok_or("Unknown speaker")?, speakers_json(&n))
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
    let before = n.attendees.len();
    for name in names {
        let name = name.trim().chars().take(60).collect::<String>();
        if !name.is_empty() && !n.attendees.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
            n.attendees.push(name);
        }
    }
    if n.attendees.len() != before {
        let _ = app.emit("note-speakers", speakers_json(&n));
    }
}

/// Summarize + title a saved note, store it, and tell the hub.
pub fn summarize(app: &AppHandle, id: &str, language: Option<String>) -> Result<Note, String> {
    let state = app.state::<App>();
    let note = state.store.note(id)?;
    let language = language.or_else(|| state.settings.lock().unwrap().summary_language.clone());
    let name_of = |l: &Line| l.speaker.as_ref().and_then(|id| note.speakers.iter().find(|s| &s.id == id)).map(|s| s.name.clone());
    let lines: Vec<serde_json::Value> = note
        .lines
        .iter()
        .map(|l| serde_json::json!({ "who": l.who, "text": l.text, "t": l.t, "name": name_of(l).unwrap_or_default() }))
        .collect();
    let lines = serde_json::Value::Array(lines);
    let (summary, title) = sidecar::summarize(&lines, language.as_deref())?;
    // Re-read before saving: thoughts or names edited during the (slow) summary must not be overwritten,
    // and a note deleted meanwhile must not come back.
    let note = state.store.update_note(id, |n| {
        n.summary = Some(summary);
        if n.title.is_empty() && !title.is_empty() {
            n.title = title; // a calendar event's name wins over the AI's guess
        }
    })?;
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

#[allow(clippy::too_many_arguments)]
fn handle(
    app: &AppHandle,
    note: &Mutex<Note>,
    langs: &str,
    job: &Job,
    next_id: &mut u64,
    voices: &mut Voices,
    talking: &Talking,
    votes: &mut std::collections::HashMap<String, std::collections::HashMap<String, u32>>,
) {
    let who = if job.source == Source::System { "them" } else { "you" };
    let audio = wav(&job.samples, job.rate);
    // The last finished line before this phrase (either side): names and the topic carry over to it.
    let context = note.lock().unwrap().lines.iter().rev().find(|l| l.t <= job.t).map(|l| l.text.clone()).unwrap_or_default();
    let mut text = sidecar::note_chunk(&audio, langs, job.partial, &context);
    // A finished phrase is never thrown away over a hiccup (engine restarting or busy): try again twice.
    for _ in 0..if job.partial { 0 } else { 2 } {
        if text.is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
        text = sidecar::note_chunk(&audio, langs, false, &context);
    }
    let text = match text {
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
            let _ = app.emit("note-partial", serde_json::json!({ "note": note_id(note), "who": who, "t": job.t, "text": text }));
        }
        return;
    }
    // Who said it (other side only; the mic is always "you"). The voiceprint is computed on the CPU.
    let voice = if who == "them" && !text.is_empty() {
        sidecar::voice(&audio).unwrap_or_default()
    } else {
        Vec::new()
    };
    let end = job.t + job.samples.len() as f32 / job.rate as f32;
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
                    let _ = app.emit("note-drop", serde_json::json!({ "note": n.id, "id": id }));
                }
            }
            let mut learned = None;
            let speaker = (who == "them").then(|| {
                let mut changed = false;
                let id = voices.assign(&mut n.speakers, &voice, job.t, end, &mut changed);
                // Name an unnamed voice after whoever the meeting app showed as the only other person able to talk
                // while it spoke (a name the user typed is never replaced).
                let sole = sole_talker(&talking.lock().unwrap(), job.t, end);
                if let (Some(id), Some(name)) = (&id, sole) {
                    let v = votes.entry(id.clone()).or_default();
                    *v.entry(name).or_default() += 1;
                    let taken = |name: &str| n.speakers.iter().any(|s| &s.id != id && s.name.eq_ignore_ascii_case(name));
                    if let Some(name) = voted_name(v).filter(|name| !taken(name)) {
                        if let Some(s) = n.speakers.iter_mut().find(|s| &s.id == id && s.name.is_empty()) {
                            s.name = name.clone();
                            learned = Some((name.clone(), s.voice.clone(), s.phrases));
                            if !n.attendees.iter().any(|a| a.eq_ignore_ascii_case(&name)) {
                                n.attendees.push(name);
                            }
                            changed = true;
                        }
                    }
                }
                if changed {
                    let _ = app.emit("note-speakers", speakers_json(&n));
                }
                id
            }).flatten();
            if let Some((name, voice, phrases)) = learned {
                // Remember the voice, so the next call knows them even when the meeting window can't be read.
                let _ = app.state::<App>().store.remember_voice(&name, &voice, phrases);
            }
            let line = Line { id: *next_id, who: who.into(), text: text.clone(), t: job.t, speaker };
            *next_id += 1;
            n.lines.push(line.clone());
            n.lines.sort_by(|a, b| a.t.total_cmp(&b.t));
            Some(line)
        }
    };
    if let Some(line) = line {
        let mut payload = serde_json::to_value(&line).unwrap_or_default();
        payload["note"] = note_id(note).into();
        let _ = app.emit("note-line", payload);
        // Autosave after every line: a crash, forced quit or update mid-call keeps everything said so far.
        let mut saved = note.lock().unwrap().clone();
        saved.seconds = saved.seconds.max(end);
        if let Err(e) = app.state::<App>().store.save_note(&saved) {
            eprintln!("note autosave: {e}");
        }
    }
    let _ = app.emit("note-partial", serde_json::json!({ "note": note_id(note), "who": who, "t": job.t, "text": "" })); // phrase done
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
    let mut taps = match (Tap::open(Source::System, None), Tap::open(Source::Mic, mic.as_deref())) {
        (Ok(system), Ok(microphone)) => [system, microphone],
        (Err(e), _) | (_, Err(e)) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(()));
    let mut sides = [Phrases::new(Source::System, taps[0].rate), Phrases::new(Source::Mic, taps[1].rate)];
    let (clock, mut last_partial, mut tick, mut lost) = (Instant::now(), Instant::now(), 0u32, [false; 2]);
    let mut pump = |flush: bool| {
        let partial_due = live && !flush && last_partial.elapsed() >= PARTIAL_EVERY;
        if partial_due {
            last_partial = Instant::now();
        }
        tick += 1;
        if !flush {
            // Live sound meters for the meeting window: is it hearing the call, and you?
            let _ = app.emit("meeting-level", serde_json::json!({ "them": taps[0].level(), "you": taps[1].level() }));
        }
        for (i, (tap, side)) in taps.iter_mut().zip(sides.iter_mut()).enumerate() {
            // Headphones plugged in, Bluetooth dropped, default device switched: follow it (checked once a second).
            if !flush && tick % 5 == 0 && tap.stale() {
                match Tap::open(side.source, if side.source == Source::Mic { mic.as_deref() } else { None }) {
                    Ok(new) => {
                        side.feed(&tap.take());
                        if let Some((samples, t)) = side.next(true) {
                            let _ = out.send(Job { source: side.source, samples, rate: tap.rate, t, partial: false });
                        }
                        *side = Phrases { start: side.start, ..Phrases::new(side.source, new.rate) };
                        *tap = new;
                        lost[i] = false;
                    }
                    Err(e) if !lost[i] => {
                        lost[i] = true;
                        let what = if side.source == Source::Mic { "your microphone" } else { "the call audio" };
                        let _ = app.emit("notes-problem", format!("Lost {what}; still trying to reconnect. {e}"));
                    }
                    Err(_) => {}
                }
            }
            let samples = tap.take();
            if samples.is_empty() && !flush {
                side.pad_to(clock.elapsed().as_secs_f32());
            }
            side.feed(&samples);
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
        p.feed(&[0.0; 900]);
        let (phrase, at) = p.next(false).expect("pause ends the phrase");
        assert_eq!((phrase.len(), at), (2900, 0.0));
        p.feed(&[0.0; 2000]);
        assert!(p.next(false).is_none(), "silence is never sent");
        assert!((p.start - 4.9).abs() < 1e-3);
        p.feed(&[0.3; 500]);
        assert!(p.next(true).is_some(), "flush sends the tail");
    }

    #[test]
    fn silent_loopback_keeps_its_clock() {
        let mut p = Phrases::new(Source::System, 1000);
        p.feed(&[0.3; 2000]);
        p.pad_to(60.0); // loopback went quiet mid-phrase: the phrase still ends
        assert_eq!(p.next(false).map(|(s, at)| (s.len(), at)), Some((2900, 0.0)));
        p.pad_to(60.0); // a long silence is skipped, not buffered
        assert!(p.buf.is_empty() && (p.start - 60.0).abs() < 1e-3);
        p.feed(&[0.3; 500]);
        assert_eq!(p.next(true).map(|(_, at)| at), Some(60.0), "next phrase is stamped at wall time");
    }

    #[test]
    fn names_come_from_who_was_unmuted() {
        let s = |t: f32, names: &[&str]| (t, names.iter().map(|n| n.to_string()).collect::<Vec<_>>());
        // Ahmed alone unmuted while the phrase (10..14 s) was said; Zaid unmuted later with Ahmed.
        let samples = vec![s(9.0, &[]), s(10.0, &["Ahmed"]), s(11.0, &["Ahmed"]), s(12.0, &["Ahmed"]), s(13.0, &["Ahmed"]),
                           s(30.0, &["Ahmed", "Zaid"]), s(31.0, &["Ahmed", "Zaid"])];
        assert_eq!(sole_talker(&samples, 10.0, 14.0).as_deref(), Some("Ahmed"));
        assert_eq!(sole_talker(&samples, 30.0, 31.0), None, "two unmuted: can't tell");
        assert_eq!(sole_talker(&samples, 50.0, 52.0), None, "no screen data");

        let mut votes = std::collections::HashMap::new();
        votes.insert("Ahmed".to_string(), 1);
        assert_eq!(voted_name(&votes), None, "one vote isn't enough");
        votes.insert("Ahmed".to_string(), 3);
        assert_eq!(voted_name(&votes).as_deref(), Some("Ahmed"));
        votes.insert("Zaid".to_string(), 3);
        assert_eq!(voted_name(&votes), None, "split votes: no name");
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
    fn short_replies_are_not_echoes_and_search_ignores_spelling_variants() {
        assert!(!is_echo("Okay.", "okay"), "both sides say okay");
        assert!(is_echo("the launch moves to Monday next week", "the launch moves to Monday"));
        assert_eq!(search_words("What did Zaid say about the launch?"), ["zaid", "launch"]);
        assert!(fold("الإطلاق").contains(&search_words("ايه اللي اتقال عن الاطلاق")[1]));
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
