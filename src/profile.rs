//! Your profile, as sent to your friends.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Maximum size of a profile picture sent or accepted.
pub const AVATAR_MAX: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profile {
    pub user: String,
    pub host: String,
    pub distro: String,
    pub kernel: String,
    pub wm: String,
    pub terminal: String,
    pub shell: String,
    /// Seconds since the machine booted.
    pub uptime_s: u64,
    /// Fingerprint of the picture: a friend only downloads it again when it changes.
    pub avatar_hash: Option<String>,
    pub version: String,
}

fn read(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn os_release_field(key: &str) -> Option<String> {
    let txt = fs::read_to_string("/etc/os-release").ok()?;
    txt.lines()
        .find_map(|l| l.strip_prefix(&format!("{key}=")))
        .map(|v| v.trim_matches('"').to_string())
}

fn login_shell(user: &str) -> Option<String> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    let line = passwd.lines().find(|l| l.split(':').next() == Some(user))?;
    let sh = line.rsplit(':').next()?;
    Some(sh.rsplit('/').next().unwrap_or(sh).to_string())
}

/// First known terminal that is installed (when $TERMINAL is unset, e.g. under systemd).
fn installed_terminal() -> String {
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin".into());
    ["kitty", "alacritty", "foot", "wezterm", "ghostty", "konsole", "gnome-terminal"]
        .iter()
        .find(|t| path.split(':').any(|d| std::path::Path::new(d).join(t).exists()))
        .map(|t| t.to_string())
        .unwrap_or_else(|| "?".into())
}

/// Profile picture: $ARCHMATES_AVATAR, else the system account picture, else ~/.face.
pub fn avatar_path(user: &str) -> Option<PathBuf> {
    let candidates = [
        std::env::var("ARCHMATES_AVATAR").ok().map(PathBuf::from),
        Some(PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".face")),
    ];
    candidates.into_iter().flatten().find(|p| {
        fs::metadata(p).map(|m| m.is_file() && (m.len() as usize) <= AVATAR_MAX).unwrap_or(false)
    })
}

pub fn read_avatar(user: &str) -> Option<Vec<u8>> {
    avatar_path(user).and_then(|p| fs::read(p).ok())
}

/// 64-bit FNV-1a fingerprint: simple, stable, good enough to detect a change.
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

impl Profile {
    pub fn local() -> Self {
        let user = std::env::var("ARCHMATES_USER")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "?".into());
        let kernel = read("/proc/sys/kernel/osrelease").unwrap_or_default();
        let kernel = kernel.strip_suffix("-cachyos").unwrap_or(&kernel).to_string();
        let uptime_s = read("/proc/uptime")
            .and_then(|t| t.split('.').next().and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        Self {
            host: read("/etc/hostname").unwrap_or_else(|| "linux".into()),
            distro: os_release_field("NAME").unwrap_or_else(|| "Linux".into()),
            kernel,
            wm: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into()),
            terminal: std::env::var("TERMINAL").ok().unwrap_or_else(installed_terminal),
            shell: login_shell(&user).unwrap_or_else(|| "?".into()),
            uptime_s,
            avatar_hash: read_avatar(&user).map(|b| fingerprint(&b)),
            version: env!("CARGO_PKG_VERSION").into(),
            user,
        }
    }
}
