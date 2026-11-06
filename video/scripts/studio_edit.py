"""Build a Trim & Caption Studio edit script (castor --page studio:media --script edit.json)
from the JSON written by `castor cli run audio.transcribe --set format=json --set vad=false`.

    py -I studio_edit.py transcript.json edit.json en|fr

Words and timings are whisper's own; caption texts get the few fixes a person would type in the
Captions tab (misheard words); two phrases are cut, as in the demo.
"""
import json
import sys

FIXES = {
    "en": [("mustard seat", "mustard seed"), ("Non-withstanding", "Notwithstanding"), ("humble mean", "humble mien")],
    "fr": [("peu sans", "peut sans"), ("servira en", "servir en"), ("« Secondum", "secundum"), ("Atom »", "artem"), ("Quoi que", "Quoique"),
           ("tous être ", "tous êtres "), ("pas unité", "pas une idée"),
           ("poser un auteur", "poser en auteur"), ("grand forfait", "grand profit"),
           ("pour autre huile", "pour autrui"), ("le rubricant", "le Rubicon"), ("me saisis", "me saisit")],
}
# Phrases cut in the demo (first and last word of the cut, matched in order).
CUTS = {
    "en": [("remote", "heart."), ("He", "this.")],
    "fr": [("car", "classique."), ("Et", "disloquer.")],
}


def main():
    src, dst, lang = sys.argv[1], sys.argv[2], sys.argv[3]
    t = json.load(open(src, encoding="utf-8"))
    words = [{"start": w["start"], "end": w["end"], "text": w["text"].strip()}
             for s in t["segments"] for w in s.get("words", []) if w["text"].strip()]
    dur = float(t.get("duration") or words[-1]["end"])

    # Cues: whole sentences when short, else split near 60 characters / 4.5 s.
    cues, cur = [], []
    for w in words:
        cur.append(w)
        text = " ".join(x["text"] for x in cur)
        end_sentence = w["text"][-1:] in ".!?»"
        if (end_sentence and len(text) > 18) or len(text) > 58 or cur[-1]["end"] - cur[0]["start"] > 4.5:
            cues.append(cur)
            cur = []
    if cur:
        cues.append(cur)
    out_cues = []
    # the first caption starts at 0:00, so the frame at the playhead shows it (a 0.04 s nudge)
    if cues:
        cues[0][0] = dict(cues[0][0], start=0.0)
    for c in cues:
        text = " ".join(x["text"] for x in c)
        for a, b in FIXES[lang]:
            text = text.replace(a, b)
        out_cues.append({"start": round(c[0]["start"], 2), "end": round(c[-1]["end"], 2), "text": text})

    # Cuts: from the start of the first word to the start of the word after the last one.
    keep, i = [(0.0, dur)], 0
    for first, last in CUTS[lang]:
        a = next(k for k in range(i, len(words)) if words[k]["text"] == first)
        b = next(k for k in range(a, len(words)) if words[k]["text"] == last)
        i = b + 1
        s, e = words[a]["start"], words[b + 1]["start"] if b + 1 < len(words) else dur
        nk = []
        for x, y in keep:
            if e <= x or s >= y:
                nk.append((x, y))
            else:
                if s > x:
                    nk.append((x, s))
                if e < y:
                    nk.append((e, y))
        keep = nk
    edit = {"keep": [[round(x, 2), round(y, 2)] for x, y in keep], "cut": "exact", "words": words, "cues": out_cues,
            "style": "box", "size": 6, "position": "bottom", "color": "#ffffffff"}
    json.dump(edit, open(dst, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    for c in out_cues:
        print(c["start"], c["end"], c["text"])
    print("keep", edit["keep"])


main()
