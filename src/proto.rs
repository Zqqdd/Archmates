//! Wire protocol between two machines.
//!
//! Each exchange: one encrypted QUIC connection (Iroh), one bidirectional
//! stream, one JSON request, one response. Only mutual friends are accepted:
//! the connection is authenticated by the other machine's public key.

use serde::{Deserialize, Serialize};

use crate::profile::Profile;

/// Protocol identifier (must match on both sides).
pub const ALPN: &[u8] = b"archmates/presence/1";

/// Maximum size of a JSON request or response.
pub const JSON_MAX: usize = 64 * 1024;

/// Close code used when the other side has not added us (yet).
pub const CODE_NOT_FRIEND: u32 = 403;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// "I'm online, here is my profile. How about you?"
    Hello { profile: Profile },
    /// "Send me your profile picture." Response: the raw image bytes.
    Avatar,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub profile: Profile,
}
