//! The voice desk: what the service is recording or playing, for every front-end.
//!
//! The service owns the microphone and the speakers (see `service/voice_desk.rs`), so Winmates,
//! the Linux widget, the Noctalia panel and the command line all record and play the same way:
//!
//!   idle → recording → review (listen, then send or discard) → sending → idle
//!
//! While something happens, the service rewrites `voice-desk.json` in the cache folder about
//! ten times a second (elapsed time, microphone level, playback position). Front-ends read that
//! file to draw the recorder; they send their requests through `ipc` (`VoiceRecord`…).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::common::config::Paths;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeskState {
    #[default]
    Idle,
    /// Recording from the microphone (5 minutes max: then it stops by itself into `Review`).
    Recording,
    /// Recorded, waiting to be listened to, sent or discarded.
    Review,
    /// Being sent to the friend (back to `Review` with an `error` if that fails).
    Sending,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Playing {
    /// The chat message id, or "draft" for the recording under review.
    pub id: String,
    pub position: f32,
    pub secs: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeskStatus {
    pub state: DeskState,
    /// Friend code the recording is for.
    #[serde(default)]
    pub to: Option<String>,
    /// Seconds recorded so far (recording), or the recording's length (review / sending).
    #[serde(default)]
    pub secs: f32,
    /// Microphone level, 0..1 (recording only).
    #[serde(default)]
    pub level: f32,
    /// The recording under review (review / sending).
    #[serde(default)]
    pub draft: Option<String>,
    #[serde(default)]
    pub playing: Option<Playing>,
    /// Why the last request failed (shown under the recorder).
    #[serde(default)]
    pub error: Option<String>,
    /// When the service wrote this: older than a few seconds means the service stopped.
    #[serde(default)]
    pub ms: u64,
}

/// Id used in `Playing` for the recording under review.
pub const DRAFT_ID: &str = "draft";

pub fn status_path(paths: &Paths) -> PathBuf {
    paths.cache.join("voice-desk.json")
}

impl DeskStatus {
    /// The service's current status (idle when the file is missing, unreadable or stale).
    pub fn read(paths: &Paths) -> Self {
        let status: Self = std::fs::read_to_string(status_path(paths))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        if crate::common::time::now_ms().saturating_sub(status.ms) > 5_000 { Self::default() } else { status }
    }

    /// True when the recorder belongs to this friend's conversation.
    pub fn is_for(&self, code: &str) -> bool {
        self.state != DeskState::Idle && self.to.as_deref() == Some(code)
    }

    /// Playback position (0..1) of this message, if it is the one playing.
    pub fn progress_of(&self, id: &str) -> Option<f32> {
        self.playing.as_ref().filter(|p| p.id == id).map(|p| (p.position / p.secs.max(0.1)).clamp(0.0, 1.0))
    }
}
