# Nabra (نبرة)

**Local-first voice typing and call notes for people who mix Arabic and English.**

Hold a key, speak naturally (Gulf, Egyptian, Levantine, MSA, English, French… switching mid-sentence),
release, and the text appears wherever your cursor is. English terms stay in Latin letters, Arabic stays
Arabic, and nothing gets translated. Speech recognition and AI cleanup run on your own GPU, so no audio
leaves the PC.

> «طيب افتح لي VS Code and create a Next.js app وبعدها install Tailwind and then push it to GitHub.»
> is typed exactly like that, not as «نكست جي اس» and not translated.

## What it does

| | |
|---|---|
| **Dictate anywhere** | Hold **Right Ctrl**, talk, release. Or hover the pill on the right edge and click the mic for hands-free mode. |
| **Code-switching** | Arabic dialects + English (+ the languages you pick) in one sentence. Language is detected per segment; Nabra never forces a language you're clearly speaking, because forcing makes Whisper translate. |
| **Technical words** | A built-in developer lexicon + your Dictionary put `Next.js`, `Supabase`, `Docker`… back in Latin script, including after Arabic articles (`بالرياكت` → `بالـ React`). |
| **Clean mode** | A local LLM fixes punctuation and leftovers. A guard rejects any edit that translates, rewrites, drops your words, or changes numbers. **Raw** mode keeps exactly what was heard. |
| **Notetaker** | **Ctrl+Alt+N** (or the pill's record button) records a call: computer audio = **They**, your mic = **You**, live transcript, then an automatic titled **overview** (summary, decisions, action items) in the call's language. |
| **Ask your notes** | "What did we decide this week?" is answered from your saved call overviews. |
| **Dictionary** | Your names, products and jargon, with optional Arabic "sounds like" spellings, plus replacements (`btw` → `by the way`). Used from the next dictation, no restart. |
| **Insights** | Words per minute, fixes made, total words, usage per app, streak calendar. |
| **Private** | History, notes, dictionary and settings stay in your app-data folder. Call audio is never saved. |

## How it works

```
 Right Ctrl / pill / Ctrl+Alt+N
            │
 desktop/ (Tauri 2, Rust) ── keyboard.rs   global hook, typing into the focused app, clipboard
            │                sound.rs      mic + WASAPI loopback capture → WAV
            │                dictation.rs  hold/toggle → engine → type → history
            │                meeting.rs    They/You phrases, echo removal, auto overview
            │                pill.rs / ui/ the right-edge pill + the hub window
            │  loopback HTTP (127.0.0.1:8770)
 engine/ (Python)  ───────── speech.py     Whisper large-v3 on GPU (faster-whisper), language policy
            │                lexicon.py    cross-script term restore + your dictionary + replacements
            │                polish.py     LLM cleanup + edit guard, summaries, titles, Q&A
            │  OpenAI-compatible HTTP (127.0.0.1:8771)
 llama.cpp server ────────── Qwen3-8B (Q4_K_M), local
```

The engine and the LLM run as hidden child processes inside a Windows *job object*: if Nabra exits or
crashes, they're killed with it. Nothing listens on anything but `127.0.0.1`.

## Requirements

- Windows 10/11 x64
- NVIDIA GPU with ~10 GB VRAM for the full setup (Whisper + Qwen3-8B). Without a GPU, Whisper falls back
  to CPU (slow); without the LLM files, dictation still works and overviews are off.
- Python 3.12, Rust (stable), ~10 GB disk for models

## Setup

```powershell
# 1. Python environment + AI assets (links them from an existing folder, or tells you what to download)
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1

# 2. Build and run the app
cd desktop
cargo run
```

`.assets\` needs `llama\llama-server.exe` (llama.cpp Windows CUDA 13 build) and
`models\Qwen3-8B-Q4_K_M.gguf` (Apache-2.0). Whisper large-v3 downloads automatically on first start (~3 GB).

The UI files in `desktop/ui/` are embedded at build time: **rebuild after editing them**.

## Using it

- **Dictate:** hold Right Ctrl → talk → release. Text goes where your cursor is. If you switch windows
  while it's transcribing, the text goes to the clipboard instead of the wrong app.
- **Hands-free:** hover the pill (right edge) → click the mic → talk → click ■.
- **Call notes:** Ctrl+Alt+N or the pill's record button. Stop the same way; the overview appears in
  Notetaker a few seconds later. Tell people you're transcribing. Headphones give the cleanest They/You split.
- **Languages / mode / mic / appearance:** Settings (gear in the sidebar).
- **Tray icon:** open Nabra, copy last dictation, quit. Closing the window keeps Nabra in the tray.

## Tests

```powershell
python engine\selftest.py                                   # lexicon, guard, language vote (no GPU)
.venv\Scripts\python bench\score.py                         # metric self-check
cd desktop; cargo test                                      # storage, phrases, echo, keystrokes, WAV
cargo test -- --ignored loopback                            # needs real audio hardware
```

## Accuracy

Measured, not claimed: see [docs/BENCHMARKS.md](docs/BENCHMARKS.md). On 60 synthetic clips (30 sentences ×
2 regional voices), the clean pipeline cut word error on mixed Arabic/English from **0.51 → 0.37**, kept
**60%** of English words in Latin script (vs 23%), and spelled **83%** of key terms exactly (vs 38%), with
**zero** change on Arabic-only and English-only clips. To reproduce:

```powershell
.venv\Scripts\python bench\voices.py   # generate clips (sends sentence TEXT to Microsoft TTS)
.venv\Scripts\python bench\run.py      # needs the engine running
```

## Privacy

- Audio is processed in memory and discarded. Call audio is never written to disk.
- Opt-in only: Settings → Privacy → *keep my dictation audio* saves **your own** dictations to `bench\real\`
  for accuracy testing.
- Logs record errors and timings, never what you said.
- Data folder: `%APPDATA%\app.nabra.v2\` (settings, dictionary, history, notes). Delete history from Settings → Privacy.

## Known limits

- **Admin windows:** Windows blocks typing into elevated apps from a normal app. Use tray → *Copy last dictation*.
- **Summaries on an 8B model** sometimes mix up *done* vs *pending* in dialect speech. A larger local model
  or an optional cloud model fixes this; the overview can be rewritten per note.
- **Hotkeys are fixed** (Right Ctrl, Ctrl+Alt+N) in this version. Don't run another dictation app on the same key.
- **No-GPU users:** CPU Whisper is slow. Groq's free hosted Whisper matched local accuracy within ~2 WER
  points in testing and is the planned no-GPU option.
- **Benchmarks use synthetic voices.** Real-voice numbers come from the opt-in clip collection.

## Project layout

```
engine/    speech · lexicon (+ lexicon_builtin.tsv) · polish · service · selftest
desktop/   src/ (Rust) · ui/ (pill + hub: HTML/CSS/JS modules) · icons/ · tauri.conf.json
bench/     score · voices · run · sentences.jsonl
scripts/   setup.ps1
docs/      PLAN.md · BENCHMARKS.md
```
