//! Page tools: extract pages, organize, rotate, crop, attach to each.

use castor_core::options::{parse_page_range, Choice, Cond, OptionSpec, Values};
use castor_core::{Family, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::mupdf::Rect;
use engine_pdf::pages;

use crate::common::*;

fn clone_outline(v: &[mupdf::Outline]) -> Vec<mupdf::Outline> {
    v.iter().map(|o| mupdf::Outline { title: o.title.clone(), uri: o.uri.clone(), dest: o.dest, down: clone_outline(&o.down) }).collect()
}

/// New document with `order` pages of `src` (outline remapped to the first copy of each page).
fn rebuild(src: &mut PdfDocument, order: &[usize]) -> Result<PdfDocument> {
    let ol = clone_outline(&src.outlines().unwrap_or_default());
    let mut dst = PdfDocument::new();
    pages::copy_pages(&mut dst, src, order, None)?;
    let ol = pages::remap_outline(ol, &|p| order.iter().position(|q| *q == p));
    if !ol.is_empty() {
        dst.set_outlines(&ol)?;
    }
    // Keep document metadata.
    for k in ["Title", "Author", "Subject", "Keywords", "Creator", "CreationDate"] {
        let val = engine_pdf::get_info(src, k);
        if !val.is_empty() {
            engine_pdf::set_info(&mut dst, k, &val)?;
        }
    }
    engine_pdf::stamp_producer(&mut dst)?;
    Ok(dst)
}

// ------------------------------------------------------------------------------------------

pub struct ExtractPages {
    meta: ToolMeta,
}

impl ExtractPages {
    pub fn new() -> Self {
        ExtractPages { meta: meta("pdf.extract-pages", Family::Pdf, PDF_IN, "PDF", &["select", "pick", "subset", "pages", "copy"], "pages") }
    }
}

impl Tool for ExtractPages {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::pages("pages").common(),
            OptionSpec::choice("output", vec![Choice::key("one"), Choice::key("each_range")], "one"),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        anyhow::ensure!(!v.str("pages").trim().is_empty(), "{}", tr("pdf.extract-pages.err.empty"));
        let mut src = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = src.page_count()?.max(0) as usize;
        let mut groups: Vec<Vec<usize>> = Vec::new();
        if v.str("output") == "each_range" {
            for chunk in v.str("pages").split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
                let p = parse_page_range(chunk, n).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.pages")))?;
                if !p.is_empty() {
                    groups.push(p);
                }
            }
        } else {
            groups.push(pages(v, "pages", n)?);
        }
        anyhow::ensure!(!groups.is_empty(), "{}", tr("pdf.err.no_pages"));
        let mut outs = Vec::new();
        let mut cleanup = Cleanup::new();
        for (k, g) in groups.iter().enumerate() {
            cx.check_cancel()?;
            let dst = rebuild(&mut src, g)?;
            let out = if groups.len() == 1 {
                out_for(cx, input, "pdf")
            } else {
                let (a, b) = (g[0] + 1, g[g.len() - 1] + 1);
                cx.output_named(&format!("{}-p{}.pdf", input.stem(), if a == b { a.to_string() } else { format!("{a}-{b}") }))
            };
            cleanup.add(&out);
            engine_pdf::save_pdf(&dst, &out, &engine_pdf::Save::default())?;
            outs.push(Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &g.len().to_string())])));
            prog(cx, k + 1, groups.len(), 0.0, 1.0);
        }
        cleanup.keep();
        Ok(outs)
    }
}

// ------------------------------------------------------------------------------------------

pub struct Organize {
    meta: ToolMeta,
}

impl Organize {
    pub fn new() -> Self {
        Organize {
            meta: meta("pdf.organize", Family::Pdf, PDF_IN, "PDF", &["reorder", "arrange", "delete", "remove", "move", "pages", "sort"], "organized"),
        }
    }
}

