//! `castor-cli` — every Castor tool from the terminal.
//!
//! ```text
//! castor-cli list [--family pdf] [--json]
//! castor-cli info pdf.compress
//! castor-cli run pdf.compress report.pdf --set preset=max --out ./out
//! castor-cli chain web-photos.toml ~/Pictures/trip
//! castor-cli search "make this pdf smaller"
//! castor-cli history
//! castor-cli mcp                      # MCP server over stdio, for AI agents
//! ```
//!
//! The same code runs inside the app binary: `castor cli <args>` and `castor mcp`, so shipped
//! builds carry the engines once (see `crates/castor-launcher` for the small `castor-cli` shim).

use std::path::PathBuf;

use anyhow::{Context, bail};
use castor_core::options::{OptionKind, Values};
use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::{CancelToken, Family, FileInfo, units};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "castor-cli", version, about = "Castor: offline file tools. Free, local, yours.")]
struct Cli {
    /// UI language for names and messages (en, fr, de…). Default: system.
    #[arg(long, global = true)]
    lang: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List tools.
    List {
        #[arg(long)]
        family: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show a tool's options, types and defaults.
    Info { tool: String },
    /// Run a tool on files (or folders, expanded recursively).
    Run {
        tool: String,
        files: Vec<PathBuf>,
        /// Option value, repeatable: --set quality=80 --set format=webp
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
        /// Output folder (default: next to each input).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Print the final record as JSON.
        #[arg(long)]
        json: bool,
        /// Max parallel jobs (0 = automatic).
        #[arg(long, default_value_t = 0)]
        jobs: usize,
    },
    /// Run a chain file (TOML) on files.
    Chain {
        file: PathBuf,
        files: Vec<PathBuf>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Search tools by words, in any shipped language.
    Search { query: Vec<String> },
    /// Recent runs.
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Serve every tool to AI agents over the Model Context Protocol (stdio). See docs/MCP.md.
    Mcp,
}

/// Run the CLI with these arguments (the first one is the program name). Returns the exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("CASTOR_LOG"))
        .with_writer(std::io::stderr)
        .try_init();
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { 2 } else { 0 };
        }
    };
    match real_main(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            1
        }
    }
}

