"""The engine's loopback HTTP API, used by the desktop app.

GET  /health                                → {"device": "cuda", "llm": true}
POST /dictate?langs=ar,en&mode=clean|raw&style=formal|casual|very_casual   WAV → {"text", "raw", "language", "fixes", "ms"}
POST /instruction?langs=ar,en                WAV body → {"instruction", "action"}   (Right Alt voice commands)
POST /transform {"text", "instruction"}      → {"text"}  (user-requested rewrite / translation)
POST /note?langs=ar,en[&partial=1]         WAV body → {"text", "language"}      (calls: no LLM, never stored)
POST /summary   {"lines": [...], "language": "ar"|null}  → {"summary", "title"}
POST /voice                                 WAV body → {"voice": [256 floats] | []}   (who is speaking)
POST /catchup  {"lines": [...]}             → {"text"}   (the last minutes of a call in 3 bullets)
POST /ask      {"question", "snippets"}     → {"text"}   (answer from past call notes, citing [n])

Every request must carry the X-Nabra-Token header the app passed in NABRA_TOKEN (so no other program or web
page can use the engine), and a Host of 127.0.0.1/localhost (blocks DNS rebinding). /health also returns
"proof", so the app can tell its own engine from anything else listening on the port.

Transcripts are never written to logs.
"""
import hashlib
import hmac
import json
import logging
import os
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from threading import Lock
from urllib.parse import parse_qs, urlparse

import httpx

from lexicon import Lexicon
from polish import Llm, changed_words, check_edit
from shortcuts import STYLES, Snippets, apply_style, classify, spoken_breaks
from speech import Transcriber
from voices import Voices

log = logging.getLogger("nabra.service")


class Engine:
    def __init__(self, dictionary: Path | None, snippets: Path | None, llm_url: str | None, keep_clips: Path | None,
                 whisper: str = "large-v3", cuda_dir: Path | None = None, voice_model: Path | None = None):
        self.lexicon = Lexicon(Path(__file__).with_name("lexicon_builtin.tsv"), dictionary)
        self.snippets = Snippets(snippets)
        self.speech = Transcriber(whisper, cuda_dir)
        self.llm = Llm(llm_url) if llm_url else None
        self.keep_clips = keep_clips
        self.gpu = Lock()  # one decode at a time; dictation and call notes share the GPU
        self.clips_lock = Lock()
        self.voices = Voices(voice_model)

    def dictate(self, wav: bytes, langs: list[str], mode: str, style: str = "formal") -> dict:
        t0 = time.perf_counter()
        vocab = self.lexicon.prompt_terms()
        with self.gpu:
            raw, language = self.speech.transcribe(wav, langs, vocab)
        text, fixes = raw, {"terms": 0, "dictionary": 0, "ai": 0, "snippets": 0}
        if raw and mode != "raw":
            text, fixes["terms"], fixes["dictionary"] = self.lexicon.restore_counted(raw)
            if self.llm:
                try:
                    cleaned = self.llm.cleanup(text, vocab)
                    if (reason := check_edit(text, cleaned)) is None:
                        fixes["ai"] = changed_words(text, cleaned)
                        text = cleaned
                    else:
                        log.info("cleanup rejected: %s", reason.split(" [")[0])  # reason only, no words
                except (httpx.HTTPError, KeyError, TypeError, ValueError) as err:  # incl. a cut-off or malformed reply
                    log.warning("LLM cleanup failed (%s); using lexicon output", type(err).__name__)
            text, replaced = self.lexicon.replace(text)
            fixes["dictionary"] += replaced
            text, _ = spoken_breaks(text)
            text, fixes["snippets"] = self.snippets.expand(text)
            if not fixes["snippets"]:  # a snippet is inserted exactly as saved
                text = apply_style(text, style if style in STYLES else "formal")
        result = {"text": text, "raw": raw, "language": language, "fixes": fixes,
                  "ms": round((time.perf_counter() - t0) * 1000)}
        if self.keep_clips and text:
            self._keep(wav, result)
        return result

    def note(self, wav: bytes, langs: list[str], partial: bool = False) -> dict:
        """partial: live text while someone is still speaking (fast pass; the finished phrase is redone in full)."""
        with self.gpu:
            raw, language = self.speech.transcribe(wav, langs, self.lexicon.prompt_terms(), beam_size=1 if partial else 5)
        return {"text": self.lexicon.restore(raw) if raw else "", "language": language}

    def instruction(self, wav: bytes, langs: list[str]) -> dict:
        with self.gpu:
            text, _ = self.speech.transcribe(wav, langs, self.lexicon.prompt_terms())
        text = text.strip()
        return {"instruction": text, "action": classify(text) if text else "none"}

    def transform(self, text: str, instruction: str) -> str:
        if not self.llm:
            raise RuntimeError("local AI model is not available")
        return self.llm.transform(text, instruction)

    def summary(self, lines: list[dict], language: str | None) -> dict:
        if not self.llm:
            raise RuntimeError("local AI model is not available")
        text = self.llm.summarize(lines, language)
        return {"summary": text, "title": self.llm.title(text) if text else ""}

    def catch_up(self, lines: list[dict], me: str = "") -> str:
        if not self.llm:
            raise RuntimeError("local AI model is not available")
        with self.gpu:  # one heavy GPU job at a time: live call text waits a few seconds
            return self.llm.catch_up(lines, me)

    def ask(self, question: str, snippets: list[dict]) -> str:
        if not self.llm:
            raise RuntimeError("local AI model is not available")
        with self.gpu:
            return self.llm.ask(question, snippets)

    def _keep(self, wav: bytes, result: dict) -> None:
        """Opt-in (--keep-clips): your own dictations become benchmark clips to review later."""
        self.keep_clips.mkdir(parents=True, exist_ok=True)
        clip = time.strftime("real-%Y%m%d-%H%M%S") + f"-{time.time_ns() % 10**9:09d}"  # two in one second
        row = {"id": clip, "audio": f"{clip}.wav", "text": result["text"], "stt": result["raw"],
               "terms": [], "tags": ["real", "unreviewed"]}
        with self.clips_lock:
            (self.keep_clips / f"{clip}.wav").write_bytes(wav)
            with open(self.keep_clips / "manifest.jsonl", "a", encoding="utf-8") as f:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")


