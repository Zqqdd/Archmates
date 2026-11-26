//! What differs between Linux and Windows: home folder, notifications, starting with the
//! system, running helper programs without flashing a console window.
//!
//! Everything else (protocol, friends list, chat, files) is shared, so both systems work together.

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

/// Your home folder ($HOME on Linux, %USERPROFILE% on Windows).
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Your user name.
pub fn user_name() -> String {
    std::env::var("ARCHMATES_USER")
        .or_else(|_| std::env::var("USER"))
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "?".into())
}

/// A command that never opens a console window (on Windows, the service has no console:
/// without this flag every helper program would flash a black window).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// True if a process with this exact name is running (Linux only; false elsewhere).
pub fn process_running(name: &str) -> bool {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .any(|c| c.trim() == name)
}

/// Opens a file or folder with the default program.
pub fn open_path(path: &std::path::Path) {
    #[cfg(windows)]
    let _ = command("explorer").arg(path).spawn();
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(path).spawn();
}

// ── Notifications ───────────────────────────────────────────────────────────

/// Shows a desktop notification. `urgent` makes it stay longer where supported.
pub fn notify(title: &str, body: &str, icon: Option<&str>, urgent: bool) {
    #[cfg(windows)]
    {
        let _ = urgent;
        windows_toast(title, body, icon);
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("notify-send");
        cmd.args(["-a", "Archmates", "-u", if urgent { "normal" } else { "low" }]);
        if let Some(icon) = icon {
            cmd.args(["-i", icon]);
        }
        let _ = cmd.arg(title).arg(body).spawn();
    }
}

/// Notification with Accept / Decline buttons. Blocks until answered; None = no answer
/// (or buttons not supported: on Windows, Winmates shows the buttons instead).
pub fn ask(title: &str, body: &str) -> Option<bool> {
    #[cfg(windows)]
    {
        windows_toast(title, &format!("{body}\nOpen Winmates to accept or decline."), None);
        None
    }
    #[cfg(not(windows))]
    {
        let out = Command::new("notify-send")
            .args(["-a", "Archmates", "-u", "critical", "--wait", "--action=accept=Accept", "--action=decline=Decline"])
            .arg(title)
            .arg(body)
            .output()
            .ok()?;
        match String::from_utf8_lossy(&out.stdout).trim() {
            "accept" => Some(true),
            "decline" => Some(false),
            _ => None,
        }
    }
}

#[cfg(windows)]
fn windows_toast(title: &str, body: &str, icon: Option<&str>) {
    use tauri_winrt_notification::{IconCrop, Toast};
    let mut toast = Toast::new(Toast::POWERSHELL_APP_ID).title(title).text1(body);
    if let Some(icon) = icon {
        toast = toast.icon(std::path::Path::new(icon), IconCrop::Circular, "");
    }
    if let Err(err) = toast.show() {
        eprintln!("notification: {err}");
    }
}

// ── Start with the system ───────────────────────────────────────────────────

/// Windows: where Winmates installs itself (no admin rights needed).
#[cfg(windows)]
pub fn install_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join("AppData").join("Local"))
        .join("Programs")
        .join("Winmates")
}

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// True when Archmates (Winmates on Windows) starts by itself when you log in.
pub fn autostart_enabled() -> bool {
    #[cfg(windows)]
    {
        reg::get_string(reg::CURRENT_USER, RUN_KEY, "Winmates").is_some()
    }
    #[cfg(not(windows))]
    {
        Command::new("systemctl")
            .args(["--user", "is-enabled", "archmates.service"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
            .unwrap_or(false)
    }
}

/// Turns starting with the system on or off.
/// Windows: Winmates (in the background) from the "Run" registry key of your account.
/// Linux: the systemd user service.
pub fn set_autostart(on: bool) -> Result<()> {
    #[cfg(windows)]
    {
        if on {
            let app = install_dir().join("winmates.exe");
            let app = if app.exists() { app } else { std::env::current_exe()?.with_file_name("winmates.exe") };
            reg::set_string(reg::CURRENT_USER, RUN_KEY, "Winmates", &format!("\"{}\" --background", app.display()))
        } else {
            reg::delete_value(reg::CURRENT_USER, RUN_KEY, "Winmates");
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        let status = Command::new("systemctl")
            .args(["--user", if on { "enable" } else { "disable" }, "archmates.service"])
            .status()?;
        anyhow::ensure!(status.success(), "systemctl failed ({status})");
        Ok(())
    }
}

// ── Windows registry ────────────────────────────────────────────────────────

#[cfg(windows)]
pub mod reg {
    use windows_sys::Win32::Foundation::ERROR_SUCCESS;
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegDeleteKeyValueW,
        RegGetValueW, RegSetKeyValueW,
    };

    pub const CURRENT_USER: HKEY = HKEY_CURRENT_USER;
    pub const LOCAL_MACHINE: HKEY = HKEY_LOCAL_MACHINE;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn get_string(root: HKEY, key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let err = unsafe {
            RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr().cast(), &mut size)
        };
        if err != ERROR_SUCCESS {
            return None;
        }
        let len = (size as usize / 2).min(buf.len());
        let text = String::from_utf16_lossy(&buf[..len]);
        Some(text.trim_end_matches('\0').to_string())
    }

    pub fn get_dword(root: HKEY, key: &str, value: &str) -> Option<u32> {
        let (key, value) = (wide(key), wide(value));
        let mut out = 0u32;
        let mut size = 4u32;
        let err = unsafe {
            RegGetValueW(root, key.as_ptr(), value.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut out as *mut u32).cast(), &mut size)
        };
        (err == ERROR_SUCCESS).then_some(out)
    }

    pub fn set_string(root: HKEY, key: &str, value: &str, data: &str) -> anyhow::Result<()> {
        let (key, value, data) = (wide(key), wide(value), wide(data));
        let err = unsafe {
            RegSetKeyValueW(root, key.as_ptr(), value.as_ptr(), REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
        };
        anyhow::ensure!(err == ERROR_SUCCESS, "could not write the registry (error {err})");
        Ok(())
    }

    pub fn delete_value(root: HKEY, key: &str, value: &str) {
        let (key, value) = (wide(key), wide(value));
        unsafe {
            RegDeleteKeyValueW(root, key.as_ptr(), value.as_ptr());
        }
    }
}
