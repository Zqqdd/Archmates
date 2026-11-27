//! Voice messages: up to 5 minutes, sent directly from PC to PC, **deleted at every restart**.
//!
//! - Recorded as 16 kHz mono WAV (about 1.9 MB per minute), no extra codec needed.
//! - Sent over their own protocol (`archmates/voice/1`), only while the friend is online.
//!   No acceptance step (unlike files): it is a small audio clip from a friend.
//! - Stored in the cache folder (`voice/`), which the service empties each time it starts:
//!   after a reboot (or a restart of the service), every voice message is gone, on both sides.
//!   The conversation keeps a "voice message (expired)" line.
//!
//! Stream layout:
//!   sender   → `{"id": "...", "secs": 42, "size": 1234, "ms": ...}\n` then the WAV bytes
//!   receiver → `ok` once saved (or an error text)

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use crate::common::config::{FriendList, Paths};

pub const VOICE_ALPN: &[u8] = b"archmates/voice/1";
/// Longest voice message.
pub const MAX_SECS: u32 = 300;
/// Recording rate (Hz): plenty for speech, small files.
pub const SAMPLE_RATE: u32 = 16_000;
/// Largest voice file accepted: 5 min of 16 kHz mono 16-bit, plus room for the header.
pub const VOICE_MAX: u64 = (MAX_SECS as u64 + 2) * SAMPLE_RATE as u64 * 2 + 4096;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    id: String,
    secs: u32,
    size: u64,
    ms: u64,
}

/// Where voice messages live until the next restart.
pub fn dir(paths: &Paths) -> PathBuf {
    paths.cache.join("voice")
}

/// Deletes every voice message (called when the service starts).
pub fn clear(paths: &Paths) {
    let _ = std::fs::remove_dir_all(dir(paths));
}

/// "0:42"
pub fn format_secs(secs: u32) -> String {
    format!("{}:{:02}", secs / 60, secs % 60)
}

/// Length of a WAV file in seconds, if it is a valid one.
pub fn wav_secs(bytes: &[u8]) -> Option<u32> {
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes)).ok()?;
    let spec = reader.spec();
    if spec.sample_rate == 0 || spec.channels == 0 {
        return None;
    }
    Some(reader.duration() / spec.sample_rate)
}

/// Sends a recorded voice message (WAV bytes) to a friend who is online.
pub async fn send(endpoint: &Endpoint, to: EndpointId, id: &str, wav: &[u8], ms: u64) -> Result<u32> {
    let secs = wav_secs(wav).ok_or_else(|| anyhow!("not a valid recording"))?;
    if secs > MAX_SECS + 1 || wav.len() as u64 > VOICE_MAX {
        bail!("voice messages are limited to 5 minutes");
    }
    let connection = tokio::time::timeout(std::time::Duration::from_secs(20), endpoint.connect(to, VOICE_ALPN))
        .await
        .map_err(|_| anyhow!("your friend is offline: voice messages are only sent while they are online"))?
        .map_err(|e| anyhow!("your friend can't receive voice messages (offline, or an older Archmates): {e}"))?;
    let (mut send, recv) = connection.open_bi().await?;
    let header = serde_json::to_string(&Header { id: id.to_string(), secs, size: wav.len() as u64, ms })?;
    send.write_all(header.as_bytes()).await?;
    send.write_all(b"\n").await?;
    send.write_all(wav).await?;
    send.finish()?;
    let mut answer = Vec::new();
    BufReader::new(recv).take(256).read_to_end(&mut answer).await?;
    connection.close(0u32.into(), b"ok");
    if answer != b"ok" {
        bail!("your friend refused the voice message: {}", String::from_utf8_lossy(&answer));
    }
    Ok(secs)
}

/// Called once a voice message is saved (friend, message id, seconds, saved path, sent time).
pub type ReceivedFn = dyn Fn(EndpointId, String, u32, PathBuf, u64) + Send + Sync;

#[derive(Clone)]
pub struct VoiceReceiver {
    pub on_received: Arc<ReceivedFn>,
}

impl std::fmt::Debug for VoiceReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VoiceReceiver")
    }
}

impl ProtocolHandler for VoiceReceiver {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id();
        let paths = Paths::detect();
        let is_friend = FriendList::load(&paths).map(|l| l.by_id().contains_key(&id)).unwrap_or(false);
        if !is_friend || paths.is_invisible() {
            connection.close(crate::common::proto::CODE_NOT_FRIEND.into(), b"not a friend");
            return Ok(());
        }
        match receive(&connection, id, &paths).await {
            Ok((msg_id, secs, saved, ms)) => (self.on_received)(id, msg_id, secs, saved, ms),
            Err(err) => eprintln!("[voice] {err:#}"),
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
}

async fn receive(connection: &Connection, from: EndpointId, paths: &Paths) -> Result<(String, u32, PathBuf, u64)> {
    let (mut send, recv) = connection.accept_bi().await?;
    let mut reader = BufReader::new(recv);
    let mut line = Vec::new();
    (&mut reader).take(4096).read_until(b'\n', &mut line).await?;
    let header: Header = serde_json::from_slice(line.trim_ascii_end()).context("bad voice header")?;
    if header.size > VOICE_MAX {
        send.write_all(b"too long").await?;
        send.finish()?;
        bail!("refused a voice message over 5 minutes");
    }
    let mut wav = Vec::with_capacity(header.size as usize);
    (&mut reader).take(header.size).read_to_end(&mut wav).await?;
    let secs = match wav_secs(&wav) {
        Some(s) if wav.len() as u64 == header.size && s <= MAX_SECS + 1 => s,
        _ => {
            send.write_all(b"invalid recording").await?;
            send.finish()?;
            bail!("invalid voice message");
        }
    };
    let id: String = header.id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(64).collect();
    let target = dir(paths).join(format!("{from}-{id}.wav"));
    crate::common::config::write_atomic(&target, &wav)?;
    send.write_all(b"ok").await?;
    send.finish()?;
    Ok((id, secs, target, header.ms))
}

/// Keeps a copy of a voice message you sent (until the next restart, like received ones).
pub fn keep_sent(paths: &Paths, to: &str, id: &str, wav: &[u8]) -> Result<PathBuf> {
    let target = dir(paths).join(format!("sent-{to}-{id}.wav"));
    crate::common::config::write_atomic(&target, wav)?;
    Ok(target)
}

/// True if this chat file is a voice message that still exists.
pub fn available(path: &str) -> bool {
    Path::new(path).exists()
}

// ── Microphone and speakers (audio.rs) ──────────────────────────────────────

#[cfg(feature = "voice")]
mod audio;
#[cfg(feature = "voice")]
pub use audio::{Player, Recorder};

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(secs: u32) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut out = std::io::Cursor::new(Vec::new());
        let mut w = hound::WavWriter::new(&mut out, spec).unwrap();
        for i in 0..secs * SAMPLE_RATE {
            w.write_sample(((i % 100) as i16 - 50) * 100).unwrap();
        }
        w.finalize().unwrap();
        out.into_inner()
    }

    #[test]
    fn durations_and_limits() {
        assert_eq!(format_secs(0), "0:00");
        assert_eq!(format_secs(42), "0:42");
        assert_eq!(format_secs(300), "5:00");
        assert_eq!(wav_secs(&wav(3)), Some(3));
        assert_eq!(wav_secs(b"not a wav"), None);
        // A full 5-minute recording fits under the size limit.
        assert!((wav(MAX_SECS).len() as u64) < VOICE_MAX);
    }
}
