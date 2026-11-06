//! Integration tests for the audio tools, the engine helpers and (when whisper-cli is present)
//! transcription on a real speech sample.

mod common;
use common::*;

#[test]
fn engine_probe_thumbnail_waveform() {
    let p = probe(&fixture("rotated.mp4"));
    assert_eq!(p.rotation, 90);
    assert_eq!(p.display_size(), (240, 320));
    let p = probe(&fixture("vfr.mkv"));
    assert!(p.has_video);
    let png = engine_media::thumbnail(&fixture("clip.mp4"), None, 128).unwrap();
    assert!(png.starts_with(b"\x89PNG"));
    let png = engine_media::thumbnail(&fixture("tone.mp3"), None, 128).unwrap();
    assert!(png.starts_with(b"\x89PNG"), "audio files get a waveform picture");
    let peaks = engine_media::waveform(&fixture("gaps.wav"), 21, &castor_core::CancelToken::new()).unwrap();
    assert_eq!(peaks.len(), 21);
    // Bucket 0 (0–0.5 s) is tone, bucket 5 (2.5–3 s) is silence.
    assert!(peaks[0].1 > 0.3, "{:?}", peaks[0]);
    assert!(peaks[5].1 < 0.01, "{:?}", peaks[5]);
    assert!(engine_media::probe(&fixture("corrupt.mp4")).is_err());
}

