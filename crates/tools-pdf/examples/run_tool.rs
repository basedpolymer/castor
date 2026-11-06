//! Run one PDF tool without the whole registry (fast to build), and time it:
//! `cargo run -p tools-pdf --example run_tool -- pdf.to-images in.pdf --set dpi=60 --out out/`

use std::time::Instant;

use castor_core::options::Values;
use castor_core::{CancelToken, FileInfo, RunCx};

fn main() -> anyhow::Result<()> {
    let t0 = Instant::now();
    let mut args = std::env::args().skip(1);
    let id = args.next().ok_or_else(|| anyhow::anyhow!("usage: run_tool <tool-id> <files…> [--set k=v]… [--out dir]"))?;
    let (mut files, mut sets, mut out) = (Vec::new(), Vec::new(), std::env::temp_dir().join("castor-run-tool"));
    while let Some(a) = args.next() {
        match a.as_str() {
            "--set" => sets.push(args.next().unwrap_or_default()),
            "--out" => out = args.next().map(Into::into).unwrap_or(out),
            _ => files.push(std::path::PathBuf::from(a)),
        }
    }
    // Folders are expanded like castor-cli does; files the tool does not take are skipped.
    let files: Vec<FileInfo> = castor_core::files::expand(&files, true);
    let tool = tools_pdf::tools().into_iter().find(|t| t.meta().id == id).ok_or_else(|| anyhow::anyhow!("unknown tool {id}"))?;
    let specs = tool.options();
    let mut v = Values::defaults(&specs);
    for kv in sets {
        let (k, val) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("--set k=v"))?;
        let spec = specs.iter().find(|s| s.key == k).ok_or_else(|| anyhow::anyhow!("no option {k}"))?;
        v.set(k, spec.parse(val)?);
    }
    std::fs::create_dir_all(&out)?;
    let t1 = Instant::now();
    let files: Vec<FileInfo> = files.into_iter().filter(|f| tool.accepts(f)).collect();
    for job in tool.plan(&files, &v)? {
        let tmp = tempfile::tempdir()?;
        let cx = RunCx::new(out.clone(), tmp.path().to_path_buf(), tool.meta().suffix, CancelToken::new())
            .on_message(|m| eprintln!("  {m}"));
        for o in tool.run(&job, &v, &cx)? {
            println!("{}{}", o.path.display(), o.note.map(|n| format!("  ({n})")).unwrap_or_default());
        }
    }
    eprintln!("setup {:.2} s, run {:.2} s", (t1 - t0).as_secs_f64(), t1.elapsed().as_secs_f64());
    Ok(())
}
