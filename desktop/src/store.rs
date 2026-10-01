//! Local storage. Everything lives in the app's data folder; nothing leaves the PC.
//!   settings.json    preferences
//!   dictionary.json  personal words + replacements (the engine reads this file directly)
//!   history.jsonl    one line per dictation
//!   notes/<id>.json  one file per call

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string()) // never leaves a half-written file behind
}

fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

// --- settings -------------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    /// Languages the user speaks (ISO 639-1). The engine only picks among these.
    pub languages: Vec<String>,
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: std::env::var("USERNAME").unwrap_or_default(),
            languages: vec!["ar".into(), "en".into()],
            summary_language: None,
            mode: "clean".into(),
            microphone: None,
            keep_clips: false,
            notes_consent: false,
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
        Ok(())
    }

    pub fn langs(&self) -> String {
        self.languages.join(",")
    }
}

// --- dictionary -----------------------------------------------------------------------------

/// A personal word (`term` + optional `sounds_like`) or a replacement (`from` → `to`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Word {
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

// --- history --------------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Line {
    pub id: u64,
    pub who: String, // "them" | "you"
    pub text: String,
    pub t: f32,      // seconds since the call started
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: String,
    pub started_at: u64, // ms
    pub seconds: f32,
    pub title: String,
    pub summary: Option<String>,
    pub lines: Vec<Line>,
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
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn dictionary_path(&self) -> PathBuf {
        self.dir.join("dictionary.json")
    }

    pub fn settings(&self) -> Settings {
        read_json(&self.dir.join("settings.json"))
    }

    pub fn save_settings(&self, s: &Settings) -> Result<(), String> {
        s.validate()?;
        write_atomic(&self.dir.join("settings.json"), &serde_json::to_vec_pretty(s).unwrap())
    }

    pub fn words(&self) -> Vec<Word> {
        read_json(&self.dictionary_path())
    }

    pub fn save_word(&self, mut word: Word) -> Result<Vec<Word>, String> {
        word.validate()?;
        let mut words = self.words();
        if word.id == 0 {
            word.id = now_ms();
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
        let words: Vec<Word> = self.words().into_iter().filter(|w| w.id != id).collect();
        write_atomic(&self.dictionary_path(), &serde_json::to_vec_pretty(&words).unwrap())?;
        Ok(words)
    }

    fn history_path(&self) -> PathBuf {
        self.dir.join("history.jsonl")
    }

    pub fn add_dictation(&self, d: &Dictation) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(self.history_path()).map_err(|e| e.to_string())?;
        writeln!(f, "{}", serde_json::to_string(d).unwrap()).map_err(|e| e.to_string())
    }

    /// Newest first. ponytail: whole-file read; switch to SQLite if histories reach ~100k rows.
    pub fn history(&self) -> Vec<Dictation> {
        let raw = fs::read_to_string(self.history_path()).unwrap_or_default();
        let mut all: Vec<Dictation> = raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        all.reverse();
        all
    }

    fn rewrite_history(&self, items: &[Dictation]) -> Result<(), String> {
        let body: String = items.iter().rev().map(|d| serde_json::to_string(d).unwrap() + "\n").collect();
        write_atomic(&self.history_path(), body.as_bytes())
    }

    pub fn edit_dictation(&self, id: u64, flagged: Option<bool>, delete: bool) -> Result<(), String> {
        let mut items = self.history();
        if delete {
            items.retain(|d| d.id != id);
        } else if let Some(d) = items.iter_mut().find(|d| d.id == id) {
            if let Some(f) = flagged {
                d.flagged = f;
            }
        }
        self.rewrite_history(&items)
    }

    pub fn clear_history(&self) -> Result<(), String> {
        let _ = fs::remove_file(self.history_path());
        Ok(())
    }

    fn note_path(&self, id: &str) -> Result<PathBuf, String> {
        // Ids come back from the UI: accept only our own format so a crafted id can't escape the folder.
        let digits = id.strip_prefix("note-").filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()));
        digits.map(|_| self.dir.join("notes").join(format!("{id}.json"))).ok_or_else(|| "Invalid note id".into())
    }

    pub fn save_note(&self, n: &Note) -> Result<(), String> {
        write_atomic(&self.note_path(&n.id)?, &serde_json::to_vec_pretty(n).unwrap())
    }

    pub fn note(&self, id: &str) -> Result<Note, String> {
        let raw = fs::read_to_string(self.note_path(id)?).map_err(|_| "Note not found".to_string())?;
        serde_json::from_str(&raw).map_err(|e| e.to_string())
    }

    pub fn delete_note(&self, id: &str) -> Result<(), String> {
        fs::remove_file(self.note_path(id)?).map_err(|e| e.to_string())
    }

    pub fn notes(&self) -> Vec<Note> {
        let mut all: Vec<Note> = fs::read_dir(self.dir.join("notes"))
            .into_iter()
            .flatten()
            .flatten()
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
    fn dictionary_round_trip_in_engine_format() {
        let store = temp_store("dict");
        store.save_word(Word { term: Some("Majesty".into()), sounds_like: vec!["ماجستي".into()], ..Default::default() }).unwrap();
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
    fn note_ids_cannot_escape() {
        let store = temp_store("ids");
        assert!(store.note_path("note-123").is_ok());
        for bad in ["note-", "note-1/../../x", "..\\secrets", "note-12a"] {
            assert!(store.note_path(bad).is_err(), "{bad}");
        }
    }
}
