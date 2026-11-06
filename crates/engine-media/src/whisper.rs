//! Speech to text with whisper.cpp.
//!
//! whisper.cpp (MIT) runs as a sidecar program, `whisper-cli`, bundled next to ffmpeg (or found
//! on PATH in development, or pointed to by `CASTOR_WHISPER`). It is built from the pinned tag
//! [`WHISPER_CPP_VERSION`] (see the spec's packaging notes). Models are ggml files (MIT)
//! downloaded on demand from the whisper.cpp Hugging Face repository at a pinned revision, with
//! SHA-256 checks.
//!
//! Pipeline: ffmpeg decodes any audio/video to 16 kHz mono WAV in the job's temp folder; long
//! files are cut into chunks at pauses (bounded memory, steady progress); each chunk goes
//! through `whisper-cli -ojf` (full JSON with token timings); tokens are merged into words.

use std::path::{Path, PathBuf};

use castor_core::download::Asset;
use castor_core::{RunCx, tr};
use serde::Serialize;

use crate::Ffmpeg;

/// whisper.cpp release the sidecar is built from.
pub const WHISPER_CPP_VERSION: &str = "v1.9.5";

macro_rules! model {
    ($file:literal, $sha:literal, $size:literal) => {
        Asset { id: concat!("whisper/", $file), url: concat!("https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/", $file), sha256: $sha, size: $size, license: "MIT" }
    };
}

/// Every ggml model Castor knows, with pinned hashes (Hugging Face LFS SHA-256).
pub static MODEL_ASSETS: &[(&str, Asset)] = &[
    ("tiny", model!("ggml-tiny.bin", "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21", 77691713)),
    ("tiny-q8", model!("ggml-tiny-q8_0.bin", "c2085835d3f50733e2ff6e4b41ae8a2b8d8110461e18821b09a15c40c42d1cca", 43537433)),
    ("tiny-q5", model!("ggml-tiny-q5_1.bin", "818710568da3ca15689e31a743197b520007872ff9576237bda97bd1b469c3d7", 32152673)),
    ("tiny.en", model!("ggml-tiny.en.bin", "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f", 77704715)),
    ("tiny.en-q8", model!("ggml-tiny.en-q8_0.bin", "5bc2b3860aa151a4c6e7bb095e1fcce7cf12c7b020ca08dcec0c6d018bb7dd94", 43550795)),
    ("tiny.en-q5", model!("ggml-tiny.en-q5_1.bin", "c77c5766f1cef09b6b7d47f21b546cbddd4157886b3b5d6d4f709e91e66c7c2b", 32166155)),
    ("base", model!("ggml-base.bin", "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe", 147951465)),
    ("base-q8", model!("ggml-base-q8_0.bin", "c577b9a86e7e048a0b7eada054f4dd79a56bbfa911fbdacf900ac5b567cbb7d9", 81768585)),
    ("base-q5", model!("ggml-base-q5_1.bin", "422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898", 59707625)),
    ("base.en", model!("ggml-base.en.bin", "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002", 147964211)),
    ("base.en-q8", model!("ggml-base.en-q8_0.bin", "a4d4a0768075e13cfd7e19df3ae2dbc4a68d37d36a7dad45e8410c9a34f8c87e", 81781811)),
    ("base.en-q5", model!("ggml-base.en-q5_1.bin", "4baf70dd0d7c4247ba2b81fafd9c01005ac77c2f9ef064e00dcf195d0e2fdd2f", 59721011)),
    ("small", model!("ggml-small.bin", "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b", 487601967)),
    ("small-q8", model!("ggml-small-q8_0.bin", "49c8fb02b65e6049d5fa6c04f81f53b867b5ec9540406812c643f177317f779f", 264464607)),
    ("small-q5", model!("ggml-small-q5_1.bin", "ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb", 190085487)),
    ("small.en", model!("ggml-small.en.bin", "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d", 487614201)),
    ("small.en-q8", model!("ggml-small.en-q8_0.bin", "67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067", 264477561)),
    ("small.en-q5", model!("ggml-small.en-q5_1.bin", "bfdff4894dcb76bbf647d56263ea2a96645423f1669176f4844a1bf8e478ad30", 190098681)),
    ("medium", model!("ggml-medium.bin", "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208", 1533763059)),
    ("medium-q8", model!("ggml-medium-q8_0.bin", "42a1ffcbe4167d224232443396968db4d02d4e8e87e213d3ee2e03095dea6502", 823369779)),
    ("medium-q5", model!("ggml-medium-q5_0.bin", "19fea4b380c3a618ec4723c3eef2eb785ffba0d0538cf43f8f235e7b3b34220f", 539212467)),
    ("medium.en", model!("ggml-medium.en.bin", "cc37e93478338ec7700281a7ac30a10128929eb8f427dda2e865faa8f6da4356", 1533774781)),
    ("medium.en-q8", model!("ggml-medium.en-q8_0.bin", "43fa2cd084de5a04399a896a9a7a786064e221365c01700cea4666005218f11c", 823382461)),
    ("medium.en-q5", model!("ggml-medium.en-q5_0.bin", "76733e26ad8fe1c7a5bf7531a9d41917b2adc0f20f2e4f5531688a8c6cd88eb0", 539225533)),
    ("turbo", model!("ggml-large-v3-turbo.bin", "1fc70f774d38eb169993ac391eea357ef47c88757ef72ee5943879b7e8e2bc69", 1624555275)),
    ("turbo-q8", model!("ggml-large-v3-turbo-q8_0.bin", "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1", 874188075)),
    ("turbo-q5", model!("ggml-large-v3-turbo-q5_0.bin", "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2", 574041195)),
];

