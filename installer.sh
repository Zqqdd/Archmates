#!/usr/bin/env bash
# Installs archmates for the current user (no admin rights needed).
set -euo pipefail
cd "$(dirname "$0")"

echo "→ Building…"
cargo build --release

echo "→ Installing the command to ~/.local/bin/archmates"
install -Dm755 target/release/archmates ~/.local/bin/archmates

echo "→ Presence service (systemd user unit)"
install -Dm644 packaging/archmates.service ~/.config/systemd/user/archmates.service
systemctl --user daemon-reload
systemctl --user enable --now archmates.service
systemctl --user restart archmates.service

if command -v noctalia >/dev/null; then
  echo "→ Noctalia widget"
  DEST=~/.local/share/noctalia-plugins
  mkdir -p "$DEST"
  rm -rf "$DEST/archmates"
  cp -r noctalia/archmates "$DEST/archmates"
  noctalia msg plugins source list 2>/dev/null | grep -q "$DEST" \
    || noctalia msg plugins source add local path "$DEST" >/dev/null || true
  noctalia msg plugins disable archmates/archmates >/dev/null 2>&1 || true
  noctalia msg plugins enable archmates/archmates >/dev/null || true
  echo "  Add the \"Archmates\" widget to your desktop: noctalia msg desktop-widgets-edit"
fi

echo
echo "✓ Installed. Your friend code:"
~/.local/bin/archmates code
