//! pdf.fill-form — list the fields of a form, or fill it from a CSV / JSON file (one PDF per
//! row) or from "name = value" lines; optional flattening.

use std::collections::BTreeMap;

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_pdf::mupdf::pdf::{PdfDocument, PdfWidget, WidgetType};

use crate::common::*;

pub struct FillForm {
    meta: ToolMeta,
}

impl FillForm {
    pub fn new() -> Self {
        FillForm {
            meta: meta("pdf.fill-form", Family::Pdf, PDF_IN, "PDF · JSON", &["form", "fields", "acroform", "mail merge", "csv", "fill in"], "filled"),
        }
    }
}

fn kind_name(t: WidgetType) -> &'static str {
    match t {
        WidgetType::Text => "text",
        WidgetType::Checkbox => "checkbox",
        WidgetType::RadioButton => "radio",
        WidgetType::Combobox => "combobox",
        WidgetType::Listbox => "listbox",
        WidgetType::Signature => "signature",
        WidgetType::Button => "button",
        _ => "unknown",
    }
}

/// "On" appearance state names of a checkbox / radio widget.
fn on_states(w: &PdfWidget) -> Vec<String> {
    let mut out = Vec::new();
    let obj = w.annotation().object();
    if let Ok(Some(ap)) = obj.get_dict("AP") {
        if let Ok(Some(n)) = ap.get_dict("N") {
            for i in 0..n.dict_len().unwrap_or(0) {
                if let Ok(Some(k)) = n.get_dict_key(i as i32) {
                    if let Ok(name) = k.as_name() {
                        let s = String::from_utf8_lossy(&name).into_owned();
                        if s != "Off" {
                            out.push(s);
                        }
                    }
                }
            }
        }
    }
    out
}

fn truthy(s: &str) -> bool {
    matches!(s.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "y" | "x" | "on" | "oui" | "ja" | "si" | "sí" | "checked" | "✓" | "✔")
}

