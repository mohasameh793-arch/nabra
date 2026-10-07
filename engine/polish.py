"""Local LLM passes (Qwen3 via llama.cpp's OpenAI-compatible server): cleanup + guard, and call summaries."""
import json
import os
import re

import httpx

from textnorm import fold, is_arabic, is_latin, levenshtein

CLEANUP_RULES = """You fix dictated text. The speaker mixes Arabic (any dialect) with English and other languages.
- Never translate. Every word stays in the language it was spoken in, dialect words included (أبغى، عايز، بدي، وش).
- English or technical words written in Arabic letters by the transcriber go back to English spelling,
  but only when you are sure (كوبرنيتيس → Kubernetes, الداشبورد → الـ dashboard).
- Before an English word, write the Arabic article as "الـ " (الـ API، للـ app، بالـ React).
- Fix punctuation and capitalization. Do not rephrase, reorder, add, remove, or correct grammar.
- When unsure, leave the word as it is.
Reply with the fixed text only."""

# Few-shot pairs deliberately avoid words used in the benchmark sentences.
CLEANUP_EXAMPLES = [
    ("وش رايك نرفع الكونتينر على السيرفر الجديد بكره", "وش رايك نرفع الـ container على السيرفر الجديد بكره؟"),
    ("can you send me the slides before the meeting", "Can you send me the slides before the meeting?"),
    ("نرجو منكم الحضور في الموعد المحدد", "نرجو منكم الحضور في الموعد المحدد."),
]

LIST_RULES = """The user dictated some text, split into numbered sentences. Find the run of 3 or more CONSECUTIVE
sentences that are parallel list items: a series of questions, tasks, steps or points of the same kind. Normal prose,
a story or an argument is not a list. A sentence that announces the items ("I have a few things.", "عندي كذا شغلة.")
is NOT an item. Two alternatives ("today? or tomorrow?") and closing remarks ("tell me", "thanks") are not items.
Reply with JSON only: {"first": n, "last": n} for the run, or {"first": 0, "last": 0} if there is none."""
LIST_EXAMPLES = [
    ("1. Before the launch we have work.\n2. Fix the login.\n3. Update the docs.\n4. Test on Android.\n5. Thanks.",
     '{"first": 2, "last": 4}'),
    ("1. نسافر الخميس؟\n2. ولا الجمعة؟\n3. قول لي.", '{"first": 0, "last": 0}'),
]
SENTENCE_END = re.compile(r"(?<=[.?!؟])\s+")

SUMMARY_RULES = """You summarize a call transcript. Lines start with ME (the user), a person's name, or THEM (someone
else whose name isn't known).
Write everything in {language}; keep technical terms, products, and names exactly as spoken.
Only use what is in the transcript. Never invent names, dates, or numbers.
Separate what is DONE from what is still PENDING. Pending markers include "باقي", "لسه", "still", "not yet",
and future forms: Gulf/Levantine "بـ" + verb (بضيف = I will add), "راح"/"رح", "ح"/"هـ" (Egyptian), "سـ"/"سوف",
"I'll", "will", "going to". Only mark something done when the speaker says it is finished (خلصت، سويت، done).
In action items write the user as "I"/"أنا" (in the summary language), named people by their name, and others
as "they"/"هم", never the labels ME or THEM.
Use Markdown with these sections, headings translated into {language}, skipping empty ones:
## Summary
(two or three sentences)
## Key points
## Decisions
## Action items
- [ ] who → what (deadline if one was said)"""

LANGUAGE_NAMES = {"ar": "Arabic", "en": "English", "fr": "French", "es": "Spanish", "de": "German",
                  "ja": "Japanese", "tr": "Turkish", "ur": "Urdu", "hi": "Hindi", "fa": "Persian",
                  "zh": "Chinese", "ko": "Korean", "ru": "Russian", "pt": "Portuguese", "it": "Italian",
                  "id": "Indonesian"}
