//! Where Castor keeps things.
//!
//! | what                         | macOS                                          | Windows                                   |
//! |------------------------------|------------------------------------------------|-------------------------------------------|
//! | config (settings, chains)    | ~/Library/Application Support/eu.basedpolymer.Castor | %APPDATA%\basedpolymer\Castor\config |
//! | data (history, signatures)   | same                                           | %APPDATA%\basedpolymer\Castor\data        |
//! | cache (thumbnails)           | ~/Library/Caches/eu.basedpolymer.Castor        | %LOCALAPPDATA%\basedpolymer\Castor\cache  |
//! | downloads (models, sidecars) | data/downloads                                 | %LOCALAPPDATA%\...\data\downloads         |
//!
//! `CASTOR_HOME` overrides everything (portable mode, tests): config/data/cache under it.

use std::path::PathBuf;
use std::sync::OnceLock;

struct Dirs {
    config: PathBuf,
    data: PathBuf,
    local: PathBuf,
    cache: PathBuf,
}

fn dirs() -> &'static Dirs {
    static D: OnceLock<Dirs> = OnceLock::new();
    D.get_or_init(|| {
        if let Some(home) = std::env::var_os("CASTOR_HOME").map(PathBuf::from) {
            return Dirs { config: home.join("config"), data: home.join("data"), local: home.join("data"), cache: home.join("cache") };
        }
        match directories::ProjectDirs::from("eu", "basedpolymer", "Castor") {
            Some(p) => Dirs {
                config: p.config_dir().to_path_buf(),
                data: p.data_dir().to_path_buf(),
                local: p.data_local_dir().to_path_buf(),
                cache: p.cache_dir().to_path_buf(),
            },
            None => {
                let t = std::env::temp_dir().join("castor");
                Dirs { config: t.join("config"), data: t.join("data"), local: t.join("data"), cache: t.join("cache") }
            }
        }
    })
}

pub fn config_dir() -> PathBuf {
    dirs().config.clone()
}
pub fn data_dir() -> PathBuf {
    dirs().data.clone()
}
pub fn cache_dir() -> PathBuf {
    dirs().cache.clone()
}
/// Downloaded models and helper programs (machine-local, not roamed on Windows).
pub fn downloads_dir() -> PathBuf {
    dirs().local.join("downloads")
}
/// Scratch space for runs.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("castor-runs")
}

/// Folder of the running executable's bundled resources:
/// macOS `Castor.app/Contents/Resources`, Windows/Linux the executable's folder,
/// dev builds: `<workspace>/target/<profile>` (bundled sidecars can be dropped there),
/// overridable with `CASTOR_RESOURCES`.
pub fn resources_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("CASTOR_RESOURCES") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    #[cfg(target_os = "macos")]
    {
        if dir.ends_with("Contents/MacOS") {
            return dir.parent().unwrap().join("Resources");
        }
    }
    dir
}

/// Find a helper executable: bundled resources → downloads → PATH.
pub fn find_program(name: &str) -> Option<PathBuf> {
    let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    for base in [resources_dir(), resources_dir().join("bin"), downloads_dir().join("bin")] {
        let p = base.join(&file);
        if p.is_file() {
            return Some(p);
        }
    }
    which::which(name).ok()
}

/// Default folder for results of tools that take no input (QR code, gradients…).
pub fn default_output_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(|d| d.join("Castor")))
        .unwrap_or_else(|| std::env::temp_dir().join("Castor"))
}
