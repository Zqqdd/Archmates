//! Starting and updating the service from an app: shared by Winmates (`winmates`), the
//! background updater (`winmates-updater`) and the installer (`winmates-setup`).
//! The Windows install / uninstall steps are in `platform::windows::install`.
//!
//! Windows layout, all in your account (no admin rights):
//!   %LOCALAPPDATA%\Programs\Winmates\winmates.exe          the app
//!   %LOCALAPPDATA%\Programs\Winmates\winmates-updater.exe  WinmatesUpdater: runs at every login
//!   %LOCALAPPDATA%\Programs\Winmates\archmates.exe         the service and the command line
//!
//! At every login Windows starts WinmatesUpdater (registry "Run" key). It installs a new
//! release if there is one (SHA-256 checked), starts the service, then opens Winmates if
//! "Open Winmates when I log in" is on.

use std::path::PathBuf;
use std::time::Duration;

use crate::common::{config::Paths, ipc, update};
use crate::platform;

/// The `archmates` command next to the running program (service + command line).
pub fn cli_program() -> PathBuf {
    let name = if cfg!(windows) { "archmates.exe" } else { "archmates" };
    let beside = std::env::current_exe().ok().map(|e| e.with_file_name(name));
    match beside {
        Some(p) if p.exists() => p,
        _ => PathBuf::from(name),
    }
}

/// Checks GitHub; on Windows installs a newer release. Returns the new version when one was
/// installed (the caller restarts), or the available one on Linux in `Err`-free `Offer`.
pub enum UpdateResult {
    UpToDate,
    /// Installed (Windows): restart to use it.
    Installed(String),
    /// Available but not installed by itself (Linux: `archmates update --install`).
    Available(String),
    Failed(String),
}

pub async fn update(retry_offline: bool) -> UpdateResult {
    if std::env::var_os("ARCHMATES_NO_UPDATE_CHECK").is_some() {
        return UpdateResult::UpToDate;
    }
    // Right after login the network may not be up yet: try again for about two minutes.
    let tries = if retry_offline { 8 } else { 1 };
    let mut latest = None;
    for attempt in 0..tries {
        match tokio::time::timeout(Duration::from_secs(15), update::latest_version()).await {
            Ok(Ok(v)) => {
                latest = Some(v);
                break;
            }
            Ok(Err(e)) if attempt + 1 == tries => return UpdateResult::Failed(format!("{e:#}")),
            _ => tokio::time::sleep(Duration::from_secs(15)).await,
        }
    }
    let Some(latest) = latest else { return UpdateResult::Failed("GitHub unreachable".into()) };
    if !update::is_newer(&latest, update::CURRENT) {
        return UpdateResult::UpToDate;
    }
    if !cfg!(windows) {
        return UpdateResult::Available(latest);
    }
    match update::install(&latest).await {
        Ok(()) => {
            // Stop the old service; the new version starts its own.
            let _ = ipc::request(&ipc::Command::Quit).await;
            tokio::time::sleep(Duration::from_millis(600)).await;
            UpdateResult::Installed(latest)
        }
        Err(e) => UpdateResult::Failed(format!("{e:#}")),
    }
}

/// Makes sure the service runs, on this version. Returns true when it answers.
pub async fn ensure_service() -> bool {
    if let Ok(answer) = ipc::request(&ipc::Command::Ping).await {
        if answer.message == update::CURRENT || !cfg!(windows) {
            return true;
        }
        // Windows: an older service is still running after an update.
        let _ = ipc::request(&ipc::Command::Quit).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    #[cfg(not(windows))]
    if std::env::var_os("ARCHMATES_HOME").is_none() {
        // Linux: the systemd user service is the normal way (not for test instances).
        platform::start_service();
        tokio::time::sleep(Duration::from_secs(2)).await;
        if ipc::request(&ipc::Command::Ping).await.is_ok() {
            return true;
        }
    }
    let log = Paths::detect().cache.join("service.log");
    let _ = std::fs::create_dir_all(log.parent().unwrap());
    let mut cmd = platform::command(cli_program());
    cmd.arg("daemon");
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        if let Ok(copy) = file.try_clone() {
            cmd.stdout(copy);
        }
        cmd.stderr(file);
    }
    let started = cmd.spawn().is_ok();
    tokio::time::sleep(Duration::from_secs(2)).await;
    started && ipc::request(&ipc::Command::Ping).await.is_ok()
}

/// "Open Winmates when I log in" (on unless turned off).
pub fn open_at_login(paths: &Paths) -> bool {
    !paths.data.join("no-open-at-login").exists()
}

pub fn set_open_at_login(paths: &Paths, on: bool) {
    let flag = paths.data.join("no-open-at-login");
    if on {
        let _ = std::fs::remove_file(flag);
    } else {
        let _ = std::fs::create_dir_all(&paths.data);
        let _ = std::fs::write(flag, b"");
    }
}
