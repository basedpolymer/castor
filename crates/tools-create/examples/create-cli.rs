//! Create + files twin of `castor-cli run` for development (no MuPDF / ONNX Runtime build):
//!
//! ```text
//! cargo run -p tools-create --example create-cli -- list
//! cargo run -p tools-create --example create-cli -- run create.qr-code --set style=dots --out out
//! cargo run -p tools-create --example create-cli -- preview create.gradient --set palette=ocean --out g.png
//! ```
//! `preview <tool> [file] [--set k=v]… --out file.png` writes the tool's live preview.
//! `undo <tool> <journal.json>` undoes an in-place run (the journal printed by `run --journal`).

use std::path::PathBuf;

use castor_core::options::Values;
use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::{CancelToken, FileInfo, units};

fn main() {
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> anyhow::Result<()> {
    castor_core::i18n::set_language(&std::env::var("CASTOR_LANG").unwrap_or_else(|_| "en".into()));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut tools = tools_create::tools();
    tools.extend(tools_files::tools());
    let find = |id: &str| tools.iter().find(|t| t.meta().id == id).cloned().ok_or_else(|| anyhow::anyhow!("unknown tool {id}"));
    match args.first().map(String::as_str) {
        Some("list") | None => {
            for t in &tools {
                let m = t.meta();
                println!("{:<26} {:<26} {}", m.id, m.name(), m.desc());
            }
        }
        Some("info") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("info <tool>"))?;
            let t = find(id)?;
            for o in t.options() {
                println!("  {:<16} {:<40} default {:?}", o.key, castor_core::tr(&o.label_key(id)), o.default);
            }
        }
        Some(cmd @ ("run" | "preview")) => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("{cmd} <tool> files… [--set k=v] [--out dir]"))?;
            let t = find(id)?;
            let specs = t.options();
            let mut values = Values::defaults(&specs);
            let mut files = Vec::new();
            let mut out = None;
            let mut journal = false;
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--set" => {
                        let kv = it.next().ok_or_else(|| anyhow::anyhow!("--set needs KEY=VALUE"))?;
                        let (k, v) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("--set expects KEY=VALUE"))?;
                        let s = specs.iter().find(|s| s.key == k).ok_or_else(|| anyhow::anyhow!("{id} has no option {k}"))?;
                        values.set(k, s.parse(&v.replace("\\n", "\n"))?);
                    }
                    "--out" => out = it.next().map(PathBuf::from),
                    "--journal" => journal = true,
                    f => files.push(PathBuf::from(f)),
                }
            }
            if cmd == "preview" {
                let input = files.first().map(FileInfo::from_path);
                let t0 = std::time::Instant::now();
                let p = t.preview(input.as_ref(), &values, 900)?.ok_or_else(|| anyhow::anyhow!("no preview"))?;
                let dest = out.unwrap_or_else(|| PathBuf::from("preview.png"));
                std::fs::write(&dest, &p.png)?;
                eprintln!("{} ({}) {}", dest.display(), units::duration(t0.elapsed().as_secs_f64()), p.caption.unwrap_or_default());
                return Ok(());
            }
            // Folders are passed as folders (as the app does), not expanded like castor-cli.
            let inputs: Vec<FileInfo> = files.iter().map(FileInfo::from_path).filter(|f| t.accepts(f)).collect();
            let req = RunRequest {
                steps: vec![Step { tool: t, values }],
                inputs,
                output: out.map(OutputPolicy::Folder).unwrap_or(OutputPolicy::NextToInput { fallback: std::env::current_dir()? }),
                chain_name: None,
                max_parallel: 0,
                record: false,
            };
            let rec = runner::run_blocking(req, CancelToken::new(), |ev| match ev {
                RunEvent::JobMessage { msg, .. } => eprintln!("  {msg}"),
                RunEvent::JobDone { outputs, secs, .. } => {
                    for o in outputs {
                        eprintln!("  ✓ {}  ({})  {}", o.path.display(), units::duration(*secs as f64), o.note.clone().unwrap_or_default());
                    }
                }
                RunEvent::JobFailed { error, .. } => eprintln!("  ✗ {error}"),
                _ => {}
            });
            eprintln!("{:?} in {}", rec.status, units::duration(rec.duration_secs as f64));
            if journal {
                for (tool, j) in &rec.undo {
                    println!("{tool} {}", serde_json::to_string(j)?);
                }
            }
            if rec.outputs.is_empty() {
                std::process::exit(2);
            }
        }
        Some("undo") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("undo <tool> <journal.json>"))?;
            let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(args.get(2).ok_or_else(|| anyhow::anyhow!("journal file"))?)?)?;
            find(id)?.undo(&j)?;
            eprintln!("undone");
        }
        Some("identify") => {
            for f in &args[1..] {
                let id = tools_files::identify(std::path::Path::new(f))?;
                println!("{f}: {} — {} ({:.0} %, {})", id.label, id.description, id.confidence * 100.0, id.engine);
            }
        }
        Some(o) => anyhow::bail!("unknown command {o}"),
    }
    Ok(())
}