PART_CHARS = 6000  # transcript characters per summary request (fits an 8k context with room to answer)


class Llm:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")
        key = os.environ.get("NABRA_LLM_KEY", "")  # llama-server runs with --api-key: only Nabra can use it
        self.http = httpx.Client(headers={"Authorization": f"Bearer {key}"} if key else {})

    def ready(self) -> bool:
        try:
            return self.http.get(f"{self.base_url}/health", timeout=1).status_code == 200
        except httpx.HTTPError:
            return False

    def chat(self, messages: list[dict], max_tokens: int, reasoning: bool = False, timeout: float = 60,
             whole: bool = False) -> str:
        """whole: the reply replaces the user's text, so one cut off at max_tokens raises instead of being returned."""
        r = self.http.post(f"{self.base_url}/v1/chat/completions", timeout=timeout, json={
            "messages": messages, "temperature": 0, "max_tokens": max_tokens,
            "chat_template_kwargs": {"enable_thinking": reasoning},
        })
        r.raise_for_status()
        choice = r.json()["choices"][0]
        if whole and choice.get("finish_reason") == "length":
            raise ValueError("LLM reply was cut off")
        text = re.sub(r"<think>.*?</think>", "", choice["message"]["content"], flags=re.S)
        return re.sub(r"<think>.*", "", text, flags=re.S).strip()  # reasoning cut off before </think>: drop it

    def cleanup(self, text: str, vocabulary: list[str]) -> str:
        messages = [{"role": "system", "content": f"{CLEANUP_RULES}\nPreferred spellings: {', '.join(vocabulary)}"}]
        for before, after in CLEANUP_EXAMPLES:
            messages += [{"role": "user", "content": before}, {"role": "assistant", "content": after}]
        messages.append({"role": "user", "content": text})
        return self.chat(messages, max_tokens=400, whole=True)

    def lists(self, text: str) -> str:
        """Spoken enumerations become "- " bullet lists (like Wispr Flow). The LLM only picks which sentences
        are items; the text is rebuilt here from the user's own sentences, so no word can change."""
        sentences = SENTENCE_END.split(text.strip())
        if len(sentences) < 3:
            return text
        messages = [{"role": "system", "content": LIST_RULES}]
        for numbered, answer in LIST_EXAMPLES:
            messages += [{"role": "user", "content": numbered}, {"role": "assistant", "content": answer}]
        numbered = "\n".join(f"{i + 1}. {one_line(s)}" for i, s in enumerate(sentences))
        reply = self.chat(messages + [{"role": "user", "content": numbered}], max_tokens=40)
        found = re.search(r"\{.*?\}", reply, re.S)
        run = json.loads(found[0]) if found else {}
        return as_list(sentences, int(run.get("first", 0)), int(run.get("last", 0)))

    def summarize(self, lines: list[dict], language: str | None) -> str:
        spoken = [l for l in lines if l.get("text", "").strip()]
        if not spoken:
            return ""
        code = language or main_language([l["text"] for l in spoken])
        system = SUMMARY_RULES.format(language=LANGUAGE_NAMES.get(code, code))
        parts = chunks([f"{speaker(l)}: {one_line(l['text'])}\n" for l in spoken])

        def ask(body: str) -> str:
            # Reasoning on: a few seconds slower, but pending work stops being reported as finished.
            return self.chat([{"role": "system", "content": system}, {"role": "user", "content": body}],
                             max_tokens=4000, reasoning=True, timeout=300)

        if len(parts) == 1:
            return ask(parts[0])
        partials = [ask(f"Part {i + 1} of {len(parts)} of one call:\n{p}") for i, p in enumerate(parts)]
        while len(partials) > 1:  # merge in batches that fit the context, not all at once
            groups = chunks([p + "\n\n" for p in partials])
            if len(groups) == len(partials):  # each partial is already long: merge them in pairs
                groups = ["\n\n".join(partials[i:i + 2]) for i in range(0, len(partials), 2)]
            partials = [ask("Merge these summaries of consecutive parts of one call into one summary:\n\n" + g)
                        for g in groups]
        return partials[0]

    def transform(self, text: str, instruction: str) -> str:
        """Rewrite `text` as the user explicitly asked ("make it shorter", "ترجمها للإنجليزي").
        This is the only place rewriting or translating is allowed: the user requested it."""
        out = self.chat([
            {"role": "system", "content": "You edit text exactly as the user instructs. The instruction may be in Arabic or "
                                          "English. Actually make the change: rewriting, shortening, changing tone or "
                                          "translating are all expected. Write the result in the same language as the text "
                                          "unless the instruction asks for another language. Keep names, numbers, URLs and "
                                          "code exact. Reply with the edited text only: no quotes, no explanations."},
            {"role": "user", "content": "Instruction: خلها رسمية\n\nText:\nيا شباب بكرة الاجتماع الساعة ٩ لا تتأخرون"},
            {"role": "assistant", "content": "نود تذكيركم بأن الاجتماع سيُعقد غداً في تمام الساعة ٩، ونرجو الالتزام بالموعد."},
            {"role": "user", "content": f"Instruction: {instruction}\n\nText:\n{text}"},
        ], max_tokens=1500, timeout=120, whole=True)
        quotes = '"“”«»'
        return out.strip() if text.strip()[:1] in quotes else out.strip().strip(quotes)  # keep the user's own quotes

    def catch_up(self, lines: list[dict], me: str = "") -> str:
        """The last few minutes of a live call as 3 short bullets, in the call's own language."""
        spoken = [l for l in lines if l.get("text", "").strip()]
        if not spoken:
            return ""
        lang = LANGUAGE_NAMES.get(main_language([l["text"] for l in spoken]), "English")
        rows = "\n".join(f"{speaker(l)}: {one_line(l['text'])}" for l in spoken)
        return self.chat([
            {"role": "system", "content": f"Someone stepped away from a call for a few minutes. In {lang}, write exactly 3 "
                                          "short bullets (\"- \") with what they missed: what was said, decided or asked. "
                                          "If someone asked ME something, make that the first bullet. Use only the transcript; "
                                          "keep names and numbers exact; refer to ME as \"you\"."
                                          + (f" ME's name is {me}: when someone addresses {me}, they mean you." if me else "")},
            {"role": "user", "content": rows},
        ], max_tokens=300, timeout=90)

    def ask(self, question: str, snippets: list[dict]) -> str:
        """Answer a question about past calls using only the given transcript snippets, citing them as [n]."""
        rows = "\n".join(f"[{s['n']}] {one_line(s.get('date', ''))} · {one_line(s.get('title', ''))} · "
                         f"{one_line(s.get('who', ''))}: {one_line(s['text'])}" for s in snippets)
        cited = {str(s["n"]) for s in snippets}
        out = self.chat([
            {"role": "system", "content": "Answer the user's question about their past calls using ONLY the numbered transcript "
                                          "lines. Answer in the language of the question, in 1–3 sentences, and cite the lines you "
                                          "used like [3]. If the lines don't contain the answer, say you couldn't find it in the "
                                          "notes. Never invent names, dates, numbers or decisions."},
            {"role": "user", "content": f"Transcript lines:\n{rows}\n\nQuestion: {question}"},
        ], max_tokens=300, timeout=90)
        return re.sub(r"\[(\d+)\]", lambda m: m[0] if m[1] in cited else "", out)  # only citations that exist

    def title(self, summary: str) -> str:
        """A short title for a note, in the summary's own language."""
        lang = LANGUAGE_NAMES.get(main_language(summary.splitlines()), "English")
        text = self.chat([
            {"role": "system", "content": f"Write a 2–6 word title for this call in {lang}. "
                                          "Reply with the title only, no quotes or punctuation at the end."},
            {"role": "user", "content": summary[:3000]},
        ], max_tokens=30)
        return text.strip().strip('"«»').splitlines()[0][:80] if text.strip() else ""