impl Tool for Organize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::text("order", "").placeholder("pdf.organize.opt.order.placeholder"),
            OptionSpec::toggle("reverse", false),
            OptionSpec::pages("delete"),
            OptionSpec::pages("rotate_pages"),
            OptionSpec::choice("angle", vec![Choice::key("90"), Choice::key("180"), Choice::key("270")], "90")
                .when(Cond::Ne("rotate_pages", "")),
            OptionSpec::toggle("blank_after", false).advanced(),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut src = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = src.page_count()?.max(0) as usize;
        let mut order: Vec<usize> = if v.str("order").trim().is_empty() {
            (0..n).collect()
        } else {
            parse_page_range(v.str("order"), n).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.pages")))?
        };
        if v.bool("reverse") {
            order.reverse();
        }
        if !v.str("delete").trim().is_empty() {
            let del = parse_page_range(v.str("delete"), n).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.pages")))?;
            order.retain(|p| !del.contains(p));
        }
        anyhow::ensure!(!order.is_empty(), "{}", tr("pdf.organize.err.all_deleted"));
        // Rotation applies to source pages (by their original number).
        if !v.str("rotate_pages").trim().is_empty() {
            let rot = parse_page_range(v.str("rotate_pages"), n).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.pages")))?;
            let angle: i32 = v.str("angle").parse().unwrap_or(90);
            for p in rot {
                let mut page = src.load_pdf_page(p as i32)?;
                let r = page.rotation()?;
                page.set_rotation((r + angle).rem_euclid(360))?;
            }
        }
        cx.progress(0.3);
        let mut dst = rebuild(&mut src, &order)?;
        if v.bool("blank_after") {
            // A blank page after each page (for handwritten notes), same size.
            let count = dst.page_count()?;
            for i in (0..count).rev() {
                let b = dst.load_page(i)?.bounds()?;
                dst.new_page_at(i + 1, mupdf::Size { width: b.width(), height: b.height() })?;
            }
        }
        cx.progress(0.8);
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&dst, &out, &engine_pdf::Save::default())?;
        let total = dst.page_count()?.to_string();
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &total)]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Rotate {
    meta: ToolMeta,
}

impl Rotate {
    pub fn new() -> Self {
        Rotate { meta: meta("pdf.rotate", Family::Pdf, PDF_IN, "PDF", &["turn", "orientation", "landscape", "portrait", "flip"], "rotated") }
    }
}

impl Tool for Rotate {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("angle", vec![Choice::key("90"), Choice::key("180"), Choice::key("270")], "90"),
            OptionSpec::pages("pages").common(),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let angle: i32 = v.str("angle").parse().unwrap_or(90);
        for (k, p) in sel.iter().enumerate() {
            cx.check_cancel()?;
            let mut page = doc.load_pdf_page(*p as i32)?;
            let r = page.rotation()?;
            page.set_rotation((r + angle).rem_euclid(360))?;
            prog(cx, k + 1, sel.len(), 0.0, 0.8);
        }
        engine_pdf::stamp_producer(&mut doc)?;
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &sel.len().to_string())]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct Crop {
    meta: ToolMeta,
}

impl Crop {
    pub fn new() -> Self {
        Crop { meta: meta("pdf.crop", Family::Pdf, PDF_IN, "PDF", &["trim", "margins", "whitespace", "cut", "border"], "cropped") }
    }
}

/// Set the visible area of a page to `r` (visible page space).
pub fn set_visible_box(page: &mut mupdf::pdf::PdfPage, doc: &PdfDocument, r: Rect) -> Result<()> {
    let ctm = page.ctm()?;
    let inv = ctm.invert().ok_or_else(|| anyhow::anyhow!("page transform is not invertible"))?;
    let u = r.transform(&inv);
    let mb = page.media_box()?;
    let mut obj = page.object();
    // MediaBox as stored (user space).
    let media = match obj.get_dict_inheritable("MediaBox")? {
        Some(a) if a.is_array()? && a.len()? == 4 => {
            let g = |i: i32| a.get_array(i).ok().flatten().and_then(|x| x.as_float().ok()).unwrap_or(0.0);
            Rect { x0: g(0).min(g(2)), y0: g(1).min(g(3)), x1: g(0).max(g(2)), y1: g(1).max(g(3)) }
        }
        _ => Rect { x0: 0.0, y0: 0.0, x1: mb.width(), y1: mb.height() },
    };
    let c = Rect { x0: u.x0.max(media.x0), y0: u.y0.max(media.y0), x1: u.x1.min(media.x1), y1: u.y1.min(media.y1) };
    anyhow::ensure!(c.x1 - c.x0 > 1.0 && c.y1 - c.y0 > 1.0, "{}", tr("pdf.crop.err.too_much"));
    let mut arr = doc.new_array()?;
    for x in [c.x0, c.y0, c.x1, c.y1] {
        arr.array_push(mupdf::pdf::PdfObject::new_real(x)?)?;
    }
    obj.dict_put("CropBox", arr)?;
    obj.dict_delete("TrimBox")?;
    obj.dict_delete("ArtBox")?;
    obj.dict_delete("BleedBox")?;
    Ok(())
}

