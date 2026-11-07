# Spec F — Studios, workspaces, system integration

Owner: agent D, continued (wave 2). Same ownership as D (`crates/castor-app`, `locales/app.yml`)
plus this file. Read the wave-1 Reports of A (engine-pdf API), C (engine-media API: probe,
thumbnail, waveform, whisper) and your own D report first.

Studios are full-window editors for jobs a form cannot do. They call the same engines/tools;
results go through the runner (history, undo) whenever possible. House style (DESIGN.md) applies;
keep each studio simple: one main canvas, one side panel, a bottom bar with the primary action.

## 1. PDF Studio
Open a PDF (drop, Open… ⌘O, "Open in PDF Studio" from a result). Page thumbnails rail (virtual
list), main page view (zoom, fit width/page, smooth scroll, rendered by engine-pdf at the right
scale on a background thread, cached tiles). Modes:
- **Organize**: drag to reorder, multi-select, rotate, delete, insert pages from another PDF,
  extract selection → new file.
- **Annotate**: highlight/underline text selection, pen, rectangle, arrow, text note, comments
  with a reviewer name (setting), colour presets from the family tints.
- **Sign**: place saved signatures (create: draw with mouse/trackpad, type with Chorus, import
  image), resize, date line.
- **Redact**: draw boxes or search text → marks; "Apply redactions" truly removes content
  (engine-pdf), warning dialog.
- **Fill forms**: fields highlighted, type values, tab order.
Save → new file next to the original ("-edited"), or overwrite with explicit confirmation.

## 2. Trim & Caption Studio (video/audio)
Timeline with filmstrip (engine-media thumbnails) and waveform (`waveform`), playhead,
zoom; preview = frame at playhead (scrub) + play using a lightweight frame pump (ffmpeg decode
to RGBA at preview size + audio via `rodio`/`cpal`, best effort) — if real-time playback is not
achievable on GPUI in this wave, provide scrub + "Play segment in system player" and say so.
- Trim: in/out handles, multiple segments (cut out middle parts), keyframe-fast vs exact.
- Silence removal: detect (silencedetect/VAD), show as shaded regions, remove all/selected.
- Edit by transcript: whisper transcript beside the timeline; deleting words/sentences cuts the
  matching ranges.
- Captions: style presets of `video.captions`, edit text and timing in the transcript list,
  export burned-in video or SRT/VTT.
Export goes through the runner with the same tools (`video.trim`, `video.captions`…).

## 3. Workspaces
A workspace = name, logo, brand colours, signature, default output folder, its own run history
filter. Switcher in the sidebar header. Tools that take a logo/colour/signature (watermark,
stamp, sign, beautify, QR, social card, brand sheet) get "Use workspace brand" defaults through
the values pre-fill (no tool code change needed: pre-fill by option key names `logo`, `color`,
`signature` when present — document the convention for other agents).

## 4. System integration
- macOS: Finder Quick Actions / Services "Open in Castor" and "Castor › <favorite tool>"
  (an `NSServices` entry in Info.plist handled through the single-instance channel); Dock icon
  drop; "Open with".
- Windows: Explorer context menu "Castor ›" with favorites (registry under HKCU written by the
  app on first run and by the installer hooks; removable in Settings).
- Watch folders ★: Settings → Watch folders: folder + chain; new files are processed (debounced,
  stable size), results in a sub-folder; menu bar shows activity; pause/resume. Uses `notify`.

## 5. Polish pass
Startup time, memory with 1 000 files, every screen at 100/150/200 % scaling and on the Mac,
keyboard navigation, accessibility names, empty states with the dry humour of DESIGN §7,
French typography. Fix the coordinator's review notes (sent separately).

## Report (fill in before finishing)

Build, test and screenshots (Windows, Git Bash), as in D-app.md, plus:

```sh
. crates/engine-pdf/env.sh
CARGO_TARGET_DIR=target-d cargo build -p castor-app                    # default (real registry)
CARGO_TARGET_DIR=target-d cargo test  -p castor-app                    # 28 tests + 1 slow ignored
CARGO_TARGET_DIR=target-d cargo test  -p castor-app thumbnail_during_compress -- --ignored   # ≈ 80 s
CARGO_TARGET_DIR=target-d cargo build -p castor-app --features shots   # screenshots, real registry only
castor --page studio:pdf   --files x.pdf --mode organize|annotate|sign|redact|forms [--script edit.json]
castor --page studio:media --files x.mp4 --mode cut|transcript|captions          [--script edit.json]
castor --tool studio.pdf x.pdf      # what "Open in PDF Studio" can use from outside
```
`--script` (screenshots, demos) starts a studio from an edit script (the JSON the studio saves
through the runner; partial scripts are fine). Studios need the engines: with
`--no-default-features --features demo-tools` the Studios page says so and the editors are not
compiled.

