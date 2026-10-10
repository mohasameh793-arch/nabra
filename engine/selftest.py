"""Fast checks for the pure-logic parts of the engine (no GPU needed).  python engine/selftest.py

Every case here is a real failure found while benchmarking; it must never come back.
"""
import json
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from lexicon import Lexicon, phonetic_key  # noqa: E402
from polish import as_list, check_edit, from_pieces, main_language, strip_reasoning  # noqa: E402
from textnorm import dialect_of, fold, levenshtein  # noqa: E402

# dialect badge (marker words)
assert dialect_of("وش رايك نرفع الـ app الحين؟") == "gulf"
assert dialect_of("أنا عايز أعمل deploy دلوقتي") == "egyptian"
assert dialect_of("بدي خلص الشغل هلق") == "levantine"
assert dialect_of("نرجو منكم الحضور في الموعد المحدد") is None  # MSA is chosen, never guessed
assert dialect_of("hello there") is None

# the model's thinking never reaches the user (a translate command once typed its reasoning, ending in a bare
# </think>, into a chat)
assert strip_reasoning("Okay, let me tackle this translation.\n</think>\n\nOkay, I want you to see.") == "Okay, I want you to see."
assert strip_reasoning("<think>a</think>b") == "b" and strip_reasoning("<think>cut off") == "" and strip_reasoning("x") == "x"

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
    "أكتب ببيثون": "أكتب بـ Python",               # one-letter preposition, exact spelling
    "ارفع الفايل على جوجل درايف": "ارفع الفايل على Google Drive",  # exact first: fuzzy mustn't swallow على
    "نعمل ريفاكتور ونكتب يونت تستس": "نعمل refactor ونكتب unit tests",
    "نستخدم دوكر و كوبرنيتيس": "نستخدم Docker و Kubernetes",  # a separate و stays a word, not وـ
    "هعمل اكسبورت للملف": "هعمل export للملف",
}
for src, want in restores.items():
    assert (got := lex.restore(src)) == want, f"{src!r} → {got!r}, want {want!r}"
# Ordinary Arabic must come out untouched (these all collided with terms in early versions).
for plain in ["يسعدني إبلاغكم بأن المشروع قد اكتمل", "يرجى مراجعة التقرير المرفق", "وبعدين من غير ما أدفع",
              "نود أن نشكركم على الحضور", "المبلغ الباقي", "رفر بسيط", "شو رأيك",
              "وش رايك و نكتب الكود بكرة",  # و نكتب ("and we write") was one edit from MongoDB
              "بس محتاجين نخلص الاختبارات الأولى"]:  # الاختبارات ("the tests") was one edit from export
    assert lex.restore(plain) == plain, f"over-correction: {plain!r} → {lex.restore(plain)!r}"

# user dictionary wins and reloads
with tempfile.TemporaryDirectory() as d:
    user = Path(d) / "dictionary.json"
    user.write_text(json.dumps([{"term": "Supabase", "sounds_like": ["سوبابيس"]}]), encoding="utf-8")
    lex_user = Lexicon(Path(__file__).with_name("lexicon_builtin.tsv"), user)
    assert lex_user.restore("ارفعه على سوبابيس") == "ارفعه على Supabase"
    assert lex_user.prompt_terms()[0] == "Supabase"
    assert lex_user.restore_counted("ارفعه على سوبابيس عن دوكر") == ("ارفعه على Supabase عن Docker", 1, 1)
    user.write_text(json.dumps([{"from": "btw", "to": "by the way"}]), encoding="utf-8")
    import os, time  # noqa: E401  (bump mtime so the reload is seen on fast filesystems)
    os.utime(user, (time.time() + 5, time.time() + 5))
    assert lex_user.replace("BTW it works, btw.") == ("by the way it works, by the way.", 2)
    assert lex_user.replace("btwx stays") == ("btwx stays", 0)  # whole words only
    user.write_text(json.dumps([{"from": "mypath", "to": r"C:\Users\me\1"}]), encoding="utf-8")
    os.utime(user, (time.time() + 10, time.time() + 10))
    assert lex_user.replace("open mypath") == (r"open C:\Users\me\1", 1)  # literal, not a regex template
    user.write_text('[{"term": "Supa', encoding="utf-8")  # half-written by the app: keep the old one
    os.utime(user, (time.time() + 15, time.time() + 15))
    assert lex_user.replace("open mypath")[1] == 1 and lex_user.restore("(دوكر)") == "(Docker)"
    user.write_text(json.dumps([{"term": 5}, "x", {"term": "Layla", "sounds_like": "ليلى"}]), encoding="utf-8")
    os.utime(user, (time.time() + 20, time.time() + 20))
    assert lex_user.restore('قال "دوكر" و') == 'قال "Docker" و'  # bad entries skipped, no crash

