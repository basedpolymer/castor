"""Insert translations into a locale YAML after each key's `fr:` line.

Usage: py -I tools/i18n/merge_translations.py locales/x.yml batch.json [batch2.json ...]
batch.json = {key: {lang: text}}. Existing comments/order are preserved; languages
already present for a key are skipped.
"""
import json
import re
import sys

LANGS = ["de", "es", "it", "pt-BR", "ja", "zh-CN", "ko"]


def q(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'


def main():
    path, batches = sys.argv[1], sys.argv[2:]
    tr = {}
    for b in batches:
        with open(b, encoding="utf-8") as f:
            if b.endswith(".json"):
                tr.update(json.load(f))
            else:  # text: key || de || es || it || pt-BR || ja || zh-CN || ko
                for ln in f:
                    ln = ln.strip()
                    if not ln.strip():
                        continue
                    parts = ["" if x.strip() == "∅" else x.strip() for x in ln.split(" || ")]
                    assert len(parts) == 8, ln
                    tr[parts[0]] = dict(zip(LANGS, parts[1:]))
    with open(path, encoding="utf-8", newline="") as f:
        text = f.read()
    nl = "\r\n" if "\r\n" in text else "\n"
    lines = text.split(nl)
    out, key, present, used = [], None, set(), set()
    for ln in lines:
        m = re.match(r"^([A-Za-z0-9_.\-]+):\s*$", ln)
        if m:
            key, present = m.group(1), set()
        m2 = re.match(r"^  ([A-Za-z\-]+):", ln)
        if m2 and key:
            present.add(m2.group(1))
        out.append(ln)
        if m2 and m2.group(1) == "fr" and key in tr and not any(
            l in present for l in LANGS
        ):
            for l in LANGS:
                if l in tr[key]:
                    out.append(f"  {l}: {q(tr[key][l])}")
            used.add(key)
    unused = set(tr) - used
    if unused:
        print("not inserted:", sorted(unused)[:10], len(unused))
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(nl.join(out))
    print("inserted", len(used))


main()
