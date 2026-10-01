# Benchmarks

Reproduce: start the engine (see README), then

    .venv\Scripts\python bench\voices.py     # 30 sentences × 2 regional voices = 60 clips
    .venv\Scripts\python bench\run.py        # each clip in mode=raw and mode=clean

Metrics (`bench/score.py`, all on `engine/textnorm.fold`):
- **wer / cer**: word / character error rate after Arabic normalization (tashkeel, alef/ya/ta-marbuta, digits)
- **wer_ar / wer_en**: WER over only the Arabic or only the Latin words: which language is failing
- **latin_kept**: share of English words returned in Latin letters (catches `نكست جي اس` for `Next.js`)
- **terms**: exact spelling of key terms (`Next.js`, `RTX 5070 Ti`)
- **p50 / p95 ms**: engine-side latency per clip, warm, RTX 5070 Ti

## 2026-10-01 — v2 engine, Whisper large-v3 fp16 + vocabulary prompt; clean = + lexicon + Qwen3-8B guarded cleanup

| subset | mode | n | wer | cer | wer_ar | wer_en | latin_kept | terms | p50 ms | p95 ms |
|---|---|---|---|---|---|---|---|---|---|---|
| all | raw | 60 | 0.326 | 0.240 | 0.541 | 0.561 | 0.439 | 0.516 | 958 | 1194 |
| all | **clean** | 60 | **0.252** | **0.161** | **0.438** | **0.315** | **0.685** | **0.797** | 1167 | 1458 |
| mixed AR/EN | raw | 32 | 0.509 | 0.371 | 0.713 | 0.772 | 0.228 | 0.375 | 1022 | 1222 |
| mixed AR/EN | **clean** | 32 | **0.370** | **0.224** | **0.571** | **0.404** | **0.596** | **0.825** | 1220 | 1461 |
| Arabic only | raw | 12 | 0.083 | 0.071 | 0.083 | – | – | – | 1047 | 1194 |
| Arabic only | clean | 12 | 0.083 | 0.071 | 0.083 | – | – | – | 1247 | 1358 |
| English | raw | 12 | 0.192 | 0.139 | – | 0.184 | 0.816 | 0.700 | 731 | 783 |
| English | clean | 12 | 0.192 | 0.139 | – | 0.184 | 0.816 | 0.700 | 845 | 940 |

**Reading it**
- On mixed speech the clean pipeline cuts WER by 27% (0.51 → 0.37), keeps 2.6× more English words in Latin
  script, and more than doubles exact term spelling. Cost: ≈ +0.2 s.
- **No over-correction:** Arabic-only and English-only clips are identical in raw and clean mode.
- `wer_ar` stays high on mixed clips because transliterations the lexicon doesn't know still count as Arabic
  insertions (e.g. `الميتنج` for "meeting"). Adding words to the Dictionary fixes those per user.
- Number words: «ثلاثمية وخمسين» comes back as `350`. That's correct output but counts as an error here.

**Caveats.** Synthetic voices: cleaner than real microphones, and Arabic voices read English with a heavy accent.
Some key terms are also in the built-in lexicon (that is its purpose), so `terms` is optimistic for unknown
words. Real-voice clips (Settings → Privacy → keep clips) are the next benchmark set.

## History (v1, same method, different sentences)
- Plain Whisper wrote ~90% of English words in Arabic script on mixed speech; a mixed-script prompt fixed
  most of it (latin_kept 0.10 → 0.55).
- The first lexicon over-corrected ordinary Arabic (وبعدين→Python, مراجعة→merge, التقرير→Docker). The safety
  rules in `engine/lexicon.py` and the cases in `engine/selftest.py` exist because of that.
- An unguarded LLM pass made results worse (`ونcreate`, Postgres→PostgreSQL); `check_edit` rejects such edits.
- Forcing a language makes Whisper translate; `speech.py` only forces when detection is unsure.
- Groq's hosted large-v3 (free tier) came within ~2 WER points of local with the same pipeline: the planned
  no-GPU path.
