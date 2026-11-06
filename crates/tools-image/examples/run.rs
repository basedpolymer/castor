//! Run an image tool through the real runner without the whole CLI (handy while other crates of
//! the workspace are being rebuilt). Same syntax as `castor-cli run`:
//!
//! ```text
//! cargo run -p tools-image --example run -- list
//! cargo run -p tools-image --example run -- image.compress photo.jpg --set mode=identical --out out
//! ```

use std::path::PathBuf;

use castor_core::options::Values;
use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::{CancelToken, Registry};

fn main() -> anyhow::Result<()> {
    castor_core::i18n::set_language("en");
    let reg = Registry::new(tools_image::tools());
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("list") {
        for t in reg.all() {
            let m = t.meta();
            println!("{:<26} {:<26} {:?}  {}", m.id, m.name(), m.status, m.desc());
        }
        return Ok(());
    }
    let id = args.first().ok_or_else(|| anyhow::anyhow!("usage: run <tool> files… [--set k=v] [--out dir]"))?;
    let tool = reg.get(id).ok_or_else(|| anyhow::anyhow!("unknown tool {id}"))?;
    let specs = tool.options();
    let mut values = Values::defaults(&specs);
    let mut files = Vec::new();
    let mut out = std::env::current_dir()?;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--set" => {
                let kv = it.next().ok_or_else(|| anyhow::anyhow!("--set needs KEY=VALUE"))?;
                let (k, v) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("--set needs KEY=VALUE"))?;
                let spec = specs.iter().find(|s| s.key == k).ok_or_else(|| anyhow::anyhow!("no option {k}"))?;
                values.set(k, spec.parse(v)?);
            }
            "--out" => out = PathBuf::from(it.next().ok_or_else(|| anyhow::anyhow!("--out needs a folder"))?),
            f => files.push(PathBuf::from(f)),
        }
    }
    let inputs: Vec<_> = castor_core::files::expand(&files, true).into_iter().filter(|f| tool.accepts(f)).collect();
    let req = RunRequest {
        steps: vec![Step { tool: tool.clone(), values }],
        inputs,
        output: OutputPolicy::Folder(out),
        chain_name: None,
        max_parallel: 0,
        record: false,
    };
    let t = std::time::Instant::now();
    let rec = runner::run_blocking(req, CancelToken::new(), |ev| match ev {
        RunEvent::JobFailed { error, .. } => eprintln!("  failed: {error}"),
        RunEvent::JobDone { outputs, secs, .. } => {
            for o in outputs {
                println!("  {}  {}  ({secs:.1} s)", o.path.display(), o.note.clone().unwrap_or_default());
            }
        }
        _ => {}
    });
    println!("  {} output(s) in {:.1} s", rec.outputs.len(), t.elapsed().as_secs_f32());
    Ok(())
}
