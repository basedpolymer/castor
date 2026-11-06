# Spec H — presentation page and promo video

Owner: agent H (wave 2/3). You own `video/` (Remotion project) and, in the website repository
`C:\Users\demor\depots\website` (branch `v5`, deployed by Vercel on push, ~8 min), the folder
`quartz/static/castor/` plus the rewrites for `/castor` in `vercel.json` (copy how `/fouine` is
done: `quartz/static/fouine/`, absolute paths `/castor/…`, no external resources). Do not touch
other pages. Do not push: the coordinator reviews and pushes.

## Page basedpolymer.eu/castor (English, + /castor/fr in French)

The TeXtured look, like https://basedpolymer.eu/nutrition (fonts Latin Modern served from
`/castor/fonts/` as WOFF2, TeX Gyre Chorus ghost initial behind the title, numbered sections,
"Figure n /" captions, page number rule with diamond, booktabs tables, light/dark), responsive,
fast (no framework, no tracker, < 1.5 MB total excluding the video, lazy video).

Sections:
1. Hero: icon (`assets/brand/castor-mark.svg`), "Castor", tagline "Every file tool, right on
   your desk. Free, offline, yours.", download buttons (macOS universal DMG, Windows Setup.exe —
   links to the latest GitHub release assets via `https://github.com/basedpolymer/castor/releases/latest/download/<name>`),
   version, "Open source · GPL-3.0", the promo video (poster image, click to play, MP4 H.264 +
   WebM, captions).
2. "1. Every tool" — the catalogue by family with counts, searchable list (tiny inline JS).
3. "2. Calm, fast, local" — screenshots (light and dark), privacy, speed.
4. "3. Chains, runs, menu bar" — features beyond single tools; CLI and MCP snippet.
5. "4. Castor and Hangar" — honest, friendly comparison table (Table 1 — …): price, licence,
   platforms, tool count, CLI/MCP, offline; credit Hangar as the inspiration, no disparagement.
6. "5. Download" — requirements (macOS 12+, Windows 10/11 x64), SmartScreen note (unsigned),
   Homebrew line, changelog link, source link, licence.
7. Footer with a dry joke ("No beaver was harmed in the making of this app.").

No personal names anywhere (author = basedpolymer). Accessible (WCAG AA), Open Graph image
(1200×630, generated in the house style).

## Video (Remotion, `video/`)

60–75 s, 1920×1080 (+ a 1080×1920 cut for social), 30 fps, **two full language versions —
English and French** (every on-screen text, caption and app footage in the matching language:
record the app with `--lang fr` for the French cut), no voice-over, music only if royalty-free with a clear licence (or none —
subtle UI sounds made with Remotion/ffmpeg are fine). Storyboard:
1. (0–6 s) Paper texture, the bitten-diamond mark draws itself (TikZ-like stroke), "Castor".
2. (6–15 s) "Gnaws through your files." — cards of tools cascade in by family.
3. (15–40 s) Real app footage: drop a scanned PDF → Compress → its real before → after sizes; remove a background;
   trim and caption a clip; ⌘K "make this pdf smaller"; menu bar quick panel with clipboard.
   Use real screen recordings of the app (ffmpeg gdigrab on Windows, `screencapture -v` on the
   Mac) driven by `castor --page …` deterministic states — ask the coordinator for help with
   captures if needed.
4. (40–55 s) Chains and runs; "Free. Offline. Open source." with the numbers (100 tools, 0 €,
   0 bytes uploaded).
5. (55–70 s) Download URL basedpolymer.eu/castor, the mark, a crumb falls off the diamond.
Typography and colours from `docs/DESIGN.md`. Render with `npx remotion render`; output
`video/out/castor-promo.mp4` (+ `-vertical.mp4`, poster PNG). Remotion's licence is free for
individuals — record it.

## Images for X/Twitter (launch thread)

A set of 4–5 images, 1600×900 (16:9, X's in-feed format) + 1080×1350 variants, in the TeXtured
house style (paper, Latin Modern, Chorus ghost initial, numbered titles, "Figure n /" captions,
booktabs tables, diamond motif, family pastels; light by default, one dark variant). Produced
from code (HTML/CSS rendered by headless Chrome, or Remotion stills) so they can be regenerated:
`video/social/*.png` + the source.
1. **Hero**: the mark, "Castor — gnaws through your files. Free, offline, yours.",
   a real app screenshot in a window frame.
2. **The catalogue**: "≈ 110 tools" by family (counts + a few names per family).
3. **Castor and Hangar** (the inspiration): a factual booktabs table — price (Hangar: one-time
   licence from $45 to $239 at launch prices, as published on hangar.tools; Castor: free),
   licence (proprietary / GPL-3.0), platforms, number of tools (Hangar: 77 shipped + 12 "soon"
   on 2026-11-06), CLI, MCP server, offline, menu bar, chains — with the date of the comparison
   and a friendly line crediting Hangar ("Hangar showed the way; Castor is the free cousin").
   Facts only, no disparagement, **no Hangar logo or screenshots** (name only, nominative use).
4. **Beyond the window**: chains, runs + undo, menu-bar quick panel, CLI + MCP snippet.
5. **Download**: basedpolymer.eu/castor, macOS + Windows, GitHub.
Also write the thread text (EN, ≤ 280 chars per post, + FR version) in `video/social/thread.md`.

## Report (fill in before finishing)
