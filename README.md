# archmates

> 🤖 **This project was almost entirely written by [Claude](https://claude.ai), Anthropic's AI assistant**
> (through Claude Code). The repository owner directed the idea, the design choices and the testing.

A **peer-to-peer** friends list for CachyOS / Arch Linux **and Windows**: see when your friends are online, with
their profile (user, host, distro, kernel, WM…) and their picture, right on your Noctalia desktop.

On Windows the client is called **Winmates**. Both run the same code and speak the same protocol: a friend on
Windows and a friend on Linux see each other, chat and send files to each other exactly as two Linux users do.

No accounts, no servers of your own: your friends list stays on your machine, and machines talk to each other
directly over encrypted connections, powered by [Iroh](https://iroh.computer).

## Install

### Windows (Winmates)

Download **`WinmatesSetup-<version>.exe`** from the
[releases page](https://github.com/Zqqdd/Archmates/releases/latest) and run it. No admin rights needed:
it installs Winmates for your account in `%LOCALAPPDATA%\Programs\Winmates`, adds it to the Start menu
(and the desktop if you want), and to *Settings → Apps* to uninstall it.

Winmates looks like the Linux rice: a small frosted-glass card with your friends (acrylic blur, your Windows
accent color or your pywal / winwal colors), and a friend panel in the style of the Noctalia panel.

- **WinmatesUpdater** starts with Windows at every login. It installs a new release if there is one
  (SHA-256 checked), starts the service, then opens Winmates (*Settings → Open Winmates when I log in* to turn
  that off). Winmates also checks for an update each time you open it.
  Set `ARCHMATES_NO_UPDATE_CHECK=1` to turn updates off.
- **Closing the window keeps Winmates running** in the background: your friends still see you online and you
  still get notifications. *Settings → Quit Winmates* stops it completely.
- Your data is in `%APPDATA%\Winmates` (friend code, friends list, conversations), received files in
  `Downloads\Winmates\<friend>\`.
- The `archmates` command is installed next to it, with the same commands as on Linux:
  `%LOCALAPPDATA%\Programs\Winmates\archmates.exe list`.
- Windows may show a firewall prompt the first time: allowing it lets friends reach you directly instead of
  through a relay.

To uninstall: *Settings → Apps → Winmates → Uninstall*. Your friend code, friends list and conversations
(`%APPDATA%\Winmates`) are kept, so you can reinstall later without adding your friends again.

The zip (`winmates-<version>-x86_64-windows.zip`) still works too: unzip it and run `winmates.exe`, it installs
itself the same way. It is also what WinmatesUpdater downloads.

### Linux: from a release archive (recommended, no Rust needed)

Download `archmates-<version>-x86_64-linux.tar.gz` from the
[releases page](https://github.com/Zqqdd/Archmates/releases), then:

```bash
tar -xzf archmates-*-x86_64-linux.tar.gz && cd archmates-*-x86_64-linux && ./installer.sh
```

### Linux: from source

You need Rust (`sudo pacman -S rustup && rustup default stable`) and `alsa-lib` (for voice messages, almost
always installed already), then:

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
archmates send <nickname> <text>   # send a message (delivered when they are online)
archmates send-file <nickname> <f> # send a file (they must be online)
archmates chat <nickname>          # show the conversation
archmates stats on|off             # share your RAM and storage usage (on by default)
archmates avatar [<image>]         # show or set your profile picture
archmates update [--install]       # check for a new version (and install it)
```

To place the widget on your desktop: `noctalia msg desktop-widgets-edit`.

### Messages, files and stats

Click a friend in the widget (Noctalia or universal) to open their panel: their profile, their RAM and storage
usage, your conversation, a message field and a "send a file" button.

- **Messages** go directly from PC to PC, encrypted. A message written while your friend is offline waits on
  your machine and is delivered as soon as they come back. Conversations are stored locally in
  `~/.local/share/archmates/chat/`.
- **Files** (up to 4 GB) are sent directly too, while your friend is online. **Nothing is transferred until
  your friend accepts**: they get a notification and an offer in the chat with Accept / Decline buttons
  (or `archmates offers`, `archmates accept <id>`, `archmates decline <id>`). Offers expire after 5 minutes.
  Accepted files land in `~/Downloads/Archmates/<friend>/` (your XDG download folder).
- **Pictures** you send or receive are shown directly in the conversation. In Winmates (and the desktop app) and
  in the universal widget, they are **blurred automatically whenever the chat is not the active window**, and
  sharp again as soon as you come back to it.
- **Voice messages** (up to 5 minutes): 🎤 in Winmates / the desktop app, or `archmates voice send <friend>`.
  They are sent directly while your friend is online, play in Winmates, the universal widget and the Noctalia
  panel (`archmates voice play <file>`), and are **deleted at every restart**, on both sides.
- **Delete a conversation**: ⚙ in the friend panel (Winmates, universal widget, Noctalia) or
  `archmates delete-chat <friend>`. It deletes every message and voice message, and every file that friend sent
  you (their folder in your downloads), for good. Files you sent stay where they are.
- **RAM and storage** usage is shared with your friends only. Turn it off with `archmates stats off`.

### Profile picture

Your system account picture is sent automatically: the one set in KDE / GNOME settings, Noctalia, or any
tool using AccountsService, or else `~/.face` / `~/.face.icon`. Large photos are cropped and shrunk to a
256×256 picture before being sent. To use another picture just for archmates:

```bash
archmates avatar ~/Pictures/me.png   # PNG, JPEG, WebP or GIF
archmates avatar                     # shows which picture is used
archmates avatar --remove            # back to the system picture
```

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

**Drag the widget to move it**: its position is remembered. Clicking a friend opens a small floating chat
panel above your windows, which you can move by dragging its header (Esc closes it).

Or set the position by hand in `~/.config/archmates/widget.toml`:

```toml
corner = "top-right"   # top-left, top-right, bottom-left, bottom-right
margin_x = 40
margin_y = 60
output = "DP-1"        # screen to use (default: the main screen)
```

Its colors follow your theme automatically, and update live when it changes: your pywal / wallust colors if
you use them (`~/.cache/wal/colors.json`), otherwise your GTK theme (including libadwaita and
matugen-generated colors). For full control, add `~/.config/archmates/widget.css` (GTK CSS, loaded after the
default style; `@bg`, `@fg` and `@accent` hold the theme colors).

### Desktop app on Linux (optional)

Winmates' window also runs on Linux, named "Archmates" there, next to the widgets:

```bash
cargo build --release --features app && ./target/release/winmates
```

### Try it alone

No friend at hand? Start a fake friend on your own machine:

```bash
./scripts/demo-friend.sh
```

It shows up online in the widget and in `archmates list` within ~30 s. Press Ctrl+C to stop it: it is
removed from your list automatically.

## Updating

Your friend code, your friends list and your settings are kept: you never need to add your friends again.

### One command (v0.2.4 and later)

```bash
archmates update --install
```

It downloads the latest release from GitHub, checks its SHA-256 checksum, and runs the installer it contains.
Nothing is installed if the checksum does not match. No password is needed. The service restarts by itself
with the new version.

To only check whether a new version exists: `archmates update`.

### Coming from an older version (v0.2.3 or before)

Those versions do not have `--install` yet: update by hand once, and use the command above afterwards.

1. Download the latest `archmates-<version>-x86_64-linux.tar.gz` from the
   [latest release](https://github.com/Zqqdd/Archmates/releases/latest).
2. In the folder where you downloaded it, run:

   ```bash
   tar -xzf archmates-*-x86_64-linux.tar.gz
   cd archmates-*-x86_64-linux
   ./installer.sh
   ```

> **Installed v0.2.0 with a service you created by hand?** Run `systemctl --user disable archmates` once
> before `./installer.sh`, so the installer can put the right service in place.

### You installed from source (git clone)

```bash
cd Archmates          # the folder you cloned
git pull
./installer.sh
```

### What happens automatically

- Once a day, the service checks GitHub for a new release. When there is one, you get a notification and the
  widget shows "Update …: archmates update --install". Nothing is installed without you running that command.
  Set `ARCHMATES_NO_UPDATE_CHECK=1` to turn the check off.
- After an update, the running service notices that its program was replaced and restarts itself
  (within ~30 s): there is nothing to restart by hand.

### AUR

New AUR package submissions are currently closed. A ready `PKGBUILD` for `archmates-bin` is kept in
`packaging/aur/` for when they reopen.

### Publishing a new version (maintainers)

Bump the version in `Cargo.toml` (and `noctalia/archmates/plugin.toml`), run `cargo build` to refresh
`Cargo.lock`, commit, then push a tag. GitHub Actions builds and publishes the Linux archive and the
Winmates archive (built on a Windows runner):

```bash
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
| `src/bin/winmates.rs` | Winmates, the Windows client (also a desktop app on Linux) |
| `src/bin/winmates-updater.rs` | WinmatesUpdater: started at every Windows login, updates and starts everything |
| `src/bin/winmates-setup.rs` | WinmatesSetup.exe, the installer (the programs are embedded in it) |
| `src/ui/theme.rs` | the Archmates look for Winmates: colors, frosted glass, card |
| `src/launcher.rs` | update, service, install and uninstall, shared by the three Windows programs |
| `src/voice.rs` | voice messages: recording, playback, sending (`archmates/voice/1`) |
| `src/platform.rs` | what differs between Linux and Windows: notifications, start with the system… |
| `src/config.rs` | local files: key, friends list, invisible mode |
| `src/profile.rs` | builds your profile (distro, kernel, picture…) |
| `src/proto.rs` | the format of the messages exchanged between machines |
| `src/daemon.rs` | the service: listens, contacts friends, writes the state, notifications |
| `src/chat.rs` | conversations and the offline message queue |
| `src/transfer.rs` | file transfers (own protocol, `archmates/file/1`) |
| `src/ipc.rs` | local socket (named pipe on Windows) used by the CLI, widgets and Winmates to reach the service |
| `src/stats.rs` | RAM and storage usage |
| `noctalia/archmates/friend.luau` | the Noctalia friend panel |
| `src/widget.rs` | starts and stops the universal widget |
| `src/update.rs` | checks GitHub for new releases |
| `noctalia/archmates/` | the Noctalia desktop widget (Luau) |
| `widget/archmates-widget` | the universal desktop widget (Python + GTK4) |
| `packaging/archmates.service` | the systemd user unit |

## Prototype limitations

- A directly connected friend can see your IP address (normal for peer to peer).
- If you lose `secret.key`, your code changes: back up that file.
- No "playing… / listening to…" status yet, no AUR package yet.
