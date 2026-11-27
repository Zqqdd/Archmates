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
    pub stats: Option<crate::common::stats::Stats>,
}

/// Largest source picture we accept (it is shrunk before being sent).
const SOURCE_MAX: u64 = 30 * 1024 * 1024;
/// Size of the picture sent to friends (square, in pixels).
const AVATAR_SIZE: u32 = 256;

/// Every place a profile picture can be, in order of preference: $ARCHMATES_AVATAR, the one
/// set with `archmates avatar`, then the system ones (account picture…, see `platform`).
fn avatar_candidates(user: &str) -> Vec<PathBuf> {
    [std::env::var("ARCHMATES_AVATAR").ok().map(PathBuf::from), Some(crate::common::config::Paths::detect().avatar())]
        .into_iter()
        .flatten()
        .chain(crate::platform::avatar_candidates(user))
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
    let found = avatar_candidates(user)
        .into_iter()
        .find(|p| fs::metadata(p).map(|m| m.is_file() && m.len() > 0 && m.len() <= SOURCE_MAX).unwrap_or(false));
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
    let cache_dir = crate::common::config::Paths::detect().cache;
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
    let _ = crate::common::config::write_atomic(&cached, &small);
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
    /// Your profile right now: system details, picture fingerprint, version, usage.
    pub fn local() -> Self {
        let user = crate::platform::user_name();
        let system = crate::platform::system_info(&user);
        Self {
            host: system.host,
            distro: system.distro,
            kernel: system.kernel,
            wm: system.wm,
            terminal: system.terminal,
            shell: system.shell,
            uptime_s: system.uptime_s,
            avatar_hash: read_avatar(&user).map(|b| fingerprint(&b)),
            version: env!("CARGO_PKG_VERSION").into(),
            stats: crate::common::stats::current(&crate::common::config::Paths::detect()),
            user,
        }
    }
}
