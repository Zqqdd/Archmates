//! Conversations, stored locally: one JSON line per message in
//! ~/.local/share/archmates/chat/<friend code>.jsonl
//!
//! Outgoing messages are written first with `delivered = false`; the service sends them as
//! soon as the friend is reachable (right away, or when they come back online).

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Paths;

/// Longest message accepted (characters).
pub const MESSAGE_MAX: usize = 4000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique id (sender's clock + random part), used to ignore duplicates.
    pub id: String,
    /// "in" (received) or "out" (sent by you).
    pub dir: String,
    pub text: String,
    pub ms: u64,
    #[serde(default)]
    pub delivered: bool,
    /// For file transfers: the file name (text holds a short description).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

fn path(paths: &Paths, code: &str) -> PathBuf {
    paths.data.join("chat").join(format!("{code}.jsonl"))
}

fn read_marker(paths: &Paths, code: &str) -> PathBuf {
    paths.data.join("chat").join(format!("{code}.read"))
}

pub fn load(paths: &Paths, code: &str) -> Vec<Message> {
    fs::read_to_string(path(paths, code))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn save_all(paths: &Paths, code: &str, messages: &[Message]) -> Result<()> {
    let mut out = String::new();
    for m in messages {
        out.push_str(&serde_json::to_string(m)?);
        out.push('\n');
    }
    crate::config::write_atomic(&path(paths, code), out.as_bytes())
}

pub fn append(paths: &Paths, code: &str, message: &Message) -> Result<()> {
    let p = path(paths, code);
    fs::create_dir_all(p.parent().unwrap())?;
    let mut f = fs::OpenOptions::new().create(true).append(true).open(p)?;
    writeln!(f, "{}", serde_json::to_string(message)?)?;
    Ok(())
}

/// Records a received message, ignoring one already stored (same id).
pub fn receive(paths: &Paths, code: &str, message: Message) -> Result<bool> {
    if load(paths, code).iter().any(|m| m.id == message.id) {
        return Ok(false);
    }
    append(paths, code, &Message { dir: "in".into(), delivered: true, ..message })?;
    Ok(true)
}

pub fn pending(paths: &Paths, code: &str) -> Vec<Message> {
    load(paths, code).into_iter().filter(|m| m.dir == "out" && !m.delivered).collect()
}

pub fn mark_delivered(paths: &Paths, code: &str, id: &str) -> Result<()> {
    let mut all = load(paths, code);
    let mut changed = false;
    for m in all.iter_mut().filter(|m| m.id == id) {
        m.delivered = true;
        changed = true;
    }
    if changed {
        save_all(paths, code, &all)?;
    }
    Ok(())
}

pub fn mark_read(paths: &Paths, code: &str) -> Result<()> {
    let p = read_marker(paths, code);
    fs::create_dir_all(p.parent().unwrap())?;
    fs::write(p, crate::daemon::now_ms().to_string())?;
    Ok(())
}

pub fn unread(paths: &Paths, code: &str) -> usize {
    let since: u64 = fs::read_to_string(read_marker(paths, code))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(0);
    load(paths, code).iter().filter(|m| m.dir == "in" && m.ms > since).count()
}

pub fn new_id() -> String {
    let mut random = [0u8; 6];
    let _ = getrandom::fill(&mut random);
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", crate::daemon::now_ms())
}
