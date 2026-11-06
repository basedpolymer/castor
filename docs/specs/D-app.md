# Spec D — the desktop app (GPUI Kit)

Owner: agent D (wave 1). You own `crates/castor-app`, `locales/app.yml` (keep the existing keys,
add yours), `assets/icons/` (placeholders), this file. Build dir: `CARGO_TARGET_DIR=target-d`
(the first GPUI build takes ~8 min). Read `CLAUDE.md`, `docs/ARCHITECTURE.md`, **`docs/DESIGN.md`
(binding)**, `docs/TOOLS.md`, then the GPUI Kit skills in `.claude/skills/gpui-kit/SKILL.md` and
`.claude/skills/gpui-kit-design-guides/SKILL.md` and the references they point to. The skills
come from gpui-kit `main`; the crate we use is **0.9.0** — check every API against the registry
source (`~/.cargo/registry/src/*/gpui-kit-0.9.0`, `gpui-component-0.9.0`, `gpui-*`).

The current `crates/castor-app/src/main.rs` is a spike (window + tray icon) proven to build and
run on Windows; replace it.

## Mission

A fast, calm, intuitive app that makes ~100 tools feel simple, in the TeXtured style of
`docs/DESIGN.md`. Tools are provided by `castor_tools::registry()`; you never special-case a tool
by id (except in tests). Agents A, B, C fill the registry in parallel with you: today it holds
only `files.checksum`; add **demo tools behind a `demo-tools` cargo feature** inside castor-app
(one of each option kind, a slow one with progress, one with preview, one that fails, one with
no input) to build and test every UI path. Never ship them.

## Scope

1. **Bootstrap**: `VelopackApp::build().run()` first (velopack crate; see Updates), logging to
   `paths::data_dir()/logs`, settings load, language (`castor_core::i18n::set_language`), fonts
   from `assets/fonts` (embedded with `include_bytes!` and registered with the text system),
   custom theme light/dark from DESIGN §1 following Settings::appearance (System follows OS
   changes live).
2. **Main window** per DESIGN §4: title bar with search, sidebar (Tools, Studios — "Coming soon"
   page for now, Chains, Runs, Favorites, Settings, version + update dot), pages below.
3. **Tools home**: drop zone + "Choose files…", family tabs with counts, numbered family
   sections, cards; after a drop/choose, the matching tools highlight (DESIGN §4), with a
   "Suggested" row on top (best tools for these files); clicking a card opens the tool screen
   with the files already queued. Keyboard: `/` focuses search, arrows move between cards,
   Enter opens.
4. **Tool screen**: generated options form (DESIGN §6) from `Tool::options()`; values persisted
   per tool; `visible_if`, `advanced`; file queue table (thumbnails via
   `castor_tools::thumbnail`, async + cached); live `preview` (debounced ~150 ms, on a background
   thread, cancel stale renders); consent dialog for `Tool::downloads()` not yet present (names,
   licences, total size); `unavailable()` banner; run with `castor_core::runner::start` and
   stream `RunEvent`s into the UI (per-row progress, messages, results, errors with a details
   disclosure); cancel; summary + "Show in folder" + compare view for image results (side by
   side / swipe, Fit / 100 %); output location line + change (folder picker), from
   `Settings::output_policy`.
5. **Command palette** (⌘K / Ctrl+K, and the title-bar search): tools (via `Registry::search`,
   then recency/favorites boost), chains, pages, actions (Settings, Check for updates, Open
   output folder, Toggle theme). Fuzzy-ish matching; ↑↓ Enter Esc. Typing a plain sentence like
   "make this pdf smaller" must surface `pdf.compress` (keywords do it; agents A/B/C add
   localized `{id}.keywords`).
6. **Runs page** (DESIGN §4) from `castor_core::history::load`, filters, details, re-run with
   same values, undo (`Tool::undo` with `RunRecord::undo`, then `history::update`), clear history.
7. **Chains page**: templates (`chain::templates`, skip steps whose tool id is missing) + user
   chains (`chain::load_all/save`); editor with steps (add from a tool picker, remove, move
   up/down, per-step generated form), run on chosen files/folder, export/import TOML.
