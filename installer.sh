#!/usr/bin/env bash
# Installs archmates for the current user (no admin rights needed).
#
# What it does:
#   1. builds the program and installs the `archmates` command to ~/.local/bin
#   2. starts the presence service (systemd user unit) at every login
#   3. if Noctalia is installed: installs the widget and places it on your desktop
#   4. if Hyprland is running: adds a blur rule so the widget looks like frosted glass
set -euo pipefail
cd "$(dirname "$0")"

WIDGET_TYPE="archmates/archmates:friends"
WIDGET_WIDTH=320

echo "→ Building…"
cargo build --release

echo "→ Installing the command to ~/.local/bin/archmates"
install -Dm755 target/release/archmates ~/.local/bin/archmates

echo "→ Presence service (systemd user unit)"
install -Dm644 packaging/archmates.service ~/.config/systemd/user/archmates.service
systemctl --user daemon-reload
systemctl --user enable --now archmates.service
systemctl --user restart archmates.service

# Prints "NAME WIDTH HEIGHT" for the main monitor (the one at 0,0, else the focused one).
main_monitor() {
  command -v hyprctl >/dev/null && command -v python3 >/dev/null || return 1
  hyprctl monitors -j 2>/dev/null | python3 -c '
import json, sys
mons = json.load(sys.stdin)
main = next((m for m in mons if m["x"] == 0 and m["y"] == 0), None) \
    or next((m for m in mons if m.get("focused")), None) or (mons[0] if mons else None)
if main:
    s = main.get("scale") or 1
    print(main["name"], int(main["width"] / s), int(main["height"] / s))
'
}

place_widget() {
  local order
  order=$(noctalia msg config-get desktop_widgets.widget_order 2>/dev/null || echo "[]")
  if [[ "$order" == *"archmates@"* ]]; then
    echo "  The widget is already on your desktop."
    return
  fi

  local mon name width height id cx cy
  if ! mon=$(main_monitor) || [[ -z "$mon" ]]; then
    echo "  Could not detect your screen: add the widget with  noctalia msg desktop-widgets-edit"
    return
  fi
  read -r name width height <<<"$mon"
  id="archmates@$name"
  cx=$(( width - WIDGET_WIDTH / 2 - 40 ))   # top-right corner, 40 px from the edge
  cy=200

  local key="desktop_widgets.widget.\"$id\""
  noctalia msg config-set "$key.type" "\"$WIDGET_TYPE\"" >/dev/null
  noctalia msg config-set "$key.output" "\"$name\"" >/dev/null
  noctalia msg config-set "$key.enabled" true >/dev/null
  noctalia msg config-set "$key.cx" "$cx.0" >/dev/null
  noctalia msg config-set "$key.cy" "$cy.0" >/dev/null
  noctalia msg config-set "$key.placement_width" "$width.0" >/dev/null
  noctalia msg config-set "$key.placement_height" "$height.0" >/dev/null
  noctalia msg config-set "$key.rotation" 0.0 >/dev/null
  # The widget draws its own glass background: turn off the default one.
  noctalia msg config-set "$key.settings.background_opacity" 0.0 >/dev/null

  # Append to the list of displayed widgets, keeping the existing ones.
  local new_order
  order=${order//\'/\"}
  if [[ "$order" =~ \"[^\"]+\" ]]; then
    new_order="${order%]*}, \"$id\" ]"
  else
    new_order="[ \"$id\" ]"
  fi
  noctalia msg config-set desktop_widgets.widget_order "$new_order" >/dev/null
  echo "  Widget placed at the top right of $name (move it with: noctalia msg desktop-widgets-edit)"
}

# Frosted-glass blur behind the widget. Only added if Hyprland accepts it:
# the config is restored if the rule causes an error.
add_blur_rule() {
  command -v hyprctl >/dev/null && hyprctl version >/dev/null 2>&1 || return 0
  local dir=~/.config/hypr
  if grep -rqs "noctalia-desktop-widget" "$dir"; then
    return 0   # already there
  fi

  local file rule
  if [[ -f "$dir/hyprland.lua" ]]; then
    file="$dir/hyprland.lua"
    rule='
-- archmates: frosted glass behind Noctalia desktop widgets
hl.layer_rule({ name = "noctalia-desktop-widgets-glass", match = { namespace = "^noctalia-desktop-widget-.*" }, blur = true, ignore_alpha = 0.05 })'
  elif [[ -f "$dir/hyprland.conf" ]]; then
    file="$dir/hyprland.conf"
    rule='
# archmates: frosted glass behind Noctalia desktop widgets
layerrule = blur, ^(noctalia-desktop-widget-.*)$
layerrule = ignorealpha 0.05, ^(noctalia-desktop-widget-.*)$'
  else
    return 0
  fi

  local errors_before errors_after
  errors_before=$(hyprctl configerrors 2>/dev/null || true)
  cp "$file" "$file.bak-archmates"
  printf '%s\n' "$rule" >>"$file"
  hyprctl reload >/dev/null 2>&1 || true
  sleep 1
  errors_after=$(hyprctl configerrors 2>/dev/null || true)
  if [[ "$errors_after" != "$errors_before" ]]; then
    mv "$file.bak-archmates" "$file"
    hyprctl reload >/dev/null 2>&1 || true
    echo "  Blur rule not supported by your Hyprland version: skipped (config restored)."
  else
    rm -f "$file.bak-archmates"
    echo "  Added a blur rule to $file"
  fi
}

if command -v noctalia >/dev/null && noctalia msg config-get desktop_widgets.enabled >/dev/null 2>&1; then
  echo "→ Noctalia widget"
  DEST="$HOME/.local/share/noctalia-plugins"
  mkdir -p "$DEST"
  rm -rf "$DEST/archmates"
  cp -r noctalia/archmates "$DEST/archmates"
  noctalia msg plugins source list 2>/dev/null | grep -qF "$DEST" \
    || noctalia msg plugins source add archmates path "$DEST" >/dev/null || true
  noctalia msg plugins disable archmates/archmates >/dev/null 2>&1 || true
  noctalia msg plugins enable archmates/archmates >/dev/null || true
  noctalia msg config-set desktop_widgets.enabled true >/dev/null || true
  place_widget
  add_blur_rule
else
  echo "→ Noctalia is not running: skipping the widget (use 'archmates list' in a terminal)."
fi

echo
echo "✓ Installed. Your friend code:"
~/.local/bin/archmates code
