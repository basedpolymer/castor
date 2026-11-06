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
castor-app ─┐
castor-cli ─┼─> castor-tools ─> tools-pdf ───> engine-pdf ──┐
            │                   tools-image ─> engine-ml ───┤
            │                               └> engine-pdf   ├─> castor-core
            │                   tools-media ─> engine-media ┤
            │                   tools-create ─> (engines)   │
            │                   tools-files ────────────────┘
            └─> castor-core
```

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

## Runtime layout

- macOS: `Castor.app/Contents/MacOS/castor` (+ `castor-cli`), helpers in `Contents/Resources/bin`
  (ffmpeg, ffprobe, gs…), libraries in `Contents/Frameworks`.
- Windows: Velopack install in `%LOCALAPPDATA%\Castor\current\` with `castor.exe`,
  `castor-cli.exe`, `bin\` helpers.
- Downloads (models, tessdata, whisper models, yt-dlp): `paths::downloads_dir()`.

## Updates

Velopack: `VelopackApp::build().run()` first thing in `main`; `UpdateManager` with a
`GithubSource` on github.com/basedpolymer/castor releases; check at launch (setting), show notes,
download with progress, "Restart to update" (or automatic if the user opted in). Packaging
(`vpk pack`, macOS signing + notarisation, Windows Setup.exe) is wave 2.
