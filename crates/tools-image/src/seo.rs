//! image.seo — search-friendly file names (slug pattern) and title / alt text / keywords /
//! author / copyright / GPS written to XMP (+ IPTC for JPEG) without recompression.
//! Optional "Write with AI": the user's own Anthropic API key (Settings), opt-in per run.

use anyhow::{Context, Result};
use base64::Engine;
use castor_core::options::{Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Requirement, RunCx, Tool, ToolMeta, ToolStatus};

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::common;
use crate::meta::{self, XmpFields};

pub struct Seo {
    meta: ToolMeta,
}

impl Seo {
    pub fn new() -> Self {
        Seo {
            meta: common::meta(
                "image.seo",
                "JPG · PNG · WEBP",
                &["seo", "alt text", "rename", "slug", "keywords", "metadata", "xmp", "iptc", "copyright", "google images"],
                "",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Seo {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::text("pattern", "{title}-{n}"),
            OptionSpec::text("title", ""),
            OptionSpec::text("alt", ""),
            OptionSpec::multiline("description", "").advanced(),
            OptionSpec::text("keywords", ""),
            OptionSpec::text("author", ""),
            OptionSpec::text("copyright", ""),
            OptionSpec::text("gps", "").advanced(),
            OptionSpec::toggle("ai", false),
            OptionSpec::text("ai_language", "").when(Cond::True("ai")),
        ]
    }

    /// One job for the whole batch: names are numbered in order.
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(!inputs.is_empty(), "{}", castor_core::tr("image.seo.empty"));
        Ok(vec![Job::many(inputs.to_vec())])
    }

    fn unavailable(&self) -> Option<String> {
        None
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let key = if v.bool("ai") {
            let k = castor_core::settings::Settings::load().ai_api_key.filter(|k| !k.trim().is_empty());
            Some(k.context(castor_core::tr("image.seo.no_key"))?)
        } else {
            None
        };
        let gps = parse_gps(v.str("gps"));
        if !v.str("gps").trim().is_empty() && gps.is_none() {
            anyhow::bail!("{}", castor_core::tr("image.seo.bad_gps"));
        }
        let mut outs = Vec::new();
        let n = job.inputs.len();
        for (i, f) in job.inputs.iter().enumerate() {
            cx.check_cancel()?;
            cx.message(format!("{} ({}/{n})", f.name(), i + 1));
            let mut fields = XmpFields {
                title: non_empty(v.str("title")),
                description: non_empty(v.str("description")),
                alt: non_empty(v.str("alt")),
                keywords: v.str("keywords").split([',', ';']).map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect(),
                creator: non_empty(v.str("author")),
                rights: non_empty(v.str("copyright")),
                gps,
            };
            if let Some(k) = &key {
                let lang = if v.str("ai_language").trim().is_empty() { castor_core::i18n::current_language() } else { v.str("ai_language").trim().to_string() };
                let ai = ask_ai(f, k, &lang).with_context(|| castor_core::tr("image.seo.ai_failed"))?;
                fields.title = fields.title.or(ai.title);
                fields.alt = fields.alt.or(ai.alt);
                fields.description = fields.description.or(ai.description);
                if fields.keywords.is_empty() {
                    fields.keywords = ai.keywords;
                }
            }
            let fmt = codec::detect(&f.path).context(common::err("err.unknown_format", &f.path))?;
            let src = std::fs::read(&f.path).with_context(|| common::err("err.read", &f.path))?;
            let ext = if fmt == Fmt::Jpeg { "jpg".to_string() } else { f.extension() };
            let name = file_name(v.str("pattern"), &fields, f, i + 1, n);
            let out = cx.output_named(&format!("{name}.{ext}"));
            let (bytes, note) = if fmt.container_editable() {
                let xmp = meta::build_xmp(&fields);
                let mut b = meta::embed(src, fmt, None, None, Some(&xmp))?;
                if fmt == Fmt::Jpeg {
                    let mut rec = Vec::new();
                    if let Some(t) = &fields.title {
                        rec.push((5u8, t.clone()));
                    }
                    if let Some(d) = fields.description.as_ref().or(fields.alt.as_ref()) {
                        rec.push((120, d.clone()));
                    }
                    for k in &fields.keywords {
                        rec.push((25, k.clone()));
                    }
                    if let Some(c) = &fields.creator {
                        rec.push((80, c.clone()));
                    }
                    if let Some(c) = &fields.rights {
                        rec.push((116, c.clone()));
                    }
                    b = meta::set_jpeg_iptc(b, &meta::iptc_records(&rec))?;
                }
                (b, castor_core::tr("image.seo.written"))
            } else {
                (src, castor_core::tr("image.seo.renamed_only"))
            };
            common::write(&out, &bytes)?;
            let alt_note = fields.alt.as_deref().map(|a| format!(" — {a}")).unwrap_or_default();
            outs.push(Output::new(out).with_note(format!("{note}{alt_note}")));
            cx.progress((i + 1) as f32 / n as f32);
        }
        Ok(outs)
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// "48.8584, 2.2945" or "48.8584 2.2945".
pub fn parse_gps(s: &str) -> Option<(f64, f64)> {
    let parts: Vec<f64> = s.split([',', ' ', ';']).filter(|p| !p.trim().is_empty()).filter_map(|p| p.trim().parse().ok()).collect();
    match parts[..] {
        [lat, lon] if lat.abs() <= 90.0 && lon.abs() <= 180.0 => Some((lat, lon)),
        _ => None,
    }
}

/// Lowercase ASCII slug: accents folded, everything else becomes "-".
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        let folded: &str = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' => "a",
            'æ' | 'Æ' => "ae",
            'ç' | 'Ç' => "c",
            'è' | 'é' | 'ê' | 'ë' | 'È' | 'É' | 'Ê' | 'Ë' => "e",
            'ì' | 'í' | 'î' | 'ï' | 'Ì' | 'Í' | 'Î' | 'Ï' => "i",
            'ñ' | 'Ñ' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' => "o",
            'œ' | 'Œ' => "oe",
            'ù' | 'ú' | 'û' | 'ü' | 'Ù' | 'Ú' | 'Û' | 'Ü' => "u",
            'ý' | 'ÿ' | 'Ý' => "y",
            'ß' => "ss",
            _ => "",
        };
        if !folded.is_empty() {
            out.push_str(folded);
        } else if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if c.is_alphanumeric() {
            // Other scripts are kept (valid in URLs once encoded, and better than nothing).
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let s = out.trim_matches('-').to_string();
    let mut s: String = s.chars().take(80).collect();
    while s.ends_with('-') {
        s.pop();
    }
    s
}

fn file_name(pattern: &str, f: &XmpFields, input: &FileInfo, n: usize, total: usize) -> String {
    let title = f.title.as_deref().map(slugify).filter(|s| !s.is_empty()).unwrap_or_else(|| slugify(&input.stem()));
    let width = total.to_string().len().max(1);
    let pattern = if pattern.trim().is_empty() { "{title}-{n}" } else { pattern };
    let date = time::OffsetDateTime::now_utc().date().to_string();
    let raw = pattern
        .replace("{title}", &title)
        .replace("{name}", &slugify(&input.stem()))
        .replace("{n}", &format!("{n:0width$}"))
        .replace("{date}", &date);
    let s = slugify(&raw);
    // A single file does not need its number: "{title}-1" → "{title}".
    let s = if total == 1 && pattern.contains("{n}") { s.trim_end_matches(&format!("-{n:0width$}")).to_string() } else { s };
    if s.is_empty() { "image".into() } else { s }
}

#[derive(Default, serde::Deserialize)]
struct AiFields {
    title: Option<String>,
    alt: Option<String>,
    description: Option<String>,
    #[serde(default)]
    keywords: Vec<String>,
}

/// Ask Claude (the user's key, small JPEG of the image) for title / alt / description / keywords.
fn ask_ai(f: &FileInfo, key: &str, lang: &str) -> Result<AiFields> {
    let d = codec::decode(&f.path, DecodeOpts::default())?;
    let small = d.image.thumbnail(768, 768);
    let jpg = codec::encode(&small, Fmt::Jpeg, &EncodeOpts { quality: 80, ..Default::default() })?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(jpg);
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "alt": {"type": "string"},
            "description": {"type": "string"},
            "keywords": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["title", "alt", "description", "keywords"],
        "additionalProperties": false
    });
    let prompt = format!(
        "Write SEO metadata for this image, in the language with code '{lang}'. title: 3-8 words, no trailing period. alt: one sentence (max 125 characters) describing what is visible, for screen readers. description: one or two sentences. keywords: 5-10 short lowercase keywords. Describe only what is visible; do not guess names of people."
    );
    let body = serde_json::json!({
        "model": "claude-haiku-5-5",
        "max_tokens": 2000,
        "output_config": {"effort": "low", "format": {"type": "json_schema", "schema": schema}},
        "messages": [{
            "role": "user",
            "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/jpeg", "data": b64}},
                {"type": "text", "text": prompt}
            ]
        }]
    });
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(concat!("Castor/", env!("CARGO_PKG_VERSION")))
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(90)))
        .build()
        .into();
    let mut resp = agent
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", key.trim())
        .header("anthropic-version", "2026-10-21")
        .header("content-type", "application/json")
        .send(serde_json::to_vec(&body)?)?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().read_to_string()?;
    let v: serde_json::Value = serde_json::from_str(&text).context("invalid API response")?;
    if status != 200 {
        let msg = v["error"]["message"].as_str().unwrap_or("request failed");
        anyhow::bail!("HTTP {status}: {msg}");
    }
    if v["stop_reason"] == "refusal" {
        anyhow::bail!("{}", castor_core::tr("image.seo.ai_refused"));
    }
    let out: String = v["content"]
        .as_array()
        .map(|blocks| blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect())
        .unwrap_or_default();
    let mut ai: AiFields = serde_json::from_str(out.trim()).context("unexpected answer format")?;
    let clean = |s: Option<String>| s.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    ai.title = clean(ai.title);
    ai.alt = clean(ai.alt);
    ai.description = clean(ai.description);
    ai.keywords.retain(|k| !k.trim().is_empty());
    let _ = Requirement::Network;
    Ok(ai)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugs() {
        assert_eq!(slugify("Crème brûlée — été 2026!"), "creme-brulee-ete-2026");
        assert_eq!(slugify("  --  "), "");
    }
    #[test]
    fn gps() {
        assert_eq!(parse_gps("48.8584, 2.2945"), Some((48.8584, 2.2945)));
        assert_eq!(parse_gps("200, 1"), None);
    }
    #[test]
    fn names() {
        let f = FileInfo { path: "IMG_0001.JPG".into(), kind: castor_core::FileKind::Image, size: 1, is_dir: false };
        let x = XmpFields { title: Some("Red Bike".into()), ..Default::default() };
        assert_eq!(file_name("{title}-{n}", &x, &f, 3, 12), "red-bike-03");
        assert_eq!(file_name("{title}-{n}", &x, &f, 1, 1), "red-bike");
        assert_eq!(file_name("{name}", &XmpFields::default(), &f, 1, 1), "img-0001");
    }
}
