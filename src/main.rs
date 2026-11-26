//! amis : liste d'amis pair à pair pour CachyOS / Arch.
//!
//!   archmates code                    affiche ton code d'ami à partager
//!   archmates add <code> <nickname>   ajoute un ami (il doit t'ajouter aussi)
//!   archmates remove <nickname|code>  retire un ami
//!   archmates list                    affiche tes amis et leur statut
//!   archmates invisible on|off        mode invisible
//!   archmates profile                 affiche ce que tes amis voient de toi
//!   archmates daemon                  lance le service (normalement via systemd)

mod config;
mod daemon;
mod profil;
mod proto;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use config::{Chemins, ListeAmis, charger_ou_creer_cle};

#[derive(Parser)]
#[command(name = "archmates", version, about = "Liste d'amis pair à pair : vois qui est en ligne.")]
struct Cli {
    #[command(subcommand)]
    commande: Commande,
}

#[derive(Subcommand)]
enum Commande {
    /// Affiche ton code d'ami, à donner à tes amis
    Code,
    /// Ajoute un ami à partir de son code
    Add {
        /// Le code d'ami reçu
        code: String,
        /// Le surnom affiché pour cet ami
        #[arg(value_name = "NICKNAME")]
        surnom: String,
    },
    /// Retire un ami (par surnom ou par code)
    Remove {
        /// Surnom ou code de l'ami
        #[arg(value_name = "NICKNAME|CODE")]
        qui: String,
    },
    /// Affiche tes amis et leur statut
    List,
    /// Active ou désactive le mode invisible
    Invisible {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Affiche ton profil tel que tes amis le voient
    Profile,
    /// Lance le service de présence (au premier plan)
    Daemon,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

fn depuis(ms: u64) -> String {
    let maintenant = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let s = maintenant.saturating_sub(ms) / 1000;
    match s {
        0..=59 => "à l'instant".into(),
        60..=3599 => format!("il y a {} min", s / 60),
        3600..=86_399 => format!("il y a {} h", s / 3600),
        _ => format!("il y a {} j", s / 86_400),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let ch = Chemins::detecter();

    match cli.commande {
        Commande::Code => {
            let cle = charger_ou_creer_cle(&ch)?;
            println!("{}", cle.public());
            eprintln!("\nDonne ce code à tes amis. Ils l'ajoutent avec :\n  archmates add <code> <nickname>");
        }
        Commande::Add { code, surnom } => {
            let mut liste = ListeAmis::charger(&ch)?;
            liste.ajouter(&code, &surnom)?;
            liste.enregistrer(&ch)?;
            println!("✓ {surnom} ajouté.");
            println!("  Il apparaîtra en ligne quand il t'aura ajouté à son tour (archmates code → envoie-lui ton code).");
        }
        Commande::Remove { qui } => {
            let mut liste = ListeAmis::charger(&ch)?;
            if liste.retirer(&qui) {
                liste.enregistrer(&ch)?;
                println!("✓ {qui} retiré.");
            } else {
                println!("Aucun ami « {qui} ».");
            }
        }
        Commande::List => {
            let liste = ListeAmis::charger(&ch)?;
            if liste.amis.is_empty() {
                println!("Aucun ami pour l'instant. Ajoute-en un avec : archmates add <code> <nickname>");
                return Ok(());
            }
            let etat: Option<daemon::FichierEtat> = std::fs::read_to_string(ch.etat())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            if etat.is_none() {
                println!("(le service ne tourne pas : statuts inconnus)\n");
            }
            for ami in &liste.amis {
                let e = etat.as_ref().and_then(|f| f.amis.iter().find(|e| e.code == ami.code));
                let (pastille, texte) = match e.map(|e| e.statut) {
                    Some(daemon::Statut::EnLigne) => ("\x1b[32m●\x1b[0m", "en ligne".to_string()),
                    Some(daemon::Statut::EnAttente) => ("\x1b[33m●\x1b[0m", "ne t'a pas encore ajouté".to_string()),
                    _ => (
                        "\x1b[90m●\x1b[0m",
                        e.and_then(|e| e.vu_ms).map(|v| format!("hors ligne, vu {}", depuis(v))).unwrap_or("hors ligne".into()),
                    ),
                };
                let detail = e
                    .and_then(|e| e.profil.as_ref())
                    .map(|p| format!("  {}@{} · {} · {}", p.nom, p.hote, p.distro, p.wm))
                    .unwrap_or_default();
                println!("{pastille} {:<16} {texte}{detail}", ami.surnom);
            }
            if let Some(f) = etat.filter(|f| f.moi.invisible) {
                let _ = f;
                println!("\n(tu es en mode invisible)");
            }
        }
        Commande::Invisible { state } => match state {
            OnOff::On => {
                std::fs::create_dir_all(&ch.donnees)?;
                std::fs::write(ch.invisible(), b"")?;
                println!("✓ Mode invisible activé : tes amis te voient hors ligne.");
            }
            OnOff::Off => {
                let _ = std::fs::remove_file(ch.invisible());
                println!("✓ Mode invisible désactivé.");
            }
        },
        Commande::Profile => {
            println!("{}", serde_json::to_string_pretty(&profil::Profil::local())?);
        }
        Commande::Daemon => daemon::lancer().await?,
    }
    Ok(())
}
