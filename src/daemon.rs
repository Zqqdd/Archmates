//! Le service de présence (« archmates daemon »).
//!
//! - écoute les connexions entrantes de tes amis et leur répond avec ton profil ;
//! - toutes les 30 s, contacte chacun de tes amis ;
//! - écrit l'état de tout le monde dans ~/.cache/archmates/etat.json pour le widget ;
//! - envoie une notification quand un ami se connecte.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};

use crate::config::{Chemins, ListeAmis, ecrire_atomique};
use crate::profil::{AVATAR_MAX, Profil, empreinte, lire_avatar};
use crate::proto::{ALPN, CODE_PAS_AMI, JSON_MAX, Reponse, Requete};

const INTERVALLE: Duration = Duration::from_secs(30);
const DELAI_CONNEXION: Duration = Duration::from_secs(20);
/// Un ami est « en ligne » si on a échangé avec lui il y a moins de ça.
const EN_LIGNE_SI_MOINS_DE_MS: u64 = 75_000;

fn maintenant_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ── État partagé, tel qu'écrit dans etat.json ───────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Statut {
    EnLigne,
    HorsLigne,
    /// L'ami ne t'a pas encore ajouté de son côté.
    EnAttente,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtatAmi {
    pub surnom: String,
    pub code: String,
    pub statut: Statut,
    /// Dernier échange réussi (ms depuis 1970), conservé entre les redémarrages.
    pub vu_ms: Option<u64>,
    pub profil: Option<Profil>,
    /// Chemin local de sa photo de profil (en cache).
    pub avatar: Option<String>,
    #[serde(default)]
    pub en_attente: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Moi {
    pub code: String,
    pub invisible: bool,
    pub maj_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FichierEtat {
    pub moi: Moi,
    pub amis: Vec<EtatAmi>,
}

struct Partage {
    ch: Chemins,
    moi: EndpointId,
    amis: Mutex<BTreeMap<EndpointId, EtatAmi>>,
    deja_notifies: Mutex<HashSet<EndpointId>>,
}

impl Partage {
    /// Synchronise l'état avec amis.toml (pris en compte sans redémarrer le service).
    fn recharger_liste(&self) -> Result<BTreeMap<EndpointId, crate::config::Ami>> {
        let liste = ListeAmis::charger(&self.ch)?.par_id();
        let mut amis = self.amis.lock().unwrap();
        amis.retain(|id, _| liste.contains_key(id));
        for (id, a) in &liste {
            let e = amis.entry(*id).or_insert_with(|| EtatAmi {
                surnom: a.surnom.clone(),
                code: a.code.clone(),
                statut: Statut::HorsLigne,
                vu_ms: None,
                profil: None,
                avatar: None,
                en_attente: false,
            });
            e.surnom = a.surnom.clone();
        }
        Ok(liste)
    }

    fn est_ami(&self, id: &EndpointId) -> bool {
        ListeAmis::charger(&self.ch).map(|l| l.par_id().contains_key(id)).unwrap_or(false)
    }

    /// Enregistre un échange réussi avec un ami.
    fn contact(&self, id: EndpointId, profil: Profil) {
        let mut amis = self.amis.lock().unwrap();
        if let Some(e) = amis.get_mut(&id) {
            e.vu_ms = Some(maintenant_ms());
            e.profil = Some(profil);
            e.en_attente = false;
        }
    }

    fn marquer_en_attente(&self, id: EndpointId, oui: bool) {
        if let Some(e) = self.amis.lock().unwrap().get_mut(&id) {
            e.en_attente = oui;
        }
    }

    fn avatar_a_jour(&self, id: &EndpointId, empreinte_distante: &Option<String>) -> bool {
        let amis = self.amis.lock().unwrap();
        let Some(e) = amis.get(id) else { return true };
        let Some(emp) = empreinte_distante else { return true };
        let fichier = self.ch.avatars().join(format!("{id}-{emp}"));
        e.avatar.as_deref() == Some(fichier.to_string_lossy().as_ref()) && fichier.exists()
    }

    fn enregistrer_avatar(&self, id: EndpointId, octets: &[u8]) -> Result<()> {
        let emp = empreinte(octets);
        let dossier = self.ch.avatars();
        std::fs::create_dir_all(&dossier)?;
        // Supprime les anciennes photos de cet ami.
        for entree in std::fs::read_dir(&dossier)?.flatten() {
            if entree.file_name().to_string_lossy().starts_with(&format!("{id}-")) {
                let _ = std::fs::remove_file(entree.path());
            }
        }
        let fichier = dossier.join(format!("{id}-{emp}"));
        ecrire_atomique(&fichier, octets)?;
        if let Some(e) = self.amis.lock().unwrap().get_mut(&id) {
            e.avatar = Some(fichier.to_string_lossy().into_owned());
        }
        Ok(())
    }

    /// Recalcule les statuts, écrit etat.json et envoie les notifications.
    fn publier(&self) -> Result<()> {
        let invisible = self.ch.est_invisible();
        let t = maintenant_ms();
        let mut nouveaux = Vec::new();
        let liste: Vec<EtatAmi> = {
            let mut amis = self.amis.lock().unwrap();
            let mut notifies = self.deja_notifies.lock().unwrap();
            for (id, e) in amis.iter_mut() {
                let recent = e.vu_ms.is_some_and(|v| t.saturating_sub(v) < EN_LIGNE_SI_MOINS_DE_MS);
                e.statut = if invisible {
                    Statut::HorsLigne
                } else if recent {
                    Statut::EnLigne
                } else if e.en_attente {
                    Statut::EnAttente
                } else {
                    Statut::HorsLigne
                };
                if e.statut == Statut::EnLigne {
                    if notifies.insert(*id) {
                        nouveaux.push(e.clone());
                    }
                } else {
                    notifies.remove(id);
                }
            }
            amis.values().cloned().collect()
        };
        let fichier = FichierEtat {
            moi: Moi { code: self.moi.to_string(), invisible, maj_ms: t },
            amis: liste,
        };
        ecrire_atomique(&self.ch.etat(), serde_json::to_string_pretty(&fichier)?.as_bytes())?;
        for e in nouveaux {
            notifier(&e);
        }
        Ok(())
    }
}

fn notifier(e: &EtatAmi) {
    let corps = match &e.profil {
        Some(p) => format!("{}@{} · {}", p.nom, p.hote, p.distro),
        None => String::new(),
    };
    let mut cmd = std::process::Command::new("notify-send");
    cmd.args(["-a", "Archmates", "-u", "low"]);
    if let Some(a) = &e.avatar {
        cmd.args(["-i", a]);
    }
    cmd.arg(format!("{} est en ligne", e.surnom)).arg(corps);
    let _ = cmd.spawn();
}

// ── Côté serveur : répondre aux amis qui nous contactent ────────────────────

#[derive(Clone)]
struct Accueil(Arc<Partage>);

impl std::fmt::Debug for Accueil {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Accueil")
    }
}

impl ProtocolHandler for Accueil {
    async fn accept(&self, connexion: Connection) -> Result<(), AcceptError> {
        let id = connexion.remote_id();
        let p = &self.0;
        // Refus : pas dans notre liste, ou mode invisible.
        if p.ch.est_invisible() || !p.est_ami(&id) {
            connexion.close(CODE_PAS_AMI.into(), b"pas ami");
            return Ok(());
        }
        if let Err(err) = servir(p, &connexion, id).await {
            eprintln!("[entrant {}] {err:#}", court(&id));
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), connexion.closed()).await;
        Ok(())
    }
}

