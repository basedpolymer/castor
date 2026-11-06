//! `audio.transcribe`: speech to TXT / SRT / VTT / JSON / DOCX / PDF with whisper.cpp.

use castor_core::download::{self, Asset};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Requirement, Result, RunCx, Tool, ToolMeta, ToolStatus, tr, tr_args, units};
use engine_media::whisper::{self, Options, Transcript};

use crate::common::*;
use crate::transcript_docs as docs;

pub struct Transcribe {
    meta: ToolMeta,
}

impl Transcribe {
    pub fn new() -> Self {
        Transcribe {
            meta: ToolMeta {
                id: "audio.transcribe",
                family: Family::Audio,
                inputs: InputSpec::kinds(AV),
                output: "TXT · SRT · VTT · DOCX · PDF",
                keywords: &["speech to text", "subtitles", "whisper", "dictation", "captions", "transcript", "interview", "podcast"],
                status: ToolStatus::Ready,
                requires: &[Requirement::Ffmpeg, Requirement::Whisper],
                suffix: "",
                in_place: false,
                serial: true,
            },
        }
    }
}

/// Whisper options shared by transcription and captions. `vad_default`: captions want exact
/// word timings (VAD off), transcripts want robustness on silences (VAD on).
pub fn whisper_options(vad_default: bool) -> Vec<OptionSpec> {
    vec![
        OptionSpec::choice(
            "model",
            vec![Choice::key("tiny"), Choice::key("base"), Choice::key("small"), Choice::key("medium"), Choice::key("turbo")],
            "small",
        ),
        OptionSpec::toggle("detect", true),
        OptionSpec::language("language", whisper::LANGUAGES, false, "en").when(Cond::False("detect")),
        OptionSpec::toggle("translate", false),
        OptionSpec::choice("variant", vec![Choice::key("q5"), Choice::key("q8"), Choice::key("full")], "q8").advanced(),
        OptionSpec::toggle("vad", vad_default).advanced(),
        OptionSpec::toggle("accurate", false).advanced(),
        OptionSpec::text("prompt", "").placeholder("media.ph.prompt").advanced(),
    ]
}

fn english_only(v: &Values) -> bool {
    // The .en models are a little better for English; only when English is chosen explicitly.
    !v.bool("detect") && v.str("language") == "en" && !v.bool("translate") && v.str("model") != "turbo" && v.str("model") != "medium"
}

pub fn whisper_downloads(v: &Values) -> Vec<Asset> {
    whisper::assets(v.str("model"), v.str("variant"), english_only(v), v.bool("vad"))
}

pub fn whisper_unavailable() -> Option<String> {
    need_ffmpeg().or_else(|| whisper::whisper_cli().err().map(|e| e.to_string()))
}

/// Download what is missing (progress 0..`hi`) and build the transcription options.
pub fn prepare(v: &Values, cx: &RunCx, hi: f32) -> Result<Options> {
    let assets = whisper_downloads(v);
    let model_asset = assets.first().cloned().ok_or_else(|| anyhow::anyhow!(tr("media.err.whisper")))?;
    let mut paths = Vec::new();
    for a in &assets {
        if !a.is_present() {
            cx.message(tr_args("media.msg.fetching_model", &[("size", &units::bytes(a.size))]));
        }
        let p = download::ensure(a, &cx.cancel_token(), &|done, total| {
            if total > 0 {
                cx.progress(hi * done as f32 / total as f32);
            }
        })?;
        paths.push(p);
    }
    let lang = if v.bool("detect") { "auto".to_string() } else { v.str("language").to_string() };
    Ok(Options {
        model: download::Asset::path(&model_asset),
        language: if lang.is_empty() { "auto".into() } else { lang },
        translate: v.bool("translate"),
        vad: if v.bool("vad") { paths.get(1).cloned() } else { None },
        beam: v.bool("accurate"),
        prompt: Some(v.str("prompt").to_string()).filter(|s| !s.trim().is_empty()),
        threads: 0,
    })
}

fn lang_name(code: &str) -> String {
    whisper::LANGUAGES.iter().find(|(c, _)| *c == code).map(|(_, n)| n.to_string()).unwrap_or_else(|| code.to_string())
}

impl Tool for Transcribe {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut o = vec![OptionSpec::choice(
            "format",
            vec![
                Choice::lit("txt", "TXT"),
                Choice::lit("srt", "SRT"),
                Choice::lit("vtt", "VTT"),
                Choice::lit("docx", "DOCX"),
                Choice::lit("pdf", "PDF"),
                Choice::lit("json", "JSON"),
                Choice::key("all"),
            ],
            "txt",
        )];
        o.push(OptionSpec::toggle("timestamps", false));
        o.extend(whisper_options(true));
        o
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        whisper_downloads(v)
    }
    fn unavailable(&self) -> Option<String> {
        whisper_unavailable()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let opts = prepare(v, cx, 0.05)?;
        let started = std::time::Instant::now();
        let t = whisper::transcribe(&input.path, &opts, cx, (0.05, 0.97))?;
        anyhow::ensure!(!t.segments.is_empty(), tr("media.err.no_speech"));
        let secs = started.elapsed().as_secs_f64();
        write_outputs(input, &t, v.str("format"), v.bool("timestamps"), cx, secs)
    }
}

/// Write the chosen format(s) next to the input.
pub fn write_outputs(input: &castor_core::FileInfo, t: &Transcript, format: &str, stamps: bool, cx: &RunCx, secs: f64) -> Result<Vec<Output>> {
    let formats: Vec<&str> = if format == "all" { vec!["txt", "srt", "vtt", "docx", "pdf", "json"] } else { vec![format] };
    let words = t.words().count();
    let subtitle = tr_args(
        "media.doc.subtitle",
        &[("file", &input.name()), ("language", &lang_name(&t.language)), ("duration", &units::duration(t.duration))],
    );
    let note = tr_args(
        "media.note.transcribed",
        &[
            ("language", &lang_name(&t.language)),
            ("words", &words.to_string()),
            ("duration", &units::duration(t.duration)),
            ("secs", &units::duration(secs)),
        ],
    );
    let mut outs = Vec::new();
    for f in formats {
        let path = cx.output_path(Some(input), f);
        let bytes: Vec<u8> = match f {
            "srt" => docs::to_srt(t).into_bytes(),
            "vtt" => docs::to_vtt(t).into_bytes(),
            "json" => docs::to_json(t)?.into_bytes(),
            "docx" => docs::to_docx(t, &input.stem(), &subtitle, stamps)?,
            "pdf" => docs::to_pdf(t, &input.stem(), &subtitle, stamps)?,
            _ => docs::to_txt(t, stamps).into_bytes(),
        };
        std::fs::write(&path, bytes)?;
        outs.push(Output::new(path).with_note(note.clone()));
    }
    Ok(outs)
}
