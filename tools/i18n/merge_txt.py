"""Convert batch txt files (key ¦ de ¦ es ¦ it ¦ pt-BR ¦ ja ¦ zh-CN ¦ ko) to one json.
Usage: py -I merge_txt.py out.json in1.txt [in2.txt ...]"""
import sys, json
L = ["de", "es", "it", "pt-BR", "ja", "zh-CN", "ko"]
out = {}
for path in sys.argv[2:]:
    for ln in open(path, encoding="utf-8"):
        ln = ln.rstrip("\n")
        if not ln.strip():
            continue
        p = [x.strip() for x in ln.split(" ¦ ")]
        if len(p) != 8:
            print("BAD", len(p), p[0])
            continue
        out[p[0]] = dict(zip(L, p[1:]))
json.dump(out, open(sys.argv[1], "w", encoding="utf-8"), ensure_ascii=False)
print(len(out))
