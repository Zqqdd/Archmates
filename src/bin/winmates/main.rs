//! Winmates: the Windows client of Archmates, drawn like the Noctalia version: a frosted-glass
//! desktop widget with your friends, pinned to the desktop, and a separate panel opened from
//! it (picture, status, RAM / storage, chat, files, voice; adding a friend; settings).
//! It speaks the same protocol as the Linux version: Windows and Linux friends chat as usual.
//!
//! It also runs on Linux as a desktop app, named "Archmates" there
//! (`cargo build --release --features app`), next to the Noctalia and universal widgets.
//!
//! - Installed by WinmatesSetup.exe; WinmatesUpdater starts it at every login after updating.
//! - Each time it starts it also checks for an update itself, and makes sure the service runs.
//! - Started from the zip (older way), it installs itself on first run.
//! - Only one copy: starting it again brings the widget above your windows;
//!   `winmates --open <friend>` opens that conversation (nickname or friend code).
//! - Closing the panel keeps the widget; the service keeps running either way.
//! - Pictures in the chat are blurred whenever the window is not the active one.
//! - Voice messages: up to 5 minutes, deleted at every restart.
//! - ⚙ in a conversation: delete it for good, with every file that friend sent you.

#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod desktop;
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
        archmates::platform::cleanup_old_programs();
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
    let open = args.iter().position(|a| a == "--open").and_then(|i| args.get(i + 1)).cloned();
    let show = Arc::new(AtomicBool::new(false));
    if !runtime.block_on(instance::claim(show.clone(), &instance::request(open.as_deref()))) {
        return;
    }
    if let Some(who) = open {
        *instance::OPEN.lock().unwrap() = Some(who);
    }

    let background = Arc::new(Mutex::new(startup::Background::default()));
    runtime.spawn(startup::startup(background.clone(), !has("--from-updater")));

    // The main window is the desktop widget (see desktop.rs); conversations open in the panel.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(desktop::widget_title())
            .with_app_id("winmates")
            .with_inner_size([views::WIDGET_W, 200.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_window_level(desktop::WIDGET_LEVEL),
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
