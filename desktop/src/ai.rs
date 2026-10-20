//! The local AI model behind Right Alt commands, cleanup and summaries: one of Nabra's downloadable models (run by
//! llama.cpp), or a model the user already has in Ollama or LM Studio. Settings → AI model picks one; "auto" takes
//! the strongest downloaded model that fits this PC, then Ollama / LM Studio.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

use crate::assets::{self, Part};

pub struct Model {
    pub id: &'static str,
    pub name: &'static str,
    repo: &'static str,
    rev: &'static str,
    file: &'static str,
    /// Download size.
    pub size_mb: u64,
    /// Memory it needs to run (weights + an 8k context).
    need_mb: u64,
}

/// Weakest first. Official Qwen3 GGUF builds, pinned by commit and SHA-256 (assets::PINS).
pub const MODELS: [Model; 4] = [
    Model { id: "qwen3-1.7b", name: "Qwen3 1.7B", repo: "Qwen/Qwen3-1.7B-GGUF", rev: "90862c4b9d2787eaed51d12237eafdfe7c5f6077",
            file: "Qwen3-1.7B-Q8_0.gguf", size_mb: 1_750, need_mb: 2_600 },
    Model { id: "qwen3-4b", name: "Qwen3 4B", repo: "Qwen/Qwen3-4B-GGUF", rev: "bc640142c66e1fdd12af0bd68f40445458f3869b",
            file: "Qwen3-4B-Q4_K_M.gguf", size_mb: 2_380, need_mb: 3_600 },
    Model { id: "qwen3-8b", name: "Qwen3 8B", repo: "Qwen/Qwen3-8B-GGUF", rev: "7c41481f57cb95916b40956ab2f0b139b296d974",
            file: assets::QWEN_FILE, size_mb: 4_800, need_mb: 6_200 },
    Model { id: "qwen3-14b", name: "Qwen3 14B", repo: "Qwen/Qwen3-14B-GGUF", rev: "530227a7d994db8eca5ab5ced2fb692b614357fd",
            file: "Qwen3-14B-Q4_K_M.gguf", size_mb: 8_590, need_mb: 10_600 },
];

pub fn path(m: &Model) -> PathBuf {
    assets::dir().join("models").join(m.file)
}

pub fn installed(m: &Model) -> bool {
    path(m).exists() && Part::Llama.installed()
}

/// Memory a local model may use on this PC, and whether it runs on the NVIDIA GPU (next to Whisper, if that's there).
pub fn budget() -> (u64, bool) {
    match assets::gpu() {
        Some((vram, cap)) if cap >= assets::LLM_MIN_CAP => {
            let whisper = if Part::WhisperGpu.installed() { 4_000 } else { 0 };
            (vram.saturating_sub(whisper + 500), true)
        }
        // ponytail: on the processor anything past ~4B answers too slowly to wait for; tune if CPUs prove faster.
        _ => ((assets::ram_mb() * 2 / 5).min(4_000), false),
    }
}

/// "too_heavy" (won't fit or is too slow here), "best" (the strongest that fits), "light" (fits easily, less smart).
pub fn fit(m: &Model, budget: u64) -> &'static str {
    let best = MODELS.iter().filter(|x| x.need_mb <= budget).map(|x| x.need_mb).max();
    if m.need_mb > budget {
        "too_heavy"
    } else if Some(m.need_mb) == best {
        "best"
    } else {
        "light"
    }
}

/// A model already installed in Ollama or LM Studio (they must be running).
#[derive(Clone, Serialize)]
pub struct External {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub url: String,
    #[serde(skip)]
    pub model: String,
}

fn get(url: &str) -> Option<Value> {
    ureq::get(url).timeout(Duration::from_millis(800)).call().ok()?.into_json().ok()
}