def main_language(lines: list[str]) -> str:
    """Each line votes Arabic or English with its word count. (Counting characters fails: English terms
    inside Arabic sentences outweigh the Arabic.)"""
    votes = {"ar": 0, "en": 0}
    for line in lines:
        words = line.split()
        arabic = sum(is_arabic(w) for w in words)
        votes["ar" if 2 * arabic >= len(words) else "en"] += len(words)
    return max(votes, key=votes.get)


def as_list(sentences: list[str], first: int, last: int) -> str:
    """Sentences first..last (1-based) as "- " lines; the sentences before them become the intro, ending in ":"."""
    if not (1 <= first and last - first >= 2 and last <= len(sentences)):
        return " ".join(sentences)
    if first == 1 and last - first >= 3:
        # ponytail: the 8B model often counts a short opener ("For tomorrow.") as an item; with 4+ items it's the intro.
        first = 2
    intro = " ".join(sentences[:first - 1])
    lines = [re.sub(r"[.،,]$", "", intro) + ":"] if intro else []
    lines += [f"- {s}" for s in sentences[first - 1:last]]
    if rest := " ".join(sentences[last:]):
        lines.append(rest)
    return "\n".join(lines)


def one_line(text) -> str:
    """Transcript text is untrusted: a newline inside it could fake a new "ME:" row in the prompt."""
    return " ".join(str(text).split())


