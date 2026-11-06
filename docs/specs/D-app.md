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

### Status per scope item (1–12)
| item | status | notes |
|---|---|---|

### Screenshots (paths)

### Third-party
| component | version | licence |
|---|---|---|

### Requests to the coordinator (core changes, engine APIs, icons…)
