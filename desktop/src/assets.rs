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
/// For weak laptops: about 3× faster than turbo on a processor, still good Arabic and English.
const WHISPER_SMALL_REPO: &str = "Systran/faster-whisper-small";
const WHISPER_SMALL_FILES: [&str; 4] = ["config.json", "model.bin", "tokenizer.json", "vocabulary.txt"];
const QWEN_REPO: &str = "Qwen/Qwen3-8B-GGUF";
pub const QWEN_FILE: &str = "Qwen3-8B-Q4_K_M.gguf";
const LLAMA_TAG: &str = "b11200";
const LLAMA_ASSETS: [&str; 2] = ["llama-b11200-bin-win-cuda-13.4-x64.zip", "cudart-llama-bin-win-cuda-13.4-x64.zip"];
const CUDA_WHEELS: [(&str, &str); 2] = [("nvidia-cublas-cu12", "12.9.2.10"), ("nvidia-cudnn-cu12", "9.27.0.42")];
/// The local LLM needs ~6 GB of VRAM next to Whisper's ~4 GB.
const LLM_MIN_VRAM_MB: u64 = 10_000;

/// Exact versions and SHA-256 of everything Setup downloads. A file that doesn't match is refused, so a changed
/// or compromised upstream repo/release can't put different code or models on users' PCs. Update together with
/// the versions above (scripts: query each source once, then paste here).
const WHISPER_GPU_REV: &str = "edaa852ec7e145841d8ffdb056a99866b5f0a478";
const WHISPER_CPU_REV: &str = "0a363e9161cbc7ed1431c9597a8ceaf0c4f78fcf";
const QWEN_REV: &str = "7c41481f57cb95916b40956ab2f0b139b296d974";
const WHISPER_SMALL_REV: &str = "536b0662742c02347bc0e980a01041f333bce120";
const PINS: &[(&str, &str, u64)] = &[
    ("Systran/faster-whisper-large-v3/config.json", "a9306624f5ec14270a014b647e5c316b6e03a662c369758d1b90697a7b0655b9", 2394),
    ("Systran/faster-whisper-large-v3/model.bin", "69f74147e3334731bc3a76048724833325d2ec74642fb52620eda87352e3d4f1", 3087284237),
    ("Systran/faster-whisper-large-v3/preprocessor_config.json", "7ccc62c6f2765af1f3b46c00c9b5894426835a05021c8b9c01eecb6dfb542711", 340),
    ("Systran/faster-whisper-large-v3/tokenizer.json", "6d8cbd7cd0d8d5815e478dac67b85a26bbe77c1f5e0c6d76d1ce2abc0e5f21ca", 2480617),
    ("Systran/faster-whisper-large-v3/vocabulary.json", "c69260f2ab26d659b7c398f9a2b2b48ed0df16c3b47d7326782fd9cba71690c1", 1068114),
    ("dropbox-dash/faster-whisper-large-v3-turbo/config.json", "b0253ea6c0d3bea6b1e19e91a02acfd3b53f4467362efcb5a3e6b16c9b3a9b7e", 2263),
    ("dropbox-dash/faster-whisper-large-v3-turbo/model.bin", "e76620f83d5f5b69efd3d87e3dc180c1bd21df9fbebacfd4335e5e1efcc018da", 1617884929),
    ("dropbox-dash/faster-whisper-large-v3-turbo/preprocessor_config.json", "7ccc62c6f2765af1f3b46c00c9b5894426835a05021c8b9c01eecb6dfb542711", 340),
    ("dropbox-dash/faster-whisper-large-v3-turbo/tokenizer.json", "297b13372ac43916285644fb9687add3cc62ee2a1adb60da3dc25cc94c1871fd", 2710337),
    ("dropbox-dash/faster-whisper-large-v3-turbo/vocabulary.json", "c69260f2ab26d659b7c398f9a2b2b48ed0df16c3b47d7326782fd9cba71690c1", 1068114),
    ("Systran/faster-whisper-small/config.json", "b55496ac7940a7ae47d2c01eab40edfd8701feec1229d9cce3b40014383fb828", 2370),
    ("Systran/faster-whisper-small/model.bin", "3e305921506d8872816023e4c273e75d2419fb89b24da97b4fe7bce14170d671", 483546902),
    ("Systran/faster-whisper-small/tokenizer.json", "fb7b63191e9bb045082c79fd742a3106a12c99513ab30df4a0d47fa6cb6fd0ab", 2203239),
    ("Systran/faster-whisper-small/vocabulary.txt", "34ce3fe1c5041027b3f8d42912270993f986dbc4bb34cf27f951e34a1e453913", 459861),
    ("Qwen/Qwen3-8B-GGUF/Qwen3-8B-Q4_K_M.gguf", "d98cdcbd03e17ce47681435b5150e34c1417f50b5c0019dd560e4882c5745785", 5027783488),
    ("pypi/nvidia-cublas-cu12", "623f43027d40d44ceadf0043f002bd25cf353e8f13ce90b9a87057019f560661", 553162896),
    ("pypi/nvidia-cudnn-cu12", "06e9b0026f3bad97d2b58666330fabec04fe1672f776661ecb0ce0029c27f142", 743068852),
    ("llama/llama-b11200-bin-win-cuda-13.4-x64.zip", "ac88b6102fb9cb6344f897ddfa7400e67ff8d687ad34c124ca5764293ef5ef3f", 152319780),
    ("llama/cudart-llama-bin-win-cuda-13.4-x64.zip", "738f8c251ac22b70c3ae6f83a10cf222725df0395246a2cf58f32bdb85fbe668", 423535356),
];