async fn servir(p: &Arc<Partage>, connexion: &Connection, id: EndpointId) -> Result<()> {
    // Un flux par requête, jusqu'à ce que l'ami ferme la connexion.
    while let Ok((mut envoi, mut reception)) = connexion.accept_bi().await {
        let octets = reception.read_to_end(JSON_MAX).await?;
        let requete: Requete = serde_json::from_slice(&octets).context("requête illisible")?;
        match requete {
            Requete::Bonjour { profil } => {
                p.contact(id, profil);
                let rep = Reponse { profil: Profil::local() };
                envoi.write_all(&serde_json::to_vec(&rep)?).await?;
                let _ = p.publier();
            }
            Requete::Avatar => {
                let octets = lire_avatar(&Profil::local().nom).unwrap_or_default();
                envoi.write_all(&octets).await?;
            }
        }
        envoi.finish()?;
    }
    Ok(())
}

// ── Côté client : contacter un ami ──────────────────────────────────────────

async fn saluer(endpoint: &Endpoint, p: &Arc<Partage>, id: EndpointId) -> Result<()> {
    let connexion = tokio::time::timeout(DELAI_CONNEXION, endpoint.connect(id, ALPN))
        .await
        .map_err(|_| anyhow!("injoignable (délai dépassé)"))??;

    let resultat = echanger(p, &connexion, id).await;
    if resultat.is_err() && ferme_pour_pas_ami(&connexion) {
        p.marquer_en_attente(id, true);
        connexion.close(0u32.into(), b"ok");
        return Err(anyhow!("ne t'a pas encore ajouté"));
    }
    connexion.close(0u32.into(), b"ok");
    resultat
}

