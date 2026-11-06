//! Colour management: convert embedded ICC profiles (Display P3, Adobe RGB, ProPhoto…) to sRGB
//! so that images look the same everywhere once the profile is dropped.

use image::DynamicImage;

/// True if the ICC profile is (a variant of) sRGB.
pub fn is_srgb(icc: &[u8]) -> bool {
    let ascii = icc.windows(4).any(|w| w == b"sRGB");
    let utf16 = icc.windows(8).any(|w| w == b"\0s\0R\0G\0B" || w == b"s\0R\0G\0B\0");
    ascii || utf16
}

/// Convert pixels from the embedded profile to sRGB. Returns None when nothing needs doing
/// (no profile, already sRGB, unsupported profile).
pub fn to_srgb(img: &DynamicImage, icc: Option<&[u8]>) -> Option<DynamicImage> {
    let icc = icc?;
    if is_srgb(icc) {
        return None;
    }
    let src = moxcms::ColorProfile::new_from_slice(icc).ok()?;
    let dst = moxcms::ColorProfile::new_srgb();
    let opts = moxcms::TransformOptions::default();
    let alpha = img.color().has_alpha();
    if matches!(img, DynamicImage::ImageLuma8(_) | DynamicImage::ImageLumaA8(_) | DynamicImage::ImageLuma16(_)) {
        return None; // grey profiles: leave as is
    }
    let (layout, data) =
        if alpha { (moxcms::Layout::Rgba, img.to_rgba8().into_raw()) } else { (moxcms::Layout::Rgb, img.to_rgb8().into_raw()) };
    let t = src.create_transform_8bit(layout, &dst, layout, opts).ok()?;
    let mut out = vec![0u8; data.len()];
    t.transform(&data, &mut out).ok()?;
    let (w, h) = (img.width(), img.height());
    Some(if alpha {
        DynamicImage::ImageRgba8(image::RgbaImage::from_raw(w, h, out)?)
    } else {
        DynamicImage::ImageRgb8(image::RgbImage::from_raw(w, h, out)?)
    })
}
