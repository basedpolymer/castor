//! Image tools. Owner: agent B (see docs/specs/B-image.md).
//!
//! Shared machinery: [`codec`] (every format in, the best encoder per format out),
//! [`meta`] (metadata without recompression), [`ops`] (resizing, compositing, text),
//! [`color`] (ICC → sRGB). Each tool lives in its own module.

use std::sync::Arc;

use castor_core::Tool;

pub mod codec;
pub mod color;
pub mod common;
pub mod jpegq;
pub mod meta;
pub mod ops;

mod beautify;
mod colorblind;
mod compare;
mod compress;
mod contact_sheet;
mod convert;
mod crop;
mod duplicates;
mod icon_pack;
mod ml_tools;
mod optimize;
mod palette;
mod redact;
mod resize;
mod rotate;
mod seo;
mod strip;
mod stylize;
mod vectorize;
mod watermark;

/// Every tool of this crate, in display order.
pub fn tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ml_tools::RemoveBackground::new()),
        Arc::new(ml_tools::Upscale::new()),
        Arc::new(convert::Convert::new()),
        Arc::new(vectorize::Vectorize::new()),
        Arc::new(compress::Compress::new()),
        Arc::new(palette::Palette::new()),
        Arc::new(resize::Resize::new()),
        Arc::new(crop::Crop::new()),
        Arc::new(rotate::Rotate::new()),
        Arc::new(strip::Strip::new()),
        Arc::new(seo::Seo::new()),
        Arc::new(redact::Redact::new()),
        Arc::new(watermark::Watermark::new()),
        Arc::new(beautify::Beautify::new()),
        Arc::new(icon_pack::IconPack::new()),
        Arc::new(contact_sheet::ContactSheet::new()),
        Arc::new(compare::Compare::new()),
        Arc::new(stylize::Stylize::new()),
        Arc::new(duplicates::Duplicates::new()),
        Arc::new(optimize::OptimizeGif::new()),
        Arc::new(optimize::OptimizeSvg::new()),
        Arc::new(colorblind::Colorblind::new()),
    ]
}
