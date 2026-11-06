//! image.strip-metadata — remove EXIF (GPS, camera, dates), XMP, IPTC and comments without
//! recompression for JPEG, PNG and WebP; lossless re-encode for TIFF / BMP / TGA / QOI.

use anyhow::{Context, Result};
use castor_core::options::{OptionSpec, Values};
use castor_core::{Job, Output, RunCx, Tool, ToolMeta, ToolStatus};

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::common;
use crate::meta::{self, Keep};

pub struct Strip {
    meta: ToolMeta,
}

impl Strip {
    pub fn new() -> Self {
        Strip {
            meta: common::meta(
                "image.strip-metadata",
                "JPG · PNG · WEBP",
                &["exif", "gps", "location", "privacy", "metadata", "remove", "clean", "xmp", "iptc", "camera"],
                "clean",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Strip {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::toggle("keep_orientation", true),
            OptionSpec::toggle("keep_icc", true),
            OptionSpec::toggle("keep_copyright", false),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let fmt = codec::detect(&input.path).context(common::err("err.unknown_format", &input.path))?;
        let keep = Keep { icc: v.bool("keep_icc"), orientation: v.bool("keep_orientation"), copyright: v.bool("keep_copyright") };
        let src = std::fs::read(&input.path).with_context(|| common::err("err.read", &input.path))?;
        anyhow::ensure!(!src.is_empty(), "{}", common::err("err.empty", &input.path));
        let (bytes, note) = if fmt.container_editable() {
            let (b, s) = meta::strip(src.clone(), fmt, keep).map_err(|e| anyhow::anyhow!("{}: {e}", common::err("err.corrupt", &input.path)))?;
            (b, note(&s))
        } else if matches!(fmt, Fmt::Tiff | Fmt::Bmp | Fmt::Tga | Fmt::Qoi) {
            // Lossless formats: decode and write the pixels again (nothing else survives).
            let d = common::load(input, DecodeOpts { orient: keep.orientation, ..Default::default() })?;
            let o = EncodeOpts { icc: if keep.icc { d.icc.clone() } else { None }, ..Default::default() };
            (codec::encode(&d.image, fmt, &o)?, castor_core::tr("image.strip-metadata.reencoded"))
        } else {
            anyhow::bail!("{}", common::err("strip-metadata.unsupported", &input.path));
        };
        cx.check_cancel()?;
        let out = cx.output_path(Some(input), &input.extension());
        common::write(&out, &bytes)?;
        Ok(vec![Output::new(out).with_note(note)])
    }
}

fn note(s: &meta::Stripped) -> String {
    let mut parts = Vec::new();
    if s.gps {
        parts.push(castor_core::tr("image.strip-metadata.gps"));
    }
    if s.exif {
        parts.push("EXIF".to_string());
    }
    if s.xmp {
        parts.push("XMP".to_string());
    }
    if s.iptc {
        parts.push("IPTC".to_string());
    }
    if s.comments {
        parts.push(castor_core::tr("image.strip-metadata.comments"));
    }
    if s.icc {
        parts.push("ICC".to_string());
    }
    if parts.is_empty() && s.other == 0 {
        return castor_core::tr("image.strip-metadata.nothing");
    }
    if parts.is_empty() {
        parts.push(castor_core::tr("image.strip-metadata.other"));
    }
    castor_core::tr_args("image.strip-metadata.removed", &[("list", &parts.join(", "))])
}
