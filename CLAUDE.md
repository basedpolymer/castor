# CLAUDE.md — Castor

Rules and map for every AI agent working in this repository (main session and sub-agents).
`AGENTS.md` points here.

## The project

Castor is a free (GPL-3.0-or-later), offline, cross-platform (macOS + Windows) desktop app that
bundles ~100 file tools: PDF, image, video, audio, "create" and file utilities. It is a free
alternative to hangar.tools, with more tools, a CLI and an MCP server. Rust 2024, GPUI Kit 0.9 UI.
Owner: basedpolymer. Repo: github.com/basedpolymer/castor. Site: basedpolymer.eu/castor.

Read first: `docs/ARCHITECTURE.md` (crates, the Tool contract), `docs/DESIGN.md` (visual style,
UX rules, wording), `docs/TOOLS.md` (the catalogue), and your own spec in `docs/specs/`.

## Workspace

```
crates/castor-core    Tool trait, option schema, runner, history, chains, settings, i18n, downloads, processes
crates/engine-pdf     MuPDF (+ Tesseract OCR, Ghostscript sidecar if used)       owner: agent A
crates/engine-ml      ONNX models: background removal, upscale, faces…            owner: agent B
crates/engine-media   ffmpeg/ffprobe sidecars, whisper, yt-dlp                    owner: agent C
crates/tools-pdf      PDF tools                                                   owner: agent A
crates/tools-image    image tools                                                 owner: agent B
crates/tools-media    video + audio tools                                         owner: agent C
crates/tools-create   create tools                                                owner: wave 2
crates/tools-files    files & archive tools (files.checksum = reference example)  owner: wave 2
crates/castor-tools   registry of all tools
crates/castor-cli     `castor-cli` binary (list / info / run / chain / search / history)
crates/castor-app     `castor` GUI binary (GPUI Kit)                              owner: agent D
locales/*.yml         translations, one file per owner (app, pdf, image, media, create, files)
assets/fonts          Latin Modern (GUST licence), TeX Gyre Chorus
```

## Building on Windows

Anything that depends on `engine-pdf` (MuPDF via bindgen) needs its environment first, in Git
Bash: `. crates/engine-pdf/env.sh` (sets `BINDGEN_EXTRA_CLANG_ARGS` for `max_align_t` and
`LIBCLANG_PATH`). Without it the `mupdf` build fails with a `max_align_t` bindgen error.

## Hard rules

1. **Stay in your lane.** Edit only the crates, locale file and spec you own. `castor-core`,
   the root `Cargo.toml`, `CLAUDE.md` and other owners' files belong to the coordinator: if you
   need a change there, write it under "Requests to the coordinator" in your spec file and work
   around it locally meanwhile. Never revert or reformat someone else's file.
2. **Separate build directories.** Parallel agents share the working tree. Always build with your
   own target dir: `CARGO_TARGET_DIR=target-<letter>` (e.g. `target-a`). Never `cargo clean` the
   shared `target/`. Never run `cargo update` on the whole workspace (only `-p` your own deps).
3. **No git commits, no pushes, no branch changes.** The coordinator reviews and commits.
4. **No sub-agents.** Do the work yourself.
5. **Never invent an API.** For GPUI Kit, read `.claude/skills/gpui-kit/` and the crate source in
   `~/.cargo/registry/src/*/gpui-kit-0.9*` / `gpui-component-0.9*`. For any crate, check docs.rs or
   the registry source of the exact version you use.
6. **Licences.** Castor is GPL-3.0-or-later: GPL, LGPL, AGPL (GPLv3 §13), MIT, Apache, BSD, MPL,
   Zlib, Unlicense deps are fine. Not fine: non-commercial or "research only" model weights
   (e.g. RMBG-1.4/2.0, CodeFormer), unclear-licence binaries. Record every bundled or downloaded
   third-party component (name, version, licence, URL) in your spec's "Third-party" table.
7. **Offline by default.** Nothing leaves the machine. Downloads (models, helper programs) happen
   only on first use of the tool that needs them, after the UI shows name + size; use
   `castor_core::download::{Asset, ensure}` with a pinned SHA-256.
8. **Never overwrite user files.** Write through `RunCx::output_path` / `output_named`
   (unique names). In-place tools (`meta.in_place`) must store an undo journal.
9. **Every tool is testable from the CLI**:
   `CARGO_TARGET_DIR=target-x cargo run -p castor-cli -- run <tool-id> <files> --set k=v --out <dir>`.
   Use `CASTOR_HOME=<tmp>` to keep history/settings out of the real profile.
10. **English is the reference language.** Add every user-visible string to your locale file with
    `en` and `fr` (French by you, carefully; other languages come later). Keys: see
    `crates/castor-core/src/tool.rs` and `options.rs` docs. No user-visible string in code.
11. **Performance.** Batches of hundreds of files must work. Stream, don't load everything; poll
    `cx.check_cancel()?`; report `cx.progress()`; release big buffers early.
12. Windows shell is Git Bash or PowerShell; a Mac (Intel, macOS 15, Xcode 26) is reachable over SSH as
    `macmac` (exact command in your brief). Use `~/castor-mac/` there for Mac builds (rsync/scp
    your crate); do not touch other folders on the Mac.

## Definition of done (per tool)

- Implements `Tool`, registered in its crate's `tools()`; `castor-cli info <id>` lists sane options.
- Works on real files from the CLI on Windows; edge cases (empty, huge, corrupt, password,
  unicode file names, read-only output folder) give clear errors, never a panic.
- Unit or integration test for the core transformation (`cargo test -p <crate>`).
- `en` + `fr` strings: name, desc, keywords, every option/choice label, helpful `.help` texts.
- Listed in your spec with status ✅ / ⚠️ (limits) / ❌ (not done, why).
