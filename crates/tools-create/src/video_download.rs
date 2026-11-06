//! `create.video-download`: yt-dlp (fetched once, verified, kept up to date by engine-media)
//! downloads videos or audio from the links you paste. Network access, by definition.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{InputSpec, Job, Output, Requirement, RunCx, Tool, ToolMeta, tr, tr_args};
use engine_media::ytdlp;

use crate::common;

pub struct VideoDownloadTool {
    meta: ToolMeta,
}

impl VideoDownloadTool {
    pub fn new() -> Self {
        VideoDownloadTool {
            meta: common::serial(common::with_requires(
                common::meta(
                    "create.video-download",
                    InputSpec::none(),
                    "MP4 · MP3 · M4A",
                    &["download", "video", "youtube", "vimeo", "yt-dlp", "save", "audio", "podcast", "playlist"],
                    "",
                ),
                &[Requirement::YtDlp, Requirement::Ffmpeg, Requirement::Network],
            )),
        }
    }
}

impl Tool for VideoDownloadTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::multiline("urls", ""),
            OptionSpec::choice(
                "quality",
                vec![
                    Choice::key("best"),
                    Choice::lit("2160", "4K (2160p)"),
                    Choice::lit("1080", "1080p"),
                    Choice::lit("720", "720p"),
                    Choice::lit("480", "480p"),
                    Choice::key("audio"),
                ],
                "1080",
            ),
            OptionSpec::choice("audio_format", vec![Choice::lit("mp3", "MP3"), Choice::lit("m4a", "M4A"), Choice::lit("opus", "Opus")], "mp3")
                .when(Cond::Eq("quality", "audio")),
            OptionSpec::toggle("playlist", false),
            OptionSpec::toggle("subtitles", false),
            OptionSpec::text("name_pattern", "{title}").advanced(),
            OptionSpec::toggle("update", false).advanced(),
        ]
    }

    fn downloads(&self, _v: &Values) -> Vec<castor_core::download::Asset> {
        let a = ytdlp::asset();
        if a.is_present() { Vec::new() } else { vec![a] }
    }

    fn unavailable(&self) -> Option<String> {
        if engine_media::ffmpeg().is_err() { Some(tr("create.video-download.err.ffmpeg")) } else { None }
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let urls: Vec<String> = v
            .str("urls")
            .split(|c: char| c.is_whitespace() || c == ',')
            .map(str::trim)
            .filter(|u| !u.is_empty())
            .map(String::from)
            .collect();
        anyhow::ensure!(!urls.is_empty(), "{}", tr("create.video-download.err.no_url"));
        for u in &urls {
            anyhow::ensure!(
                u.starts_with("http://") || u.starts_with("https://"),
                "{}",
                tr_args("create.video-download.err.bad_url", &[("url", u)])
            );
        }
        if v.bool("update") {
            cx.message(tr("create.video-download.msg.updating"));
            let _ = ytdlp::update(cx);
        }
        let quality = v.str("quality");
        let audio = quality == "audio";
        let opts = ytdlp::DownloadOptions {
            audio_only: audio,
            audio_format: v.str("audio_format").to_string(),
            max_height: quality.parse().unwrap_or(0),
            playlist: v.bool("playlist"),
            subtitles: v.bool("subtitles") && !audio,
            // Downloads land in a scratch folder first, then get their final names.
            out_dir: cx.tmp_dir().join("dl"),
        };
        let mut outs = Vec::new();
        for (i, u) in urls.iter().enumerate() {
            cx.check_cancel()?;
            cx.message(tr_args("create.video-download.msg.item", &[("n", &(i + 1).to_string()), ("total", &urls.len().to_string()), ("url", u)]));
            let files = ytdlp::download(u, &opts, cx)?;
            anyhow::ensure!(!files.is_empty(), "{}", tr_args("create.video-download.err.nothing", &[("url", u)]));
            for f in files {
                let ext = f.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
                let stem = f.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let name = final_name(v.str("name_pattern"), &stem, i + 1);
                let dest = cx.output_named(&format!("{name}.{ext}"));
                if std::fs::rename(&f, &dest).is_err() {
                    std::fs::copy(&f, &dest)?;
                    let _ = std::fs::remove_file(&f);
                }
                let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
                outs.push(Output::new(dest).with_note(castor_core::units::bytes(size)));
            }
            cx.progress((i + 1) as f32 / urls.len() as f32);
        }
        Ok(outs)
    }
}

/// yt-dlp names files "Title [id]". Pattern tokens: {title}, {id}, {n}.
pub fn final_name(pattern: &str, ytdlp_stem: &str, n: usize) -> String {
    let (title, id) = match ytdlp_stem.rfind(" [") {
        Some(i) if ytdlp_stem.ends_with(']') => (&ytdlp_stem[..i], &ytdlp_stem[i + 2..ytdlp_stem.len() - 1]),
        _ => (ytdlp_stem, ""),
    };
    let title = title.replace('_', " ");
    let p = if pattern.trim().is_empty() { "{title}" } else { pattern };
    let s = p.replace("{title}", title.trim()).replace("{id}", id).replace("{n}", &format!("{n:02}"));
    castor_core::job::sanitize_file_name(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert_eq!(final_name("{title}", "My_Video [abc123]", 1), "My Video");
        assert_eq!(final_name("{n} - {title} ({id})", "Talk [x9]", 3), "03 - Talk (x9)");
    }
}
