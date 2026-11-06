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

All 28 tools are implemented, registered in `tools_media::tools()` and covered by integration tests
(`crates/tools-media/tests/{video,audio}.rs`, 24 tests incl. real EN + FR transcription, plus
24 unit tests and one ignored network test for yt-dlp). `CARGO_TARGET_DIR=target-c cargo test
-p engine-media -p tools-media` passes (≈ 3 min). `castor-cli list` could not be checked: at the
time of my last build the workspace failed in `tools-image` (`zenjpeg` unresolved, agent B's
crate in progress); `cargo run -p tools-media --example media-cli -- list` lists all 28 tools and
runs them exactly like `castor-cli run` (same runner). Every
test probes its outputs (duration, codec, size, rotation, bit depth, track counts). Fixtures are
generated with ffmpeg on first use into `tests/fixtures/generated/media/` (gitignored); two
LibriVox recordings (public domain) live in `tests/fixtures/generated/media/speech/`.

### Engine (`engine-media`)

- `ffmpeg()` / `ffprobe()`: `CASTOR_FFMPEG`/`CASTOR_FFPROBE` (dev override) → bundled resources →
  downloads → PATH; ffprobe is taken next to the ffmpeg in use. Clear localised error if missing.
- `probe()` → `Probe` (duration, coded + display size, fps, VFR flag, rotation normalised to
  0/90/180/270 counter-clockwise, codecs, every stream with language/bits/channels, pixel format,
  bit depth, HDR transfer, audio bit depth for lossless codecs, cover art detection).
- `Ffmpeg` builder / `run_ffmpeg()`: `-progress pipe:1 -nostats`, progress mapped to a sub-range
  of the job (multi-pass tools), ETA message every 5 s, cancel kills ffmpeg, optional `cwd` so
  filter graphs refer to temp files by relative names (no Windows path escaping in
  `subtitles=`/`drawtext`/`vidstab`/`arnndn`/x265 stats), errors classified into user-facing
  messages (unreadable, permission, disk full, encoder) with the ffmpeg tail as detail.
- `thumbnail()` (video frame at 10 %, cover art, or a drawn waveform for audio), `frame_png()`
  (any time + extra filter: used by the GIF preview), `waveform()` (streams PCM from ffmpeg,
  O(n) memory; native rate for short files, 16 kHz above ~30 M samples).
- `analyze`: `silences` (silencedetect), `scenes` (scdet on a 320 px copy), `black_bars`
  (cropdetect on keyframes only), `loudness` (loudnorm pass 1 JSON).
- `hw`: NVENC → QSV → AMF (Windows), VideoToolbox (macOS), probed once with a tiny encode;
  tools retry in software when a hardware encode fails.
- `subs`: SRT/VTT/ASS parsing, SRT/VTT writing, caption layout from word timings (42 chars ×
  2 lines, 6 s max, new cue on pauses and sentence ends, balanced line breaks).
- `whisper`: sidecar `whisper-cli` (see decisions), 27 pinned ggml models (tiny → large-v3-turbo,
  full/q8/q5, `.en` variants) + Silero VAD v6.2.0, all with SHA-256 from the Hugging Face LFS
  index at a pinned revision. 16 kHz mono WAV via ffmpeg, chunks of ~15 min cut at pauses for
  long files (bounded memory, steady progress, language detected on chunk 1 reused), `-ojf`
  token timings merged into words (VAD runs: words spread over their segment, since
  whisper.cpp remaps segment but not token times).
- `ytdlp`: `ensure()` resolves the latest release tag, downloads `SHA2-256SUMS` then the binary
  for the OS (verified before the atomic rename) into `downloads_dir()/bin/`; `update()` runs
  `yt-dlp -U`; `download(url, opts, cx)` with progress, `--ffmpeg-location` our ffmpeg.
  Not exercised by tests (network); wave 2 wires `create.video-download`.
- `time::parse_time`: "90", "1:30", "01:02:03.5", "1:02,5", "1m30s", "2h", "500ms".

### Decisions taken (please confirm)

