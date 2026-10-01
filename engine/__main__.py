"""python engine --port 8770 [--dictionary FILE] [--llm-url URL] [--keep-clips DIR]"""
import argparse
import logging
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))  # flat module imports when run as `python engine`

from service import Engine, serve  # noqa: E402


def main() -> None:
    ap = argparse.ArgumentParser(prog="engine")
    ap.add_argument("--port", type=int, default=8770)
    ap.add_argument("--dictionary", type=Path, help="user dictionary JSON written by the app")
    ap.add_argument("--llm-url", help="llama.cpp server, e.g. http://127.0.0.1:8771")
    ap.add_argument("--keep-clips", type=Path, help="opt-in: save dictations as benchmark clips")
    args = ap.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(name)s %(levelname)s %(message)s")
    serve(args.port, Engine(args.dictionary, args.llm_url, args.keep_clips))


if __name__ == "__main__":
    main()
