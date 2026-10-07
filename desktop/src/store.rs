//! Local storage. Everything lives in the app's data folder; nothing leaves the PC.
//!   settings.json    preferences
//!   dictionary.json  personal words + replacements (the engine reads this file directly)
//!   history.jsonl    one line per dictation
//!   notes/<id>.json  one file per call

use std::fs;
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A time-based id that never repeats, even for two items made in the same millisecond.
pub fn new_id() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = now_ms();
    LAST.fetch_max(now, Ordering::SeqCst); // jump to the clock, then take the next free value
    LAST.fetch_add(1, Ordering::SeqCst)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // Own tmp name per write, so two overlapping saves never share (or steal) one file.
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = path.with_file_name(format!("{name}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let write = || -> std::io::Result<()> {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all() // on disk before the rename, so a power cut can't leave an empty file
    };
    if let Err(e) = write() {
        let _ = fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    // Windows refuses to replace a file someone (the engine, an antivirus) has open for a moment: retry briefly.
    let mut tries = 0;
    loop {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()), // never leaves a half-written file behind
            Err(_) if tries < 10 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e.to_string());
            }
        }
    }
}

/// Missing file = empty/default. A file that can't be read or parsed is an error (and a corrupt one is copied
/// to `<name>.corrupt`), so callers never save over data they couldn't load.
fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(format!("Couldn't read {name}: {e}")),
    };
    serde_json::from_str(&raw).map_err(|e| {
        let _ = fs::copy(path, path.with_file_name(format!("{name}.corrupt")));
        format!("{name} is damaged ({e}); a copy was kept as {name}.corrupt")
    })
}

/// For display only: whatever could be loaded, else empty. Never use this before a write.
fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    load_json(path).unwrap_or_else(|e| {
        eprintln!("store: {e}");
        T::default()
    })
}

// --- settings -------------------------------------------------------------------------------

pub const DIALECTS: [&str; 5] = ["auto", "gulf", "egyptian", "levantine", "msa"];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    /// Languages the user speaks (ISO 639-1). The engine only picks among these.
    pub languages: Vec<String>,
    /// Arabic dialect lock: "auto" | "gulf" | "egyptian" | "levantine" | "msa" (steers recognition and cleanup).
    pub dialect: String,
    /// Call summary language; None = the call's main language.
    pub summary_language: Option<String>,
    /// "clean" (lexicon + AI cleanup) or "raw" (speech-to-text only).
    pub mode: String,
    /// Microphone device name; None = Windows default.
    pub microphone: Option<String>,
    /// Opt-in: save dictation audio for the accuracy benchmark (applies on next launch).
    pub keep_clips: bool,
    /// The user has seen "tell people you're transcribing".
    pub notes_consent: bool,
    /// Writing style per kind of app.
    pub styles: Styles,
    /// Ask to take notes when a calendar meeting starts.
    pub meeting_prompts: bool,
    /// Download and install signed updates from GitHub automatically.
    pub auto_update: bool,
    /// Where the pill sits: "right" | "left" | "bottom" (drag it to change).
    pub pill_dock: String,
    /// The push-to-talk key (see keyboard::TALK_KEYS).
    pub talk_key: String,
    /// How long dictation history is kept: "forever", "30" / "7" (days) or "off" (not saved at all).
    pub history_keep: String,
    /// Version that ran last time (to say "Nabra updated to …" once after an update).
    pub last_version: String,
}

/// "formal" | "casual" | "very_casual" for each kind of app (see keyboard::app_kind).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Styles {
    pub personal: String,
    pub work: String,
    pub email: String,
    pub other: String,
}

impl Default for Styles {
    fn default() -> Self {
        Self { personal: "casual".into(), work: "casual".into(), email: "formal".into(), other: "formal".into() }
    }
}

