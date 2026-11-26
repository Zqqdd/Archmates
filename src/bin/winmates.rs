//! Winmates: the Windows client of Archmates. Your friends, their profile, chat, files and
//! settings in one window. It speaks the same protocol as the Linux version, so Windows and
//! Linux friends see each other and chat as usual.
//!
//! It also runs on Linux as a desktop app, named "Archmates" there
//! (`cargo build --release --features app`), next to the Noctalia and universal widgets.
//!
//! Each time it starts it:
//!   1. (Windows) installs itself in %LOCALAPPDATA%\Programs\Winmates on first run, with a
//!      Start menu shortcut, and turns on "start with Windows";
//!   2. (Windows) checks GitHub for a new release and, if there is one, installs it (SHA-256
//!      checked) and restarts itself on the new version;
//!   3. makes sure the presence service (`archmates daemon`) is running, on the same version.
//!
//! `winmates --background` (what Windows runs at login) does all that without a window.
//! Closing the window keeps the service running: your friends still see you, and you still get
//! messages and notifications. "Quit" in the settings stops everything.
//!
//! Pictures in the chat are blurred whenever the window is not the active one.


#![cfg_attr(windows, windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archmates::config::{FriendList, Paths};
use archmates::daemon::{FriendState, StateFile, Status};
use archmates::{chat, ipc, platform, stats, update};
use eframe::egui::{self, Color32, RichText, Vec2};

/// The app's name: Winmates on Windows, Archmates elsewhere.
const APP_NAME: &str = if cfg!(windows) { "Winmates" } else { "Archmates" };

fn main() {
    let background = std::env::args().any(|a| a == "--background");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    #[cfg(windows)]
    {
        update::cleanup_old_programs();
        if let Some(installed) = windows_install::self_install() {
            relaunch(&installed);
        }
    }

    let shared = Arc::new(Mutex::new(Background::default()));
    runtime.spawn(startup(shared.clone()));

    if background {
        // No window: wait for the update check and the service, then leave the service running.
        runtime.block_on(async {
            let deadline = Instant::now() + Duration::from_secs(600);
            while !shared.lock().unwrap().startup_done && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
        return;
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(APP_NAME)
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    let handle = runtime.handle().clone();
    let result = eframe::run_native(
        APP_NAME,
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, handle, shared)))),
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

// ── Startup: update, service ────────────────────────────────────────────────

#[derive(Default)]
struct Background {
    /// Line shown at the top of the window about updates.
    update: String,
    /// Newer version available but not installed automatically (Linux).
    update_offer: Option<String>,
    startup_done: bool,
    service_ok: bool,
}

async fn startup(shared: Arc<Mutex<Background>>) {
    check_for_update(&shared).await;
    ensure_service(&shared).await;
    shared.lock().unwrap().startup_done = true;
    // Keep an eye on the service: restart it if it stopped.
    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;
        ensure_service(&shared).await;
    }
}

async fn check_for_update(shared: &Arc<Mutex<Background>>) {
    if std::env::var_os("ARCHMATES_NO_UPDATE_CHECK").is_some() {
        return;
    }
    let latest = match tokio::time::timeout(Duration::from_secs(15), update::latest_version()).await {
        Ok(Ok(v)) => v,
        Ok(Err(err)) => {
            eprintln!("update check: {err:#}");
            return;
        }
        Err(_) => return,
    };
    if !update::is_newer(&latest, update::CURRENT) {
        return;
    }
    #[cfg(windows)]
    {
        shared.lock().unwrap().update = format!("Updating to {latest}…");
        match update::install(&latest).await {
            Ok(()) => {
                // Stop the old service; the new app starts the new one.
                let _ = ipc::request(&ipc::Command::Quit).await;
                tokio::time::sleep(Duration::from_millis(500)).await;
                relaunch(&platform::install_dir().join("winmates.exe"));
            }
            Err(err) => shared.lock().unwrap().update = format!("Update to {latest} failed: {err:#}"),
        }
    }
    #[cfg(not(windows))]
    {
        let mut s = shared.lock().unwrap();
        s.update = format!("{APP_NAME} {latest} is available.");
        s.update_offer = Some(latest);
    }
}

