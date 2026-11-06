//! Audio tools.

pub mod compress;
pub mod convert;
pub mod denoise;
pub mod merge;
pub mod normalize;
pub mod transcribe;

use engine_media::Probe;

/// Map the first audio track (and cover art when the format keeps it), tags, and re-encode in
/// the same format at a quality at least as high as the source.
pub fn convert_args_same(p: &Probe, ext: &str) -> Vec<String> {
    let mut a: Vec<String> = vec!["-map".into(), "0:a:0".into()];
    let cover = p.streams.iter().find(|s| s.kind == engine_media::StreamKind::Picture);
    match cover {
        Some(c) if matches!(ext, "mp3" | "m4a" | "flac") => {
            a.extend(["-map".into(), format!("0:{}", c.index), "-c:v".into(), "copy".into(), "-disposition:v:0".into(), "attached_pic".into()]);
        }
        _ => a.push("-vn".into()),
    }
    a.extend(["-map_metadata".into(), "0".into()]);
    a.extend(crate::video::trim::audio_reencode(p, ext));
    if ext == "mp3" {
        a.extend(["-id3v2_version".into(), "3".into()]);
    }
    a
}
