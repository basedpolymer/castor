"""Check Castor locale files: every key has every language and the same %{placeholders}.

Usage: py -I tools/i18n/check_locales.py [file.yml ...]   (default: all locales/*.yml)
Needs PyYAML (`py -m pip install --user pyyaml`). Exit code 1 on any problem.
"""
import glob
import os
import re
import sys

import yaml

LANGS = ["en", "fr", "de", "es", "it", "pt-BR", "ja", "zh-CN", "ko"]
PH = re.compile(r"%\{[a-zA-Z0-9_]+\}")


def check(path, require_all):
    with open(path, encoding="utf-8") as f:
        data = yaml.safe_load(f)
    problems = []
    for key, val in data.items():
        if key == "_version":
            continue
        if not isinstance(val, dict) or "en" not in val:
            problems.append(f"{key}: no 'en'")
            continue
        want = sorted(PH.findall(str(val["en"])))
        langs = LANGS if require_all else [l for l in LANGS if l in val]
        for lang in langs:
            if lang not in val or val[lang] in (None, ""):
                problems.append(f"{key}: missing {lang}")
                continue
            got = sorted(PH.findall(str(val[lang])))
            if got != want:
                problems.append(f"{key} [{lang}]: placeholders {got} != {want}")
    return problems


def main():
    files = sys.argv[1:] or sorted(glob.glob(os.path.join(os.path.dirname(__file__), "..", "..", "locales", "*.yml")))
    total = 0
    for path in files:
        probs = check(path, require_all=True)
        missing = sum(1 for p in probs if "missing" in p)
        other = [p for p in probs if "missing" not in p]
        print(f"{os.path.basename(path)}: {missing} missing translations, {len(other)} other problems")
        for p in other[:20]:
            print("   ", p)
        total += len(other)
    sys.exit(1 if total else 0)


if __name__ == "__main__":
    main()