def speaker(line: dict) -> str:
    return "ME" if line.get("who") == "you" else (one_line(line.get("name") or "") or "THEM")


def chunks(rows: list[str]) -> list[str]:
    """Rows joined into pieces of at most PART_CHARS characters (a longer row gets a piece of its own)."""
    parts, buf = [], ""
    for row in rows:
        if buf and len(buf) + len(row) > PART_CHARS:
            parts.append(buf)
            buf = ""
        buf += row
    parts.append(buf)
    return parts


ARTICLE_TOKENS = {"ال", "لل", "بال", "وال"}


def changed_words(before: str, after: str) -> int:
    """How many words an accepted cleanup actually changed (for the Insights "fixes" counter)."""
    return levenshtein(fold(before).split(), fold(after).split())


def check_edit(before: str, after: str) -> str | None:
    """Why the LLM edit must be rejected, or None if it looks like cleanup rather than rewriting."""
    b, a = fold(before).split(), fold(after).split()
    b_ar, a_ar = [t for t in b if is_arabic(t)], [t for t in a if is_arabic(t)]
    invented = [t for t in a_ar if t not in b_ar and t not in ARTICLE_TOKENS]
    if len(invented) > max(1, len(b_ar) // 10):
        return f"invented Arabic words {invented}"
    lost_ar = sum(t not in a_ar for t in b_ar)
    if b_ar and lost_ar / len(b_ar) > 0.4:
        return "removed most Arabic words (translation?)"
    a_lat = {t for t in a if is_latin(t)}
    if lost := [t for t in b if is_latin(t) and t not in a_lat]:
        return f"dropped English words {lost}"
    # New English words may only replace Arabic-spelled ones (كوبرنيتيس → Kubernetes), never be added:
    # a preamble ("Sure, here is the text") or an answer to a dictated question.
    if len(added := {t for t in a_lat if t not in b}) > lost_ar + len(b) // 10:
        return f"added English words {sorted(added)}"
    n_b, n_a = (sum(t not in ARTICLE_TOKENS for t in words) for words in (b, a))
    if n_a > n_b * 1.25 + 3 or n_a < n_b * 0.8:  # e.g. a chatty addition, or a reply cut off early
        return "added or removed words"
    if re.findall(r"\d+", before) != re.findall(r"\d+", after):
        return "changed numbers"
    return None