8. **Settings page** (DESIGN §4): language (9 languages; switch live), appearance, output
   location, reveal when done, menu bar/tray toggles, global shortcut (record a new one),
   launch at login (`auto-launch` crate), models & helpers (list assets known so far from
   tools' `downloads()` with sizes, licence, present/absent, delete), updates (auto check,
   auto-install, check now, current version), privacy (statement: nothing leaves the machine;
   AI key field for opt-in features, stored only if set), about (version, GPL-3.0, link to
   THIRD_PARTY notices file once it exists, website, GitHub, dry joke).
9. **Tray / menu bar + quick panel** (DESIGN §5) with `tray-icon` (template monochrome icon on
   macOS; placeholder diamond now — final icon comes in wave 2), left click → panel anchored to
   the icon rect from the tray event; right click → menu (Open Castor, recent tools, Check for
   updates, Quit). Global shortcut (`global-hotkey`) toggles the panel. Clipboard actions via
   GPUI clipboard (image + file paths): result written back to the clipboard. Closing the main
   window keeps the app in the tray when `keep_running_in_tray`; on macOS optionally hide the
   Dock icon while only the panel is used (activation policy accessory/regular via `objc2`).
10. **Single instance + open files**: launching `castor file1 file2` (or "Open with" / drag on
    the app icon / dock) while running forwards the paths to the running instance (local
    socket, e.g. `interprocess`) which opens the tools home with those files; macOS
    `application:openURLs:`/open-files events through GPUI's `on_open_urls` if available.
    `castor --tool pdf.compress file.pdf` opens that tool directly (used later by the Finder /
    Explorer context menu).
11. **Updates (in-app)**: `velopack` crate, `UpdateManager` + `GithubSource`
    ("https://github.com/basedpolymer/castor"); when not installed via Velopack (dev builds) show
    "Updates are disabled in development builds" instead of failing; check at launch when
    enabled (non-blocking, at most daily), notification "Castor x.y is ready — What's new /
    Restart to update", download progress, `apply_updates_and_restart`. Auto-install if opted in
    (apply on quit).
12. **Deterministic screenshots** (for review, docs and the promo video): `castor --page
    <tools|tool:ID|runs|chains|settings|panel> [--files …] [--theme light|dark] [--lang fr]
    [--size 1280x800]` opens that state directly. Provide a PowerShell script
    `crates/castor-app/tools/shot.ps1` capturing the Castor window (by PID) to PNG, and the
    macOS equivalent `shot.sh` (`screencapture -l <window id>`).

## Quality bar

- Startup to interactive < 400 ms (release) — lazy-load heavy things; no blocking I/O on the UI
  thread (thumbnails, previews, history, settings save all async).
- Keyboard-first: every action reachable; visible focus ring (link colour).
- Accessible: AccessKit names on custom controls; contrast AA for text.
- Smooth with 1 000 queued files (virtual list) and 10 000 history rows.
- Works on Windows 11 (100 %, 150 %, 200 % scaling) and macOS (Retina and non-Retina); check
  the Mac build on `ssh macmac` (`~/castor-mac/`, `cargo build -p castor-app`), take
  screenshots there too (ask the coordinator if the GUI session needs the user).

## Testing

GPUI test windows (`#[gpui_kit::test]`) for the form generator, palette search, run event
handling (with demo tools). Manual: screenshot every page in light and dark, EN and FR, and
look at them (Read the PNG) against DESIGN.md before reporting.

## Report (fill in before finishing)

Build & run (Windows, Git Bash):

```sh
. crates/engine-pdf/env.sh                       # MuPDF bindgen environment (see CLAUDE.md)
CARGO_TARGET_DIR=target-d cargo build -p castor-app                       # real registry
CARGO_TARGET_DIR=target-d cargo build -p castor-app --features demo-tools,shots
CARGO_TARGET_DIR=target-d cargo build -p castor-app --no-default-features --features demo-tools,shots
                                                   # UI work: demo tools only, builds in minutes
CARGO_TARGET_DIR=target-d cargo test  -p castor-app --features demo-tools,shots
target-d/debug/castor.exe --page tool:demo.slow --files a.png b.png --theme dark --run     --size 1280x800 --shot target-d/shots/x.png --shot-delay 5   # offscreen screenshot, then quit
```

