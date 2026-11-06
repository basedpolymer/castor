//! Copying pages between documents with one shared graft map (shared fonts and images are
//! copied once), keeping annotations, links between copied pages and form fields — which
//! MuPDF's plain `pdf_graft_page` drops.

use std::collections::HashMap;

use anyhow::Result;
use mupdf::link::LinkDestination;
use mupdf::pdf::{PdfDocument, PdfGraftMap, PdfObject};
use mupdf::{DestinationKind, Outline};

/// Page keys copied (inheritable ones are resolved), like MuPDF's graft code, plus annotations.
const COPY_KEYS: &[&str] = &["Contents", "Resources", "MediaBox", "CropBox", "BleedBox", "TrimBox", "ArtBox", "Rotate", "UserUnit", "Group"];

/// Prepare a source document for copying (it is modified in memory, never saved):
/// - annotations lose `/P` (it would drag the whole source page tree along);
/// - internal link destinations `[pageRef …]` become `[pageIndex …]`, fixed after copying.
pub fn prepare_source(src: &mut PdfDocument) -> Result<()> {
    // Idempotent and marked, so repeated copies from the same source do not redo the work.
    let mut cat = src.catalog()?;
    if cat.get_dict("CastorPrepared")?.is_some() {
        return Ok(());
    }
    prepare(src)?;
    cat.dict_put("CastorPrepared", PdfObject::new_bool(true))?;
    Ok(())
}

