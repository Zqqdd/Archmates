//! The `archmates` command: argument parsing here, one file per topic.
//!
//!   archmates code                    show your friend code to share
//!   archmates add <code> <nickname>   add a friend (they must add you too)
//!   archmates remove <nickname|code>  remove a friend
//!   archmates list                    show your friends and their status
//!   archmates invisible on|off        invisible mode
//!   archmates profile                 show what your friends see of you
//!   archmates send <friend> <text>    send a message (delivered when they are online)
//!   archmates send-file <friend> <f>  send a file (your friend must be online)
//!   archmates chat <friend>           show the conversation
//!   archmates delete-chat <friend>    delete a conversation for good, with the files they sent you
//!   archmates voice send <friend>     record and send a voice message (5 min max)
//!   archmates voice play <file>       play a voice message
//!   archmates stats on|off            share your RAM and storage usage with friends
//!   archmates avatar [<image>]        show or set your profile picture
//!   archmates widget auto|on|off      universal desktop widget (auto = when Noctalia is absent)
//!   archmates update [--install]      check for a new version (and install it)
//!   archmates daemon                  run the service (systemd on Linux, Winmates on Windows)

mod friends;
mod messages;
mod profile;
mod system;
mod voice;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use archmates::config::Paths;

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
    /// Send a message to a friend (delivered as soon as they are online)
    Send {
        #[arg(value_name = "NICKNAME|CODE")]
        to: String,
        /// The message (quotes are optional)
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
    },
    /// Send a file to a friend (they must be online)
    SendFile {
        #[arg(value_name = "NICKNAME|CODE")]
        to: String,
        file: std::path::PathBuf,
    },
    /// Accept a file a friend wants to send you (id shown in `archmates offers`)
    Accept { id: String },
    /// Decline a file a friend wants to send you
    Decline { id: String },
    /// List files waiting for your answer
    Offers,
    /// Show the conversation with a friend (and mark it as read)
    Chat {
        #[arg(value_name = "NICKNAME|CODE")]
        with: String,
        /// Number of messages to show
        #[arg(short = 'n', long, default_value_t = 20)]
        count: usize,
        /// Print as JSON (used by the widgets)
        #[arg(long)]
        json: bool,
    },
    /// Delete a conversation for good, with every file this friend sent you
    DeleteChat {
        #[arg(value_name = "NICKNAME|CODE")]
        with: String,
        /// Don't ask for confirmation
        #[arg(long)]
        yes: bool,
    },
    /// Voice messages (up to 5 minutes, deleted at every restart)
    #[command(subcommand)]
    Voice(VoiceCommand),
    /// Share your RAM and storage usage with your friends (on by default)
    Stats {
        #[arg(value_enum)]
        state: OnOff,
    },
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

#[derive(Subcommand)]
enum VoiceCommand {
    /// Record from your microphone (press Enter to stop, 5 min max) and send it
    Send {
        #[arg(value_name = "NICKNAME|CODE")]
        to: String,
    },
    /// Play a voice message (the file shown in the conversation)
    Play { file: std::path::PathBuf },
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

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::detect();
    match cli.command {
        Command::Code => friends::code(&paths),
        Command::Add { code, nickname } => friends::add(&paths, code, nickname),
        Command::Remove { who } => friends::remove(&paths, who),
        Command::List => friends::list(&paths),
        Command::Invisible { state } => friends::invisible(&paths, matches!(state, OnOff::On)),
        Command::Profile => profile::show(),
        Command::Send { to, text } => messages::send(to, text).await,
        Command::SendFile { to, file } => messages::send_file(to, file).await,
        Command::Accept { id } => messages::answer(id, true).await,
        Command::Decline { id } => messages::answer(id, false).await,
        Command::Offers => messages::offers(&paths),
        Command::Chat { with, count, json } => messages::chat(&paths, with, count, json),
        Command::DeleteChat { with, yes } => messages::delete_chat(&paths, with, yes),
        Command::Voice(VoiceCommand::Send { to }) => voice::send(to).await,
        Command::Voice(VoiceCommand::Play { file }) => voice::play(&file),
        Command::Stats { state } => profile::stats(&paths, matches!(state, OnOff::On)),
        Command::Avatar { image, remove } => profile::avatar(&paths, image, remove),
        Command::Widget { mode } => system::widget(
            &paths,
            mode.map(|m| match m {
                WidgetMode::Auto => "auto",
                WidgetMode::On => "on",
                WidgetMode::Off => "off",
            }),
        ),
        Command::Update { install } => system::update(install).await,
        Command::Daemon => archmates::service::run().await,
    }
}
