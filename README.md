# archmates

Une liste d'amis **pair à pair** pour CachyOS / Arch Linux : vois quand tes amis sont en ligne, avec leur profil
(nom, machine, distro, noyau, WM…) et leur photo, directement sur ton bureau Noctalia.

Aucun compte, aucun serveur à toi : ta liste d'amis reste sur ton PC, et les PC se parlent directement,
de façon chiffrée, grâce à [Iroh](https://iroh.computer).

## Installation

Il faut Rust (`sudo pacman -S rustup && rustup default stable`), puis :

```bash
./installer.sh
```

Le script compile, installe la commande `archmates` dans `~/.local/bin`, démarre le service de présence
(systemd utilisateur) et, si tu utilises Noctalia, installe le widget « Amis ». Aucun mot de passe n'est demandé.

## Utilisation

```bash
archmates code                    # ton code d'ami, à envoyer à tes amis
archmates add <code> <nickname>   # ajoute un ami (il doit t'ajouter aussi)
archmates list                    # qui est en ligne
archmates remove <nickname>       # retire un ami
archmates invisible on|off        # mode invisible
amis profil                  # ce que tes amis voient de toi
```

Pour placer le widget sur ton bureau : `noctalia msg desktop-widgets-edit`.

## Comment ça marche

```
 Ton PC                                         PC d'un ami
┌─────────────────────┐   connexion directe    ┌─────────────────────┐
│ amis.toml (local)   │◄══ chiffrée (QUIC) ═══►│ amis.toml (local)   │
│ service archmates daemon │   « bonjour » + profil │ service archmates daemon │
│ widget Amis         │                        │ widget Amis         │
└─────────────────────┘                        └─────────────────────┘
```

1. **Identité** : au premier lancement, une clé secrète est créée dans `~/.local/share/archmates/cle.secrete`.
   Ton code d'ami est la clé publique correspondante.
2. **Amitié réciproque** : un PC n'accepte que les connexions des clés présentes dans son `amis.toml`.
   Tant que ton ami ne t'a pas ajouté, il apparaît en jaune : « ne t'a pas encore ajouté ».
3. **Présence** : toutes les 30 s, le service contacte chaque ami et échange les profils.
   Un ami est « en ligne » si un échange a réussi il y a moins de 75 s.
4. **Photos** : chaque profil contient l'empreinte de la photo. Elle n'est retéléchargée que si elle change.
5. **Widget** : le service écrit `~/.cache/archmates/etat.json`, que le widget Noctalia relit toutes les 5 s.

Iroh perce les box et les routeurs (*hole punching*) pour une connexion directe. Quand c'est impossible,
le trafic passe par un relais public d'Iroh, qui ne voit que des données chiffrées.

## Organisation du code

| Fichier | Rôle |
|---|---|
| `src/main.rs` | les commandes (`archmates code`, `add`, `list`…) |
| `src/config.rs` | fichiers locaux : clé, liste d'amis, mode invisible |
| `src/profil.rs` | construit ton profil (distro, noyau, photo…) |
| `src/proto.rs` | le format des messages échangés entre PC |
| `src/daemon.rs` | le service : écoute, contacte les amis, écrit l'état, notifications |
| `noctalia/archmates/` | le widget de bureau (Luau) |
| `packaging/archmates.service` | le service systemd utilisateur |

## Limites du prototype

- Un ami connecté directement voit ton adresse IP (normal en pair à pair).
- Si tu perds `cle.secrete`, ton code change : sauvegarde ce fichier.
- Pas encore de « joue à… / écoute… », ni de PKGBUILD.
