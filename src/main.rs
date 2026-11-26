//! archmates: peer-to-peer friends list for CachyOS / Arch Linux.
//!
//!   archmates code                    show your friend code to share
//!   archmates add <code> <nickname>   add a friend (they must add you too)
//!   archmates remove <nickname|code>  remove a friend
//!   archmates list                    show your friends and their status
//!   archmates invisible on|off        invisible mode
//!   archmates profile                 show what your friends see of you
//!   archmates avatar [<image>]        show or set your profile picture
//!   archmates widget auto|on|off      universal desktop widget (auto = when Noctalia is absent)
//!   archmates update [--install]      check for a new version (and install it)
//!   archmates daemon                  run the service (normally via systemd)

mod config;
mod daemon;
mod profile;
mod proto;
mod update;
mod widget;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use config::{FriendList, Paths, load_or_create_key};

#[derive(Parser)]
#[command(name = "archmates", version, about = "Peer-to-peer friends list: see who is online.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show your friend code, to share with your friends
    Code,
    /// Add a friend from their code
    Add {
        /// The friend code you received
        code: String,
        /// The nickname shown for this friend
        nickname: String,
    },
    /// Remove a friend (by nickname or code)
    Remove {
        /// Friend's nickname or code
        #[arg(value_name = "NICKNAME|CODE")]
        who: String,
    },
    /// Show your friends and their status
    List,
    /// Turn invisible mode on or off
    Invisible {
        #[arg(value_enum)]
        state: OnOff,
    },
    /// Show your profile as your friends see it
    Profile,
    /// Show or set your profile picture (default: your system account picture)
    Avatar {
        /// Image file to use; without it, shows the current picture
        image: Option<std::path::PathBuf>,
        /// Remove the picture set with this command
        #[arg(long, conflicts_with = "image")]
        remove: bool,
    },
    /// Universal desktop widget: auto (when Noctalia is absent), on or off
    Widget {
        #[arg(value_enum)]
        mode: Option<WidgetMode>,
    },
    /// Check whether a new version is available (--install: download and install it)
    Update {
        /// Download the latest release, check it and install it
        #[arg(long)]
        install: bool,
    },
    /// Run the presence service (in the foreground)
    Daemon,
}

#[derive(Clone, Copy, ValueEnum)]
enum WidgetMode {
    Auto,
    On,
    Off,
}

#[derive(Clone, Copy, ValueEnum)]
enum OnOff {
    On,
    Off,
}