1. **whisper.cpp as a sidecar `whisper-cli`**, not a Rust binding: whisper-rs is archived, a
   cmake build script would add minutes to every agent's and CI's build, and a sidecar matches
   ffmpeg (cancel = kill, crash isolation, no GPU runtime linked into Castor). Built from the
   pinned tag **v1.9.5** (commit `d1be6fd`).
2. ffmpeg **≥ 7.1 required** (`-/filter_complex file`, `-display_rotation`/`-display_hflip`).
   Dev uses Gyan 8.1.2 (Windows) / Homebrew 9.0.1 (Mac).
3. Default transcription model **small, 8-bit (264 MB)**, **VAD on** (also for captions) and
   greedy decoding ("Thorough" = beam 5). Observed on the English LibriVox sample (2 min):
   `small` always drops the LibriVox disclaimer (≈ 10 s; `base` keeps it — most likely a
   training-data quirk: LibriVox boilerplate is absent from reference transcripts); without VAD,
   greedy `small` also lost the first ≈ 30 s of the story, with VAD nothing else was lost and
   the run was faster. whisper.cpp does not remap token times after VAD, so words are shifted
   onto their segment start (exact within a speech chunk) and segment ends are trimmed to the
   last word.
4. Trims/splits default to **exact** (re-encode, frame-accurate); "fast" copies at keyframes.
5. Compression never returns a larger file: the original is copied to the output path and the
   note says so (same for "already under target size", "no black bars", "no pauses").
6. HDR: tone-mapped (zscale + hable) when the target is 8-bit (H.264, hardware encoders);
   kept as 10-bit PQ/HLG with metadata for software H.265/AV1/VP9.
7. Lossless rotation via the display matrix for MP4/MOV (instant); re-encode otherwise or when
   the user turns "Lossless" off.
8. Denoise "voice" uses ffmpeg `arnndn` with the RNNoise model `sh.rnnn` (somnolent-hogwash,
   fetched once, 298 KB); "general" uses `afftdn` (no download).

