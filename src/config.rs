//! Fichiers locaux : clé secrète, liste d'amis, mode invisible, état pour le widget.
//!
//! Tout est rangé selon les conventions XDG :
//!   ~/.local/share/archmates/cle.secrete   identité (32 octets, lisible par toi seul)
//!   ~/.local/share/archmates/amis.toml     ta liste d'amis
//!   ~/.local/share/archmates/invisible     présent = mode invisible
//!   ~/.cache/archmates/etat.json           état des amis, lu par le widget Noctalia
//!   ~/.cache/archmates/avatars/            photos de profil des amis
//!
//! La variable ARCHMATES_HOME remplace ces deux dossiers (utile pour tester deux
//! instances sur le même PC).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use iroh::{EndpointId, SecretKey};
use serde::{Deserialize, Serialize};

pub struct Chemins {
    pub donnees: PathBuf,
    pub cache: PathBuf,
}

impl Chemins {
    pub fn detecter() -> Self {
        if let Ok(base) = std::env::var("ARCHMATES_HOME") {
            let base = PathBuf::from(base);
            return Self { donnees: base.join("donnees"), cache: base.join("cache") };
        }
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
        let donnees = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".local/share"))
            .join("archmates");
        let cache = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".cache"))
            .join("archmates");
        Self { donnees, cache }
    }

    pub fn cle(&self) -> PathBuf { self.donnees.join("cle.secrete") }
    pub fn amis(&self) -> PathBuf { self.donnees.join("amis.toml") }
    pub fn invisible(&self) -> PathBuf { self.donnees.join("invisible") }
    pub fn etat(&self) -> PathBuf { self.cache.join("etat.json") }
    pub fn avatars(&self) -> PathBuf { self.cache.join("avatars") }

    pub fn est_invisible(&self) -> bool { self.invisible().exists() }
}

/// Charge la clé secrète, ou en crée une la première fois.
pub fn charger_ou_creer_cle(ch: &Chemins) -> Result<SecretKey> {
    let p = ch.cle();
    if p.exists() {
        let octets = fs::read(&p).with_context(|| format!("lecture de {}", p.display()))?;
        let tab: [u8; 32] = octets
            .as_slice()
            .try_into()
            .context("clé secrète corrompue (taille différente de 32 octets)")?;
        return Ok(SecretKey::from_bytes(&tab));
    }
    fs::create_dir_all(&ch.donnees)?;
    fs::set_permissions(&ch.donnees, fs::Permissions::from_mode(0o700))?;
    let cle = SecretKey::generate();
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&p)?;
    f.write_all(&cle.to_bytes())?;
    Ok(cle)
}

// ── Liste d'amis ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ami {
    pub surnom: String,
    pub code: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ListeAmis {
    #[serde(default, rename = "ami")]
    pub amis: Vec<Ami>,
}

impl ListeAmis {
    pub fn charger(ch: &Chemins) -> Result<Self> {
        let p = ch.amis();
        if !p.exists() {
            return Ok(Self::default());
        }
        let txt = fs::read_to_string(&p)?;
        toml::from_str(&txt).with_context(|| format!("{} est mal formé", p.display()))
    }

    pub fn enregistrer(&self, ch: &Chemins) -> Result<()> {
        fs::create_dir_all(&ch.donnees)?;
        let txt = format!(
            "# Ta liste d'amis (modifiable à la main, ou avec « archmates add / remove »)\n\n{}",
            toml::to_string_pretty(self)?
        );
        ecrire_atomique(&ch.amis(), txt.as_bytes())
    }

    /// Amis dont le code est valide, indexés par identifiant.
    pub fn par_id(&self) -> BTreeMap<EndpointId, Ami> {
        self.amis
            .iter()
            .filter_map(|a| a.code.parse::<EndpointId>().ok().map(|id| (id, a.clone())))
            .collect()
    }

    pub fn ajouter(&mut self, code: &str, surnom: &str) -> Result<()> {
        let id: EndpointId = code.trim().parse().context("ce code d'ami n'est pas valide")?;
        if self.par_id().contains_key(&id) {
            bail!("cet ami est déjà dans ta liste");
        }
        self.amis.push(Ami { surnom: surnom.to_string(), code: id.to_string() });
        Ok(())
    }

    pub fn retirer(&mut self, qui: &str) -> bool {
        let avant = self.amis.len();
        self.amis.retain(|a| a.surnom != qui && a.code != qui);
        self.amis.len() != avant
    }
}

/// Écrit via un fichier temporaire puis renomme : le widget ne lit jamais un fichier à moitié écrit.
pub fn ecrire_atomique(p: &Path, contenu: &[u8]) -> Result<()> {
    if let Some(dossier) = p.parent() {
        fs::create_dir_all(dossier)?;
    }
    let tmp = p.with_extension("tmp");
    fs::write(&tmp, contenu)?;
    fs::rename(&tmp, p)?;
    Ok(())
}