fn ago(ms: u64) -> String {
    let s = daemon::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::detect();

    match cli.command {
        Command::Code => {
            let key = load_or_create_key(&paths)?;
            println!("{}", key.public());
            eprintln!("\nShare this code with your friends. They add you with:\n  archmates add <code> <nickname>");
        }
        Command::Add { code, nickname } => {
            let mut list = FriendList::load(&paths)?;
            list.add(&code, &nickname)?;
            list.save(&paths)?;
            println!("✓ {nickname} added.");
            println!("  They will show up online once they add you back (run `archmates code` and send them your code).");
        }
        Command::Remove { who } => {
            let mut list = FriendList::load(&paths)?;
            if list.remove(&who) {
                list.save(&paths)?;
                println!("✓ {who} removed.");
            } else {
                println!("No friend named \"{who}\".");
            }
        }
        Command::List => {
            let list = FriendList::load(&paths)?;
            if list.friends.is_empty() {
                println!("No friends yet. Add one with: archmates add <code> <nickname>");
                return Ok(());
            }
            let state: Option<daemon::StateFile> = std::fs::read_to_string(paths.state())
                .ok()
                .and_then(|t| serde_json::from_str(&t).ok());
            if state.is_none() {
                println!("(the service is not running: status unknown)\n");
            }
            for friend in &list.friends {
                let f = state.as_ref().and_then(|s| s.friends.iter().find(|f| f.code == friend.code));
                let (dot, text) = match f.map(|f| f.status) {
                    Some(daemon::Status::Online) => ("\x1b[32m●\x1b[0m", "online".to_string()),
                    Some(daemon::Status::Pending) => ("\x1b[33m●\x1b[0m", "hasn't added you yet".to_string()),
                    _ => (
                        "\x1b[90m●\x1b[0m",
                        f.and_then(|f| f.last_seen_ms)
                            .map(|v| format!("offline, last seen {}", ago(v)))
                            .unwrap_or("offline".into()),
                    ),
                };
                let detail = f
                    .and_then(|f| f.profile.as_ref())
                    .map(|p| format!("  {}@{} · {} · {}", p.user, p.host, p.distro, p.wm))
                    .unwrap_or_default();
                println!("{dot} {:<16} {text}{detail}", friend.nickname);
            }
            if state.is_some_and(|s| s.me.invisible) {
                println!("\n(you are in invisible mode)");
            }
        }
        Command::Invisible { state } => match state {
            OnOff::On => {
                std::fs::create_dir_all(&paths.data)?;
                std::fs::write(paths.invisible(), b"")?;
                println!("✓ Invisible mode on: your friends see you as offline.");
            }
            OnOff::Off => {
                let _ = std::fs::remove_file(paths.invisible());
                println!("✓ Invisible mode off.");
            }
        },
        Command::Profile => {
            println!("{}", serde_json::to_string_pretty(&profile::Profile::local())?);
        }
        Command::Avatar { image, remove } => {
            if remove {
                let _ = std::fs::remove_file(paths.avatar());
                println!("✓ Profile picture removed.");
            } else if let Some(image) = image {
                let bytes = std::fs::read(&image)
                    .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", image.display()))?;
                let Some(kind) = profile::image_kind(&bytes) else {
                    anyhow::bail!("{} is not a PNG, JPEG, WebP or GIF image", image.display());
                };
                if bytes.len() > 30 * 1024 * 1024 {
                    anyhow::bail!("the image is larger than 30 MB");
                }
                config::write_atomic(&paths.avatar(), &bytes)?;
                println!("✓ Profile picture set ({kind}, {} KB).", bytes.len() / 1024);
                println!("  Your friends get it within a minute.");
            }
            let user = profile::Profile::local().user;
            match profile::avatar_path(&user) {
                Some(p) => match profile::read_avatar(&user) {
                    Some(sent) => println!(
                        "Current picture: {}\n  Sent to your friends as a 256×256 image ({} KB).",
                        p.display(),
                        sent.len() / 1024
                    ),
                    None => println!("Found {}, but it is not a readable image.", p.display()),
                },
                None => println!(
                    "No profile picture found (system account picture, ~/.face…).\n  Set one with: archmates avatar <image>"
                ),
            }
        }
        Command::Widget { mode } => {
            if let Some(mode) = mode {
                let text = match mode {
                    WidgetMode::Auto => "auto",
                    WidgetMode::On => "on",
                    WidgetMode::Off => "off",
                };
                std::fs::create_dir_all(&paths.data)?;
                std::fs::write(paths.widget_mode(), text)?;
            }
            let mode = widget::Mode::load(&paths);
            let shown = widget::should_show(mode);
            println!("Universal widget: {} ({})", mode.as_str(), if shown { "shown" } else { "hidden" });
            match widget::program() {
                None => println!("  archmates-widget is not installed."),
                Some(p) if shown => println!("  The service starts {} within 15 s.", p.display()),
                Some(_) if mode == widget::Mode::Auto => println!("  Hidden because Noctalia is running (it has its own widget)."),
                Some(_) => {}
            }
        }
        Command::Update { install } => {
            println!("Installed version: {}", update::CURRENT);
            let latest = update::latest_version().await?;
            let newer = update::is_newer(&latest, update::CURRENT);
            // ARCHMATES_UPDATE_FORCE=1 reinstalls the latest release even when up to date (testing).
            let force = std::env::var_os("ARCHMATES_UPDATE_FORCE").is_some();
            if !install {
                if newer {
                    println!("✓ Version {latest} is available. Update with:\n  {}", update::how_to_update());
                } else {
                    println!("✓ You are up to date (latest release: {latest}).");
                }
            } else if update::managed_by_pacman() {
                println!("archmates is managed by pacman here: update it with  sudo pacman -Syu");
            } else if newer || force {
                update::install(&latest).await?;
                println!("\n✓ archmates {latest} installed. The service restarts by itself.");
            } else {
                println!("✓ You are up to date (latest release: {latest}).");
            }
        }
        Command::Daemon => daemon::run().await?,
    }
    Ok(())
}