/// List every field: name → (type, value, options).
fn list_fields(doc: &PdfDocument) -> Result<serde_json::Value> {
    let mut fields = serde_json::Map::new();
    let n = doc.page_count()?;
    for i in 0..n {
        let page = doc.load_pdf_page(i)?;
        for w in page.widgets() {
            let Some(name) = w.name().ok().flatten() else { continue };
            let t = w.r#type().unwrap_or(WidgetType::Unknown);
            let entry = fields.entry(name).or_insert_with(|| {
                serde_json::json!({ "type": kind_name(t), "value": w.value().ok().flatten().unwrap_or_default(), "page": i + 1 })
            });
            if matches!(t, WidgetType::Checkbox | WidgetType::RadioButton) {
                let states = on_states(&w);
                if let Some(obj) = entry.as_object_mut() {
                    let arr = obj.entry("options").or_insert_with(|| serde_json::json!([]));
                    if let Some(a) = arr.as_array_mut() {
                        for s in states {
                            if !a.iter().any(|x| x == &serde_json::Value::String(s.clone())) {
                                a.push(s.into());
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(serde_json::Value::Object(fields))
}

/// Fill one record; returns (fields set, names not found).
fn fill(doc: &mut PdfDocument, rec: &BTreeMap<String, String>) -> Result<(usize, Vec<String>)> {
    let mut used: Vec<&String> = Vec::new();
    let mut set = 0;
    let n = doc.page_count()?;
    for i in 0..n {
        let page = doc.load_pdf_page(i)?;
        let widgets: Vec<PdfWidget> = page.widgets().collect();
        for mut w in widgets {
            let Some(name) = w.name().ok().flatten() else { continue };
            let Some((key, val)) = rec.get_key_value(&name).or_else(|| rec.iter().find(|(k, _)| k.eq_ignore_ascii_case(&name))) else {
                continue;
            };
            let t = w.r#type().unwrap_or(WidgetType::Unknown);
            let value = match t {
                WidgetType::Checkbox => {
                    let states = on_states(&w);
                    if truthy(val) || states.iter().any(|s| s == val) {
                        states.first().cloned().unwrap_or_else(|| "Yes".into())
                    } else {
                        "Off".into()
                    }
                }
                WidgetType::RadioButton => {
                    let states = on_states(&w);
                    if states.iter().any(|s| s == val) { val.clone() } else { continue }
                }
                WidgetType::Signature | WidgetType::Button => continue,
                _ => val.clone(),
            };
            if w.set_value(doc, &value, true).unwrap_or(false) {
                let _ = w.update();
                set += 1;
            }
            used.push(key);
        }
    }
    let missing = rec.keys().filter(|k| !used.contains(k)).cloned().collect();
    Ok((set, missing))
}

/// Minimal CSV/TSV parser (quotes, doubled quotes, separators , ; or tab).
fn parse_csv(s: &str) -> Vec<Vec<String>> {
    let first = s.lines().next().unwrap_or("");
    let sep = if first.contains('\t') {
        '\t'
    } else if first.matches(';').count() > first.matches(',').count() {
        ';'
    } else {
        ','
    };
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = s.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cell.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                cell.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            if row.iter().any(|x| !x.is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
        } else {
            cell.push(c);
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

fn records(v: &Values) -> Result<Vec<BTreeMap<String, String>>> {
    let mut recs = Vec::new();
    if let Some(p) = v.path("data") {
        let s = std::fs::read_to_string(p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?;
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if ext == "json" {
            let j: serde_json::Value = serde_json::from_str(&s)?;
            let to_rec = |o: &serde_json::Map<String, serde_json::Value>| {
                o.iter()
                    .map(|(k, v)| (k.clone(), match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Bool(b) => if *b { "true".into() } else { "false".into() },
                        other => other.to_string(),
                    }))
                    .collect::<BTreeMap<_, _>>()
            };
            match j {
                serde_json::Value::Array(a) => recs.extend(a.iter().filter_map(|x| x.as_object()).map(to_rec)),
                serde_json::Value::Object(o) => recs.push(to_rec(&o)),
                _ => anyhow::bail!("{}", tr("pdf.fill-form.err.json")),
            }
        } else {
            let rows = parse_csv(&s);
            anyhow::ensure!(rows.len() >= 2, "{}", tr("pdf.fill-form.err.csv"));
            let head = &rows[0];
            for r in &rows[1..] {
                recs.push(head.iter().cloned().zip(r.iter().cloned().chain(std::iter::repeat(String::new()))).collect());
            }
        }
    }
    let mut manual = BTreeMap::new();
    for line in v.str("values").lines() {
        if let Some((k, val)) = line.split_once('=') {
            if !k.trim().is_empty() {
                manual.insert(k.trim().to_string(), val.trim().to_string());
            }
        }
    }
    if !manual.is_empty() {
        if recs.is_empty() {
            recs.push(manual);
        } else {
            for r in recs.iter_mut() {
                for (k, val) in &manual {
                    r.insert(k.clone(), val.clone());
                }
            }
        }
    }
    anyhow::ensure!(!recs.is_empty(), "{}", tr("pdf.fill-form.err.no_data"));
    Ok(recs)
}

impl Tool for FillForm {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("fill"), Choice::key("list")], "fill"),
            OptionSpec::file("data", &["csv", "tsv", "json"]).when(Cond::Eq("mode", "fill")),
            OptionSpec::multiline("values", "").when(Cond::Eq("mode", "fill")),
            OptionSpec::text("name_field", "").when(Cond::Eq("mode", "fill")).advanced(),
            OptionSpec::toggle("flatten", false).when(Cond::Eq("mode", "fill")),
            password_opt(),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        if v.str("mode") == "list" {
            let doc = engine_pdf::open_pdf(&input.path, password(v))?;
            let fields = list_fields(&doc)?;
            let count = fields.as_object().map(|o| o.len()).unwrap_or(0);
            anyhow::ensure!(count > 0, "{}", tr("pdf.fill-form.err.no_fields"));
            let out = cx.output_named(&format!("{}-fields.json", input.stem()));
            std::fs::write(&out, serde_json::to_string_pretty(&fields)?)?;
            return Ok(vec![Output::new(out).with_note(tr_args("pdf.fill-form.note.list", &[("count", &count.to_string())]))]);
        }
        let recs = records(v)?;
        let mut outs = Vec::new();
        let mut cleanup = Cleanup::new();
        let digits = recs.len().to_string().len();
        for (i, rec) in recs.iter().enumerate() {
            cx.check_cancel()?;
            let mut doc = engine_pdf::open_pdf(&input.path, password(v))?;
            anyhow::ensure!(doc.has_acro_form()?, "{}", tr("pdf.fill-form.err.no_fields"));
            let (set, missing) = fill(&mut doc, rec)?;
            if v.bool("flatten") {
                doc.bake(true, true)?;
                doc.catalog()?.dict_delete("AcroForm")?;
            }
            engine_pdf::stamp_producer(&mut doc)?;
            let out = if recs.len() == 1 {
                out_for(cx, input, "pdf")
            } else {
                let nf = v.str("name_field").trim();
                let label = rec.get(nf).filter(|s| !nf.is_empty() && !s.trim().is_empty()).cloned().unwrap_or_else(|| format!("{:0digits$}", i + 1));
                cx.output_named(&format!("{}-{}.pdf", input.stem(), label))
            };
            cleanup.add(&out);
            engine_pdf::save_pdf(&doc, &out, &engine_pdf::Save { appearance: true, ..Default::default() })?;
            let mut note = tr_args("pdf.fill-form.note.fill", &[("count", &set.to_string())]);
            if !missing.is_empty() {
                note.push_str(" · ");
                note.push_str(&tr_args("pdf.fill-form.note.missing", &[("names", &missing.join(", "))]));
            }
            outs.push(Output::new(out).with_note(note));
            prog(cx, i + 1, recs.len(), 0.0, 1.0);
        }
        cleanup.keep();
        Ok(outs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv() {
        let r = parse_csv("a;b\n\"x;1\";\"say \"\"hi\"\"\"\n");
        assert_eq!(r, vec![vec!["a", "b"], vec!["x;1", "say \"hi\""]]);
    }
}
