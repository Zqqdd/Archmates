//! File transfers, over their own protocol so big files never block presence updates.
//!
//! Stream layout:
//!   sender   → `{"name": "...", "size": 1234}\n`        (the offer)
//!   receiver → `accept\n` or `decline\n`               (the person chooses, up to 5 min)
//!   sender   → the raw bytes, only after `accept`
//!   receiver → `ok` once the whole file is written
//! Received files go to ~/Downloads/Archmates/<nickname>/ (your XDG download folder).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointId};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use crate::config::{FriendList, Paths};

pub const FILE_ALPN: &[u8] = b"archmates/file/2";
/// How long the receiver has to accept a file.
pub const ANSWER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
/// Largest file accepted.
pub const FILE_MAX: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
struct Header {
    name: String,
    size: u64,
}

/// Your download folder (XDG_DOWNLOAD_DIR from ~/.config/user-dirs.dirs, else ~/Downloads).
pub fn download_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()));
    let config = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| home.join(".config"));
    let from_file = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|txt| {
        txt.lines().find_map(|l| l.strip_prefix("XDG_DOWNLOAD_DIR=")).map(|v| {
            let v = v.trim().trim_matches('"');
            PathBuf::from(v.replace("$HOME", &home.to_string_lossy()))
        })
    });
    from_file.unwrap_or_else(|| home.join("Downloads"))
}

/// Keeps only a safe file name: no folders, no hidden files, no control characters.
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { "file".into() } else { cleaned.chars().take(200).collect() }
}

/// `name`, or `name (2)`, `name (3)`… if it already exists.
fn free_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

// ── Sending ─────────────────────────────────────────────────────────────────

/// Sends a file once the friend accepts it. Returns the file name.
pub async fn send(endpoint: &Endpoint, to: EndpointId, path: &Path) -> Result<String> {
    let meta = tokio::fs::metadata(path).await.with_context(|| format!("cannot read {}", path.display()))?;
    if !meta.is_file() {
        bail!("{} is not a file", path.display());
    }
    if meta.len() > FILE_MAX {
        bail!("files are limited to 4 GB");
    }
    let name = safe_name(&path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    let connection = tokio::time::timeout(std::time::Duration::from_secs(20), endpoint.connect(to, FILE_ALPN))
        .await
        .map_err(|_| anyhow!("your friend is unreachable (offline?)"))??;
    let (mut send, recv) = connection.open_bi().await?;
    let header = serde_json::to_string(&Header { name: name.clone(), size: meta.len() })?;
    send.write_all(header.as_bytes()).await?;
    send.write_all(b"\n").await?;

    // Wait for the friend to accept (or decline) the file.
    let mut reader = BufReader::new(recv);
    let mut decision = String::new();
    let read = tokio::time::timeout(ANSWER_TIMEOUT + std::time::Duration::from_secs(10), (&mut reader).take(64).read_line(&mut decision))
        .await
        .map_err(|_| anyhow!("your friend did not answer"))?;
    read?;
    match decision.trim() {
        "accept" => {}
        "decline" => bail!("your friend declined the file"),
        _ => bail!("your friend did not answer"),
    }

    let mut file = tokio::fs::File::open(path).await?;
    tokio::io::copy(&mut file, &mut send).await?;
    send.finish()?;
    let mut answer = Vec::new();
    reader.take(64).read_to_end(&mut answer).await?;
    connection.close(0u32.into(), b"ok");
    if answer != b"ok" {
        bail!("your friend refused the file: {}", String::from_utf8_lossy(&answer));
    }
    Ok(name)
}

// ── Receiving ───────────────────────────────────────────────────────────────

/// Asks the person whether to accept a file (friend, name, size); resolves to their answer.
pub type OfferFn = dyn Fn(EndpointId, String, u64) -> tokio::sync::oneshot::Receiver<bool> + Send + Sync;
/// Called once a file is saved (friend, name, saved path).
pub type SavedFn = dyn Fn(EndpointId, String, PathBuf) + Send + Sync;

#[derive(Clone)]
pub struct FileReceiver {
    pub on_offer: Arc<OfferFn>,
    pub on_saved: Arc<SavedFn>,
}

impl std::fmt::Debug for FileReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FileReceiver")
    }
}

impl ProtocolHandler for FileReceiver {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let id = connection.remote_id();
        let paths = Paths::detect();
        let friend = FriendList::load(&paths).ok().and_then(|l| l.by_id().get(&id).cloned());
        let Some(friend) = friend.filter(|_| !paths.is_invisible()) else {
            connection.close(crate::proto::CODE_NOT_FRIEND.into(), b"not a friend");
            return Ok(());
        };
        match receive(&connection, id, &friend.nickname, &self.on_offer).await {
            Ok(Some((name, saved))) => (self.on_saved)(id, name, saved),
            Ok(None) => {}
            Err(err) => eprintln!("[file from {}] {err:#}", friend.nickname),
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), connection.closed()).await;
        Ok(())
    }
}

async fn receive(connection: &Connection, id: EndpointId, nickname: &str, on_offer: &Arc<OfferFn>) -> Result<Option<(String, PathBuf)>> {
    let (mut send, recv) = connection.accept_bi().await?;
    let mut reader = BufReader::new(recv);
    let mut line = Vec::new();
    (&mut reader).take(4096).read_until(b'\n', &mut line).await?;
    let header: Header = serde_json::from_slice(line.trim_ascii_end()).context("bad file header")?;
    if header.size > FILE_MAX {
        send.write_all(b"too large").await?;
        send.finish()?;
        bail!("refused a file over 4 GB");
    }
    let name = safe_name(&header.name);

    // Ask the person; no answer within 5 minutes counts as "decline".
    let answer = on_offer(id, name.clone(), header.size);
    let accepted = matches!(tokio::time::timeout(ANSWER_TIMEOUT, answer).await, Ok(Ok(true)));
    if !accepted {
        send.write_all(b"decline\n").await?;
        send.finish()?;
        return Ok(None);
    }
    send.write_all(b"accept\n").await?;

    let dir = download_dir().join("Archmates").join(safe_name(nickname));
    tokio::fs::create_dir_all(&dir).await?;
    let target = free_path(&dir, &name);
    let partial = target.with_extension("part");
    let mut file = tokio::fs::File::create(&partial).await?;
    let written = tokio::io::copy(&mut (&mut reader).take(header.size), &mut file).await?;
    file.flush().await?;
    if written != header.size {
        let _ = tokio::fs::remove_file(&partial).await;
        bail!("transfer interrupted ({written} of {} bytes)", header.size);
    }
    tokio::fs::rename(&partial, &target).await?;
    send.write_all(b"ok").await?;
    send.finish()?;
    Ok(Some((name, target)))
}
