//! Thumbnails for the file queue: rendered off the UI thread by `castor_tools::thumbnail`,
//! cached in memory, requested lazily by the rows that are on screen.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use castor_core::FileInfo;
use gpui_kit::*;

pub const THUMB_PX: u32 = 72;

const MAX_IN_FLIGHT: usize = 4;

#[derive(Clone)]
pub enum Thumb {
    Loading,
    Ready(Arc<Image>),
    None,
}

#[derive(Default)]
pub struct Thumbs {
    map: HashMap<PathBuf, Thumb>,
    queue: VecDeque<FileInfo>,
    in_flight: usize,
}

impl Global for Thumbs {}

/// The cached thumbnail of a file, starting a render if needed. Returns None while loading or
/// when the kind has no thumbnail.
pub fn get(f: &FileInfo, cx: &mut App) -> Option<Arc<Image>> {
    let t = cx.global_mut::<Thumbs>();
    match t.map.get(&f.path) {
        Some(Thumb::Ready(i)) => return Some(i.clone()),
        Some(_) => return None,
        None => {}
    }
    if !has_thumbnail(f.kind) {
        t.map.insert(f.path.clone(), Thumb::None);
        return None;
    }
    t.map.insert(f.path.clone(), Thumb::Loading);
    t.queue.push_back(f.clone());
    pump(cx);
    None
}

/// Kinds `castor_tools::thumbnail` can draw: every image format, PDF first pages, video frames,
/// audio covers or waveforms.
pub fn has_thumbnail(k: castor_core::FileKind) -> bool {
    use castor_core::FileKind as K;
    k.is_image_like() || matches!(k, K::Pdf | K::Video | K::Audio)
}

fn pump(cx: &mut App) {
    loop {
        let t = cx.global_mut::<Thumbs>();
        if t.in_flight >= MAX_IN_FLIGHT {
            return;
        }
        let Some(f) = t.queue.pop_front() else { return };
        t.in_flight += 1;
        let task = cx.background_executor().spawn(async move {
            let png = thumbnail(&f, THUMB_PX);
            (f.path, png)
        });
        cx.spawn(async move |cx| {
            let (path, png) = task.await;
            let _ = cx.update(|cx| {
                let t = cx.global_mut::<Thumbs>();
                t.in_flight -= 1;
                t.map.insert(
                    path,
                    match png {
                        Some(b) => Thumb::Ready(Arc::new(Image::from_bytes(ImageFormat::Png, b))),
                        None => Thumb::None,
                    },
                );
                pump(cx);
                cx.refresh_windows();
            });
        })
        .detach();
    }
}

/// Forget a file (it changed on disk, e.g. after an in-place run).
#[allow(dead_code)]
pub fn invalidate(p: &std::path::Path, cx: &mut App) {
    cx.global_mut::<Thumbs>().map.remove(p);
}

#[cfg(feature = "tools")]
fn thumbnail(f: &FileInfo, side: u32) -> Option<Vec<u8>> {
    castor_tools::thumbnail(f, side)
}

/// Images only, when built without the tool registry.
#[cfg(not(feature = "tools"))]
fn thumbnail(f: &FileInfo, side: u32) -> Option<Vec<u8>> {
    if f.kind != castor_core::FileKind::Image {
        return None;
    }
    let img = image::ImageReader::open(&f.path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.thumbnail(side, side).write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
