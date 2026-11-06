//! pdf.compare — visual diff of two PDFs: a report PDF (summary page, then the pages of the
//! second file with changed regions highlighted) and a short note.

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, FileInfo, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::draw::Canvas;
use engine_pdf::fonts::Face;
use engine_pdf::mupdf::pdf::PdfDocument;
use engine_pdf::mupdf::Rect;

use crate::common::*;

pub struct Compare {
    meta: ToolMeta,
}

impl Compare {
    pub fn new() -> Self {
        Compare {
            meta: meta("pdf.compare", Family::Pdf, PDF_IN.min(2).max(2), "PDF", &["diff", "difference", "changes", "versions", "redline"], "compare"),
        }
    }
}

/// Changed regions (pixel boxes) between two gray images of the same size.
pub fn diff_regions(a: &image::GrayImage, b: &image::GrayImage, threshold: u8, cell: u32) -> Vec<(u32, u32, u32, u32)> {
    let (w, h) = a.dimensions();
    let (cw, ch) = (w.div_ceil(cell), h.div_ceil(cell));
    let mut grid = vec![false; (cw * ch) as usize];
    for y in 0..h {
        for x in 0..w {
            let d = (a.get_pixel(x, y)[0] as i16 - b.get_pixel(x, y)[0] as i16).unsigned_abs();
            if d > threshold as u16 {
                grid[((y / cell) * cw + x / cell) as usize] = true;
            }
        }
    }
    // Connected components of changed cells, with a one-cell bridge.
    let mut seen = vec![false; grid.len()];
    let mut out = Vec::new();
    for start in 0..grid.len() {
        if !grid[start] || seen[start] {
            continue;
        }
        let mut stack = vec![start];
        seen[start] = true;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
        while let Some(i) = stack.pop() {
            let (cx, cy) = (i as u32 % cw, i as u32 / cw);
            x0 = x0.min(cx);
            y0 = y0.min(cy);
            x1 = x1.max(cx);
            y1 = y1.max(cy);
            for dy in -2i32..=2 {
                for dx in -2i32..=2 {
                    let (nx, ny) = (cx as i32 + dx, cy as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= cw as i32 || ny >= ch as i32 {
                        continue;
                    }
                    let j = (ny as u32 * cw + nx as u32) as usize;
                    if grid[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        out.push((x0 * cell, y0 * cell, ((x1 + 1) * cell).min(w), ((y1 + 1) * cell).min(h)));
    }
    out
}

fn render_gray(doc: &PdfDocument, i: usize, dpi: f32) -> Result<(image::GrayImage, Rect)> {
    let page = doc.load_page(i as i32)?;
    let b = page.bounds()?;
    let pix = engine_pdf::render(&page, dpi, true)?;
    Ok((engine_pdf::pixmap_to_gray(&pix), b))
}

impl Tool for Compare {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("sensitivity", vec![Choice::key("high"), Choice::key("normal"), Choice::key("low")], "normal"),
            OptionSpec::color("highlight", "#e74c3c66"),
            password_opt(),
        ]
    }
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(inputs.len() == 2, "{}", tr("pdf.compare.err.two"));
        Ok(vec![Job::many(inputs.to_vec())])
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        anyhow::ensure!(job.inputs.len() == 2, "{}", tr("pdf.compare.err.two"));
        let (fa, fb) = (&job.inputs[0], &job.inputs[1]);
        let a = engine_pdf::open_pdf(&fa.path, password(v))?;
        let mut b = engine_pdf::open_pdf(&fb.path, password(v))?;
        let (na, nb) = (a.page_count()?.max(0) as usize, b.page_count()?.max(0) as usize);
        let (threshold, dpi) = match v.str("sensitivity") {
            "high" => (12u8, 110.0),
            "low" => (60u8, 72.0),
            _ => (32u8, 90.0),
        };
        let (hl, alpha) = color(v, "highlight");
        // Report = summary page + copy of B's pages.
        let mut rep = PdfDocument::new();
        rep.new_page(mupdf::Size::A4)?;
        engine_pdf::pages::copy_pages(&mut rep, &mut b, &(0..nb).collect::<Vec<_>>(), None)?;
        let mut changed_pages: Vec<(usize, usize)> = Vec::new();
        let common = na.min(nb);
        for i in 0..common {
            cx.check_cancel()?;
            page_msg(cx, i, common);
            let (ga, ra) = render_gray(&a, i, dpi)?;
            let (gb, rb) = render_gray(&b, i, dpi)?;
            let regions: Vec<Rect> = if ga.dimensions() != gb.dimensions() || (ra.width() - rb.width()).abs() > 1.0 {
                vec![rb]
            } else {
                let k = 72.0 / dpi;
                diff_regions(&ga, &gb, threshold, 6)
                    .into_iter()
                    .map(|(x0, y0, x1, y1)| Rect {
                        x0: rb.x0 + x0 as f32 * k - 2.0,
                        y0: rb.y0 + y0 as f32 * k - 2.0,
                        x1: rb.x0 + x1 as f32 * k + 2.0,
                        y1: rb.y0 + y1 as f32 * k + 2.0,
                    })
                    .collect()
            };
            if regions.is_empty() {
                continue;
            }
            changed_pages.push((i, regions.len()));
            let mut page = rep.load_pdf_page(i as i32 + 1)?;
            let mut canvas = Canvas::new(&page)?;
            for r in &regions {
                canvas.rect(&mut rep, &mut page, *r, Some(hl), Some(([hl[0] * 0.8, hl[1] * 0.3, hl[2] * 0.3], 0.8)), alpha.max(0.15))?;
            }
            canvas.commit(&mut rep, &mut page, true)?;
            prog(cx, i + 1, common, 0.05, 0.9);
        }
        // Pages only in B: mark the whole page.
        for i in common..nb {
            let mut page = rep.load_pdf_page(i as i32 + 1)?;
            let mut canvas = Canvas::new(&page)?;
            let bb = canvas.bounds;
            canvas.rect(&mut rep, &mut page, bb, None, Some((hl, 6.0)), 1.0)?;
            canvas.commit(&mut rep, &mut page, true)?;
        }
        // Summary page.
        let summary_lines = {
            let mut l = vec![
                tr("pdf.compare.report.title"),
                String::new(),
                format!("A: {} ({na})", fa.name()),
                format!("B: {} ({nb})", fb.name()),
                String::new(),
            ];
            if changed_pages.is_empty() && na == nb {
                l.push(tr("pdf.compare.report.same"));
            } else {
                for (p, n) in &changed_pages {
                    l.push(tr_args("pdf.compare.report.page", &[("page", &(p + 1).to_string()), ("count", &n.to_string())]));
                }
                if na > nb {
                    l.push(tr_args("pdf.compare.report.removed", &[("count", &(na - nb).to_string())]));
                }
                if nb > na {
                    l.push(tr_args("pdf.compare.report.added", &[("count", &(nb - na).to_string())]));
                }
            }
            l
        };
        {
            let mut page = rep.load_pdf_page(0)?;
            let mut canvas = Canvas::new(&page)?;
            let mut y = 80.0;
            for (k, line) in summary_lines.iter().take(60).enumerate() {
                let (face, size) = if k == 0 { (Face::LatinModernBold, 20.0) } else { (Face::LatinModern, 11.0) };
                if !line.is_empty() {
                    canvas.text(&mut rep, &mut page, face, size, line, 60.0, y, 0.0, [0.1, 0.1, 0.1], 1.0)?;
                }
                y += size * 1.5;
            }
            canvas.commit(&mut rep, &mut page, true)?;
        }
        engine_pdf::stamp_producer(&mut rep)?;
        let out = cx.output_named(&format!("{}-vs-{}.pdf", fa.stem(), fb.stem()));
        engine_pdf::save_pdf(&rep, &out, &engine_pdf::Save::default())?;
        let note = if changed_pages.is_empty() && na == nb {
            tr("pdf.compare.note.same")
        } else {
            tr_args("pdf.compare.note.diff", &[("pages", &changed_pages.len().to_string()), ("total", &na.max(nb).to_string())])
        };
        Ok(vec![Output::new(out).with_note(note)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regions() {
        let a = image::GrayImage::from_pixel(100, 100, image::Luma([255]));
        let mut b = a.clone();
        for y in 10..20 {
            for x in 10..30 {
                b.put_pixel(x, y, image::Luma([0]));
            }
        }
        let r = diff_regions(&a, &b, 30, 5);
        assert_eq!(r.len(), 1);
        assert!(r[0].0 <= 10 && r[0].2 >= 30);
    }
}
