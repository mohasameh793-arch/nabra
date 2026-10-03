"""Deterministic text features applied after recognition: snippets, spoken line breaks, writing style,
and recognizing voice commands. No LLM involved, so none of these can rewrite the user's words."""
import json
import re
from pathlib import Path

from textnorm import fold

# ---- spoken line breaks ("new line" / «سطر جديد») --------------------------------------------
_BREAKS = [
    (r"new paragraph|فقرة جديدة|سطرين جداد", "\n\n"),
    (r"new line|next line|سطر جديد|سطر جديده", "\n"),
]
# A comma before "new line" is noise; a full stop belongs to the previous sentence and stays.
_BREAK_RES = [(re.compile(rf"\s*[,،]?\s*(?<!\w)(?:{words})(?!\w)[,.،]?\s*", re.IGNORECASE), out) for words, out in _BREAKS]


def spoken_breaks(text: str) -> tuple[str, int]:
    hits = 0
    for pattern, out in _BREAK_RES:
        text, n = pattern.subn(out, text)
        hits += n
    # Capitalize a Latin letter that now starts a line.
    text = re.sub(r"(\n)([a-z])", lambda m: m.group(1) + m.group(2).upper(), text)
    return text.strip(" "), hits


# ---- writing style --------------------------------------------------------------------------
STYLES = ("formal", "casual", "very_casual")


def apply_style(text: str, style: str) -> str:
    """formal: as written. casual: no full stop at the very end. very_casual: also no capital letter at the
    start of sentences (brand names and acronyms keep their casing: only sentence-initial letters change)."""
    if style not in ("casual", "very_casual") or not text:
        return text
    text = re.sub(r"(?<![.])\.$", "", text.rstrip())  # a final "." (not "...") goes; ? and ! stay
    if style == "very_casual":
        def lower_start(m: re.Match) -> str:
            word = m.group(2)
            keep = len(word) > 1 and (word.isupper() or any(c.isupper() for c in word[1:]))  # API, GitHub
            return m.group(1) + (word if keep else word[0].lower() + word[1:])
        text = re.sub(r"(^|[.!?]\s+|\n)([A-Z][\w.]*)", lower_start, text)
    return text


# ---- snippets -------------------------------------------------------------------------------
class Snippets:
    """[{"trigger": "my address", "text": "..."}] from the app. Saying the trigger inserts the text."""

    def __init__(self, path: Path | None):
        self.path = path
        self._items: list[tuple[str, re.Pattern, str]] = []
        self._mtime = None

    def _reload(self) -> None:
        if not (self.path and self.path.exists()):
            self._items = []
            return
        mtime = self.path.stat().st_mtime
        if mtime == self._mtime:
            return
        data = json.loads(self.path.read_text(encoding="utf-8") or "[]")
        self._items = [(fold(d["trigger"]), re.compile(rf"(?<!\w){re.escape(d['trigger'].strip())}(?!\w)[.!?،]?", re.IGNORECASE), d["text"])
                       for d in data if d.get("trigger", "").strip() and d.get("text")]
        self._mtime = mtime

    def expand(self, text: str) -> tuple[str, int]:
        self._reload()
        whole = fold(text)
        for trigger, _, body in self._items:
            if whole == trigger:  # the whole utterance was the trigger
                return body, 1
        hits = 0
        for _, pattern, body in self._items:
            text, n = pattern.subn(lambda _m, b=body: b, text)
            hits += n
        return text, hits


# ---- voice commands (Right Alt) ----------------------------------------------------------------
_COMMANDS = [
    ("delete_last", r"scratch that|delete that|remove that|undo that|امسح(?:ها| هذا| اللي قلته)?|احذف(?:ها| هذا)?|شيلها|الغيها"),
    ("new_paragraph", r"new paragraph|فقرة جديدة"),
    ("new_line", r"new line|next line|سطر جديد|انزل سطر"),
    ("undo", r"undo|تراجع"),
    ("select_all", r"select all|حدد الكل"),
]
_COMMAND_RES = [(name, re.compile(rf"^\s*(?:{words})\s*[.!،]?\s*$", re.IGNORECASE)) for name, words in _COMMANDS]


# A question about past calls ("what did Zaid say about the launch?", "إيه اللي اتفقنا عليه في الميتنج؟").
_MEETING_WORDS = re.compile(r"\b(?:meetings?|calls?|standup|sync)\b|ميتنج|ميتينج|اجتماع|الاجتماع|المكالمة|الكول|المكالمه", re.IGNORECASE)
_QUESTION = re.compile(r"\?|؟|^\s*(?:what|who|when|why|how|did|does|do|was|were|which)\b|\b(?:say|said|agree|agreed|decide|decided|mention|mentioned)\b|"
                       r"إيه|ايه|ماذا|ما هو|مين|من قال|متى|امتى|ليش|ليه|كيف|ازاي|قال|قالت|قالوا|اتفقنا|اتفقوا|قررنا", re.IGNORECASE)
_ASK = re.compile(r"^\s*(?:ask my (?:meetings|notes)|search my (?:meetings|notes)|اسأل(?: عن)?|دور في)\b", re.IGNORECASE)


def classify(instruction: str) -> str:
    """Map a spoken instruction to a fixed action, "ask" (a question about past calls), or "transform"
    (rewrite the target text with the LLM)."""
    for name, pattern in _COMMAND_RES:
        if pattern.match(instruction):
            return name
    if _ASK.search(instruction) or (_MEETING_WORDS.search(instruction) and _QUESTION.search(instruction)):
        return "ask"
    return "transform"
