#!/usr/bin/env bash
# Installe archmates pour l'utilisateur courant (aucun droit administrateur requis).
set -euo pipefail
cd "$(dirname "$0")"

echo "→ Compilation…"
cargo build --release

echo "→ Installation de la commande dans ~/.local/bin/archmates"
install -Dm755 target/release/archmates ~/.local/bin/archmates

echo "→ Service de présence (systemd utilisateur)"
install -Dm644 packaging/archmates.service ~/.config/systemd/user/archmates.service
systemctl --user daemon-reload
systemctl --user enable --now archmates.service
systemctl --user restart archmates.service

if command -v noctalia >/dev/null; then
  echo "→ Widget Noctalia"
  DEST=~/.local/share/noctalia-plugins
  mkdir -p "$DEST"
  rm -rf "$DEST/archmates"
  cp -r noctalia/archmates "$DEST/archmates"
  noctalia msg plugins source list 2>/dev/null | grep -q "$DEST" \
    || noctalia msg plugins source add local path "$DEST" >/dev/null || true
  noctalia msg plugins disable archmates/archmates >/dev/null 2>&1 || true
  noctalia msg plugins enable archmates/archmates >/dev/null || true
  echo "  Ajoute le widget « Amis » sur ton bureau : noctalia msg desktop-widgets-edit"
fi

echo
echo "✓ Installé. Ton code d'ami :"
~/.local/bin/archmates code
