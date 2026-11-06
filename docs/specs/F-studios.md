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
