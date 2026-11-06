# Architecture

## Principles

1. **Tools know nothing about the UI.** A tool is a value implementing `castor_core::Tool`:
   static `ToolMeta`, a declarative option schema, `plan` (inputs → jobs) and `run` (one job →
   outputs). The app generates every tool screen from that; the CLI, chains and the MCP server
   use the very same objects. ~100 tools, one screen implementation.
2. **Tool crates do not depend on GPUI.** They build in seconds and are tested from the CLI.
   Only `castor-app` pulls GPUI Kit.
3. **Engines are separate crates** wrapping the heavy machinery (MuPDF, ONNX Runtime, ffmpeg…)
   behind small, safe Rust APIs, with lazy initialisation and on-demand downloads.
4. **Everything is local.** Downloads only for models/helpers, on first use, with consent.

## Crate graph

```
castor-app (binary `castor`: GUI, `castor cli …`, `castor mcp`) ─┐
castor-cli (library + dev binary) ── castor-mcp (library + dev binary) ─┤
                                                                 └─> castor-tools ─> tools-pdf ───> engine-pdf ──┐
                                                                                     tools-image ─> engine-ml ───┤
                                                                                                 └> engine-pdf   ├─> castor-core
                                                                                     tools-media ─> engine-media ┤
                                                                                     tools-create ─> engines     │
                                                                                     tools-files ────────────────┘
castor-launcher (binary `castor-cli`, 0.2 MB, no dependency): runs `castor cli <args>` next to it.
```

**One binary ships.** The engines (MuPDF, ONNX Runtime, Typst, codecs) are linked once, into
`castor`. `castor cli <args>` and `castor mcp` are dispatched in `crates/castor-app/src/main.rs`
before anything graphical (on Windows the GUI-subsystem binary attaches to the parent console when
started from a terminal). The `castor-cli` users type is the tiny launcher, which resolves its own
path (Homebrew symlinks it) and execs `castor cli` with the same stdio and exit code.

## castor-core (frozen contract — coordinator only)

- `tool.rs` — `Tool`, `ToolMeta`, `Family`, `InputSpec`, `Requirement`, `ToolStatus`, `Preview`.
- `options.rs` — `OptionSpec` builders (`choice`, `toggle`, `int`, `float`, `text`, `password`,
  `color`, `file`, `folder`, `pages`, `anchor`, `size`, `font`, `language`) with `.slider()`,
  `.unit()`, `.when(Cond)`, `.advanced()`, `.common()`; `Values` getters; `parse_page_range`,
  `parse_hex_color`.
- `files.rs` — `FileInfo::from_path`, `FileKind`, `expand` (folders → files), `natural_cmp`.
- `job.rs` — `Job`, `Output` (`note`, `undo`), `RunCx` (`progress`, `message`, `check_cancel`,
  `output_path`, `output_named`, `tmp_dir`), `Cancelled`, `CancelToken`.
- `runner.rs` — `start(RunRequest) -> RunHandle` (events over a channel) and `run_blocking`;
  parallel jobs (or `meta.serial`), panics caught, chains (step k outputs → step k+1 inputs,
  intermediates in temp), history record.
- `history.rs` — `RunRecord` in `runs.jsonl`; `chain.rs` — TOML chains + templates;
  `settings.rs` — `Settings` TOML; `paths.rs` — config/data/cache/downloads dirs, `resources_dir`,
  `find_program`; `download.rs` — resumable, SHA-256-checked downloads; `process.rs` — child
  processes with cancel + line callbacks, no console window on Windows, `open`/`reveal`;
  `i18n.rs` — `tr`, `tr_args`, languages; `registry.rs` — `Registry` + baseline search;
  `units.rs` — `bytes`, `duration`, `timecode`, `change`.

## Writing a tool (see `crates/tools-files/src/checksum.rs`)

```rust
pub struct Compress { meta: ToolMeta }
impl Tool for Compress {
    fn meta(&self) -> &ToolMeta { &self.meta }
    fn options(&self) -> Vec<OptionSpec> {
        vec![OptionSpec::choice("preset", vec![Choice::key("balanced"), Choice::key("max")], "balanced")]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let out = cx.output_path(Some(input), "pdf");
        /* … cx.check_cancel()?; cx.progress(i as f32 / n as f32); … */
        Ok(vec![Output::new(out)])
    }
}
```

Strings in `locales/<owner>.yml`: `{id}.name`, `{id}.desc`, `{id}.verb` (button verb),
`{id}.keywords` (space-separated search words), `{id}.opt.{key}`, `{id}.opt.{key}.help`,
`{id}.opt.{key}.{choice}`, `{id}.done` (optional dry one-liner).

## The app (`crates/castor-app`)

- `app.rs`, `shell.rs`, `home.rs`, `tool_screen.rs`, `form.rs` — window, sidebar, tools home,
  the tool screen generated from `Tool::options()`; `palette.rs` (⌘K), `runs.rs`, `chains.rs`,
  `settings_page.rs`, `panel.rs` + `tray.rs` (menu bar / tray quick panel, global shortcut),
  `workspaces.rs` / `workspace_editor.rs`, `integration.rs` (Explorer menu via Velopack hooks,
  watch folders), `ipc.rs` (single instance, file hand-off), `updates.rs` (Velopack;
  `CASTOR_UPDATE_FEED` overrides the feed for tests), `thumbs.rs` (`castor_tools::thumbnail`).
- `studio/` — PDF Studio and Trim & Caption Studio. They save through the runner as app-side
  tools `studio.pdf` / `studio.media` (history, undo), calling engine-pdf / engine-media and the
  registry's own tools.
- `--page … --shot out.png` (feature `shots`) renders a window offscreen to a PNG: screenshots
  without a desktop. `demo-tools` adds fake tools for UI work; never shipped.

## Engines worth knowing

- **engine-pdf**: MuPDF 1.27 (mupdf-rs) + a C shim (`csrc/castor_mupdf.c`) for image rewriting,
  font subsetting, recolouring and Tesseract OCR. Shim contexts are clones of mupdf-rs's locked
  base context (`ffi::ensure_base`): independent unlocked contexts crashed under concurrency
  (`tests/concurrency.rs`). Typst renders Markdown → PDF.
- **engine-ml**: ONNX Runtime (built in; loaded at run time from the bundle on Intel Macs) with a
  pure-Rust `tract` fallback; tiled inference; models downloaded on demand with pinned SHA-256.
- **engine-media**: ffmpeg/ffprobe and `whisper-cli` as bundled sidecars; yt-dlp fetched and
  checksum-verified on first use.

## Runtime layout

- macOS (one app per architecture): `Castor.app/Contents/MacOS/castor` + `castor-cli` (launcher),
  helpers in `Contents/Resources/bin`
  (ffmpeg, ffprobe, gs…), libraries in `Contents/Frameworks`.
- Windows: Velopack install in `%LOCALAPPDATA%\Castor\current\` with `castor.exe`,
  `castor-cli.exe`, `bin\` helpers.
- Downloads (models, tessdata, whisper models, yt-dlp): `paths::downloads_dir()`.

## Updates

Velopack: `VelopackApp::build().run()` first thing in `main`; `UpdateManager` on the GitHub
releases of basedpolymer/castor (channels `win`, `osx-x64`, `osx-arm64`); checked at most daily,
notes shown, download with progress, applied on restart or on quit. Packaging and releases:
`packaging/`, `docs/RELEASING.md`.
