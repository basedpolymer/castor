//! Frames → video: raw RGB frames piped into ffmpeg (engine-media's binary), encoded as
//! H.264 MP4, VP9 WebM (optionally with alpha) or ProRes 4444 MOV. Cancel kills ffmpeg.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Stdio};

use anyhow::{Result, anyhow, bail};
use castor_core::RunCx;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    /// H.264 in MP4, yuv420p, faststart (plays everywhere).
    Mp4,
    /// VP9 in WebM.
    Webm,
    /// VP9 in WebM with an alpha channel.
    WebmAlpha,
    /// ProRes 4444 with alpha in MOV (editing software).
    ProresAlpha,
    /// Animated GIF (palette per video).
    Gif,
}

impl Codec {
    pub fn from_ext(ext: &str) -> Codec {
        match ext {
            "webm" => Codec::Webm,
            "mov" => Codec::ProresAlpha,
            "gif" => Codec::Gif,
            _ => Codec::Mp4,
        }
    }
    pub fn has_alpha(self) -> bool {
        matches!(self, Codec::WebmAlpha | Codec::ProresAlpha)
    }
}

pub struct FrameSink {
    child: Child,
    stdin: Option<ChildStdin>,
    err: Option<std::thread::JoinHandle<String>>,
    frame_len: usize,
    cx: RunCx,
}

impl FrameSink {
    /// Start ffmpeg. Frames must be `w × h` RGB (or RGBA for alpha codecs), `fps` per second.
    /// `audio`: optional audio file muxed in (shortest wins).
    pub fn start(out: &Path, w: u32, h: u32, fps: u32, codec: Codec, audio: Option<&Path>, cx: &RunCx) -> Result<FrameSink> {
        anyhow::ensure!(w % 2 == 0 && h % 2 == 0, "video size must be even");
        let ff = engine_media::ffmpeg()?;
        let mut cmd = castor_core::process::command(&ff);
        let pix_in = if codec.has_alpha() { "rgba" } else { "rgb24" };
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
        cmd.args(["-f", "rawvideo", "-pix_fmt", pix_in, "-s", &format!("{w}x{h}"), "-r", &fps.to_string(), "-i", "-"]);
        if let Some(a) = audio {
            cmd.arg("-i").arg(a);
        }
        match codec {
            Codec::Mp4 => {
                cmd.args(["-c:v", "libx264", "-preset", "medium", "-crf", "17", "-pix_fmt", "yuv420p", "-movflags", "+faststart"]);
            }
            Codec::Webm => {
                cmd.args(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-row-mt", "1", "-pix_fmt", "yuv420p"]);
            }
            Codec::WebmAlpha => {
                cmd.args(["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-row-mt", "1", "-pix_fmt", "yuva420p", "-auto-alt-ref", "0"]);
            }
            Codec::ProresAlpha => {
                cmd.args(["-c:v", "prores_ks", "-profile:v", "4444", "-pix_fmt", "yuva444p10le", "-vendor", "apl0"]);
            }
            Codec::Gif => {
                cmd.args(["-filter_complex", "[0:v]split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a"]);
                cmd.args(["-loop", "0"]);
            }
        }
        if audio.is_some() {
            match codec {
                Codec::Webm | Codec::WebmAlpha => cmd.args(["-c:a", "libopus", "-b:a", "128k"]),
                Codec::Gif => cmd.args(["-an"]),
                _ => cmd.args(["-c:a", "aac", "-b:a", "192k"]),
            };
            cmd.arg("-shortest");
        }
        cmd.arg(out);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| anyhow!("could not start ffmpeg: {e}"))?;
        let stdin = child.stdin.take();
        let mut stderr = child.stderr.take().ok_or_else(|| anyhow!("ffmpeg: no stderr"))?;
        let err = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let bpp = if codec.has_alpha() { 4 } else { 3 };
        Ok(FrameSink { child, stdin, err: Some(err), frame_len: (w * h) as usize * bpp, cx: cx.clone() })
    }

    pub fn push(&mut self, frame: &[u8]) -> Result<()> {
        if let Err(e) = self.cx.check_cancel() {
            self.kill();
            return Err(e);
        }
        anyhow::ensure!(frame.len() == self.frame_len, "frame has {} bytes, expected {}", frame.len(), self.frame_len);
        let stdin = self.stdin.as_mut().ok_or_else(|| anyhow!("ffmpeg closed"))?;
        if let Err(e) = stdin.write_all(frame) {
            self.stdin = None;
            let _ = self.child.wait();
            let log = self.err.take().and_then(|t| t.join().ok()).unwrap_or_default();
            bail!("ffmpeg stopped: {e}\n{}", log.trim());
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let st = self.child.wait()?;
        let log = self.err.take().and_then(|t| t.join().ok()).unwrap_or_default();
        if !st.success() {
            bail!("ffmpeg failed ({st}):\n{}", log.trim());
        }
        Ok(())
    }

    fn kill(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for FrameSink {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            self.kill();
        }
    }
}

/// RGBA pixmap data (premultiplied tiny-skia) → straight RGB bytes over `bg`.
pub fn pixmap_rgb(pm: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    let mut v = Vec::with_capacity((pm.width() * pm.height() * 3) as usize);
    for p in pm.pixels() {
        // Premultiplied over black = the colour itself when opaque.
        v.extend_from_slice(&[p.red(), p.green(), p.blue()]);
    }
    v
}

/// tiny-skia pixmap → straight RGBA bytes.
pub fn pixmap_rgba(pm: &resvg::tiny_skia::Pixmap) -> Vec<u8> {
    let mut v = Vec::with_capacity((pm.width() * pm.height() * 4) as usize);
    for p in pm.pixels() {
        let c = p.demultiply();
        v.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    v
}

/// Is an ffmpeg binary available?
pub fn available() -> bool {
    engine_media::ffmpeg().is_ok()
}
