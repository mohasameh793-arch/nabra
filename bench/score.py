"""Accuracy metrics for code-switched speech. Built on engine/textnorm.fold so the benchmark and the
engine agree on what "the same text" means."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "engine"))
from textnorm import fold, is_arabic, is_latin, levenshtein  # noqa: E402


def wer(ref: str, hyp: str) -> float:
    r = fold(ref).split()
    return levenshtein(r, fold(hyp).split()) / max(len(r), 1)


def cer(ref: str, hyp: str) -> float:
    r = fold(ref).replace(" ", "")
    return levenshtein(r, fold(hyp).replace(" ", "")) / max(len(r), 1)


def wer_in_script(ref: str, hyp: str, script: str) -> float | None:
    """WER over only the Arabic (`ar`) or only the Latin (`en`) words. None when the reference has none."""
    keep = is_arabic if script == "ar" else is_latin
    r = [t for t in fold(ref).split() if keep(t)]
    return levenshtein(r, [t for t in fold(hyp).split() if keep(t)]) / len(r) if r else None


def latin_kept(ref: str, hyp: str) -> float | None:
    """Share of the reference's English words that came back in Latin letters (not as نكست جي اس)."""
    want = [t for t in fold(ref).split() if is_latin(t)]
    if not want:
        return None
    pool = [t for t in fold(hyp).split() if is_latin(t)]
    hits = 0
    for t in want:
        if t in pool:
            pool.remove(t)
            hits += 1
    return hits / len(want)


def term_hits(hyp: str, terms: list[str]) -> float | None:
    """Exact-spelling rate for the sentence's key terms (Next.js must stay 'Next.js')."""
    return sum(t in hyp for t in terms) / len(terms) if terms else None


if __name__ == "__main__":
    ref = "أنا أبغى أسوي website باستخدام Next.js وبعدها أرفع على Vercel."
    bad = "أنا أبغي أسوي ويب سايت باستخدام نكست جي اس وبعدها أرفع على فيرسل"
    assert wer(ref, ref) == cer(ref, ref) == 0
    assert latin_kept(ref, ref) == 1 and latin_kept(ref, bad) == 0
    assert wer_in_script(ref, bad, "en") == 1.0
    assert term_hits(ref, ["Next.js", "Vercel"]) == 1 and term_hits(bad, ["Next.js"]) == 0
    assert abs(wer("a b c d", "a x c") - 0.5) < 1e-9
    print("score selftest passed")
