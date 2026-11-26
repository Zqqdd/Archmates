# archmates

> 🤖 **This project was almost entirely written by [Claude](https://claude.ai), Anthropic's AI assistant**
> (through Claude Code). The repository owner directed the idea, the design choices and the testing.

A **peer-to-peer** friends list for CachyOS / Arch Linux: see when your friends are online, with their profile
(user, host, distro, kernel, WM…) and their picture, right on your Noctalia desktop.

No accounts, no servers of your own: your friends list stays on your machine, and machines talk to each other
directly over encrypted connections, powered by [Iroh](https://iroh.computer).

## Install

### From the AUR (recommended, updates come with your system updates)

```bash
paru -S archmates-bin
systemctl --user enable --now archmates
archmates-setup-desktop
```

### From a release archive (no Rust needed)

Download `archmates-<version>-x86_64-linux.tar.gz` from the
[releases page](https://github.com/Zqqdd/Archmates/releases), then:

```bash
tar -xzf archmates-*-x86_64-linux.tar.gz && cd archmates-*-x86_64-linux && ./installer.sh
```

### From source

You need Rust (`sudo pacman -S rustup && rustup default stable`), then:

```bash
git clone https://github.com/Zqqdd/Archmates.git && cd Archmates
./installer.sh
```

The script does everything, with no password needed:

- builds the project (or uses the ready-made binary from a release archive) and installs the `archmates` and
  `archmates-widget` commands to `~/.local/bin`;
- starts the presence service (systemd user unit) at every login;
- if you use **Noctalia**: installs the "Archmates" widget and places it at the top right of your main screen,
  without touching your other widgets (move it later with `noctalia msg desktop-widgets-edit`);
- without Noctalia: the **universal widget** is used instead (see below), and the script tells you which
  packages it needs, if any;
- if you use **Hyprland**: adds a frosted-glass blur rule behind the widget. The rule is only kept if Hyprland
  accepts it; otherwise your config is restored as it was.

Running the installer again (for an update) keeps your widget where you put it.

## Usage

```bash
archmates code                     # your friend code, to send to your friends
archmates add <code> <nickname>    # add a friend (they must add you too)
archmates list                     # who is online
archmates remove <nickname>        # remove a friend
archmates invisible on|off         # invisible mode
archmates profile                  # what your friends see of you
archmates avatar <image>           # set your profile picture (PNG, JPEG, WebP, GIF, max 2 MB)
archmates update                   # check whether a new version is available
```

To place the widget on your desktop: `noctalia msg desktop-widgets-edit`.

### Universal widget (without Noctalia)

`archmates-widget` is a small frosted-glass card pinned to your desktop, behind your windows. It works on
Wayland compositors with the layer-shell protocol: Hyprland, sway, KDE Plasma, niri, river, labwc, Wayfire…
(not GNOME). It needs:

```bash
sudo pacman -S --needed gtk4 gtk4-layer-shell python-gobject
```

The service starts it by itself as soon as your desktop is up, and only when Noctalia is not running
(Noctalia has its own widget). Change that with:

```bash
archmates widget auto   # default: shown when Noctalia is absent
archmates widget on     # always shown
archmates widget off    # never shown
```

Customise it with `~/.config/archmates/widget.toml`:

```toml
corner = "top-right"   # top-left, top-right, bottom-left, bottom-right
margin_x = 40
margin_y = 60
output = "DP-1"        # screen to use (default: the main screen)
```

and its look with `~/.config/archmates/widget.css` (GTK CSS, loaded after the default style).

### Try it alone

No friend at hand? Start a fake friend on your own machine:

```bash
./scripts/demo-friend.sh
```

It shows up online in the widget and in `archmates list` within ~30 s. Press Ctrl+C to stop it: it is
removed from your list automatically.

## Updates

- **AUR install**: archmates is updated with the rest of your system (`paru -Syu`).
- **Source install**: `git pull && ./installer.sh`.
- Once a day, the service checks GitHub for a new release. When there is one, you get a notification and the
  widget shows "Update available". Nothing is downloaded or installed automatically: updates always go through
  pacman or the installer, so you can see what lands on your machine. Set `ARCHMATES_NO_UPDATE_CHECK=1` to turn
  the check off, or run `archmates update` to check by hand.
- After an update, the running service notices that its program was replaced and restarts itself.

New versions are published by pushing a tag: GitHub Actions builds the release archive used by the AUR package.

```bash
# bump the version in Cargo.toml first, then:
git tag v0.3.0 && git push origin v0.3.0
```

## How it works

```
 Your PC                                        A friend's PC
┌─────────────────────────┐   direct encrypted  ┌─────────────────────────┐
│ friends.toml (local)    │◄═══ connection ═══►│ friends.toml (local)    │
│ archmates daemon        │   hello + profile   │ archmates daemon        │
│ Archmates widget        │       (QUIC)        │ Archmates widget        │
└─────────────────────────┘                     └─────────────────────────┘
```

1. **Identity**: on first run, a secret key is created in `~/.local/share/archmates/secret.key`.
   Your friend code is the matching public key.
2. **Mutual friendship**: a machine only accepts connections from keys listed in its `friends.toml`.
   Until your friend adds you back, they show up in yellow: "hasn't added you yet".
3. **Presence**: every 30 s, the service contacts each friend and exchanges profiles.
   A friend is "online" if an exchange succeeded less than 75 s ago.
4. **Pictures**: each profile carries a fingerprint of the picture, so it is only downloaded again when it changes.
5. **Widget**: the service writes `~/.cache/archmates/state.json`, which the Noctalia widget reads every 5 s.

Iroh punches through routers and NATs (*hole punching*) to get a direct connection. When that is impossible,
traffic goes through a public Iroh relay, which only ever sees encrypted data.

## Code layout

| File | Role |
|---|---|
| `src/main.rs` | the commands (`archmates code`, `add`, `list`…) |
| `src/config.rs` | local files: key, friends list, invisible mode |
| `src/profile.rs` | builds your profile (distro, kernel, picture…) |
| `src/proto.rs` | the format of the messages exchanged between machines |
| `src/daemon.rs` | the service: listens, contacts friends, writes the state, notifications |
| `src/widget.rs` | starts and stops the universal widget |
| `src/update.rs` | checks GitHub for new releases |
| `noctalia/archmates/` | the Noctalia desktop widget (Luau) |
| `widget/archmates-widget` | the universal desktop widget (Python + GTK4) |
| `packaging/archmates.service` | the systemd user unit |

## Prototype limitations

- A directly connected friend can see your IP address (normal for peer to peer).
- If you lose `secret.key`, your code changes: back up that file.
- No "playing… / listening to…" status yet, no AUR package yet.
