//! Media-only twin of `castor-cli run` for development (builds without the PDF/ML engines):
//!
//! ```text
//! cargo run -p tools-media --example media-cli -- list
//! cargo run -p tools-media --example media-cli -- run video.compress clip.mp4 --set quality=small --out out
//! ```

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
    castor_core::i18n::set_language(&std::env::var("CASTOR_LANG").unwrap_or_default());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tools = tools_media::tools();
    match args.first().map(String::as_str) {
        Some("list") | None => {
            for t in &tools {
                let m = t.meta();
                println!("{:<26} {:<26} {}", m.id, m.name(), m.desc());
            }
        }
        Some("info") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("info <tool>"))?;
            let t = tools.iter().find(|t| t.meta().id == id).ok_or_else(|| anyhow::anyhow!("unknown tool {id}"))?;
            for o in t.options() {
                println!("  {:<14} {:<40} default {:?}", o.key, castor_core::tr(&o.label_key(id)), o.default);
            }
        }
        Some("run") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("run <tool> files… [--set k=v] [--out dir]"))?;
            let t = tools.iter().find(|t| t.meta().id == id).ok_or_else(|| anyhow::anyhow!("unknown tool {id}"))?.clone();
            let specs = t.options();
            let mut values = Values::defaults(&specs);
            let mut files = Vec::new();
            let mut out = None;
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--set" => {
                        let kv = it.next().ok_or_else(|| anyhow::anyhow!("--set needs KEY=VALUE"))?;
                        let (k, v) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("--set expects KEY=VALUE"))?;
                        let s = specs.iter().find(|s| s.key == k).ok_or_else(|| anyhow::anyhow!("{id} has no option {k}"))?;
                        values.set(k, s.parse(v)?);
                    }
                    "--out" => out = it.next().map(PathBuf::from),
                    f => files.push(PathBuf::from(f)),
                }
            }
            let inputs: Vec<FileInfo> = castor_core::files::expand(&files, true).into_iter().filter(|f| t.accepts(f)).collect();
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
            if rec.outputs.is_empty() {
                std::process::exit(2);
            }
        }
        // Engine helpers: `waveform <file> <buckets>`, `thumbnail <file> <out.png>`, `probe <file>`.
        Some("waveform") => {
            let f = PathBuf::from(args.get(1).ok_or_else(|| anyhow::anyhow!("waveform <file> <n>"))?);
            let n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1000);
            let t = std::time::Instant::now();
            let peaks = engine_media::waveform(&f, n, &CancelToken::new())?;
            let max = peaks.iter().map(|p| p.1).fold(0.0f32, f32::max);
            eprintln!("{} buckets in {}, max peak {max:.3}", peaks.len(), units::duration(t.elapsed().as_secs_f64()));
        }
        Some("thumbnail") => {
            let f = PathBuf::from(args.get(1).ok_or_else(|| anyhow::anyhow!("thumbnail <file> <out.png>"))?);
            let png = engine_media::thumbnail(&f, None, 512)?;
            std::fs::write(args.get(2).map(String::as_str).unwrap_or("thumb.png"), png)?;
        }
        Some("probe") => {
            let f = PathBuf::from(args.get(1).ok_or_else(|| anyhow::anyhow!("probe <file>"))?);
            println!("{:#?}", engine_media::probe(&f)?);
        }
        Some(o) => anyhow::bail!("unknown command {o}"),
    }
    Ok(())
}
