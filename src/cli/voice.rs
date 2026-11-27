//! Voice messages from the command line, through the service's voice desk (the same recorder
//! as Winmates, the Linux widget and the Noctalia panel): record, review, send, play.

use std::time::Duration;

use anyhow::{Result, bail};

use archmates::config::Paths;
use archmates::ipc::{self, Command};
use archmates::voice::{self, DeskState, DeskStatus};

async fn ask(command: Command) -> Result<String> {
    let answer = ipc::request(&command).await?;
    Ok(answer.message)
}

/// Records, then sends when you press Enter.
pub async fn send(to: String) -> Result<()> {
    ask(Command::VoiceRecord { to }).await?;
    println!("● Recording… press Enter to stop and send (5 min max).");
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
    })
    .await?;
    println!("✓ {}", ask(Command::VoiceSendRecording).await?);
    Ok(())
}

pub async fn record(to: String) -> Result<()> {
    println!("● {}", ask(Command::VoiceRecord { to }).await?);
    println!("  Stop with: archmates voice stop   (then send-recording or discard)");
    Ok(())
}

pub async fn stop() -> Result<()> {
    println!("■ {}", ask(Command::VoiceStop).await?);
    Ok(())
}

pub async fn send_recording() -> Result<()> {
    println!("✓ {}", ask(Command::VoiceSendRecording).await?);
    Ok(())
}

pub async fn discard() -> Result<()> {
    println!("✓ {}", ask(Command::VoiceDiscard).await?);
    Ok(())
}

/// Plays a voice message (or "draft", the recording under review) and waits for its end,
/// unless `no_wait`.
pub async fn play(file: Option<std::path::PathBuf>, id: Option<String>, no_wait: bool) -> Result<()> {
    let paths = Paths::detect();
    let (id, path) = match file {
        Some(f) => (id.unwrap_or_else(|| "cli".into()), f.to_string_lossy().into_owned()),
        None => match DeskStatus::read(&paths).draft {
            Some(d) => (voice::desk::DRAFT_ID.to_string(), d),
            None => bail!("give the voice file to play (shown by: archmates chat <friend>)"),
        },
    };
    println!("▶ {}", ask(Command::VoicePlay { id: id.clone(), path }).await?);
    if no_wait {
        return Ok(());
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    while DeskStatus::read(&paths).progress_of(&id).is_some() {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

pub async fn stop_playing() -> Result<()> {
    ask(Command::VoicePlayStop).await?;
    Ok(())
}

pub fn status(json: bool) -> Result<()> {
    let status = DeskStatus::read(&Paths::detect());
    if json {
        println!("{}", serde_json::to_string(&status)?);
        return Ok(());
    }
    let secs = voice::format_secs(status.secs as u32);
    match status.state {
        DeskState::Idle => println!("Nothing recorded."),
        DeskState::Recording => println!("● Recording {secs} / 5:00"),
        DeskState::Review => println!("■ Recorded {secs}: archmates voice send-recording (or discard)"),
        DeskState::Sending => println!("… Sending {secs}"),
    }
    if let Some(e) = status.error {
        println!("  {e}");
    }
    Ok(())
}
