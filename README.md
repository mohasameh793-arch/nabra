<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="brand/nabra-logo-dark.svg">
    <img src="brand/nabra-logo.svg" alt="Nabra · نبرة" width="380">
  </picture>
</p>

# Nabra · نبرة

**Free, open-source voice typing and meeting notes that run on your own PC.**
Built for people who speak Arabic and English together, and great at either one on its own.

Hold a key, talk naturally, let go, and your words appear wherever your cursor is: in WhatsApp, Gmail, VS Code,
Word, ChatGPT, anywhere. Switch between Gulf, Egyptian, Levantine or Modern Standard Arabic and English (or
French, Spanish…) in the middle of a sentence. Nabra keeps each word in the language you said it in, keeps
technical words like `Next.js` and `Docker` spelled correctly, and never translates unless you ask.

> «طيب افتح لي VS Code and create a Next.js app وبعدها install Tailwind and then push it to GitHub.»
> is typed exactly like that, not as «نكست جي اس» and not translated.

Nabra is an independent, open-source alternative **inspired by [Wispr Flow](https://wisprflow.ai)**. It is
**not affiliated with or endorsed by Wispr**. The big difference: everything runs locally. Your voice never
leaves your computer, it works offline, and it's free.

---

## Contents
- [Install](#install)
- [First launch](#first-launch)
- [How to use it](#how-to-use-it)
- [Everything Nabra can do](#everything-nabra-can-do)
- [How it works](#how-it-works)
- [Privacy](#privacy)
- [Connect Claude, ChatGPT and other AI tools](#connect-claude-chatgpt-and-other-ai-tools)
- [Troubleshooting](#troubleshooting)
- [Build from source](#build-from-source)
- [Make a release (maintainers)](#make-a-release-maintainers)
- [Accuracy](#accuracy)
- [Roadmap](#roadmap)
- [Credits & license](#credits--license)

---

## Install

1. Go to the **[Releases](../../releases)** page and download **`Nabra-Setup.exe`** from the latest release.
2. Run it. It installs for your Windows user only (no admin rights needed). The installer asks whether Nabra
   should **start with Windows** (recommended, the default) and offers a **desktop shortcut** (ticked by default).
   Both can be changed later in Settings → General.
3. Windows may say *"Windows protected your PC"*, because the installer isn't code-signed yet. Click
   **More info → Run anyway**. You can check the download first: every release lists the file's SHA-256, and
   PowerShell's `Get-FileHash Nabra-Setup.exe` should print the same value.

**Requirements**

| | Minimum | Recommended (full experience) |
|---|---|---|
| OS | Windows 10/11, 64-bit | Windows 11 |
| GPU | none (runs on the processor) | NVIDIA GPU with **10 GB+** memory (e.g. RTX 3080 / 4070 / 5070 Ti) |
| Disk | 3 GB | 12 GB |
| Microphone | any | a headset for call notes |

## First launch

The installer is small; the speech and AI models are downloaded once, on first launch. The **Setup** screen
detects your PC and downloads only what it needs:

| Your PC | What Nabra downloads | Size |
|---|---|---|
| NVIDIA GPU, 10 GB+ | CUDA runtime · Whisper large-v3 · llama.cpp · Qwen3 8B | ≈ 10 GB |
| NVIDIA GPU, less memory | CUDA runtime · Whisper large-v3 (AI cleanup off) | ≈ 4.4 GB |
| No NVIDIA GPU | Whisper large-v3-turbo for the processor (AI cleanup off) | ≈ 1.6 GB |

Every file is checked against the checksum published by its source (PyPI, Hugging Face, GitHub). If your
connection drops, click **Try again** and the download resumes where it stopped. After setup, Nabra works
fully offline.

## How to use it

### Controls

| Do this | What happens |
|---|---|
| **Hold `Right Ctrl`**, talk, release | Your words are typed where your cursor is |
| Hover the **pill** on the right edge of the screen → click the **mic** | Hands-free dictation: talk as long as you like, click **■** to insert |
| **Hold `Right Alt`** and say a command | Edit text with your voice (see below) |
| **`Ctrl` + `Alt` + `N`**, or the pill's **record** button | Start / stop meeting notes |
| Double-click the pill, or the tray icon → **Open Nabra** | Open the Nabra window |
| Say **"new line"** / «سطر جديد» while dictating | Line break (**"new paragraph"** / «فقرة جديدة» for a blank line) |

### Voice commands (hold `Right Alt`)

Select text in any app first, or skip that to change what you just dictated.

| Say | Result |
|---|---|
| "make it shorter" · «اختصرها» | Shortens it |
| "make it more professional" · «خلها رسمية» | Rewrites the tone |
| "fix the grammar" · «صحح الأخطاء» | Corrects it |
| "translate to English" · «ترجمها للإنجليزي» | Translates it |
| "turn it into bullet points" · «حطها نقاط» | Restructures it |
| "scratch that" · «امسحها» | Deletes what Nabra just typed |
| "undo" · «تراجع», "select all" · «حدد الكل» | Exactly that |

Anything else you say is treated as an instruction for the selected text.

### The pill

The slim capsule on the right edge of your screen is Nabra's status light. Hover it to see the **Dictate** and
**Start note taking** buttons with their shortcuts. While you talk it shows a live sound meter and timer; after
that, a preview of what was typed. Clicking it never takes the cursor away from the app you're typing in.

## Everything Nabra can do

| Page | What it's for |
|---|---|
| **Dictation** | Your history of everything you've dictated, grouped by day, with search, copy, flag a bad transcription, add a misheard word to the dictionary. Shows total words, words per minute and your daily streak. |
| **Notetaker** | Meeting notes. Starting a note opens a **meeting window** with a live timer, sound meters and three tabs: **My thoughts** (your own notes, saved automatically), **Transcript** (the other people, from Zoom, Meet, Teams, YouTube or any app playing sound, as **They**, your microphone as **You**, live) and **Summary** (after you stop, click **Summarize** for key points, decisions and action items, in the meeting's language). Past notes stay in the Notetaker page. Connect your calendar to see today's meetings and get a nudge when one starts. |
| **Insights** | Words per minute, fixes Nabra made, total words, the apps you dictate in most, a streak calendar, and **Your voice**: language mix, the hours you dictate, the words you use most. |
| **Dictionary** | Names, products and jargon Nabra should always spell your way. Add how they sound in Arabic letters (e.g. `Supabase` sounds like «سوبابيس») and they're fixed in mixed dictation too. Also replacements, like `btw` → `by the way`. |
| **Snippets** | Say a short trigger ("my email", «توقيعي», "meeting link") and Nabra types the full saved text. |
| **Style** | Formal, casual or very casual writing for each kind of app: personal chats, work chats, email, everything else. Only capital letters and end punctuation change; your words never do. |
| **Transforms** | How Right Alt editing works, the commands you can say, and a box to try them. |
| **Scratchpad** | Autosaved drafts: click in, dictate, copy when ready. |
| **Settings** | Name, appearance (light/dark), start with Windows, microphone, Clean/Raw mode, your languages, summary language, meeting reminders, calendar, AI connections, privacy, models. |

**Clean vs Raw.** *Clean* (the default) fixes punctuation, technical terms and leftovers with the local AI, and a
built-in guard rejects any AI edit that would translate, rewrite, drop your words or change numbers. *Raw* types
exactly what was heard.

**Languages.** In Settings → Languages, tick the languages you speak. Nabra chooses among those, even mid-sentence.
It only ever forces a language when it's unsure, because forcing one would make the model *translate*.

## How it works

```
 Right Ctrl · Right Alt · Ctrl+Alt+N · the pill
        │
 Nabra app (Tauri + Rust)       hotkeys · microphone + computer-audio capture · typing into the focused app
        │                       pill · main window · history · notes · calendar · first-run setup
        │  local HTTP (127.0.0.1 only)
 Speech engine (Python)         Whisper large-v3 (faster-whisper) · cross-script term fixing · your dictionary
        │                       snippets · styles · voice commands
        │  local HTTP (127.0.0.1 only)
 Local AI (llama.cpp)           Qwen3 8B: guarded cleanup · transforms · meeting summaries
```

- The engine and the AI run as hidden helper processes tied to the app with a Windows *job object*: if Nabra
  closes or crashes, they're shut down with it.
- Nothing listens on your network; every connection is to `127.0.0.1`.
- Idle cost is essentially zero: the microphone only opens while you dictate or take notes.

## Privacy

- **Audio stays on your PC.** It's processed in memory and thrown away. Meeting audio is never saved.
- **No account, no telemetry, no cloud.** Nabra makes network requests only to download models during setup
  and, if you connect one, to fetch your own calendar link.
- **Your data:** settings, dictionary, snippets, history and notes are in `%APPDATA%\app.nabra.v2\`. Models are
  in `%LOCALAPPDATA%\Nabra\assets\`, logs in `%LOCALAPPDATA%\Nabra\logs\` (errors and timings, never what you
  said). Delete history anytime in Settings → Privacy.
- The calendar link is stored in **Windows Credential Manager**, not in a file.
- **Meeting notes:** tell people you're transcribing. Recording laws differ by country.

## Connect Claude, ChatGPT and other AI tools

Nabra includes a read-only [MCP](https://modelcontextprotocol.io) server so AI assistants can search and read
your meeting notes (list, search, read; they can't change or delete anything). Open **Settings →
Connections** for a ready-to-paste command:

```bash
claude mcp add -s user nabra-notes -- "<path to nabra-engine.exe>" --mcp --notes "%APPDATA%\app.nabra.v2\notes"
```

…or the JSON block for Claude Desktop, Cursor and other MCP apps.

## Troubleshooting

| Problem | Fix |
|---|---|
| Nothing happens when I hold Right Ctrl | Is the pill on the right edge? If it says "Finish setting up", open Nabra and complete Setup. Another dictation app (e.g. Wispr Flow) on the same key will conflict: quit one. |
| Text didn't appear in an app | Windows blocks typing into apps running as administrator. Use the tray icon → **Copy last dictation** and paste. |
| "You switched windows, so it's on your clipboard" | Intentional: Nabra won't type into a different window than the one you started in. Paste it. |
| Right Alt types special characters on my keyboard | That's AltGr. Nabra notices when you're typing with it and ignores it. Hold Right Alt *alone* for commands. |
| Wrong word for a name or term | Add it to the **Dictionary**, optionally with how it sounds in Arabic letters. |
| Dictation is slow | Without an NVIDIA GPU, each dictation takes a few seconds on the processor. That's expected. |
| "The local AI model isn't running" | Your GPU has less than 10 GB, or the AI download was skipped. Dictation still works; cleanup and overviews are off. |
| Setup failed | Click **Try again**: it resumes. Check free disk space. Details are in `%LOCALAPPDATA%\Nabra\logs`. |
| Start over | Quit Nabra and delete `%LOCALAPPDATA%\Nabra` (models) and/or `%APPDATA%\app.nabra.v2` (your data). |

## Build from source

You need **Windows 10/11**, **Python 3.11+**, **Rust** (stable, via [rustup](https://rustup.rs)) and an NVIDIA
GPU for the full experience.

```powershell
git clone https://github.com/<you>/nabra.git
cd nabra
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1   # Python environment for the engine
cd desktop
cargo run                                                    # first launch opens Setup to download models
```

The UI files in `desktop/ui/` are embedded at build time, so rebuild after editing them.

**Tests**

```powershell
python engine\selftest.py               # engine logic: term fixing, guard, styles, snippets, commands
cd desktop; cargo test                  # app logic: storage, calendar, downloads, audio, keyboard
cargo test -- --ignored loopback        # needs real audio hardware
```

**Project layout**

```
engine/    speech · lexicon (+ lexicon_builtin.tsv) · shortcuts · polish · service · mcp_notes · selftest
desktop/   src/ (Rust app) · ui/ (pill + main window: HTML/CSS/JS) · icons/ · tauri*.conf.json
bench/     accuracy benchmark: score · voices · run · sentences.jsonl
scripts/   setup.ps1 · build-installer.ps1
docs/      PLAN.md · BENCHMARKS.md
brand/     logo + app icon (SVG) · export.py → icon.ico, PNGs, GitHub social preview
```

## Make a release (maintainers)

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1          # once
powershell -ExecutionPolicy Bypass -File scripts\build-installer.ps1
```

This freezes the engine with PyInstaller, smoke-tests it, builds the app in release mode, and writes
**`dist\Nabra-Setup.exe`** plus **`dist\Nabra-Setup.exe.sha256`** (needs Node.js for the Tauri CLI).

Then on GitHub: **Releases → Draft a new release** → tag (e.g. `v2.0.0`) → attach `Nabra-Setup.exe` → paste
the SHA-256 into the notes → **Publish**. The "Install" link at the top of this README always points to the
latest release.

## Accuracy

Measured, not claimed: see [docs/BENCHMARKS.md](docs/BENCHMARKS.md). On 60 test clips (30 sentences × 2
regional voices), Nabra's clean pipeline cut word errors on mixed Arabic/English from **51% → 37%**, kept **60%**
of English words in Latin letters (vs 23% from plain Whisper), and spelled **83%** of key terms exactly
(vs 38%), with **no change** on Arabic-only and English-only speech. Reproduce with `bench\voices.py` then
`bench\run.py`.

## Roadmap

- **No-GPU cloud option:** free hosted Whisper (Groq) with your own key, matched local accuracy within ~2 points in tests
- Custom hotkeys
- macOS
- Code-signed installer and auto-updates
- Not planned for the local app: accounts, a mobile app and team sharing (they'd need a hosted server)

**Known limits:** the 8B local AI sometimes mixes up *done* vs *pending* in dialect meeting overviews; monthly
and yearly repeating calendar events show their first occurrence only; with nothing selected, some editors
(VS Code) copy the whole line on Ctrl+C, so a voice command then edits that line.

## Credits & license

Nabra is **MIT-licensed** (see [LICENSE](LICENSE)). It stands on excellent open-source work:
[Whisper](https://github.com/openai/whisper) (MIT) via [faster-whisper](https://github.com/SYSTRAN/faster-whisper)
and [CTranslate2](https://github.com/OpenNMT/CTranslate2), [llama.cpp](https://github.com/ggml-org/llama.cpp) (MIT),
[Qwen3](https://huggingface.co/Qwen) (Apache-2.0), [Tauri](https://tauri.app) and [cpal](https://github.com/RustAudio/cpal).
Models are downloaded from their original publishers and keep their own licenses.

"Wispr Flow" is a trademark of its owner, mentioned only to describe what inspired this project.

Contributions are welcome: open an issue or a pull request. Please run the tests above first.
