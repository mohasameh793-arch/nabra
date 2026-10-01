"""Fast checks for the pure-logic parts of the engine (no GPU needed).  python engine/selftest.py

Every case here is a real failure found while benchmarking; it must never come back.
"""
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from lexicon import Lexicon, phonetic_key  # noqa: E402
from polish import check_edit, main_language  # noqa: E402
from textnorm import fold, levenshtein  # noqa: E402

# textnorm
assert fold("إِنَّ الْمَدْرَسَةَ") == fold("ان المدرسه")
assert fold("Hello, Next.js!") == "hello next.js"
assert fold("٣ و ۳") == "3 و 3"
assert levenshtein("kitten", "sitting") == 3 and levenshtein([], ["a"]) == 1

# phonetic keys meet across scripts
assert phonetic_key("دوكر") == phonetic_key("Docker") == "tkr"
assert phonetic_key("نكست جي اس") == phonetic_key("Next.js")
assert phonetic_key("كتهاب") == phonetic_key("GitHub")  # CamelCase split: no "th" digraph

lex = Lexicon(Path(__file__).with_name("lexicon_builtin.tsv"))
restores = {
    "نستخدم دوكر بدل": "نستخدم Docker بدل",
    "افتح لي فيس كود": "افتح لي VS Code",
    "بيثون سكريبت": "Python script",
    "على كتهاب وأنا": "على GitHub وأنا",
    "من الأبيأي ونخزنه": "من API ونخزنه",
    "باستخدام next.js": "باستخدام Next.js",
    "بالرياكت وأربطها": "بالـ React وأربطها",      # exact form beats fuzzy "pull request"
    "deploy للـ app": "deploy للـ app",             # Latin span never swallows the Arabic article
}
for src, want in restores.items():
    assert (got := lex.restore(src)) == want, f"{src!r} → {got!r}, want {want!r}"
# Ordinary Arabic must come out untouched (these all collided with terms in early versions).
for plain in ["يسعدني إبلاغكم بأن المشروع قد اكتمل", "يرجى مراجعة التقرير المرفق", "وبعدين من غير ما أدفع",
              "نود أن نشكركم على الحضور", "المبلغ الباقي", "رفر بسيط", "شو رأيك"]:
    assert lex.restore(plain) == plain, f"over-correction: {plain!r} → {lex.restore(plain)!r}"

# user dictionary wins and reloads
with tempfile.TemporaryDirectory() as d:
    user = Path(d) / "dictionary.json"
    user.write_text(json.dumps([{"term": "Majesty", "sounds_like": ["ماجستي"]}]), encoding="utf-8")
    lex_user = Lexicon(Path(__file__).with_name("lexicon_builtin.tsv"), user)
    assert lex_user.restore("كلم ماجستي") == "كلم Majesty"
    assert lex_user.prompt_terms()[0] == "Majesty"

# the guard
assert check_edit("وبعدها أضيف انستول", "وبعدها أضيف install") is None
assert check_edit("نستخدم Docker بدل", "نستخدم Docker بدل.") is None
assert check_edit("أنا أبغى أسوي موقع", "I want to build a website")              # translation
assert check_edit("نستخدم Postgres", "نستخدم PostgreSQL")                          # changed user's word
assert check_edit("عندي 16 جيجا", "عندي 32 جيجا")                                 # number
assert check_edit("يرجى مراجعة التقرير", "نرجو منكم التكرم بمراجعة التقرير")       # rewriting
assert check_edit("then create a Next.js app", "ونcreate a Next.js app")         # glued prefix

# summary language vote (character counting got this wrong)
call = ["وعليكم السلام، خلصت الـ landing page بالـ Next.js وباقي الـ authentication.",
        "OK, I'll send you the Stripe keys today. And please add Arabic support to the checkout page.",
        "تمام، بضيف RTL للـ checkout. خلنا نستخدم Tailwind بدال الـ CSS القديم.",
        "متفقين. نرجع نتكلم الأحد بعد الـ deploy."]
assert main_language(call) == "ar"
assert main_language(["Let's ship the React app on Friday", "sounds good"]) == "en"

print("engine selftest: all checks passed")
