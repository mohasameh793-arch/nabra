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


# Words only one dialect uses (folded spelling). MSA has no markers: it is never guessed, only chosen.
_DIALECT_WORDS = {
    # Left out on purpose: shared or ambiguous words (ليش, هلا, زين, ايه = "what"/"yes", ابي = "I want"/"my father").
    "gulf": {"وش", "ابغي", "ابغا", "تبي", "شلون", "وايد", "الحين", "يبي", "عساك", "هالحين", "شفيك"},
    "egyptian": {"عايز", "عاوز", "عايزه", "ازاي", "دلوقتي", "كده", "اوي", "بتاع", "بتاعي", "امبارح", "النهارده"},
    "levantine": {"بدي", "بدك", "شو", "هلق", "كتير", "منيح", "هيك", "مشان", "لكان", "هون"},
}


def dialect_of(text: str) -> str | None:
    """The Arabic dialect whose marker words appear most ("gulf" / "egyptian" / "levantine"), or None.
    ponytail: word lists, not a model; a short or neutral sentence gives None, and that's fine for a badge."""
    tokens = fold(text).split()
    tokens += [t[1:] for t in tokens if len(t) > 3 and t[0] in "وف"]  # وعايز → عايز
    hits = {d: sum(t in words for t in tokens) for d, words in _DIALECT_WORDS.items()}
    best = max(hits, key=hits.get)
    return best if hits[best] and list(hits.values()).count(hits[best]) == 1 else None
