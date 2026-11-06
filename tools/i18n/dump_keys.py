"""Dump keys."""
import sys; sys.stdout.reconfigure(encoding="utf-8")
import sys, yaml
d = yaml.safe_load(open(sys.argv[1], encoding="utf-8"))
ks = [k for k in d if k != "_version"]
for k in ks[int(sys.argv[2]):int(sys.argv[3])]:
    print(f"{k}\t{d[k]['en']}\t{d[k].get('fr','')}")
