//! Messages and files: send, files waiting for your answer, conversations.

use anyhow::Result;

use archmates::config::{FriendList, Paths};
use archmates::service::StateFile;
use archmates::time::ago;
use archmates::{chat, ipc};

pub async fn send(to: String, text: Vec<String>) -> Result<()> {
    let answer = ipc::request(&ipc::Command::SendMessage { to, text: text.join(" ") }).await?;
    println!("✓ {}", answer.message);
    Ok(())
}

pub async fn send_file(to: String, file: std::path::PathBuf) -> Result<()> {
    let path = std::fs::canonicalize(&file).map_err(|e| anyhow::anyhow!("cannot read {}: {e}", file.display()))?;
    println!("→ Waiting for your friend to accept {}…", path.display());
    let answer = ipc::request(&ipc::Command::SendFile { to, path: path.to_string_lossy().into_owned() }).await?;
    println!("✓ {}", answer.message);
    Ok(())
}

/// Accepts or declines a file a friend wants to send you.
pub async fn answer(id: String, accept: bool) -> Result<()> {
    let answer = ipc::request(&ipc::Command::AnswerFile { id, accept }).await?;
    if accept {
        println!("✓ {} (the transfer starts now)", answer.message);
    } else {
        println!("✓ {}", answer.message);
    }
    Ok(())
}

pub fn offers(paths: &Paths) -> Result<()> {
    let state: Option<StateFile> =
        std::fs::read_to_string(paths.state()).ok().and_then(|t| serde_json::from_str(&t).ok());
    let offers: Vec<_> = state
        .iter()
        .flat_map(|s| s.friends.iter())
        .flat_map(|f| f.offers.iter().map(move |o| (f.nickname.clone(), o.clone())))
        .collect();
    if offers.is_empty() {
        println!("No file waiting for your answer.");
    }
    for (nick, o) in offers {
        println!(
            "{nick} wants to send {} ({:.1} MB)\n  archmates accept {}   |   archmates decline {}",
            o.name,
            o.size as f64 / 1_048_576.0,
            o.id,
            o.id
        );
    }
    Ok(())
}

pub fn chat(paths: &Paths, with: String, count: usize, json: bool) -> Result<()> {
    let list = FriendList::load(paths)?;
    let Some(friend) = list.friends.iter().find(|f| f.nickname == with || f.code == with) else {
        anyhow::bail!("no friend named \"{with}\"");
    };
    let messages = chat::load(paths, &friend.code);
    let start = messages.len().saturating_sub(count);
    chat::mark_read(paths, &friend.code)?;
    if json {
        println!("{}", serde_json::to_string(&messages[start..])?);
    } else if messages.is_empty() {
        println!("No messages with {} yet. Write one: archmates send {} <text>", friend.nickname, friend.nickname);
    } else {
        for m in &messages[start..] {
            let who = if m.dir == "out" { "you" } else { friend.nickname.as_str() };
            let state = if m.dir == "out" && !m.delivered { "  (waiting)" } else { "" };
            println!("\x1b[90m{}\x1b[0m {who}: {}{state}", ago(m.ms), m.text);
        }
    }
    Ok(())
}

pub fn delete_chat(paths: &Paths, with: String, yes: bool) -> Result<()> {
    let list = FriendList::load(paths)?;
    let Some(friend) = list.friends.iter().find(|f| f.nickname == with || f.code == with) else {
        anyhow::bail!("no friend named \"{with}\"");
    };
    if !yes {
        println!(
            "Delete your whole conversation with {} for good, with every file they sent you ({})?",
            friend.nickname,
            archmates::transfer::received_dir(&friend.nickname).display()
        );
        print!("This cannot be undone. Type \"yes\" to confirm: ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if answer.trim() != "yes" {
            println!("Nothing deleted.");
            return Ok(());
        }
    }
    let d = chat::delete_conversation(paths, &friend.code, &friend.nickname)?;
    println!("✓ Conversation with {} deleted ({} messages, {} files).", friend.nickname, d.messages, d.files);
    Ok(())
}
