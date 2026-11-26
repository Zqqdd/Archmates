//! WinmatesUpdater: installed in %LOCALAPPDATA%\Programs\Winmates and started by Windows at
//! every login (registry "Run" key). No window:
//!
//!   1. removes programs left aside by the previous update;
//!   2. checks GitHub for a new release (waits up to ~2 min for the network after a reboot)
//!      and installs it: SHA-256 checked, all three programs replaced, itself included;
//!   3. starts the service (`archmates.exe daemon`) on the right version;
//!   4. opens Winmates, unless "Open Winmates when I log in" is off.
//!
//! `winmates-updater --uninstall` (Settings → Apps → Winmates → Uninstall) removes Winmates.

#![cfg_attr(windows, windows_subsystem = "windows")]

use archmates::launcher::{self, UpdateResult};

fn main() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let args: Vec<String> = std::env::args().collect();

    #[cfg(windows)]
    if args.iter().any(|a| a == "--uninstall") {
        let ok = archmates::platform::confirm(
            "Uninstall Winmates",
            "Remove Winmates from this PC?\n\nYour friend code, friends list and conversations are kept \
             (in %APPDATA%\\Winmates), so you can reinstall later without adding your friends again.",
        );
        if ok {
            runtime.block_on(launcher::windows::uninstall(false));
        }
        return;
    }

    #[cfg(windows)]
    archmates::update::cleanup_old_programs();

    let result = runtime.block_on(launcher::update(true));
    match &result {
        UpdateResult::Installed(v) => {
            eprintln!("winmates-updater: installed {v}");
            // Run the new updater: it starts the new service and opens the new app.
            #[cfg(windows)]
            {
                let _ = std::process::Command::new(launcher::windows::updater()).arg("--after-update").spawn();
                return;
            }
        }
        UpdateResult::Failed(e) => eprintln!("winmates-updater: update check failed: {e}"),
        UpdateResult::Available(v) => eprintln!("winmates-updater: {v} is available (archmates update --install)"),
        UpdateResult::UpToDate => {}
    }
    #[cfg(windows)]
    launcher::windows::refresh_version();

    let service = runtime.block_on(launcher::ensure_service());
    if !service {
        eprintln!("winmates-updater: the service did not start");
    }
    if launcher::open_at_login(&archmates::config::Paths::detect()) && !args.iter().any(|a| a == "--no-window") {
        let app = std::env::current_exe().ok().map(|e| e.with_file_name(if cfg!(windows) { "winmates.exe" } else { "winmates" }));
        if let Some(app) = app.filter(|a| a.exists()) {
            let _ = std::process::Command::new(app).arg("--from-updater").spawn();
        }
    }
}