### Status
| item | status | notes |
|---|---|---|
| Pre-1. Update feed override | ✅ | `updates.rs`: `CASTOR_UPDATE_FEED=<folder or URL>` → `velopack::sources::AutoSource::new(feed)`, else `AutoSource::new(REPO)` (= `GithubSource`), exactly as G-release.md request 1. |
| Pre-2. New Settings fields | ✅ | `last_update_check` (launch check ≤ daily, also when the clock went back; the wave-1 stamp file is migrated then deleted); `window_bounds` saved quietly while moving/resizing (debounced, no re-render) and restored only if ≥ 120 × 80 px land on a connected display; `workspaces` + `active_workspace` (§3). |
| Pre-3. `castor cli/mcp` + thumbnails | ✅ | Dispatch kept. The file table only asked `castor_tools::thumbnail` for images/PDF/video: **audio was filtered out by the app**, fixed (`thumbs::has_thumbnail`). Checked: PDF page, video frame, MP3/FLAC/WAV waveforms, PNG/JPG (`target-d/shots/thumbs-light.png`). |
| Pre-4. `Requirement::Whisper` on captions | ✅ | The app never gated on `meta().requires` (only `unavailable()` + the `downloads()` consent); test `captions_from_a_file_need_no_whisper`. |
| 1. PDF Studio | ✅ | Below. |
| 2. Trim & Caption Studio | ✅ (⚠️ playback) | Below. No real-time player: scrub + “Play” (selection or 10 s) opens a stream-copied segment in the system player. |
| 3. Workspaces | ✅ | Below. |
| 4. System integration | ✅ Windows / ⚠️ macOS | Explorer “Castor ›” menu + Velopack hooks + watch folders + stale-socket fix done; macOS Services/Quick Actions not done (requests). |
| 5. Polish | ⚠️ | Below. Not measured: 150/200 % scaling, Mac, release startup. |
| Drag-to-reorder favourites | ✅ | Drag in the sidebar; drop puts it before/after the target; the Explorer menu follows. |

#### 1. PDF Studio (`src/studio/pdf_view.rs`, `pdf_engine.rs`, `sign_pad.rs`)
Toolbar (crumbs, mode segmented control, zoom −/+, Width/Page fit), page rail (virtual list,
thumbnails, ◆ = page has marks), pages (virtual list, Ctrl+wheel zooms), side panel per mode,
bottom bar (page n of N, changes, Undo, Overwrite original…, **Save a copy**). A renderer
thread owns the MuPDF documents (open, render at the right device-pixel size in 128 px steps,
newest request first, text with character boxes, search); bitmaps are BGRA `RenderImage`s in an
LRU of 90, released with `drop_image`.
- **Organize**: click / Ctrl / Shift selection, drag in the rail to reorder (several at once),
  rotate ±90°, delete, insert all pages of other PDFs after the current one, extract the
  selection to `name-pages.pdf`.
- **Annotate**: select/move/delete, highlight & underline **by text selection** (nearest
  character, one box per line; box fallback on scans), pen, rectangle, arrow, text box, comment
  (sticky note); 6 highlight tints (family pastels), 8 stroke colours; reviewer name (kept) =
  annotation author. Real PDF annotations (Highlight/Underline/Ink/Square/Line with arrow/
  FreeText/Text), visible in any viewer.
- **Sign**: saved signatures (folder shared with `pdf.sign`); “New signature…”: draw
  (mouse/trackpad → transparent 4× PNG), type (TeX Gyre Chorus), import an image (white paper
  knocked out); click to place, drag to move, corner to resize, optional date line.
- **Redact**: draw boxes or “Find text → Mark all”; warning dialog; on save MuPDF removes text,
  image pixels and covered line art, paints black boxes, **re-extracts the text under every box
  and refuses to save if anything is left**, garbage-4 save.
- **Fill forms**: text/choice inputs, checkboxes, radio groups as segmented controls; fields
  highlighted on the page (click → focus the input or toggle the box); labels from the field
  tooltip, else **the text printed next to the field** (W-9: “1 Name of entity/individual…”,
  “Individual/sole proprietor”…).
- Save = run of the app-side tool `studio.pdf` (edit script as JSON) through the runner:
  progress, Runs history (“Run again” reopens the studio), output `name-edited.pdf` next to the
  original, or **Overwrite** after confirmation: original copied to `{data}/undo/studios/`,
  Runs → Undo puts it back, the studio reloads the file. In-studio undo/redo (Ctrl+Z,
  Ctrl+Shift+Z, Ctrl+Y), Ctrl+O, Ctrl+S, Delete, PgUp/PgDn, Esc. “Open in PDF Studio” / “Open in
  Trim Studio” on finished tool runs; the palette finds both studios by name and by task words.
