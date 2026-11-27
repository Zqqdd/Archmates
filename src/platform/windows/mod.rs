//! Windows (Winmates): toast notifications, the registry "Run" key to start with Windows,
//! programs that never flash a console window, and the system details shown in your profile.
//! The installer / uninstaller is in `install.rs`, updates in `update.rs`.

mod info;
pub mod install;
pub mod reg;
mod update;

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

pub use info::{avatar_candidates, system_info};
pub use update::{
    PROGRAMS, binary_replaced, cleanup_old_programs, install_release, managed_by_pacman, replace_program, update_hint,
};

/// A command that never opens a console window (the service has no console: without this
/// flag every helper program would flash a black window).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Not needed on Windows (only Linux looks for Noctalia or a compositor).
pub fn process_running(_name: &str) -> bool {
    false
}

/// Opens a file or folder with the default program.
pub fn open_path(path: &std::path::Path) {
    let _ = command("explorer").arg(path).spawn();
}

/// Where Winmates installs itself (no admin rights needed).
pub fn install_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| super::home().join("AppData").join("Local"))
        .join("Programs")
        .join("Winmates")
}

// ── Notifications ───────────────────────────────────────────────────────────

fn toast(title: &str, body: &str, icon: Option<&str>) {
    use tauri_winrt_notification::{IconCrop, Toast};
    let mut toast = Toast::new(Toast::POWERSHELL_APP_ID).title(title).text1(body);
    if let Some(icon) = icon {
        toast = toast.icon(std::path::Path::new(icon), IconCrop::Circular, "");
    }
    if let Err(err) = toast.show() {
        eprintln!("notification: {err}");
    }
}

/// Shows a desktop notification (clicking a toast brings up Windows' default handling).
pub fn notify(title: &str, body: &str, icon: Option<&str>, _urgent: bool, _on_click: super::OnClick) {
    toast(title, body, icon);
}

/// Toasts can't wait for an answer here: Winmates shows the Accept / Decline buttons instead.
pub fn ask(title: &str, body: &str, _on_shown: impl FnOnce(u32)) -> Option<bool> {
    toast(title, &format!("{body}\nOpen Winmates to accept or decline."), None);
    None
}

/// Toasts are not closed by id here.
pub fn close_notification(_id: u32) {}

/// A Yes / No question in a standard dialog box (used by the uninstaller).
pub fn confirm(title: &str, text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONQUESTION, MB_YESNO, MessageBoxW};
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (t, c) = (wide(text), wide(title));
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), MB_YESNO | MB_ICONQUESTION) == IDYES }
}

// ── Start with Windows: WinmatesUpdater in the "Run" registry key ───────────

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn autostart_enabled() -> bool {
    reg::get_string(reg::CURRENT_USER, RUN_KEY, "WinmatesUpdater").is_some()
        || reg::get_string(reg::CURRENT_USER, RUN_KEY, "Winmates").is_some()
}

/// WinmatesUpdater starts at every login: it updates, starts the service, then opens Winmates.
pub fn set_autostart(on: bool) -> Result<()> {
    // v0.4.0 started the app itself ("Winmates"): replaced by the updater.
    reg::delete_value(reg::CURRENT_USER, RUN_KEY, "Winmates");
    if on {
        let updater = install_dir().join(install::UPDATER);
        reg::set_string(reg::CURRENT_USER, RUN_KEY, "WinmatesUpdater", &format!("\"{}\"", updater.display()))
    } else {
        reg::delete_value(reg::CURRENT_USER, RUN_KEY, "WinmatesUpdater");
        Ok(())
    }
}

/// No system service manager on Windows: Winmates starts `archmates.exe daemon` itself.
pub fn start_service() {}

// ── Folders and usage ───────────────────────────────────────────────────────

/// (data, cache) folders: %APPDATA%\Winmates (follows your account) and %LOCALAPPDATA%\Winmates.
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let home = super::home();
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming"));
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Local"));
    (roaming.join("Winmates"), local.join("Winmates"))
}

/// The Downloads folder set in Explorer, else %USERPROFILE%\Downloads.
pub fn download_dir() -> PathBuf {
    const SHELL_FOLDERS: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
    let home = super::home();
    // Known folder id of "Downloads"; the value may contain %USERPROFILE%.
    reg::get_string(reg::CURRENT_USER, SHELL_FOLDERS, "{374DE290-123F-4565-9164-39C4925E467B}")
        .map(|v| PathBuf::from(v.replace("%USERPROFILE%", &home.to_string_lossy())))
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join("Downloads"))
}

/// (used, total) RAM in bytes.
pub fn memory_usage() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        return None;
    }
    Some((status.ullTotalPhys.saturating_sub(status.ullAvailPhys), status.ullTotalPhys))
}

/// (used, total) bytes on the system drive (usually C:).
pub fn disk_usage() -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
    let root: Vec<u16> = format!("{drive}\\").encode_utf16().chain(std::iter::once(0)).collect();
    let (mut free, mut total) = (0u64, 0u64);
    if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), std::ptr::null_mut(), &mut total, &mut free) } == 0 {
        return None;
    }
    Some((total.saturating_sub(free), total))
}
