# Nabra v2 — full rebuild in 13 hours (same plan, new code, Flow-style UI)

## Context
Rebuild the whole project from zero in `C:\Users\Majesty\Desktop\nabra-v2` with entirely new code and
design, keeping the product plan and every proven behavior from v1 (`Desktop\ai voice flow`, left untouched).
The UI follows Wispr Flow's *layout and interaction* (a small floating pill + a hub window with a left
sidebar) with Nabra's own visuals. Pill stays on the **right edge**. Large downloaded assets (Whisper cache,
Qwen3-8B GGUF, llama.cpp, pip wheels) are reused; no code is copied. The user watches progress in
**VS Code**: every new or changed file is opened with `code -r <file>` right after it's written.

Locked decisions: Windows first · Tauri 2 (Rust) + Python engine sidecar · Whisper large-v3 local GPU,
Groq as the no-GPU path later · Qwen3-8B local LLM with mandatory edit guard · push-to-talk Right Ctrl ·
languages = user-picked list (never forces translation) · call notes They/You · local-first, nothing uploaded.

## Already done (hour 0, ~1 h used)
`engine/`: `textnorm.py`, `lexicon.py` + `lexicon_builtin.tsv`, `speech.py`, `polish.py`, `service.py`,
`__main__.py`, `selftest.py` — **selftest passes**. Not yet opened in VS Code, not committed.

## UI spec (from the user's Flow screenshots)
Mirror the **layout and interactions**; never their logo, name, photos, or promo content (MCP banner,
"Invite your team", "Get a free month", mobile promo). Own wordmark "Nabra", own banner art (CSS gradient),
own palette: warm canvas `#F3F2EF`, content sheet `#FBFAF8` with 1px `#E7E4DF` border and 16px radius, ink
`#1F1E1C`, muted `#8A867F`, teal accent `#1F6F68` (light + dark variants), black primary buttons.

**Pill (right edge, vertically centered)**
- Idle: slim dark vertical capsule with a light rim (like screenshot 2).
- Hover: grows into a taller capsule with two round buttons stacked — **mic** (Dictate) and **record-dot**
  (Start notes) (screenshot 1). Hovering a button shows a tooltip to its left with the action + shortcut:
  "Dictate · hold Right Ctrl (click to toggle)", "Start notes · Ctrl+Alt+N". Click starts it immediately.
- Mic click = hands-free dictation toggle (click again to stop & insert); pill is non-activating so the
  text still lands in the app you were in. Record click = start/stop call notes.
- Active states: wide pill with level bars + timer (dictating), red dot + timer (notes), result preview, errors.
- Second global hotkey **Ctrl+Alt+N** (RegisterHotKey) toggles notes.

**Hub window** — frameless with custom title bar: left = sidebar toggle + profile icon; right = bell,
minimize, maximize, close. Left sidebar: wordmark, nav **Dictation · Notetaker · Insights · Dictionary**;
bottom **Settings · Help**. Content sits in the rounded sheet.
- **Dictation (home)**: "Welcome back, {name}"; dark gradient banner ("Make Nabra spell like you" → opens
  Dictionary); right stats card (total words, wpm, day streak); history grouped TODAY / YESTERDAY / date,
  rows = time + text, hover actions copy · flag (bad transcription → saved for benchmark review) · ⋯ (delete);
  search icon filters.
- **Notetaker**: title, gear (notes settings), "Start Notetaker" button; while live: transcript They/You;
  "Past notes" list grouped by date (doc icon, AI title, time); right panel = selected note title, date,
  **OVERVIEW** (summary, RTL-aware) + transcript + Summarize/language; bottom "Ask your notes" bar → local LLM
  answers over recent summaries (stretch, hour 11).
- **Insights**: tabs Your usage; cards: WPM gauge, fixes made (words corrected by lexicon/LLM + dictionary
  fixes), total words dictated, **App usage** bars (top apps by words, from foreground exe — honest instead of
  guessed categories), **streak heatmap** (GitHub-style, weeks × days) + longest streak.
- **Dictionary**: title + black "Add new"; tabs All / Personal / Replacements; banner with chips of recent
  terms + "Add new word"; list rows (term, or `btw → by the way` replacement); edit/delete on hover.
  Engine applies replacements after the lexicon.
- **Settings**: modal dialog with left nav — General (name, hotkeys shown, microphone picker, launch at
  login later), Languages (chips), Notes (summary language), Privacy (keep clips opt-in, delete all history),
  About (versions, engine/GPU status).

Data additions: history rows store `app`, `words`, `seconds`, `fixes` (engine returns `fixes`: lexicon
replacements + accepted LLM edits + dictionary hits); notes get an AI `title`.

## 13-hour schedule