/// The `archmates` command next to this app.
fn cli_program() -> PathBuf {
    let name = if cfg!(windows) { "archmates.exe" } else { "archmates" };
    let beside = std::env::current_exe().ok().map(|e| e.with_file_name(name));
    match beside {
        Some(p) if p.exists() => p,
        _ => PathBuf::from(name),
    }
}

/// Makes sure the service runs, on this app's version.
async fn ensure_service(shared: &Arc<Mutex<Background>>) {
    let running = ipc::request(&ipc::Command::Ping).await;
    if let Ok(answer) = &running {
        if answer.message == update::CURRENT || !cfg!(windows) {
            shared.lock().unwrap().service_ok = true;
            return;
        }
        // Windows: an older service is still running after an update.
        let _ = ipc::request(&ipc::Command::Quit).await;
        tokio::time::sleep(Duration::from_millis(800)).await;
    }
    #[cfg(not(windows))]
    if std::env::var_os("ARCHMATES_HOME").is_none() {
        // Linux: the systemd user service is the normal way (not for test instances).
        let _ = std::process::Command::new("systemctl").args(["--user", "start", "archmates.service"]).status();
        tokio::time::sleep(Duration::from_secs(2)).await;
        if ipc::request(&ipc::Command::Ping).await.is_ok() {
            shared.lock().unwrap().service_ok = true;
            return;
        }
    }
    let log = Paths::detect().cache.join("service.log");
    let _ = std::fs::create_dir_all(log.parent().unwrap());
    let mut cmd = platform::command(cli_program());
    cmd.arg("daemon");
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
        if let Ok(copy) = file.try_clone() {
            cmd.stdout(copy);
        }
        cmd.stderr(file);
    }
    let started = cmd.spawn().is_ok();
    tokio::time::sleep(Duration::from_secs(2)).await;
    shared.lock().unwrap().service_ok = started && ipc::request(&ipc::Command::Ping).await.is_ok();
}

// ── Windows install ─────────────────────────────────────────────────────────

#[cfg(windows)]
mod windows_install {
    use std::path::PathBuf;

    use archmates::{platform, update};

    /// First run from anywhere else (e.g. Downloads): copies both programs into the install
    /// folder, adds a Start menu shortcut and turns on "start with Windows".
    /// Returns the installed app to restart from, or None when already running from there.
    pub fn self_install() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = platform::install_dir();
        let here = exe.parent()?;
        if same_dir(here, &dir) {
            return None;
        }
        std::fs::create_dir_all(&dir).ok()?;
        for name in update::WINDOWS_PROGRAMS {
            let from = here.join(name);
            let bytes = std::fs::read(&from).ok()?;
            update::replace_program(&dir.join(name), &bytes).ok()?;
        }
        let app = dir.join("winmates.exe");
        let first_time = !platform::autostart_enabled();
        if first_time {
            let _ = platform::set_autostart(true);
        }
        start_menu_shortcut(&app);
        Some(app)
    }

    fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    fn start_menu_shortcut(app: &std::path::Path) {
        let Some(appdata) = std::env::var_os("APPDATA") else { return };
        let link = PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Winmates.lnk");
        let quote = |p: &std::path::Path| p.display().to_string().replace('\'', "''");
        let script = format!(
            "$s=(New-Object -ComObject WScript.Shell).CreateShortcut('{}');$s.TargetPath='{}';$s.WorkingDirectory='{}';$s.Save()",
            quote(&link),
            quote(app),
            quote(app.parent().unwrap_or(app)),
        );
        let _ = platform::command("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status();
    }
}

// ── The window ──────────────────────────────────────────────────────────────

#[derive(PartialEq)]
enum Tab {
    Friends,
    Settings,
}

/// A picture shown in the chat, sharp and blurred.
struct Picture {
    sharp: egui::TextureHandle,
    blurred: egui::TextureHandle,
    size: Vec2,
}

