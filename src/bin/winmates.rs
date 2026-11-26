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

#[path = "../ui/theme.rs"]
mod theme;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archmates::config::{FriendList, Paths};
use archmates::daemon::{FriendState, StateFile, Status};
use archmates::launcher::{self, UpdateResult};
use archmates::{chat, ipc, platform, stats, update, voice};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2, pos2, vec2};
use theme::{Theme, alpha, mix};

/// The app's name: Winmates on Windows, Archmates elsewhere.
const APP_NAME: &str = if cfg!(windows) { "Winmates" } else { "Archmates" };

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    #[cfg(windows)]
    {
        update::cleanup_old_programs();
        if let Some(installed) = windows_install::self_install() {
            relaunch(&installed);
        }
    }

    // v0.4.0 started "winmates.exe --background" at login: now WinmatesUpdater's job.
    if has("--background") {
        runtime.block_on(async {
            #[cfg(windows)]
            windows_install::repair().await;
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

    let background = Arc::new(Mutex::new(Background::default()));
    runtime.spawn(startup(background.clone(), !has("--from-updater")));

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
        Box::new(move |cc| Ok(Box::new(App::new(cc, handle, background, show)))),
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

// ── Single instance ─────────────────────────────────────────────────────────

mod instance {
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
        let Ok(mut server) = ServerOptions::new().first_pipe_instance(true).reject_remote_clients(true).create(&name) else {
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
        let dir = std::env::var("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(|_| archmates::config::Paths::detect().cache);
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
}

// ── Startup: update, service ────────────────────────────────────────────────

#[derive(Default)]
struct Background {
    /// Line shown about updates.
    update: String,
    /// Newer version available but not installed by itself (Linux).
    update_offer: Option<String>,
    service_ok: bool,
    checking: bool,
}

async fn startup(background: Arc<Mutex<Background>>, check_update: bool) {
    #[cfg(windows)]
    windows_install::repair().await;
    if check_update {
        check_for_update(&background).await;
    }
    loop {
        let ok = launcher::ensure_service().await;
        background.lock().unwrap().service_ok = ok;
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

async fn check_for_update(background: &Arc<Mutex<Background>>) {
    background.lock().unwrap().checking = true;
    let result = launcher::update(false).await;
    let mut b = background.lock().unwrap();
    b.checking = false;
    match result {
        UpdateResult::Installed(_v) => {
            #[cfg(windows)]
            relaunch(&launcher::windows::app());
        }
        UpdateResult::Available(v) => {
            b.update = format!("{APP_NAME} {v} is available");
            b.update_offer = Some(v);
        }
        UpdateResult::Failed(e) => b.update = format!("Update check failed: {e}"),
        UpdateResult::UpToDate => b.update = "Up to date".into(),
    }
}

// ── Windows install (zip users, and upgrades from v0.4.0) ───────────────────

#[cfg(windows)]
mod windows_install {
    use std::path::PathBuf;

    use archmates::launcher::windows as win;
    use archmates::{platform, update};

    /// Started from an unzipped release (not from the install folder): installs the programs
    /// found next to it, then returns the installed app to restart from.
    pub fn self_install() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let here = exe.parent()?;
        let dir = platform::install_dir();
        if same_dir(here, &dir) {
            return None;
        }
        let mut programs = Vec::new();
        for name in update::WINDOWS_PROGRAMS {
            if let Ok(bytes) = std::fs::read(here.join(name)) {
                programs.push((name, bytes));
            }
        }
        let borrowed: Vec<(&str, &[u8])> = programs.iter().map(|(n, b)| (*n, b.as_slice())).collect();
        win::install(&borrowed, false, true).ok()?;
        Some(win::app())
    }

    fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
        match (a.canonicalize(), b.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    /// Installs made by v0.4.0 have no WinmatesUpdater: fetch this version again to get it,
    /// and move "start with Windows" over to the updater.
    pub async fn repair() {
        if win::updater().exists() || !win::app().exists() {
            return;
        }
        let autostart = platform::autostart_enabled();
        if update::install(update::CURRENT).await.is_ok() {
            let _ = win::finish_install(false, autostart);
        }
    }
}

// ── The window ──────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum View {
    List,
    Chat(String),
    Settings,
}

/// A picture shown in the chat, sharp and blurred.
struct Picture {
    sharp: egui::TextureHandle,
    blurred: egui::TextureHandle,
    size: Vec2,
}

/// What a click asks for, applied once the frame is drawn.
enum Action {
    Open(View),
    Back,
    Send(String),
    SendFile(String),
    Answer(String, bool),
    StartRecording,
    CancelRecording,
    SendRecording,
    Play(String, String),
    StopPlaying,
    AskDelete(String),
    RemoveFriend(String),
}

struct App {
    paths: Paths,
    rt: tokio::runtime::Handle,
    background: Arc<Mutex<Background>>,
    show: Arc<AtomicBool>,
    theme: Theme,
    theme_checked: Instant,
    glass: bool,
    view: View,
    /// When the current view appeared, and from which side (for the slide).
    view_since: f64,
    view_forward: bool,
    state: Option<StateFile>,
    messages: Vec<chat::Message>,
    /// Messages already on screen: only new ones get the "arrive" animation.
    seen: HashSet<String>,
    last_reload: Instant,
    draft: String,
    /// Feedback line under the message field ("sent", errors…), filled by background tasks.
    status: Arc<Mutex<String>>,
    add_open: bool,
    add_code: String,
    add_nickname: String,
    add_status: String,
    copied_at: Option<Instant>,
    avatars: HashMap<String, Option<egui::TextureHandle>>,
    pictures: HashMap<String, Option<Picture>>,
    #[cfg(feature = "voice")]
    recorder: Option<voice::Recorder>,
    #[cfg(feature = "voice")]
    player: Option<(String, voice::Player)>,
    confirm_delete: Option<String>,
    autostart: bool,
    open_at_login: bool,
    share_stats: bool,
    settings_status: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, rt: tokio::runtime::Handle, background: Arc<Mutex<Background>>, show: Arc<AtomicBool>) -> Self {
        let _ = instance::CTX.set(cc.egui_ctx.clone());
        let theme = Theme::detect();
        theme.apply(&cc.egui_ctx);
        let glass = theme::glass(cc, &theme);
        let paths = Paths::detect();
        let mut app = Self {
            share_stats: stats::sharing_enabled(&paths),
            open_at_login: launcher::open_at_login(&paths),
            paths,
            rt,
            background,
            show,
            theme,
            theme_checked: Instant::now(),
            glass,
            view: View::List,
            view_since: 0.0,
            view_forward: true,
            state: None,
            messages: Vec::new(),
            seen: HashSet::new(),
            last_reload: Instant::now() - Duration::from_secs(60),
            draft: String::new(),
            status: Arc::new(Mutex::new(String::new())),
            add_open: false,
            add_code: String::new(),
            add_nickname: String::new(),
            add_status: String::new(),
            copied_at: None,
            avatars: HashMap::new(),
            pictures: HashMap::new(),
            #[cfg(feature = "voice")]
            recorder: None,
            #[cfg(feature = "voice")]
            player: None,
            confirm_delete: None,
            autostart: platform::autostart_enabled(),
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
        if self.state.is_none() {
            // Friends from the list, even before the service wrote its first state.
            if let Ok(list) = FriendList::load(&self.paths) {
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
        if let View::Chat(code) = &self.view {
            self.messages = chat::load(&self.paths, code);
        }
    }

    fn friends(&self) -> Vec<FriendState> {
        let mut list = self.state.as_ref().map(|s| s.friends.clone()).unwrap_or_default();
        let order = |s: Status| match s {
            Status::Online => 0,
            Status::Pending => 1,
            Status::Offline => 2,
        };
        list.sort_by_key(|f| (order(f.status), f.nickname.to_lowercase()));
        list
    }

    fn my_code(&self) -> String {
        match self.state.as_ref().map(|s| s.me.code.clone()).filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => archmates::config::load_or_create_key(&self.paths).map(|k| k.public().to_string()).unwrap_or_default(),
        }
    }

    fn go(&mut self, ctx: &egui::Context, view: View, forward: bool) {
        if let View::Chat(code) = &view {
            self.messages = chat::load(&self.paths, code);
            // Messages already there don't animate; only the ones arriving from now on.
            self.seen = self.messages.iter().map(|m| m.id.clone()).collect();
            let _ = chat::mark_read(&self.paths, code);
            self.status.lock().unwrap().clear();
        }
        self.view = view;
        self.view_forward = forward;
        self.view_since = ctx.input(|i| i.time);
    }

    /// Runs a command on the service in the background; its answer goes to `status`.
    fn send_command(&self, command: ipc::Command, waiting: &str, ctx: &egui::Context, cleanup: Option<PathBuf>) {
        let status = self.status.clone();
        *status.lock().unwrap() = waiting.to_string();
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let text = match ipc::request(&command).await {
                Ok(a) => format!("✓ {}", a.message),
                Err(e) => format!("✗ {e:#}"),
            };
            if let Some(file) = cleanup {
                let _ = std::fs::remove_file(file);
            }
            *status.lock().unwrap() = text;
            ctx.request_repaint();
        });
    }

    fn avatar_texture(&mut self, ctx: &egui::Context, path: Option<&str>) -> Option<egui::TextureHandle> {
        let path = path?;
        self.avatars
            .entry(path.to_string())
            .or_insert_with(|| {
                let img = image::open(path).ok()?;
                let img = img.resize_to_fill(128, 128, image::imageops::FilterType::Triangle).to_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied([128, 128], img.as_raw());
                Some(ctx.load_texture(format!("avatar:{path}"), color, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    fn picture(&mut self, ctx: &egui::Context, path: &str) -> Option<&Picture> {
        self.pictures.entry(path.to_string()).or_insert_with(|| load_picture(ctx, path)).as_ref()
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
        size: vec2(w as f32 * scale / 2.0, h as f32 * scale / 2.0),
    })
}

fn status_text(f: &FriendState) -> String {
    match f.status {
        Status::Online => "online".into(),
        Status::Pending => "hasn't added you yet".into(),
        Status::Offline => match f.last_seen_ms {
            Some(ms) => format!("last seen {}", ago(ms)),
            None => "never seen".into(),
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

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

// ── Small drawing helpers ───────────────────────────────────────────────────

impl App {
    fn dot_color(&self, status: Status) -> Color32 {
        match status {
            Status::Online => self.theme.accent,
            Status::Pending => theme::PENDING,
            Status::Offline => alpha(self.theme.fg, 0.35),
        }
    }

    /// Round picture (or the first letter), faded when offline, with a status dot.
    fn avatar(&mut self, ui: &mut egui::Ui, f: &FriendState, size: f32, with_dot: bool) {
        let tex = self.avatar_texture(ui.ctx(), f.avatar.as_deref());
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        let faded = if f.status == Status::Online { 1.0 } else { 0.45 };
        let painter = ui.painter();
        painter.circle_filled(rect.center(), size / 2.0, alpha(self.theme.fg, 0.12));
        match tex {
            Some(tex) => {
                egui::Image::new((tex.id(), rect.size()))
                    .corner_radius(size / 2.0)
                    .tint(Color32::WHITE.gamma_multiply(faded))
                    .paint_at(ui, rect);
            }
            None => {
                let letter = f.nickname.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, letter, egui::FontId::proportional(size * 0.42), alpha(self.theme.fg, 0.8 * faded));
            }
        }
        if with_dot {
            let c = rect.right_bottom() - vec2(size * 0.14, size * 0.14);
            ui.painter().circle(c, size * 0.13, self.dot_color(f.status), Stroke::new(2.0_f32, mix(self.theme.bg, self.theme.fg, 0.08)));
        }
    }

    /// A thin bar like the Linux widget's levelbar, sliding to its value.
    fn bar(&self, ui: &mut egui::Ui, id: egui::Id, frac: f32, width: f32) {
        let frac = ui.ctx().animate_value_with_time(id, frac.clamp(0.0, 1.0), 0.6);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 5.0), Sense::hover());
        ui.painter().rect_filled(rect, 3, alpha(self.theme.fg, 0.15));
        let fill = egui::Rect::from_min_size(rect.min, vec2(rect.width() * frac, rect.height()));
        let color = if frac > 0.85 { theme::DANGER } else { self.theme.accent };
        ui.painter().rect_filled(fill, 3, color);
    }

    fn ghost(&self, ui: &mut egui::Ui, text: &str, hint: &str) -> egui::Response {
        ui.add(egui::Button::new(RichText::new(text).size(15.0)).frame(false).min_size(Vec2::splat(30.0))).on_hover_text(hint)
    }

    fn pill(&self, ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.add(egui::Button::new(RichText::new(text).small()).fill(alpha(self.theme.fg, 0.08)).corner_radius(10))
    }

    fn accent_button(&self, ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.add(egui::Button::new(RichText::new(text).strong().color(self.theme.on_accent())).fill(self.theme.accent).corner_radius(10).min_size(vec2(34.0, 32.0)))
    }
}

// ── Frame ───────────────────────────────────────────────────────────────────

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.show.swap(false, Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if self.theme_checked.elapsed() > Duration::from_secs(5) {
            // Colors follow your theme live, like on Linux.
            self.theme_checked = Instant::now();
            let t = Theme::detect();
            if t != self.theme {
                self.theme = t;
                t.apply(ctx);
            }
        }
        self.reload(false);
        ctx.request_repaint_after(Duration::from_secs(2));
        // "On the page" = the window is the active one and not minimized.
        let on_page = ctx.input(|i| i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false));
        let now = ctx.input(|i| i.time);

        let mut action: Option<Action> = None;
        let theme = self.theme;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            theme::card(ui, &theme, self.glass);
            let full = ui.max_rect();
            egui::Frame::NONE.inner_margin(16).show(ui, |ui| {
                ui.set_min_size(full.shrink(16.0).size());
                self.header(ui, ctx, &mut action);
                // Slide + fade between views.
                let t = ease_out(((now - self.view_since) / 0.24) as f32);
                if t < 1.0 {
                    ctx.request_repaint();
                }
                let shift = (1.0 - t) * 26.0 * if self.view_forward { 1.0 } else { -1.0 };
                let area = ui.available_rect_before_wrap().translate(vec2(shift, 0.0));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
                child.set_opacity(t);
                match self.view.clone() {
                    View::List => self.list(&mut child, ctx, &mut action),
                    View::Chat(code) => self.conversation(&mut child, ctx, &code, on_page, &mut action),
                    View::Settings => self.settings(&mut child, ctx),
                }
            });
            // Resize from the bottom-right corner (the window has no frame).
            let grip = egui::Rect::from_min_size(full.right_bottom() - vec2(16.0, 16.0), Vec2::splat(16.0));
            let r = ui.interact(grip, ui.id().with("grip"), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeSouthEast);
            if r.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
            }
        });
        self.delete_dialog(ctx);
        if let Some(a) = action {
            self.apply(ctx, a);
        }
    }
}

impl App {
    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        let row = ui.horizontal(|ui| {
            if self.view != View::List && self.ghost(ui, "←", "Back").clicked() {
                *action = Some(Action::Back);
            }
            match &self.view {
                View::List => theme::title(ui, &theme, &APP_NAME.to_lowercase()),
                View::Settings => {
                    ui.label(RichText::new("settings").monospace().strong().size(14.0).color(theme.accent));
                }
                View::Chat(_) => {
                    ui.label(RichText::new(APP_NAME.to_lowercase()).monospace().size(12.0).color(theme.muted()));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.ghost(ui, "✕", "Close (Winmates keeps running in the background)").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if self.ghost(ui, "—", "Minimize").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
                if self.view == View::List {
                    if self.ghost(ui, "⚙", "Settings").clicked() {
                        *action = Some(Action::Open(View::Settings));
                    }
                    let friends = self.friends();
                    let online = friends.iter().filter(|f| f.status == Status::Online).count();
                    ui.label(RichText::new(format!("{online}/{} online", friends.len())).monospace().size(11.0).color(theme.muted()));
                }
            });
        });
        // Drag the window by its header.
        let drag = ui.interact(row.response.rect, ui.id().with("header-drag"), Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        ui.add_space(6.0);
    }

    // ── Friends list (the Linux widget card) ──

    fn list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        let (service_ok, update_line) = {
            let b = self.background.lock().unwrap();
            (b.service_ok, b.update.clone())
        };
        let me = self.state.as_ref().map(|s| s.me.clone());
        if !service_ok {
            ui.label(RichText::new("Starting the service…").small().color(theme::PENDING));
        } else if me.as_ref().is_some_and(|m| m.updated_ms > 0 && archmates::daemon::now_ms().saturating_sub(m.updated_ms) > 120_000) {
            ui.label(RichText::new("Service stopped (status frozen)").small().color(theme::DANGER));
        }
        if update_line.contains("available") {
            ui.label(RichText::new(&update_line).small().color(theme.accent));
        }
        if self.paths.is_invisible() {
            ui.label(RichText::new("You are in invisible mode").small().color(theme.accent));
        }
        ui.separator();

        let friends = self.friends();
        let bottom_space = if self.add_open { 150.0 } else { 70.0 };
        egui::ScrollArea::vertical().id_salt("friends").max_height((ui.available_height() - bottom_space).max(80.0)).show(ui, |ui| {
            if friends.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("No friends yet. Add one below with their code, and send them yours.").small().color(theme.muted()));
            }
            for f in &friends {
                if self.friend_row(ui, f).clicked() {
                    *action = Some(Action::Open(View::Chat(f.code.clone())));
                }
            }
        });

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
            ui.horizontal(|ui| {
                let copied = self.copied_at.is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
                if self.pill(ui, if copied { "✓ Copied" } else { "Copy my code" }).on_hover_text(self.my_code()).clicked() {
                    ctx.copy_text(self.my_code());
                    self.copied_at = Some(Instant::now());
                }
                if self.pill(ui, if self.add_open { "Cancel" } else { "+ Add a friend" }).clicked() {
                    self.add_open = !self.add_open;
                    self.add_status.clear();
                }
            });
            if self.add_open {
                let open = ctx.animate_bool_with_time(ui.id().with("add"), self.add_open, 0.2);
                ui.set_opacity(open);
                if !self.add_status.is_empty() {
                    ui.label(RichText::new(&self.add_status).small());
                }
                ui.horizontal(|ui| {
                    let add = self.accent_button(ui, "Add");
                    ui.add(egui::TextEdit::singleline(&mut self.add_nickname).hint_text("nickname").desired_width(ui.available_width()));
                    if add.clicked() {
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
                ui.add(egui::TextEdit::singleline(&mut self.add_code).hint_text("their friend code").desired_width(f32::INFINITY));
            }
        });
    }

    fn friend_row(&mut self, ui: &mut egui::Ui, f: &FriendState) -> egui::Response {
        let theme = self.theme;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 48.0), Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.15);
        ui.painter().rect_filled(rect, 10, alpha(theme.fg, 0.07 * hover));
        let inner = rect.shrink2(vec2(6.0, 6.0)).translate(vec2(2.0 * hover, 0.0));
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
        self.avatar(&mut row, f, 36.0, false);
        row.add_space(4.0);
        row.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                let (d, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(d.center(), 4.0, self.dot_color(f.status));
                ui.label(RichText::new(&f.nickname).strong());
                if f.unread > 0 {
                    ui.label(RichText::new(format!(" {} ", f.unread)).small().strong().color(theme.on_accent()).background_color(theme.accent));
                }
                if !f.offers.is_empty() {
                    ui.label(RichText::new("📎").small());
                }
            });
            let sub = match (&f.profile, f.status) {
                (Some(p), Status::Online) => format!("{}@{} · {}", p.user, p.host, p.distro),
                _ => status_text(f),
            };
            ui.add(egui::Label::new(RichText::new(sub).small().color(theme.muted())).truncate());
        });
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        match &f.profile {
            Some(p) => response.on_hover_text(format!(
                "{}@{}\n{} · kernel {}\n{} · {} · {}",
                p.user, p.host, p.distro, p.kernel, p.wm, p.terminal, p.shell
            )),
            None => response,
        }
    }

    // ── Friend panel (the Noctalia panel) ──

    fn conversation(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, code: &str, on_page: bool, action: &mut Option<Action>) {
        let theme = self.theme;
        let Some(friend) = self.friends().into_iter().find(|f| f.code == code) else {
            ui.label(RichText::new("Friend not found.").color(theme::DANGER));
            return;
        };
        if on_page && friend.unread > 0 {
            let _ = chat::mark_read(&self.paths, &friend.code);
        }

        // Picture, name, profile, status, ⚙.
        ui.horizontal(|ui| {
            self.avatar(ui, &friend, 52.0, false);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(&friend.nickname).size(17.0).strong());
                if let Some(p) = &friend.profile {
                    ui.add(egui::Label::new(RichText::new(format!("{}@{} · {}", p.user, p.host, p.distro)).small().color(theme.muted())).truncate());
                }
                ui.horizontal(|ui| {
                    let (d, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                    ui.painter().circle_filled(d.center(), 4.0, self.dot_color(friend.status));
                    ui.label(RichText::new(status_text(&friend)).small().color(theme.muted()));
                });
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                let gear = self.ghost(ui, "⚙", "Conversation settings");
                egui::Popup::menu(&gear).show(|ui| {
                    ui.set_min_width(220.0);
                    if ui.button(RichText::new("🗑  Delete conversation…").color(theme::DANGER)).clicked() {
                        *action = Some(Action::AskDelete(friend.code.clone()));
                    }
                    if ui.button("Remove friend").clicked() {
                        *action = Some(Action::RemoveFriend(friend.code.clone()));
                    }
                });
            });
        });

        // File offers waiting for an answer.
        for o in &friend.offers {
            egui::Frame::NONE.fill(alpha(theme.accent, 0.15)).corner_radius(12).inner_margin(10).show(ui, |ui| {
                ui.label(RichText::new(format!("📎 {} wants to send you {} ({:.1} MB)", friend.nickname, o.name, o.size as f64 / 1_048_576.0)).small());
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("Accept").fill(alpha(theme.accent, 0.45))).clicked() {
                            *action = Some(Action::Answer(o.id.clone(), true));
                        }
                        if ui.button("Decline").clicked() {
                            *action = Some(Action::Answer(o.id.clone(), false));
                        }
                    });
                });
            });
        }

        // RAM and storage.
        match friend.profile.as_ref().and_then(|p| p.stats.clone()) {
            Some(s) => {
                ui.columns(2, |cols| {
                    for (i, (name, used, total)) in [("RAM", s.ram_used, s.ram_total), ("Disk", s.disk_used, s.disk_total)].into_iter().enumerate() {
                        let ui = &mut cols[i];
                        let pct = if total > 0 { used as f32 / total as f32 } else { 0.0 };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).small().color(theme.muted()));
                            ui.label(RichText::new(format!("{} / {} GB", gb(used), gb(total))).small());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(format!("{:.0}%", pct * 100.0)).small().strong().color(theme.accent));
                            });
                        });
                        let w = ui.available_width();
                        self.bar(ui, egui::Id::new(("bar", name, &friend.code)), pct, w);
                    }
                });
            }
            None => {
                ui.label(RichText::new("RAM and storage not shared").small().color(theme.muted()));
            }
        }
        ui.separator();

        // Message field (or the recorder) at the bottom, conversation above it.
        let composer_height = 64.0;
        let messages_height = (ui.available_height() - composer_height).max(60.0);
        let messages = self.messages.clone();
        let focus = ctx.animate_bool_with_time(egui::Id::new("on-page"), on_page, 0.25);
        egui::ScrollArea::vertical().id_salt(("chat", code)).stick_to_bottom(true).max_height(messages_height).auto_shrink([false, false]).show(ui, |ui| {
            if messages.is_empty() {
                ui.label(RichText::new("No messages yet. Say hi!").size(12.0).color(theme.muted()));
            }
            for m in &messages {
                self.bubble(ui, ctx, m, focus, action);
            }
        });
        ui.add_space(6.0);
        self.composer(ui, ctx, &friend, action);
    }

    fn bubble(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, m: &chat::Message, focus: f32, action: &mut Option<Action>) {
        let theme = self.theme;
        let mine = m.dir == "out";
        // New messages fade in and rise a little.
        let id = egui::Id::new(("arrive", &m.id));
        let arrive = if self.seen.insert(m.id.clone()) {
            ctx.animate_bool_with_time(id, false, 0.25);
            ctx.request_repaint();
            0.0
        } else {
            ctx.animate_bool_with_time(id, true, 0.25)
        };
        let layout = if mine { egui::Layout::right_to_left(egui::Align::TOP) } else { egui::Layout::left_to_right(egui::Align::TOP) };
        ui.add_space(4.0 + (1.0 - arrive) * 10.0);
        ui.with_layout(layout, |ui| {
            ui.set_opacity(arrive);
            let fill = if mine { alpha(theme.accent, 0.25) } else { alpha(theme.fg, 0.10) };
            egui::Frame::NONE.fill(fill).corner_radius(12).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                ui.set_max_width(270.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    let file = m.file.as_deref();
                    if m.voice.is_some() {
                        self.voice_bubble(ui, m, action);
                    } else if let Some(path) = file.filter(|f| is_image(f)) {
                        let path = path.to_string();
                        if let Some(pic) = self.picture(ctx, &path) {
                            // Cross-fade: blurred when the window is not the active one.
                            let (rect, r) = ui.allocate_exact_size(pic.size, Sense::click());
                            egui::Image::new((pic.blurred.id(), pic.size)).corner_radius(8).paint_at(ui, rect);
                            egui::Image::new((pic.sharp.id(), pic.size))
                                .corner_radius(8)
                                .tint(Color32::WHITE.gamma_multiply(focus))
                                .paint_at(ui, rect);
                            if r.on_hover_text("Open").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                platform::open_path(Path::new(&path));
                            }
                        }
                    } else if let Some(file) = file {
                        if ui.link(&m.text).on_hover_text(file).clicked() {
                            platform::open_path(Path::new(file));
                        }
                    } else {
                        let text = if mine && !m.delivered { format!("{}  ⏳", m.text) } else { m.text.clone() };
                        ui.add(egui::Label::new(RichText::new(text).size(13.0)).wrap().selectable(true));
                    }
                    ui.label(RichText::new(ago(m.ms)).size(9.0).color(alpha(theme.fg, 0.6)));
                });
            });
        });
    }

    fn voice_bubble(&mut self, ui: &mut egui::Ui, m: &chat::Message, action: &mut Option<Action>) {
        let theme = self.theme;
        let secs = m.voice.unwrap_or(0);
        let path = m.file.clone().unwrap_or_default();
        if !voice::available(&path) {
            ui.label(RichText::new(format!("🎤 Voice message · {} · expired", voice::format_secs(secs))).size(13.0).color(theme.muted()));
            return;
        }
        #[cfg(feature = "voice")]
        let (playing, progress) = match &self.player {
            Some((id, p)) if id == &m.id && !p.is_done() => (true, p.position() / p.secs.max(0.1)),
            _ => (false, 0.0),
        };
        #[cfg(not(feature = "voice"))]
        let (playing, progress) = (false, 0.0f32);
        ui.horizontal(|ui| {
            let button = egui::Button::new(RichText::new(if playing { "■" } else { "▶" }).color(theme.on_accent())).fill(theme.accent).corner_radius(14).min_size(Vec2::splat(28.0));
            if ui.add(button).clicked() {
                *action = Some(if playing { Action::StopPlaying } else { Action::Play(m.id.clone(), path.clone()) });
            }
            // A little waveform that fills up while playing.
            let (rect, _) = ui.allocate_exact_size(vec2(130.0, 24.0), Sense::hover());
            let bars = 26;
            for i in 0..bars {
                let x = rect.left() + i as f32 * (rect.width() / bars as f32);
                let seed = (i as u32).wrapping_mul(2_654_435_761).wrapping_add(m.ms as u32) >> 27;
                let h = 5.0 + seed as f32 * 0.55;
                let done = (i as f32 / bars as f32) < progress;
                let color = if done { theme.accent } else { alpha(theme.fg, 0.35) };
                ui.painter().line_segment([pos2(x, rect.center().y - h / 2.0), pos2(x, rect.center().y + h / 2.0)], Stroke::new(2.5_f32, color));
            }
            ui.label(RichText::new(voice::format_secs(secs)).small().color(theme.muted()));
            if playing {
                ui.ctx().request_repaint_after(Duration::from_millis(40));
            }
        });
    }

    fn composer(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, friend: &FriendState, action: &mut Option<Action>) {
        let theme = self.theme;
        #[cfg(feature = "voice")]
        if let Some(rec) = &self.recorder {
            // Recording: pulsing dot, time, level, cancel / send.
            let secs = rec.elapsed().as_secs() as u32;
            let pulse = (ctx.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
            ctx.request_repaint_after(Duration::from_millis(40));
            if rec.is_full() {
                *action = Some(Action::SendRecording);
            }
            ui.horizontal(|ui| {
                if self.ghost(ui, "✕", "Cancel").clicked() {
                    *action = Some(Action::CancelRecording);
                }
                let (d, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
                ui.painter().circle_filled(d.center(), 4.0 + pulse * 2.0, mix(theme::DANGER, Color32::from_rgb(0xe0, 0x4f, 0x4f), pulse));
                ui.label(RichText::new(format!("{} / 5:00", voice::format_secs(secs))).monospace());
                let level = rec.level();
                let w = (ui.available_width() - 50.0).max(30.0);
                self.bar(ui, egui::Id::new("mic-level"), (level * 1.6).min(1.0), w);
                if self.accent_button(ui, "➤").on_hover_text("Send the voice message").clicked() {
                    *action = Some(Action::SendRecording);
                }
            });
            let status = self.status.lock().unwrap().clone();
            ui.label(RichText::new(status).small().color(theme.muted()));
            return;
        }
        ui.horizontal(|ui| {
            if self.ghost(ui, "📎", "Send a file").clicked() {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    *action = Some(Action::SendFile(path.to_string_lossy().into_owned()));
                }
            }
            if cfg!(feature = "voice") && self.ghost(ui, "🎤", "Record a voice message (5 min max)").clicked() {
                *action = Some(Action::StartRecording);
            }
            let send_w = 40.0;
            let field = egui::TextEdit::singleline(&mut self.draft)
                .hint_text("Message…")
                .margin(egui::Margin::symmetric(10, 7))
                .desired_width(ui.available_width() - send_w);
            let field = ui.add(field);
            let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (self.accent_button(ui, "➤").clicked() || enter) && !self.draft.trim().is_empty() {
                *action = Some(Action::Send(std::mem::take(&mut self.draft)));
                field.request_focus();
            }
        });
        let status = self.status.lock().unwrap().clone();
        ui.label(RichText::new(status).small().color(theme.muted()));
        let _ = friend;
    }

    // ── Settings ──

    fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let theme = self.theme;
        let section = |ui: &mut egui::Ui, title: &str| {
            ui.add_space(8.0);
            ui.label(RichText::new(title).small().strong().color(theme.accent));
        };
        egui::ScrollArea::vertical().id_salt("settings").show(ui, |ui| {
            section(ui, "STARTUP");
            let label = if cfg!(windows) { "Start with Windows (and update by itself)" } else { "Start the service when I log in (systemd)" };
            if ui.checkbox(&mut self.autostart, label).changed() {
                self.settings_status = match platform::set_autostart(self.autostart) {
                    Ok(()) => "✓ Saved".into(),
                    Err(e) => {
                        self.autostart = platform::autostart_enabled();
                        format!("✗ {e:#}")
                    }
                };
            }
            if cfg!(windows) && ui.checkbox(&mut self.open_at_login, format!("Open {APP_NAME} when I log in")).changed() {
                launcher::set_open_at_login(&self.paths, self.open_at_login);
            }

            section(ui, "PRIVACY");
            let mut invisible = self.paths.is_invisible();
            if ui.checkbox(&mut invisible, "Invisible mode (friends see you offline)").changed() {
                if invisible {
                    let _ = std::fs::create_dir_all(&self.paths.data);
                    let _ = std::fs::write(self.paths.invisible(), b"");
                } else {
                    let _ = std::fs::remove_file(self.paths.invisible());
                }
            }
            if ui.checkbox(&mut self.share_stats, "Share my RAM and storage usage").changed() {
                let _ = stats::set_sharing(&self.paths, self.share_stats);
            }
            ui.label(RichText::new("Pictures in the chat are blurred while the window is not active. Voice messages are deleted at every restart.").small().color(theme.muted()));

            section(ui, "PROFILE PICTURE");
            ui.horizontal(|ui| {
                if self.pill(ui, "Choose…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Pictures", &IMAGE_EXT).pick_file() {
                        self.settings_status = match set_avatar(&self.paths, &path) {
                            Ok(()) => "✓ Picture set. Your friends get it within a minute.".into(),
                            Err(e) => format!("✗ {e:#}"),
                        };
                    }
                }
                if self.pill(ui, "Use my account picture").clicked() {
                    let _ = std::fs::remove_file(self.paths.avatar());
                    self.settings_status = "✓ Back to your account picture".into();
                }
            });

            section(ui, "FILES");
            let dir = archmates::transfer::download_dir().join(archmates::transfer::DOWNLOAD_FOLDER);
            if ui.link(RichText::new(dir.display().to_string()).small()).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                platform::open_path(&dir);
            }

            section(ui, "VERSION");
            let (line, offer, checking) = {
                let b = self.background.lock().unwrap();
                (b.update.clone(), b.update_offer.clone(), b.checking)
            };
            ui.label(format!("{APP_NAME} {}", update::CURRENT));
            if !line.is_empty() {
                ui.label(RichText::new(line).small().color(theme.muted()));
            }
            ui.horizontal(|ui| {
                if checking {
                    ui.spinner();
                } else if self.pill(ui, "Check for updates").clicked() {
                    let b = self.background.clone();
                    let ctx = ctx.clone();
                    self.rt.spawn(async move {
                        check_for_update(&b).await;
                        ctx.request_repaint();
                    });
                }
                if let Some(latest) = offer {
                    if self.pill(ui, &format!("Install {latest}")).clicked() {
                        let b = self.background.clone();
                        let ctx = ctx.clone();
                        b.lock().unwrap().update = format!("Installing {latest}…");
                        self.rt.spawn(async move {
                            let text = match update::install(&latest).await {
                                Ok(()) => format!("✓ {latest} installed. Restart the app to use it."),
                                Err(e) => format!("✗ {e:#}"),
                            };
                            let mut g = b.lock().unwrap();
                            g.update = text;
                            g.update_offer = None;
                            ctx.request_repaint();
                        });
                    }
                }
            });
            if cfg!(windows) {
                ui.label(RichText::new("WinmatesUpdater checks for a new version at every login.").small().color(theme.muted()));
            }

            ui.add_space(14.0);
            let quit = egui::Button::new(RichText::new(format!("Quit {APP_NAME}")).color(theme::DANGER)).fill(alpha(theme::DANGER, 0.12));
            if ui.add(quit).on_hover_text("Stops the service too: your friends will see you offline").clicked() {
                self.rt.block_on(async {
                    let _ = ipc::request(&ipc::Command::Quit).await;
                });
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.label(RichText::new(format!("Closing the window keeps {APP_NAME} running in the background.")).small().color(theme.muted()));
            if !self.settings_status.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.settings_status).small());
            }
        });
    }

    /// "Delete conversation?" confirmation.
    fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(code) = self.confirm_delete.clone() else { return };
        let nickname = self.friends().into_iter().find(|f| f.code == code).map(|f| f.nickname).unwrap_or_default();
        let theme = self.theme;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("delete-chat")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.label(RichText::new(format!("Delete your conversation with {nickname}?")).strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Every message and voice message, and every file {nickname} sent you ({}), is deleted from this PC for good. Files you sent stay where they are.",
                    archmates::transfer::received_dir(&nickname).display()
                ))
                .small()
                .color(theme.muted()),
            );
            ui.add_space(4.0);
            ui.label(RichText::new("This cannot be undone.").small().color(theme::DANGER));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let delete = egui::Button::new(RichText::new("Delete").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0xc4, 0x3c, 0x3c));
                    if ui.add(delete).clicked() {
                        let result = chat::delete_conversation(&self.paths, &code, &nickname);
                        *self.status.lock().unwrap() = match result {
                            Ok(d) => format!("✓ Conversation deleted ({} messages, {} files)", d.messages, d.files),
                            Err(e) => format!("✗ {e:#}"),
                        };
                        self.messages.clear();
                        self.pictures.clear();
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        });
        if close || modal.should_close() {
            self.confirm_delete = None;
        }
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        let friend_code = match &self.view {
            View::Chat(c) => Some(c.clone()),
            _ => None,
        };
        match action {
            Action::Open(view) => self.go(ctx, view, true),
            Action::Back => {
                #[cfg(feature = "voice")]
                {
                    self.recorder = None;
                }
                self.go(ctx, View::List, false)
            }
            Action::Send(text) => {
                if let Some(to) = friend_code {
                    self.send_command(ipc::Command::SendMessage { to, text }, "Sending…", ctx, None);
                    self.last_reload = Instant::now() - Duration::from_secs(60);
                }
            }
            Action::SendFile(path) => {
                if let Some(to) = friend_code {
                    self.send_command(ipc::Command::SendFile { to, path }, "Waiting for your friend to accept the file…", ctx, None);
                }
            }
            Action::Answer(id, accept) => {
                let waiting = if accept { "Receiving…" } else { "…" };
                self.send_command(ipc::Command::AnswerFile { id, accept }, waiting, ctx, None);
            }
            Action::AskDelete(code) => self.confirm_delete = Some(code),
            Action::RemoveFriend(code) => {
                if let Ok(mut list) = FriendList::load(&self.paths) {
                    if list.remove(&code) && list.save(&self.paths).is_ok() {
                        self.reload(true);
                        self.go(ctx, View::List, false);
                    }
                }
            }
            #[cfg(feature = "voice")]
            Action::StartRecording => match voice::Recorder::start() {
                Ok(r) => {
                    self.recorder = Some(r);
                    *self.status.lock().unwrap() = "Recording… (5 min max)".into();
                }
                Err(e) => *self.status.lock().unwrap() = format!("✗ {e:#}"),
            },
            #[cfg(feature = "voice")]
            Action::CancelRecording => {
                self.recorder = None;
                self.status.lock().unwrap().clear();
            }
            #[cfg(feature = "voice")]
            Action::SendRecording => {
                if let (Some(rec), Some(to)) = (self.recorder.take(), friend_code) {
                    match rec.finish() {
                        Ok(wav) => {
                            let tmp = self.paths.cache.join(format!("recording-{}.wav", archmates::daemon::now_ms()));
                            match archmates::config::write_atomic(&tmp, &wav) {
                                Ok(()) => {
                                    let path = tmp.to_string_lossy().into_owned();
                                    self.send_command(ipc::Command::SendVoice { to, path }, "Sending the voice message…", ctx, Some(tmp));
                                }
                                Err(e) => *self.status.lock().unwrap() = format!("✗ {e:#}"),
                            }
                        }
                        Err(e) => *self.status.lock().unwrap() = format!("✗ {e:#}"),
                    }
                }
            }
            #[cfg(feature = "voice")]
            Action::Play(id, path) => match voice::Player::play(Path::new(&path)) {
                Ok(p) => self.player = Some((id, p)),
                Err(e) => *self.status.lock().unwrap() = format!("✗ {e:#}"),
            },
            #[cfg(feature = "voice")]
            Action::StopPlaying => self.player = None,
            #[cfg(not(feature = "voice"))]
            Action::StartRecording | Action::CancelRecording | Action::SendRecording | Action::Play(..) | Action::StopPlaying => {}
        }
    }
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
