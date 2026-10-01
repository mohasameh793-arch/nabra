"""Text primitives shared by the engine and the benchmark: normalization, scripts, edit distance."""
import re
import unicodedata

ARABIC = re.compile(r"[؀-ۿ]")
LATIN = re.compile(r"[A-Za-z]")

_TASHKEEL = re.compile(r"[ً-ْٰـ]")  # short vowels, dagger alef, tatweel
_FOLD = str.maketrans({
    "أ": "ا", "إ": "ا", "آ": "ا", "ٱ": "ا", "ى": "ي", "ة": "ه",
    **{chr(0x0660 + i): str(i) for i in range(10)},  # Arabic-Indic digits
    **{chr(0x06F0 + i): str(i) for i in range(10)},  # Eastern Arabic-Indic digits
})
# Drop punctuation but keep dots/hyphens inside tokens (Next.js, getting-started).
_PUNCT = re.compile(r"(?<!\w)[.\-]|[.\-](?!\w)|[^\w\s.\-]")


def fold(text: str) -> str:
    """Canonical form for comparing transcripts: no tashkeel, unified alef/ya/ta-marbuta, ASCII digits,
    lowercase, no punctuation, single spaces."""
    text = unicodedata.normalize("NFKC", text)
    text = _TASHKEEL.sub("", text).translate(_FOLD).lower()
    return " ".join(_PUNCT.sub(" ", text).split())


def levenshtein(a, b) -> int:
    """Edit distance between two sequences (strings or token lists)."""
    if len(a) < len(b):
        a, b = b, a
    row = list(range(len(b) + 1))
    for i, x in enumerate(a, 1):
        prev, row[0] = row[0], i
        for j, y in enumerate(b, 1):
            prev, row[j] = row[j], min(row[j] + 1, row[j - 1] + 1, prev + (x != y))
    return row[-1]


def is_arabic(token: str) -> bool:
    return bool(ARABIC.search(token))


def is_latin(token: str) -> bool:
    return bool(LATIN.search(token))