struct App {
    paths: Paths,
    rt: tokio::runtime::Handle,
    background: Arc<Mutex<Background>>,
    tab: Tab,
    state: Option<StateFile>,
    selected: Option<String>,
    messages: Vec<chat::Message>,
    last_reload: Instant,
    draft: String,
    /// Feedback line under the message field ("sent", errors…), filled by background tasks.
    status: Arc<Mutex<String>>,
    add_code: String,
    add_nickname: String,
    add_status: String,
    avatars: HashMap<String, Option<egui::TextureHandle>>,
    pictures: HashMap<String, Option<Picture>>,
    autostart: bool,
    share_stats: bool,
    settings_status: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle, background: Arc<Mutex<Background>>) -> Self {
        cc.egui_ctx.set_theme(egui::ThemePreference::System);
        let paths = Paths::detect();
        let share_stats = stats::sharing_enabled(&paths);
        let mut app = Self {
            paths,
            rt,
            background,
            tab: Tab::Friends,
            state: None,
            selected: None,
            messages: Vec::new(),
            last_reload: Instant::now() - Duration::from_secs(60),
            draft: String::new(),
            status: Arc::new(Mutex::new(String::new())),
            add_code: String::new(),
            add_nickname: String::new(),
            add_status: String::new(),
            avatars: HashMap::new(),
            pictures: HashMap::new(),
            autostart: platform::autostart_enabled(),
            share_stats,
            settings_status: String::new(),
        };
        app.reload(true);
        app
    }

    /// Re-reads state.json and the open conversation (every 2 s, or right away).
    fn reload(&mut self, now: bool) {
        if !now && self.last_reload.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_reload = Instant::now();
        self.state = std::fs::read_to_string(self.paths.state()).ok().and_then(|t| serde_json::from_str(&t).ok());
        // Friends from the list, even before the service wrote its first state.
        if self.state.is_none() {
            if let Ok(list) = FriendList::load(&self.paths) {
                if !list.friends.is_empty() {
                    self.state = Some(StateFile {
                        me: archmates::daemon::Me {
                            code: String::new(),
                            invisible: self.paths.is_invisible(),
                            updated_ms: 0,
                            version: update::CURRENT.into(),
                            update_available: None,
                        },
                        friends: list
                            .friends
                            .iter()
                            .map(|f| FriendState {
                                nickname: f.nickname.clone(),
                                code: f.code.clone(),
                                status: Status::Offline,
                                last_seen_ms: None,
                                profile: None,
                                avatar: None,
                                pending: false,
                                unread: 0,
                                offers: Vec::new(),
                            })
                            .collect(),
                    });
                }
            }
        }
        if self.selected.is_none() {
            self.selected = self.friends().first().map(|f| f.code.clone());
        }
        if let Some(code) = &self.selected {
            self.messages = chat::load(&self.paths, code);
        }
    }

    fn friends(&self) -> &[FriendState] {
        self.state.as_ref().map(|s| s.friends.as_slice()).unwrap_or(&[])
    }

    fn my_code(&self) -> String {
        match self.state.as_ref().map(|s| s.me.code.clone()).filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => archmates::config::load_or_create_key(&self.paths).map(|k| k.public().to_string()).unwrap_or_default(),
        }
    }

    /// Runs a command on the service in the background; its answer goes to `status`.
    fn send_command(&self, command: ipc::Command, waiting: &str, ctx: &egui::Context) {
        let status = self.status.clone();
        *status.lock().unwrap() = waiting.to_string();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let text = match ipc::request(&command).await {
                Ok(a) => format!("✓ {}", a.message),
                Err(e) => format!("✗ {e:#}"),
            };
            *status.lock().unwrap() = text;
            ctx.request_repaint();
        });
    }

    fn avatar(&mut self, ctx: &egui::Context, path: Option<&str>) -> Option<egui::TextureHandle> {
        let path = path?;
        self.avatars
            .entry(path.to_string())
            .or_insert_with(|| {
                let img = image::open(path).ok()?;
                let img = img.resize_to_fill(96, 96, image::imageops::FilterType::Triangle).to_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied([96, 96], img.as_raw());
                Some(ctx.load_texture(format!("avatar:{path}"), color, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    fn picture(&mut self, ctx: &egui::Context, path: &str) -> Option<&Picture> {
        self.pictures
            .entry(path.to_string())
            .or_insert_with(|| load_picture(ctx, path))
            .as_ref()
    }
}

const IMAGE_EXT: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

fn is_image(path: &str) -> bool {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    IMAGE_EXT.contains(&ext.as_str()) && Path::new(path).exists()
}

/// Loads a chat picture, shrunk to fit 240×170, plus a heavily blurred copy.
fn load_picture(ctx: &egui::Context, path: &str) -> Option<Picture> {
    let img = image::open(path).ok()?;
    let small = img.resize(480, 340, image::imageops::FilterType::Triangle).to_rgba8();
    let (w, h) = small.dimensions();
    // Blur a tiny copy: cheap, and once stretched back it reveals nothing.
    let tiny = image::imageops::resize(&small, (w / 12).max(1), (h / 12).max(1), image::imageops::FilterType::Triangle);
    let tiny = image::imageops::blur(&tiny, 2.5);
    let sharp = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], small.as_raw());
    let (tw, th) = tiny.dimensions();
    let blurred = egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], tiny.as_raw());
    let scale = (240.0 / w as f32).min(170.0 / h as f32).min(1.0) * 2.0;
    Some(Picture {
        sharp: ctx.load_texture(format!("pic:{path}"), sharp, egui::TextureOptions::LINEAR),
        blurred: ctx.load_texture(format!("pic-blur:{path}"), blurred, egui::TextureOptions::LINEAR),
        size: Vec2::new(w as f32 * scale / 2.0, h as f32 * scale / 2.0),
    })
}

