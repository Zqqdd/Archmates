//! Windows install from an unzipped release, and repair of v0.4.0 installs.

use std::path::PathBuf;

use archmates::platform::windows::install as win;
use archmates::{platform, update};

/// Started from an unzipped release (not from the install folder): installs the programs
/// found next to it, then returns the installed app to restart from.
pub fn self_install() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let here = exe.parent()?;
    let dir = platform::install_dir();
    if same_dir(here, &dir) {
        return None;
    }
    let mut programs = Vec::new();
    for name in platform::PROGRAMS {
        if let Ok(bytes) = std::fs::read(here.join(name)) {
            programs.push((name, bytes));
        }
    }
    let borrowed: Vec<(&str, &[u8])> = programs.iter().map(|(n, b)| (*n, b.as_slice())).collect();
    win::install(&borrowed, false, true).ok()?;
    Some(win::app())
}

fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Installs made by v0.4.0 have no WinmatesUpdater: fetch this version again to get it,
/// and move "start with Windows" over to the updater.
pub async fn repair() {
    if win::updater().exists() || !win::app().exists() {
        return;
    }
    let autostart = platform::autostart_enabled();
    if update::install(update::CURRENT).await.is_ok() {
        let _ = win::finish_install(false, autostart);
    }
}