pub fn external() -> Vec<External> {
    let mut out = Vec::new();
    let chat = |name: &str| !name.to_ascii_lowercase().contains("embed"); // embedding models can't answer
    if let Some(v) = get("http://127.0.0.1:11434/api/tags") {
        for m in v["models"].as_array().into_iter().flatten() {
            let Some(name) = m["name"].as_str().filter(|n| chat(n)) else { continue };
            out.push(External { id: format!("ollama:{name}"), name: format!("{name} (Ollama)"),
                                url: "http://127.0.0.1:11434".into(), model: name.into() });
        }
    }
    if let Some(v) = get("http://127.0.0.1:1234/v1/models") {
        for m in v["data"].as_array().into_iter().flatten() {
            let Some(name) = m["id"].as_str().filter(|n| chat(n)) else { continue };
            out.push(External { id: format!("lmstudio:{name}"), name: format!("{name} (LM Studio)"),
                                url: "http://127.0.0.1:1234".into(), model: name.into() });
        }
    }
    out
}

pub enum Pick {
    /// Nabra's own model file, run by llama-server (`gpu`: offload to the NVIDIA card).
    Own { model: PathBuf, gpu: bool },
    External { url: String, model: String },
}

/// The model to run for the user's setting; "auto" (or a pick that's gone) falls back in order.
pub fn choose(setting: &str) -> Option<Pick> {
    let (budget, gpu) = budget();
    let own = |m: &Model| Pick::Own { model: path(m), gpu };
    if let Some(m) = MODELS.iter().find(|m| m.id == setting && installed(m)) {
        return Some(own(m));
    }
    let ext = external();
    if let Some(e) = ext.iter().find(|e| e.id == setting) {
        return Some(Pick::External { url: e.url.clone(), model: e.model.clone() });
    }
    let have: Vec<&Model> = MODELS.iter().filter(|m| installed(m)).collect();
    if let Some(m) = have.iter().rev().find(|m| m.need_mb <= budget).or(have.first()) {
        return Some(own(m));
    }
    ext.into_iter().next().map(|e| Pick::External { url: e.url, model: e.model })
}

/// Valid values for Settings.ai_model.
pub fn valid_setting(s: &str) -> bool {
    s == "auto"
        || MODELS.iter().any(|m| m.id == s)
        || ((s.starts_with("ollama:") || s.starts_with("lmstudio:")) && s.len() <= 200 && !s.chars().any(char::is_control))
}

/// Downloads a model (and llama.cpp, if this PC doesn't have it yet). `progress(done, total)` in bytes.
pub fn download(id: &str, mut progress: impl FnMut(u64, u64)) -> Result<(), String> {
    let m = MODELS.iter().find(|m| m.id == id).ok_or("Unknown model")?;
    let need_llama = !Part::Llama.installed();
    let mut files = if need_llama { assets::resolve(Part::Llama)? } else { Vec::new() };
    files.extend(assets::hf_files(m.repo, m.rev, &[m.file], &assets::dir().join("models"))?);
    let total: u64 = files.iter().map(|f| f.size).sum();
    if assets::free_mb().is_some_and(|free| free * 1_048_576 < total + total / 10) {
        return Err(format!("Not enough free space: {} needs {:.1} GB.", m.name, total as f64 / 1e9));
    }
    let mut before = 0;
    for f in &files {
        assets::fetch(f, |done| progress(before + done, total))?;
        before += f.size;
    }
    if need_llama {
        assets::mark_complete(Part::Llama);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_marks_the_strongest_model_that_fits() {
        let by = |id| MODELS.iter().find(|m| m.id == id).unwrap();
        assert_eq!(fit(by("qwen3-8b"), 7_000), "best");
        assert_eq!(fit(by("qwen3-4b"), 7_000), "light");
        assert_eq!(fit(by("qwen3-14b"), 7_000), "too_heavy");
        assert_eq!(fit(by("qwen3-1.7b"), 1_000), "too_heavy");
    }

    #[test]
    fn settings_accept_only_known_models() {
        assert!(valid_setting("auto") && valid_setting("qwen3-4b") && valid_setting("ollama:llama3.1:8b"));
        assert!(!valid_setting("gpt-5") && !valid_setting("ollama:\n"));
    }
}
