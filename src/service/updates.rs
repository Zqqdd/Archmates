//! The daily update check: notifies once per new version, never installs by itself.

use std::sync::Arc;
use std::time::Duration;

use super::state::Shared;
use crate::common::config::write_atomic;
use crate::common::update;

const UPDATE_CHECK_EVERY: Duration = Duration::from_secs(24 * 3600);

/// Checks GitHub one minute after start, then once a day. Notifies once per new version.
pub(super) async fn check_updates(shared: Arc<Shared>) {
    if std::env::var_os("ARCHMATES_NO_UPDATE_CHECK").is_some() {
        return;
    }
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        match update::latest_version().await {
            Ok(latest) if update::is_newer(&latest, update::CURRENT) => {
                *shared.update_available.lock().unwrap() = Some(latest.clone());
                let marker = shared.paths.cache.join("update-notified");
                let already = std::fs::read_to_string(&marker).ok().is_some_and(|v| v.trim() == latest);
                if !already {
                    crate::platform::notify(
                        &format!("Archmates {latest} is available"),
                        &format!("You have {}. Update with: {}", update::CURRENT, update::how_to_update()),
                        None,
                        true,
                        crate::platform::OnClick::Nothing,
                    );
                    let _ = write_atomic(&marker, latest.as_bytes());
                }
                let _ = shared.publish();
            }
            Ok(_) => {
                *shared.update_available.lock().unwrap() = None;
            }
            Err(err) => eprintln!("update check: {err:#}"),
        }
        tokio::time::sleep(UPDATE_CHECK_EVERY).await;
    }
}
