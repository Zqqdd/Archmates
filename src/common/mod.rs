//! The shared core, identical on Linux and Windows: identity and friends list, profile,
//! protocol, chat, file transfers, voice messages, local control socket and updates.
//! Nothing here depends on the operating system: that lives in `crate::platform`.

pub mod chat;
pub mod config;
pub mod ipc;
pub mod profile;
pub mod proto;
pub mod stats;
pub mod time;
pub mod transfer;
pub mod update;
pub mod voice;
