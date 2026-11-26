//! Local files: secret key, friends list, invisible mode, state for the widget.
//!
//! Everything follows the XDG conventions:
//!   ~/.local/share/archmates/secret.key     your identity (32 bytes, readable by you only)
//!   ~/.local/share/archmates/friends.toml   your friends list
//!   ~/.local/share/archmates/invisible      present = invisible mode
//!   ~/.local/share/archmates/avatar         your profile picture (set with `archmates avatar`)
//!   ~/.local/share/archmates/widget         universal widget mode: auto, on or off
//!   ~/.cache/archmates/state.json           friends' status, read by the Noctalia widget
//!   ~/.cache/archmates/avatars/             friends' profile pictures
//!
//! On Windows (Winmates): %APPDATA%\Winmates (data) and %LOCALAPPDATA%\Winmates (cache).
//!
//! The ARCHMATES_HOME variable replaces both folders (handy to test several
//! instances on the same machine).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

pub struct Paths {
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn detect() -> Self {
        if let Ok(base) = std::env::var("ARCHMATES_HOME") {
            let base = PathBuf::from(base);
            return Self { data: base.join("data"), cache: base.join("cache") };
        }
        #[cfg(windows)]
        {
            let home = crate::platform::home();
            let roaming = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Roaming"));
            let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Local"));
            return Self { data: roaming.join("Winmates"), cache: local.join("Winmates") };
        }
        #[allow(unreachable_code)]
        let home = crate::platform::home();
        let data = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".local/share"))
            .join("archmates");
        let cache = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".cache"))
            .join("archmates");
        Self { data, cache }
    }

    pub fn secret_key(&self) -> PathBuf { self.data.join("secret.key") }
    pub fn friends(&self) -> PathBuf { self.data.join("friends.toml") }
    pub fn invisible(&self) -> PathBuf { self.data.join("invisible") }
    pub fn widget_mode(&self) -> PathBuf { self.data.join("widget") }
    pub fn avatar(&self) -> PathBuf { self.data.join("avatar") }
    pub fn state(&self) -> PathBuf { self.cache.join("state.json") }
    pub fn avatars(&self) -> PathBuf { self.cache.join("avatars") }

    pub fn is_invisible(&self) -> bool { self.invisible().exists() }
}

/// Loads the secret key, or creates one on first run.
pub fn load_or_create_key(paths: &Paths) -> Result<SecretKey> {
    let p = paths.secret_key();
    if p.exists() {
        let bytes = fs::read(&p).with_context(|| format!("reading {}", p.display()))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .context("corrupted secret key (not 32 bytes long)")?;
        return Ok(SecretKey::from_bytes(&arr));
    }
    fs::create_dir_all(&paths.data)?;
    let key = SecretKey::generate();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    // Readable by you only. (On Windows, files in your profile folder already are.)
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        fs::set_permissions(&paths.data, fs::Permissions::from_mode(0o700))?;
        options.mode(0o600);
    }
    let mut f = options.open(&p)?;
    f.write_all(&key.to_bytes())?;
    Ok(key)
}

// ── Friends list ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Friend {
    pub nickname: String,
    pub code: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct FriendList {
    #[serde(default, rename = "friend")]
    pub friends: Vec<Friend>,
}

impl FriendList {
    pub fn load(paths: &Paths) -> Result<Self> {
        let p = paths.friends();
        if !p.exists() {
            return Ok(Self::default());
        }
        let txt = fs::read_to_string(&p)?;
        toml::from_str(&txt).with_context(|| format!("{} is malformed", p.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::create_dir_all(&paths.data)?;
        let txt = format!(
            "# Your friends list (edit by hand, or with `archmates add` / `archmates remove`)\n\n{}",
            toml::to_string_pretty(self)?
        );
        write_atomic(&paths.friends(), txt.as_bytes())
    }

    /// Friends with a valid code, indexed by endpoint id.
    pub fn by_id(&self) -> BTreeMap<EndpointId, Friend> {
        self.friends
            .iter()
            .filter_map(|f| f.code.parse::<EndpointId>().ok().map(|id| (id, f.clone())))
            .collect()
    }

    pub fn add(&mut self, code: &str, nickname: &str) -> Result<()> {
        let id: EndpointId = code.trim().parse().context("this friend code is not valid")?;
        if self.by_id().contains_key(&id) {
            bail!("this friend is already in your list");
        }
        self.friends.push(Friend { nickname: nickname.to_string(), code: id.to_string() });
        Ok(())
    }

    pub fn remove(&mut self, who: &str) -> bool {
        let before = self.friends.len();
        self.friends.retain(|f| f.nickname != who && f.code != who);
        self.friends.len() != before
    }
}

/// Writes to a temporary file then renames it: the widget never reads a half-written file.
pub fn write_atomic(p: &Path, content: &[u8]) -> Result<()> {
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, p)?;
    Ok(())
}