fn prepare(src: &mut PdfDocument) -> Result<HashMap<i32, usize>> {
    let n = src.page_count()?.max(0) as usize;
    let mut page_index: HashMap<i32, usize> = HashMap::new();
    for i in 0..n {
        let p = src.find_page(i as i32)?;
        if p.is_indirect()? {
            page_index.insert(p.as_indirect()?, i);
        }
    }
    let fix_dest = |d: &mut PdfObject| -> Result<()> {
        if d.is_array()? && d.len()? > 0 {
            if let Some(first) = d.get_array(0)? {
                if first.is_indirect()? {
                    if let Some(idx) = page_index.get(&first.as_indirect()?) {
                        d.array_put(0, PdfObject::new_int(*idx as i32)?)?;
                    }
                }
            }
        }
        Ok(())
    };
    for i in 0..n {
        let p = src.find_page(i as i32)?;
        let Some(annots) = p.get_dict("Annots")? else { continue };
        if !annots.is_array()? {
            continue;
        }
        for k in 0..annots.len()? {
            let Some(mut a) = annots.get_array(k as i32)? else { continue };
            if !a.is_dict()? {
                continue;
            }
            a.dict_delete("P")?;
            if let Some(mut d) = a.get_dict("Dest")? {
                fix_dest(&mut d)?;
            }
            if let Some(act) = a.get_dict("A")? {
                if let Some(mut d) = act.get_dict("D")? {
                    fix_dest(&mut d)?;
                }
            }
            // Walk up the field hierarchy: kids of parents also carry /P.
            let mut parent = a.get_dict("Parent")?;
            let mut guard = 0;
            while let Some(par) = parent {
                if let Some(kids) = par.get_dict("Kids")? {
                    for j in 0..kids.len().unwrap_or(0) {
                        if let Some(mut kid) = kids.get_array(j as i32)? {
                            if kid.is_dict()? {
                                kid.dict_delete("P")?;
                            }
                        }
                    }
                }
                parent = par.get_dict("Parent")?;
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
        }
    }
    Ok(page_index)
}

/// Copy `pages` (0-based, any order, repeats allowed) of `src` into `dst`, inserted at
/// `at` (None = append). Returns the dst index of each copied page.
pub fn copy_pages(dst: &mut PdfDocument, src: &mut PdfDocument, pages: &[usize], at: Option<usize>) -> Result<Vec<usize>> {
    prepare_source(src)?;
    let mut map: PdfGraftMap = dst.new_graft_map()?;
    let start = at.unwrap_or(dst.page_count()?.max(0) as usize);
    let mut placed = Vec::with_capacity(pages.len());
    let mut fields: Vec<PdfObject> = Vec::new();
    for (k, &i) in pages.iter().enumerate() {
        let sp = src.find_page(i as i32)?;
        let mut nd = dst.new_dict()?;
        nd.dict_put("Type", PdfObject::new_name("Page")?)?;
        for key in COPY_KEYS {
            if let Some(v) = sp.get_dict_inheritable(*key)? {
                let g = map.graft_object(&v)?;
                nd.dict_put(*key, g)?;
            }
        }
        if let Some(annots) = sp.get_dict("Annots")? {
            if annots.is_array()? && annots.len()? > 0 {
                let g = map.graft_object(&annots)?;
                nd.dict_put("Annots", g)?;
            }
        }
        let r = dst.add_object(&nd)?;
        let pos = start + k;
        dst.insert_page(pos as i32, &r)?;
        placed.push(pos);
    }
    // Re-attach annotations to their page, fix link targets, collect form fields.
    let index_of: HashMap<usize, usize> = pages.iter().copied().zip(placed.iter().copied()).collect();
    for &pos in &placed {
        let page = dst.find_page(pos as i32)?;
        let Some(annots) = page.get_dict("Annots")? else { continue };
        let mut keep = dst.new_array()?;
        for k in 0..annots.len()? {
            let Some(mut a) = annots.get_array(k as i32)? else { continue };
            if !a.is_dict()? {
                continue;
            }
            a.dict_put("P", page.clone())?;
            let mut ok = true;
            for holder in [Some(a.clone()), a.get_dict("A")?] {
                let Some(h) = holder else { continue };
                let key = if h.get_dict("Dest")?.is_some() { "Dest" } else { "D" };
                if let Some(mut d) = h.get_dict(key)? {
                    if d.is_array()? && d.len()? > 0 {
                        if let Some(first) = d.get_array(0)? {
                            if first.is_int()? {
                                match index_of.get(&(first.as_int()? as usize)) {
                                    Some(p) => d.array_put(0, dst.find_page(*p as i32)?)?,
                                    None => ok = false,
                                }
                            }
                        }
                    }
                }
            }
            if !ok {
                continue; // link to a page that was not copied
            }
            if a.get_dict("FT")?.is_some() || a.get_dict("Parent")?.is_some() {
                // Widget: root field.
                let mut root = a.clone();
                let mut guard = 0;
                while let Some(p) = root.get_dict("Parent")? {
                    root = p;
                    guard += 1;
                    if guard > 64 {
                        break;
                    }
                }
                fields.push(root);
            }
            keep.array_push(a)?;
        }
        let mut page = page;
        page.dict_put("Annots", keep)?;
    }
    if !fields.is_empty() {
        merge_acroform(dst, src, &mut map, fields)?;
    }
    Ok(placed)
}

fn merge_acroform(dst: &mut PdfDocument, src: &PdfDocument, map: &mut PdfGraftMap, fields: Vec<PdfObject>) -> Result<()> {
    let mut cat = dst.catalog()?;
    let mut form = match cat.get_dict("AcroForm")? {
        Some(f) => f,
        None => {
            let f = dst.new_dict()?;
            let f = dst.add_object(&f)?;
            cat.dict_put("AcroForm", f.clone())?;
            f
        }
    };
    let mut arr = match form.get_dict("Fields")? {
        Some(a) if a.is_array()? => a,
        _ => {
            let a = dst.new_array()?;
            form.dict_put("Fields", a)?;
            form.get_dict("Fields")?.ok_or_else(|| anyhow::anyhow!("Fields"))?
        }
    };
    let mut seen: Vec<i32> = Vec::new();
    for i in 0..arr.len()? {
        if let Some(o) = arr.get_array(i as i32)? {
            if o.is_indirect()? {
                seen.push(o.as_indirect()?);
            }
        }
    }
    for f in fields {
        if f.is_indirect()? {
            let n = f.as_indirect()?;
            if seen.contains(&n) {
                continue;
            }
            seen.push(n);
        }
        arr.array_push(f)?;
    }
    if let Some(sf) = src.catalog()?.get_dict("AcroForm")? {
        for key in ["DR", "DA", "NeedAppearances", "Q"] {
            if form.get_dict(key)?.is_none() {
                if let Some(v) = sf.get_dict(key)? {
                    let g = map.graft_object(&v)?;
                    form.dict_put(key, g)?;
                }
            }
        }
    }
    Ok(())
}

/// The outline (bookmarks) of a document, flattened with page indices: (level, title, page).
pub fn outline_entries(doc: &mupdf::Document) -> Vec<(usize, String, Option<usize>)> {
    fn walk(v: &[Outline], level: usize, out: &mut Vec<(usize, String, Option<usize>)>) {
        for o in v {
            out.push((level, o.title.clone(), o.dest.map(|d| d.loc.page_number as usize)));
            walk(&o.down, level + 1, out);
        }
    }
    let mut out = Vec::new();
    if let Ok(v) = doc.outlines() {
        walk(&v, 0, &mut out);
    }
    out
}

/// A bookmark pointing at a page (fit).
pub fn bookmark(title: &str, page: usize, down: Vec<Outline>) -> Outline {
    Outline {
        title: title.to_string(),
        uri: None,
        dest: Some(LinkDestination {
            loc: mupdf::document::Location { chapter: 0, page_in_chapter: page as u32, page_number: page as u32 },
            kind: DestinationKind::Fit,
        }),
        down,
    }
}

/// Re-target an outline tree through `map` (old page → new page); entries whose page is gone
/// are dropped (their children are kept, lifted one level).
pub fn remap_outline(v: Vec<Outline>, map: &dyn Fn(usize) -> Option<usize>) -> Vec<Outline> {
    let mut out = Vec::new();
    for o in v {
        let down = remap_outline(o.down, map);
        match o.dest {
            Some(d) => match map(d.loc.page_number as usize) {
                Some(p) => out.push(Outline {
                    title: o.title,
                    uri: None,
                    dest: Some(LinkDestination {
                        loc: mupdf::document::Location { chapter: 0, page_in_chapter: p as u32, page_number: p as u32 },
                        kind: d.kind,
                    }),
                    down,
                }),
                None => out.extend(down),
            },
            None if o.uri.is_some() => out.push(Outline { title: o.title, uri: o.uri, dest: None, down }),
            None => out.extend(down),
        }
    }
    out
}