- e2e tests (real fixtures, through the runner): reorder + rotate + delete + redaction (title
  really gone, the rest kept) + highlight/note/pen/arrow (author set) + typed signature;
  overwrite + undo; forms (text + checkbox read back) and extract; pages inserted from another
  PDF with a mark following its page.

#### 2. Trim & Caption Studio (`src/studio/media_view.rs`, `media_engine.rs`)
Frame at the playhead (debounced ffmpeg frame, newest wins) with a Figure caption; tabs Cut ·
Transcript · Captions; timeline: ruler, filmstrip (18 frames, progressive), waveform (1 600
buckets), amber pauses, cut-out parts dimmed with a red rule, selection, playhead ◆; drag to
select, `+`/`−` zoom, arrows / Shift+arrows, Home/End, I/O, Delete, Space = play. One source of
truth: the **kept ranges** (interval set).
- **Trim**: start/end at the playhead, cut or restore a selection, table of kept parts, undo all
  cuts; **Exact** (re-encode) or **Fast** (stream copy; ranges are snapped to the real keyframes
  first so a fast cut never repeats picture — found by the e2e test: sparse keyframes gave an
  18.7 s output from a 10.5 s file before the fix).
- **Pauses**: silencedetect at 3 room levels, shaded, “Remove all” (0.12 s padding kept).
- **Edit by transcript**: `audio.transcribe` through the runner (consent dialog for missing
  models, JSON, VAD off for exact word times) → clickable words; click seeks, Shift-click
  selects, “Cut words” removes the words and the pause after them; cut words are struck through.
- **Captions**: from the transcript (engine-media caption layout) or SRT/VTT/ASS import; edit
  text, nudge start/end ±0.1 s; the four `video.captions` styles, position; live preview over
  the frame; captions **follow the cuts** (remapped, dropped when cut out).
- Export = app-side tool `studio.media` through the runner, calling the registry's own tools:
  `video.trim`/`audio.trim` per kept part → `video.join`/`audio.merge` → (Burn in)
  `video.captions` with a generated SRT; or SRT/VTT written directly. Outputs `name-edited.*`.
  e2e test: fast and exact cuts (durations checked), SRT (cue removed and shifted), burn-in.
- Real-time playback: not in this wave (no audio output / frame pump on GPUI; `rodio`/`cpal`
  unused) — “Play” opens the segment in the system player, as the brief allows.

#### 3. Workspaces (`src/workspaces.rs`, `workspace_editor.rs`)
Name, logo, two brand colours, signature image, output folder, run ids. Switcher in the
sidebar header (logo or brand diamond; list + “Manage workspaces…”); Settings → 2. Workspaces
(list, Use / Edit / Delete, editor). The active workspace's output folder overrides the output
policy (tools, chains, quick panel, watch folders keep their own). Runs: “<workspace> / All
workspaces” filter. **Pre-fill convention** (documented in `workspaces.rs` for tool authors):
option keys `logo` (file) ← logo, `color` (colour) ← colour 1, `accent` (colour) ← colour 2,
`colors` (text) ← both as “Name #hex” lines, `signature` (file) ← signature. Applied in
`Model::tool_values` (the workspace wins over the last used value while it is active).

