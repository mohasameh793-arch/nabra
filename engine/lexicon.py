"""Lexicon: put English/technical terms back in Latin script when STT wrote them in Arabic letters.

Every term and every candidate span is reduced to a phonetic key: consonants only, with letters that
transliteration confuses merged into one class. `دوكر` and `Docker` both become "tkr".

Safety rules (each one came from a benchmark failure, see docs/BENCHMARKS.md):
  1. Exact spoken forms beat fuzzy matches.        بالرياكت → بالـ React, never "pull request"
  2. Fuzzy only on pure-Arabic spans, keys ≥ 4.    وبعدين must not become Python
  3. Text already in Latin script: casing only.    next.js → Next.js, never a different word
"""
import json
import re
from dataclasses import dataclass
from pathlib import Path

from textnorm import fold, is_arabic, is_latin, levenshtein

_AR_CLASS = {
    **dict.fromkeys("بپ", "b"), **dict.fromkeys("تطدضث", "t"), **dict.fromkeys("سصشزذظ", "s"),
    **dict.fromkeys("كقجغگخ", "k"), **dict.fromkeys("فڤ", "f"),
    "ر": "r", "ل": "l", "م": "m", "ن": "n", "ه": "h", "ح": "h",
}
_LATIN_RULES = [  # applied in order, on lowercase text
    (r"th", "t"), (r"sh", "s"), (r"ch", "k"), (r"ph", "f"), (r"ck", "k"), (r"qu", "k"), (r"x", "ks"),
    (r"c(?=[eiy])", "s"), (r"[cgjq]", "k"), (r"p", "b"), (r"d", "t"), (r"v", "f"), (r"z", "s"),
    (r"[^bfhklmnrst0-9]", ""),
]
# Letter names as spoken in Arabic-English speech, for acronyms ("اس دي كي" = SDK).
_SPELLED = dict(zip("abcdefghijklmnopqrstuvwxyz",
                    ["", "b", "s", "t", "", "f", "k", "ts", "", "k", "k", "l", "m", "n", "", "b", "k", "r", "s", "t",
                     "", "f", "tbl", "ks", "", "st"]))
_ARTICLES = ("وبال", "وال", "بال", "فال", "كال", "لل", "ال")  # longest first
_PREPOSITIONS = ("و", "ب", "ل", "ف", "ك")
MIN_FUZZY_KEY = 4


def _squeeze(s: str) -> str:
    return re.sub(r"(.)\1+", r"\1", s)


def phonetic_key(text: str) -> str:
    parts = []
    for word in re.sub(r"(?<=[a-z])(?=[A-Z])", " ", text).split():  # GitHub → Git Hub (t|h isn't "th")
        if is_latin(word):
            w = word.lower()
            for pattern, repl in _LATIN_RULES:
                w = re.sub(pattern, repl, w)
            parts.append(w)
        else:
            parts.append("".join(_AR_CLASS.get(c, c if c.isdigit() else "") for c in word))
    return _squeeze("".join(parts))


@dataclass(frozen=True)
class Entry:
    term: str
    forms: frozenset  # folded exact spoken forms (includes the term itself)
    keys: frozenset   # phonetic keys long enough for fuzzy matching

    @staticmethod
    def make(term: str, spoken: list[str]) -> "Entry":
        keys = {phonetic_key(term)}
        if term.isupper():  # acronym: also match it spelled out letter by letter
            keys.add(_squeeze("".join(_SPELLED.get(c, c) for c in term.lower())))
        return Entry(term, frozenset(fold(s) for s in [term, *spoken]),
                     frozenset(k for k in keys if len(k) >= MIN_FUZZY_KEY))


def load_builtin(path: Path) -> list[Entry]:
    """TSV: term <TAB> comma-separated spoken forms (optional). '#' starts a comment."""
    entries = []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.strip() and not line.startswith("#"):
            term, _, forms = line.partition("\t")
            entries.append(Entry.make(term.strip(), [f.strip() for f in forms.split(",") if f.strip()]))
    return entries


