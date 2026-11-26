#!/usr/bin/env bash
# Installs archmates for the current user (no admin rights needed).
#
# What it does:
#   1. builds the program (or uses the ready-made one from a release archive) and installs
#      the `archmates` and `archmates-widget` commands to ~/.local/bin
#   2. starts the presence service (systemd user unit) at every login
#   3. runs scripts/setup-desktop.sh: sets up the desktop widget (Noctalia's, or the
#      universal one) and adds a Hyprland blur rule so it looks like frosted glass
set -euo pipefail
cd "$(dirname "$0")"

if [[ -f Cargo.toml ]]; then
  echo "→ Building…"
  cargo build --release
  BIN=target/release/archmates
else
  BIN=./archmates   # release archive: already built
fi

echo "→ Installing the commands to ~/.local/bin"
install -Dm755 "$BIN" ~/.local/bin/archmates
install -Dm755 widget/archmates-widget ~/.local/bin/archmates-widget

echo "→ Presence service (systemd user unit)"
install -Dm644 packaging/archmates.service ~/.config/systemd/user/archmates.service
systemctl --user daemon-reload
systemctl --user disable archmates.service >/dev/null 2>&1 || true   # drop links from older versions
systemctl --user enable --now archmates.service
systemctl --user restart archmates.service

echo "→ Desktop setup"
./scripts/setup-desktop.sh

echo
echo "✓ Installed. Your friend code:"
~/.local/bin/archmates code