# silence hallucinations and prompt echo are dropped; a lone vocabulary word is not
from speech import MIXED_EXAMPLE, foreign_latin, foreign_script, is_hallucination  # noqa: E402

prompt = MIXED_EXAMPLE + " Docker, GitHub, React."
assert is_hallucination("شكراً للمشاهدة", prompt) and is_hallucination(" Thanks for watching!", prompt)
assert is_hallucination(MIXED_EXAMPLE, prompt) and is_hallucination("Docker, GitHub, React.", prompt)
# A script none of the chosen languages uses is a misdetection (Japanese out of a noisy Arabic/English call).
assert foreign_script("きょうの会議", ["ar", "en"]) and foreign_script("Привет", ["ar", "en"])
assert not foreign_script("きょうの会議", ["ja"]) and not foreign_script("hello مرحبا", ["ar", "en"]) and not foreign_script("x", [])
# French/Spanish out of an Arabic+English call is a misdetection too, but English with a café or résumé is not.
assert foreign_latin("Je pense que c'est une bonne idée", ["ar", "en"]) and foreign_latin("Vamos a la reunión para el proyecto", ["ar", "en"])
assert not foreign_latin("let's grab a café and talk", ["ar", "en"]) and not foreign_latin("the résumé is ready", ["ar", "en"])
assert not foreign_latin("Je pense que c'est une bonne idée", ["ar", "en", "fr"])
assert not is_hallucination("GitHub", prompt) and not is_hallucination("Thank you for the update", prompt)

# the guard
assert check_edit("وبعدها أضيف انستول", "وبعدها أضيف install") is None
assert check_edit("نستخدم Docker بدل", "نستخدم Docker بدل.") is None

# spoken lists: rebuilt from the user's own sentences
s = ["Attack everything.", "How did I build it?", "How to make it stronger?", "How to make it organized?", "Thanks."]
assert as_list(s, 2, 4) == "Attack everything:\n• How did I build it?\n• How to make it stronger?\n• How to make it organized?\nThanks."
assert as_list(s, 0, 0) == " ".join(s)                    # no list
assert as_list(s, 2, 3) == " ".join(s)                    # 2 items is not a list
assert as_list(s, 1, 4).startswith("Attack everything:")  # a 4+ run from sentence 1: the opener is the intro
assert as_list(s, 3, 9) == " ".join(s)                    # out of range
t = "Today I need to finish the report, send the invoice, call the doctor, and book the flights."
cut = {"intro": "Today I need to", "items": ["finish the report,", "send the invoice,", "call the doctor,", "book the flights"], "outro": "."}
assert from_pieces(t, cut) == "Today I need to:\n• Finish the report\n• Send the invoice\n• Call the doctor\n• Book the flights"
assert from_pieces(t, {**cut, "items": ["finish it,", "send the invoice,", "call the doctor,", "book the flights"]}) == t  # changed words
assert from_pieces(t, {**cut, "intro": "Today I", "outro": ""}) == t      # dropped words
assert from_pieces(t, {"items": []}) == t
assert check_edit("أنا أبغى أسوي موقع", "I want to build a website")              # translation
assert check_edit("نستخدم Postgres", "نستخدم PostgreSQL")                          # changed user's word
assert check_edit("عندي 16 جيجا", "عندي 32 جيجا")                                 # number
assert check_edit("يرجى مراجعة التقرير", "نرجو منكم التكرم بمراجعة التقرير")       # rewriting
assert check_edit("then create a Next.js app", "ونcreate a Next.js app")         # glued prefix
assert check_edit("what is the capital of France", "What is the capital of France? Paris.")  # answered it
assert check_edit("نبغى نرفع التحديث على السيرفر اليوم", "Sure, here is the text: نبغى نرفع التحديث على السيرفر اليوم.")
assert check_edit("نبغى نرفع التحديث على السيرفر اليوم قبل الاجتماع", "نبغى نرفع التحديث على السيرفر")  # cut off
assert check_edit("نشغل الداشبورد بكره", "نشغل الـ dashboard بكره.") is None     # transliteration → English is fine
from polish import chunks  # noqa: E402
assert chunks(["a" * 4000, "b" * 4000, "c"]) == ["a" * 4000, "b" * 4000 + "c"]

