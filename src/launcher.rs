//! Starting Winmates: update, service, install, uninstall. Shared by the app (`winmates`),
//! the background updater (`winmates-updater`) and the installer (`winmates-setup`).
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

use crate::{config::Paths, ipc, platform, update};

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
        let _ = std::process::Command::new("systemctl").args(["--user", "start", "archmates.service"]).status();
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

// ── Windows install / uninstall ─────────────────────────────────────────────

#[cfg(windows)]
pub mod windows {
    use std::path::{Path, PathBuf};

    use anyhow::Result;

    use crate::platform::{self, reg};

    pub const APP: &str = "winmates.exe";
    pub const UPDATER: &str = "winmates-updater.exe";
    pub const CLI: &str = "archmates.exe";
    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Winmates";

    pub fn app() -> PathBuf {
        platform::install_dir().join(APP)
    }

    pub fn updater() -> PathBuf {
        platform::install_dir().join(UPDATER)
    }

    fn start_menu() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(r"Microsoft\Windows\Start Menu\Programs\Winmates.lnk"))
    }

    fn desktop() -> PathBuf {
        platform::home().join(r"Desktop\Winmates.lnk")
    }

    /// Puts the programs in place and registers everything. `programs` = (file name, bytes).
    pub fn install(programs: &[(&str, &[u8])], desktop_shortcut: bool, autostart: bool) -> Result<()> {
        let dir = platform::install_dir();
        std::fs::create_dir_all(&dir)?;
        for (name, bytes) in programs {
            crate::update::replace_program(&dir.join(name), bytes)?;
        }
        finish_install(desktop_shortcut, autostart)
    }

    /// Shortcuts, "start with Windows", and the entry in Settings → Apps (to uninstall).
    pub fn finish_install(desktop_shortcut: bool, autostart: bool) -> Result<()> {
        let app = app();
        if let Some(link) = start_menu() {
            shortcut(&link, &app);
        }
        if desktop_shortcut {
            shortcut(&desktop(), &app);
        }
        platform::set_autostart(autostart)?;
        let k = UNINSTALL_KEY;
        let cu = reg::CURRENT_USER;
        reg::set_string(cu, k, "DisplayName", "Winmates")?;
        reg::set_string(cu, k, "DisplayVersion", crate::update::CURRENT)?;
        reg::set_string(cu, k, "Publisher", "Archmates")?;
        reg::set_string(cu, k, "DisplayIcon", &app.display().to_string())?;
        reg::set_string(cu, k, "InstallLocation", &platform::install_dir().display().to_string())?;
        reg::set_string(cu, k, "UninstallString", &format!("\"{}\" --uninstall", updater().display()))?;
        reg::set_string(cu, k, "URLInfoAbout", "https://github.com/Zqqdd/Archmates")?;
        reg::set_dword(cu, k, "NoModify", 1)?;
        reg::set_dword(cu, k, "NoRepair", 1)?;
        Ok(())
    }

    /// Keeps the version shown in Settings → Apps right after an update.
    pub fn refresh_version() {
        if reg::get_string(reg::CURRENT_USER, UNINSTALL_KEY, "DisplayName").is_some() {
            let _ = reg::set_string(reg::CURRENT_USER, UNINSTALL_KEY, "DisplayVersion", crate::update::CURRENT);
        }
    }

    fn shortcut(link: &Path, target: &Path) {
        let quote = |p: &Path| p.display().to_string().replace('\'', "''");
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.IconLocation='{},0';$s.Save()",
            quote(link),
            quote(target),
            quote(target.parent().unwrap_or(target)),
            quote(target),
        );
        let _ = platform::command("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status();
    }

    /// Removes Winmates. Your friends list and conversations (%APPDATA%\Winmates) stay,
    /// unless `remove_data`.
    pub async fn uninstall(remove_data: bool) {
        let _ = crate::ipc::request(&crate::ipc::Command::Quit).await;
        let _ = platform::set_autostart(false);
        if let Some(link) = start_menu() {
            let _ = std::fs::remove_file(link);
        }
        let _ = std::fs::remove_file(desktop());
        reg::delete_tree(reg::CURRENT_USER, UNINSTALL_KEY);
        if remove_data {
            let paths = crate::config::Paths::detect();
            let _ = std::fs::remove_dir_all(&paths.data);
            let _ = std::fs::remove_dir_all(&paths.cache);
        }
        // The running updater can't delete its own folder: a short-lived cmd does it once we exit.
        let dir = platform::install_dir();
        let _ = platform::command("cmd")
            .args(["/C", &format!("ping 127.0.0.1 -n 3 > nul & rmdir /S /Q \"{}\"", dir.display())])
            .spawn();
    }
}
