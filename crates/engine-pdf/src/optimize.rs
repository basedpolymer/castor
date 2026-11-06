//! Whole-file rewrites done in C: image downsampling/recompression, font subsetting,
//! recolouring (gray, CMYK→RGB), garbage collection, object streams.

use std::ffi::CString;
use std::path::Path;

use anyhow::Result;

use crate::ffi;

/// Options for [`optimize`]. Defaults: lossless structural clean-up only.
#[derive(Clone, Debug)]
pub struct Optimize {
    pub recolor: Recolor,
    pub subset_fonts: bool,
    /// Downsample colour and gray images above `image_threshold_dpi` to `image_dpi` (0 = never).
    pub image_dpi: u32,
    pub image_threshold_dpi: u32,
    /// Re-encode images (JPEG for photos); off = images untouched unless downsampled/recoloured.
    pub rewrite_images: bool,
    /// Also turn losslessly compressed colour/gray images into JPEG.
    pub lossless_to_jpeg: bool,
    pub jpeg_quality: u8,
    /// Bitonal (1-bit) images: downsample above this dpi (0 = never), CCITT G4 recompression.
    pub bitonal_dpi: u32,
    pub garbage: i32,
    pub object_streams: bool,
    /// Deflate effort 0 (default) … 100 (max).
    pub effort: i32,
    pub clean: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recolor {
    Keep,
    Gray,
    Rgb,
}

impl Default for Optimize {
    fn default() -> Self {
        Optimize {
            recolor: Recolor::Keep,
            subset_fonts: false,
            image_dpi: 0,
            image_threshold_dpi: 0,
            rewrite_images: false,
            lossless_to_jpeg: false,
            jpeg_quality: 80,
            bitonal_dpi: 0,
            garbage: 3,
            object_streams: true,
            effort: 0,
            clean: false,
        }
    }
}

/// Rewrite `input` into `output` (written atomically). `password` opens encrypted inputs; the
/// output is saved unencrypted.
pub fn optimize(input: &Path, output: &Path, password: &str, o: &Optimize) -> Result<()> {
    let opt = ffi::CastorOpt {
        recolor: match o.recolor {
            Recolor::Keep => 0,
            Recolor::Gray => 1,
            Recolor::Rgb => 3,
        },
        subset_fonts: o.subset_fonts as i32,
        rewrite_images: (o.rewrite_images || o.image_dpi > 0 || o.bitonal_dpi > 0) as i32,
        color_threshold: if o.image_dpi > 0 { o.image_threshold_dpi.max(o.image_dpi) as i32 } else { 0 },
        color_to: o.image_dpi as i32,
        lossless_to_jpeg: o.lossless_to_jpeg as i32,
        jpeg_quality: o.jpeg_quality.clamp(1, 100) as i32,
        bitonal_threshold: if o.bitonal_dpi > 0 { (o.bitonal_dpi as f32 * 1.3) as i32 } else { 0 },
        bitonal_to: o.bitonal_dpi as i32,
        garbage: o.garbage.clamp(0, 4),
        objstms: o.object_streams as i32,
        effort: o.effort.clamp(0, 100),
        clean: o.clean as i32,
        linear: 0,
    };
    let cin = CString::new(crate::path_str(input)?)?;
    let tmp = crate::tmp_sibling(output);
    let cout = CString::new(crate::path_str(&tmp)?)?;
    let cpw = CString::new(password)?;
    let mut err = vec![0u8; 1024];
    let rc = unsafe {
        ffi::castor_optimize(cin.as_ptr(), cout.as_ptr(), cpw.as_ptr(), &opt, err.as_mut_ptr().cast(), err.len() as i32)
    };
    if rc != 0 {
        let _ = std::fs::remove_file(&tmp);
        let msg = ffi::err_string(&err);
        if msg == "password required" {
            anyhow::bail!("{}", castor_core::tr("pdf.err.password_wrong"));
        }
        anyhow::bail!("{}: {msg}", castor_core::tr("pdf.err.save"));
    }
    crate::persist(&tmp, output)
}