#### 4. System integration (`src/integration.rs`, `main.rs`, `ipc.rs`)
- **Windows Explorer**: per-user keys `HKCU\Software\Classes\*\shell\Castor` (cascading
  “Castor” menu: Open in Castor + one entry per favourite → `castor.exe --tool <id> "%1"`) and
  `…\Directory\shell\Castor` (Open in Castor). Written by the **Velopack hooks in `main.rs`**
  (install/update → register, before-uninstall → remove; G-release.md request 2, extended to the
  favourites sub-menu), re-synced at launch and when favourites change; switch in Settings →
  4. System integration. Development builds never touch the registry unless switched on.
  Multi-selection (one process per file) is gathered by the single-instance channel into one
  queue (launches for the same tool within 2 s). Not exercised here (I did not write to the
  user's registry); the packaging update test runs the hooks.
- **Stale single-instance socket** (packaging finding, macOS): on `AddrInUse`, if nobody answers
  on the name, the new instance takes it over (`try_overwrite`, 2 s max). Windows named pipes
  disappear with the process.
- **Watch folders** ★: Settings list (folder → chain, click the chain to change it,
  Pause/Resume, Delete, “Watch a folder…”); `notify`; a file runs once its size is stable for
  2 s; partial/temp names skipped; results in `<folder>/Castor/` (never picked up again); one run
  at a time, recorded in Runs; status in Settings and in the tray tooltip. Live test: a JPEG
  copied into a watched folder came out as `Castor/holiday-resized-min.jpg` (“Web-ready photos”).
- macOS: “Open with” and Dock drops reach the home through `on_open_urls` (wave 1); Services /
  Quick Actions not done (request 4).

#### 5. Polish
- **Crash fixed** (found while making the website shots): a PDF thumbnail rendered while
  `pdf.compress` rewrites a JBIG2 scan killed the app (access violation; reproduced by the
  ignored test `thumbnail_during_compress`). Cause in engine-pdf (request 1); app-side guard
  `pdf_gate.rs`: MuPDF viewing (thumbnails, PDF tool previews, PDF Studio) and PDF-engine jobs
  never overlap; viewers wait asynchronously (no executor thread is blocked).
- Runs: size change only when files map one to one (Checksums showed “−100 %”).
- French typography checked over app.yml (U+202F before ; : ! ? and inside « », ’ apostrophes);
  singular forms (“1 change”, “1 page selected”, “1 cut”).
- Accessibility names on the new custom controls (rail pages, colour and signature swatches,
  timeline, workspace switcher); keyboard shortcuts above.
- Not done: screens at 150/200 % (the disconnected session's DPI cannot be changed), Mac,
  release-build startup and 1 000-file memory measurements.

### Screenshots (paths)
Website / X set — real registry only, clean profile with real runs, 1280 × 800 (panel
360 × 520) — in `target-d/shots/final/`: `shot-home-{light,dark,fr-light,fr-dark}.png`,
`shot-home-video-{light,dark}.png`, `shot-tool-light.png` / `shot-tool-fr-dark.png` (Compress
PDF, 3 real PDFs), `shot-tool-running-dark.png`, `shot-tool-done-light.png` /
`shot-tool-done-fr-light.png` (image.compress on photos + compare view), `shot-palette-light.png`
/ `shot-palette-fr-dark.png`, `shot-chains-light.png`, `shot-runs-{light,dark}.png`,
`shot-settings-light.png` / `shot-settings-fr-dark.png`, `shot-panel-{light,dark,fr-light}.png`,
`shot-studios-light.png`, `shot-studio-pdf-light.png` (annotate), `shot-studio-pdf-dark.png`
(sign), `shot-studio-pdf-redact-fr-light.png`, `shot-studio-pdf-forms-light.png`,
`shot-studio-trim-light.png` (transcript), `shot-studio-trim-dark.png` (captions),
`shot-studio-trim-fr-light.png` (cut). Working shots in `target-d/shots/`: `studio-pdf-*.png`,
`studio-media-*.png`, `settings-workspaces-light.png`, `settings-integration-light.png`,
`thumbs-light.png`.

### Third-party (new direct dependencies of castor-app)
| component | version | licence |
|---|---|---|
| notify (watch folders) | 8.2.1 (already in Cargo.lock) | CC0-1.0 / Artistic-2.0 |
| winreg (Explorer menu, Windows) | 0.56.0 (already in Cargo.lock) | MIT |
| engine-pdf, engine-media | workspace crates (feature `tools`) | — |

### Requests to the coordinator
Status 0.9.1: 1 (gate removed), 2, 3, 4 (Quick Actions) and 5 are done; 6 and 7 stay open.

1. **engine-pdf thread safety (crash)**: `csrc/castor_mupdf.c::new_ctx()` creates contexts with
   `fz_new_context(NULL, NULL, …)` (no locks) while mupdf-rs uses a locked base context on other
   threads; rendering a page during `pdf.compress` image rewriting of `yearbook-1890-scan.pdf` is
   an access violation (`cargo test -p castor-app thumbnail_during_compress -- --ignored` without
   the gate → 0xC0000005). Please give the shim lock callbacks or clone mupdf-rs's base context;
   then `pdf_gate.rs` can go. The CLI and MCP server have no guard.
2. **Settings fields** (stored meanwhile in `tool_values`): `context_menu: Option<bool>`,
   `watch_folders: Vec<toml::Table>` (now `tool_values["app.integration"]`), `pdf_reviewer`
   (now `tool_values["studio.pdf"].reviewer`).
3. **`RunRecord.workspace: Option<String>`**: workspaces keep run ids themselves (≤ 10 000 each).
4. **macOS Services / Quick Actions** (packaging): `NSServices` needs an Objective-C service
   provider on GPUI's NSApplication (not exposed by gpui-kit 0.9); simpler: Quick Action
   `.workflow` bundles in `~/Library/Services` running `open -a Castor --args --tool <id> <files>`
   (`--tool` and the single-instance forwarding already work). Not written (no Mac builds).
5. **Packaging**: the hooks live in `castor.exe`; please add “Explorer menu present after install,
   gone after uninstall” to `update-test.ps1`.
6. **Locales of other owners**: straight apostrophes in French tool names, e.g. `image.yml`
   “Supprimer l'arrière-plan” (house style ’).
7. `studio.pdf` / `studio.media` are app-side tools, not in the registry, so `castor cli` cannot
   replay them; they could move to a tool crate as hidden tools if wanted.