### Status per tool
| id | status | notes |
|---|---|---|
| `video.trim` | ✅ | exact (re-encode, same codec family, best quality, audio copied when possible) or fast (stream copy at keyframes); start + end or start + duration; timecode validation |
| `video.convert` | ✅ | MP4/MOV/MKV/WebM/AVI/GIF; remux when codecs fit ("no quality loss" note); H.264/H.265/AV1/VP9/ProRes; all audio tracks + text subtitles (→ mov_text / webvtt) kept |
| `video.extract-audio` | ✅ | "Original" copies the codec into its natural container (AAC→M4A, FLAC, Opus, PCM→WAV…); MP3/M4A/WAV/FLAC/Opus re-encode; first or every track (language in the name) |
| `video.to-gif` | ✅ | palettegen/paletteuse, presets Quality/Balanced/Small + custom fps/width/colours/dither (Sierra, Floyd–Steinberg, Bayer, none), start/duration, loop; `preview()` renders one frame with the palette applied + caption |
| `video.compress` | ✅ | quality presets (x264 CRF 20/23/28, x265 22/26/30, SVT-AV1 28/35/42), target size (two-pass x264/x265/VP9, budget after audio + 4 % overhead, clear error when impossible), same-codec mode, max resolution, audio passthrough when already small, hardware option with fallback, never bigger |
| `video.split` | ✅ | N parts / every X / scene changes (sensitivity, shortest part); exact or fast |
| `video.resize` | ✅ | 2160…360 (short side, so portrait works), percent, exact size with fit / pad (colour) / fill / stretch; no upscaling unless allowed |
| `video.crop` | ✅ | 9:16, 1:1, 4:5, 16:9, 4:3, 21:9 with 9-point anchor, exact size, auto black bars; works on the displayed (rotated) picture |
| `video.rotate` | ✅ | 90/180/270 + flips; lossless display-matrix for MP4/MOV, re-encode otherwise |
| `video.speed` | ✅ | 0.25–4×, atempo chain (pitch kept) or asetrate (pitch shifted); frame rate kept |
| `video.mute` | ✅ | stream copy, subtitles kept |
| `video.join` | ✅ | one job, queue order; concat demuxer + copy when every stream matches, else normalising re-encode (size of first/largest, letterbox, fps, 48 kHz stereo, silent track for files without audio) |
| `video.insert-clips` | ✅ | intro / clip at the middle of the first pause after X s (silencedetect) / outro; everything normalised to the main video |
| `video.watermark` | ✅ | image (scaled to % of width, opacity) or text (drawtext from a text file: any characters, bundled Latin Modern or a system font via fontdb), 9-point anchor, margin |
| `video.captions` | ✅ | from speech (whisper, word timings) or SRT/VTT/ASS; burn-in via generated ASS + libass with styles classic / box / karaoke word highlight / minimal, font, size, position, colours; or soft subtitle track (mov_text/srt/webvtt, language tag); SRT export |
| `video.add-audio` | ✅ | replace or mix (volumes), loop short audio, fades; video copied |
| `video.frames` ★ | ✅ | N evenly spaced, every X s, one per scene, contact sheet (tile + timestamps); PNG/JPG/WebP; capped at 2,000 frames |
| `video.remove-silences` ★ | ✅ | silencedetect → kept segments with padding → select/aselect in one pass (also accepts audio files) |
| `video.stabilize` ★ | ✅ | vid.stab two-pass (strength, zoom or black edges) + light unsharp; `deshake` fallback when vid.stab is absent |
| `audio.transcribe` | ✅ | audio and video; model tiny→turbo, precision, language auto or chosen (100 languages, endonyms), translate to English, VAD, prompt; TXT (paragraphs, optional timestamps), SRT, VTT, JSON (segments + words), DOCX (hand-written OOXML), PDF (krilla, Latin Modern, justified, folios; system font fallback for scripts LM lacks); long files chunked |
| `audio.merge` | ✅ | stitch (optional crossfade) or layer (per-file volumes, limiter); output format auto or chosen |
| `audio.convert` | ✅ | MP3/M4A/WAV/FLAC/OGG/Opus/AIFF/ALAC; 24-bit and 96 kHz kept for lossless targets; cover art + tags kept for MP3/M4A/FLAC; sample rate / channels options |
| `audio.compress` | ✅ | voice (mono, lower rates) or music, 3 levels, same format family or MP3/M4A/Opus, never bigger |
| `audio.trim` | ✅ | as video.trim; exact re-encodes in the same format at ≥ source bitrate (PCM depth kept) |
| `audio.split` | ✅ | pauses (threshold, shortest pause, shortest part) / length / parts |
| `audio.speed` | ✅ | as video.speed |
| `audio.normalize` | ✅ | EBU R128 two-pass loudnorm `linear=true` (single gain when the peak allows; note says when dynamic was needed), targets −16/−14/−23/custom + true peak; peak normalisation; fixed gain; measured loudness in the note; works on video soundtracks (video copied); bit depth and sample rate kept |
| `audio.denoise` ★ | ✅ | RNNoise (`arnndn`, model on demand) or spectral (`afftdn`), strength, 70 Hz high-pass; audio or video |

Known limits (⚠️ but not blocking): video.join/insert-clips re-encode to H.264 + AAC only;
video.normalize/denoise on videos process the first audio track only; transcript PDF line
breaking is space-based (CJK text breaks per character, no RTL reordering); the karaoke style
needs word timings (from speech; an SRT file gets per-cue highlighting only).

### Benchmarks (machine, file, time)

All Windows numbers were taken while three other agents were compiling (CPU at 100 %,
whisper got ≈ 1.5 of 6 cores): they are upper bounds, not representative speeds.