impl Styles {
    pub fn for_kind(&self, kind: &str) -> &str {
        match kind {
            "personal" => &self.personal,
            "work" => &self.work,
            "email" => &self.email,
            _ => &self.other,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: std::env::var("USERNAME").unwrap_or_default(),
            languages: vec!["ar".into(), "en".into()],
            dialect: "auto".into(),
            summary_language: None,
            mode: "clean".into(),
            microphone: None,
            keep_clips: false,
            notes_consent: false,
            styles: Styles::default(),
            meeting_prompts: true,
            auto_update: true,
            pill_dock: "right".into(),
            talk_key: "right_ctrl".into(),
            history_keep: "forever".into(),
            last_version: String::new(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        let code = |c: &str| (2..=3).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_lowercase());
        if self.languages.is_empty() || !self.languages.iter().all(|c| code(c)) {
            return Err("Choose at least one language".into());
        }
        if self.summary_language.as_deref().is_some_and(|c| !code(c)) {
            return Err("Unknown summary language".into());
        }
        if self.mode != "clean" && self.mode != "raw" {
            return Err("Unknown mode".into());
        }
        if !DIALECTS.contains(&self.dialect.as_str()) {
            return Err("Unknown dialect".into());
        }
        let s = &self.styles;
        if ![&s.personal, &s.work, &s.email, &s.other].iter().all(|v| ["formal", "casual", "very_casual"].contains(&v.as_str())) {
            return Err("Unknown style".into());
        }
        if !keep_ok(&self.history_keep) {
            return Err("Unknown history setting".into());
        }
        if !crate::keyboard::TALK_KEYS.iter().any(|k| k.0 == self.talk_key) {
            return Err("Unknown talk key".into());
        }
        if !["right", "left", "bottom"].contains(&self.pill_dock.as_str()) {
            return Err("Unknown pill position".into());
        }
        Ok(())
    }

    /// Values an older/newer build may have stored that this one can't use go back to safe defaults
    /// (so every later save doesn't fail validation).
    fn repair(mut self) -> Self {
        let d = Settings::default();
        if !keep_ok(&self.history_keep) {
            self.history_keep = d.history_keep; // "forever": never delete because of a value we don't understand
        }
        if !crate::keyboard::TALK_KEYS.iter().any(|k| k.0 == self.talk_key) {
            self.talk_key = d.talk_key;
        }
        if !["right", "left", "bottom"].contains(&self.pill_dock.as_str()) {
            self.pill_dock = d.pill_dock;
        }
        if !DIALECTS.contains(&self.dialect.as_str()) {
            self.dialect = d.dialect; // settings saved before the dialect lock existed have ""
        }
        self
    }

    pub fn langs(&self) -> String {
        self.languages.join(",")
    }
}

/// "forever", "off", or a number of days 1..=3650.
fn keep_ok(keep: &str) -> bool {
    keep == "forever" || keep == "off" || keep.parse::<u64>().is_ok_and(|d| (1..=3650).contains(&d))
}

// --- dictionary -----------------------------------------------------------------------------

/// A personal word (`term` + optional `sounds_like`) or a replacement (`from` → `to`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Word {
    #[serde(default)] // a hand-written entry without an id must not make the whole file unreadable
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds_like: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

impl Word {
    pub fn validate(&self) -> Result<(), String> {
        let filled = |s: &Option<String>| s.as_deref().is_some_and(|v| !v.trim().is_empty() && v.len() <= 200);
        match (filled(&self.term), filled(&self.from) && filled(&self.to)) {
            (true, false) | (false, true) => Ok(()),
            _ => Err("Enter a word, or both sides of a replacement".into()),
        }
    }
}

// --- snippets & scratchpad ------------------------------------------------------------------

/// Say `trigger`, get `text` (the engine reads snippets.json directly).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snippet {
    pub id: u64,
    pub trigger: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Pad {
    pub id: u64,
    pub body: String,
    pub updated: u64,
}

/// Insert-or-update by id (0 = new) in a JSON list file; returns the new list.
fn upsert<T: Clone + Serialize + for<'de> Deserialize<'de>>(
    path: &Path,
    mut item: T,
    id_of: impl Fn(&T) -> u64,
    set_id: impl Fn(&mut T, u64),
) -> Result<Vec<T>, String> {
    let mut items: Vec<T> = load_json(path)?;
    if id_of(&item) == 0 {
        set_id(&mut item, new_id());
        items.insert(0, item);
    } else if let Some(slot) = items.iter_mut().find(|x| id_of(x) == id_of(&item)) {
        *slot = item;
    } else {
        return Err("That item no longer exists".into());
    }
    write_atomic(path, &serde_json::to_vec_pretty(&items).unwrap())?;
    Ok(items)
}

fn remove<T: Serialize + for<'de> Deserialize<'de>>(path: &Path, id: u64, id_of: impl Fn(&T) -> u64) -> Result<Vec<T>, String> {
    let items: Vec<T> = load_json::<Vec<T>>(path)?.into_iter().filter(|x| id_of(x) != id).collect();
    write_atomic(path, &serde_json::to_vec_pretty(&items).unwrap())?;
    Ok(items)
}

// --- history --------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Fixes {
    pub terms: u32,
    pub dictionary: u32,
    pub ai: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Dictation {
    pub id: u64, // = time in ms
    pub app: String,
    pub text: String,
    pub language: Option<String>,
    pub words: u32,
    pub seconds: f32,
    pub fixes: Fixes,
    pub flagged: bool,
}

// --- notes ----------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Line {
    pub id: u64,
    pub who: String, // "them" | "you"
    pub text: String,
    pub t: f32,      // seconds since the call started
    /// Which voice on the other side said it ("s1", "s2"…); the name lives in `Note::speakers`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
}

