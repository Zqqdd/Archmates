//! Conversations, stored locally: one JSON line per message in
//! ~/.local/share/archmates/chat/<friend code>.jsonl
//!
//! Outgoing messages are written first with `delivered = false`; the service sends them as
//! soon as the friend is reachable (right away, or when they come back online).

use std::fs;
use std::io::Write;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::common::config::Paths;

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
    /// For voice messages: length in seconds (`file` is the recording, deleted at restart).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<u32>,
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
    crate::common::config::write_atomic(&path(paths, code), out.as_bytes())
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
    fs::write(p, crate::common::time::now_ms().to_string())?;
    Ok(())
}

pub fn unread(paths: &Paths, code: &str) -> usize {
    let since: u64 = fs::read_to_string(read_marker(paths, code)).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0);
    load(paths, code).iter().filter(|m| m.dir == "in" && m.ms > since).count()
}

pub fn new_id() -> String {
    let mut random = [0u8; 6];
    let _ = getrandom::fill(&mut random);
    let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{hex}", crate::common::time::now_ms())
}

/// What `delete_conversation` removed.
#[derive(Debug, Default)]
pub struct Deleted {
    pub messages: usize,
    pub files: usize,
}

/// Deletes a conversation for good: every message, the voice messages, and every file this
/// friend sent you (their folder in your downloads). Files *you* sent stay where they are on
/// your PC: only the received copies are removed. Cannot be undone.
pub fn delete_conversation(paths: &Paths, code: &str, nickname: &str) -> Result<Deleted> {
    let messages = load(paths, code);
    let mut deleted = Deleted { messages: messages.len(), files: 0 };
    let downloads = crate::common::transfer::download_dir().join(crate::common::transfer::DOWNLOAD_FOLDER);
    let voice = crate::common::voice::dir(paths);
    // Received files and voice messages listed in the conversation (even if the friend was
    // renamed since), but only inside the Archmates download folder or the voice folder.
    for m in &messages {
        let Some(file) = m.file.as_deref() else { continue };
        let path = std::path::Path::new(file);
        let ours = (m.dir == "in" && path.starts_with(&downloads)) || path.starts_with(&voice);
        if ours && fs::remove_file(path).is_ok() {
            deleted.files += 1;
        }
    }
    // The friend's whole download folder.
    let folder = crate::common::transfer::received_dir(nickname);
    if folder.starts_with(&downloads) && folder != downloads {
        if let Ok(entries) = fs::read_dir(&folder) {
            deleted.files += entries.flatten().filter(|e| e.path().is_file()).count();
        }
        let _ = fs::remove_dir_all(&folder);
    }
    // Voice messages, sent or received.
    if let Ok(entries) = fs::read_dir(&voice) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if (name.starts_with(&format!("{code}-")) || name.starts_with(&format!("sent-{code}-")))
                && fs::remove_file(e.path()).is_ok()
            {
                deleted.files += 1;
            }
        }
    }
    let _ = fs::remove_file(path(paths, code));
    let _ = fs::remove_file(read_marker(paths, code));
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deleting_a_conversation_keeps_your_own_files() {
        let base = std::env::temp_dir().join(format!("archmates-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        // Only this test reads these variables.
        unsafe {
            std::env::set_var("ARCHMATES_HOME", base.join("home"));
            std::env::set_var("HOME", &base);
            std::env::set_var("USERPROFILE", &base);
            std::env::set_var("XDG_CONFIG_HOME", base.join("config"));
        }
        let paths = Paths::detect();
        let code = "friendcode";
        let received_dir = crate::common::transfer::received_dir("bob");
        fs::create_dir_all(&received_dir).unwrap();
        let received = received_dir.join("photo.png");
        fs::write(&received, b"x").unwrap();
        let mine = base.join("my-own-file.txt");
        fs::write(&mine, b"keep me").unwrap();
        let voice_file = crate::common::voice::dir(&paths).join(format!("{code}-1.wav"));
        fs::create_dir_all(voice_file.parent().unwrap()).unwrap();
        fs::write(&voice_file, b"x").unwrap();
        let msg = |id: &str, dir: &str, file: Option<&Path>| Message {
            id: id.into(),
            dir: dir.into(),
            text: "t".into(),
            ms: 1,
            delivered: true,
            file: file.map(|f| f.to_string_lossy().into_owned()),
            voice: None,
        };
        append(&paths, code, &msg("1", "in", Some(&received))).unwrap();
        append(&paths, code, &msg("2", "out", Some(&mine))).unwrap();
        append(&paths, code, &msg("3", "in", None)).unwrap();

        let d = delete_conversation(&paths, code, "bob").unwrap();
        assert_eq!(d.messages, 3);
        assert!(!received.exists() && !voice_file.exists());
        assert!(mine.exists(), "a file you sent must never be deleted");
        assert!(load(&paths, code).is_empty());
        let _ = fs::remove_dir_all(&base);
    }
}
