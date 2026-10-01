"""Local LLM passes (Qwen3 via llama.cpp's OpenAI-compatible server): cleanup + guard, and call summaries."""
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

SUMMARY_RULES = """You summarize a call transcript. Lines start with THEM (other people) or ME (the user).
Write everything in {language}; keep technical terms, products, and names exactly as spoken.
Only use what is in the transcript. Never invent names, dates, or numbers.
Separate what is DONE from what is still PENDING. Pending markers include "باقي", "لسه", "still", "not yet",
and future forms: Gulf/Levantine "بـ" + verb (بضيف = I will add), "راح"/"رح", "ح"/"هـ" (Egyptian), "سـ"/"سوف",
"I'll", "will", "going to". Only mark something done when the speaker says it is finished (خلصت، سويت، done).
In action items write the user as "I"/"أنا" (in the summary language) and others as "they"/"هم",
never the labels ME or THEM.
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
        self.http = httpx.Client()

    def ready(self) -> bool:
        try:
            return self.http.get(f"{self.base_url}/health", timeout=1).status_code == 200
        except httpx.HTTPError:
            return False

    def chat(self, messages: list[dict], max_tokens: int, reasoning: bool = False, timeout: float = 60) -> str:
        r = self.http.post(f"{self.base_url}/v1/chat/completions", timeout=timeout, json={
            "messages": messages, "temperature": 0, "max_tokens": max_tokens,
            "chat_template_kwargs": {"enable_thinking": reasoning},
        })
        r.raise_for_status()
        text = r.json()["choices"][0]["message"]["content"]
        return re.sub(r"<think>.*?</think>", "", text, flags=re.S).strip()

    def cleanup(self, text: str, vocabulary: list[str]) -> str:
        messages = [{"role": "system", "content": f"{CLEANUP_RULES}\nPreferred spellings: {', '.join(vocabulary)}"}]
        for before, after in CLEANUP_EXAMPLES:
            messages += [{"role": "user", "content": before}, {"role": "assistant", "content": after}]
        messages.append({"role": "user", "content": text})
        return self.chat(messages, max_tokens=400)

    def summarize(self, lines: list[dict], language: str | None) -> str:
        spoken = [l for l in lines if l.get("text", "").strip()]
        if not spoken:
            return ""
        code = language or main_language([l["text"] for l in spoken])
        system = SUMMARY_RULES.format(language=LANGUAGE_NAMES.get(code, code))
        parts, buf = [], ""
        for l in spoken:
            row = f"{'ME' if l['who'] == 'you' else 'THEM'}: {l['text']}\n"
            if buf and len(buf) + len(row) > PART_CHARS:
                parts.append(buf)
                buf = ""
            buf += row
        parts.append(buf)

        def ask(body: str) -> str:
            # Reasoning on: a few seconds slower, but pending work stops being reported as finished.
            return self.chat([{"role": "system", "content": system}, {"role": "user", "content": body}],
                             max_tokens=4000, reasoning=True, timeout=300)

        if len(parts) == 1:
            return ask(parts[0])
        partials = [ask(f"Part {i + 1} of {len(parts)} of one call:\n{p}") for i, p in enumerate(parts)]
        return ask("Merge these summaries of consecutive parts of one call into one summary:\n\n" + "\n\n".join(partials))

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
        ], max_tokens=1500, timeout=120)
        return out.strip().strip('"“”«»')

    def title(self, summary: str) -> str:
        """A short title for a note, in the summary's own language."""
        lang = LANGUAGE_NAMES.get(main_language(summary.splitlines()), "English")
        text = self.chat([
            {"role": "system", "content": f"Write a 2–6 word title for this call in {lang}. "
                                          "Reply with the title only, no quotes or punctuation at the end."},
            {"role": "user", "content": summary[:3000]},
        ], max_tokens=30)
        return text.strip().strip('"«»').splitlines()[0][:80] if text.strip() else ""

    def ask(self, question: str, notes: list[dict]) -> str:
        """Answer a question from the user's own saved call summaries (newest first)."""
        context = "\n\n".join(f"### {n.get('title') or 'Call'} ({n.get('date', '')})\n{n.get('summary', '')}"
                              for n in notes if n.get("summary"))[:12000]
        if not context:
            return ""
        lang = LANGUAGE_NAMES.get(main_language([question]), "English")
        return self.chat([
            {"role": "system", "content": "Answer the user's question using ONLY these summaries of their calls. "
                                          "If the answer isn't there, say so. Be brief. Name the call you used. "
                                          f"Answer in {lang}.\n\n{context}"},
            {"role": "user", "content": question},
        ], max_tokens=1500, reasoning=True, timeout=180)


def main_language(lines: list[str]) -> str:
    """Each line votes Arabic or English with its word count. (Counting characters fails: English terms
    inside Arabic sentences outweigh the Arabic.)"""
    votes = {"ar": 0, "en": 0}
    for line in lines:
        words = line.split()
        arabic = sum(is_arabic(w) for w in words)
        votes["ar" if 2 * arabic >= len(words) else "en"] += len(words)
    return max(votes, key=votes.get)


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
    if b_ar and sum(t not in a_ar for t in b_ar) / len(b_ar) > 0.4:
        return "removed most Arabic words (translation?)"
    a_lat = {t for t in a if is_latin(t)}
    if lost := [t for t in b if is_latin(t) and t not in a_lat]:
        return f"dropped English words {lost}"
    if re.findall(r"\d+", before) != re.findall(r"\d+", after):
        return "changed numbers"
    return None
