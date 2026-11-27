//! Background work started with the window: update check, keeping the service running.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use archmates::service::control::{self as launcher, UpdateResult};

use crate::APP_NAME;

#[derive(Default)]
pub struct Background {
    /// Line shown about updates.
    pub update: String,
    /// Newer version available but not installed by itself (Linux).
    pub update_offer: Option<String>,
    pub service_ok: bool,
    pub checking: bool,
}

pub async fn startup(background: Arc<Mutex<Background>>, check_update: bool) {
    #[cfg(windows)]
    crate::install::repair().await;
    if check_update {
        check_for_update(&background).await;
    }
    loop {
        let ok = launcher::ensure_service().await;
        background.lock().unwrap().service_ok = ok;
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

pub async fn check_for_update(background: &Arc<Mutex<Background>>) {
    background.lock().unwrap().checking = true;
    let result = launcher::update(false).await;
    let mut b = background.lock().unwrap();
    b.checking = false;
    match result {
        UpdateResult::Installed(_v) => {
            #[cfg(windows)]
            crate::relaunch(&archmates::platform::windows::install::app());
        }
        UpdateResult::Available(v) => {
            b.update = format!("{APP_NAME} {v} is available");
            b.update_offer = Some(v);
        }
        UpdateResult::Failed(e) => b.update = format!("Update check failed: {e}"),
        UpdateResult::UpToDate => b.update = "Up to date".into(),
    }
}
