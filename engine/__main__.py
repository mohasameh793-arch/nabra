"""Nabra engine.

    python engine --port 8770 [--dictionary FILE] [--snippets FILE] [--llm-url URL] [--keep-clips DIR]
                  [--whisper MODEL_DIR_OR_NAME] [--cuda-dir DIR]
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

    from service import Engine, serve

    ap = argparse.ArgumentParser(prog="engine")
    ap.add_argument("--port", type=int, default=8770)
    ap.add_argument("--dictionary", type=Path, help="user dictionary JSON written by the app")
    ap.add_argument("--snippets", type=Path, help="snippets JSON written by the app")
    ap.add_argument("--llm-url", help="llama.cpp server, e.g. http://127.0.0.1:8771")
    ap.add_argument("--keep-clips", type=Path, help="opt-in: save dictations as benchmark clips")
    ap.add_argument("--whisper", default="large-v3", help="local model folder or faster-whisper model name")
    ap.add_argument("--cuda-dir", type=Path, help="folder with cuBLAS/cuDNN DLLs downloaded by the app")
    args = ap.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(levelname)s %(message)s")
    serve(args.port, Engine(args.dictionary, args.snippets, args.llm_url, args.keep_clips, args.whisper, args.cuda_dir))


if __name__ == "__main__":
    main()
