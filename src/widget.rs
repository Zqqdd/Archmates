//! Starts and stops the universal desktop widget (`archmates-widget`).
//!
//! Mode, stored in ~/.local/share/archmates/widget:
//!   auto (default)  show the widget when Noctalia is not running (Noctalia has its own)
//!   on              always show it
//!   off             never show it
//!
//! The service checks every 15 s and (re)starts the widget as soon as a Wayland desktop is up,
//! so it works whatever compositor or login method you use.

#![cfg_attr(windows, allow(dead_code))]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use crate::config::Paths;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Auto,
    On,
    Off,
}

impl Mode {
    pub fn load(paths: &Paths) -> Self {
        match std::fs::read_to_string(paths.widget_mode()).unwrap_or_default().trim() {
            "on" => Mode::On,
            "off" => Mode::Off,
            _ => Mode::Auto,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::On => "on",
            Mode::Off => "off",
        }
    }
}

/// The widget program: next to the archmates binary, in /usr/share/archmates, or on $PATH.
pub fn program() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("archmates-widget"));
        }
    }
    candidates.push(PathBuf::from("/usr/share/archmates/archmates-widget"));
    if let Ok(path) = std::env::var("PATH") {
        candidates.extend(path.split(':').map(|d| PathBuf::from(d).join("archmates-widget")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn process_running(name: &str) -> bool {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
        .any(|c| c.trim() == name)
}

/// Name of the Wayland socket to draw on, if a Wayland desktop is running.
fn wayland_display() -> Option<String> {
    if let Ok(d) = std::env::var("WAYLAND_DISPLAY") {
        if !d.is_empty() {
            return Some(d);
        }
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let mut sockets: Vec<String> = std::fs::read_dir(runtime)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("wayland-") && !n.ends_with(".lock"))
        .collect();
    sockets.sort();
    sockets.into_iter().next()
}

pub fn should_show(mode: Mode) -> bool {
    match mode {
        Mode::Off => false,
        Mode::On => true,
        Mode::Auto => !process_running("noctalia"),
    }
}

/// Keeps the widget running while it should be shown.
pub async fn supervise(paths: Paths) {
    let mut child: Option<Child> = None;
    let mut started_at = Instant::now();
    let mut retry_after = Instant::now();

    loop {
        // Reap a widget that exited; back off if it crashed right away (e.g. missing dependency).
        if let Some(c) = child.as_mut() {
            if let Ok(Some(status)) = c.try_wait() {
                child = None;
                if started_at.elapsed() < Duration::from_secs(10) {
                    eprintln!("widget: exited right away ({status}); retrying in 15 s");
                    retry_after = Instant::now() + Duration::from_secs(15);
                }
            }
        }

        let wanted = should_show(Mode::load(&paths));
        match (&mut child, wanted) {
            (None, true) if Instant::now() >= retry_after => {
                if let (Some(prog), Some(display)) = (program(), wayland_display()) {
                    match Command::new(&prog).env("WAYLAND_DISPLAY", &display).spawn() {
                        Ok(c) => {
                            eprintln!("widget: started on {display}");
                            child = Some(c);
                            started_at = Instant::now();
                        }
                        Err(err) => {
                            eprintln!("widget: could not start {}: {err}", prog.display());
                            retry_after = Instant::now() + Duration::from_secs(15);
                        }
                    }
                }
            }
            (Some(c), false) => {
                let _ = c.kill();
                let _ = c.wait();
                child = None;
                eprintln!("widget: stopped");
            }
            _ => {}
        }
        tokio::time::sleep(Duration::from_secs(15)).await;
    }
}
