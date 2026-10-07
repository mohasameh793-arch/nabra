"""Speech → text with faster-whisper (Whisper large-v3), GPU first."""
import io
import logging
import os
import re
import site
from pathlib import Path

import numpy as np

from textnorm import fold

log = logging.getLogger("nabra.speech")

SAMPLE_RATE = 16_000
# A code-switched example in the prompt keeps English words in Latin script (benchmark: 10% → 55%).
# The dialect lock swaps it for one in the user's dialect, which nudges Whisper toward that dialect's words.
MIXED_EXAMPLES = {
    "gulf": "طيب خلنا نسوي deploy للـ app على Vercel وبعدين push على GitHub.",
    "egyptian": "طب يلا نعمل deploy للـ app على Vercel وبعد كده نعمل push على GitHub.",
    "levantine": "طيب خلينا نعمل deploy للـ app على Vercel وبعدين push على GitHub هلق.",
    "msa": "حسناً، لنقم بعمل deploy للـ app على Vercel ثم push على GitHub.",
}
MIXED_EXAMPLE = MIXED_EXAMPLES["gulf"]
# Below this, detection is a guess (Gulf Arabic → "Persian"); above it the speaker really is using
# that language, and forcing another one would make Whisper TRANSLATE instead of transcribe.
CONFIDENT_DETECTION = 0.8
# Quieter than this is silence/fan noise (~-50 dBFS); Whisper "hears" Thank you. in it.
SILENCE_RMS = 0.003
# What Whisper says over silence (trained on subtitled video). Compared folded, whole segment only.
_HALLUCINATIONS = {fold(s) for s in [
    "Thank you for watching", "Thanks for watching", "Please subscribe", "Subtitles by the Amara.org community",
    "شكرا للمشاهدة", "شكرا لكم على المشاهدة", "اشتركوا في القناة", "ترجمة نانسي قنقر",
]}


# Scripts that only some languages use. Text in one of these when none of its languages is allowed is a
# misdetection (e.g. Japanese out of noisy call audio), never what the user said.
_SCRIPTS = [
    (re.compile(r"[぀-ヿ㐀-鿿가-힯]"), {"ja", "zh", "ko", "yue"}),
    (re.compile(r"[Ѐ-ӿ]"), {"ru", "uk", "bg", "sr", "mk", "be", "kk", "mn", "tg", "ba", "tt"}),
    (re.compile(r"[֐-׿]"), {"he", "yi"}),
    (re.compile(r"[Ͱ-Ͽ]"), {"el"}),
    (re.compile(r"[ऀ-ॿ]"), {"hi", "mr", "ne", "sa"}),
    (re.compile(r"[฀-๿]"), {"th"}),
]


def foreign_script(text: str, allowed: list[str]) -> bool:
    """True if `text` contains a script that none of the allowed languages is written in."""
    return bool(allowed) and any(rx.search(text) and not langs & set(allowed) for rx, langs in _SCRIPTS)


# French/Spanish/German/… written in the same letters as English: Whisper's usual misdetections in an Arabic +
# English call. Letters English doesn't use, or several of these languages' everyday words.
_EUROPEAN_LETTERS = re.compile(r"[àâäãåæçèéêëìíîïñòóôõöøùúûüÿœß]", re.I)
_EUROPEAN_WORDS = set("le les des est une pour dans avec pas vous nous je c'est qui que il elle sont "
                      "el los las por para con una es muy pero como del ist und nicht ein eine der die das ich wir "
                      "sie mit auf zu non sono della per che".split())
_LATIN_LANGS = set("fr es de it pt nl ca ro sv da no pl cs tr cy".split())


def foreign_latin(text: str, allowed: list[str]) -> bool:
    """True if Latin-script text reads like a European language the user didn't pick (only checked when English is
    the one Latin-script language allowed, so a French or Spanish speaker who picked theirs is never second-guessed)."""
    if not allowed or "en" not in allowed or _LATIN_LANGS & set(allowed):
        return False
    words = [w for w in re.findall(r"[^\W\d_]+(?:'[^\W\d_]+)?", text.lower()) if w.isascii() or _EUROPEAN_LETTERS.search(w)]
    if not words:
        return False
    odd = sum(bool(_EUROPEAN_LETTERS.search(w)) or w in _EUROPEAN_WORDS for w in words)
    return odd >= 2 and odd / len(words) >= 0.2


def is_hallucination(segment: str, prompt: str) -> bool:
    """A segment that is a stock silence phrase, or the prompt (example sentence / vocabulary) echoed back.
    One- or two-word segments may legitimately be a vocabulary term, so only longer echoes count."""
    f = fold(segment)
    return not f or f in _HALLUCINATIONS or (len(f.split()) >= 3 and f in fold(prompt))