/// Silero VAD v6.2.0 converted to ggml (MIT), used by whisper.cpp's `--vad`.
pub static VAD_ASSET: Asset = Asset {
    id: "whisper/ggml-silero-v6.2.0.bin",
    url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v6.2.0.bin",
    sha256: "2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987",
    size: 885098,
    license: "MIT",
};

/// Model sizes offered in the UI, smallest first.
pub const MODELS: &[&str] = &["tiny", "base", "small", "medium", "turbo"];
/// Variants: full precision, 8-bit or 5-bit quantised.
pub const VARIANTS: &[&str] = &["full", "q8", "q5"];

/// Catalog key for (size, variant, english-only). Turbo has no English-only variant.
pub fn model_key(model: &str, variant: &str, english_only: bool) -> String {
    let mut k = model.to_string();
    if english_only && model != "turbo" {
        k.push_str(".en");
    }
    match variant {
        "q8" => k.push_str("-q8"),
        "q5" => k.push_str("-q5"),
        _ => {}
    }
    k
}

/// The downloadable model for (size, variant, english-only).
pub fn model_asset(model: &str, variant: &str, english_only: bool) -> Option<Asset> {
    let key = model_key(model, variant, english_only);
    MODEL_ASSETS.iter().find(|(k, _)| *k == key).map(|(_, a)| a.clone())
}

/// What one run needs downloaded (model + VAD model when used).
pub fn assets(model: &str, variant: &str, english_only: bool, vad: bool) -> Vec<Asset> {
    let mut v: Vec<Asset> = model_asset(model, variant, english_only).into_iter().collect();
    if vad {
        v.push(VAD_ASSET.clone());
    }
    v
}

/// Located `whisper-cli`, or a clear error.
pub fn whisper_cli() -> anyhow::Result<PathBuf> {
    if let Some(p) = std::env::var_os("CASTOR_WHISPER").map(PathBuf::from).filter(|p| p.is_file()) {
        return Ok(p);
    }
    castor_core::paths::find_program("whisper-cli")
        .ok_or_else(|| anyhow::anyhow!(castor_core::tr_args("media.err.missing_program", &[("program", "whisper-cli")])))
}

/// True if transcription can run here (sidecar present).
pub fn available() -> bool {
    whisper_cli().is_ok()
}

