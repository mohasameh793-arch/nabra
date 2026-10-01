//! First-run setup: downloads the speech and AI models this PC needs into %LOCALAPPDATA%\Nabra\assets.
//!
//! Sources are pinned (exact versions) and every large file is checked against the SHA-256 its source
//! publishes (PyPI, Hugging Face, GitHub). Interrupted downloads resume from where they stopped.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

const WHISPER_GPU_REPO: &str = "Systran/faster-whisper-large-v3";
const WHISPER_CPU_REPO: &str = "dropbox-dash/faster-whisper-large-v3-turbo";
const WHISPER_FILES: [&str; 5] = ["config.json", "model.bin", "preprocessor_config.json", "tokenizer.json", "vocabulary.json"];
const QWEN_REPO: &str = "Qwen/Qwen3-8B-GGUF";
pub const QWEN_FILE: &str = "Qwen3-8B-Q4_K_M.gguf";
const LLAMA_TAG: &str = "b11200";
const LLAMA_ASSETS: [&str; 2] = ["llama-b11200-bin-win-cuda-13.4-x64.zip", "cudart-llama-bin-win-cuda-13.4-x64.zip"];
const CUDA_WHEELS: [(&str, &str); 2] = [("nvidia-cublas-cu12", "12.9.2.10"), ("nvidia-cudnn-cu12", "9.27.0.42")];
/// The local LLM needs ~6 GB of VRAM next to Whisper's ~4 GB.
const LLM_MIN_VRAM_MB: u64 = 10_000;

pub fn dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("Nabra").join("assets")
}

pub fn whisper_gpu() -> PathBuf {
    dir().join("whisper").join("large-v3")
}
pub fn whisper_cpu() -> PathBuf {
    dir().join("whisper").join("large-v3-turbo")
}
pub fn cuda() -> PathBuf {
    dir().join("cuda")
}
pub fn llama_server() -> PathBuf {
    dir().join("llama").join("llama-server.exe")
}
pub fn qwen() -> PathBuf {
    dir().join("models").join(QWEN_FILE)
}

/// NVIDIA GPU memory in MB (via nvidia-smi, which ships with the driver), or None without one.
pub fn gpu_vram_mb() -> Option<u64> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total", "--format=csv,noheader,nounits"])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).lines().filter_map(|l| l.trim().parse::<u64>().ok()).max()
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Cuda,
    WhisperGpu,
    WhisperCpu,
    Llama,
    Qwen,
}

impl Part {
    pub fn label(self) -> &'static str {
        match self {
            Part::Cuda => "NVIDIA CUDA runtime (cuBLAS + cuDNN)",
            Part::WhisperGpu => "Speech recognition: Whisper large-v3",
            Part::WhisperCpu => "Speech recognition: Whisper large-v3-turbo (CPU)",
            Part::Llama => "Local AI server: llama.cpp (CUDA)",
            Part::Qwen => "Local AI model: Qwen3 8B",
        }
    }

    /// Approximate download size, shown before downloading (exact sizes come from the sources).
    pub fn approx_mb(self) -> u64 {
        match self {
            Part::Cuda => 1_300,
            Part::WhisperGpu => 3_100,
            Part::WhisperCpu => 1_600,
            Part::Llama => 580,
            Part::Qwen => 5_030,
        }
    }

    pub fn installed(self) -> bool {
        match self {
            Part::Cuda => cuda().join(".complete").exists(),
            Part::WhisperGpu => whisper_gpu().join(".complete").exists(),
            Part::WhisperCpu => whisper_cpu().join(".complete").exists(),
            Part::Llama => llama_server().exists() && dir().join("llama").join(".complete").exists(),
            Part::Qwen => qwen().exists(),
        }
    }
}

/// What this PC should have: GPU → CUDA + large-v3 (+ the LLM when VRAM allows); otherwise turbo on CPU.
pub fn plan() -> (Option<u64>, Vec<Part>) {
    let vram = gpu_vram_mb();
    let parts = match vram {
        Some(mb) if mb >= LLM_MIN_VRAM_MB => vec![Part::Cuda, Part::WhisperGpu, Part::Llama, Part::Qwen],
        Some(_) => vec![Part::Cuda, Part::WhisperGpu],
        None => vec![Part::WhisperCpu],
    };
    (vram, parts)
}

pub fn ready() -> bool {
    plan().1.iter().all(|p| p.installed())
}