/// The pinned (sha256, size) for a download, or an error: nothing unpinned is ever downloaded.
fn pin(key: &str) -> Result<(String, u64), String> {
    PINS.iter()
        .find(|p| p.0 == key)
        .map(|p| (p.1.to_string(), p.2))
        .ok_or_else(|| format!("No pinned checksum for {key}"))
}

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
pub fn whisper_small() -> PathBuf {
    dir().join("whisper").join("small")
}
pub fn cuda() -> PathBuf {
    dir().join("cuda")
}
pub fn llama_server() -> PathBuf {
    dir().join("llama").join("llama-server.exe")
}
/// Speaker-recognition model for call notes (who is speaking). Not part of setup: ~26 MB, fetched in the
/// background by `ensure_voice_model`, so existing installs get it without going through setup again.
const VOICE_MODEL_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/speaker-recongition-models/wespeaker_en_voxceleb_resnet34_LM.onnx";
const VOICE_MODEL_SHA256: &str = "e9848563da86f263117134dfd7ad63c92355b37de492b55e325400c9d9c39012";
const VOICE_MODEL_BYTES: u64 = 26_530_550;

pub fn voice_model() -> PathBuf {
    dir().join("voice").join("wespeaker_resnet34.onnx")
}

/// Downloads the voice model if it's missing (verified by SHA-256; resumes a partial download).
pub fn ensure_voice_model() -> Result<(), String> {
    if voice_model().exists() {
        return Ok(());
    }
    let f = Fetch {
        url: VOICE_MODEL_URL.into(),
        dest: voice_model(),
        size: VOICE_MODEL_BYTES,
        sha256: Some(VOICE_MODEL_SHA256.into()),
        unzip: None,
    };
    fetch(&f, |_| {})
}

pub fn qwen() -> PathBuf {
    dir().join("models").join(QWEN_FILE)
}

/// The NVIDIA GPU (via nvidia-smi, which ships with the driver): (memory in MB, compute capability ×10, e.g.
/// 86 for an RTX 30-series), or None without one.
pub fn gpu() -> Option<(u64, u32)> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.total,compute_cap", "--format=csv,noheader,nounits"])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    let line = String::from_utf8_lossy(&out.stdout).lines().next()?.to_string();
    let mut fields = line.split(',').map(str::trim);
    let mb = fields.next()?.parse().ok()?;
    let cap = fields.next().and_then(|c| c.parse::<f32>().ok()).map(|c| (c * 10.0).round() as u32).unwrap_or(0);
    Some((mb, cap))
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Part {
    Cuda,
    WhisperGpu,
    WhisperCpu,
    WhisperSmall,
    Llama,
    Qwen,
}

impl Part {
    pub fn label(self) -> &'static str {
        match self {
            Part::Cuda => "NVIDIA CUDA runtime (cuBLAS + cuDNN)",
            Part::WhisperGpu => "Speech recognition: Whisper large-v3",
            Part::WhisperCpu => "Speech recognition: Whisper large-v3-turbo (CPU)",
            Part::WhisperSmall => "Speech recognition: Whisper small (light, for this laptop)",
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
            Part::WhisperSmall => 490,
            Part::Llama => 580,
            Part::Qwen => 5_030,
        }
    }

    pub fn installed(self) -> bool {
        match self {
            Part::Cuda => cuda().join(".complete").exists(),
            Part::WhisperGpu => whisper_gpu().join(".complete").exists(),
            Part::WhisperCpu => whisper_cpu().join(".complete").exists(),
            Part::WhisperSmall => whisper_small().join(".complete").exists(),
            Part::Llama => llama_server().exists() && dir().join("llama").join(".complete").exists(),
            Part::Qwen => qwen().exists(),
        }
    }
}

/// Large-v3 on the GPU needs a card from 2016 on (compute 6.0+) with enough memory; the llama.cpp CUDA 13 build
/// needs Turing (7.5+, RTX 20-series and newer). Anything less is better served by turbo on the processor than
/// by downloading 4 GB of GPU files that fall back to a slow CPU run.
const GPU_MIN_CAP: u32 = 60;
const GPU_MIN_VRAM_MB: u64 = 3_500;
const LLM_MIN_CAP: u32 = 75;

