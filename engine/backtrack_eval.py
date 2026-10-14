"""Self-correction check: realistic dictations (English, Egyptian, Saudi, mixed) run through Llm.backtrack against a
local llama-server, compared with what the speaker meant. Negatives must come back unchanged.

  python backtrack_eval.py http://127.0.0.1:8799
"""
import re
import sys

from polish import Llm

CASES = [
    # (dictated, meant)
    ("Let's meet at 2, actually 3.", "Let's meet at 3."),
    ("Send it to Omar, no, to Lina.", "Send it to Lina."),
    ("Book the table for four, no wait, six people.", "Book the table for six people."),
    ("The meeting is on Monday, sorry, Tuesday.", "The meeting is on Tuesday."),
    ("Call me tomorrow, or rather tonight.", "Call me tonight."),
    ("We need 50, I mean 500 units.", "We need 500 units."),
    ("Put it in the blue folder. No, the red folder.", "Put it in the red folder."),
    ("Email John, scratch that, email Sarah about the report.", "Email Sarah about the report."),
    ("الاجتماع الساعة 2، لا 3.", "الاجتماع الساعة 3."),
    ("ابعتها لعمر، لا لا، لمنى.", "ابعتها لمنى."),
    ("هنتقابل يوم الحد، قصدي يوم الاتنين.", "هنتقابل يوم الاتنين."),
    ("هبعتلك الملف بكرة، لا استنى، النهاردة.", "هبعتلك الملف النهاردة."),
    ("عايز تلاتة، لأ خليهم أربعة.", "عايز أربعة."),
    ("نتقابل الساعة 4، لا لحظة، 5.", "نتقابل الساعة 5."),
    ("أرسلها لفهد، لا، لسعود.", "أرسلها لسعود."),
    ("أبي اثنين، لا، خلها ثلاثة.", "أبي ثلاثة."),
    ("الموعد الأحد، أقصد الاثنين.", "الموعد الاثنين."),
    ("ابعتلي الـ file يوم الحد، sorry، يوم الاتنين.", "ابعتلي الـ file يوم الاتنين."),
    ("الـ meeting الساعة 2، no wait، الساعة 3.", "الـ meeting الساعة 3."),
    # not self-corrections: must stay exactly as said
    ("I actually think the new design is better.", None),
    ("No problem, I'll send it today.", None),
    ("I mean it, this is the best version.", None),
    ("Sorry for the late reply, the file is attached.", None),
    ("لا، أنا مش موافق على الفكرة دي.", None),
    ("قصدي إن الفكرة حلوة بس محتاجة وقت.", None),
    ("لا لا، ده ممتاز جداً.", None),
    ("خلها على راحتك، مافي مشكلة.", None),
    ("I said no to the offer, then we left.", None),
    ("قلت له لا وخلصنا الموضوع.", None),
    ("Wait for me at the gate, I'm almost there.", None),
    ("خليها بكرة الصبح أحسن.", None),
]


def norm(s: str) -> str:
    return re.sub(r"[\s.,،!?؟]+", " ", s).strip().lower()


def main(url: str) -> None:
    llm, ok = Llm(url), 0
    for said, meant in CASES:
        got = llm.backtrack(said)
        good = norm(got) == norm(meant if meant is not None else said)
        ok += good
        print("PASS" if good else "FAIL", "|", said, "→", got)
    print(f"{ok}/{len(CASES)} correct")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:8799")