fn status_color(status: Status) -> Color32 {
    match status {
        Status::Online => Color32::from_rgb(0x3f, 0xb9, 0x50),
        Status::Pending => Color32::from_rgb(0xd2, 0x99, 0x22),
        Status::Offline => Color32::from_gray(0x80),
    }
}

fn status_text(f: &FriendState) -> String {
    match f.status {
        Status::Online => "online".into(),
        Status::Pending => "hasn't added you yet".into(),
        Status::Offline => match f.last_seen_ms {
            Some(ms) => format!("offline, seen {}", ago(ms)),
            None => "offline".into(),
        },
    }
}

fn ago(ms: u64) -> String {
    let s = archmates::daemon::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", s / 60),
        3600..=86_399 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86_400),
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_073_741_824.0)
}

/// Round picture (or a placeholder with the first letter).
fn avatar_widget(ui: &mut egui::Ui, tex: Option<egui::TextureHandle>, name: &str, size: f32) {
    match tex {
        Some(tex) => {
            ui.add(egui::Image::new((tex.id(), Vec2::splat(size))).corner_radius(size / 2.0));
        }
        None => {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
            let painter = ui.painter();
            painter.circle_filled(rect.center(), size / 2.0, ui.visuals().widgets.inactive.bg_fill);
            let letter = name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                letter,
                egui::FontId::proportional(size * 0.45),
                ui.visuals().text_color(),
            );
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.reload(false);
        ctx.request_repaint_after(Duration::from_secs(2));
        // "On the page" = the window is the active one and not minimized.
        let on_page = ctx.input(|i| i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false));

        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui));
        match self.tab {
            Tab::Friends => {
                egui::SidePanel::left("friends").resizable(true).default_width(270.0).show(ctx, |ui| self.friend_list(ui, ctx));
                egui::CentralPanel::default().show(ctx, |ui| self.conversation(ui, ctx, on_page));
            }
            Tab::Settings => {
                egui::CentralPanel::default().show(ctx, |ui| self.settings(ui, ctx));
            }
        }
    }
}

