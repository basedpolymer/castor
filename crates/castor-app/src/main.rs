//! Castor — the desktop app.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![recursion_limit = "512"]

mod app;
mod assets;
mod chains;
mod cli;
#[cfg(feature = "demo-tools")]
mod demo;
mod form;
mod home;
mod ipc;
mod model;
mod palette;
mod panel;
mod runs;
mod search;
mod settings_page;
mod shell;
mod theme;
mod thumbs;
mod tool_screen;
mod tray;
mod ui;
mod updates;

#[cfg(test)]
mod tests;

fn main() {
    // `castor cli <args>` and `castor mcp`: the command line and the MCP server share this binary,
    // so shipped builds carry the engines once. Handled before anything GUI-related.
    #[cfg(feature = "tools")]
    {
        let mut it = std::env::args_os().skip(1);
        if let Some(mode) = it.next().filter(|m| m == "cli" || m == "mcp") {
            #[cfg(windows)]
            console::attach_parent_if_needed();
            let mut args = vec![std::ffi::OsString::from("castor-cli")];
            if mode == "mcp" {
                args.push("mcp".into());
            }
            args.extend(it);
            std::process::exit(castor_cli::run(args));
        }
    }

    // Velopack must run first: on install/update/uninstall hooks it does its work and exits.
    velopack::VelopackApp::build().run();

    let args = cli::Args::parse(std::env::args().skip(1));
    let screenshot = args.page.is_some();

    // Single instance: a second launch forwards its files to the first one and leaves.
    if !screenshot && ipc::forward_to_running(&args) {
        return;
    }

    init_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Castor starting");
    app::run(args);
}

/// Log to `{data_dir}/logs/castor.log` (truncated when it grows past 5 MB) and to stderr.
fn init_logging() {
    use tracing_subscriber::prelude::*;
    let dir = castor_core::paths::data_dir().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("castor.log");
    if std::fs::metadata(&path).map(|m| m.len() > 5_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("castor.old.log"));
    }
    let filter = tracing_subscriber::EnvFilter::try_from_env("CASTOR_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,naga=warn,wgpu=warn"));
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
    let file_layer = file.map(|f| {
        tracing_subscriber::fmt::layer().with_ansi(false).with_writer(std::sync::Mutex::new(f))
    });
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .try_init();
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("panic: {info}");
    }));
}

/// Release builds on Windows use the GUI subsystem: started from a terminal as `castor cli …`,
/// they have no stdout. Attach to the parent console then, unless stdio was inherited (pipes,
/// the `castor-cli` launcher, an MCP client), in which case there is nothing to do.
#[cfg(all(windows, feature = "tools"))]
mod console {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        SetStdHandle,
    };

    pub fn attach_parent_if_needed() {
        // SAFETY: plain Win32 calls on handles we own; failures leave the process unchanged.
        unsafe {
            let out = GetStdHandle(STD_OUTPUT_HANDLE);
            if !out.is_null() && out != INVALID_HANDLE_VALUE {
                return;
            }
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                return;
            }
            let open = |name: &str| {
                let w: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                CreateFileW(
                    w.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            let o = open("CONOUT$");
            if o != INVALID_HANDLE_VALUE {
                SetStdHandle(STD_OUTPUT_HANDLE, o);
                SetStdHandle(STD_ERROR_HANDLE, o);
            }
            let i = open("CONIN$");
            if i != INVALID_HANDLE_VALUE {
                SetStdHandle(STD_INPUT_HANDLE, i);
            }
        }
    }
}
