"""Castor catalogue for the website and the social images, built from the locale files.

    py video/site/catalogue.py                      # writes video/social/src/tools.json
    py video/site/catalogue.py --site <website>     # + rewrites the catalogue of
                                                    #   <website>/quartz/static/castor/{,fr/}index.html

In each page, the block between <!-- catalogue:start --> and <!-- catalogue:end --> is
regenerated, and every <span data-n="total">…</span> / data-n="<family>" gets the new count.
Names and descriptions come from `{id}.name` / `{id}.desc`, search keywords from `{id}.keywords`.
"""
import argparse, html, json, pathlib, re
from collections import OrderedDict

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
FAMILIES = ["pdf", "image", "video", "audio", "create", "files"]
NNBSP = " "  # espace fine insécable

UI = {
    "en": {
        "family": {"pdf": "PDF", "image": "Image", "video": "Video", "audio": "Audio", "create": "Create", "files": "Files"},
        "all": "All", "search": "Search the tools", "filter": "Filter by family",
        "placeholder": "Search {n} tools, e.g. “smaller”",
        "one": "{n} tool", "many": "{n} tools",
        "none": "No tool matches “{q}”. The beaver looked everywhere.",
        "more": "Show all {n}", "less": "Show fewer",
    },
    "fr": {
        "family": {"pdf": "PDF", "image": "Image", "video": "Vidéo", "audio": "Audio", "create": "Créer", "files": "Fichiers"},
        "all": "Tous", "search": "Chercher un outil", "filter": "Filtrer par famille",
        "placeholder": "Chercher parmi {n} outils",
        "one": "{n} outil", "many": "{n} outils",
        "none": "Aucun outil ne correspond à « {q} ». Le castor a pourtant cherché partout.",
        "more": "Voir les {n}", "less": "Voir moins",
    },
}


def fr_typo(s: str) -> str:
    """Curly apostrophes and narrow no-break spaces before ; : ! ? (French typography)."""
    s = s.replace("'", "’")
    s = re.sub(r"\s+([;:!?»])", lambda m: NNBSP + m.group(1), s)
    s = re.sub(r"«\s+", "«" + NNBSP, s)
    return s


def load_tools():
    data = {}
    for f in sorted((ROOT / "locales").glob("*.yml")):
        data.update(yaml.safe_load(f.read_text(encoding="utf-8")))
    tools = []
    for k in data:
        m = re.fullmatch(r"([a-z]+)\.([a-z0-9-]+)\.name", k)
        if not (m and m.group(1) in FAMILIES and f"{m.group(1)}.{m.group(2)}.desc" in data):
            continue
        i = f"{m.group(1)}.{m.group(2)}"
        get = lambda key, lang: (data.get(f"{i}.{key}") or {}).get(lang) or (data.get(f"{i}.{key}") or {}).get("en", "")
        tools.append({
            "id": i, "family": m.group(1),
            "name": {"en": get("name", "en"), "fr": fr_typo(get("name", "fr"))},
            "desc": {"en": get("desc", "en"), "fr": fr_typo(get("desc", "fr"))},
            "keywords": {"en": get("keywords", "en"), "fr": get("keywords", "fr")},
        })
    by = OrderedDict((f, [t for t in tools if t["family"] == f]) for f in FAMILIES)
    # Locale order follows the registries, except a few tools whose strings sit next to a
    # sibling of another family (media.yml lists audio.speed right after video.speed).
    for f, after, tid in [("audio", "audio.split", "audio.speed")]:
        lst = by[f]
        t = next((x for x in lst if x["id"] == tid), None)
        a = next((i for i, x in enumerate(lst) if x["id"] == after), None)
        if t is not None and a is not None:
            lst.remove(t)
            lst.insert(next(i for i, x in enumerate(lst) if x["id"] == after) + 1, t)
    # files.checksum (the reference example) is first in files.yml but last in the registry.
    lst = by["files"]
    t = next((x for x in lst if x["id"] == "files.checksum"), None)
    if t is not None:
        lst.remove(t)
        lst.append(t)
    return tools, by


def block(by, lang: str, section: int = 1) -> str:
    u = UI[lang]
    total = sum(len(v) for v in by.values())
    e = html.escape
    out = [
        f'<div class="large" id="catalogue" data-un="{e(u["one"])}" data-plusieurs="{e(u["many"])}" '
        f'data-rien="{e(u["none"])}" data-plus="{e(u["more"])}" data-moins="{e(u["less"])}">',
        '  <div class="barre">',
        f'    <label class="recherche"><span class="sr">{e(u["search"])}</span><input type="search" id="q" '
        f'placeholder="{e(u["placeholder"].format(n=total))}" autocomplete="off" spellcheck="false"></label>',
        f'    <p class="compte" id="compte" role="status">{e(u["many"].format(n=total))}</p>',
        f'    <div class="chips" role="group" aria-label="{e(u["filter"])}">'
        f'<button type="button" class="chip" data-f="all" aria-pressed="true">{e(u["all"])}<span>{total}</span></button>'
        + "".join(f'<button type="button" class="chip" data-f="{f}" aria-pressed="false">{e(u["family"][f])}<span>{len(v)}</span></button>'
                  for f, v in by.items()) + "</div>",
        "  </div>",
        '  <p class="rien cache" id="rien"></p>',
    ]
    for n, (f, v) in enumerate(by.items(), 1):
        fam = u["family"][f]
        out.append(f'  <div class="famille" id="{f}" data-f="{f}" data-nom="{e(fam)}">')
        out.append(f'    <h3><span class="num">{section}.{n}</span>{e(fam)}<span class="n">{e(u["many"].format(n=len(v)))}</span></h3>')
        out.append(f'    <ul class="outils f-{f}" id="ul-{f}">')
        for t in v:
            kw = t["keywords"]["en"] + (" " + t["keywords"]["fr"] if lang == "fr" else "")
            out.append(f'      <li data-k="{e(kw)}"><b>{e(t["name"][lang])}</b><span>{e(t["desc"][lang])}</span></li>')
        out.append("    </ul>")
        out.append(f'    <button type="button" class="plus cache" aria-controls="ul-{f}" aria-expanded="false"></button>')
        out.append("  </div>")
    out.append("</div>")
    return "\n".join(out)


def patch(page: pathlib.Path, by, lang: str):
    s = page.read_text(encoding="utf-8")
    s, k = re.subn(r"(<!-- catalogue:start -->\r?\n).*?(<!-- catalogue:end -->)",
                   lambda m: m.group(1) + block(by, lang) + "\n" + m.group(2), s, flags=re.S)
    if not k:
        raise SystemExit(f"{page}: no catalogue markers")
    total = sum(len(v) for v in by.values())
    s = re.sub(r'(<span data-n="total">)\d+(</span>)', lambda m: f"{m.group(1)}{total}{m.group(2)}", s)
    for f, v in by.items():
        s = re.sub(rf'(<span data-n="{f}">)\d+(</span>)', lambda m: f"{m.group(1)}{len(v)}{m.group(2)}", s)
    page.write_text(s, encoding="utf-8", newline="\n")
    print("patched", page)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--site", type=pathlib.Path, help="website repository (quartz/static/castor is patched)")
    a = ap.parse_args()
    tools, by = load_tools()
    out = ROOT / "video" / "social" / "src" / "tools.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(tools, ensure_ascii=False, indent=1), encoding="utf-8")
    print(len(tools), "tools:", ", ".join(f"{f} {len(v)}" for f, v in by.items()))
    if a.site:
        d = a.site / "quartz" / "static" / "castor"
        patch(d / "index.html", by, "en")
        patch(d / "fr" / "index.html", by, "fr")
