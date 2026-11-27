//! Winmates: the Windows client of Archmates, made to look like the Linux rice: a small
//! frosted-glass card with your friends (like the universal widget), and a friend panel in
//! the style of the Noctalia panel (picture, status, RAM / storage, chat, files, voice).
//! It speaks the same protocol as the Linux version: Windows and Linux friends chat as usual.
//!
//! It also runs on Linux as a desktop app, named "Archmates" there
//! (`cargo build --release --features app`), next to the Noctalia and universal widgets.
//!
//! - Installed by WinmatesSetup.exe; WinmatesUpdater starts it at every login after updating.
//! - Each time it starts it also checks for an update itself, and makes sure the service runs.
//! - Started from the zip (older way), it installs itself on first run.
//! - Only one window: starting it again brings the open one to the front.
//! - Closing it keeps the service running (friends still see you, notifications still come).
//! - Pictures in the chat are blurred whenever the window is not the active one.
//! - Voice messages: up to 5 minutes, deleted at every restart.
//! - ⚙ in a conversation: delete it for good, with every file that friend sent you.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
#[cfg(windows)]
mod install;
mod instance;
mod pictures;
mod startup;
#[path = "../../gui/theme.rs"]
mod theme;
mod views;
mod widgets;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use archmates::service::control::{self as launcher, UpdateResult};
use eframe::egui;

/// The app's name: Winmates on Windows, Archmates elsewhere.
pub const APP_NAME: &str = if cfg!(windows) { "Winmates" } else { "Archmates" };

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    #[cfg(windows)]
    {
        platform::cleanup_old_programs();
        if let Some(installed) = install::self_install() {
            relaunch(&installed);
        }
    }

    // v0.4.0 started "winmates.exe --background" at login: now WinmatesUpdater's job.
    if has("--background") {
        runtime.block_on(async {
            #[cfg(windows)]
            install::repair().await;
            if let UpdateResult::Installed(_) = launcher::update(true).await {
                return;
            }
            launcher::ensure_service().await;
        });
        return;
    }

    // One window only: if one is open, bring it to the front instead.
    let show = Arc::new(AtomicBool::new(false));
    if !runtime.block_on(instance::claim(show.clone())) {
        return;
    }

    let background = Arc::new(Mutex::new(startup::Background::default()));
    runtime.spawn(startup::startup(background.clone(), !has("--from-updater")));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_app_id("winmates")
            .with_inner_size([380.0, 600.0])
            .with_min_inner_size([320.0, 420.0])
            .with_decorations(false)
            .with_transparent(true),
        persist_window: true,
        ..Default::default()
    };
    let handle = runtime.handle().clone();
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, handle, background, show)))),
    );
    if let Err(err) = result {
        eprintln!("{APP_NAME}: {err}");
    }
    drop(runtime);
}

/// Starts this program again with the same arguments, then exits.
#[cfg_attr(not(windows), allow(dead_code))]
fn relaunch(program: &Path) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = std::process::Command::new(program).args(args).spawn();
    std::process::exit(0);
}
