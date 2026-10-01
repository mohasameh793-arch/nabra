"""Speech → text with faster-whisper (Whisper large-v3), GPU first."""
import io
import logging
import os
import site
from pathlib import Path

import numpy as np

log = logging.getLogger("nabra.speech")

SAMPLE_RATE = 16_000
# A code-switched example in the prompt keeps English words in Latin script (benchmark: 10% → 55%).
MIXED_EXAMPLE = "طيب خلنا نسوي deploy للـ app على Vercel وبعدين push على GitHub."
# Below this, detection is a guess (Gulf Arabic → "Persian"); above it the speaker really is using
# that language, and forcing another one would make Whisper TRANSLATE instead of transcribe.
CONFIDENT_DETECTION = 0.8


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

        try:
            self.model, self.device = WhisperModel(model, device="cuda", compute_type="float16"), "cuda"
        except Exception as err:  # no NVIDIA GPU / old driver / out of VRAM
            log.warning("GPU unavailable (%s), using CPU", type(err).__name__)
            self.model, self.device = WhisperModel(model, device="cpu", compute_type="int8"), "cpu"
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

    def transcribe(self, wav: bytes, allowed: list[str], vocabulary: list[str]) -> tuple[str, str | None]:
        """Returns (text, language)."""
        from faster_whisper import decode_audio

        audio = decode_audio(io.BytesIO(wav), sampling_rate=SAMPLE_RATE)
        if len(audio) < SAMPLE_RATE * 0.3:
            return "", None
        forced = self.choose_language(audio, allowed)
        arabic_in_play = not allowed or "ar" in allowed
        prompt = (MIXED_EXAMPLE + " " if arabic_in_play else "") + ", ".join(vocabulary) + "."
        segments, info = self.model.transcribe(
            audio, language=forced, multilingual=forced is None, vad_filter=True, beam_size=5,
            initial_prompt=prompt)
        return " ".join(s.text.strip() for s in segments).strip(), forced or info.language
