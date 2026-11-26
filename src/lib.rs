//! archmates: peer-to-peer friends list for Linux and Windows.
//!
//! The Windows client is called Winmates. The same code runs on both systems and speaks the same protocol, so a friend on Windows
//! and a friend on Linux see each other, chat and send files exactly the same way.
//! What differs per system lives in `platform.rs` (notifications, start with the system…).

pub mod chat;
pub mod config;
pub mod daemon;
pub mod ipc;
pub mod platform;
pub mod profile;
pub mod proto;
pub mod stats;
pub mod transfer;
pub mod update;
pub mod widget;