// --- resolving pinned sources ---------------------------------------------------------------

/// One file to fetch. `unzip`: extract the archive into `dest`'s folder, keeping entries that pass the
/// filter (flattened), then delete the archive.
#[derive(Clone, Debug)]
pub struct Fetch {
    pub url: String,
    pub dest: PathBuf,
    pub size: u64,
    pub sha256: Option<String>,
    pub unzip: Option<fn(&str) -> bool>,
}

fn get_json(url: &str) -> Result<Value, String> {
    ureq::get(url)
        .timeout(Duration::from_secs(30))
        .set("User-Agent", "Nabra-setup")
        .call()
        .map_err(|e| format!("Couldn't reach {}: {}", url.split('/').nth(2).unwrap_or(url), e.kind()))?
        .into_json()
        .map_err(|e| e.to_string())
}

fn hf_files(repo: &str, wanted: &[&str], into: &Path) -> Result<Vec<Fetch>, String> {
    let tree = get_json(&format!("https://huggingface.co/api/models/{repo}/tree/main"))?;
    let files = tree.as_array().ok_or("Unexpected reply from Hugging Face")?;
    wanted
        .iter()
        .map(|name| {
            let f = files.iter().find(|f| f["path"] == *name).ok_or(format!("{name} is missing from {repo}"))?;
            Ok(Fetch {
                url: format!("https://huggingface.co/{repo}/resolve/main/{name}"),
                dest: into.join(name),
                size: f["size"].as_u64().unwrap_or(0),
                sha256: f["lfs"]["oid"].as_str().map(String::from), // large files carry their SHA-256
                unzip: None,
            })
        })
        .collect()
}

fn is_dll(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".dll") && name.contains("/bin/")
}

fn keep_all(_: &str) -> bool {
    true
}

pub fn resolve(part: Part) -> Result<Vec<Fetch>, String> {
    match part {
        Part::Cuda => CUDA_WHEELS
            .iter()
            .map(|(name, version)| {
                let meta = get_json(&format!("https://pypi.org/pypi/{name}/{version}/json"))?;
                let wheel = meta["urls"]
                    .as_array()
                    .and_then(|u| u.iter().find(|w| w["filename"].as_str().is_some_and(|f| f.ends_with("win_amd64.whl"))))
                    .ok_or(format!("No Windows package for {name}"))?;
                Ok(Fetch {
                    url: wheel["url"].as_str().unwrap_or_default().into(),
                    dest: cuda().join(format!("{name}.whl")),
                    size: wheel["size"].as_u64().unwrap_or(0),
                    sha256: wheel["digests"]["sha256"].as_str().map(String::from),
                    unzip: Some(is_dll), // a wheel is a zip; keep only nvidia/*/bin/*.dll
                })
            })
            .collect(),
        Part::WhisperGpu => hf_files(WHISPER_GPU_REPO, &WHISPER_FILES, &whisper_gpu()),
        Part::WhisperCpu => hf_files(WHISPER_CPU_REPO, &WHISPER_FILES, &whisper_cpu()),
        Part::Qwen => hf_files(QWEN_REPO, &[QWEN_FILE], &dir().join("models")),
        Part::Llama => {
            let release = get_json(&format!("https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/{LLAMA_TAG}"))?;
            let assets = release["assets"].as_array().ok_or("Unexpected reply from GitHub")?;
            LLAMA_ASSETS
                .iter()
                .map(|name| {
                    let a = assets.iter().find(|a| a["name"] == *name).ok_or(format!("{name} missing from llama.cpp {LLAMA_TAG}"))?;
                    Ok(Fetch {
                        url: a["browser_download_url"].as_str().unwrap_or_default().into(),
                        dest: dir().join("llama").join(name),
                        size: a["size"].as_u64().unwrap_or(0),
                        sha256: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(String::from),
                        unzip: Some(keep_all),
                    })
                })
                .collect()
        }
    }
}

// --- downloading ----------------------------------------------------------------------------

fn sha256_of_file(path: &Path, hasher: &mut Sha256) -> Result<(), String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        hasher.update(&buf[..n]);
    }
}

