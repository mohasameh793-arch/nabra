//! The engine (Python) and the local LLM (llama-server) run as hidden child processes inside a Windows
//! job object, so they die with Nabra even if it crashes. The app talks to the engine over loopback HTTP.

use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

const ENGINE: &str = "http://127.0.0.1:8770";
const LLM_PORT: u16 = 8771;
const NO_WINDOW: u32 = 0x0800_0000;
/// Without a GPU the engine works the processor hard: keep the PC responsive by letting other apps go first.
const BELOW_NORMAL_PRIORITY: u32 = 0x0000_4000;

/// A secret made fresh at every launch and given only to our own engine and llama-server (environment /
/// --api-key). Every request carries it, so no other program or web page can use them, and the engine proves
/// it knows it on /health, so another program squatting on the port is never trusted.
fn token() -> &'static str {
    static TOKEN: OnceLock<String> = OnceLock::new();
    TOKEN.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        // RandomState is seeded from the OS's secure random source; four of them give 256 random bits.
        (0..4).map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            format!("{:016x}", h.finish())
        }).collect()
    })
}

/// HMAC-SHA256(token, message) as hex: what our engine answers on /health.
fn proof(message: &[u8]) -> String {
    hmac_hex(token().as_bytes(), message)
}

/// HMAC-SHA256 (keys up to 64 bytes, which is all we use) as lowercase hex.
fn hmac_hex(secret: &[u8], message: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut key = [0u8; 64];
    key[..secret.len().min(64)].copy_from_slice(&secret[..secret.len().min(64)]);
    let pad = |b: u8| key.iter().map(|k| k ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(message).finalize();
    let outer = Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize();
    outer.iter().map(|b| format!("{b:02x}")).collect()
}

fn post(path: &str) -> ureq::Request {
    ureq::post(&format!("{ENGINE}{path}")).set("X-Nabra-Token", token())
}

/// Source checkout root (the repo this binary was built from), or NABRA_ROOT. Only used when the app runs
/// from source; an installed app uses its bundled engine.
pub fn root() -> PathBuf {
    std::env::var_os("NABRA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf())
}

pub fn logs_dir() -> PathBuf {
    crate::assets::dir().parent().map(|p| p.join("logs")).unwrap_or_else(|| PathBuf::from("logs"))
}

/// How to run the engine: the bundled `nabra-engine.exe` (installer), else Python from the source checkout.
/// Returns (program, leading args, working dir).
pub fn engine_command(app: &tauri::AppHandle) -> Result<(PathBuf, Vec<String>, PathBuf), String> {
    use tauri::Manager;
    if let Ok(res) = app.path().resource_dir() {
        let exe = res.join("engine").join("nabra-engine.exe");
        if exe.exists() {
            return Ok((exe, vec![], res.join("engine")));
        }
    }
    let root = root();
    let python = root.join(".venv").join("Scripts").join("python.exe");
    if python.exists() {
        return Ok((python, vec![root.join("engine").display().to_string()], root));
    }
    Err("The speech engine isn't installed. Reinstall Nabra, or run scripts\\setup.ps1 in a source checkout.".into())
}

fn job() -> Result<HANDLE, String> {
    unsafe {
        let job = CreateJobObjectW(None, None).map_err(|e| e.to_string())?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            size_of_val(&limits) as u32,
        )
        .map_err(|e| e.to_string())?;
        Ok(job) // kept open for the app's lifetime on purpose
    }
}

fn launch(job: HANDLE, exe: &Path, args: &[String], cwd: &Path, log: &Path, flags: u32) -> Result<(), String> {
    let out = std::fs::File::create(log).map_err(|e| e.to_string())?;
    let child = Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .env("PYTHONIOENCODING", "utf-8")
        .env("NABRA_TOKEN", token())
        .env("NABRA_LLM_KEY", token())
        .stdin(Stdio::null())
        .stdout(out.try_clone().map_err(|e| e.to_string())?)
        .stderr(out)
        .creation_flags(NO_WINDOW | flags)
        .spawn()
        .map_err(|e| format!("Couldn't start {}: {e}", exe.display()))?;
    unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) }.map_err(|e| e.to_string())?;
    std::mem::forget(child); // the job object owns its lifetime
    Ok(())
}

pub struct Launch<'a> {
    pub dictionary: &'a Path,
    pub snippets: &'a Path,
    pub clips: &'a Path,
    pub keep_clips: bool,
}

