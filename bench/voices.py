"""Make benchmark clips with Microsoft neural voices (edge-tts). Sends sentence TEXT to Microsoft, no audio
of yours. Two regional voices per sentence → bench/audio/<id>-<voice>.wav (16 kHz mono) + bench/audio/clips.jsonl

    .venv\\Scripts\\python bench\\voices.py
"""
import asyncio
import io
import json
import wave
from pathlib import Path

import av
import edge_tts

HERE = Path(__file__).parent
VOICES = {  # first matching tag wins
    "gulf": ("ar-SA-HamedNeural", "ar-AE-FatimaNeural"),
    "egyptian": ("ar-EG-ShakirNeural", "ar-EG-SalmaNeural"),
    "levantine": ("ar-SY-LaithNeural", "ar-LB-LaylaNeural"),
    "msa": ("ar-SA-ZariyahNeural", "ar-JO-TaimNeural"),
    "english": ("en-US-AndrewNeural", "en-US-AvaNeural"),
    "french": ("fr-FR-HenriNeural", "fr-FR-DeniseNeural"),
}


async def speak(text: str, voice: str) -> bytes:
    audio = bytearray()
    async for part in edge_tts.Communicate(text, voice).stream():
        if part["type"] == "audio":
            audio += part["data"]
    return bytes(audio)


def mp3_to_wav16k(mp3: bytes, path: Path) -> None:
    resample = av.AudioResampler(format="s16", layout="mono", rate=16000)
    pcm = bytearray()
    with av.open(io.BytesIO(mp3)) as container:
        for frame in container.decode(audio=0):
            pcm += b"".join(f.to_ndarray().tobytes() for f in resample.resample(frame))
    pcm += b"".join(f.to_ndarray().tobytes() for f in resample.resample(None))
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(bytes(pcm))


async def main() -> None:
    out = HERE / "audio"
    out.mkdir(exist_ok=True)
    clips = []
    for line in (HERE / "sentences.jsonl").read_text(encoding="utf-8").splitlines():
        s = json.loads(line)
        voices = next((VOICES[t] for t in s["tags"] if t in VOICES), ())
        for voice in voices:
            name = f"{s['id']}-{voice.split('-')[2].removesuffix('Neural').lower()}"
            wav = out / f"{name}.wav"
            if not wav.exists():
                mp3_to_wav16k(await speak(s["text"], voice), wav)
            clips.append({**s, "id": name, "audio": wav.name, "voice": voice})
    (out / "clips.jsonl").write_text("\n".join(json.dumps(c, ensure_ascii=False) for c in clips) + "\n", encoding="utf-8")
    print(f"{len(clips)} clips in {out}")


if __name__ == "__main__":
    asyncio.run(main())