/// Downloads `f` (resuming a partial `.part` file), verifies size and SHA-256, then unzips if asked.
/// `progress(bytes_this_file)` is called as data arrives.
pub fn fetch(f: &Fetch, mut progress: impl FnMut(u64)) -> Result<(), String> {
    let folder = f.dest.parent().ok_or("bad path")?;
    fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let part = f.dest.with_extension(format!("{}.part", f.dest.extension().and_then(|e| e.to_str()).unwrap_or("")));
    let mut hasher = Sha256::new();
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if f.size > 0 && have > f.size {
        let _ = fs::remove_file(&part);
        have = 0;
    }
    if have > 0 {
        sha256_of_file(&part, &mut hasher)?;
    }
    if f.size == 0 || have < f.size {
        // No overall timeout (a 5 GB file takes a while); fail only if the connection stalls for 60 s.
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(20)).timeout_read(Duration::from_secs(60)).build();
        let mut req = agent.get(&f.url).set("User-Agent", "Nabra-setup");
        if have > 0 {
            req = req.set("Range", &format!("bytes={have}-"));
        }
        let resp = req.call().map_err(|e| format!("Download failed ({})", e.kind()))?;
        if have > 0 && resp.status() != 206 {
            // Server ignored the range: start over.
            have = 0;
            hasher = Sha256::new();
        }
        let mut out = OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(&part).map_err(|e| e.to_string())?;
        let mut body = resp.into_reader();
        let mut buf = vec![0u8; 1 << 20];
        let mut done = have;
        progress(done);
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("Download interrupted: {e}"))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| format!("Couldn't write the file (disk full?): {e}"))?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(done);
        }
        have = done;
    }
    if f.size > 0 && have != f.size {
        return Err(format!("{} is incomplete ({have} of {} bytes). Try again to resume.", f.dest.display(), f.size));
    }
    if let Some(want) = &f.sha256 {
        let got = format!("{:x}", hasher.finalize());
        if !got.eq_ignore_ascii_case(want) {
            let _ = fs::remove_file(&part);
            return Err(format!("{} failed its integrity check and was deleted. Try again.", f.dest.file_name().unwrap().to_string_lossy()));
        }
    }
    match f.unzip {
        Some(keep) => {
            extract(&part, folder, keep)?;
            let _ = fs::remove_file(&part);
        }
        None => fs::rename(&part, &f.dest).map_err(|e| e.to_string())?,
    }
    Ok(())
}

/// Extracts archive entries that pass `keep` into `into` (flattened to file names; no path tricks possible).
fn extract(archive: &Path, into: &Path, keep: fn(&str) -> bool) -> Result<(), String> {
    let mut zip = zip::ZipArchive::new(File::open(archive).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if entry.is_dir() || !keep(&name) {
            continue;
        }
        let Some(file_name) = Path::new(&name).file_name() else { continue };
        let mut out = File::create(into.join(file_name)).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Marks a part complete (its folder gets a `.complete` file once every fetch succeeded).
pub fn mark_complete(part: Part) {
    let folder = match part {
        Part::Cuda => cuda(),
        Part::WhisperGpu => whisper_gpu(),
        Part::WhisperCpu => whisper_cpu(),
        Part::Llama => dir().join("llama"),
        Part::Qwen => return, // a single file: its presence is the marker
    };
    let _ = fs::write(folder.join(".complete"), b"ok");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_flattens_and_filters() {
        let tmp = std::env::temp_dir().join(format!("nabra-zip-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let archive = tmp.join("t.zip");
        {
            let mut w = zip::ZipWriter::new(File::create(&archive).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for (name, body) in [("nvidia/cublas/bin/cublas64_12.dll", "a"), ("nvidia/cublas/include/x.h", "b"), ("../../evil.dll", "c")] {
                w.start_file(name, opts).unwrap();
                w.write_all(body.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        extract(&archive, &tmp, is_dll).unwrap();
        assert!(tmp.join("cublas64_12.dll").exists());
        assert!(!tmp.join("x.h").exists());
        assert!(!tmp.parent().unwrap().parent().unwrap().join("evil.dll").exists()); // no escaping the folder
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn local_file_download_verifies_hash() {
        // A tiny file served from GitHub would need network; instead check the hash helper on a temp file.
        let tmp = std::env::temp_dir().join(format!("nabra-hash-{}.bin", std::process::id()));
        fs::write(&tmp, b"abc").unwrap();
        let mut h = Sha256::new();
        sha256_of_file(&tmp, &mut h).unwrap();
        assert_eq!(format!("{:x}", h.finalize()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let _ = fs::remove_file(&tmp);
    }
}