| Hour | Block | Deliverables | Done when |
|---|---|---|---|
| 1 | Show + scaffold | `code -r` all engine files; `.gitignore`, `requirements.txt`, `README.md` skeleton, `docs/PLAN.md` (this plan) | Files visible in VS Code; first commit |
| 2 | Environment & assets | New `.venv` (pip cache → near-offline); `.assets/` **hard links** to v1 `Qwen3-8B-Q4_K_M.gguf` + llama.cpp files; `scripts/setup.ps1` documenting it | `python -m engine` starts, `/health` → cuda + llm true |
| 3 | Engine live test | Curl `/dictate`, `/note`, `/summary` with generated clips; fix anything found | Mixed AR/EN clip ≈1 s warm; Arabic summary for Arabic call |
| 4 | Benchmark | `bench/score.py`, `bench/voices.py` (edge-tts), `bench/run.py`, `bench/sentences.jsonl` (new 30+ sentences: Gulf/Egyptian/Levantine/MSA/EN/FR, names, numbers, URLs) | Report table written to `docs/BENCHMARKS.md`; ≈ v1 numbers (WER ≈0.21) |
| 5 | Desktop core (Rust) | `desktop/` Cargo + tauri.conf + capabilities + icon; `sound.rs` (mic + WASAPI loopback, WAV), `keyboard.rs` (Right-Ctrl hook, SendInput unicode, clipboard, foreground app name) + unit tests | `cargo test` green |
| 6 | Sidecars + dictation | `sidecar.rs` (job object, spawn engine + llama-server, HTTP), `store.rs` (settings, dictionary.json, history.jsonl, notes/), `dictation.rs` controller (hold → record → /dictate → insert, window-change safety, history append) | App dictates into Notepad |
| 7 | Pill | `pill.rs` (right edge, non-activating, idle/hover/call/wide sizes) + `ui/pill.html|js`: slim capsule → hover capsule with mic + record buttons and shortcut tooltips, click-to-toggle dictation, Ctrl+Alt+N notes hotkey, level bars, timer, preview, errors | Hover shows both buttons; clicks work without stealing focus |
| 8 | Hub shell + Dictation home | Frameless hub, custom title bar, sidebar (Dictation, Notetaker, Insights, Dictionary · Settings, Help), sheet layout; Dictation page: welcome, banner, stats card, grouped history with hover actions + search | Hub opens from pill/tray, real history shown |
| 9 | Dictionary + Insights + Settings | Dictionary (tabs, banner chips, add/edit/delete, replacements; engine hot-reload); Insights (WPM gauge, fixes, total words, app bars, streak heatmap); Settings modal (General/Languages/Notes/Privacy/About, mic picker) | Added term restored next dictation; Insights reflect history |
| 10 | Call notes backend | `meeting.rs`: two captures, pause chunker, echo dedupe, live events, session save + AI title; commands start/stop/summarize/list/load/delete/ask + tests | Playing audio shows "They" lines |
| 11 | Notetaker UI | List grouped by date, right detail panel (title, date, OVERVIEW, transcript, summarize + language), live view, consent notice, Ask-your-notes bar | Full call → titled summary in panel |
| 12 | Hardening | Error paths (no mic, no GPU, LLM down, engine crash restart, elevated window), idle CPU check, sidecar cleanup on kill, perf numbers | Checklist below all pass |
| 13 | Docs + ship | Final `README.md` (features, architecture, setup, run, tests, benchmarks, privacy, limits), screenshots, final commits | User can clone-free run from README |

Commit after every block (message ends with the Co-Authored-By line).

## Architecture (new layout)
```
engine/   textnorm · lexicon(+tsv) · speech · polish · service · __main__ · selftest
bench/    score · voices · run · sentences.jsonl  → docs/BENCHMARKS.md
desktop/  Cargo.toml · tauri.conf.json · capabilities/ · icons/
  src/    main · commands · dictation · meeting · pill · sidecar · sound · keyboard · store
  ui/     pill.html/js · hub.html/css/js
.assets/  (git-ignored hard links) llama/ · models/
```
Engine API: `GET /health`, `POST /dictate?langs&mode`, `POST /note?langs`, `POST /summary`.
Storage (app data dir): `settings.json`, `dictionary.json`, `history.jsonl`, `notes/<id>.json`.

## Risks & mitigations
- **WebView/assets embedded at build** → rebuild after UI edits (documented in README).
- **Wispr Flow running alongside** → hotkey overlap; README note, Settings shows active hotkey.
- **Hard links need same drive** (both on C:) → fallback: copy, documented in setup script.
- **Speakers echo in calls** → word-overlap dedupe + headphones tip.
- **Forced language = translation** → policy in `speech.py`, covered by docs; never force when confident.
- **LLM over-correction** → `check_edit` guard always on; selftest replays known failures.
- **UIPI (admin windows)** → can't detect; clipboard fallback via tray "Copy last".
- **Time overrun** → order is priority order; hours 12–13 shrink before core features do.

## Verification checklist
- `python engine/selftest.py` ✔ · `cargo test` ✔ (+ `--ignored loopback` on hardware)
- Bench report reproduces v1-level accuracy; pure-Arabic WER unchanged by lexicon/LLM
- Dictation into Notepad/VS Code/Chrome (Arabic, English, mixed); window switch → clipboard
- Pill on right edge, click opens hub, typing focus kept; idle CPU ≈0; killing app kills sidecars
- Notes: computer audio = They, mic = You, summary in call language and in a chosen language
- Dictionary term added in hub is used on the next dictation
