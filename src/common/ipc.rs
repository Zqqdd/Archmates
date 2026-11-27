//! Local control socket: how the `archmates` command and the widgets ask the running
//! service to send a message or a file (the service is the one holding your P2P identity).
//!
//! One JSON request per line, one JSON answer per line, on $XDG_RUNTIME_DIR/archmates.sock
//! (readable by you only). On Windows: the named pipe \\.\pipe\winmates-<user>, local only.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    SendMessage {
        to: String,
        text: String,
    },
    SendFile {
        to: String,
        path: String,
    },
    AnswerFile {
        id: String,
        accept: bool,
    },
    /// Sends a recorded voice message (a WAV file, up to 5 minutes) to a friend who is online.
    SendVoice {
        to: String,
        path: String,
    },
    /// Voice desk (see voice/desk.rs): the service records from the microphone for this friend.
    VoiceRecord {
        to: String,
    },
    /// Stops the recording and keeps it for review (listen, then send or discard).
    VoiceStop,
    /// Sends the recording to its friend (stopping it first if it is still going).
    VoiceSendRecording,
    /// Cancels the recording, or discards the one under review.
    VoiceDiscard,
    /// Plays a voice file: a message (`id` = its id) or the recording under review ("draft").
    VoicePlay {
        id: String,
        path: String,
    },
    VoicePlayStop,
    /// "Are you running?" Answer: ok, with the service version as message.
    Ping,
    /// Stops the service (used by Winmates to restart it after an update, or to quit).
    Quit,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    #[serde(default)]
    pub message: String,
}

#[cfg(windows)]
pub fn pipe_name() -> String {
    // One pipe per user, and per test instance (ARCHMATES_HOME).
    let tag =
        std::env::var("ARCHMATES_HOME").map(|h| crate::common::profile::fingerprint(h.as_bytes())).unwrap_or_default();
    let user: String = crate::platform::user_name().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!(r"\\.\pipe\winmates-{user}{tag}")
}

pub fn socket_path() -> PathBuf {
    if std::env::var_os("ARCHMATES_HOME").is_some() {
        // Test instances keep their socket next to their own data.
        return crate::common::config::Paths::detect().cache.join("archmates.sock");
    }
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::common::config::Paths::detect().cache)
        .join("archmates.sock")
}

/// Sends one command to the service and returns its answer.
#[cfg(unix)]
async fn connect() -> Result<tokio::net::UnixStream> {
    tokio::net::UnixStream::connect(socket_path())
        .await
        .context("the archmates service is not running (start it: systemctl --user start archmates)")
}

#[cfg(windows)]
async fn connect() -> Result<tokio::net::windows::named_pipe::NamedPipeClient> {
    use tokio::net::windows::named_pipe::ClientOptions;
    const ERROR_PIPE_BUSY: i32 = 231;
    for _ in 0..50 {
        match ClientOptions::new().open(pipe_name()) {
            Ok(client) => return Ok(client),
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e).context("the archmates service is not running (open Winmates)"),
        }
    }
    Err(anyhow!("the archmates service is busy, try again"))
}

pub async fn request(command: &Command) -> Result<Answer> {
    let stream = connect().await?;
    let (read, mut write) = tokio::io::split(stream);
    write.write_all(serde_json::to_string(command)?.as_bytes()).await?;
    write.write_all(b"\n").await?;
    write.shutdown().await?;
    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await?;
    let answer: Answer = serde_json::from_str(&line).context("bad answer from the service")?;
    if answer.ok { Ok(answer) } else { Err(anyhow!(answer.message)) }
}

/// Runs the socket server; `handle` turns each command into an answer.
pub async fn serve<F, Fut>(handle: F) -> Result<()>
where
    F: Fn(Command) -> Fut + Clone + Send + 'static,
    Fut: std::future::Future<Output = Answer> + Send + 'static,
{
    #[cfg(unix)]
    {
        let path = socket_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&path);
        let listener =
            tokio::net::UnixListener::bind(&path).with_context(|| format!("cannot open {}", path.display()))?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        loop {
            let (stream, _) = listener.accept().await?;
            tokio::spawn(answer_one(stream, handle.clone()));
        }
    }
    #[cfg(windows)]
    {
        use tokio::net::windows::named_pipe::ServerOptions;
        let name = pipe_name();
        // first_pipe_instance: fails if another service already runs for this user.
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .with_context(|| format!("cannot open {name} (is the service already running?)"))?;
        loop {
            server.connect().await?;
            let client = server;
            server = ServerOptions::new().reject_remote_clients(true).create(&name)?;
            tokio::spawn(answer_one(client, handle.clone()));
        }
    }
}

/// Reads one command from a client and writes the answer.
async fn answer_one<S, F, Fut>(stream: S, handle: F)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
    F: Fn(Command) -> Fut,
    Fut: std::future::Future<Output = Answer>,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut line = String::new();
    if BufReader::new(read).read_line(&mut line).await.is_err() {
        return;
    }
    let answer = match serde_json::from_str::<Command>(&line) {
        Ok(cmd) => handle(cmd).await,
        Err(err) => Answer { ok: false, message: format!("bad request: {err}") },
    };
    let mut out = serde_json::to_string(&answer).unwrap_or_default();
    out.push('\n');
    let _ = write.write_all(out.as_bytes()).await;
}