#[test]
fn convert_keeps_bit_depth() {
    let (o, _d) = run("audio.convert", &[fixture("hires.flac")], &[("format", "wav")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.audio_codec.as_deref(), Some("pcm_s24le"));
    assert_eq!(p.sample_rate, 96000);
    let (o, _d) = run("audio.convert", &[fixture("hires.flac")], &[("format", "mp3")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.audio_codec.as_deref(), Some("mp3"));
    assert_eq!(p.sample_rate, 48000);
    let (o, _d) = run("audio.convert", &[fixture("tone.mp3")], &[("format", "opus"), ("channels", "mono")]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!((p.audio_codec.as_deref(), p.channels), (Some("opus"), 1));
}

#[test]
fn compress_never_bigger() {
    let (o, _d) = run("audio.compress", &[fixture("gaps.wav")], &[("content", "voice")]).unwrap();
    assert!(std::fs::metadata(&o[0].path).unwrap().len() < std::fs::metadata(fixture("gaps.wav")).unwrap().len());
    assert_eq!(probe(&o[0].path).channels, 1);
    // A 192 kbps MP3 asked to become "high" (192 kbps): not smaller → original kept.
    let (o, _d) = run("audio.compress", &[fixture("tone.mp3")], &[("quality", "high")]).unwrap();
    assert!(o[0].note.as_deref().unwrap_or("").len() > 3);
    assert!(std::fs::metadata(&o[0].path).unwrap().len() <= std::fs::metadata(fixture("tone.mp3")).unwrap().len());
}

#[test]
fn trim_split_speed() {
    let (o, _d) = run("audio.trim", &[fixture("tone.mp3")], &[("start", "1"), ("end", "3.5")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 2.5, 0.08));
    let (o, _d) = run("audio.split", &[fixture("gaps.wav")], &[("by", "pauses"), ("pause", "1"), ("min_part", "1")]).unwrap();
    assert_eq!(o.len(), 3, "three tones");
    let (o, _d) = run("audio.split", &[fixture("tone.mp3")], &[("by", "length"), ("length", "2")]).unwrap();
    assert_eq!(o.len(), 3);
    let (o, _d) = run("audio.speed", &[fixture("tone.mp3")], &[("factor", "1.5")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 4.0, 0.1));
}

#[test]
fn normalize_loudness_and_peak() {
    let (o, _d) = run("audio.normalize", &[fixture("quiet.wav")], &[]).unwrap();
    let p = probe(&o[0].path);
    assert_eq!(p.audio_codec.as_deref(), Some("pcm_s24le"), "bit depth kept");
    assert_eq!(p.sample_rate, 44100);
    // Measure the result: close to -16 LUFS.
    let cx = castor_core::RunCx::new(std::env::temp_dir(), std::env::temp_dir(), "", castor_core::CancelToken::new());
    let m = engine_media::analyze::loudness(&o[0].path, -16.0, -1.5, 11.0, 5.0, &cx, (0.0, 1.0)).unwrap();
    assert!(approx(m.i, -16.0, 1.0), "integrated {}", m.i);
    assert!(o[0].note.as_deref().unwrap().contains("LUFS"));
    let (o, _d) = run("audio.normalize", &[fixture("quiet.wav")], &[("mode", "peak"), ("peak", "-1")]).unwrap();
    assert!(o[0].note.as_deref().unwrap().contains("-1.0"));
    // Works on a video soundtrack too, video copied.
    let (o, _d) = run("audio.normalize", &[fixture("clip.mp4")], &[("target", "streaming")]).unwrap();
    assert_eq!(probe(&o[0].path).video_codec.as_deref(), Some("h264"));
}

#[test]
fn merge_stitch_and_layer() {
    let (o, _d) = run("audio.merge", &[fixture("tone.mp3"), fixture("gaps.wav")], &[]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 16.5, 0.15));
    let (o, _d) = run("audio.merge", &[fixture("tone.mp3"), fixture("gaps.wav")], &[("crossfade", "1")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 15.5, 0.15));
    let (o, _d) = run("audio.merge", &[fixture("tone.mp3"), fixture("gaps.wav")], &[("mode", "layer"), ("volumes", "100, 30"), ("format", "flac")]).unwrap();
    let p = probe(&o[0].path);
    assert!(approx(p.duration, 10.5, 0.15));
    assert_eq!(p.audio_codec.as_deref(), Some("flac"));
}

#[test]
fn denoise_general() {
    let (o, _d) = run("audio.denoise", &[fixture("noisy.wav")], &[("method", "general")]).unwrap();
    assert!(approx(probe(&o[0].path).duration, 4.0, 0.05));
}

#[test]
fn every_label_is_translated() {
    init();
    for lang in ["en", "fr"] {
        castor_core::i18n::set_language(lang);
        for t in tools_media::tools() {
            let id = t.meta().id;
            for k in ["name", "desc", "verb", "keywords"] {
                assert!(castor_core::i18n::has(&format!("{id}.{k}")), "{lang}: {id}.{k}");
            }
            for o in t.options() {
                let key = o.label_key(id);
                assert!(castor_core::i18n::has(&key), "{lang}: {key}");
                if let castor_core::OptionKind::Choice(cs) = &o.kind {
                    for c in cs.iter().filter(|c| c.literal.is_none()) {
                        assert!(castor_core::i18n::has(&format!("{key}.{}", c.value)), "{lang}: {key}.{}", c.value);
                    }
                }
            }
        }
    }
    castor_core::i18n::set_language("en");
}

/// Real transcription with the tiny model (32 MB, fetched once into the gitignored test
/// profile). Skipped when whisper-cli has not been built (`CASTOR_WHISPER` / PATH).
#[test]
fn transcribe_english_and_french() {
    init();
    if engine_media::whisper::whisper_cli().is_err() {
        eprintln!("skipped: whisper-cli not found");
        return;
    }
    let Some(en) = speech("en-canvassers-tale.mp3") else {
        eprintln!("skipped: speech samples not downloaded");
        return;
    };
    // First 40 s only, to keep the test quick.
    let dir = tempfile::tempdir().unwrap();
    let short = dir.path().join("en40.mp3");
    let st = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-t", "40", "-i"])
        .arg(&en)
        .args(["-c", "copy"])
        .arg(&short)
        .status()
        .unwrap();
    assert!(st.success());
    let (o, _d) = run("audio.transcribe", &[short], &[("model", "base"), ("variant", "q5"), ("format", "all")]).unwrap();
    assert_eq!(o.len(), 6);
    let txt = std::fs::read_to_string(o.iter().find(|x| x.path.extension().unwrap() == "txt").unwrap().path.clone()).unwrap();
    let low = txt.to_lowercase();
    assert!(low.contains("mark twain") && low.contains("public domain"), "{txt}");
    let srt = std::fs::read_to_string(o.iter().find(|x| x.path.extension().unwrap() == "srt").unwrap().path.clone()).unwrap();
    assert!(srt.contains(" --> "));

    if let Some(fr) = speech("fr-anciens-canadiens-01.mp3") {
        let short = dir.path().join("fr30.mp3");
        let st = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", "30", "-t", "30", "-i"])
            .arg(&fr)
            .args(["-c", "copy"])
            .arg(&short)
            .status()
            .unwrap();
        assert!(st.success());
        let (o, _d) = run("audio.transcribe", &[short], &[("model", "base"), ("variant", "q5"), ("format", "json")]).unwrap();
        let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&o[0].path).unwrap()).unwrap();
        assert_eq!(j["language"], "fr");
        assert!(j["segments"].as_array().unwrap().len() > 2);
    }
}
