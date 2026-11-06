# Spec C — media engine, video and audio tools

Owner: agent C (wave 1). You own `crates/engine-media`, `crates/tools-media`,
`locales/media.yml`, this file. Build dir: `CARGO_TARGET_DIR=target-c`. Read `CLAUDE.md`,
`docs/ARCHITECTURE.md`, `docs/TOOLS.md` first, and `crates/tools-files/src/checksum.rs`.

## Mission

Ship every tool of `docs/TOOLS.md` § Video (19) and § Audio (9). Quality bar: HandBrake-level
defaults for compression, frame-accurate trims where requested, correct handling of rotation
metadata, variable frame rate, odd dimensions, multiple audio tracks, subtitles, HDR (tone-map
or keep), very long files (3 h podcasts), and clear progress with ETA.

## Engine choices (decide, verify, record)

GPL-3.0-or-later: GPL ffmpeg builds (x264, x265, libvpx, libopus, libmp3lame, libsvtav1,
librubberband, vidstab…) are fine.
- **ffmpeg + ffprobe as sidecars** (`castor_core::paths::find_program`), driven with
  `castor_core::process::run` and `-progress pipe:1 -nostats` for progress (out_time_us vs
  probed duration). Dev on Windows: the winget `ffmpeg` on PATH (Gyan full build). Packaging
  (wave 2) bundles a pinned build per OS: record which builds you recommend (Windows: BtbN or
  Gyan GPL; macOS x86_64 + arm64: e.g. osxexperts/evermeet static or self-built) with URLs and
  licences.
- Hardware encoders (VideoToolbox on macOS, NVENC/QSV/AMF/MediaFoundation on Windows) as an
  option "Fast (hardware)" with automatic fallback to software when it fails.
- **Transcription: whisper.cpp** (MIT). `whisper-rs` is archived — choose between a maintained
  binding, building whisper.cpp yourself via `cmake` in a build script, or a sidecar
  `whisper-cli`. Must run on Windows x64 (CPU, optionally Vulkan) and **Intel Mac** (CPU; Metal
  on Apple Silicon). Models (ggml, MIT) downloaded on demand with pinned SHA-256: tiny, base,
  small, medium, large-v3-turbo (+ quantized q5/q8 variants); English-only fast variants.
  Word-level timestamps for captions; VAD (Silero VAD in whisper.cpp) for silence handling.
  Output TXT, SRT, VTT, JSON (words), DOCX (simple OOXML writer — the `docx-rs` crate or
  hand-written XML), PDF (simple: you may use `printpdf`/`krilla`; nice typography matters:
  Latin Modern from `assets/fonts`).
- **yt-dlp** is for wave 2 (`create.video-download`) but put the engine part here now:
  `engine_media::ytdlp::ensure()` downloads the official release binary for the OS to
  `downloads_dir()/bin/` (Unlicense; no SHA pin possible because it self-updates — verify the
  GitHub release checksum file instead) and `update()` runs `yt-dlp -U`.
- Denoise: ffmpeg `arnndn` with an RNNoise model (BSD) or `nnnoiseless` (pure Rust, BSD).
- Silence detection: ffmpeg `silencedetect` (and/or whisper VAD) for `audio.split`,
  `video.remove-silences`, `video.insert-clips`.

## Public API of engine-media (used by the app and wave 2)

