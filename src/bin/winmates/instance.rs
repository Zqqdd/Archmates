//! One window only: starting Winmates again brings the open window to the front.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Set once the window exists, to wake it up when asked to show itself.
pub static CTX: std::sync::OnceLock<eframe::egui::Context> = std::sync::OnceLock::new();

fn wake(show: &AtomicBool) {
    show.store(true, Ordering::SeqCst);
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

/// True if this is the only window; otherwise asks the open one to come to the front.
#[cfg(windows)]
pub async fn claim(show: Arc<AtomicBool>) -> bool {
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
    let user: String = archmates::platform::user_name().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let name = format!(r"\\.\pipe\winmates-ui-{user}");
    let Ok(mut server) = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name)
    else {
        if let Ok(mut client) = ClientOptions::new().open(&name) {
            let _ = client.write_all(b"show").await;
        }
        return false;
    };
    tokio::spawn(async move {
        loop {
            if server.connect().await.is_err() {
                break;
            }
            let mut buf = [0u8; 8];
            let _ = server.read(&mut buf).await;
            wake(&show);
            let _ = server.disconnect();
        }
    });
    true
}

#[cfg(not(windows))]
pub async fn claim(show: Arc<AtomicBool>) -> bool {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| archmates::config::Paths::detect().cache);
    let path = dir.join("winmates-ui.sock");
    if let Ok(mut s) = tokio::net::UnixStream::connect(&path).await {
        let _ = s.write_all(b"show").await;
        return false;
    }
    let _ = std::fs::remove_file(&path);
    let Ok(listener) = tokio::net::UnixListener::bind(&path) else { return true };
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let mut buf = [0u8; 8];
            let _ = s.read(&mut buf).await;
            wake(&show);
        }
    });
    true
}
