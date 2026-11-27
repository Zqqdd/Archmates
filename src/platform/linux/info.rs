//! What your profile says about a Linux system: distro, kernel, compositor, terminal, shell,
//! and where your account picture is.

use std::fs;
use std::path::PathBuf;

use crate::platform::SystemInfo;

fn read(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn os_release_field(key: &str) -> Option<String> {
    let txt = fs::read_to_string("/etc/os-release").ok()?;
    txt.lines().find_map(|l| l.strip_prefix(&format!("{key}="))).map(|v| v.trim_matches('"').to_string())
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

/// Desktop or compositor in use. The service may start before the desktop (and without its
/// variables), so fall back to looking for a known compositor among running processes.
fn window_manager() -> String {
    if let Ok(wm) = std::env::var("XDG_CURRENT_DESKTOP") {
        if !wm.is_empty() {
            return wm;
        }
    }
    const KNOWN: [(&str, &str); 9] = [
        ("Hyprland", "Hyprland"),
        ("sway", "sway"),
        ("niri", "niri"),
        ("river", "river"),
        ("labwc", "labwc"),
        ("wayfire", "Wayfire"),
        ("kwin_wayland", "KDE"),
        ("kwin_x11", "KDE"),
        ("gnome-shell", "GNOME"),
    ];
    KNOWN
        .iter()
        .find(|(process, _)| super::process_running(process))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| "?".into())
}

pub fn system_info(user: &str) -> SystemInfo {
    let kernel = read("/proc/sys/kernel/osrelease").unwrap_or_default();
    SystemInfo {
        host: read("/etc/hostname").unwrap_or_else(|| "linux".into()),
        distro: os_release_field("NAME").unwrap_or_else(|| "Linux".into()),
        kernel: kernel.strip_suffix("-cachyos").unwrap_or(&kernel).to_string(),
        wm: window_manager(),
        terminal: std::env::var("TERMINAL").ok().unwrap_or_else(installed_terminal),
        shell: login_shell(user).unwrap_or_else(|| "?".into()),
        uptime_s: read("/proc/uptime").and_then(|t| t.split('.').next().and_then(|s| s.parse().ok())).unwrap_or(0),
    }
}

/// Picture set in the system account settings (KDE, GNOME, Noctalia… all use AccountsService).
/// Asked over D-Bus because its config folder is not readable by users.
fn accounts_service_icon(user: &str) -> Option<PathBuf> {
    fn quoted(output: &[u8]) -> Option<String> {
        let text = String::from_utf8_lossy(output);
        let start = text.find('"')? + 1;
        let end = start + text[start..].find('"')?;
        Some(text[start..end].to_string())
    }
    let found = std::process::Command::new("busctl")
        .args([
            "--system",
            "call",
            "org.freedesktop.Accounts",
            "/org/freedesktop/Accounts",
            "org.freedesktop.Accounts",
            "FindUserByName",
            "s",
            user,
        ])
        .output()
        .ok()?;
    let object = quoted(&found.stdout)?;
    let icon = std::process::Command::new("busctl")
        .args([
            "--system",
            "get-property",
            "org.freedesktop.Accounts",
            &object,
            "org.freedesktop.Accounts.User",
            "IconFile",
        ])
        .output()
        .ok()?;
    quoted(&icon.stdout).filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// System places a profile picture can be, in order of preference: AccountsService,
/// ~/.face, ~/.face.icon, the SDDM face.
pub fn avatar_candidates(user: &str) -> Vec<PathBuf> {
    let home = std::env::var("HOME").map(PathBuf::from).ok();
    [
        accounts_service_icon(user),
        Some(PathBuf::from(format!("/var/lib/AccountsService/icons/{user}"))),
        home.as_ref().map(|h| h.join(".face")),
        home.as_ref().map(|h| h.join(".face.icon")),
        Some(PathBuf::from(format!("/usr/share/sddm/faces/{user}.face.icon"))),
    ]
    .into_iter()
    .flatten()
    .collect()
}