```rust
pub fn ffmpeg() -> anyhow::Result<std::path::PathBuf>;   // located binary or clear error
pub struct Probe { pub duration: f64, pub width: u32, pub height: u32, pub fps: f64, pub rotation: i32,
                   pub video_codec: Option<String>, pub audio_codec: Option<String>, pub audio_tracks: usize,
                   pub has_video: bool, pub has_audio: bool, pub bitrate: u64 /* … */ }
pub fn probe(path: &std::path::Path) -> anyhow::Result<Probe>;
/// Run ffmpeg with progress mapped to cx.progress() using `duration`.
pub fn run_ffmpeg(args: &[std::ffi::OsString], duration: f64, cx: &castor_core::RunCx) -> anyhow::Result<()>;
/// PNG thumbnail at `at` seconds (default 10 % in), longest side ≤ max_side.
pub fn thumbnail(path: &std::path::Path, at: Option<f64>, max_side: u32) -> anyhow::Result<Vec<u8>>;
/// Waveform peaks (for Trim Studio / audiograms in wave 2): `n` buckets of (min, max) in -1..1.
pub fn waveform(path: &std::path::Path, n: usize, cancel: &castor_core::CancelToken) -> anyhow::Result<Vec<(f32, f32)>>;
pub mod whisper { /* models(), assets(model), transcribe(path, opts, cx) -> Transcript { segments, words } */ }
pub mod ytdlp { /* ensure(cx), update(cx), download(url, opts, cx) */ }
```

## Tool notes (beyond docs/TOOLS.md)

- `video.compress`: modes: quality (CRF presets "Best / Balanced / Small"), target size (two-pass
  bitrate), "keep codec" (re-encode same codec), max resolution option, H.264/H.265/AV1, audio
  bitrate; **never output a bigger file** (keep the original, note it).
- `video.convert`: MP4/MOV/MKV/WebM/AVI/GIF…, stream copy when compatible ("remux, no quality
  loss" option shown when possible), codec choice.
- `video.to-gif`: palettegen/paletteuse with dithering choices, fps, width, loop, start/duration;
  presets Quality / Balanced / Small; implement `preview` (one frame with the palette applied).
- `video.trim` / `audio.trim`: start/end or start+duration (timecode inputs as Text with
  validation), "fast (keyframe cut, no re-encode)" vs "exact (re-encode)".
- `video.split`: N parts / every X seconds / at scene changes; `audio.split`: at pauses
  (silence threshold + min length) / by length / N parts.
- `video.join`: concat demuxer when streams match, else re-encode with explicit quality;
  `plan` = one job with all inputs in queue order.
- `video.insert-clips`: options intro (path), clip (path) inserted at the first pause after X s,
  outro (path); normalise formats.
- `video.captions`: burn-in from transcription (whisper) or an SRT/VTT file; styles (classic,
  bold box, karaoke word highlight, minimal), font, size, position, colours; also "soft
  subtitles" (embedded track) option; export SRT too.
- `video.watermark`: image or text, anchor, margin, opacity, scale.
- `video.crop`: aspect presets (9:16, 1:1, 4:5, 16:9) with anchor, or auto black-bar removal
  (cropdetect).
- `audio.transcribe`: accepts audio and video; language auto or chosen
  (`OptionSpec::language` with whisper's list); model choice with sizes; translate-to-English
  option; speaker turns not required. Long files: chunk + progress.
- `audio.normalize`: EBU R128 two-pass loudnorm targets (podcast −16 LUFS, streaming −14,
  broadcast −23, custom), true peak; report measured loudness in `Output::note`.
- `audio.merge`: stitch (concat) or layer (amix) with per-file volume (layer mode).
- `video.speed` / `audio.speed`: atempo chain (pitch kept) or asetrate (pitch shifted), 0.25–4×.
- Implement `downloads()` for whisper models; `unavailable()` if ffmpeg is missing.

## Testing

Generate fixtures with ffmpeg (`testsrc2`, `sine`, `anoisesrc`, rotation metadata, VFR, odd sizes,
5.1 audio, long silent gaps) into `tests/fixtures/generated/media/` (gitignored) + one real
speech sample (public domain: LibriVox / Wikimedia Commons) for transcription (check French and
English). Integration tests per tool family in `crates/tools-media/tests/`. Probe every output
to verify duration/size/codecs. Build and run transcription on the Mac once and record speed.

## Report (fill in before finishing)

### Status per tool
| id | status | notes |
|---|---|---|

### Benchmarks (machine, file, time)

### Third-party
| component | version | licence | how shipped |
|---|---|---|---|

### Requests to the coordinator

### Packaging notes (exact ffmpeg / whisper builds to bundle per OS, sizes)
