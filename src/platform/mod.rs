//! Everything that differs between Linux and Windows lives here, behind one set of functions
//! with the same names on both systems:
//!
//! | function                         | Linux (`linux/`)                  | Windows (`windows/`)              |
//! |----------------------------------|-----------------------------------|-----------------------------------|
//! | `command`                        | plain `Command`                   | no console window                 |
//! | `notify`, `ask`                  | `notify-send`                     | toast notifications               |
//! | `autostart_enabled/set_autostart`| systemd user service              | registry "Run" key                |
//! | `download_dir`                   | XDG download folder               | Explorer's Downloads folder       |
//! | `memory_usage`, `disk_usage`     | `/proc/meminfo`, `df`             | Win32 API                         |
//! | `system_info`, `avatar_candidates`| os-release, AccountsService…     | registry, account picture         |
//! | `install_release`                | runs the archive's `installer.sh` | replaces the programs in place    |
//!
//! The rest of the code (protocol, friends, chat, files, voice) is shared, so Linux and Windows
//! friends work together. Desktop integrations that only exist on Linux (the universal widget
//! supervisor) are in `linux/`; the Windows installer and registry helpers are in `windows/`.

use std::path::PathBuf;

#[cfg(not(windows))]
pub mod linux;
#[cfg(not(windows))]
pub use linux::*;

#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::*;

/// What a click on a notification does.
#[derive(Debug, Clone)]
pub enum OnClick {
    Nothing,
    /// Opens the conversation with this friend (friend code).
    OpenChat(String),
    /// Opens a file or folder.
    OpenPath(PathBuf),
}

/// What a friend sees about your system (the "fetch" lines of your profile).
#[derive(Debug, Clone, Default)]
pub struct SystemInfo {
    pub host: String,
    pub distro: String,
    pub kernel: String,
    pub wm: String,
    pub terminal: String,
    pub shell: String,
    pub uptime_s: u64,
}

/// Your home folder ($HOME on Linux, %USERPROFILE% on Windows).
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Your user name ($ARCHMATES_USER overrides it, for test instances).
pub fn user_name() -> String {
    std::env::var("ARCHMATES_USER")
        .or_else(|_| std::env::var("USER"))
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "?".into())
}