async fn echanger(p: &Arc<Partage>, connexion: &Connection, id: EndpointId) -> Result<()> {
    let (mut envoi, mut reception) = connexion.open_bi().await?;
    let req = Requete::Bonjour { profil: Profil::local() };
    envoi.write_all(&serde_json::to_vec(&req)?).await?;
    envoi.finish()?;
    let octets = reception.read_to_end(JSON_MAX).await?;
    let rep: Reponse = serde_json::from_slice(&octets).context("réponse illisible")?;

    let emp = rep.profil.avatar_empreinte.clone();
    p.contact(id, rep.profil);

    if emp.is_some() && !p.avatar_a_jour(&id, &emp) {
        let (mut envoi, mut reception) = connexion.open_bi().await?;
        envoi.write_all(&serde_json::to_vec(&Requete::Avatar)?).await?;
        envoi.finish()?;
        let image = reception.read_to_end(AVATAR_MAX).await?;
        if !image.is_empty() {
            p.enregistrer_avatar(id, &image)?;
        }
    }
    Ok(())
}

fn ferme_pour_pas_ami(connexion: &Connection) -> bool {
    // L'autre côté ferme avec le code 403 quand il ne nous a pas dans sa liste.
    connexion
        .close_reason()
        .map(|r| format!("{r:?}").contains(&format!("error_code: {CODE_PAS_AMI}")))
        .unwrap_or(false)
}

fn court(id: &EndpointId) -> String {
    id.to_string().chars().take(8).collect()
}

// ── Boucle principale ───────────────────────────────────────────────────────

pub async fn lancer() -> Result<()> {
    let ch = Chemins::detecter();
    let cle = crate::config::charger_ou_creer_cle(&ch)?;
    let moi = cle.public();

    // Reprend le dernier état connu (photos, « vu il y a… ») après un redémarrage.
    let mut initial = BTreeMap::new();
    if let Ok(txt) = std::fs::read_to_string(ch.etat()) {
        if let Ok(ancien) = serde_json::from_str::<FichierEtat>(&txt) {
            for e in ancien.amis {
                if let Ok(id) = e.code.parse::<EndpointId>() {
                    initial.insert(id, EtatAmi { en_attente: false, ..e });
                }
            }
        }
    }
    let partage = Arc::new(Partage {
        ch,
        moi,
        amis: Mutex::new(initial),
        deja_notifies: Mutex::new(HashSet::new()),
    });

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(cle)
        .bind()
        .await
        .context("impossible de démarrer le réseau P2P")?;
    let router = Router::builder(endpoint.clone()).accept(ALPN, Accueil(partage.clone())).spawn();
    eprintln!("archmates : service démarré, ton code est {moi}");
    let _ = tokio::time::timeout(Duration::from_secs(15), endpoint.online()).await;
    eprintln!("archmates : connecté au réseau");

    let mut horloge = tokio::time::interval(INTERVALLE);
    loop {
        tokio::select! {
            _ = horloge.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        let liste = match partage.recharger_liste() {
            Ok(l) => l,
            Err(err) => {
                eprintln!("amis.toml : {err:#}");
                continue;
            }
        };
        if !partage.ch.est_invisible() {
            let mut taches = Vec::new();
            for (id, ami) in liste {
                let (ep, p) = (endpoint.clone(), partage.clone());
                taches.push(tokio::spawn(async move {
                    match saluer(&ep, &p, id).await {
                        Ok(()) => {
                            p.marquer_en_attente(id, false);
                        }
                        Err(err) => eprintln!("[{}] {err:#}", ami.surnom),
                    }
                }));
            }
            for t in taches {
                let _ = t.await;
            }
        }
        if let Err(err) = partage.publier() {
            eprintln!("etat.json : {err:#}");
        }
    }

    router.shutdown().await.ok();
    Ok(())
}