/// A voice heard on the other side of a call. `voice` is its voiceprint (numbers, never audio).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Speaker {
    pub id: String,
    pub name: String, // "" until named
    pub voice: Vec<f32>,
    pub phrases: u32,
    /// The name is Nabra's guess (voice memory or the meeting window), not typed by the user: newer evidence from
    /// the meeting window may replace it.
    pub guessed: bool,
}

/// A voice the user has named, recognised automatically in later calls.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KnownVoice {
    pub name: String,
    pub voice: Vec<f32>,
    pub phrases: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub started_at: u64, // ms
    pub seconds: f32,
    pub title: String,
    pub summary: Option<String>,
    /// The user's own notes typed during the meeting ("My thoughts").
    pub thoughts: String,
    pub lines: Vec<Line>,
    pub speakers: Vec<Speaker>,
    /// Names found when the call started (calendar invite, meeting window), offered when naming speakers.
    pub attendees: Vec<String>,
}

#[derive(Serialize)]
pub struct NoteCard {
    pub id: String,
    pub started_at: u64,
    pub seconds: f32,
    pub title: String,
    pub has_summary: bool,
}

// --- the store ------------------------------------------------------------------------------

pub struct Store {
    pub dir: PathBuf,
    /// Held across every read-modify-write (and the history append), so two saves never lose each other's change.
    /// ponytail: one lock for all files; per-file locks if saves ever queue up noticeably.
    lock: Mutex<()>,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, lock: Mutex::new(()) }
    }

    fn guard(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn dictionary_path(&self) -> PathBuf {
        self.dir.join("dictionary.json")
    }

    pub fn settings(&self) -> Settings {
        read_json::<Settings>(&self.dir.join("settings.json")).repair() // a damaged file is kept as settings.json.corrupt
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        s.validate()?;
        let _g = self.guard();
        let path = self.dir.join("settings.json");
        // Only replace a file we could read (a locked one would have loaded as defaults).
        match fs::read(&path) {
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(format!("Couldn't read settings.json: {e}")),
            _ => {}
        }
        write_atomic(&path, &serde_json::to_vec_pretty(s).unwrap())
    }

    pub fn words(&self) -> Vec<Word> {
        read_json(&self.dictionary_path())
    }

    pub fn save_word(&self, mut word: Word) -> Result<Vec<Word>, String> {
        word.validate()?;
        let _g = self.guard();
        let mut words: Vec<Word> = load_json(&self.dictionary_path())?;
        if word.id == 0 {
            word.id = new_id();
            words.insert(0, word);
        } else if let Some(slot) = words.iter_mut().find(|w| w.id == word.id) {
            *slot = word;
        } else {
            return Err("That word no longer exists".into());
        }
        write_atomic(&self.dictionary_path(), &serde_json::to_vec_pretty(&words).unwrap())?;
        Ok(words)
    }

    pub fn delete_word(&self, id: u64) -> Result<Vec<Word>, String> {
        let _g = self.guard();
        let words: Vec<Word> = load_json::<Vec<Word>>(&self.dictionary_path())?.into_iter().filter(|w| w.id != id).collect();
        write_atomic(&self.dictionary_path(), &serde_json::to_vec_pretty(&words).unwrap())?;
        Ok(words)
    }

    fn voices_path(&self) -> PathBuf {
        self.dir.join("voices.json")
    }

    /// Voices the user has named in earlier calls.
    pub fn known_voices(&self) -> Vec<KnownVoice> {
        self.load_voices().unwrap_or_else(|e| {
            eprintln!("store: {e}");
            Vec::new()
        })
    }

    /// Names saved before 2.1.17 can carry the browser's invisible direction marks; drop them so they match.
    fn load_voices(&self) -> Result<Vec<KnownVoice>, String> {
        let mut all: Vec<KnownVoice> = load_json(&self.voices_path())?;
        for k in &mut all {
            k.name = crate::attendees::strip_bidi(&k.name).trim().to_string();
        }
        Ok(all)
    }

    /// Remember (or refine) `name`'s voice. Averaging over calls makes recognition steadier.
    pub fn remember_voice(&self, name: &str, voice: &[f32], phrases: u32) -> Result<(), String> {
        let _g = self.guard();
        let mut all = self.load_voices()?;
        match all.iter_mut().find(|k| k.name.eq_ignore_ascii_case(name)) {
            Some(k) if k.voice.len() == voice.len() => {
                let (a, b) = (k.phrases.max(1) as f32, phrases.max(1) as f32);
                k.voice = crate::meeting::unit(k.voice.iter().zip(voice).map(|(x, y)| x * a + y * b).collect());
                k.phrases += phrases;
            }
            Some(k) => *k = KnownVoice { name: name.into(), voice: voice.to_vec(), phrases },
            None => all.push(KnownVoice { name: name.into(), voice: voice.to_vec(), phrases }),
        }
        write_atomic(&self.voices_path(), &serde_json::to_vec(&all).unwrap())
    }

    /// Forget one remembered voice (it was proven wrong: the meeting window showed someone else talking).
    pub fn forget_voice(&self, name: &str) -> Result<(), String> {
        let _g = self.guard();
        let mut all = self.load_voices()?;
        let before = all.len();
        all.retain(|k| !k.name.eq_ignore_ascii_case(name));
        if all.len() == before {
            return Ok(());
        }
        write_atomic(&self.voices_path(), &serde_json::to_vec(&all).unwrap())
    }

    /// Forget every voiceprint (Settings → Privacy): the named voices, and the voices kept in each saved note
    /// for naming later. Names and transcripts stay.
    pub fn forget_voices(&self) -> Result<(), String> {
        let _g = self.guard();
        match fs::remove_file(self.voices_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
        for mut note in self.notes() {
            if note.speakers.iter().any(|s| !s.voice.is_empty()) {
                note.speakers.iter_mut().for_each(|s| s.voice.clear());
                write_atomic(&self.note_path(&note.id)?, &serde_json::to_vec_pretty(&note).unwrap())?;
            }
        }
        Ok(())
    }

    pub fn snippets_path(&self) -> PathBuf {
        self.dir.join("snippets.json")
    }

    pub fn snippets(&self) -> Vec<Snippet> {
        read_json(&self.snippets_path())
    }

    pub fn save_snippet(&self, s: Snippet) -> Result<Vec<Snippet>, String> {
        let (t, x) = (s.trigger.trim(), s.text.trim());
        if t.is_empty() || x.is_empty() || t.len() > 100 || x.len() > 20_000 {
            return Err("A snippet needs a short trigger phrase and some text".into());
        }
        let _g = self.guard();
        if load_json::<Vec<Snippet>>(&self.snippets_path())?.iter().any(|o| o.id != s.id && o.trigger.trim().eq_ignore_ascii_case(t)) {
            return Err("Another snippet already uses that trigger".into());
        }
        upsert(&self.snippets_path(), s, |s| s.id, |s, id| s.id = id)
    }

    pub fn delete_snippet(&self, id: u64) -> Result<Vec<Snippet>, String> {
        let _g = self.guard();
        remove(&self.snippets_path(), id, |s: &Snippet| s.id)
    }

    fn pads_path(&self) -> PathBuf {
        self.dir.join("scratchpad.json")
    }

    pub fn pads(&self) -> Vec<Pad> {
        read_json(&self.pads_path())
    }

    pub fn save_pad(&self, mut p: Pad) -> Result<Vec<Pad>, String> {
        p.updated = now_ms();
        let _g = self.guard();
        upsert(&self.pads_path(), p, |p| p.id, |p, id| p.id = id)
    }

    pub fn delete_pad(&self, id: u64) -> Result<Vec<Pad>, String> {
        let _g = self.guard();
        remove(&self.pads_path(), id, |p: &Pad| p.id)
    }

    fn history_path(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    pub fn add_dictation(&self, d: &Dictation) -> Result<(), String> {
        let _g = self.guard();
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let append = || -> std::io::Result<()> {
            let mut f = fs::OpenOptions::new().create(true).read(true).append(true).open(self.history_path())?;
            // A crash mid-append leaves a line without its newline: end it, so this entry isn't glued onto it.
            let mut last = [b'\n'];
            if f.metadata()?.len() > 0 {
                f.seek(SeekFrom::End(-1))?;
                f.read_exact(&mut last)?;
            }
            let sep = if last[0] == b'\n' { "" } else { "\n" };
            writeln!(f, "{sep}{}", serde_json::to_string(d).unwrap())
        };
        append().map_err(|e| e.to_string())
    }

    /// Newest first. ponytail: whole-file read; switch to SQLite if histories reach ~100k rows.
    pub fn history(&self) -> Vec<Dictation> {
        self.load_history().unwrap_or_else(|e| {
            eprintln!("store: {e}");
            Vec::new()
        })
    }

    /// Like `history`, but an unreadable file is an error, so it's never rewritten from nothing.
    fn load_history(&self) -> Result<Vec<Dictation>, String> {
        let raw = match fs::read_to_string(self.history_path()) {
            Ok(raw) => raw,
            Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("Couldn't read history.jsonl: {e}")),
        };
        let mut all: Vec<Dictation> = raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        all.reverse();
        Ok(all)
    }

    fn rewrite_history(&self, items: &[Dictation]) -> Result<(), String> {
        let body: String = items.iter().rev().map(|d| serde_json::to_string(d).unwrap() + "\n").collect();
        write_atomic(&self.history_path(), body.as_bytes())
    }

    pub fn edit_dictation(&self, id: u64, flagged: Option<bool>, delete: bool) -> Result<(), String> {
        let _g = self.guard();
        let mut items = self.load_history()?;
        if delete {
            items.retain(|d| d.id != id);
        } else if let Some(d) = items.iter_mut().find(|d| d.id == id) {
            if let Some(f) = flagged {
                d.flagged = f;
            }
        }
        self.rewrite_history(&items)
    }

    /// Apply Settings → Privacy → "Keep dictation history": drop entries older than `keep` days ("off" = all).
    pub fn prune_history(&self, keep: &str) -> Result<(), String> {
        let days: u64 = match keep {
            "off" => 0,
            d => match d.parse() {
                Ok(n @ 1..=3650) => n,
                _ => return Ok(()), // "forever", or a value we don't understand: never delete on a guess
            },
        };
        let cutoff = now_ms().saturating_sub(days * 86_400_000);
        let _g = self.guard();
        let all = self.load_history()?;
        let kept: Vec<Dictation> = all.iter().filter(|d| days > 0 && d.id >= cutoff).cloned().collect();
        if kept.len() != all.len() {
            self.rewrite_history(&kept)?;
        }
        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), String> {
        let _g = self.guard();
        let _ = fs::remove_file(self.history_path());
        Ok(())
    }

    fn note_path(&self, id: &str) -> Result<PathBuf, String> {
        // Ids come back from the UI: accept only our own format so a crafted id can't escape the folder.
        let digits = id.strip_prefix("note-").filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        digits.map(|_| self.dir.join("notes").join(format!("{id}.json"))).ok_or_else(|| "Invalid note id".into())
    }

    pub fn save_note(&self, n: &Note) -> Result<(), String> {
        let _g = self.guard();
        write_atomic(&self.note_path(&n.id)?, &serde_json::to_vec_pretty(n).unwrap())
    }

    pub fn note(&self, id: &str) -> Result<Note, String> {
        let raw = fs::read_to_string(self.note_path(id)?).map_err(|_| "Note not found".to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    /// Change a saved note in place: re-read under the lock, so edits made meanwhile (thoughts, speaker names,
    /// a summary) aren't overwritten by an older copy, and a deleted note isn't brought back.
    pub fn update_note(&self, id: &str, change: impl FnOnce(&mut Note)) -> Result<Note, String> {
        let _g = self.guard();
        let mut note = self.note(id)?;
        change(&mut note);
        write_atomic(&self.note_path(id)?, &serde_json::to_vec_pretty(&note).unwrap())?;
        Ok(note)
    }

    pub fn delete_note(&self, id: &str) -> Result<(), String> {
        let _g = self.guard();
        fs::remove_file(self.note_path(id)?).map_err(|e| e.to_string())
    }

    pub fn notes(&self) -> Vec<Note> {
        let mut all: Vec<Note> = fs::read_dir(self.dir.join("notes"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json")) // not .tmp / .corrupt leftovers
            .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
            .collect();
        all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        all
    }

    pub fn note_cards(&self) -> Vec<NoteCard> {
        self.notes()
            .into_iter()
            .map(|n| NoteCard {
                has_summary: n.summary.is_some(),
                title: if n.title.is_empty() {
                    n.lines.first().map(|l| l.text.chars().take(48).collect()).unwrap_or_else(|| "Untitled call".into())
                } else {
                    n.title
                },
                id: n.id,
                started_at: n.started_at,
                seconds: n.seconds,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("nabra-test-{tag}-{}", now_ms()));
        Store::new(dir)
    }

    #[test]
    fn settings_defaults_and_validation() {
        let s: Settings = serde_json::from_str(r#"{"languages":["fr"]}"#).unwrap();
        assert_eq!(s.langs(), "fr");
        assert_eq!(s.mode, "clean");
        assert!(Settings { languages: vec![], ..Settings::default() }.validate().is_err());
        assert!(Settings { languages: vec!["../x".into()], ..Settings::default() }.validate().is_err());
    }

    #[test]
    fn voice_names_lose_direction_marks() {
        let store = temp_store("voices");
        fs::create_dir_all(&store.dir).unwrap();
        fs::write(store.voices_path(), "[{\"name\":\"\\u202a\\u202aheba nassar\\u202c\\u200f\",\"voice\":[1.0],\"phrases\":3}]").unwrap();
        assert_eq!(store.known_voices()[0].name, "heba nassar");
        store.remember_voice("heba nassar", &[1.0], 1).unwrap(); // refines the same voice, no duplicate
        assert_eq!(store.known_voices().len(), 1);
        store.forget_voice("Heba Nassar").unwrap();
        assert!(store.known_voices().is_empty());
    }

    #[test]
    fn dictionary_round_trip_in_engine_format() {
        let store = temp_store("dict");
        store.save_word(Word { term: Some("Layla".into()), sounds_like: vec!["ليلى".into()], ..Default::default() }).unwrap();
        let words = store.save_word(Word { from: Some("btw".into()), to: Some("by the way".into()), ..Default::default() }).unwrap();
        assert_eq!(words.len(), 2);
        let raw = fs::read_to_string(store.dictionary_path()).unwrap();
        assert!(raw.contains("\"sounds_like\"") && raw.contains("\"from\": \"btw\"")); // what engine/lexicon.py reads
        assert!(store.save_word(Word::default()).is_err());
        assert_eq!(store.delete_word(words[0].id).unwrap().len(), 1);
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn history_newest_first_flag_delete() {
        let store = temp_store("hist");
        for id in [1, 2, 3] {
            store.add_dictation(&Dictation { id, text: format!("t{id}"), ..Default::default() }).unwrap();
        }
        assert_eq!(store.history().iter().map(|d| d.id).collect::<Vec<_>>(), [3, 2, 1]);
        store.edit_dictation(2, Some(true), false).unwrap();
        store.edit_dictation(3, None, true).unwrap();
        let h = store.history();
        assert_eq!((h.len(), h[0].id, h[0].flagged), (2, 2, true));
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn snippets_validate_and_upsert() {
        let store = temp_store("snip");
        let list = store.save_snippet(Snippet { trigger: "my email".into(), text: "a@b.c".into(), ..Default::default() }).unwrap();
        assert!(store.save_snippet(Snippet { trigger: "My Email".into(), text: "x".into(), ..Default::default() }).is_err());
        let mut s = list[0].clone();
        s.text = "new@b.c".into();
        assert_eq!(store.save_snippet(s).unwrap()[0].text, "new@b.c");
        assert!(store.delete_snippet(list[0].id).unwrap().is_empty());
        let pads = store.save_pad(Pad { body: "draft".into(), ..Default::default() }).unwrap();
        assert!(pads[0].id > 0 && pads[0].updated > 0);
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn styles_validate() {
        let mut s = Settings::default();
        assert_eq!(s.styles.for_kind("email"), "formal");
        s.styles.personal = "shouty".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn corrupt_file_is_never_overwritten() {
        let store = temp_store("corrupt");
        fs::create_dir_all(&store.dir).unwrap();
        let bad = r#"[{"id": 1, "term": "Layla"}, {"id": "#; // half-written
        fs::write(store.dictionary_path(), bad).unwrap();
        assert!(store.words().is_empty()); // display: nothing
        assert!(store.save_word(Word { term: Some("x".into()), ..Default::default() }).is_err());
        assert!(store.delete_word(1).is_err());
        assert_eq!(fs::read_to_string(store.dictionary_path()).unwrap(), bad); // untouched
        assert!(store.dir.join("dictionary.json.corrupt").exists());
        // An entry without an id no longer makes the whole file unreadable.
        fs::write(store.dictionary_path(), r#"[{"term": "Layla"}]"#).unwrap();
        assert_eq!(store.save_word(Word { term: Some("x".into()), ..Default::default() }).unwrap().len(), 2);
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn torn_history_line_and_bad_keep_values() {
        let store = temp_store("torn");
        fs::create_dir_all(&store.dir).unwrap();
        fs::write(store.dir.join("history.jsonl"), "{\"id\":1,\"text\":\"a\"}\n{\"id\":2,\"te").unwrap();
        store.add_dictation(&Dictation { id: 3, text: "b".into(), ..Default::default() }).unwrap();
        assert_eq!(store.history().iter().map(|d| d.id).collect::<Vec<_>>(), [3, 1]);
        for keep in ["0", "-1", "7d", "99999999999999"] {
            store.prune_history(keep).unwrap();
            assert!(Settings { history_keep: keep.into(), ..Settings::default() }.validate().is_err(), "{keep}");
        }
        assert_eq!(store.history().len(), 2);
        assert!(new_id() != new_id());
        let _ = fs::remove_dir_all(&store.dir);
    }

    #[test]
    fn note_ids_cannot_escape() {
        let store = temp_store("ids");
        assert!(store.note_path("note-123").is_ok());
        for bad in ["note-", "note-1/../../x", "..\\secrets", "note-12a"] {
            assert!(store.note_path(bad).is_err(), "{bad}");
        }
    }
}
