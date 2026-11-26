//! Local control socket: how the `archmates` command and the widgets ask the running
//! service to send a message or a file (the service is the one holding your P2P identity).
//!
//! One JSON request per line, one JSON answer per line, on $XDG_RUNTIME_DIR/archmates.sock
//! (readable by you only).

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    SendMessage { to: String, text: String },
    SendFile { to: String, path: String },
    AnswerFile { id: String, accept: bool },
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    #[serde(default)]
    pub message: String,
}

pub fn socket_path() -> PathBuf {
    if std::env::var_os("ARCHMATES_HOME").is_some() {
        // Test instances keep their socket next to their own data.
        return crate::config::Paths::detect().cache.join("archmates.sock");
    }
    std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| crate::config::Paths::detect().cache)
        .join("archmates.sock")
}

/// Sends one command to the service and returns its answer.
pub async fn request(command: &Command) -> Result<Answer> {
    let stream = UnixStream::connect(socket_path())
        .await
        .context("the archmates service is not running (start it: systemctl --user start archmates)")?;
    let (read, mut write) = stream.into_split();
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
    Fut: std::future::Future<Output = Answer> + Send,
{
    let path = socket_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).with_context(|| format!("cannot open {}", path.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    loop {
        let (stream, _) = listener.accept().await?;
        let handle = handle.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
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
        });
    }
}
