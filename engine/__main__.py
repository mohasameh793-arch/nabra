"""Nabra engine.

    python engine --port 8770 [--dictionary FILE] [--snippets FILE] [--llm-url URL] [--keep-clips DIR]
                  [--whisper MODEL_DIR_OR_NAME] [--cuda-dir DIR] [--openvino DIR] [--voice-model ONNX] [--no-auth]
    python engine --mcp [--notes DIR]          read-only MCP server for call notes (stdio)
"""
import argparse
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))  # flat module imports when run as `python engine`


def main() -> None:
    if "--mcp" in sys.argv:  # the packaged app has one engine executable; MCP is a mode of it
        sys.argv.remove("--mcp")
        import mcp_notes
        return mcp_notes.main()

    if "--voice-check" in sys.argv:  # build smoke test: the packaged voice model runtime works
        return voice_check(Path(sys.argv[sys.argv.index("--voice-check") + 1]))

    from service import TOKEN, Engine, serve

    ap = argparse.ArgumentParser(prog="engine")
    ap.add_argument("--port", type=int, default=8770)
    ap.add_argument("--dictionary", type=Path, help="user dictionary JSON written by the app")
    ap.add_argument("--snippets", type=Path, help="snippets JSON written by the app")
    ap.add_argument("--llm-url", help="llama.cpp server, e.g. http://127.0.0.1:8771")
    ap.add_argument("--keep-clips", type=Path, help="opt-in: save dictations as benchmark clips")
    ap.add_argument("--whisper", default="large-v3", help="local model folder or faster-whisper model name")
    ap.add_argument("--cuda-dir", type=Path, help="folder with cuBLAS/cuDNN DLLs downloaded by the app")
    ap.add_argument("--openvino", type=Path, help="folder with the OpenVINO packages downloaded by the app (NPU PCs)")
    ap.add_argument("--voice-model", type=Path, help="speaker-recognition ONNX model (call notes: who is speaking)")
    ap.add_argument("--no-auth", action="store_true", help="dev only (bench/run.py): run without NABRA_TOKEN")
    args = ap.parse_args()
    if not TOKEN and not args.no_auth:  # fail closed: without a token any local program or web page could use it
        sys.exit("NABRA_TOKEN is required (the app sets it; use --no-auth for a local dev run)")
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(levelname)s %(message)s")
    if args.openvino:  # not inside the engine executable (85 MB that only NPU PCs need): setup downloads it
        sys.path.insert(0, str(args.openvino))
    serve(args.port, Engine(args.dictionary, args.snippets, args.llm_url, args.keep_clips, args.whisper, args.cuda_dir,
                            args.voice_model))


def voice_check(model: Path) -> None:
    import io, math, wave
    from voices import Voices
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:  # 2 s of a voice-like tone
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(16000)
        w.writeframes(b"".join(int(8000 * math.sin(i / 16000 * 2 * math.pi * (140 + 60 * math.sin(i / 3000)))).to_bytes(2, "little", signed=True) for i in range(32000)))
    print("voiceprint", len(Voices(model).embed(buf.getvalue())))


if __name__ == "__main__":
    main()