/// Turbo on the processor needs a strong one to feel instant; below this Nabra uses the small model.
const TURBO_MIN_RAM_MB: u64 = 12_000;
const TURBO_MIN_THREADS: usize = 8;

/// Installed memory in MB.
pub fn ram_mb() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.map(|_| m.ullTotalPhys / 1_048_576).unwrap_or(0)
}

/// The processor's thread count.
pub fn threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
}

/// The speech model for a PC without a usable NVIDIA GPU. A PC that already has turbo keeps it.
fn cpu_tier(ram_mb: u64, threads: usize, has_turbo: bool) -> Part {
    if has_turbo || (ram_mb >= TURBO_MIN_RAM_MB && threads >= TURBO_MIN_THREADS) {
        Part::WhisperCpu
    } else {
        Part::WhisperSmall
    }
}

/// What this PC should have, speech first: GPU → CUDA + large-v3 (+ the LLM when the card allows); a strong
/// processor → turbo; a weak laptop → small.
pub fn plan() -> (Option<u64>, Vec<Part>) {
    let g = gpu();
    let parts = match g {
        Some((mb, cap)) if cap >= GPU_MIN_CAP && mb >= GPU_MIN_VRAM_MB => {
            let mut p = vec![Part::Cuda, Part::WhisperGpu];
            if mb >= LLM_MIN_VRAM_MB && cap >= LLM_MIN_CAP {
                p.extend([Part::Llama, Part::Qwen]);
            }
            p
        }
        _ => vec![cpu_tier(ram_mb(), threads(), Part::WhisperCpu.installed())],
    };
    (g.map(|g| g.0), parts)
}

/// Dictation can start: the speech parts are in (the local AI model may still be downloading).
pub fn speech_ready() -> bool {
    plan().1.iter().filter(|p| p.is_speech()).all(|p| p.installed())
}

impl Part {
    pub fn is_speech(self) -> bool {
        matches!(self, Part::Cuda | Part::WhisperGpu | Part::WhisperCpu | Part::WhisperSmall)
    }
}

/// Free space on the drive that holds Nabra's downloads, in MB.
pub fn free_mb() -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let d = dir();
    let _ = fs::create_dir_all(&d);
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(&HSTRING::from(d.as_os_str()), Some(&mut free), None, None) }.ok()?;
    Some(free / 1_048_576)
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

/// Files from a Hugging Face repo at a fixed commit, each checked against its pinned SHA-256.
fn hf_files(repo: &str, rev: &str, wanted: &[&str], into: &Path) -> Result<Vec<Fetch>, String> {
    wanted
        .iter()
        .map(|name| {
            let (sha256, size) = pin(&format!("{repo}/{name}"))?;
            Ok(Fetch {
                url: format!("https://huggingface.co/{repo}/resolve/{rev}/{name}"),
                dest: into.join(name),
                size,
                sha256: Some(sha256),
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
                let (sha256, size) = pin(&format!("pypi/{name}"))?;
                Ok(Fetch {
                    url: wheel["url"].as_str().unwrap_or_default().into(),
                    dest: cuda().join(format!("{name}.whl")),
                    size,
                    sha256: Some(sha256),
                    unzip: Some(is_dll), // a wheel is a zip; keep only nvidia/*/bin/*.dll
                })
            })
            .collect(),
        Part::WhisperGpu => hf_files(WHISPER_GPU_REPO, WHISPER_GPU_REV, &WHISPER_FILES, &whisper_gpu()),
        Part::WhisperCpu => hf_files(WHISPER_CPU_REPO, WHISPER_CPU_REV, &WHISPER_FILES, &whisper_cpu()),
        Part::WhisperSmall => hf_files(WHISPER_SMALL_REPO, WHISPER_SMALL_REV, &WHISPER_SMALL_FILES, &whisper_small()),
        Part::Qwen => hf_files(QWEN_REPO, QWEN_REV, &[QWEN_FILE], &dir().join("models")),
        Part::Llama => LLAMA_ASSETS
            .iter()
            .map(|name| {
                let (sha256, size) = pin(&format!("llama/{name}"))?;
                Ok(Fetch {
                    url: format!("https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_TAG}/{name}"),
                    dest: dir().join("llama").join(name),
                    size,
                    sha256: Some(sha256),
                    unzip: Some(keep_all),
                })
            })
            .collect(),
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
        Part::WhisperSmall => whisper_small(),
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
    fn weak_laptops_get_the_small_model() {
        assert_eq!(cpu_tier(16_000, 12, false), Part::WhisperCpu); // e.g. a Core Ultra 5 with 16 GB
        assert_eq!(cpu_tier(8_000, 8, false), Part::WhisperSmall); // too little memory
        assert_eq!(cpu_tier(16_000, 4, false), Part::WhisperSmall); // old dual/quad-core
        assert_eq!(cpu_tier(8_000, 4, true), Part::WhisperCpu); // already set up with turbo: keep it
        assert!(PINS.iter().any(|p| p.0 == "Systran/faster-whisper-small/vocabulary.txt"));
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
