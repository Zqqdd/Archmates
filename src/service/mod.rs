//! The background service (`archmates daemon`), the same on Linux and Windows:
//!
//! - listens for your friends (presence, files, voice) and answers with your profile;
//! - every 30 s, contacts each of your friends (`presence.rs`);
//! - writes everyone's status to `state.json` for the widgets (`state.rs`);
//! - takes requests from the CLI and the widgets on the local socket (`commands.rs`);
//! - handles incoming files and voice messages (`receivers.rs`) and checks for updates.
//!
//! `control.rs` is for the apps that start and update it (Winmates, WinmatesUpdater).

mod commands;
pub mod control;
mod presence;
mod receivers;
mod state;
mod updates;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use iroh::endpoint::presets;
use iroh::protocol::Router;
use iroh::{Endpoint, EndpointId};

use state::Shared;
pub use state::{FriendState, Me, Offer, StateFile, Status};

use crate::common::config::Paths;
use crate::common::proto::ALPN;
use crate::common::transfer::FILE_ALPN;

const INTERVAL: Duration = Duration::from_secs(30);

pub async fn run() -> Result<()> {
    // Only one service per user: two would fight over the same identity.
    if crate::common::ipc::request(&crate::common::ipc::Command::Ping).await.is_ok() {
        return Err(anyhow!("the archmates service is already running"));
    }
    let paths = Paths::detect();
    let key = crate::common::config::load_or_create_key(&paths)?;
    let me = key.public();

    // Restore the last known state (pictures, "last seen") after a restart.
    let mut initial = BTreeMap::new();
    if let Ok(txt) = std::fs::read_to_string(paths.state()) {
        if let Ok(old) = serde_json::from_str::<StateFile>(&txt) {
            for f in old.friends {
                if let Ok(id) = f.code.parse::<EndpointId>() {
                    initial.insert(id, FriendState { pending: false, ..f });
                }
            }
        }
    }
    // Voice messages only last until the next restart.
    crate::common::voice::clear(&paths);
    let shared = Arc::new(Shared {
        paths,
        me,
        friends: Mutex::new(initial),
        notified: Mutex::new(HashSet::new()),
        offers: Mutex::new(HashMap::new()),
        update_available: Mutex::new(None),
    });
    tokio::spawn(updates::check_updates(shared.clone()));
    #[cfg(not(windows))]
    tokio::spawn(crate::platform::linux::widget::supervise(Paths::detect()));

    let endpoint =
        Endpoint::builder(presets::N0).secret_key(key).bind().await.context("could not start the P2P network")?;
    let router = Router::builder(endpoint.clone())
        .accept(ALPN, presence::Greeter(shared.clone()))
        .accept(FILE_ALPN, receivers::files(shared.clone()))
        .accept(crate::common::voice::VOICE_ALPN, receivers::voice(shared.clone()))
        .spawn();
    {
        let (endpoint, shared) = (endpoint.clone(), shared.clone());
        tokio::spawn(async move {
            let result =
                crate::common::ipc::serve(move |cmd| commands::handle_command(endpoint.clone(), shared.clone(), cmd))
                    .await;
            if let Err(err) = result {
                eprintln!("control socket: {err:#}");
            }
        });
    }
    eprintln!("archmates: service started, your code is {me}");
    let _ = tokio::time::timeout(Duration::from_secs(15), endpoint.online()).await;
    eprintln!("archmates: connected to the network");

    let mut ticker = tokio::time::interval(INTERVAL);
    loop {
        tokio::select! {
            _ = ticker.tick() => {}
            _ = tokio::signal::ctrl_c() => break,
        }
        // After an update the program file is replaced: exit so systemd restarts the new version.
        if crate::platform::binary_replaced() {
            eprintln!("archmates: new version installed, restarting");
            router.shutdown().await.ok();
            std::process::exit(75);
        }
        let list = match shared.reload_list() {
            Ok(l) => l,
            Err(err) => {
                eprintln!("friends.toml: {err:#}");
                continue;
            }
        };
        if !shared.paths.is_invisible() {
            let mut tasks = Vec::new();
            for (id, friend) in list {
                let (ep, sh) = (endpoint.clone(), shared.clone());
                tasks.push(tokio::spawn(async move {
                    match presence::greet(&ep, &sh, id).await {
                        Ok(()) => sh.set_pending(id, false),
                        Err(err) => eprintln!("[{}] {err:#}", friend.nickname),
                    }
                }));
            }
            for t in tasks {
                let _ = t.await;
            }
        }
        if let Err(err) = shared.publish() {
            eprintln!("state.json: {err:#}");
        }
    }

    router.shutdown().await.ok();
    Ok(())
}
