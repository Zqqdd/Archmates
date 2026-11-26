//! Your profile, as sent to your friends.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    /// RAM and storage usage (absent when the friend turned sharing off, or on older versions).
    #[serde(default)]
    pub stats: Option<crate::stats::Stats>,
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
    let running: Vec<String> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| fs::read_to_string(e.path().join("comm")).ok())
        .map(|c| c.trim().to_string())
        .collect();
    KNOWN
        .iter()
        .find(|(process, _)| running.iter().any(|r| r == process))
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| "?".into())
}

/// Largest source picture we accept (it is shrunk before being sent).
const SOURCE_MAX: u64 = 30 * 1024 * 1024;
/// Size of the picture sent to friends (square, in pixels).
const AVATAR_SIZE: u32 = 256;

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
        .args(["--system", "call", "org.freedesktop.Accounts", "/org/freedesktop/Accounts",
               "org.freedesktop.Accounts", "FindUserByName", "s", user])
        .output()
        .ok()?;
    let object = quoted(&found.stdout)?;
    let icon = std::process::Command::new("busctl")
        .args(["--system", "get-property", "org.freedesktop.Accounts", &object,
               "org.freedesktop.Accounts.User", "IconFile"])
        .output()
        .ok()?;
    quoted(&icon.stdout).filter(|p| !p.is_empty()).map(PathBuf::from)
}

/// Every place a profile picture can be, in order of preference.
fn avatar_candidates(user: &str) -> Vec<PathBuf> {
    let home = std::env::var("HOME").map(PathBuf::from).ok();
    [
        std::env::var("ARCHMATES_AVATAR").ok().map(PathBuf::from),
        Some(crate::config::Paths::detect().avatar()),
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

/// Profile picture source, first found: $ARCHMATES_AVATAR, the one set with `archmates avatar`,
/// the system account picture (AccountsService), ~/.face, ~/.face.icon, the SDDM face.
pub fn avatar_path(user: &str) -> Option<PathBuf> {
    // Looked up at most once a minute: it runs on every exchange with a friend.
    static CACHE: Mutex<Option<(Instant, Option<PathBuf>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, found)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(60) {
            return found.clone();
        }
    }
    let found = avatar_candidates(user).into_iter().find(|p| {
        fs::metadata(p).map(|m| m.is_file() && m.len() > 0 && m.len() <= SOURCE_MAX).unwrap_or(false)
    });
    *cache = Some((Instant::now(), found.clone()));
    found
}

/// Image type from its first bytes, if it is one the widgets can show.
pub fn image_kind(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => Some("PNG"),
        [0xFF, 0xD8, 0xFF, ..] => Some("JPEG"),
        [b'G', b'I', b'F', b'8', ..] => Some("GIF"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("WebP"),
        _ => None,
    }
}

/// Crops the picture to a square and shrinks it to AVATAR_SIZE, as PNG.
fn thumbnail(bytes: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.resize_to_fill(AVATAR_SIZE, AVATAR_SIZE, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The picture sent to friends: a small square PNG, cached until the source file changes.
pub fn read_avatar(user: &str) -> Option<Vec<u8>> {
    let source = avatar_path(user)?;
    let meta = fs::metadata(&source).ok()?;
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    let key = fingerprint(format!("{}|{}|{:?}", source.display(), meta.len(), mtime).as_bytes());
    let cache_dir = crate::config::Paths::detect().cache;
    let cached = cache_dir.join(format!("my-avatar-{key}.png"));
    if let Ok(bytes) = fs::read(&cached) {
        return Some(bytes);
    }
    let original = fs::read(&source).ok()?;
    let small = match thumbnail(&original) {
        Some(t) => t,
        // Unknown to the decoder but already small and in a usable format: send as is.
        None if original.len() <= AVATAR_MAX && image_kind(&original).is_some() => original,
        None => return None,
    };
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("my-avatar-") {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let _ = crate::config::write_atomic(&cached, &small);
    Some(small)
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
            wm: window_manager(),
            terminal: std::env::var("TERMINAL").ok().unwrap_or_else(installed_terminal),
            shell: login_shell(&user).unwrap_or_else(|| "?".into()),
            uptime_s,
            avatar_hash: read_avatar(&user).map(|b| fingerprint(&b)),
            version: env!("CARGO_PKG_VERSION").into(),
            stats: crate::stats::current(&crate::config::Paths::detect()),
            user,
        }
    }
}
