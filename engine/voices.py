"""Voiceprints for call notes: who is speaking.

A small speaker-recognition model (WeSpeaker ResNet34, ONNX, ~26 MB, CPU) turns a phrase into a 256-number
"voiceprint". The app compares voiceprints to tell people apart and to recognise voices it has been told the
names of. Nothing here is stored; the app keeps the numbers (never the audio).
"""
import io
from pathlib import Path

MIN_SECONDS = 1.0  # shorter phrases don't carry enough voice to compare


class Voices:
    def __init__(self, model: Path | None):
        self.model = model
        self._extractor = None

    def _load(self):
        if self._extractor is None:
            if not self.model or not self.model.exists():
                raise RuntimeError("The voice model isn't downloaded yet")
            import sherpa_onnx  # imported lazily: only call notes need it
            config = sherpa_onnx.SpeakerEmbeddingExtractorConfig(model=str(self.model), num_threads=2)
            self._extractor = sherpa_onnx.SpeakerEmbeddingExtractor(config)
        return self._extractor

    def embed(self, wav: bytes) -> list[float]:
        """The phrase's voiceprint (unit length), or [] if it's too short."""
        import numpy as np
        from faster_whisper import decode_audio

        audio = decode_audio(io.BytesIO(wav), sampling_rate=16000)
        if len(audio) < 16000 * MIN_SECONDS:
            return []
        extractor = self._load()
        stream = extractor.create_stream()
        stream.accept_waveform(16000, audio)
        stream.input_finished()
        v = np.asarray(extractor.compute(stream), dtype=np.float32)
        return (v / (np.linalg.norm(v) or 1.0)).round(5).tolist()
