//! archmates: peer-to-peer friends list for Linux and Windows.
//!
//! The Windows client is called Winmates. The same code runs on both systems and speaks the
//! same protocol, so a friend on Windows and a friend on Linux see each other, chat and send
//! files exactly the same way.
//!
//! Layout:
//! - `common`   — shared core: identity, friends, profile, protocol, chat, files, voice, updates
//! - `platform` — what differs per system: `platform::linux` and `platform::windows`
//! - `service`  — the background service (`archmates daemon`) and how apps start it
//!
//! Front-ends live outside the library: the `archmates` command (`src/main.rs`, `src/cli/`), Winmates (`src/bin/winmates/`), the Noctalia
//! plugin (`desktop/noctalia/`) and the universal Linux widget (`desktop/linux/`).

pub mod common;
pub mod platform;
pub mod service;

// Short paths for the front-ends: `archmates::chat`, `archmates::config`…
pub use common::{chat, config, ipc, profile, proto, stats, time, transfer, update, voice};
