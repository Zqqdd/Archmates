//! Requests from the `archmates` command and the widgets, through the local socket (`ipc`):
//! send a message, a file or a voice message, answer a file offer, ping, quit.

use std::sync::Arc;
use std::time::Duration;

use iroh::Endpoint;

use super::state::Shared;
use crate::common::time::now_ms;
use crate::common::{chat, update};

/// Requests from the `archmates` command and the widgets (see ipc.rs).
pub(super) async fn handle_command(
    endpoint: Endpoint,
    shared: Arc<Shared>,
    command: crate::common::ipc::Command,
) -> crate::common::ipc::Answer {
    use crate::common::ipc::{Answer, Command};
    let fail = |message: String| Answer { ok: false, message };
    match command {
        Command::Ping => Answer { ok: true, message: update::CURRENT.to_string() },
        Command::Quit => {
            eprintln!("archmates: asked to stop");
            // Answer first, then exit.
            tokio::spawn(async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                std::process::exit(0);
            });
            Answer { ok: true, message: "stopping".into() }
        }
        Command::SendMessage { to, text } => {
            let Some((id, code, nickname)) = shared.resolve(&to) else {
                return fail(format!("no friend named \"{to}\""));
            };
            let text = text.trim().to_string();
            if text.is_empty() {
                return fail("empty message".into());
            }
            if text.chars().count() > chat::MESSAGE_MAX {
                return fail(format!("messages are limited to {} characters", chat::MESSAGE_MAX));
            }
            let message = chat::Message {
                id: chat::new_id(),
                dir: "out".into(),
                text,
                ms: now_ms(),
                delivered: false,
                file: None,
                voice: None,
                received_ms: None,
            };
            if let Err(err) = chat::append(&shared.paths, &code, &message) {
                return fail(format!("{err:#}"));
            }
            let _ = super::presence::greet(&endpoint, &shared, id).await;
            let _ = shared.publish();
            if chat::pending(&shared.paths, &code).is_empty() {
                Answer { ok: true, message: format!("sent to {nickname}") }
            } else {
                Answer {
                    ok: true,
                    message: format!("{nickname} is offline: the message will be sent when they come back"),
                }
            }
        }
        Command::AnswerFile { id, accept } => {
            if shared.answer_offer(&id, accept) {
                Answer { ok: true, message: if accept { "file accepted".into() } else { "file declined".into() } }
            } else {
                fail("this file offer has expired".into())
            }
        }
        Command::SendVoice { to, path } => match std::fs::read(&path) {
            Ok(wav) => send_voice(&endpoint, &shared, &to, &wav).await,
            Err(e) => fail(format!("cannot read the recording: {e}")),
        },
        Command::VoiceRecord { to } => shared.voice.record(&shared, to).await,
        Command::VoiceStop => shared.voice.stop(&shared.paths),
        Command::VoiceSendRecording => shared.voice.send(&endpoint, &shared).await,
        Command::VoiceDiscard => shared.voice.discard(&shared.paths),
        Command::VoicePlay { id, path } => shared.voice.play(&shared.paths, id, path).await,
        Command::VoicePlayStop => shared.voice.stop_playing(&shared.paths),
        Command::SendFile { to, path } => {
            let Some((id, code, nickname)) = shared.resolve(&to) else {
                return fail(format!("no friend named \"{to}\""));
            };
            match crate::common::transfer::send(&endpoint, id, std::path::Path::new(&path)).await {
                Ok(name) => {
                    let message = chat::Message {
                        id: chat::new_id(),
                        dir: "out".into(),
                        text: format!("📎 {name}"),
                        ms: now_ms(),
                        delivered: true,
                        file: Some(path.clone()),
                        voice: None,
                        received_ms: None,
                    };
                    let _ = chat::append(&shared.paths, &code, &message);
                    let _ = shared.publish();
                    Answer { ok: true, message: format!("{name} sent to {nickname}") }
                }
                Err(err) => fail(format!("{err:#}")),
            }
        }
    }
}

/// Sends a recorded voice message (WAV bytes) and adds it to the conversation.
pub(super) async fn send_voice(
    endpoint: &Endpoint,
    shared: &Shared,
    to: &str,
    wav: &[u8],
) -> crate::common::ipc::Answer {
    use crate::common::ipc::Answer;
    use crate::common::voice;
    let Some((id, code, nickname)) = shared.resolve(to) else {
        return Answer { ok: false, message: format!("no friend named \"{to}\"") };
    };
    let msg_id = chat::new_id();
    let ms = now_ms();
    match voice::send(endpoint, id, &msg_id, wav, ms).await {
        Ok(secs) => {
            let kept = voice::keep_sent(&shared.paths, &code, &msg_id, wav).ok();
            let message = chat::Message {
                id: msg_id,
                dir: "out".into(),
                text: format!("🎤 Voice message ({})", voice::format_secs(secs)),
                ms,
                delivered: true,
                file: kept.map(|p| p.to_string_lossy().into_owned()),
                voice: Some(secs),
                received_ms: None,
            };
            let _ = chat::append(&shared.paths, &code, &message);
            let _ = shared.publish();
            Answer { ok: true, message: format!("voice message sent to {nickname}") }
        }
        Err(err) => Answer { ok: false, message: format!("{err:#}") },
    }
}
