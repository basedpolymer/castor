//! `castor-mcp`: Castor's MCP server over stdio. Owner: agent E — see docs/MCP.md.
//!
//! Logs go to stderr (stdout carries the protocol). `CASTOR_LOG=info` for details,
//! `CASTOR_LANG=fr` for French names and messages, `CASTOR_HOME` for a portable profile.

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_env("CASTOR_LOG"))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
    if let Err(e) = castor_mcp::run_stdio() {
        eprintln!("castor-mcp: {e:#}");
        std::process::exit(1);
    }
}
