//! The engine (Python) and the local LLM (llama-server) run as hidden child processes inside a Windows
//! job object, so they die with Nabra even if it crashes. The app talks to the engine over loopback HTTP.

use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

/// Project root: NABRA_ROOT, else the repo this binary was built from (dev layout).
pub fn root() -> PathBuf {
    std::env::var_os("NABRA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf())
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

fn launch(job: HANDLE, exe: &Path, args: &[String], log: &Path) -> Result<(), String> {
    let out = std::fs::File::create(log).map_err(|e| e.to_string())?;
    let child = Command::new(exe)
        .args(args)
        .current_dir(root())
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(out.try_clone().map_err(|e| e.to_string())?)
        .stderr(out)
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Couldn't start {}: {e}", exe.display()))?;
    unsafe { AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())) }.map_err(|e| e.to_string())?;
    std::mem::forget(child); // the job object owns its lifetime
    Ok(())
}

pub struct Launch<'a> {
    pub dictionary: &'a Path,
    pub snippets: &'a Path,
    pub keep_clips: bool,
}

pub fn start(opts: Launch) -> Result<(), String> {
    let root = root();
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).map_err(|e| e.to_string())?;
    let job = job()?;

    let python = root.join(".venv/Scripts/python.exe");
    if !python.exists() {
        return Err("Python environment missing. Run scripts\\setup.ps1 first.".into());
    }
    let mut engine = vec![
        root.join("engine").display().to_string(),
        "--port".into(),
        "8770".into(),
        "--dictionary".into(),
        opts.dictionary.display().to_string(),
        "--snippets".into(),
        opts.snippets.display().to_string(),
    ];
    let (llama, model) = (root.join(".assets/llama/llama-server.exe"), root.join(".assets/models/Qwen3-8B-Q4_K_M.gguf"));
    if llama.exists() && model.exists() {
        let args = ["-m", &model.display().to_string(), "-ngl", "99", "-c", "8192", "--host", "127.0.0.1", "--port",
            &LLM_PORT.to_string(), "--jinja"];
        launch(job, &llama, &args.map(String::from), &logs.join("llama.log"))?;
        engine.extend(["--llm-url".into(), format!("http://127.0.0.1:{LLM_PORT}")]);
    }
    if opts.keep_clips {
        engine.extend(["--keep-clips".into(), root.join("bench/real").display().to_string()]);
    }
    launch(job, &python, &engine, &logs.join("engine.log"))
}

#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Health {
    pub device: String,
    pub llm: bool,
}

pub fn health() -> Option<Health> {
    ureq::get(&format!("{ENGINE}/health")).timeout(Duration::from_secs(2)).call().ok()?.into_json().ok()
}

pub fn wait_ready(limit: Duration) -> Result<Health, String> {
    let t0 = Instant::now();
    loop {
        if let Some(h) = health() {
            return Ok(h);
        }
        if t0.elapsed() > limit {
            return Err("The speech engine didn't start. Details: logs\\engine.log".into());
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

fn explain(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(503, _) => "The local AI model isn't running.".into(),
        ureq::Error::Status(code, _) => format!("The speech engine hit an error ({code}). Details: logs\\engine.log"),
        ureq::Error::Transport(_) => "The speech engine isn't running.".into(),
    }
}

#[derive(Debug, Deserialize)]
pub struct Dictated {
    pub text: String,
    pub language: Option<String>,
    pub ms: u64,
    #[serde(default)]
    pub fixes: crate::store::Fixes,
}

/// `langs`, `mode` and `style` come from validated settings (ASCII words, commas, underscores only).
pub fn dictate(wav: &[u8], langs: &str, mode: &str, style: &str) -> Result<Dictated, String> {
    ureq::post(&format!("{ENGINE}/dictate?langs={langs}&mode={mode}&style={style}"))
        .timeout(Duration::from_secs(120))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())
}

/// A spoken command (Right Alt): what was said, and the engine's fixed action or "transform".
pub fn instruction(wav: &[u8], langs: &str) -> Result<(String, String), String> {
    let v: Value = ureq::post(&format!("{ENGINE}/instruction?langs={langs}"))
        .timeout(Duration::from_secs(60))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok((v["instruction"].as_str().unwrap_or_default().into(), v["action"].as_str().unwrap_or("none").into()))
}

pub fn transform(text: &str, instruction: &str) -> Result<String, String> {
    let v: Value = ureq::post(&format!("{ENGINE}/transform"))
        .timeout(Duration::from_secs(180))
        .send_json(json!({ "text": text, "instruction": instruction }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().into())
}

pub fn note_chunk(wav: &[u8], langs: &str) -> Result<String, String> {
    let v: Value = ureq::post(&format!("{ENGINE}/note?langs={langs}"))
        .timeout(Duration::from_secs(120))
        .send_bytes(wav)
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["text"].as_str().unwrap_or_default().trim().to_string())
}

pub fn summarize(lines: &Value, language: Option<&str>) -> Result<(String, String), String> {
    let v: Value = ureq::post(&format!("{ENGINE}/summary"))
        .timeout(Duration::from_secs(600))
        .send_json(json!({ "lines": lines, "language": language }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok((v["summary"].as_str().unwrap_or_default().into(), v["title"].as_str().unwrap_or_default().into()))
}

pub fn ask(question: &str, notes: &Value) -> Result<String, String> {
    let v: Value = ureq::post(&format!("{ENGINE}/ask"))
        .timeout(Duration::from_secs(300))
        .send_json(json!({ "question": question, "notes": notes }))
        .map_err(explain)?
        .into_json()
        .map_err(|e| e.to_string())?;
    Ok(v["answer"].as_str().unwrap_or_default().into())
}
