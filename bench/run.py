"""Benchmark the running engine (python engine --port 8770 ...) on bench/audio/clips.jsonl.

Each clip is sent twice: mode=raw (Whisper + vocabulary prompt) and mode=clean (+ lexicon + guarded LLM),
so the report shows what each stage adds.

    .venv\\Scripts\\python bench\\run.py [--langs ar,en] [--engine http://127.0.0.1:8770]
"""
import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path

import httpx

import score

HERE = Path(__file__).parent
METRICS = ("wer", "cer", "wer_ar", "wer_en", "latin_kept", "terms")


def measure(ref: str, hyp: str, terms: list[str]) -> dict:
    return {"wer": score.wer(ref, hyp), "cer": score.cer(ref, hyp), "wer_ar": score.wer_in_script(ref, hyp, "ar"),
            "wer_en": score.wer_in_script(ref, hyp, "en"), "latin_kept": score.latin_kept(ref, hyp),
            "terms": score.term_hits(hyp, terms)}


def mean(values) -> float | None:
    values = [v for v in values if v is not None]
    return round(statistics.mean(values), 3) if values else None


def table(rows: list[dict], title: str) -> str:
    groups = defaultdict(list)
    for r in rows:
        groups[("all", r["mode"])].append(r)
        for tag in ("mixed", "arabic-only", "english"):
            if tag in r["tags"]:
                groups[(tag, r["mode"])].append(r)
    out = [f"### {title}", "", "| subset | mode | n | " + " | ".join(METRICS) + " | p50 ms | p95 ms |",
           "|---|---|---|" + "---|" * (len(METRICS) + 2)]
    for (subset, mode), rs in sorted(groups.items(), key=lambda kv: (kv[0][0] != "all", kv[0])):
        ms = sorted(r["ms"] for r in rs)
        cells = " | ".join(str(mean(r[m] for r in rs)) for m in METRICS)
        out.append(f"| {subset} | {mode} | {len(rs)} | {cells} | {ms[len(ms) // 2]} | {ms[int(len(ms) * 0.95)]} |")
    return "\n".join(out)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", default="http://127.0.0.1:8770")
    ap.add_argument("--langs", default="ar,en,fr")
    args = ap.parse_args()

    audio = HERE / "audio"
    clips = [json.loads(l) for l in (audio / "clips.jsonl").read_text(encoding="utf-8").splitlines() if l]
    http = httpx.Client(timeout=120)
    rows = []
    for clip in clips:
        wav = (audio / clip["audio"]).read_bytes()
        for mode in ("raw", "clean"):
            r = http.post(f"{args.engine}/dictate", params={"mode": mode, "langs": args.langs}, content=wav).json()
            rows.append({"id": clip["id"], "mode": mode, "tags": clip["tags"], "ref": clip["text"],
                         "hyp": r["text"], "ms": r["ms"], **measure(clip["text"], r["text"], clip["terms"])})
        print(f"{clip['id']:24} {rows[-1]['wer']:.2f}  {rows[-1]['hyp']}")

    results = HERE / "results"
    results.mkdir(exist_ok=True)
    (results / "last.jsonl").write_text("\n".join(json.dumps(r, ensure_ascii=False) for r in rows), encoding="utf-8")
    report = table(rows, f"{len(clips)} synthetic clips, langs={args.langs}")
    (results / "last.md").write_text(report + "\n", encoding="utf-8")
    print("\n" + report)


if __name__ == "__main__":
    main()