def _cuda_dlls_on_path(cuda_dir: Path | None) -> None:
    """Make cuBLAS/cuDNN findable: the folder the app downloaded them to, else the nvidia-* pip wheels
    (from-source setups). Windows won't find either on its own."""
    dirs = [cuda_dir] if cuda_dir and cuda_dir.is_dir() else []
    try:
        dirs += [d for root in site.getsitepackages() for d in Path(root).glob("nvidia/*/bin")]
    except AttributeError:  # frozen (packaged) builds have no site-packages
        pass
    for d in dirs:
        os.environ["PATH"] = str(d) + os.pathsep + os.environ["PATH"]


class Transcriber:
    def __init__(self, model: str = "large-v3", cuda_dir: Path | None = None):
        """`model`: a local model folder (what the app downloads) or a faster-whisper model name."""
        _cuda_dlls_on_path(cuda_dir)
        from faster_whisper import WhisperModel

        self.model, self.device = None, "cpu"
        # float16 needs a recent card; older or smaller ones still run fast with int8 on the GPU.
        for compute in ("float16", "int8_float16", "int8"):
            try:
                self.model, self.device = WhisperModel(model, device="cuda", compute_type=compute), "cuda"
                break
            except Exception as err:  # no NVIDIA GPU / old driver / unsupported type / out of VRAM
                log.warning("GPU %s unavailable (%s)", compute, type(err).__name__)
        if self.model is None:
            log.warning("no usable GPU, using CPU")
            # More than 4 threads makes Whisper no faster (measured); it only takes the PC away from the user.
            threads = min(4, os.cpu_count() or 4)
            self.model, self.device = WhisperModel(model, device="cpu", compute_type="int8", cpu_threads=threads), "cpu"
        # The first decode initializes CUDA kernels and the VAD model (~2–3 s). Pay that now.
        list(self.model.transcribe(np.zeros(SAMPLE_RATE, np.float32), language="en", vad_filter=True)[0])

    def choose_language(self, audio: np.ndarray, allowed: list[str], strict: bool = False) -> tuple[str | None, str | None]:
        """(forced, best allowed). forced None = let Whisper detect per segment (keeps Arabic/English mixing).
        strict (call notes): another language is never kept, however sure Whisper is: short, noisy call audio is
        where it is confidently wrong (Japanese, Welsh…), and the user picked the languages they speak."""
        if not allowed:
            return None, None
        _, _, ranked = self.model.detect_language(audio, vad_filter=True)
        in_list = [(code, p) for code, p in ranked if code in allowed]
        best = max(in_list, key=lambda x: x[1])[0] if in_list else allowed[0]
        top, top_p = ranked[0] if ranked else (None, 0.0)
        if top in allowed or (top_p >= CONFIDENT_DETECTION and not strict):
            return None, best
        return best, best

    def transcribe(self, wav: bytes, allowed: list[str], vocabulary: list[str], beam_size: int = 5,
                   dialect: str = "auto", strict: bool = False, detect: bool = True,
                   context: str = "") -> tuple[str, str | None]:
        """Returns (text, language). beam_size=1 is the fast pass for live (still-speaking) call text.
        detect=False skips the separate language check (live call text: speed over certainty; the finished
        phrase is redone with it). context: what was said just before, so names and topics carry over."""
        from faster_whisper import decode_audio

        audio = decode_audio(io.BytesIO(wav), sampling_rate=SAMPLE_RATE)
        if len(audio) < SAMPLE_RATE * 0.3 or np.sqrt(np.mean(audio ** 2)) < SILENCE_RMS:
            return "", None
        forced, best = self.choose_language(audio, allowed, strict) if detect else (None, None)
        arabic_in_play = not allowed or "ar" in allowed
        prompt = (MIXED_EXAMPLES.get(dialect, MIXED_EXAMPLE) + " " if arabic_in_play else "") + ", ".join(vocabulary) + "."
        context = context.strip()[-200:]
        if self.device == "cpu":
            beam_size = 1  # several times faster on a processor, for a small accuracy cost

        def run(language: str | None) -> tuple[str, str | None]:
            segments, info = self.model.transcribe(
                audio, language=language, multilingual=language is None, vad_filter=True, beam_size=beam_size,
                initial_prompt=prompt + (" " + context if context else ""),
                condition_on_previous_text=False)  # no repetition loops on long dictations
            text = " ".join(s.text.strip() for s in segments
                            if not is_hallucination(s.text, prompt) and fold(s.text) != fold(context)).strip()
            return text, language or info.language

        text, language = run(forced)
        latin = bool(text) and foreign_latin(text, allowed)
        drifted = strict and bool(text) and bool(allowed) and language not in allowed  # e.g. Whisper itself said "fr"
        if text and (foreign_script(text, allowed) or latin or drifted):
            # A language the user doesn't speak came out (per-segment detection can still drift): redo it in the
            # most likely allowed one; French-looking text was most likely English.
            text, language = run(best or ("en" if latin and "en" in allowed else allowed[0]))
            if foreign_script(text, allowed) or foreign_latin(text, allowed):
                # Whisper keeps writing e.g. Japanese even when told "English": it's not speech in a language the
                # user speaks (or noise it misheard). Nothing is better than text nobody in the call said.
                text = ""
        return text, language if text else None