class Lexicon:
    """Built-in terms + the user's dictionary file, which the app writes as a JSON list of
    {"term": "Layla", "sounds_like": ["ليلى"]}  or  {"from": "btw", "to": "by the way"}."""

    def __init__(self, builtin: Path, user: Path | None = None):
        self.builtin = load_builtin(builtin)
        self.user_path = user
        self._user: list[Entry] = []
        self._replacements: list[tuple[re.Pattern, str]] = []
        self._user_mtime = None

    def _reload_user(self) -> None:
        if not (self.user_path and self.user_path.exists()):
            return
        mtime = self.user_path.stat().st_mtime
        if mtime == self._user_mtime:
            return
        data = json.loads(self.user_path.read_text(encoding="utf-8") or "[]")
        self._user = [Entry.make(d["term"], d.get("sounds_like", [])) for d in data if d.get("term")]
        self._replacements = [
            (re.compile(rf"(?<!\w){re.escape(d['from'])}(?!\w)", re.IGNORECASE), d["to"])
            for d in data if d.get("from") and d.get("to")
        ]
        self._user_mtime = mtime

    @property
    def entries(self) -> list[Entry]:
        """User dictionary first (it wins ties), reloaded whenever the app saves it."""
        self._reload_user()
        return self._user + self.builtin

    def replace(self, text: str) -> tuple[str, int]:
        """Apply the user's text replacements (btw → by the way). Returns (text, how many fired)."""
        self._reload_user()
        hits = 0
        for pattern, to in self._replacements:
            text, n = pattern.subn(to, text)
            hits += n
        return text, hits

    def prompt_terms(self, limit: int = 30) -> list[str]:
        return [e.term for e in self.entries[:limit]]

    # --- matching -------------------------------------------------------------------------------

    def _exact(self, span: str) -> Entry | None:
        f = fold(span)
        return next((e for e in self.entries if f in e.forms), None)

    def _fuzzy(self, span: str) -> Entry | None:
        if is_latin(span):
            return None
        key = phonetic_key(span)
        if len(key) < MIN_FUZZY_KEY:
            return None
        best, best_d = None, None
        for e in self.entries:
            for k in e.keys:
                budget = 0 if len(k) <= 4 else 1
                if abs(len(k) - len(key)) > budget:
                    continue
                d = levenshtein(key, k)
                if d <= budget and (best_d is None or d < best_d):
                    best, best_d = e, d
        return best

    def _lookup(self, span: str) -> tuple[Entry, str] | None:
        def strip(prefixes):
            return [(span[len(p):], p + "ـ ") for p in prefixes if span.startswith(p) and len(span) > len(p) + 1]

        candidates = [(span, "")] + strip(_ARTICLES)
        # One-letter prepositions (ببيثون = بـ Python) are only trusted for exact spellings; fuzzy
        # matching with them stripped would chew through ordinary words starting with و/ب/ل.
        passes = [(self._exact, candidates + strip(_PREPOSITIONS)), (self._fuzzy, candidates)]
        for matcher, cands in passes:
            for text, prefix in cands:
                if (e := matcher(text)) is not None:
                    return e, prefix
        return None

    def restore(self, text: str, max_span: int = 4) -> str:
        return self.restore_counted(text, max_span)[0]

    def restore_counted(self, text: str, max_span: int = 4) -> tuple[str, int, int]:
        """Returns (text, built-in term fixes, personal dictionary fixes). Casing-only fixes don't count."""
        self._reload_user()
        user_terms = {e.term for e in self._user}
        fixes = {"builtin": 0, "user": 0}

        def count(entry: Entry) -> None:
            fixes["user" if entry.term in user_terms else "builtin"] += 1

        tokens, out, i = text.split(), [], 0
        while i < len(tokens):
            for n in range(min(max_span, len(tokens) - i), 0, -1):
                joined = " ".join(tokens[i:i + n])
                m = re.search(r"[^\w.]+$", joined)
                span, tail = (joined[:m.start()], m.group()) if m else (joined, "")
                if not span:
                    continue
                if not is_arabic(span):
                    # Latin text is the speaker's own wording: only normalize casing of known terms.
                    hit = next((e for e in self.entries if e.term.lower() == span.lower()), None)
                    if hit:
                        out.append(hit.term + tail)
                        i += n
                        break
                    continue
                found = self._lookup(span)
                if found:
                    entry, prefix = found
                    count(entry)
                    out.append(prefix + entry.term + tail)
                    i += n
                    break
            else:
                out.append(tokens[i])
                i += 1
        return " ".join(out), fixes["builtin"], fixes["user"]
