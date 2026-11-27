//! Friends: your code, adding / removing friends, the list, invisible mode.

use anyhow::Result;

use archmates::config::{FriendList, Paths, load_or_create_key};
use archmates::service::{StateFile, Status};
use archmates::time::ago;

pub fn code(paths: &Paths) -> Result<()> {
    let key = load_or_create_key(paths)?;
    println!("{}", key.public());
    eprintln!("\nShare this code with your friends. They add you with:\n  archmates add <code> <nickname>");
    Ok(())
}

pub fn add(paths: &Paths, code: String, nickname: String) -> Result<()> {
    let mut list = FriendList::load(paths)?;
    list.add(&code, &nickname)?;
    list.save(paths)?;
    println!("✓ {nickname} added.");
    println!("  They will show up online once they add you back (run `archmates code` and send them your code).");
    Ok(())
}

pub fn remove(paths: &Paths, who: String) -> Result<()> {
    let mut list = FriendList::load(paths)?;
    if list.remove(&who) {
        list.save(paths)?;
        println!("✓ {who} removed.");
    } else {
        println!("No friend named \"{who}\".");
    }
    Ok(())
}

pub fn list(paths: &Paths) -> Result<()> {
    let list = FriendList::load(paths)?;
    if list.friends.is_empty() {
        println!("No friends yet. Add one with: archmates add <code> <nickname>");
        return Ok(());
    }
    let state: Option<StateFile> =
        std::fs::read_to_string(paths.state()).ok().and_then(|t| serde_json::from_str(&t).ok());
    if state.is_none() {
        println!("(the service is not running: status unknown)\n");
    }
    for friend in &list.friends {
        let f = state.as_ref().and_then(|s| s.friends.iter().find(|f| f.code == friend.code));
        let (dot, text) = match f.map(|f| f.status) {
            Some(Status::Online) => ("\x1b[32m●\x1b[0m", "online".to_string()),
            Some(Status::Pending) => ("\x1b[33m●\x1b[0m", "hasn't added you yet".to_string()),
            _ => (
                "\x1b[90m●\x1b[0m",
                f.and_then(|f| f.last_seen_ms)
                    .map(|v| format!("offline, last seen {}", ago(v)))
                    .unwrap_or("offline".into()),
            ),
        };
        let detail = f
            .and_then(|f| f.profile.as_ref())
            .map(|p| format!("  {}@{} · {} · {}", p.user, p.host, p.distro, p.wm))
            .unwrap_or_default();
        println!("{dot} {:<16} {text}{detail}", friend.nickname);
    }
    if state.is_some_and(|s| s.me.invisible) {
        println!("\n(you are in invisible mode)");
    }
    Ok(())
}

pub fn invisible(paths: &Paths, on: bool) -> Result<()> {
    if on {
        std::fs::create_dir_all(&paths.data)?;
        std::fs::write(paths.invisible(), b"")?;
        println!("✓ Invisible mode on: your friends see you as offline.");
    } else {
        let _ = std::fs::remove_file(paths.invisible());
        println!("✓ Invisible mode off.");
    }
    Ok(())
}
