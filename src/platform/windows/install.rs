//! Installing and uninstalling Winmates, all in your account (no admin rights):
//!   %LOCALAPPDATA%\Programs\Winmates\winmates.exe          the app
//!   %LOCALAPPDATA%\Programs\Winmates\winmates-updater.exe  WinmatesUpdater: runs at every login
//!   %LOCALAPPDATA%\Programs\Winmates\archmates.exe         the service and the command line
//! plus the Start menu shortcut, "start with Windows", and the entry in Settings → Apps.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::reg;
use crate::platform;

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
        super::replace_program(&dir.join(name), bytes)?;
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
    reg::set_string(cu, k, "DisplayVersion", crate::common::update::CURRENT)?;
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
        let _ = reg::set_string(reg::CURRENT_USER, UNINSTALL_KEY, "DisplayVersion", crate::common::update::CURRENT);
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
    let _ = crate::common::ipc::request(&crate::common::ipc::Command::Quit).await;
    let _ = platform::set_autostart(false);
    if let Some(link) = start_menu() {
        let _ = std::fs::remove_file(link);
    }
    let _ = std::fs::remove_file(desktop());
    reg::delete_tree(reg::CURRENT_USER, UNINSTALL_KEY);
    if remove_data {
        let paths = crate::common::config::Paths::detect();
        let _ = std::fs::remove_dir_all(&paths.data);
        let _ = std::fs::remove_dir_all(&paths.cache);
    }
    // The running updater can't delete its own folder: a short-lived cmd does it once we exit.
    let dir = platform::install_dir();
    let _ = platform::command("cmd")
        .args(["/C", &format!("ping 127.0.0.1 -n 3 > nul & rmdir /S /Q \"{}\"", dir.display())])
        .spawn();
}