pub fn start(app: &tauri::AppHandle, opts: Launch) -> Result<(), String> {
    use crate::assets::{self, Part};
    let logs = logs_dir();
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let job = job()?;
    let (program, mut args, cwd) = engine_command(app)?;
    args.extend([
        "--port".into(),
        "8770".into(),
        "--dictionary".into(),
        opts.dictionary.display().to_string(),
        "--snippets".into(),
        opts.snippets.display().to_string(),
    ]);
    // Speech model chosen by first-run setup: large-v3 on an NVIDIA GPU, turbo on a strong processor, small on a
    // weak laptop.
    if Part::WhisperGpu.installed() {
        args.extend(["--whisper".into(), assets::whisper_gpu().display().to_string()]);
    } else if Part::WhisperCpu.installed() {
        args.extend(["--whisper".into(), assets::whisper_cpu().display().to_string()]);
    } else if Part::WhisperSmall.installed() {
        args.extend(["--whisper".into(), assets::whisper_small().display().to_string()]);
    }
    if Part::Cuda.installed() {
        args.extend(["--cuda-dir".into(), assets::cuda().display().to_string()]);
    }
    if Part::Llama.installed() && Part::Qwen.installed() {
        let llm = ["-m", &assets::qwen().display().to_string(), "-ngl", "99", "-c", "8192", "--host", "127.0.0.1",
            "--port", &LLM_PORT.to_string(), "--jinja", "--api-key", token()];
        let llama = assets::llama_server();
        launch(job, &llama, &llm.map(String::from), llama.parent().unwrap(), &logs.join("llama.log"), 0)?;
        args.extend(["--llm-url".into(), format!("http://127.0.0.1:{LLM_PORT}")]);
    }
    if opts.keep_clips {
        args.extend(["--keep-clips".into(), opts.clips.display().to_string()]);
    }
    // Downloaded in the background on first run (see assets::ensure_voice_model); loaded on first use.
    args.extend(["--voice-model".into(), assets::voice_model().display().to_string()]);
    let flags = if Part::WhisperGpu.installed() { 0 } else { BELOW_NORMAL_PRIORITY };
    launch(job, &program, &args, &cwd, &logs.join("engine.log"), flags)
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Health {
    pub device: String,
    pub llm: bool,
    #[serde(default, skip_serializing)]
    proof: String,
}

/// Our engine's health, or None if nothing answers, or if whatever answers can't prove it's ours.
pub fn health() -> Option<Health> {
    let h: Health = ureq::get(&format!("{ENGINE}/health"))
        .set("X-Nabra-Token", token())
        .timeout(Duration::from_secs(2))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    (h.proof == proof(b"nabra-health")).then_some(h)
}

pub fn wait_ready(limit: Duration) -> Result<Health, String> {
    let t0 = Instant::now();
    loop {
        if let Some(h) = health() {
            return Ok(h);
        }
        if t0.elapsed() > limit {
            return Err(format!("The speech engine didn't start. Details: {}", logs_dir().join("engine.log").display()));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn explain(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(503, _) => "The local AI model isn't running.".into(),
        ureq::Error::Status(code, _) => {
            format!("The speech engine hit an error ({code}). Details: {}", logs_dir().join("engine.log").display())
        }
        ureq::Error::Transport(_) => "The speech engine isn't running.".into(),
    }
}

#[derive(Debug, Deserialize)]
pub struct Dictated {
    pub text: String,
    pub language: Option<String>,
    #[serde(default)]
    pub fixes: crate::store::Fixes,
}

/// `langs`, `mode` and `style` come from validated settings (ASCII words, commas, underscores only).
pub fn dictate(wav: &[u8], langs: &str, mode: &str, style: &str) -> Result<Dictated, String> {
    post(&format!("/dictate?langs={langs}&mode={mode}&style={style}"))
        .timeout(Duration::from_secs(120))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())
}

/// A spoken command (Right Alt): what was said, and the engine's fixed action or "transform".
pub fn instruction(wav: &[u8], langs: &str) -> Result<(String, String), String> {
    let v: Value = post(&format!("/instruction?langs={langs}"))
        .timeout(Duration::from_secs(60))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok((v["instruction"].as_str().unwrap_or_default().into(), v["action"].as_str().unwrap_or("none").into()))
}

pub fn transform(text: &str, instruction: &str) -> Result<String, String> {
    let v: Value = post(&format!("/transform"))
        .timeout(Duration::from_secs(180))
        .send_json(json!({ "text": text, "instruction": instruction }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().into())
}

/// `partial`: live text for a phrase that's still being spoken (fast pass).
pub fn note_chunk(wav: &[u8], langs: &str, partial: bool) -> Result<String, String> {
    let v: Value = post(&format!("/note?langs={langs}&partial={}", partial as u8))
        .timeout(Duration::from_secs(120))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
}

/// The phrase's voiceprint ([] if too short or the voice model isn't downloaded).
pub fn voice(wav: &[u8]) -> Result<Vec<f32>, String> {
    let v: Value = post(&format!("/voice"))
        .timeout(Duration::from_secs(30))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["voice"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).map(|x| x as f32).collect()).unwrap_or_default())
}

/// "Catch me up": the last minutes of a call as 3 bullets.
pub fn catch_up(lines: &Value, me: &str) -> Result<String, String> {
    let v: Value = post("/catchup")
        .timeout(Duration::from_secs(120))
        .send_json(json!({ "lines": lines, "me": me }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
}

/// "Ask my meetings": an answer from the given transcript snippets, citing them as [n].
pub fn ask(question: &str, snippets: &Value) -> Result<String, String> {
    let v: Value = post("/ask")
        .timeout(Duration::from_secs(120))
        .send_json(json!({ "question": question, "snippets": snippets }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
}

pub fn summarize(lines: &Value, language: Option<&str>) -> Result<(String, String), String> {
    let v: Value = post(&format!("/summary"))
        .timeout(Duration::from_secs(600))
        .send_json(json!({ "lines": lines, "language": language }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok((v["summary"].as_str().unwrap_or_default().into(), v["title"].as_str().unwrap_or_default().into()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn health_proof_matches_pythons_hmac() {
        // python -c "import hmac,hashlib;print(hmac.new(b'0123456789abcdef'*4,b'nabra-health',hashlib.sha256).hexdigest())"
        assert_eq!(super::hmac_hex(b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", b"nabra-health"), "06b6e043551204ee015c1e2f62044612e057524966452fa6906eedeae1c8e773");
        assert_eq!(super::token().len(), 64);
    }
}
