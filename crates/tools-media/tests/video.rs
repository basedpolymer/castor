//! Integration tests for the video tools: run each tool on generated fixtures and probe the
//! outputs (durations, codecs, sizes, rotation).

mod common;
use common::*;

#[test]
fn trim_exact_and_fast() {
    let clip = fixture("clip.mp4");
    let (o, _d) = run("video.trim", &[clip.clone()], &[("start", "1.5"), ("end", "4")]).unwrap();
    let p = probe(&o[0].path);
    assert!(approx(p.duration, 2.5, 0.1), "exact trim duration {}", p.duration);
    assert_eq!(p.video_codec.as_deref(), Some("h264"));
    assert!(p.has_audio);

    let (o, _d) = run("video.trim", &[clip.clone()], &[("start", "0:02"), ("until", "duration"), ("length", "3"), ("cut", "fast")]).unwrap();
    let p = probe(&o[0].path);
    assert!(p.duration > 2.5 && p.duration < 5.5, "fast trim duration {}", p.duration);

    // Bad input is a clear error, not a panic.
    let e = run("video.trim", &[clip.clone()], &[("start", "abc")]).unwrap_err();
    assert!(e.to_string().contains("abc"));
    let e = run("video.trim", &[clip], &[("start", "20")]).unwrap_err();
    assert!(!e.to_string().is_empty());
}

#[test]
fn corrupt_and_unicode() {
    let e = run("video.compress", &[fixture("corrupt.mp4")], &[]).unwrap_err();
    assert!(!format!("{e}").contains("panicked"));
    let (o, _d) = run("video.mute", &[fixture("vidéo été 夏.mp4")], &[]).unwrap();
    assert!(o[0].path.file_name().unwrap().to_string_lossy().contains("夏"));
    assert!(!probe(&o[0].path).has_audio);
}

