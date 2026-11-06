"""Use full-width punctuation next to CJK text in zh-CN and ja lines (commas, semicolons, colons,
question and exclamation marks). Usage: py -I tools/i18n/fix_cjk_punct.py locales/x.yml ..."""
import re, sys
CJK = r"[぀-ヿ㐀-䶿一-鿿＀-￯]"
MAP = {"zh-CN": {",": "，", ";": "；", ":": "：", "?": "？", "!": "！"},
       "ja": {",": "、", "?": "？", "!": "！"}}
for path in sys.argv[1:]:
    lines = open(path, encoding="utf-8").read().split("\n")
    changed = 0
    for i, line in enumerate(lines):
        for lang, table in MAP.items():
            prefix = f"  {lang}: "
            if not line.startswith(prefix):
                continue
            body = line[len(prefix):]
            new = body
            for a, b in table.items():
                # punctuation directly after a CJK char, optionally followed by a space
                new = re.sub(f"({CJK}){re.escape(a)} ?", lambda m: m.group(1) + b, new)
            if new != body:
                lines[i] = prefix + new
                changed += 1
    open(path, "w", encoding="utf-8", newline="\n").write("\n".join(lines))
    print(f"{path}: {changed} lines fixed")
