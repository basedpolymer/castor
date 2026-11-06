//! Video and audio tools (ffmpeg / ffprobe sidecars, whisper.cpp). Owner: agent C.
//! Spec: docs/specs/C-media.md.

use std::sync::Arc;

use castor_core::Tool;

mod audio;
mod common;
mod fonts;
mod transcript_docs;
mod video;

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        // Video
        Arc::new(video::trim::Trim::video()),
        Arc::new(video::convert::Convert::new()),
        Arc::new(video::extract_audio::ExtractAudio::new()),
        Arc::new(video::gif::ToGif::new()),
        Arc::new(video::compress::Compress::new()),
        Arc::new(video::split::Split::video()),
        Arc::new(video::resize::Resize::new()),
        Arc::new(video::crop::Crop::new()),
        Arc::new(video::rotate::Rotate::new()),
        Arc::new(video::speed::Speed::video()),
        Arc::new(video::mute::Mute::new()),
        Arc::new(video::join::Join::new()),
        Arc::new(video::insert::InsertClips::new()),
        Arc::new(video::watermark::Watermark::new()),
        Arc::new(video::captions::Captions::new()),
        Arc::new(video::add_audio::AddAudio::new()),
        Arc::new(video::frames::Frames::new()),
        Arc::new(video::silences::RemoveSilences::new()),
        Arc::new(video::stabilize::Stabilize::new()),
        // Audio
        Arc::new(audio::transcribe::Transcribe::new()),
        Arc::new(audio::merge::Merge::new()),
        Arc::new(audio::convert::AudioConvert::new()),
        Arc::new(audio::compress::AudioCompress::new()),
        Arc::new(video::trim::Trim::audio()),
        Arc::new(video::split::Split::audio()),
        Arc::new(video::speed::Speed::audio()),
        Arc::new(audio::normalize::Normalize::new()),
        Arc::new(audio::denoise::Denoise::new()),
    ]
}