#[test]
fn convert_remux_and_reencode() {
    let (o, _d) = run("video.convert", &[fixture("clip.mp4")], &[("format", "mkv")]).unwrap();
    assert_eq!(o[0].path.extension().unwrap(), "mkv");
    let p = probe(&o[0].path);
    assert_eq!(p.video_codec.as_deref(), Some("h264"));
    assert!(o[0].note.as_deref().unwrap_or("").contains("→"));

    let (o, _d) = run("video.convert", &[fixture("clip.mp4")], &[("format", "webm"), ("quality", "small")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.video_codec.as_deref(), Some("vp9"));
    assert_eq!(p.audio_codec.as_deref(), Some("opus"));
    assert!(approx(p.duration, 8.0, 0.2));

    // Multi-track MKV → MP4: every audio track kept, SRT converted to mov_text.
    let (o, _d) = run("video.convert", &[fixture("multi.mkv")], &[("format", "mp4")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.audio_tracks, 2);
    assert_eq!(p.subtitle_tracks, 1);
    assert_eq!(p.streams.iter().find(|s| s.kind == engine_media::StreamKind::Subtitle).unwrap().codec, "mov_text");
}

#[test]
fn compress_quality_size_and_never_bigger() {
    let (o, _d) = run("video.compress", &[fixture("clip.mp4")], &[("quality", "small")]).unwrap();
    let p = probe(&o[0].path);
    assert!(approx(p.duration, 8.0, 0.2));
    assert!(std::fs::metadata(&o[0].path).unwrap().len() <= std::fs::metadata(fixture("clip.mp4")).unwrap().len());

    // Target size: 0.1 MB for 8 s.
    let (o, _d) = run("video.compress", &[fixture("clip.mp4")], &[("mode", "size"), ("target", "1")]).unwrap();
    let size = std::fs::metadata(&o[0].path).unwrap().len();
    assert!(size <= 1_050_000, "target size respected: {size}");

    // HEVC + HDR source to H.264: tone-mapped to 8-bit BT.709.
    let (o, _d) = run("video.compress", &[fixture("hdr.mkv")], &[("codec", "h264"), ("quality", "best")]).unwrap();
    let p = probe(&o[0].path);
    if o[0].note.as_deref().unwrap_or("").contains("original") {
        return;
    }
    assert_eq!(p.video_codec.as_deref(), Some("h264"));
    assert_eq!(p.bit_depth, 8);
    assert!(!p.is_hdr());
}

#[test]
fn compress_keeps_rotation_and_odd_sizes() {
    let (o, _d) = run("video.compress", &[fixture("rotated.mp4")], &[("mode", "keep"), ("quality", "best")]).unwrap();
    let p = probe(&o[0].path);
    // Either rotated pixels with no flag, or the original kept untouched with its flag.
    assert_eq!(p.display_size(), (240, 320), "display orientation kept");

    let (o, _d) = run("video.resize", &[fixture("odd.mkv")], &[("preset", "percent"), ("percent", "50")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.width % 2, 0);
    assert_eq!(p.height % 2, 0);
}

#[test]
fn vfr_and_odd_inputs() {
    let (o, _d) = run("video.compress", &[fixture("vfr.mkv")], &[("quality", "best")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 6.0, 0.25), "VFR duration kept");
    let (o, _d) = run("video.convert", &[fixture("odd.mkv")], &[("format", "mp4"), ("remux", "false")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.video_codec.as_deref(), Some("h264"));
    assert!(p.width % 2 == 0 && p.height % 2 == 0 && p.width >= 320);
    let (o, _d) = run("video.trim", &[fixture("vfr.mkv")], &[("start", "2"), ("end", "5")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 3.0, 0.2));
}

#[test]
fn gif_and_preview() {
    let (o, _d) = run("video.to-gif", &[fixture("clip.mp4")], &[("preset", "small"), ("start", "1"), ("length", "2")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.video_codec.as_deref(), Some("gif"));
    assert_eq!(p.width, 320);
    let t = tool("video.to-gif");
    let specs = t.options();
    let v = castor_core::Values::defaults(&specs);
    let pv = t.preview(Some(&castor_core::FileInfo::from_path(fixture("clip.mp4"))), &v, 200).unwrap().unwrap();
    assert!(pv.png.starts_with(b"\x89PNG"));
    assert!(pv.caption.unwrap().contains("320"));
}

#[test]
fn extract_audio_lossless_and_tracks() {
    let (o, _d) = run("video.extract-audio", &[fixture("clip.mp4")], &[]).unwrap();
    assert_eq!(o[0].path.extension().unwrap(), "m4a");
    assert_eq!(probe(&o[0].path).audio_codec.as_deref(), Some("aac"));
    let (o, _d) = run("video.extract-audio", &[fixture("multi.mkv")], &[("tracks", "all")]).unwrap();
    assert_eq!(o.len(), 2);
    assert_eq!(o[0].path.extension().unwrap(), "flac");
    assert_eq!(probe(&o[0].path).channels, 6);
    let (o, _d) = run("video.extract-audio", &[fixture("clip.mp4")], &[("format", "mp3"), ("bitrate", "128")]).unwrap();
    assert_eq!(probe(&o[0].path).audio_codec.as_deref(), Some("mp3"));
}

#[test]
fn split_parts_scenes_length() {
    let (o, _d) = run("video.split", &[fixture("clip.mp4")], &[("parts", "4")]).unwrap();
    assert_eq!(o.len(), 4);
    let total: f64 = o.iter().map(|x| probe(&x.path).duration).sum();
    assert!(approx(total, 8.0, 0.3), "parts add up: {total}");
    let (o, _d) = run("video.split", &[fixture("scenes.mp4")], &[("by", "scenes")]).unwrap();
    assert_eq!(o.len(), 3, "three scenes");
    let (o, _d) = run("video.split", &[fixture("clip.mp4")], &[("by", "length"), ("length", "3"), ("cut", "fast")]).unwrap();
    assert!(o.len() >= 2);
}

#[test]
fn crop_rotate_speed_mute() {
    let (o, _d) = run("video.crop", &[fixture("clip.mp4")], &[("mode", "1:1")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!((p.width, p.height), (240, 240));
    let (o, _d) = run("video.crop", &[fixture("bars.mp4")], &[("mode", "auto")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!((p.width, p.height), (320, 180), "black bars removed");
    // 9:16 on a rotated (portrait-displayed) video works on the displayed picture.
    let (o, _d) = run("video.crop", &[fixture("rotated.mp4")], &[("mode", "1:1")]).unwrap();
    assert_eq!(probe(&o[0].path).display_size(), (240, 240));

    let (o, _d) = run("video.rotate", &[fixture("clip.mp4")], &[("angle", "90")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.display_size(), (240, 320));
    assert_eq!(p.width, 320, "lossless: pixels untouched");
    let (o, _d) = run("video.rotate", &[fixture("clip.mp4")], &[("angle", "270"), ("lossless", "false")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!((p.width, p.height, p.rotation), (240, 320, 0));

    let (o, _d) = run("video.speed", &[fixture("clip.mp4")], &[("factor", "2")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 4.0, 0.2));
    let (o, _d) = run("video.speed", &[fixture("clip.mp4")], &[("factor", "0.5"), ("pitch", "shift")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 16.0, 0.3));

    let (o, _d) = run("video.mute", &[fixture("multi.mkv")], &[]).unwrap();
    let p = probe(&o[0].path);
    assert!(!p.has_audio && p.subtitle_tracks == 1);
}

#[test]
fn join_copy_and_reencode() {
    let c = fixture("clip.mp4");
    let (o, _d) = run("video.join", &[c.clone(), c.clone()], &[]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 16.0, 0.2));
    let (o, _d) = run("video.join", &[c.clone(), fixture("other.mov"), fixture("clip2.mp4")], &[]).unwrap();
    let p = probe(&o[0].path);
    assert!(approx(p.duration, 15.0, 0.3), "joined {}", p.duration);
    assert_eq!((p.width, p.height), (320, 240));
    assert!(p.has_audio);
}

#[test]
fn insert_clips_at_pause() {
    let (o, _d) = run(
        "video.insert-clips",
        &[fixture("gaps.mp4")],
        &[("clip", fixture("clip2.mp4").to_str().unwrap()), ("after", "1"), ("outro", fixture("other.mov").to_str().unwrap())],
    )
    .unwrap();
    let p = probe(&o[0].path);
    assert!(approx(p.duration, 10.5 + 4.0 + 3.0, 0.4), "{}", p.duration);
    assert!(o[0].note.as_deref().unwrap().contains("00:00:02.7"), "{:?}", o[0].note);
}

#[test]
fn watermark_text_and_image() {
    let (o, _d) = run("video.watermark", &[fixture("clip.mp4")], &[("kind", "text"), ("text", "Castor: test 100%")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 8.0, 0.2));
    let (o, _d) = run("video.watermark", &[fixture("rotated.mp4")], &[("image", fixture("logo.png").to_str().unwrap()), ("position", "top-left")]).unwrap();
    assert_eq!(probe(&o[0].path).display_size(), (240, 320));
}

#[test]
fn add_audio_frames_silences_stabilize() {
    let (o, _d) = run("video.add-audio", &[fixture("clip.mp4")], &[("audio", fixture("gaps.wav").to_str().unwrap()), ("mode", "mix")]).unwrap();
    let p = probe(&o[0].path);
    assert!(approx(p.duration, 8.0, 0.2) && p.has_audio);

    let (o, _d) = run("video.frames", &[fixture("clip.mp4")], &[("count", "3"), ("format", "png")]).unwrap();
    assert_eq!(o.len(), 3);
    let (o, _d) = run("video.frames", &[fixture("clip.mp4")], &[("mode", "sheet"), ("columns", "3"), ("rows", "2")]).unwrap();
    assert_eq!(o.len(), 1);
    assert!(probe(&o[0].path).width > 400);

    let (o, _d) = run("video.remove-silences", &[fixture("gaps.mp4")], &[("min_silence", "0.5"), ("padding", "0.1")]).unwrap();
    let p = probe(&o[0].path);
    // 3 tones × 2 s + padding, the 1.5 s gaps (and the trailing one) removed.
    assert!(p.duration > 6.0 && p.duration < 7.6, "tightened to {}", p.duration);

    let (o, _d) = run("video.stabilize", &[fixture("clip2.mp4")], &[("strength", "3")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 4.0, 0.2));
}

#[test]
fn captions_from_file_burn_and_soft() {
    let srt = fixtures_dir().join("multi.srt");
    let _ = fixture("multi.mkv");
    for style in ["classic", "box", "minimal"] {
        let (o, _d) = run("video.captions", &[fixture("clip.mp4")], &[("source", "file"), ("subtitles", srt.to_str().unwrap()), ("style", style)]).unwrap();
        assert_eq!(o.len(), 2, "video + srt");
        assert!(approx(probe(&o[0].path).duration, 8.0, 0.2));
    }
    let (o, _d) = run("video.captions", &[fixture("clip.mp4")], &[("source", "file"), ("subtitles", srt.to_str().unwrap()), ("mode", "soft"), ("export_srt", "false")]).unwrap();
    assert_eq!(o.len(), 1);
    assert_eq!(probe(&o[0].path).subtitle_tracks, 1);
}
