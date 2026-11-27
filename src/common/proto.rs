//! Wire protocol between two machines.
//!
//! Each exchange: one encrypted QUIC connection (Iroh), one bidirectional
//! stream, one JSON request, one response. Only mutual friends are accepted:
//! the connection is authenticated by the other machine's public key.

use serde::{Deserialize, Serialize};

use crate::common::profile::Profile;

/// Protocol identifier (must match on both sides).
pub const ALPN: &[u8] = b"archmates/presence/1";

/// Maximum size of a JSON request or response.
pub const JSON_MAX: usize = 64 * 1024;

/// Close code used when the other side has not added us (yet).
pub const CODE_NOT_FRIEND: u32 = 403;

/// Close code used when the other side is in invisible mode: shown as "offline", not as
/// "hasn't added you yet" (older versions closed with 403 in both cases).
pub const CODE_UNAVAILABLE: u32 = 503;

/// Refuses a connection: 503 while invisible, 403 for someone not in the friends list.
pub fn refuse(connection: &iroh::endpoint::Connection, invisible: bool) {
    if invisible {
        connection.close(CODE_UNAVAILABLE.into(), b"unavailable");
    } else {
        connection.close(CODE_NOT_FRIEND.into(), b"not a friend");
    }
}

/// The close code the other side gave, if it closed the connection on purpose.
pub fn close_code(connection: &iroh::endpoint::Connection) -> Option<u64> {
    use iroh::endpoint::{ApplicationClose, ConnectionError};
    match connection.close_reason()? {
        ConnectionError::ApplicationClosed(ApplicationClose { error_code, .. }) => Some(error_code.into_inner()),
        _ => None,
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// "I'm online, here is my profile. How about you?"
    Hello { profile: Profile },
    /// "Send me your profile picture." Response: the raw image bytes.
    Avatar,
    /// A chat message. Response: `ok`.
    Message { id: String, text: String, ms: u64 },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub profile: Profile,
}