| machine | file | operation | time |
|---|---|---|---|
| Win 11, 6-core (12 threads), RTX 2060, loaded | 20 s 1080p30 (55 MB, CRF 16) | compress H.264 balanced (x264 medium CRF 23) | 23 s → 28 MB (−49 %) |
| same | same | compress H.264 hardware (NVENC p5 cq 26) | 12 s → 26 MB (−52 %) |
| same | same | compress H.265 hardware (NVENC) | 4.4 s → 25 MB (−55 %) |
| same | same | target size 8 MB (two-pass x264) | 24 s → 7.8 MB |
| same | same | AV1 (SVT-AV1 preset 7, CRF 35) + 720p | 27 s → 6.6 MB (−88 %) |
| same | 3 h MP3 (48 kbps, 16 kHz) | `waveform` 2,000 buckets | 33 s, O(n) memory |
| same | same | `audio.normalize` (two-pass loudnorm, linear) | 5 min 56 s, −18.7 → −16.0 LUFS |
| same | same | `audio.split` at pauses, shortest part 10 min (exact) | 1 min 2 s → 17 parts |
| same | 20 min FR LibriVox | transcribe small q8 + VAD, 2 chunks, all 6 formats | 57 min (CPU-starved) |
| same | 14 min EN LibriVox | transcribe base q8 + VAD → PDF | 10 min 12 s (CPU-starved) |
| same | 30 s EN | captions karaoke, base q8 + VAD, burn-in 720p | 39 s |
| Intel Mac i5-8257U (4 cores), macOS 15, whisper-cli CPU + Accelerate, another agent building | 2 min EN | base q8, greedy, no VAD | 32 s (3.8× real time) |
| same | 2 min FR | base q8, greedy, no VAD | 31 s |
| same | 2 min EN | small q8, greedy / beam 5, no VAD | 33–72 s / 56 s |
| same | 2 min EN | **small q8 + VAD through the Castor tool** (default settings) | **28 s (4.3× real time)** |
| same | 2 min FR | base q8 + VAD through the tool | 11 s (11× real time) |

Quality: French and English are transcribed correctly with base/small (accents, punctuation,
"Philippe Aubert de Gaspé"); `small` is noticeably better on French proper nouns
("LibriVox" vs "brévox" with base). Language detection was right on every run.

### Third-party
| component | version | licence | how shipped |
|---|---|---|---|
| FFmpeg + ffprobe (GPL build: x264, x265, SVT-AV1, libvpx, libopus, LAME, libvorbis, libass, freetype, harfbuzz, fribidi, zimg, libvidstab, libwebp) | ≥ 7.1 (dev: 8.1.2 Gyan / 9.0.1 Homebrew) | GPL-3.0-or-later build | sidecar in `bin/` (see packaging) |
| whisper.cpp `whisper-cli` | v1.9.5 (`d1be6fd`) | MIT | sidecar in `bin/`, built in CI |
| ggml Whisper models (tiny…large-v3-turbo, q5/q8/full, .en) | HF `ggerganov/whisper.cpp` @ `5359861c` | MIT | downloaded on demand, SHA-256 pinned |
| Silero VAD (ggml) | v6.2.0, HF `ggml-org/whisper-vad` @ `9ffd54a1` | MIT | downloaded on demand, SHA-256 pinned |
| RNNoise model `sh.rnnn` (somnolent-hogwash) | GregorR/rnnoise-models @ `3eee541a` | no copyright claimed (README: "not subject to copyright"); RNNoise code BSD-3 inside ffmpeg | downloaded on demand, SHA-256 pinned |
| yt-dlp | latest release | Unlicense | downloaded on demand, verified against release `SHA2-256SUMS` |
| Latin Modern Sans/Roman | bundled `assets/fonts` | GUST Font License | `include_bytes!` (watermark, captions, transcript PDF) |
| krilla | 0.8.2 | MIT OR Apache-2.0 | crate (transcript PDF) |
| zip | 2.6 (deflate) | MIT | crate (DOCX) |
| fontdb | 0.23 | MIT | crate (system font lookup) |
| ttf-parser | 0.25 | MIT OR Apache-2.0 | crate (text measuring) |
| LibriVox recordings (tests only) | "Les anciens Canadiens" ch. 1, "The Canvasser's Tale" | public domain | not shipped (gitignored fixtures) |

### Requests to the coordinator

1. **Thumbnails**: wire `engine_media::thumbnail(path, None, max_side)` into
   `castor_tools::thumbnail` for `FileKind::Video` and `FileKind::Audio` (audio: cover art, else
   a drawn waveform). `engine_media::waveform(path, n, cancel)` is ready for Trim Studio.
