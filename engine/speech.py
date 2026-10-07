"""Speech → text with faster-whisper (Whisper large-v3), GPU first."""
import io
import logging
import os
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

    def choose_language(self, audio: np.ndarray, allowed: list[str]) -> str | None:
        """None = let Whisper detect per segment (keeps Arabic/English mixing). A code = force it."""
        if not allowed:
            return None
        _, _, ranked = self.model.detect_language(audio, vad_filter=True)
        top, top_p = ranked[0] if ranked else (None, 0.0)
        if top in allowed or top_p >= CONFIDENT_DETECTION:
            return None
        in_list = [(code, p) for code, p in ranked if code in allowed]
        return max(in_list, key=lambda x: x[1])[0] if in_list else allowed[0]

    def transcribe(self, wav: bytes, allowed: list[str], vocabulary: list[str], beam_size: int = 5,
                   dialect: str = "auto") -> tuple[str, str | None]:
        """Returns (text, language). beam_size=1 is the fast pass for live (still-speaking) call text."""
        from faster_whisper import decode_audio

        audio = decode_audio(io.BytesIO(wav), sampling_rate=SAMPLE_RATE)
        if len(audio) < SAMPLE_RATE * 0.3 or np.sqrt(np.mean(audio ** 2)) < SILENCE_RMS:
            return "", None
        forced = self.choose_language(audio, allowed)
        arabic_in_play = not allowed or "ar" in allowed
        prompt = (MIXED_EXAMPLES.get(dialect, MIXED_EXAMPLE) + " " if arabic_in_play else "") + ", ".join(vocabulary) + "."
        if self.device == "cpu":
            beam_size = 1  # several times faster on a processor, for a small accuracy cost
        segments, info = self.model.transcribe(
            audio, language=forced, multilingual=forced is None, vad_filter=True, beam_size=beam_size,
            initial_prompt=prompt, condition_on_previous_text=False)  # no repetition loops on long dictations
        text = " ".join(s.text.strip() for s in segments if not is_hallucination(s.text, prompt)).strip()
        return text, (forced or info.language) if text else None
