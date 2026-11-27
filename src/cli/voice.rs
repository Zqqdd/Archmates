//! Voice messages from the command line: record and send, or play.

use anyhow::Result;

use archmates::config::{self, Paths};
use archmates::ipc;

#[cfg(feature = "voice")]
pub async fn send(to: String) -> Result<()> {
    let recorder = archmates::voice::Recorder::start()?;
    println!("● Recording… press Enter to stop and send (5 min max).");
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    })
    .await?;
    let wav = recorder.finish()?;
    let tmp = Paths::detect().cache.join(format!("recording-{}.wav", std::process::id()));
    config::write_atomic(&tmp, &wav)?;
    let result = ipc::request(&ipc::Command::SendVoice { to, path: tmp.to_string_lossy().into_owned() }).await;
    let _ = std::fs::remove_file(&tmp);
    println!("✓ {}", result?.message);
    Ok(())
}

#[cfg(feature = "voice")]
pub fn play(file: &std::path::Path) -> Result<()> {
    let player = archmates::voice::Player::play(file)?;
    println!("▶ Playing ({})…", archmates::voice::format_secs(player.secs as u32));
    player.wait();
    Ok(())
}

#[cfg(not(feature = "voice"))]
pub async fn send(_to: String) -> Result<()> {
    anyhow::bail!("this archmates was built without voice support")
}

#[cfg(not(feature = "voice"))]
pub fn play(_file: &std::path::Path) -> Result<()> {
    anyhow::bail!("this archmates was built without voice support")
}
