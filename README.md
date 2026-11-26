# archmates

> 🤖 **This project was almost entirely written by [Claude](https://claude.ai), Anthropic's AI assistant**
> (through Claude Code). The repository owner directed the idea, the design choices and the testing.

A **peer-to-peer** friends list for CachyOS / Arch Linux: see when your friends are online, with their profile
(user, host, distro, kernel, WM…) and their picture, right on your Noctalia desktop.

No accounts, no servers of your own: your friends list stays on your machine, and machines talk to each other
directly over encrypted connections, powered by [Iroh](https://iroh.computer).

## Install

You need Rust (`sudo pacman -S rustup && rustup default stable`), then:

```bash
./installer.sh
```

The script does everything, with no password needed:

- builds the project and installs the `archmates` command to `~/.local/bin`;
- starts the presence service (systemd user unit) at every login;
- if you use **Noctalia**: installs the "Archmates" widget and places it at the top right of your main screen,
  without touching your other widgets (move it later with `noctalia msg desktop-widgets-edit`);
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
```

To place the widget on your desktop: `noctalia msg desktop-widgets-edit`.

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
| `noctalia/archmates/` | the desktop widget (Luau) |
| `packaging/archmates.service` | the systemd user unit |

## Prototype limitations

- A directly connected friend can see your IP address (normal for peer to peer).
- If you lose `secret.key`, your code changes: back up that file.
- No "playing… / listening to…" status yet, no AUR package yet.