Cargo features of `castor-app`: `tools` (default: the real `castor_tools::registry()`),
`demo-tools` (9 fake tools that exercise every UI path, added *on top of* the real registry;
never ship), `shots` (`--shot out.png`: renders the window offscreen through GPUI's
`render_to_image` and quits; pulls GPUI test-support, dev only).

### Status per scope item (1–12)
| item | status | notes |
|---|---|---|
| 1. Bootstrap | ✅ | `VelopackApp::build().run()` first; logs to `{data}/logs/castor.log` (rotated at 5 MB) + stderr, panics logged; settings load; `i18n::set_language` (+ GPUI Kit's own locale); the 10 bundled OTFs registered with `add_fonts` (DirectWrite names them "Latin Modern Sans 10"…, CoreText "LM Sans 10": resolved at runtime from `all_font_names`); custom theme = a GPUI Kit `ThemeSet` built from the DESIGN §1 tokens for both modes (`theme.rs`, extra tokens + family tints in `theme::Tokens`); Appearance System follows the OS live (`observe_window_appearance`). Windows icon embedded as resource 1 (`build.rs` + `resources/castor.rc`, brand `castor.ico`). |
| 2. Main window | ✅ | Custom `TitleBar` (mark + "Castor", search field → palette with Ctrl/⌘K hint), sidebar 216 px (Tools, Studios, Chains, Runs, small-caps FAVORITES with family swatches, Settings, version + update dot), 1180 × 760 default, 880 × 560 min. Studios = "Coming soon" page. Not done: drag-to-reorder favourites (they keep the order they were starred in). |
| 3. Tools home | ✅ | Page header (Chorus ghost initial, page rule + tool count), drop zone + "Choose files…" (folders expanded off-thread), family tabs with counts, numbered sections, cards 2–5 per row by width; after a drop: "Suggested" row (ranking in `Home::suggested`: kind match, family, favourites, recency, generic tools last), ink outline + "3 files" badge, others at 40 % "Not for these files"; card click opens the tool with the accepted files queued. Keyboard: `/` opens the palette, arrows move (focus ring in link colour), Enter opens, Esc clears. Star on hover toggles favourite. |
| 4. Tool screen | ✅ | Breadcrumb, title + ghost initial, desc; generated form for every `OptionKind` (segmented ≤ 4 / select > 4, checkbox, slider + number + unit, number + unit, input/textarea, masked password with reveal (never persisted), ColorSelect, path + Choose…, page range placeholder, 3 × 3 diamond anchor, W × H + lock ratio, font select rendered in each font, searchable language select with chips for multi); `visible_if`, "More options (n)"; values persisted per tool (secrets stripped, debounced background save). File queue = virtual `uniform_list` booktabs table with async cached thumbnails (`castor_tools::thumbnail`, 4 in flight), per-row open / show / remove. Live preview debounced 150 ms on a background thread, stale renders dropped by generation; no-input tools show the preview as Figure 1. Consent dialog for missing `downloads()` (name, licence, size, total). `unavailable()` banner + disabled run. Run via `runner::start`, `RunEvent`s polled every 33 ms → per-row progress bars + messages, ✓ size (change) / ✗ error with Details disclosure, overall %, Cancel; summary "n files · a → b (−x %) · t s" + `{id}.done` / dry quip + Show in folder; compare view (side by side / swipe, Fit / 100 %) for image results; output line + Change (folder picker) / Reset from `Settings::output_policy`. Ctrl/⌘↵ runs. Narrow windows get a compact table. |
| 5. Palette | ✅ | Ctrl/⌘K and the title-bar search; tools (Registry::search + sentence handling: stop-words, intent synonyms like smaller → compress, word-by-word fallback, subsequence fuzzy) boosted by favourites/recency, chains, pages, actions (Check for updates, Open output folder, Toggle light/dark, Quit); ↑↓ via a keystroke interceptor (the input would swallow them), Enter, Esc, click outside. "make this pdf smaller" → Compress PDF first (tested + screenshot). |
| 6. Runs | ✅ | "Table 1 — Runs" grouped by day (Today / Yesterday / date), virtual list for 10 000 rows, filters All / Complete / Failed (+Partial) / Cancelled, details panel (summary, steps, errors, outputs with open/show), Run again (same values + existing inputs, or reopens the chain), Undo for in-place runs (`Tool::undo` in reverse, then `history::update`), Clear history with confirmation. Empty state "No runs yet. The beaver is resting." |
| 7. Chains | ✅ | Templates (missing tools listed and skipped) + user chains; editor: name, "Step 1 ◇ Resize → Step 2 ◇ Compress" flow (current step filled diamond while running), per-step generated form, add step (searchable tool picker), remove, move up/down; Save / Save a copy (templates), Delete (confirm), Export / Import TOML; run on files or a folder with step k/n progress, cancel, summary. |
| 8. Settings | ✅ | Document with 6 numbered sections: language (system + 9, switches live), appearance, output location (+ folder), reveal when done, tray icon, keep running in tray, global shortcut recorder, launch at login (`auto-launch`, `--background`), models & helpers table from every tool's `downloads()` (licence, size, present/absent, delete), updates (auto check, auto install, status, Check now / Download / Restart), privacy statement + AI key (masked, stored only if set, forget), about (version, GPL-3.0, joke, site, GitHub, third-party notices if `THIRD_PARTY.md` is next to the exe, logs). |
| 9. Tray + quick panel | ⚠️ | Tray icon from `assets/brand` (template image on macOS; white/black on Windows from the taskbar theme), menu (Open Castor, 5 recent tools, Check for updates, Quit), left click / global shortcut (default Ctrl+Alt+Space, re-registered from Settings) toggle the 360 × 520 borderless panel (search, drop → 6 best tools run at once with last values, progress, Show / Copy, clipboard line for image or files with 3 actions writing the result back to the clipboard, favourites, last 3 runs, Open Castor / Quit; closes on blur and Esc). Closing the window keeps the app in the tray when enabled (macOS: Dock icon hidden via `ActivationPolicy::Accessory`). Verified: tray created without error, panel renders (screenshots). **Not verified by hand** (no interactive desktop for clicks): the tray click anchoring, hotkey press, clipboard round-trip. Windows panel placement is bottom-right of the work area (tray rect is in physical px with no reliable scale). |
| 10. Single instance + open files | ✅ | `interprocess` local socket per user/profile; a second `castor file…` / `castor --tool id file…` forwards its args (JSON line, ack) and exits (measured 258 ms); the first instance opens the home with those files or the tool. macOS `on_open_urls` (file://) and `on_reopen` wired, not run (no Mac build in this wave). |
| 11. Updates | ⚠️ | Velopack `UpdateManager` + `GithubSource("https://github.com/basedpolymer/castor")`; dev builds → "Updates are disabled in development builds"; check at launch if enabled (≤ daily, stamp file, 4 s after start, background), notification, download with progress (thread + channel), Restart to update, auto-install applies on quit. Untested against a real release feed (none exists yet). |
| 12. Deterministic screenshots | ✅ | `castor --page tools\|tool:ID\|runs\|chains\|chain:NAME\|settings\|studios\|panel\|palette [--query …] [--family video] [--scroll 600] [--files …] [--theme light\|dark] [--lang fr] [--size WxH] [--run]`; feature `shots` adds `--shot out.png [--shot-delay s]` which renders the window offscreen with GPUI's `render_to_image` and quits (works without a visible desktop — every screenshot below was made that way); `--family <key>` and `--scroll <px>` reach long sections of the home. `tools/shot.ps1` (PID → window → PNG, DPI-aware, skips the console; needs an unlocked desktop) and `tools/shot.sh` (macOS `screencapture -l`). |
| Tests | ✅ | `cargo test -p castor-app --features demo-tools,shots`: 13 passed — GPUI UI tests (generated form: segmented click, checkbox, `visible_if`; palette sentence search), run-event reducer, suggestions, search ranking, CLI args, hotkey parsing, theme JSON. |
| Mac | ❌ | Not built in this wave (user rule: Mac compilation only before publication). An early attempt (before the rule) stopped in `engine-pdf`; `~/castor-mac-d` holds a copy of the sources. |

### Screenshots (paths)
All under `target-d/shots/`, rendered offscreen with `--shot` from the real registry + demo tools
(1280 × 800 unless noted; when the PC's session is disconnected Windows clamps windows to the
640 × 480 virtual display, so `--shot` then yields 880 × 560 — use `--family video` /
`--scroll 600` to reach long sections):
- Home: `home-light.png`, `home-dark.png`, `home-fr-light.png`, `home-fr-dark.png`, after a drop
  `home-drop-light.png`, Video section `home-video-light.png`, `home-video-scroll-dark.png`,
  1024 px wide `home-video-1024-dark.png` (1024 × 700)
- Tool screen: `tool-real-light.png` (Compress PDF), `tool-light.png` (every option kind),
  `tool-running-dark.png` (live run), `tool-done-fr-light.png` (done + summary + compare view),
  `tool-error-light.png`, `consent-light.png`, `preview-light.png` (no-input tool, live preview)
- Palette: `palette-light.png` ("make this pdf smaller"), `palette-fr-dark.png` ("rendre ce pdf plus léger")
- Quick panel: `panel-light.png`, `panel-dark.png` (360 × 520)
- Settings: `settings-light.png`, `settings-fr-dark.png`; Runs: `runs-light.png`, `runs-dark.png`;
  Chains: `chains-light.png`

### Third-party
| component | version | licence |
|---|---|---|
| gpui-kit / gpui-component / gpui-base / gpui-pre-* (GPUI) | 0.9.0 / 0.3.11 | Apache-2.0 |
| gpui-kit-assets (Lucide icons) | 0.9.0 | Apache-2.0 (icons: ISC, LICENSE-LUCIDE) |
| tray-icon (+ muda menus) | 0.21.3 (muda 0.17.2) | MIT OR Apache-2.0 |
| global-hotkey | 0.7.0 | Apache-2.0 OR MIT |
| velopack | 1.3.39-g8d2844f | MIT |
| interprocess | 2.4.7 | 0BSD OR Apache-2.0 |
| auto-launch | 0.6.0 | MIT |
| embed-resource (build only, Windows icon) | 3.0.12 | MIT |
| image | 0.25 | MIT OR Apache-2.0 |
| Latin Modern fonts (Sans, Roman, Roman Caps, Slanted, Mono) | 2.004 (assets/fonts) | GUST Font License |
| TeX Gyre Chorus | 2.003 (assets/fonts) | GUST Font License |
| Castor brand icons | assets/brand | project (basedpolymer) |

### Requests to the coordinator (core changes, engine APIs, icons…)
1. **Workspace builds break `castor-app` whenever any tool crate is mid-edit** (seen: mupdf bindgen env, `zenjpeg` features, `tools-files` using `engine_pdf`/`engine_media` without deps). Each break or `Cargo.lock` change also forces GPUI to recompile (~15 min on the shared PC). Suggest: agents only touch `Cargo.lock` through the coordinator, and a CI-style check `cargo check -p castor-tools` before handing back. `castor-app --no-default-features --features demo-tools` builds without any tool crate for UI work.
2. **`castor_core::settings::Settings`**: add `last_update_check: Option<i64>` (today a `last-update-check` file in `data_dir`) and `window_bounds` (restore size/position).
3. **Thumbnails** for PDF pages and video frames in `castor_tools::thumbnail` (the app already requests them for `Pdf` / `Video` kinds; they come back `None` today).
4. **Locale**: `units::bytes` is English-only ("KB"); French expects "ko". A `units::bytes_localized` or locale keys for units would help.
5. **`OptionKind::Path`** has `filters` but GPUI's `PathPromptOptions` has no file-type filter: either accept it, or allow `rfd` for filtered dialogs.
6. **Icons**: Windows tray icon colour is chosen from `SystemUsesLightTheme` once at launch; a live switch needs a theme-change hook (later). Window icon uses `castor.ico` as resource 1; macOS bundle icon is wave 2 packaging.
7. **Release builds**: `windows_subsystem = "windows"` only in release; dev builds keep a console. Startup time was not measured in release (debug build: window up ≈ 1 s after process start, dominated by D3D device creation).
