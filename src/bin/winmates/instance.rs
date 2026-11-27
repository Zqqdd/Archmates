//! One copy only: starting Winmates again brings the open widget to the front, and
//! `winmates --open <friend>` opens that conversation in the running copy.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Set once the window exists, to wake it up when asked to show itself.
pub static CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

/// A conversation asked for with `--open <friend>` (nickname or code), for the app to open.
pub static OPEN: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// What a second copy sends: "show", or "open <friend>".
pub fn request(open: Option<&str>) -> String {
    match open {
        Some(who) => format!("open {who}"),
        None => "show".into(),
    }
}

fn wake(show: &AtomicBool, message: &[u8]) {
    if let Some(who) = std::str::from_utf8(message).ok().and_then(|m| m.trim_end_matches('\0').strip_prefix("open ")) {
        *OPEN.lock().unwrap() = Some(who.trim().to_string());
    }
    show.store(true, Ordering::SeqCst);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// True if this is the only window; otherwise asks the open one to come to the front.
#[cfg(windows)]
pub async fn claim(show: Arc<AtomicBool>, message: &str) -> bool {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
    let user: String = archmates::platform::user_name().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let name = format!(r"\\.\pipe\winmates-ui-{user}");
    let Ok(mut server) = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)
    else {
        if let Ok(mut client) = ClientOptions::new().open(&name) {
            let _ = client.write_all(message.as_bytes()).await;
        }
        return false;
    };
    tokio::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                break;
            }
            let mut buf = [0u8; 160];
            let n = server.read(&mut buf).await.unwrap_or(0);
            wake(&show, &buf[..n]);
            let _ = server.disconnect();
        }
    });
    true
}

#[cfg(not(windows))]
pub async fn claim(show: Arc<AtomicBool>, message: &str) -> bool {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| archmates::config::Paths::detect().cache);
    let path = dir.join("winmates-ui.sock");
    if let Ok(mut s) = tokio::net::UnixStream::connect(&path).await {
        let _ = s.write_all(message.as_bytes()).await;
        return false;
    }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = tokio::net::UnixListener::bind(&path) else { return true };
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 160];
            let n = s.read(&mut buf).await.unwrap_or(0);
            wake(&show, &buf[..n]);
        }
    });
    true
}
