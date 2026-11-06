//! Castor media engine.
//!
//! - [`ffmpeg()`] / [`ffprobe()`]: locate the sidecar binaries (bundled → downloads → PATH).
//! - [`probe`]: what is inside a media file ([`Probe`]).
//! - [`run_ffmpeg`] / [`Ffmpeg`]: run ffmpeg with progress (from `-progress pipe:1`), ETA and
//!   cancellation.
//! - [`thumbnail`], [`frame_png`], [`waveform`]: pictures and peaks for the UI.
//! - [`analyze`]: silence, scene and black-bar detection, loudness measurement.
//! - [`hw`]: hardware encoders with automatic fallback.
//! - [`subs`]: subtitle cues, SRT / VTT / ASS reading and writing.
//! - [`whisper`]: whisper.cpp transcription (sidecar `whisper-cli`, ggml models on demand).
//! - [`ytdlp`]: the official yt-dlp binary, fetched and updated on demand.
//! - [`time`]: timecode parsing for trim-like options.

pub mod analyze;
mod ff;
pub mod hw;
mod pictures;
mod probe;
pub mod subs;
pub mod time;
pub mod whisper;
pub mod ytdlp;

pub use ff::{Ffmpeg, MediaError, ffmpeg, ffprobe, filter_escape, has_filter, run_ffmpeg};
pub use pictures::{frame_png, thumbnail, waveform};
pub use probe::{Probe, Stream, StreamKind, probe};
