//! Linux: notifications with `notify-send`, the systemd user service, XDG folders, and the
//! system details shown in your profile. Works on any desktop; the universal widget
//! supervisor (for desktops without Noctalia) is in `widget.rs`.

mod info;
mod update;
pub mod widget;

use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;

pub use info::{avatar_candidates, system_info};
pub use update::{binary_replaced, install_release, managed_by_pacman, update_hint};

/// A helper command (same as `Command::new` on Linux).
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    Command::new(program)
}

/// True if a process with this exact name is running.
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
    let _ = Command::new("xdg-open").arg(path).spawn();
}

// ── Notifications ───────────────────────────────────────────────────────────

/// Shows a desktop notification. `urgent` makes it stay longer.
pub fn notify(title: &str, body: &str, icon: Option<&str>, urgent: bool) {
    let mut cmd = Command::new("notify-send");
    cmd.args(["-a", "Archmates", "-u", if urgent { "normal" } else { "low" }]);
    if let Some(icon) = icon {
        cmd.args(["-i", icon]);
    }
    let _ = cmd.arg(title).arg(body).spawn();
}

/// Notification with Accept / Decline buttons. Blocks until answered; None = no answer.
pub fn ask(title: &str, body: &str) -> Option<bool> {
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

// ── Start with the system: the systemd user service ─────────────────────────

pub fn autostart_enabled() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-enabled", "archmates.service"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "enabled")
        .unwrap_or(false)
}

pub fn set_autostart(on: bool) -> Result<()> {
    let status = Command::new("systemctl")
        .args(["--user", if on { "enable" } else { "disable" }, "archmates.service"])
        .status()?;
    anyhow::ensure!(status.success(), "systemctl failed ({status})");
    Ok(())
}

/// Starts the systemd user service (the normal way to run the service on Linux).
pub fn start_service() {
    let _ = Command::new("systemctl").args(["--user", "start", "archmates.service"]).status();
}

// ── Folders and usage ───────────────────────────────────────────────────────

/// (data, cache) folders, following XDG: ~/.local/share/archmates and ~/.cache/archmates.
pub fn app_dirs() -> (PathBuf, PathBuf) {
    let home = super::home();
    let data = std::env::var("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".local/share"));
    let cache = std::env::var("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".cache"));
    (data.join("archmates"), cache.join("archmates"))
}

/// Your download folder: XDG_DOWNLOAD_DIR from ~/.config/user-dirs.dirs, else ~/Downloads.
pub fn download_dir() -> PathBuf {
    let home = super::home();
    let config = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".config"));
    let from_file = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|txt| {
        txt.lines().find_map(|l| l.strip_prefix("XDG_DOWNLOAD_DIR=")).map(|v| {
            let v = v.trim().trim_matches('"');
            PathBuf::from(v.replace("$HOME", &home.to_string_lossy()))
        })
    });
    from_file.unwrap_or_else(|| home.join("Downloads"))
}

/// (used, total) RAM in bytes.
pub fn memory_usage() -> Option<(u64, u64)> {
    let txt = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<u64> {
        txt.lines().find(|l| l.starts_with(name))?.split_whitespace().nth(1)?.parse::<u64>().ok()
    };
    let total = field("MemTotal:")? * 1024;
    let available = field("MemAvailable:")? * 1024;
    Some((total.saturating_sub(available), total))
}

/// (used, total) bytes on the root file system.
pub fn disk_usage() -> Option<(u64, u64)> {
    let out = Command::new("df").args(["-B1", "--output=size,used", "/"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut nums = text.lines().last()?.split_whitespace().map(|n| n.parse::<u64>().ok());
    let total = nums.next()??;
    let used = nums.next()??;
    Some((used, total))
}
