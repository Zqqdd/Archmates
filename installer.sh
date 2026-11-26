#!/usr/bin/env bash
# Installs archmates for the current user (no admin rights needed).
#
# What it does:
#   1. builds the program and installs the `archmates` command to ~/.local/bin
#   2. starts the presence service (systemd user unit) at every login
#   3. runs scripts/setup-desktop.sh: places the Noctalia widget on your desktop
#      and adds a Hyprland blur rule so it looks like frosted glass
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

echo "→ Desktop setup"
./scripts/setup-desktop.sh

echo
echo "✓ Installed. Your friend code:"
~/.local/bin/archmates code