impl Tool for Crop {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("auto"), Choice::key("margins")], "auto"),
            OptionSpec::float("padding", 0.0, 50.0, 5.0).unit("mm").when(Cond::Eq("mode", "auto")),
            OptionSpec::toggle("same_box", true).when(Cond::Eq("mode", "auto")),
            OptionSpec::float("top", 0.0, 300.0, 10.0).unit("mm").when(Cond::Eq("mode", "margins")),
            OptionSpec::float("bottom", 0.0, 300.0, 10.0).unit("mm").when(Cond::Eq("mode", "margins")),
            OptionSpec::float("left", 0.0, 300.0, 10.0).unit("mm").when(Cond::Eq("mode", "margins")),
            OptionSpec::float("right", 0.0, 300.0, 10.0).unit("mm").when(Cond::Eq("mode", "margins")),
            OptionSpec::pages("pages").common(),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let n = doc.page_count()?.max(0) as usize;
        let sel = pages(v, "pages", n)?;
        let mut boxes: Vec<(usize, Rect)> = Vec::new();
        for (k, &p) in sel.iter().enumerate() {
            cx.check_cancel()?;
            page_msg(cx, k, sel.len());
            let page = doc.load_page(p as i32)?;
            let b = page.bounds()?;
            let r = if v.str("mode") == "margins" {
                Rect { x0: b.x0 + mm(v.float("left")), y0: b.y0 + mm(v.float("top")), x1: b.x1 - mm(v.float("right")), y1: b.y1 - mm(v.float("bottom")) }
            } else {
                let dpi = 50.0;
                let pix = engine_pdf::render(&page, dpi, true)?;
                let g = engine_pdf::pixmap_to_gray(&pix);
                match engine_pdf::scan::content_bbox(&g, 12) {
                    Some((x0, y0, x1, y1)) => {
                        let k = 72.0 / dpi;
                        let pad = mm(v.float("padding"));
                        Rect {
                            x0: b.x0 + x0 as f32 * k - pad,
                            y0: b.y0 + y0 as f32 * k - pad,
                            x1: b.x0 + x1 as f32 * k + pad,
                            y1: b.y0 + y1 as f32 * k + pad,
                        }
                    }
                    None => continue, // blank page: leave it alone
                }
            };
            boxes.push((p, r));
            prog(cx, k + 1, sel.len(), 0.0, 0.8);
        }
        if v.str("mode") == "auto" && v.bool("same_box") && !boxes.is_empty() {
            // One box for all pages (union), so the document keeps a single page size.
            let u = boxes.iter().skip(1).fold(boxes[0].1, |a, (_, b)| a.union(b));
            for b in boxes.iter_mut() {
                b.1 = u;
            }
        }
        for (p, r) in &boxes {
            let mut page = doc.load_pdf_page(*p as i32)?;
            let b = page.bounds()?;
            let r = Rect { x0: r.x0.max(b.x0), y0: r.y0.max(b.y0), x1: r.x1.min(b.x1), y1: r.y1.min(b.y1) };
            set_visible_box(&mut page, &doc, r)?;
        }
        engine_pdf::stamp_producer(&mut doc)?;
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &boxes.len().to_string())]))])
    }
}

// ------------------------------------------------------------------------------------------

pub struct AttachToEach {
    meta: ToolMeta,
}

impl AttachToEach {
    pub fn new() -> Self {
        AttachToEach {
            meta: meta("pdf.attach-to-each", Family::Pdf, PDF_IN, "PDF", &["cover", "prepend", "append", "terms", "insert", "add pages"], "with-pages"),
        }
    }
}

impl Tool for AttachToEach {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::file("file", &["pdf"]),
            OptionSpec::choice("position", vec![Choice::key("before"), Choice::key("after"), Choice::key("both")], "before"),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let extra = v.path("file").ok_or_else(|| anyhow::anyhow!("{}", tr("pdf.attach-to-each.err.file")))?;
        let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
        let mut add = engine_pdf::open_pdf(extra, "")?;
        let k = add.page_count()?.max(0) as usize;
        anyhow::ensure!(k > 0, "{}", tr("pdf.err.no_pages"));
        let all: Vec<usize> = (0..k).collect();
        let pos = v.str("position");
        if pos == "before" || pos == "both" {
            pages::copy_pages(&mut doc, &mut add, &all, Some(0))?;
        }
        cx.progress(0.4);
        if pos == "after" || pos == "both" {
            pages::copy_pages(&mut doc, &mut add, &all, None)?;
        }
        engine_pdf::stamp_producer(&mut doc)?;
        let out = out_for(cx, input, "pdf");
        engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { keep_encryption: !password(v).is_empty(), ..Default::default() })?;
        Ok(vec![Output::new(out)])
    }
}
