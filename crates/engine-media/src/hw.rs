//! Hardware video encoders (NVENC, Quick Sync, AMF on Windows; VideoToolbox on macOS).
//!
//! Each candidate is tried once per process with a tiny test encode; the first that works is
//! used. Tools still fall back to software when a real encode fails (old drivers, sizes the
//! hardware refuses, too many sessions).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::sync::OnceLock;

use parking_lot::Mutex;

use crate::ff::{ffmpeg, run_simple};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    H264,
    Hevc,
    Av1,
}

fn candidates(c: Codec) -> &'static [&'static str] {
    #[cfg(target_os = "macos")]
    {
        match c {
            Codec::H264 => &["h264_videotoolbox"],
            Codec::Hevc => &["hevc_videotoolbox"],
            Codec::Av1 => &[],
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        match c {
            Codec::H264 => &["h264_nvenc", "h264_qsv", "h264_amf"],
            Codec::Hevc => &["hevc_nvenc", "hevc_qsv", "hevc_amf"],
            Codec::Av1 => &["av1_nvenc", "av1_qsv", "av1_amf"],
        }
    }
}

fn cache() -> &'static Mutex<HashMap<Codec, Option<&'static str>>> {
    static C: OnceLock<Mutex<HashMap<Codec, Option<&'static str>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

/// Working hardware encoder for `codec` on this machine, if any (cached).
pub fn encoder(codec: Codec) -> Option<&'static str> {
    if let Some(v) = cache().lock().get(&codec) {
        return *v;
    }
    let found = candidates(codec).iter().copied().find(|e| works(e));
    cache().lock().insert(codec, found);
    found
}

fn works(enc: &str) -> bool {
    let Ok(ff) = ffmpeg() else { return false };
    let args = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=320x240:r=30:d=0.2",
        "-pix_fmt",
        "yuv420p",
        "-c:v",
        enc,
        "-f",
        "null",
        "-",
    ];
    let a: Vec<&OsStr> = args.iter().map(OsStr::new).collect();
    run_simple(&ff, &a, &castor_core::CancelToken::new()).is_ok()
}

/// Software encoder for `codec`.
pub fn software(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "libx264",
        Codec::Hevc => "libx265",
        Codec::Av1 => "libsvtav1",
    }
}

/// Quality arguments for a hardware encoder. `level` 0 (best) ..= 2 (smallest).
/// `fallback_bitrate` (bits/s) is used where constant quality is not supported.
pub fn quality_args(enc: &str, level: usize, fallback_bitrate: u64) -> Vec<String> {
    let l = level.min(2);
    if enc.ends_with("_nvenc") {
        let cq = [21, 26, 31][l];
        vec!["-preset".into(), "p5".into(), "-tune".into(), "hq".into(), "-rc".into(), "vbr".into(), "-cq".into(), cq.to_string(), "-b:v".into(), "0".into()]
    } else if enc.ends_with("_qsv") {
        let q = [21, 25, 30][l];
        vec!["-preset".into(), "slower".into(), "-global_quality".into(), q.to_string()]
    } else if enc.ends_with("_amf") {
        let q = [24, 28, 33][l];
        vec!["-quality".into(), "quality".into(), "-rc".into(), "cqp".into(), "-qp_i".into(), q.to_string(), "-qp_p".into(), (q + 2).to_string(), "-qp_b".into(), (q + 4).to_string()]
    } else if enc.ends_with("_videotoolbox") {
        // Constant quality (-q:v) only exists on Apple Silicon: use an average bitrate.
        vec!["-b:v".into(), fallback_bitrate.max(300_000).to_string(), "-allow_sw".into(), "1".into()]
    } else {
        Vec::new()
    }
}
