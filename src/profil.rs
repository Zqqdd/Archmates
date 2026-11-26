//! Ton profil, tel qu'il est envoyé à tes amis (les mêmes infos que ta carte de bureau).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Taille maximale d'une photo de profil envoyée ou acceptée.
pub const AVATAR_MAX: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Profil {
    pub nom: String,
    pub hote: String,
    pub distro: String,
    pub noyau: String,
    pub wm: String,
    pub terminal: String,
    pub shell: String,
    /// Secondes depuis l'allumage du PC.
    pub allume_depuis_s: u64,
    /// Empreinte de la photo : l'ami ne la retélécharge que si elle change.
    pub avatar_empreinte: Option<String>,
    pub version: String,
}

fn lire(p: &str) -> Option<String> {
    fs::read_to_string(p).ok().map(|s| s.trim().to_string())
}

fn champ_os_release(cle: &str) -> Option<String> {
    let txt = fs::read_to_string("/etc/os-release").ok()?;
    txt.lines()
        .find_map(|l| l.strip_prefix(&format!("{cle}=")))
        .map(|v| v.trim_matches('"').to_string())
}

fn shell_de_connexion(nom: &str) -> Option<String> {
    let passwd = fs::read_to_string("/etc/passwd").ok()?;
    let ligne = passwd.lines().find(|l| l.split(':').next() == Some(nom))?;
    let sh = ligne.rsplit(':').next()?;
    Some(sh.rsplit('/').next().unwrap_or(sh).to_string())
}

/// Premier terminal connu installé (quand $TERMINAL n'est pas défini, ex. sous systemd).
fn terminal_installe() -> String {
    let chemins = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin".into());
    ["kitty", "alacritty", "foot", "wezterm", "ghostty", "konsole", "gnome-terminal"]
        .iter()
        .find(|t| chemins.split(':').any(|d| std::path::Path::new(d).join(t).exists()))
        .map(|t| t.to_string())
        .unwrap_or_else(|| "?".into())
}

/// Photo de profil : celle de Noctalia si réglée, sinon celle du compte système.
pub fn chemin_avatar(nom: &str) -> Option<PathBuf> {
    let candidats = [
        std::env::var("ARCHMATES_AVATAR").ok().map(PathBuf::from),
        Some(PathBuf::from(format!("/var/lib/AccountsService/icons/{nom}"))),
        std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".face")),
    ];
    candidats.into_iter().flatten().find(|p| {
        fs::metadata(p).map(|m| m.is_file() && (m.len() as usize) <= AVATAR_MAX).unwrap_or(false)
    })
}

pub fn lire_avatar(nom: &str) -> Option<Vec<u8>> {
    chemin_avatar(nom).and_then(|p| fs::read(p).ok())
}

/// Empreinte FNV-1a 64 bits : simple, stable, suffisante pour détecter un changement.
pub fn empreinte(octets: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in octets {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

impl Profil {
    pub fn local() -> Self {
        let nom = std::env::var("ARCHMATES_NOM")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "?".into());
        let noyau = lire("/proc/sys/kernel/osrelease").unwrap_or_default();
        let noyau = noyau.strip_suffix("-cachyos").unwrap_or(&noyau).to_string();
        let allume_depuis_s = lire("/proc/uptime")
            .and_then(|t| t.split('.').next().and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        Self {
            hote: lire("/etc/hostname").unwrap_or_else(|| "linux".into()),
            distro: champ_os_release("NAME").unwrap_or_else(|| "Linux".into()),
            noyau,
            wm: std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "?".into()),
            terminal: std::env::var("TERMINAL").ok().unwrap_or_else(terminal_installe),
            shell: shell_de_connexion(&nom).unwrap_or_else(|| "?".into()),
            allume_depuis_s,
            avatar_empreinte: lire_avatar(&nom).map(|o| empreinte(&o)),
            version: env!("CARGO_PKG_VERSION").into(),
            nom,
        }
    }
}
