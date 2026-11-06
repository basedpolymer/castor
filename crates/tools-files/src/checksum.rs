//! Checksums — the reference example of a Castor tool (N files → 1 list, progress, cancel).

use std::io::Read;

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileKind, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, ToolStatus, Family};
use sha2::Digest;

pub struct Checksum {
    meta: ToolMeta,
}

impl Checksum {
    pub fn new() -> Self {
        Checksum {
            meta: ToolMeta {
                id: "files.checksum",
                family: Family::Files,
                inputs: InputSpec::kinds(&[FileKind::Any]).folders(),
                output: "TXT",
                keywords: &["hash", "sha256", "sha512", "verify", "integrity", "digest"],
                status: ToolStatus::Ready,
                requires: &[],
                suffix: "checksums",
                in_place: false,
                serial: false,
            },
        }
    }
}

impl Tool for Checksum {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("algorithm", vec![Choice::lit("sha256", "SHA-256"), Choice::lit("sha512", "SHA-512")], "sha256"),
            OptionSpec::choice("output", vec![Choice::key("list"), Choice::key("each")], "list"),
        ]
    }

    /// "list": every file in one job → one SHA256SUMS file. "each": one job per file.
    fn plan(&self, inputs: &[castor_core::FileInfo], values: &Values) -> Result<Vec<Job>> {
        let files = castor_core::files::expand(&inputs.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), true);
        anyhow::ensure!(!files.is_empty(), "no files to hash");
        Ok(if values.str("output") == "each" { files.into_iter().map(Job::single).collect() } else { vec![Job::many(files)] })
    }

    fn run(&self, job: &Job, values: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let algo = values.str("algorithm");
        let total: u64 = job.inputs.iter().map(|f| f.size).sum::<u64>().max(1);
        let mut done = 0u64;
        let mut lines = String::new();
        for (i, f) in job.inputs.iter().enumerate() {
            cx.message(format!("{} ({}/{})", f.name(), i + 1, job.inputs.len()));
            let mut file = std::fs::File::open(&f.path)?;
            let mut buf = vec![0u8; 1 << 20];
            let mut h256 = sha2::Sha256::new();
            let mut h512 = sha2::Sha512::new();
            loop {
                cx.check_cancel()?;
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                if algo == "sha512" { h512.update(&buf[..n]) } else { h256.update(&buf[..n]) }
                done += n as u64;
                cx.progress(done as f32 / total as f32);
            }
            let digest = if algo == "sha512" { hex::encode(h512.finalize()) } else { hex::encode(h256.finalize()) };
            // GNU coreutils format, readable by `sha256sum -c`.
            lines.push_str(&format!("{digest}  {}\n", f.name()));
        }
        let ext = if algo == "sha512" { "sha512" } else { "sha256" };
        let out = if job.inputs.len() == 1 {
            cx.output_named(&format!("{}.{ext}", job.inputs[0].name()))
        } else {
            cx.output_named(&format!("{}SUMS.txt", ext.to_uppercase()))
        };
        std::fs::write(&out, lines)?;
        Ok(vec![Output::new(out)])
    }
}
