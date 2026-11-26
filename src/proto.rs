//! Protocole entre deux PC.
//!
//! Chaque échange : une connexion QUIC chiffrée (Iroh), un flux bidirectionnel,
//! une requête JSON, une réponse. Seuls les amis réciproques sont acceptés :
//! la connexion est authentifiée par la clé publique de l'autre PC.

use serde::{Deserialize, Serialize};

use crate::profil::Profil;

/// Identifiant du protocole (doit être identique des deux côtés).
pub const ALPN: &[u8] = b"archmates/presence/1";

/// Taille maximale d'une requête ou d'une réponse JSON.
pub const JSON_MAX: usize = 64 * 1024;

/// Code de fermeture quand l'autre ne nous a pas (encore) ajouté.
pub const CODE_PAS_AMI: u32 = 403;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Requete {
    /// « Je suis en ligne, voici mon profil. Et toi ? »
    Bonjour { profil: Profil },
    /// « Envoie-moi ta photo de profil. » Réponse : les octets bruts de l'image.
    Avatar,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reponse {
    pub profil: Profil,
}