/// Options of one transcription.
#[derive(Clone, Debug)]
pub struct Options {
    /// Local path of the ggml model (from [`model_asset`] + `download::ensure`).
    pub model: PathBuf,
    /// ISO 639-1 code, or "auto".
    pub language: String,
    /// Translate to English.
    pub translate: bool,
    /// Path of the Silero VAD model to skip non-speech (None = no VAD).
    pub vad: Option<PathBuf>,
    /// Beam search (slower, a little more accurate) instead of greedy decoding.
    pub beam: bool,
    /// Optional initial prompt (names, jargon).
    pub prompt: Option<String>,
    /// CPU threads (0 = automatic).
    pub threads: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Word {
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Confidence 0..1 (mean token probability).
    pub p: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub text: String,
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Transcript {
    /// Detected or chosen language code.
    pub language: String,
    pub duration: f64,
    pub segments: Vec<Segment>,
}

impl Transcript {
    /// All words in order.
    pub fn words(&self) -> impl Iterator<Item = &Word> {
        self.segments.iter().flat_map(|s| s.words.iter())
    }
    /// Plain text, one paragraph per pause longer than `pause` seconds.
    pub fn text(&self, pause: f64) -> String {
        let mut out = String::new();
        let mut last_end = None::<f64>;
        for s in &self.segments {
            let t = s.text.trim();
            if t.is_empty() {
                continue;
            }
            if let Some(e) = last_end {
                if s.start - e > pause { out.push_str("\n\n") } else { out.push(' ') }
            }
            out.push_str(t);
            last_end = Some(s.end);
        }
        out.push('\n');
        out
    }
}

/// Default CPU threads for whisper: physical cores are what matter; assume SMT.
pub fn default_threads() -> usize {
    let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (if n >= 8 { n / 2 } else { n }).clamp(1, 8)
}

/// Longest chunk handed to whisper-cli at once (seconds). Longer files are cut at pauses.
const CHUNK: f64 = 15.0 * 60.0;

/// Transcribe any audio or video file. Progress is mapped to `range` of the job.
pub fn transcribe(path: &Path, opts: &Options, cx: &RunCx, range: (f32, f32)) -> anyhow::Result<Transcript> {
    let cli = whisper_cli()?;
    let probe = crate::probe(path)?;
    anyhow::ensure!(probe.has_audio, tr("media.err.no_audio"));
    let (lo, hi) = range;
    let span = hi - lo;
    let dur = probe.duration;
    let tmp = cx.tmp_dir().join("whisper");
    std::fs::create_dir_all(&tmp)?;

    // 1. Decode to 16 kHz mono PCM (whisper's native input).
    cx.message(tr("media.msg.preparing_audio"));
    let wav = tmp.join("audio.wav");
    Ffmpeg::new()
        .input(path)
        .args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"])
        .arg(&wav)
        .duration(dur)
        .range(lo, lo + span * 0.04)
        .quiet()
        .run(cx)?;

    // 2. Chunk boundaries at pauses near every CHUNK seconds.
    let mut bounds = vec![0.0];
    if dur > CHUNK * 1.2 {
        let sil = crate::analyze::silences(&wav, -35.0, 0.4, dur, cx, (lo + span * 0.04, lo + span * 0.06))?;
        let mut t = CHUNK;
        while t < dur - CHUNK * 0.2 {
            // The middle of the pause closest to t (within ±60 s), else t itself.
            let best = sil
                .iter()
                .map(|(a, b)| (a + b) / 2.0)
                .filter(|m| (m - t).abs() < 60.0)
                .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
                .unwrap_or(t);
            bounds.push(best);
            t = best + CHUNK;
        }
    }
    bounds.push(dur.max(0.001));

    let base = lo + span * 0.06;
    let t0 = std::time::Instant::now();
    let work = hi - base;
    let mut lang = opts.language.clone();
    let mut out = Transcript { language: String::new(), duration: dur, segments: Vec::new() };
    let n = bounds.len() - 1;
    for i in 0..n {
        cx.check_cancel()?;
        let (a, b) = (bounds[i], bounds[i + 1]);
        let (clo, chi) = (base + work * (a / dur.max(0.001)) as f32, base + work * (b / dur.max(0.001)) as f32);
        let input = if n == 1 {
            wav.clone()
        } else {
            let c = tmp.join(format!("chunk{i}.wav"));
            Ffmpeg::new()
                .args(["-ss", &format!("{a:.3}"), "-t", &format!("{:.3}", b - a)])
                .input(&wav)
                .args(["-c:a", "pcm_s16le"])
                .arg(&c)
                .quiet()
                .run(cx)?;
            c
        };
        if n > 1 {
            cx.message(castor_core::tr_args(
                "media.msg.transcribing_part",
                &[("n", &(i + 1).to_string()), ("total", &n.to_string())],
            ));
        } else {
            cx.message(tr("media.msg.transcribing"));
        }
        let stem = tmp.join(format!("out{i}"));
        let (segs, detected) = run_cli(&cli, &input, &stem, opts, &lang, cx, (clo, chi), (t0, base, hi))?;
        if lang == "auto" && !detected.is_empty() {
            // Keep the language detected on the first chunk for the rest: consistent output.
            lang = detected.clone();
        }
        if out.language.is_empty() {
            out.language = detected;
        }
        for mut s in segs {
            s.start += a;
            s.end += a;
            for w in &mut s.words {
                w.start += a;
                w.end += a;
            }
            out.segments.push(s);
        }
        if n > 1 {
            let _ = std::fs::remove_file(&input);
        }
    }
    let _ = std::fs::remove_file(&wav);
    if out.language.is_empty() {
        out.language = opts.language.clone();
    }
    cx.progress(hi);
    Ok(out)
}

fn run_cli(
    cli: &Path,
    wav: &Path,
    stem: &Path,
    opts: &Options,
    lang: &str,
    cx: &RunCx,
    range: (f32, f32),
    eta: (std::time::Instant, f32, f32),
) -> anyhow::Result<(Vec<Segment>, String)> {
    let mut cmd = castor_core::process::command(cli);
    let threads = if opts.threads == 0 { default_threads() } else { opts.threads };
    cmd.arg("-m").arg(&opts.model);
    cmd.arg("-f").arg(wav);
    cmd.args(["-l", lang, "-t", &threads.to_string(), "-ojf", "-pp", "-np"]);
    cmd.arg("-of").arg(stem);
    if opts.translate {
        cmd.arg("-tr");
    }
    if opts.beam {
        cmd.args(["-bs", "5"]);
    } else {
        cmd.args(["-bs", "1", "-bo", "1"]);
    }
    if let Some(vm) = &opts.vad {
        cmd.arg("--vad").arg("-vm").arg(vm);
    }
    if let Some(p) = opts.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
        cmd.arg("--prompt").arg(p);
    }
    let (lo, hi) = range;
    let (started, base, top) = eta;
    let res = castor_core::process::run(
        cmd,
        &cx.cancel_token(),
        |_| {},
        |line| {
            // "whisper_print_progress_callback: progress =  42%"
            if let Some(i) = line.find("progress =") {
                let pct: String = line[i + 10..].chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(p) = pct.parse::<f32>() {
                    let f = (p / 100.0).clamp(0.0, 1.0);
                    let overall = lo + (hi - lo) * f;
                    cx.progress(overall);
                    // whisper reports every ~5 %: an ETA line each time is not too chatty.
                    let el = started.elapsed().as_secs_f64();
                    let done = ((overall - base) / (top - base).max(1e-6)) as f64;
                    if done > 0.03 && done < 1.0 && el > 5.0 {
                        let left = el / done * (1.0 - done);
                        cx.message(castor_core::tr_args(
                            "media.msg.progress",
                            &[("pct", &format!("{:.0}", overall * 100.0)), ("eta", &castor_core::units::duration(left))],
                        ));
                    }
                }
            }
        },
    );
    if let Err(e) = res {
        if e.is::<castor_core::job::Cancelled>() {
            return Err(e);
        }
        return Err(anyhow::Error::msg(format!("{e}")).context(tr("media.err.whisper")));
    }
    let json_path = stem.with_extension("json");
    let raw = std::fs::read(&json_path).map_err(|_| anyhow::anyhow!(tr("media.err.whisper")))?;
    let parsed = parse_json(&String::from_utf8_lossy(&raw), opts.vad.is_some())?;
    let _ = std::fs::remove_file(&json_path);
    Ok(parsed)
}

/// Parse `whisper-cli -ojf` output into segments with words.
/// With VAD, whisper.cpp maps segment start times back to the original timeline but not token
/// times: words are then shifted onto their segment's start.
pub fn parse_json(s: &str, vad: bool) -> anyhow::Result<(Vec<Segment>, String)> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    let lang = v["result"]["language"].as_str().unwrap_or("").to_string();
    let mut segs = Vec::new();
    let empty = Vec::new();
    for seg in v["transcription"].as_array().unwrap_or(&empty) {
        let start = seg["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0;
        let end = seg["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0;
        let text = seg["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() {
            continue;
        }
        let mut words: Vec<Word> = Vec::new();
        let mut probs: Vec<Vec<f32>> = Vec::new();
        for t in seg["tokens"].as_array().unwrap_or(&empty) {
            let tt = t["text"].as_str().unwrap_or("");
            if tt.is_empty() || (tt.starts_with("[_") && tt.ends_with(']')) || (tt.starts_with("<|") && tt.ends_with("|>")) {
                continue;
            }
            let t0 = t["offsets"]["from"].as_f64().unwrap_or(0.0) / 1000.0;
            let t1 = t["offsets"]["to"].as_f64().unwrap_or(0.0) / 1000.0;
            let p = t["p"].as_f64().unwrap_or(1.0) as f32;
            let new_word = tt.starts_with(' ') || words.is_empty();
            if new_word {
                words.push(Word { start: t0, end: t1.max(t0), text: tt.trim_start().to_string(), p });
                probs.push(vec![p]);
            } else {
                let w = words.last_mut().unwrap();
                w.text.push_str(tt);
                w.end = w.end.max(t1);
                probs.last_mut().unwrap().push(p);
            }
        }
        for (w, ps) in words.iter_mut().zip(&probs) {
            w.p = ps.iter().sum::<f32>() / ps.len().max(1) as f32;
        }
        words.retain(|w| !w.text.trim().is_empty());
        let mut end = end;
        if vad && !words.is_empty() {
            // Token times are on the VAD-compressed timeline. Inside one speech chunk that
            // timeline is the original one shifted by a constant, so shift (do not stretch):
            // the segment end reported by whisper.cpp can include the silence that follows.
            let (a, b) = (words.first().unwrap().start, words.last().unwrap().end);
            for w in &mut words {
                w.start = start + (w.start - a);
                w.end = start + (w.end - a);
            }
            end = end.min(start + (b - a) + 0.2).max(start);
        }
        // Keep words inside their segment and in order.
        let mut prev = start;
        for w in &mut words {
            w.start = w.start.clamp(prev, end.max(prev));
            w.end = w.end.clamp(w.start, end.max(w.start));
            prev = w.start;
        }
        segs.push(Segment { start, end, text, words });
    }
    Ok((segs, lang))
}

/// Languages whisper knows: (code, endonym). `OptionSpec::language` list.
pub const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("es", "Español"),
    ("it", "Italiano"),
    ("pt", "Português"),
    ("nl", "Nederlands"),
    ("ja", "日本語"),
    ("zh", "中文"),
    ("ko", "한국어"),
    ("ru", "Русский"),
    ("uk", "Українська"),
    ("pl", "Polski"),
    ("tr", "Türkçe"),
    ("ar", "العربية"),
    ("hi", "हिन्दी"),
    ("ca", "Català"),
    ("sv", "Svenska"),
    ("da", "Dansk"),
    ("no", "Norsk"),
    ("nn", "Nynorsk"),
    ("fi", "Suomi"),
    ("cs", "Čeština"),
    ("sk", "Slovenčina"),
    ("sl", "Slovenščina"),
    ("hr", "Hrvatski"),
    ("sr", "Српски"),
    ("bs", "Bosanski"),
    ("bg", "Български"),
    ("mk", "Македонски"),
    ("ro", "Română"),
    ("hu", "Magyar"),
    ("el", "Ελληνικά"),
    ("he", "עברית"),
    ("fa", "فارسی"),
    ("ur", "اردو"),
    ("id", "Bahasa Indonesia"),
    ("ms", "Bahasa Melayu"),
    ("vi", "Tiếng Việt"),
    ("th", "ไทย"),
    ("tl", "Tagalog"),
    ("jw", "Basa Jawa"),
    ("su", "Basa Sunda"),
    ("ta", "தமிழ்"),
    ("te", "తెలుగు"),
    ("kn", "ಕನ್ನಡ"),
    ("ml", "മലയാളം"),
    ("mr", "मराठी"),
    ("bn", "বাংলা"),
    ("gu", "ગુજરાતી"),
    ("pa", "ਪੰਜਾਬੀ"),
    ("ne", "नेपाली"),
    ("si", "සිංහල"),
    ("sa", "संस्कृतम्"),
    ("as", "অসমীয়া"),
    ("my", "မြန်မာ"),
    ("km", "ខ្មែរ"),
    ("lo", "ລາວ"),
    ("bo", "བོད་སྐད"),
    ("mn", "Монгол"),
    ("kk", "Қазақ"),
    ("uz", "Oʻzbek"),
    ("tg", "Тоҷикӣ"),
    ("tk", "Türkmen"),
    ("az", "Azərbaycan"),
    ("ka", "ქართული"),
    ("hy", "Հայերեն"),
    ("be", "Беларуская"),
    ("lt", "Lietuvių"),
    ("lv", "Latviešu"),
    ("et", "Eesti"),
    ("is", "Íslenska"),
    ("fo", "Føroyskt"),
    ("cy", "Cymraeg"),
    ("br", "Brezhoneg"),
    ("eu", "Euskara"),
    ("gl", "Galego"),
    ("oc", "Occitan"),
    ("lb", "Lëtzebuergesch"),
    ("mt", "Malti"),
    ("sq", "Shqip"),
    ("la", "Latina"),
    ("yi", "ייִדיש"),
    ("af", "Afrikaans"),
    ("sw", "Kiswahili"),
    ("am", "አማርኛ"),
    ("so", "Soomaali"),
    ("yo", "Yorùbá"),
    ("ha", "Hausa"),
    ("sn", "chiShona"),
    ("ln", "Lingála"),
    ("mg", "Malagasy"),
    ("ht", "Kreyòl ayisyen"),
    ("mi", "Te Reo Māori"),
    ("haw", "ʻŌlelo Hawaiʻi"),
    ("ps", "پښتو"),
    ("sd", "سنڌي"),
    ("tt", "Татар"),
    ("ba", "Башҡорт"),
    ("yue", "粵語"),
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys() {
        assert_eq!(model_key("small", "q8", false), "small-q8");
        assert_eq!(model_key("base", "full", true), "base.en");
        assert_eq!(model_key("turbo", "q5", true), "turbo-q5");
        for m in MODELS {
            for v in VARIANTS {
                assert!(model_asset(m, v, false).is_some(), "{m} {v}");
                assert!(model_asset(m, v, true).is_some(), "{m} {v} en");
            }
        }
    }
    #[test]
    fn vad_shifts_words_and_trims_segment_end() {
        // Segment mapped to 10.0–30.0 s (end includes a removed silence); tokens on the
        // compressed timeline at 2.0–3.0 s.
        let j = r#"{"result":{"language":"en"},"transcription":[{"offsets":{"from":10000,"to":30000},"text":" Mark Twain.",
          "tokens":[{"text":" Mark","offsets":{"from":2000,"to":2400},"p":0.9},{"text":" Twain.","offsets":{"from":2500,"to":3000},"p":0.9}]}]}"#;
        let (s, _) = parse_json(j, true).unwrap();
        assert!((s[0].words[0].start - 10.0).abs() < 1e-9);
        assert!((s[0].words[1].end - 11.0).abs() < 1e-9);
        assert!((s[0].end - 11.2).abs() < 1e-9);
    }
    #[test]
    fn words_from_tokens() {
        let j = r#"{"result":{"language":"fr"},"transcription":[{"offsets":{"from":1000,"to":3000},"text":" Bonjour, l'été.",
          "tokens":[{"text":"[_BEG_]","offsets":{"from":0,"to":0},"p":0.9},
                    {"text":" Bon","offsets":{"from":1000,"to":1300},"p":0.8},
                    {"text":"jour","offsets":{"from":1300,"to":1600},"p":0.6},
                    {"text":",","offsets":{"from":1600,"to":1650},"p":0.9},
                    {"text":" l","offsets":{"from":1700,"to":1800},"p":0.9},
                    {"text":"'été.","offsets":{"from":1800,"to":2500},"p":0.9},
                    {"text":"[_TT_150]","offsets":{"from":3000,"to":3000},"p":0.9}]}]}"#;
        let (s, l) = parse_json(j, false).unwrap();
        assert_eq!(l, "fr");
        let w: Vec<_> = s[0].words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(w, ["Bonjour,", "l'été."]);
        assert!((s[0].words[0].start - 1.0).abs() < 1e-9 && (s[0].words[0].end - 1.65).abs() < 1e-9);
    }
}