impl App {
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.heading(APP_NAME);
            ui.separator();
            ui.selectable_value(&mut self.tab, Tab::Friends, "Friends");
            ui.selectable_value(&mut self.tab, Tab::Settings, "Settings");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (service_ok, update) = {
                    let b = self.background.lock().unwrap();
                    (b.service_ok, b.update.clone())
                };
                if service_ok {
                    ui.label(RichText::new("● service running").color(status_color(Status::Online)).small());
                } else {
                    ui.label(RichText::new("● service starting…").color(status_color(Status::Pending)).small());
                }
                if !update.is_empty() {
                    ui.label(RichText::new(update).small().strong());
                }
            });
        });
        ui.add_space(4.0);
    }

    fn friend_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(6.0);
        ui.label(RichText::new("Your friend code").small().weak());
        let code = self.my_code();
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(short_code(&code)).monospace()).truncate());
            if ui.small_button("Copy").on_hover_text(&code).clicked() {
                ui.ctx().copy_text(code.clone());
            }
        });
        ui.separator();

        let friends: Vec<FriendState> = self.friends().to_vec();
        egui::ScrollArea::vertical().id_salt("friend-scroll").max_height(ui.available_height() - 150.0).show(ui, |ui| {
            if friends.is_empty() {
                ui.label(RichText::new("No friends yet. Add one below with their code.").weak());
            }
            for f in &friends {
                let selected = self.selected.as_deref() == Some(f.code.as_str());
                let tex = self.avatar(ctx, f.avatar.as_deref());
                let row = ui
                    .scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                        let frame = egui::Frame::new()
                            .inner_margin(6.0)
                            .corner_radius(8.0)
                            .fill(if selected { ui.visuals().selection.bg_fill.gamma_multiply(0.35) } else { Color32::TRANSPARENT });
                        frame.show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                avatar_widget(ui, tex, &f.nickname, 36.0);
                                ui.vertical(|ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new("●").color(status_color(f.status)));
                                        ui.label(RichText::new(&f.nickname).strong());
                                        if f.unread > 0 {
                                            ui.label(RichText::new(format!(" {} ", f.unread)).small().background_color(ui.visuals().selection.bg_fill));
                                        }
                                        if !f.offers.is_empty() {
                                            ui.label("📎");
                                        }
                                    });
                                    let sub = match (&f.profile, f.status) {
                                        (Some(p), Status::Online) => p.distro.clone(),
                                        _ => status_text(f),
                                    };
                                    ui.label(RichText::new(sub).small().weak());
                                });
                            });
                        });
                    })
                    .response;
                if row.clicked() {
                    self.selected = Some(f.code.clone());
                    self.reload(true);
                }
            }
        });

        ui.separator();
        ui.label(RichText::new("Add a friend").strong());
        ui.add(egui::TextEdit::singleline(&mut self.add_code).hint_text("their friend code"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.add_nickname).hint_text("nickname").desired_width(140.0));
            if ui.button("Add").clicked() {
                self.add_status = match add_friend(&self.paths, &self.add_code, &self.add_nickname) {
                    Ok(()) => {
                        self.add_code.clear();
                        self.add_nickname.clear();
                        self.reload(true);
                        "✓ Added. They show up online once they add you too.".into()
                    }
                    Err(e) => format!("✗ {e:#}"),
                };
            }
        });
        if !self.add_status.is_empty() {
            ui.label(RichText::new(&self.add_status).small());
        }
    }

    fn conversation(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, on_page: bool) {
        let Some(friend) = self.selected.as_ref().and_then(|c| self.friends().iter().find(|f| &f.code == c)).cloned() else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Pick a friend on the left.").weak()));
            return;
        };
        if on_page && friend.unread > 0 {
            let _ = chat::mark_read(&self.paths, &friend.code);
        }

        // Header: picture, name, profile, stats.
        let tex = self.avatar(ctx, friend.avatar.as_deref());
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            avatar_widget(ui, tex, &friend.nickname, 56.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading(&friend.nickname);
                    ui.label(RichText::new(status_text(&friend)).color(status_color(friend.status)));
                });
                if let Some(p) = &friend.profile {
                    ui.label(RichText::new(format!("{}@{} · {} · {} · {}", p.user, p.host, p.distro, p.kernel, p.wm)).small().weak());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Remove").on_hover_text("Remove this friend from your list").clicked() {
                    if let Ok(mut list) = FriendList::load(&self.paths) {
                        if list.remove(&friend.code) && list.save(&self.paths).is_ok() {
                            self.selected = None;
                            self.reload(true);
                        }
                    }
                }
            });
        });
        if let Some(s) = friend.profile.as_ref().and_then(|p| p.stats.clone()) {
            ui.horizontal(|ui| {
                for (label, used, total) in [("RAM", s.ram_used, s.ram_total), ("Disk", s.disk_used, s.disk_total)] {
                    let pct = if total > 0 { used as f32 / total as f32 } else { 0.0 };
                    ui.label(RichText::new(label).small());
                    ui.add(
                        egui::ProgressBar::new(pct)
                            .desired_width(160.0)
                            .text(RichText::new(format!("{} / {} GB", gb(used), gb(total))).small()),
                    );
                }
            });
        }
        ui.separator();

        // File offers waiting for an answer.
        for o in friend.offers.clone() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("📎 {} wants to send you {} ({:.1} MB)", friend.nickname, o.name, o.size as f64 / 1_048_576.0));
                    if ui.button("Accept").clicked() {
                        self.send_command(ipc::Command::AnswerFile { id: o.id.clone(), accept: true }, &format!("Receiving {}…", o.name), ctx);
                    }
                    if ui.button("Decline").clicked() {
                        self.send_command(ipc::Command::AnswerFile { id: o.id.clone(), accept: false }, "…", ctx);
                    }
                });
            });
        }

        // Message field at the bottom, conversation above it.
        egui::TopBottomPanel::bottom("compose").show_inside(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("📎").on_hover_text("Send a file").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        let path = path.to_string_lossy().into_owned();
                        self.send_command(
                            ipc::Command::SendFile { to: friend.code.clone(), path },
                            "Waiting for your friend to accept the file…",
                            ctx,
                        );
                    }
                }
                let field = ui.add(
                    egui::TextEdit::singleline(&mut self.draft)
                        .hint_text("Message…")
                        .desired_width(ui.available_width() - 60.0),
                );
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button("Send").clicked() || enter) && !self.draft.trim().is_empty() {
                    let text = std::mem::take(&mut self.draft);
                    self.send_command(ipc::Command::SendMessage { to: friend.code.clone(), text }, "Sending…", ctx);
                    field.request_focus();
                    self.last_reload = Instant::now() - Duration::from_secs(60);
                }
            });
            let status = self.status.lock().unwrap().clone();
            ui.label(RichText::new(status).small().weak());
        });

        let messages = self.messages.clone();
        egui::ScrollArea::vertical().id_salt("chat").stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
            if messages.is_empty() {
                ui.label(RichText::new("No messages yet. Say hi!").weak());
            }
            if !on_page && messages.iter().any(|m| m.file.as_deref().is_some_and(is_image)) {
                ui.label(RichText::new(format!("Pictures are blurred while {APP_NAME} is not the active window.")).small().weak());
            }
            for m in &messages {
                let mine = m.dir == "out";
                let layout = if mine { egui::Layout::right_to_left(egui::Align::TOP) } else { egui::Layout::left_to_right(egui::Align::TOP) };
                ui.with_layout(layout, |ui| {
                    let fill = if mine { ui.visuals().selection.bg_fill.gamma_multiply(0.45) } else { ui.visuals().faint_bg_color };
                    egui::Frame::new().fill(fill).corner_radius(10.0).inner_margin(8.0).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.vertical(|ui| {
                            let picture = m.file.as_deref().filter(|f| is_image(f));
                            match picture.and_then(|p| self.picture(ctx, p).map(|pic| (p, pic))) {
                                Some((path, pic)) => {
                                    let tex = if on_page { &pic.sharp } else { &pic.blurred };
                                    let img = egui::Image::new((tex.id(), pic.size)).corner_radius(8.0).sense(egui::Sense::click());
                                    if ui.add(img).on_hover_text("Open").clicked() {
                                        platform::open_path(Path::new(path));
                                    }
                                }
                                None => {
                                    let text = if mine && !m.delivered { format!("{}  ⏳", m.text) } else { m.text.clone() };
                                    if let Some(file) = m.file.as_deref() {
                                        if ui.link(text).on_hover_text(file).clicked() {
                                            platform::open_path(Path::new(file));
                                        }
                                    } else {
                                        ui.add(egui::Label::new(text).wrap().selectable(true));
                                    }
                                }
                            }
                            ui.label(RichText::new(ago(m.ms)).small().weak());
                        });
                    });
                });
                ui.add_space(4.0);
            }
        });
    }

    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(8.0);
        ui.heading("Settings");
        ui.add_space(8.0);

        let label = if cfg!(windows) { "Start Winmates with Windows" } else { "Start the service when I log in (systemd)" };
        if ui.checkbox(&mut self.autostart, label).changed() {
            self.settings_status = match platform::set_autostart(self.autostart) {
                Ok(()) => "✓ Saved.".into(),
                Err(e) => {
                    self.autostart = platform::autostart_enabled();
                    format!("✗ {e:#}")
                }
            };
        }

        let mut invisible = self.paths.is_invisible();
        if ui.checkbox(&mut invisible, "Invisible mode (your friends see you as offline)").changed() {
            if invisible {
                let _ = std::fs::create_dir_all(&self.paths.data);
                let _ = std::fs::write(self.paths.invisible(), b"");
            } else {
                let _ = std::fs::remove_file(self.paths.invisible());
            }
        }

        if ui.checkbox(&mut self.share_stats, "Share my RAM and storage usage with my friends").changed() {
            let _ = stats::set_sharing(&self.paths, self.share_stats);
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Profile picture:");
            if ui.button("Choose…").clicked() {
                if let Some(path) = rfd::FileDialog::new().add_filter("Pictures", &IMAGE_EXT).pick_file() {
                    self.settings_status = match set_avatar(&self.paths, &path) {
                        Ok(()) => "✓ Picture set. Your friends get it within a minute.".into(),
                        Err(e) => format!("✗ {e:#}"),
                    };
                }
            }
            if ui.button("Use the system picture").clicked() {
                let _ = std::fs::remove_file(self.paths.avatar());
                self.settings_status = "✓ Back to your system account picture.".into();
            }
        });

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label("Received files:");
            let dir = archmates::transfer::download_dir().join(archmates::transfer::DOWNLOAD_FOLDER);
            if ui.link(dir.display().to_string()).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                platform::open_path(&dir);
            }
        });

        ui.add_space(12.0);
        ui.separator();
        ui.label(format!("Version {}", update::CURRENT));
        let offer = self.background.lock().unwrap().update_offer.clone();
        if let Some(latest) = offer {
            if ui.button(format!("Install version {latest}")).clicked() {
                let background = self.background.clone();
                let ctx = ctx.clone();
                background.lock().unwrap().update = format!("Installing {latest}…");
                self.rt.spawn(async move {
                    let text = match update::install(&latest).await {
                        Ok(()) => format!("✓ {latest} installed. Restart the app to use it."),
                        Err(e) => format!("✗ {e:#}"),
                    };
                    let mut b = background.lock().unwrap();
                    b.update = text;
                    b.update_offer = None;
                    ctx.request_repaint();
                });
            }
        } else if cfg!(windows) {
            ui.label(RichText::new("Winmates updates itself each time it starts.").small().weak());
        }

        ui.add_space(12.0);
        if ui.button(format!("Quit {APP_NAME}")).on_hover_text("Stops the service too: your friends will see you offline").clicked() {
            let rt = self.rt.clone();
            rt.block_on(async {
                let _ = ipc::request(&ipc::Command::Quit).await;
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.label(RichText::new(format!("Closing the window keeps {APP_NAME} running in the background.")).small().weak());

        if !self.settings_status.is_empty() {
            ui.add_space(8.0);
            ui.label(&self.settings_status);
        }
    }
}

fn short_code(code: &str) -> String {
    if code.len() > 20 { format!("{}…{}", &code[..10], &code[code.len() - 6..]) } else { code.to_string() }
}

fn add_friend(paths: &Paths, code: &str, nickname: &str) -> anyhow::Result<()> {
    let nickname = nickname.trim();
    anyhow::ensure!(!nickname.is_empty(), "pick a nickname");
    let mut list = FriendList::load(paths)?;
    anyhow::ensure!(!list.friends.iter().any(|f| f.nickname == nickname), "you already have a friend named {nickname}");
    list.add(code, nickname)?;
    list.save(paths)
}

fn set_avatar(paths: &Paths, image: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(image)?;
    anyhow::ensure!(archmates::profile::image_kind(&bytes).is_some(), "not a PNG, JPEG, WebP or GIF image");
    anyhow::ensure!(bytes.len() <= 30 * 1024 * 1024, "the image is larger than 30 MB");
    archmates::config::write_atomic(&paths.avatar(), &bytes)
}
