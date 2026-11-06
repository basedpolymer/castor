//! Command line of the `castor` GUI binary.
//!
//! ```text
//! castor [FILES…]                    open the tools home with these files
//! castor --tool pdf.compress FILES…  open that tool with the files queued
//! castor --page tools|tool:ID|runs|chains|settings|studios|panel|palette
//!        [--files FILES…] [--theme light|dark] [--lang fr] [--size 1280x800]
//!                                    deterministic state for screenshots
//! ```

use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Args {
    pub files: Vec<PathBuf>,
    pub tool: Option<String>,
    pub page: Option<String>,
    pub theme: Option<String>,
    pub lang: Option<String>,
    pub size: Option<(u32, u32)>,
    /// Start a run right away (screenshot of a running tool screen).
    pub run: bool,
    /// Start minimised to the tray (launch at login).
    pub background: bool,
    /// Render the window to this PNG after `shot_delay` seconds, then quit (`shots` feature).
    pub shot: Option<PathBuf>,
    pub shot_delay: Option<f64>,
    /// Text typed into the palette (`--page palette --query "make this pdf smaller"`).
    pub query: Option<String>,
    /// Tools home: show only this family (`--family video`).
    pub family: Option<String>,
    /// Scroll the page down by this many pixels before the shot.
    pub scroll: Option<f32>,
}

impl Args {
    /// Parse `std::env::args()` minus the program name. Unknown flags are ignored (Velopack and
    /// the OS may add their own).
    pub fn parse(args: impl IntoIterator<Item = String>) -> Args {
        let mut a = Args::default();
        let mut it = args.into_iter().peekable();
        while let Some(s) = it.next() {
            match s.as_str() {
                "--tool" => a.tool = it.next(),
                "--page" => a.page = it.next(),
                "--theme" => a.theme = it.next(),
                "--lang" => a.lang = it.next(),
                "--run" => a.run = true,
                "--background" => a.background = true,
                "--query" => a.query = it.next(),
                "--family" => a.family = it.next(),
                "--scroll" => a.scroll = it.next().and_then(|v| v.parse().ok()),
                "--shot" => a.shot = it.next().map(PathBuf::from),
                "--shot-delay" => a.shot_delay = it.next().and_then(|v| v.parse().ok()),
                "--size" => {
                    a.size = it.next().and_then(|v| {
                        let (w, h) = v.split_once(['x', 'X'])?;
                        Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
                    })
                }
                "--files" => {
                    while let Some(n) = it.peek() {
                        if n.starts_with("--") {
                            break;
                        }
                        a.files.push(PathBuf::from(it.next().unwrap()));
                    }
                }
                s if s.starts_with("--") => {}
                // macOS passes -psn_… when launched from Finder on old systems.
                s if s.starts_with("-psn_") => {}
                _ => a.files.push(PathBuf::from(s)),
            }
        }
        // Make paths absolute: they may be forwarded to another process.
        let cwd = std::env::current_dir().unwrap_or_default();
        for f in &mut a.files {
            if f.is_relative() {
                *f = cwd.join(&*f);
            }
        }
        a
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_screenshot_args() {
        let a = Args::parse(
            ["--page", "tool:pdf.compress", "--files", "a.pdf", "b.pdf", "--theme", "dark", "--size", "1280x800"]
                .map(String::from),
        );
        assert_eq!(a.page.as_deref(), Some("tool:pdf.compress"));
        assert_eq!(a.files.len(), 2);
        assert!(a.files[0].is_absolute());
        assert_eq!(a.theme.as_deref(), Some("dark"));
        assert_eq!(a.size, Some((1280, 800)));
    }
    #[test]
    fn plain_files_and_tool() {
        let a = Args::parse(["--tool", "pdf.compress", "x.pdf"].map(String::from));
        assert_eq!(a.tool.as_deref(), Some("pdf.compress"));
        assert_eq!(a.files.len(), 1);
    }
}
