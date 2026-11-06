//! Test helpers: generated fixtures (ffmpeg lavfi sources, cached in
//! `tests/fixtures/generated/media/`) and a direct way to run one tool.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, Once};

use castor_core::options::Values;
use castor_core::{CancelToken, FileInfo, Job, Output, RunCx, Tool};

pub fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

pub fn fixtures_dir() -> PathBuf {
    let d = workspace().join("tests/fixtures/generated/media");
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Point Castor's profile (downloads: whisper models) at a persistent, gitignored folder,
/// and make whisper-cli built under target-c/ext visible.
pub fn init() {
    static O: Once = Once::new();
    O.call_once(|| unsafe {
        let home = workspace().join("tests/fixtures/generated/castor-home");
        std::env::set_var("CASTOR_HOME", &home);
        if std::env::var_os("CASTOR_WHISPER").is_none() {
            for c in ["target-c/ext/whisper.cpp/build/bin/Release/whisper-cli.exe", "target-c/ext/whisper.cpp/build/bin/whisper-cli"] {
                let p = workspace().join(c);
                if p.is_file() {
                    std::env::set_var("CASTOR_WHISPER", p);
                }
            }
        }
    });
}

fn ff(args: &[&str]) {
    let st = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .status()
        .expect("ffmpeg on PATH");
    assert!(st.success(), "ffmpeg {args:?}");
}

static GEN: Mutex<()> = Mutex::new(());

/// A generated fixture by name (created on first use).
pub fn fixture(name: &str) -> PathBuf {
    init();
    // Dependencies first: the generation lock is not re-entrant.
    match name {
        "rotated.mp4" | "vidéo été 夏.mp4" => {
            fixture("clip.mp4");
        }
        "gaps.mp4" => {
            fixture("gaps.wav");
        }
        _ => {}
    }
    let _g = GEN.lock().unwrap_or_else(|e| e.into_inner());
    let dir = fixtures_dir();
    let p = dir.join(name);
    if p.is_file() {
        return p;
    }
    let tmp = dir.join(format!("tmp-{name}"));
    let t = tmp.to_str().unwrap();
    match name {
        // 8 s, 320x240 30 fps, stereo 48 kHz AAC.
        "clip.mp4" => ff(&[
            "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=8", "-f", "lavfi", "-i", "sine=f=440:d=8:sample_rate=48000",
            "-ac", "2", "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k", "-shortest", t,
        ]),
        "clip2.mp4" => ff(&[
            "-f", "lavfi", "-i", "smptebars=s=320x240:r=30:d=4", "-f", "lavfi", "-i", "sine=f=660:d=4:sample_rate=48000",
            "-ac", "2", "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-b:a", "128k", "-shortest", t,
        ]),
        // Different size, frame rate and no audio: forces a re-encoding join.
        "other.mov" => ff(&["-f", "lavfi", "-i", "mandelbrot=s=480x270:r=25", "-t", "3", "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", t]),
        // Phone-style rotation flag (displayed portrait 240x320).
        "rotated.mp4" => {
            let c = dir.join("clip.mp4");
            ff(&["-display_rotation", "90", "-i", c.to_str().unwrap(), "-c", "copy", t])
        }
        // Odd dimensions (4:4:4 so x264 accepts them), MKV.
        "odd.mkv" => ff(&[
            "-f", "lavfi", "-i", "testsrc2=s=321x241:r=24:d=4", "-f", "lavfi", "-i", "sine=f=300:d=4",
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv444p", "-c:a", "libopus", "-shortest", t,
        ]),
        // Variable frame rate: 30 fps then 10 fps.
        "vfr.mkv" => ff(&[
            "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=3", "-f", "lavfi", "-i", "testsrc2=s=320x240:r=10:d=3",
            "-filter_complex", "[0][1]concat=n=2:v=1:a=0[v]", "-map", "[v]", "-fps_mode", "vfr", "-c:v", "libx264", "-preset", "veryfast", t,
        ]),
        // 5.1 FLAC + stereo AAC commentary + SRT subtitles.
        "multi.mkv" => {
            let srt = dir.join("multi.srt");
            std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:02,000\nHello\n\n2\n00:00:03,000 --> 00:00:04,500\nWorld\n").unwrap();
            ff(&[
                "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=5",
                "-f", "lavfi", "-i", "sine=f=220:d=5:sample_rate=48000",
                "-f", "lavfi", "-i", "sine=f=880:d=5:sample_rate=48000",
                "-i", srt.to_str().unwrap(),
                "-filter_complex", "[1]pan=5.1|FL=c0|FR=c0|FC=c0|LFE=c0|BL=c0|BR=c0[a6]",
                "-map", "0:v", "-map", "[a6]", "-map", "2:a", "-map", "3:s",
                "-c:v", "libx264", "-preset", "veryfast", "-c:a:0", "flac", "-c:a:1", "aac", "-ac:a:1", "2", "-c:s", "srt",
                "-metadata:s:a:0", "language=eng", "-metadata:s:a:1", "language=fre", t,
            ])
        }
        // Tone 2 s / silence 1.5 s, three times (10.5 s), 48 kHz mono WAV 16-bit.
        "gaps.wav" => ff(&[
            "-f", "lavfi", "-i", "aevalsrc='if(lt(mod(t,3.5),2),0.5*sin(2*PI*440*t),0)':d=10.5:s=48000", "-c:a", "pcm_s16le", t,
        ]),
        "gaps.mp4" => {
            let g = dir.join("gaps.wav");
            ff(&[
                "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=10.5", "-i", g.to_str().unwrap(),
                "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", t,
            ])
        }
        // HDR10-tagged HEVC 10-bit.
        "hdr.mkv" => ff(&[
            "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=2",
            "-vf", "format=yuv420p10le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc",
            "-c:v", "libx265", "-preset", "ultrafast",
            "-x265-params", "log-level=error:hdr10=1:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc:max-cll=1000,400", t,
        ]),
        // Letterboxed picture (black bars top and bottom).
        "bars.mp4" => ff(&[
            "-f", "lavfi", "-i", "testsrc2=s=320x180:r=30:d=3", "-vf", "pad=320:240:0:30:black",
            "-c:v", "libx264", "-preset", "veryfast", "-pix_fmt", "yuv420p", t,
        ]),
        // Three hard cuts: test pattern, red, bars (3 s each).
        "scenes.mp4" => ff(&[
            "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=3", "-f", "lavfi", "-i", "color=c=red:s=320x240:r=30:d=3",
            "-f", "lavfi", "-i", "smptebars=s=320x240:r=30:d=3", "-filter_complex", "[0][1][2]concat=n=3:v=1:a=0,format=yuv420p[v]",
            "-map", "[v]", "-c:v", "libx264", "-preset", "veryfast", t,
        ]),
        // 24-bit / 96 kHz FLAC.
        "hires.flac" => ff(&["-f", "lavfi", "-i", "sine=f=1000:d=3:sample_rate=96000", "-c:a", "flac", "-sample_fmt", "s32", "-bits_per_raw_sample", "24", t]),
        "tone.mp3" => ff(&["-f", "lavfi", "-i", "sine=f=500:d=6:sample_rate=44100", "-ac", "2", "-c:a", "libmp3lame", "-b:a", "192k", t]),
        "quiet.wav" => ff(&["-f", "lavfi", "-i", "sine=f=440:d=5:sample_rate=44100", "-af", "volume=-30dB", "-c:a", "pcm_s24le", t]),
        "noisy.wav" => ff(&[
            "-f", "lavfi", "-i", "sine=f=300:d=4:sample_rate=48000", "-f", "lavfi", "-i", "anoisesrc=d=4:c=white:a=0.08:r=48000",
            "-filter_complex", "[0][1]amix=inputs=2:normalize=0", "-c:a", "pcm_s16le", t,
        ]),
        "logo.png" => ff(&["-f", "lavfi", "-i", "color=c=0xff8800:s=64x32", "-frames:v", "1", t]),
        "corrupt.mp4" => std::fs::write(&tmp, b"this is not a video at all, just text pretending").unwrap(),
        // Unicode file name.
        "vidéo été 夏.mp4" => {
            let c = dir.join("clip.mp4");
            std::fs::copy(&c, &tmp).unwrap();
        }
        other => panic!("unknown fixture {other}"),
    }
    std::fs::rename(&tmp, &p).unwrap();
    p
}

/// A real speech sample (LibriVox, public domain) if it has been downloaded into
/// `tests/fixtures/generated/media/speech/`.
pub fn speech(name: &str) -> Option<PathBuf> {
    let p = fixtures_dir().join("speech").join(name);
    p.is_file().then_some(p)
}

pub fn tool(id: &str) -> std::sync::Arc<dyn Tool> {
    init();
    tools_media::tools().into_iter().find(|t| t.meta().id == id).unwrap_or_else(|| panic!("no tool {id}"))
}

/// Run `id` on `files` with `--set`-style values; outputs go to a fresh temp folder.
pub fn run(id: &str, files: &[PathBuf], sets: &[(&str, &str)]) -> anyhow::Result<(Vec<Output>, tempfile::TempDir)> {
    let t = tool(id);
    let specs = t.options();
    let mut v = Values::defaults(&specs);
    for (k, val) in sets {
        let s = specs.iter().find(|s| s.key == *k).unwrap_or_else(|| panic!("{id} has no option {k}"));
        v.set(k, s.parse(val)?);
    }
    let infos: Vec<FileInfo> = files.iter().map(FileInfo::from_path).collect();
    for f in &infos {
        assert!(t.accepts(f), "{id} does not accept {}", f.path.display());
    }
    let jobs = t.plan(&infos, &v)?;
    let out = tempfile::tempdir()?;
    let mut all = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        let tmp = out.path().join(format!(".tmp{i}"));
        std::fs::create_dir_all(&tmp)?;
        let cx = RunCx::new(out.path().to_path_buf(), tmp.clone(), t.meta().suffix, CancelToken::new());
        let r = t.run(job as &Job, &v, &cx);
        let _ = std::fs::remove_dir_all(&tmp);
        all.extend(r?);
    }
    Ok((all, out))
}

pub fn probe(p: &Path) -> engine_media::Probe {
    engine_media::probe(p).unwrap_or_else(|e| panic!("probe {}: {e:#}", p.display()))
}

pub fn approx(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}