TOKEN = os.environ.get("NABRA_TOKEN", "")
MAX_BODY = 64 * 1024 * 1024  # the app sends 16-bit mono at the mic's rate: a 10-minute take at 48 kHz is ~58 MB


def health_proof() -> str:
    return hmac.new(TOKEN.encode(), b"nabra-health", hashlib.sha256).hexdigest() if TOKEN else ""


def handler_for(engine: Engine):
    class Handler(BaseHTTPRequestHandler):
        timeout = 30  # a client that stalls mid-upload frees its thread instead of holding it forever

        def allowed(self) -> bool:
            host = (self.headers.get("Host") or "").split(":")[0].lower()
            if host not in ("127.0.0.1", "localhost"):
                self.reply(403, {"error": "forbidden"})
                return False
            sent = self.headers.get("X-Nabra-Token", "").encode("utf-8", "replace")  # bytes: non-ASCII can't raise
            if TOKEN and not hmac.compare_digest(sent, TOKEN.encode()):
                self.reply(401, {"error": "unauthorized"})
                return False
            return True

        def reply(self, status: int, payload: dict) -> None:
            body = json.dumps(payload, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            if not self.allowed():
                return
            if self.path != "/health":
                return self.reply(404, {"error": "not found"})
            self.reply(200, {"device": engine.speech.device, "llm": bool(engine.llm and engine.llm.ready()),
                             "proof": health_proof()})

        def do_POST(self):
            url = urlparse(self.path)
            q = parse_qs(url.query)
            langs = [c for c in q.get("langs", [""])[0].split(",") if c in engine.speech.model.supported_languages]
            if not self.allowed():
                return
            size = self.headers.get("Content-Length") or ""
            size = int(size) if size.isdecimal() else 0
            if size <= 0:  # missing, chunked, malformed, negative (read(-1) waits for EOF) or empty
                return self.reply(400, {"error": "bad request"})
            if size > MAX_BODY:
                return self.reply(413, {"error": "too large"})
            body = self.rfile.read(size)
            req = {}
            if url.path in ("/transform", "/catchup", "/ask", "/summary"):
                try:
                    req = json.loads(body)
                except ValueError:
                    req = None
                if not isinstance(req, dict):
                    return self.reply(400, {"error": "bad request"})
            try:
                if url.path == "/dictate":
                    self.reply(200, engine.dictate(body, langs, q.get("mode", ["clean"])[0], q.get("style", ["formal"])[0]))
                elif url.path == "/instruction":
                    self.reply(200, engine.instruction(body, langs))
                elif url.path == "/transform":
                    req = json.loads(body)
                    self.reply(200, {"text": engine.transform(req["text"], req["instruction"])})
                elif url.path == "/note":
                    self.reply(200, engine.note(body, langs, q.get("partial", ["0"])[0] == "1"))
                elif url.path == "/voice":
                    self.reply(200, {"voice": engine.voices.embed(body)})
                elif url.path == "/catchup":
                    req = json.loads(body)
                    self.reply(200, {"text": engine.catch_up(req["lines"], req.get("me", ""))})
                elif url.path == "/ask":
                    req = json.loads(body)
                    self.reply(200, {"text": engine.ask(req["question"], req["snippets"])})
                elif url.path == "/summary":
                    req = json.loads(body)
                    self.reply(200, engine.summary(req["lines"], req.get("language")))
                else:
                    self.reply(404, {"error": "not found"})
            except RuntimeError as err:
                self.reply(503, {"error": str(err)})
            except Exception as err:
                log.exception("%s failed", url.path)  # traceback only, never transcript text
                self.reply(500, {"error": type(err).__name__})

        def log_message(self, *_):  # no access log
            pass

    return Handler


def serve(port: int, engine: Engine) -> None:
    server = ThreadingHTTPServer(("127.0.0.1", port), handler_for(engine))
    log.info("listening on 127.0.0.1:%d (device=%s, llm=%s)", port, engine.speech.device, bool(engine.llm))
    server.serve_forever()