fn real_main(cli: Cli) -> anyhow::Result<i32> {
    castor_core::i18n::set_language(cli.lang.as_deref().unwrap_or(""));
    let reg = castor_tools::registry();

    match cli.cmd {
        Cmd::List { family, json } => {
            let fam = family.as_deref().map(|f| {
                Family::ALL.into_iter().find(|x| x.key() == f).with_context(|| format!("unknown family '{f}'"))
            });
            let fam = fam.transpose()?;
            let tools: Vec<_> = reg.all().iter().filter(|t| fam.is_none_or(|f| t.meta().family == f)).collect();
            if json {
                let v: Vec<_> = tools
                    .iter()
                    .map(|t| {
                        let m = t.meta();
                        serde_json::json!({"id": m.id, "family": m.family, "name": m.name(), "desc": m.desc(),
                            "output": m.output, "status": format!("{:?}", m.status)})
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                for t in tools {
                    let m = t.meta();
                    println!("{:<32} {:<28} {}", m.id, m.name(), m.desc());
                }
            }
        }
        Cmd::Info { tool } => {
            let t = reg.get(&tool).with_context(|| format!("unknown tool '{tool}'"))?;
            let m = t.meta();
            println!("{} — {}\n{}\nid: {}   output: {}   status: {:?}", m.name(), m.family.name(), m.desc(), m.id, m.output, m.status);
            for o in t.options() {
                let label = castor_core::tr(&o.label_key(m.id));
                let ty = match &o.kind {
                    OptionKind::Choice(cs) => cs.iter().map(|c| c.value).collect::<Vec<_>>().join("|"),
                    OptionKind::Toggle => "true|false".into(),
                    OptionKind::Int { min, max, .. } => format!("int {min}..{max}"),
                    OptionKind::Float { min, max, .. } => format!("float {min}..{max}"),
                    OptionKind::Size => "WxH".into(),
                    OptionKind::Anchor => "top-left|top|…|bottom-right".into(),
                    OptionKind::PageRange => "pages e.g. 1-3,5".into(),
                    k => format!("{k:?}").split(['{', '(']).next().unwrap_or("").trim().to_lowercase(),
                };
                println!("  --set {}=…   {label}   [{ty}]   default: {:?}", o.key, o.default);
            }
        }
        Cmd::Run { tool, files, set, out, json, jobs } => {
            let t = reg.get(&tool).with_context(|| format!("unknown tool '{tool}'"))?;
            let specs = t.options();
            let mut values = Values::defaults(&specs);
            for kv in set {
                let (k, v) = kv.split_once('=').with_context(|| format!("--set expects KEY=VALUE, got '{kv}'"))?;
                let spec = specs.iter().find(|s| s.key == k).with_context(|| format!("'{tool}' has no option '{k}'"))?;
                values.set(k, spec.check(&castor_core::Value::Text(v.to_string())).with_context(|| format!("option '{k}'"))?);
            }
            let inputs = castor_core::files::expand(&files, true);
            let usable: Vec<FileInfo> = inputs.iter().filter(|f| t.accepts(f)).cloned().collect();
            if usable.len() < t.meta().inputs.min {
                bail!("{} needs at least {} matching file(s); got {} of {}", tool, t.meta().inputs.min, usable.len(), inputs.len());
            }
            let output = out.map(OutputPolicy::Folder).unwrap_or(OutputPolicy::NextToInput {
                fallback: std::env::current_dir()?,
            });
            let req = RunRequest {
                steps: vec![Step { tool: t, values }],
                inputs: usable,
                output,
                chain_name: None,
                max_parallel: jobs,
                record: true,
            };
            let rec = run_and_print(req)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rec)?);
            }
            if rec.outputs.is_empty() {
                return Ok(2);
            }
        }
        Cmd::Chain { file, files, out } => {
            let chain = castor_core::chain::Chain::from_toml(&std::fs::read_to_string(&file)?)?;
            let steps = chain.resolve_strict(&reg)?;
            let req = RunRequest {
                steps,
                inputs: castor_core::files::expand(&files, true),
                output: out.map(OutputPolicy::Folder).unwrap_or(OutputPolicy::NextToInput { fallback: std::env::current_dir()? }),
                chain_name: Some(chain.name.clone()),
                max_parallel: 0,
                record: true,
            };
            run_and_print(req)?;
        }
        Cmd::Search { query } => {
            for (t, score) in reg.search(&query.join(" ")).into_iter().take(10) {
                println!("{score:>4}  {:<32} {}", t.meta().id, t.meta().name());
            }
        }
        Cmd::Mcp => {
            castor_mcp::run_stdio()?;
        }
        Cmd::History { limit } => {
            for r in castor_core::history::load(limit) {
                println!(
                    "{}  {:<24} {:>3} → {:>3} files  {:>9} → {:<9} {:?}",
                    r.id,
                    r.chain_name.clone().unwrap_or(r.tool_id.clone()),
                    r.files_in,
                    r.files_out,
                    units::bytes(r.bytes_in),
                    units::bytes(r.bytes_out),
                    r.status
                );
            }
        }
    }
    Ok(0)
}

fn run_and_print(req: RunRequest) -> anyhow::Result<castor_core::history::RunRecord> {
    let cancel = CancelToken::new();
    {
        // Ctrl-C cancels the run cleanly (tools stop, child processes are killed, partial
        // outputs are removed by the runner) instead of killing the process half-way.
        let c = cancel.clone();
        let _ = ctrlc::set_handler(move || {
            eprintln!("
cancelling…");
            c.cancel();
        });
    }
    let mut labels: Vec<String> = Vec::new();
    let rec = runner::run_blocking(req, cancel, |ev| match ev {
        RunEvent::StepStarted { step, steps, tool_id, jobs } => {
            labels = jobs.clone();
            if *steps > 1 {
                eprintln!("— step {}/{}: {tool_id}", step + 1, steps);
            }
        }
        RunEvent::JobMessage { job, msg, .. } => eprintln!("  [{}] {msg}", labels.get(*job).cloned().unwrap_or_default()),
        RunEvent::JobDone { job, outputs, bytes_in, bytes_out, secs, .. } => {
            for o in outputs {
                eprintln!(
                    "  ✓ {} → {}  ({} → {}, {})",
                    labels.get(*job).cloned().unwrap_or_default(),
                    o.path.display(),
                    units::bytes(*bytes_in),
                    units::bytes(*bytes_out),
                    units::duration(*secs as f64)
                );
                if let Some(n) = &o.note {
                    eprintln!("    {n}");
                }
            }
        }
        RunEvent::JobFailed { job, error, .. } => {
            eprintln!("  ✗ {}: {error}", labels.get(*job).cloned().unwrap_or_default())
        }
        RunEvent::JobCancelled { job, .. } => eprintln!("  – {} cancelled", labels.get(*job).cloned().unwrap_or_default()),
        _ => {}
    });
    eprintln!(
        "{:?}: {} → {} file(s), {} → {} in {}",
        rec.status,
        rec.files_in,
        rec.files_out,
        units::bytes(rec.bytes_in),
        units::bytes(rec.bytes_out),
        units::duration(rec.duration_secs as f64)
    );
    Ok(rec)
}
