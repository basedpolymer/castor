//! Raw bindings to `csrc/castor_mupdf.c`. Every function there runs inside `fz_try`.

use std::os::raw::{c_char, c_int, c_void};

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CastorOpt {
    pub recolor: c_int,
    pub subset_fonts: c_int,
    pub rewrite_images: c_int,
    pub color_threshold: c_int,
    pub color_to: c_int,
    pub lossless_to_jpeg: c_int,
    pub jpeg_quality: c_int,
    pub bitonal_threshold: c_int,
    pub bitonal_to: c_int,
    pub garbage: c_int,
    pub objstms: c_int,
    pub effort: c_int,
    pub clean: c_int,
    pub linear: c_int,
}

#[repr(C)]
pub struct CastorOcr {
    _private: [u8; 0],
}

pub type CharCb = unsafe extern "C" fn(
    arg: *mut c_void,
    unicode: c_int,
    line_bbox: *const c_int,
    word_bbox: *const c_int,
    char_bbox: *const c_int,
    pointsize: c_int,
);
pub type ProgressCb = unsafe extern "C" fn(arg: *mut c_void, percent: c_int) -> c_int;

unsafe extern "C" {
    /// Give the shim mupdf-rs's (locked) context to clone from. Call through [`ensure_base`].
    fn castor_set_base_context(ctx: *mut c_void);
    pub fn castor_optimize(
        input: *const c_char,
        output: *const c_char,
        password: *const c_char,
        o: *const CastorOpt,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;

    pub fn castor_ocr_new(lang: *const c_char, datadir: *const c_char, err: *mut c_char, errlen: c_int) -> *mut CastorOcr;
    pub fn castor_ocr_free(h: *mut CastorOcr);
    pub fn castor_ocr_run(
        h: *mut CastorOcr,
        gray: *const u8,
        w: c_int,
        h_px: c_int,
        stride: c_int,
        dpi: c_int,
        cb: CharCb,
        progress: ProgressCb,
        arg: *mut c_void,
        confs: *mut *mut c_int,
        nconfs: *mut c_int,
        mean_conf: *mut c_int,
        err: *mut c_char,
        errlen: c_int,
    ) -> c_int;
    pub fn castor_ocr_free_confs(confs: *mut c_int);
}

/// Error buffer → String.
pub fn err_string(buf: &[u8]) -> String {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

/// Make the C shim clone its contexts from mupdf-rs's base context (shared locks and store).
/// Idempotent and cheap; call before any shim function.
pub fn ensure_base() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let ctx = mupdf::Context::get();
        // `mupdf::Context` is a single raw `*mut fz_context` field with no Drop (mupdf 0.8.0,
        // pinned with `=`); its pointer is this thread's clone of the locked base context. The
        // shim clones it again immediately and keeps that clone for the life of the process.
        // `transmute` checks the sizes at compile time.
        let raw: *mut c_void = unsafe { std::mem::transmute::<mupdf::Context, *mut c_void>(ctx) };
        unsafe { castor_set_base_context(raw) };
    });
}
