"""Print untranslated keys (en only): todo_keys.py file.yml start count"""
import sys, yaml
sys.stdout.reconfigure(encoding="utf-8")
d = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
todo = [k for k, v in d.items() if k != "_version" and "de" not in v]
print("remaining", len(todo), file=sys.stderr)
for k in todo[int(sys.argv[2]):int(sys.argv[2]) + int(sys.argv[3])]:
    print(k + "\t" + d[k]["en"])
