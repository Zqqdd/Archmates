#!/usr/bin/env bash
# Starts a fake friend on this machine, to try archmates without a second PC.
#
#   ./scripts/demo-friend.sh [nickname]
#
# The fake friend has its own identity (in a temporary folder), adds you, and you
# add it: after ~30 s it shows up online in `archmates list` and in the widget.
# Press Ctrl+C to stop: the fake friend goes offline and is removed from your list.
set -euo pipefail

NICK="${1:-demo-friend}"
BIN="$(command -v archmates || echo "$HOME/.local/bin/archmates")"
DEMO_HOME="$(mktemp -d -t archmates-demo-XXXXXX)"

if ! systemctl --user is-active --quiet archmates; then
  echo "The archmates service is not running. Start it with: systemctl --user start archmates" >&2
  exit 1
fi

MY_CODE="$("$BIN" code 2>/dev/null)"
DEMO_CODE="$(ARCHMATES_HOME="$DEMO_HOME" "$BIN" code 2>/dev/null)"

cleanup() {
  echo
  echo "→ Removing $NICK from your friends list"
  "$BIN" remove "$NICK" >/dev/null || true
  rm -rf "$DEMO_HOME"
}
trap cleanup EXIT

ARCHMATES_HOME="$DEMO_HOME" "$BIN" add "$MY_CODE" "$USER" >/dev/null
"$BIN" add "$DEMO_CODE" "$NICK" >/dev/null

echo "→ $NICK added. It should appear online within ~30 s:"
echo "    - in the widget on your desktop"
echo "    - with: archmates list"
echo "  Press Ctrl+C to stop the demo."
echo

ARCHMATES_HOME="$DEMO_HOME" ARCHMATES_USER="$NICK" ARCHMATES_NO_UPDATE_CHECK=1 "$BIN" daemon
