//! `castor-cli` as shipped: a few hundred kilobytes that start the app binary in CLI mode
//! (`castor cli <args>`) with this process's stdin/stdout/stderr, and return its exit code.
//! The engines (MuPDF, ONNX Runtime, Typst…) then live in one binary only.
//!
//! Packaging renames this binary to `castor-cli` (`castor-cli.exe`) and puts it next to
//! `castor` (Windows: install folder; macOS: `Castor.app/Contents/MacOS/`).

use std::path::PathBuf;
use std::process::{Command, ExitCode};

fn app_binary() -> Option<PathBuf> {
    // Resolve symlinks first: Homebrew links `castor-cli` from its bin folder into the app bundle.
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    let dir = exe.parent()?;
    let name = if cfg!(windows) { "castor.exe" } else { "castor" };
    let candidate = dir.join(name);
    candidate.is_file().then_some(candidate)
}

fn main() -> ExitCode {
    let Some(app) = app_binary() else {
        eprintln!("castor-cli: the Castor app binary was not found next to this program.");
        return ExitCode::from(127);
    };
    match Command::new(app).arg("cli").args(std::env::args_os().skip(1)).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("castor-cli: could not start Castor: {e}");
            ExitCode::from(126)
        }
    }
}
