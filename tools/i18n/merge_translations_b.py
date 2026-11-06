"""Insert translations after the `fr:` line of each key.
Usage: py -I tools/i18n/merge_translations_b.py file.yml translations.json [...more json]
JSON: {key: {lang: text}}. Existing languages already present are left alone."""
import json, re, sys
ORDER = ["de", "es", "it", "pt-BR", "ja", "zh-CN", "ko"]
path = sys.argv[1]
tr = {}
for p in sys.argv[2:]:
    with open(p, encoding="utf-8") as f:
        for k, v in json.load(f).items():
            tr.setdefault(k, {}).update(v)
lines = open(path, encoding="utf-8").read().split("\n")
out, key, have, done = [], None, set(), 0
for i, ln in enumerate(lines):
    m = re.match(r"^([^\s#][^:]*):\s*$", ln)
    if m: key = m.group(1); have = set()
    m2 = re.match(r"^  ([A-Za-z-]+):", ln)
    if m2: have.add(m2.group(1))
    out.append(ln)
    if m2 and m2.group(1) == "fr" and key in tr:
        for l in ORDER:
            if l in tr[key] and l not in have:
                out.append("  %s: %s" % (l, json.dumps(tr[key][l], ensure_ascii=False)))
        done += 1
open(path, "w", encoding="utf-8", newline="").write("\n".join(out))
print("merged", done, "keys")