2. **`FileKind` extensions** (castor-core `files.rs`): add `mka`, `weba`, `wv`, `ape`, `mpc`,
   `ac3`, `dts` to Audio and `mxf`, `vob`, `f4v`, `rm`, `rmvb`, `divx`, `mjpeg`, `y4m` to Video;
   all of them already work through ffmpeg. (`ts` is also used by TypeScript files: consider
   magic-byte confirmation.)
3. **`Requirement::Whisper`** on `video.captions`: the tool also works from an SRT/VTT/ASS file
   without whisper; please let the app treat `Whisper` as "needed only when `downloads()` is
   non-empty" (or drop it from the requirement list and rely on `unavailable()` + run-time error).
   `unavailable()` of captions only checks ffmpeg; transcribe checks ffmpeg + whisper-cli.
4. **Sidecar lookup**: `paths::find_program` already covers `resources/bin`; packaging must put
   `ffmpeg`, `ffprobe` **and `whisper-cli`** side by side there (ffprobe is taken next to ffmpeg).
   Dev overrides honoured by engine-media: `CASTOR_FFMPEG`, `CASTOR_FFPROBE`, `CASTOR_WHISPER`.
5. **CLI**: `castor-cli run` prints every `JobMessage`; media tools post an ETA line ("42 % · about
   1 min 3 s left") at most every 5 s — fine in a terminal, but the app should show the latest
   message in place rather than appending. `ctrlc_hook` is a stub: Ctrl-C kills the CLI, ffmpeg
   receives it too on Windows (same console), so no orphan, but outputs are left half-written
   (the `Partial` guard only runs on a cancelled run, not on a killed process).
6. **Shared label keys**: `common.opt.format` has no keyed choices; tools whose format list has a
   keyed choice ("keep", "auto", "all") use their own `{id}.opt.format` label. If you add
   `common.opt.format.{auto,keep,all}` to app.yml these can switch to `.common()`.
7. **Model management UI**: whisper models are `Asset`s with ids `whisper/ggml-*.bin`,
   VAD `whisper/ggml-silero-v6.2.0.bin`, RNNoise `rnnoise/sh.rnnn`; yt-dlp is `bin/yt-dlp(.exe)`
   (no SHA: verified against the release checksum file at download). `engine_media::whisper::
   MODEL_ASSETS` lists all 27 models for Settings → Models.
8. **Dev builds**: `crates/tools-media/examples/media-cli.rs` is a media-only twin of
   `castor-cli run` (no MuPDF / ONNX build needed) — handy for wave 2 and CI smoke tests.

### Packaging notes (exact ffmpeg / whisper builds to bundle per OS, sizes)

**Minimum**: FFmpeg ≥ 7.1, GPL build, with: libx264, libx265, libsvtav1, libvpx (VP9),
libopus, libmp3lame, libvorbis, libwebp, libass + freetype + harfbuzz + fribidi (subtitles,
drawtext), zimg (`zscale`, HDR tone-mapping), libvidstab (stabilise; `deshake` fallback exists),
plus the built-in filters used (`loudnorm`, `arnndn`, `afftdn`, `silencedetect`, `scdet`,
`cropdetect`, `palettegen/paletteuse`, `showwavespic`, `atempo`, `amix`, `acrossfade`,
`alimiter`) and hardware encoders (NVENC/QSV/AMF on Windows, VideoToolbox on macOS).
Licence: the whole app is GPL-3.0-or-later, so a GPL (not "nonfree") build is the right one;
ship the FFmpeg licence + source offer (link to the exact tag) in the About/licences page.

| OS | FFmpeg build to bundle | size | notes |
|---|---|---|---|
| Windows x64 | **BtbN `ffmpeg-n9.0-latest-win64-gpl-shared-9.0.zip`** (github.com/BtbN/FFmpeg-Builds, pin the dated release asset, record its SHA-256) | 86 MB zip, ≈ 200 MB unpacked (ffmpeg.exe + ffprobe.exe + shared DLLs) | shared build: ffmpeg and ffprobe share the DLLs (the static Gyan "full" build used in dev is 242 MB *per exe*). Contains everything above incl. libvidstab, zimg, libass, nvenc/qsv/amf. Drop `ffplay.exe`, `include/`, `lib/`, `doc/`. |
| macOS arm64 + x86_64 | **martin-riedl.de static builds** (ffmpeg.martin-riedl.de, "release" channel, 9.0.x, both arches, GPL, notarised zips) or the same recipe built in CI | ≈ 80 MB per arch (ffmpeg + ffprobe) | static, no Homebrew dylibs; check the filter list once (`ffmpeg -filters | grep -E "vidstab|zscale|subtitles|arnndn"`). Make a universal binary with `lipo` or ship per-arch DMGs. Homebrew's ffmpeg (used for dev on the Intel Mac, 9.0.1) is **not** bundleable (37 dylib deps). |

**whisper-cli** (whisper.cpp **v1.9.5**, commit `d1be6fd`, MIT): build in CI with CMake
(`-DBUILD_SHARED_LIBS=OFF -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF -DWHISPER_SDL2=OFF
--target whisper-cli`).
- Windows x64: add `-DGGML_NATIVE=OFF -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON` for a
  portable binary that picks AVX/AVX2/AVX-512 at run time (dev build here was `-march=native`
  AVX2, 2.1 MB static). Optional second variant `-DGGML_VULKAN=ON` (needs the Vulkan SDK in CI)
  for GPU speed-ups on any vendor; fall back to the CPU build when Vulkan init fails. The
  official `whisper-bin-x64.zip` (8.9 MB, release b5454) is an acceptable stop-gap.
- macOS arm64: Metal on (default) + Accelerate; embed the Metal library
  (`-DGGML_METAL_EMBED_LIBRARY=ON`). macOS x86_64: `-DGGML_METAL=OFF` (Intel GPUs are slower
  than the CPU path) + Accelerate; built and tested here on the Intel Mac: 3.2 MB, depends only
  on libSystem, libc++ and Accelerate.framework. Sign both with the app's identity.

**Mac: to verify at the final pre-publication build** (no Mac builds in the rest of wave 1).
Done once before that rule (Intel Mac, `~/castor-mac-c`, Homebrew cmake + ffmpeg 9.0.1 already
installed, nothing installed by me): whisper-cli v1.9.5 built (CPU + Accelerate, 3.2 MB) and
`tools-media` + `examples/media-cli` built (12 min 45 s, `-j 2`) and run: compress with `hardware=true`, H.265, lossless rotate, GIF, normalize and transcription
EN/FR all worked (the hardware run took 1.1 s, but whether VideoToolbox or the software
fallback encoded it was not logged). Still to check on the final build:
1. the bundled macOS ffmpeg/ffprobe (martin-riedl or own build) on both arches: `ffmpeg -filters`
   contains `subtitles`, `drawtext`, `zscale`, `vidstabdetect`, `arnndn`; `-encoders` contains
   `libx264 libx265 libsvtav1 libvpx-vp9 libopus libmp3lame h264_videotoolbox hevc_videotoolbox`;
   then `cargo test -p engine-media -p tools-media` with `CASTOR_FFMPEG` pointing at it;
2. VideoToolbox fallback path (`hardware=true` on a 10-bit HDR file → software retry);
3. whisper-cli arm64 with Metal: speed of small q8 on a 2-min sample (expect ≫ 4× real time)
   and that `-ojf` output is identical in structure; Intel x86_64 build without Metal;
4. code signing/notarisation of `ffmpeg`, `ffprobe`, `whisper-cli`, and that the downloaded
   `yt-dlp_macos` runs (quarantine attribute: clear it after download or run via `xattr -d`);
5. fonts: captions/watermark with a system font name (CoreText family names via fontdb);
6. the `-pp` progress lines and Ctrl-C/cancel killing whisper-cli and ffmpeg.

Downloads (first use, consent dialog shows size): whisper models 32 MB (tiny q5) … 1.6 GB
(turbo full), default small q8 = 264 MB; Silero VAD 0.9 MB; RNNoise model 0.3 MB; yt-dlp 18 MB
(Windows) / 36 MB (macOS universal).