# summary language vote (character counting got this wrong)
call = ["وعليكم السلام، خلصت الـ landing page بالـ Next.js وباقي الـ authentication.",
        "OK, I'll send you the Stripe keys today. And please add Arabic support to the checkout page.",
        "تمام، بضيف RTL للـ checkout. خلنا نستخدم Tailwind بدال الـ CSS القديم.",
        "متفقين. نرجع نتكلم الأحد بعد الـ deploy."]
assert main_language(call) == "ar"
assert main_language(["Let's ship the React app on Friday", "sounds good"]) == "en"

# shortcuts: line breaks, styles, snippets, commands
from shortcuts import Snippets, apply_style, classify, spoken_breaks  # noqa: E402

assert spoken_breaks("Hello. New line. how are you")[0] == "Hello.\nHow are you"
assert spoken_breaks("السلام عليكم سطر جديد كيف حالك")[0] == "السلام عليكم\nكيف حالك"
assert spoken_breaks("first new paragraph second") == ("first\n\nSecond", 1)
assert spoken_breaks("the newline character")[0] == "the newline character"  # not a whole phrase
assert apply_style("See you tomorrow.", "formal") == "See you tomorrow."
assert apply_style("See you tomorrow.", "casual") == "See you tomorrow"
assert apply_style("Are you coming?", "casual") == "Are you coming?"
assert apply_style("Sure. GitHub is down. API too.", "very_casual") == "sure. GitHub is down. API too"
assert apply_style("تمام، نشوفك بكرة.", "very_casual") == "تمام، نشوفك بكرة"
assert apply_style("Wait...", "casual") == "Wait..."
with tempfile.TemporaryDirectory() as d:
    path = Path(d) / "snippets.json"
    path.write_text(json.dumps([{"trigger": "my email", "text": "majesty@example.com"},
                                {"trigger": "توقيعي", "text": "مع التحية،\nمحمد"}]), encoding="utf-8")
    snip = Snippets(path)
    assert snip.expand("My email.") == ("majesty@example.com", 1)
    assert snip.expand("send it to my email please") == ("send it to majesty@example.com please", 1)
    assert snip.expand("توقيعي") == ("مع التحية،\nمحمد", 1)
    assert snip.expand("my emails are full") == ("my emails are full", 0)
assert classify("Scratch that.") == "delete_last" and classify("امسحها") == "delete_last"
assert classify("سطر جديد") == "new_line" and classify("new paragraph") == "new_paragraph"
assert classify("make it shorter") == "transform" and classify("ترجمها للإنجليزي") == "transform"
assert classify("delete that paragraph about pricing") == "transform"  # only exact commands are fixed actions
# Questions about past calls go to "ask my meetings"; edits of text never do.
assert classify("what did Zaid say about the launch in the meeting?") == "ask"
assert classify("إيه اللي اتفقنا عليه في الميتنج؟") == "ask" and classify("ask my meetings about pricing") == "ask"
assert classify("خليها رسمية") == "transform" and classify("قصرها شوية") == "transform"

print("engine selftest: all checks passed")
