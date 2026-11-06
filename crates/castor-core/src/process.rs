//! Running helper programs (ffmpeg, whisper, yt-dlp, soffice…) with cancellation,
//! line-by-line output callbacks and no console window flashing on Windows.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use crate::job::{Cancelled, CancelToken};

/// A `Command` for a helper program that never opens a console window on Windows.
pub fn command(program: impl AsRef<Path>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program.as_ref());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// Output of a finished program.
pub struct Finished {
    pub status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr_tail: String,
}

/// Run `cmd`, calling `on_stdout` / `on_stderr` for each line, killing the child on cancel.
/// Returns an error with the last stderr lines if the exit status is non-zero.
pub fn run(
    mut cmd: Command,
    cancel: &CancelToken,
    mut on_stdout: impl FnMut(&str) + Send,
    mut on_stderr: impl FnMut(&str) + Send,
) -> crate::Result<Finished> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let prog = format!("{:?}", cmd.get_program());
    let mut child = cmd.spawn().map_err(|e| anyhow::anyhow!("could not start {prog}: {e}"))?;
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let mut stdout_all = String::new();
    let mut tail: std::collections::VecDeque<String> = Default::default();

    let status = std::thread::scope(|s| -> crate::Result<std::process::ExitStatus> {
        let t_out = s.spawn(|| {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                on_stdout(&line);
                stdout_all.push_str(&line);
                stdout_all.push('\n');
            }
        });
        let t_err = s.spawn(|| {
            // ffmpeg ends progress lines with '\r': split on both.
            let mut r = BufReader::new(err);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                let mut byte = [0u8; 1];
                let mut got = false;
                while let Ok(1) = std::io::Read::read(&mut r, &mut byte) {
                    got = true;
                    if byte[0] == b'\n' || byte[0] == b'\r' {
                        break;
                    }
                    buf.push(byte[0]);
                }
                if !got {
                    break;
                }
                if buf.is_empty() {
                    continue;
                }
                let line = String::from_utf8_lossy(&buf).into_owned();
                on_stderr(&line);
                tail.push_back(line);
                if tail.len() > 30 {
                    tail.pop_front();
                }
            }
        });
        loop {
            if let Some(st) = child.try_wait()? {
                let _ = t_out.join();
                let _ = t_err.join();
                return Ok(st);
            }
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = t_out.join();
                let _ = t_err.join();
                return Err(Cancelled.into());
            }
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
    })?;

    let stderr_tail = tail.into_iter().collect::<Vec<_>>().join("\n");
    if !status.success() {
        anyhow::bail!("{prog} failed ({status}):\n{stderr_tail}");
    }
    Ok(Finished { status, stdout: stdout_all, stderr_tail })
}

/// Open a file or folder with the system default application.
pub fn open(path: &Path) -> crate::Result<()> {
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    command(program).arg(path).spawn()?;
    Ok(())
}

/// Show a file selected in Finder / Explorer.
pub fn reveal(path: &Path) -> crate::Result<()> {
    #[cfg(target_os = "macos")]
    {
        command("open").arg("-R").arg(path).spawn()?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // explorer needs the raw argument: /select,"C:\path with spaces\file"
        Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        command("xdg-open").arg(path.parent().unwrap_or(path)).spawn()?;
    }
    Ok(())
}
