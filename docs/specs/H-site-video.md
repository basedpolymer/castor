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

60–75 s, 1920×1080 (+ a 1080×1920 cut for social), 30 fps, English captions burned in and a
French subtitle file, no voice-over, music only if royalty-free with a clear licence (or none —
subtle UI sounds made with Remotion/ffmpeg are fine). Storyboard:
1. (0–6 s) Paper texture, the bitten-diamond mark draws itself (TikZ-like stroke), "Castor".
2. (6–15 s) "Every file tool, right on your desk." — cards of tools cascade in by family.
3. (15–40 s) Real app footage: drop a 19 MB PDF → Compress → "212 KB"; remove a background;
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

## Report (fill in before finishing)
