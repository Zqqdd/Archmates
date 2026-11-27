//! Winmates' state (`App`), what clicks ask for (`Action`), and each frame.
//!
//! Two windows, like Noctalia's widget and panel:
//! - the **desktop widget** (the main window, `views/widget.rs`): your friends and who is
//!   online, pinned to the desktop under your windows (`desktop.rs`);
//! - the **panel** (`views/panel.rs`), opened on demand from the widget: a conversation,
//!   adding a friend, or the settings. Closing it leaves the widget on the desktop.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archmates::config::{FriendList, Paths};
use archmates::service::control as launcher;
use archmates::service::{FriendState, StateFile, Status};
use archmates::voice::{DeskState, DeskStatus};
use archmates::{chat, ipc, platform, stats, update};
use eframe::egui;

use crate::instance;
use crate::pictures::{Picture, load_picture};
use crate::startup::Background;
use crate::theme::{self, Theme};

/// What the panel shows.
#[derive(Clone, PartialEq, Debug)]
pub enum View {
    Chat(String),
    AddFriend,
    Settings,
}

/// What a click asks for, applied once the frame is drawn.
pub enum Action {
    Open(View),
    ClosePanel,
    Send(String),
    SendFile(String),
    Answer(String, bool),
    /// A voice desk request (record, stop, send, discard, play, stop playing).
    Voice(ipc::Command),
    ToggleDelete,
    DeleteChat(String),
    RemoveFriend(String),
}

pub struct App {
    pub(crate) paths: Paths,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) background: Arc<Mutex<Background>>,
    pub(crate) show: Arc<AtomicBool>,
    pub(crate) theme: Theme,
    pub(crate) theme_checked: Instant,
    /// Frosted glass behind the widget (Windows acrylic).
    pub(crate) glass: bool,
    /// The widget is above your windows until you click elsewhere (after starting Winmates
    /// again while it runs), else on the desktop under them.
    pub(crate) raised: bool,
    pub(crate) panel: Option<View>,
    pub(crate) panel_since: f64,
    /// Glass behind the panel: None until tried on its window (it is found once on screen).
    pub(crate) panel_glass: Option<bool>,
    pub(crate) panel_tries: u32,
    pub(crate) state: Option<StateFile>,
    pub(crate) messages: Vec<chat::Message>,
    /// Messages already on screen: only new ones get the "arrive" animation.
    pub(crate) seen: HashSet<String>,
    pub(crate) last_reload: Instant,
    pub(crate) desk: DeskStatus,
    pub(crate) desk_read: Instant,
    pub(crate) draft: String,
    /// Feedback line under the message field ("sent", errors…), filled by background tasks.
    pub(crate) status: Arc<Mutex<String>>,
    pub(crate) add_code: String,
    pub(crate) add_nickname: String,
    pub(crate) add_status: String,
    pub(crate) copied_at: Option<Instant>,
    pub(crate) avatars: HashMap<String, Option<egui::TextureHandle>>,
    pub(crate) pictures: HashMap<String, Option<Picture>>,
    /// ⚙ in a conversation: "delete it for good?" box.
    pub(crate) confirm_delete: bool,
    /// Last (friend, unread count) marked as read (see `conversation`).
    pub(crate) marked_read: Option<(String, usize)>,
    pub(crate) autostart: bool,
    pub(crate) open_at_login: bool,
    pub(crate) share_stats: bool,
    pub(crate) settings_status: String,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        rt: tokio::runtime::Handle,
        background: Arc<Mutex<Background>>,
        show: Arc<AtomicBool>,
    ) -> Self {
        let _ = instance::CTX.set(cc.egui_ctx.clone());
        theme::install_fonts(&cc.egui_ctx);
        let theme = Theme::detect();
        theme.apply(&cc.egui_ctx);
        let glass = theme::glass(cc, &theme);
        crate::desktop::pin(cc);
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
            raised: false,
            panel: None,
            panel_since: 0.0,
            panel_glass: None,
            panel_tries: 0,
            state: None,
            messages: Vec::new(),
            seen: HashSet::new(),
            last_reload: Instant::now() - Duration::from_secs(60),
            desk: DeskStatus::default(),
            desk_read: Instant::now() - Duration::from_secs(60),
            draft: String::new(),
            status: Arc::new(Mutex::new(String::new())),
            add_code: String::new(),
            add_nickname: String::new(),
            add_status: String::new(),
            copied_at: None,
            avatars: HashMap::new(),
            pictures: HashMap::new(),
            confirm_delete: false,
            marked_read: None,
            autostart: platform::autostart_enabled(),
            settings_status: String::new(),
        };
        app.reload(true);
        app
    }

    /// Re-reads state.json and the open conversation (every 2 s, or right away).
    pub(crate) fn reload(&mut self, now: bool) {
        if !now && self.last_reload.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_reload = Instant::now();
        self.state = std::fs::read_to_string(self.paths.state()).ok().and_then(|t| serde_json::from_str(&t).ok());
        if self.state.is_none() {
            // Friends from the list, even before the service wrote its first state.
            if let Ok(list) = FriendList::load(&self.paths) {
                self.state = Some(StateFile {
                    me: archmates::service::Me {
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
        if let Some(View::Chat(code)) = &self.panel {
            self.messages = chat::load(&self.paths, code);
        }
    }

    /// The voice desk (what the service records or plays): ten times a second while busy.
    fn read_desk(&mut self) {
        let busy = self.desk.state != DeskState::Idle || self.desk.playing.is_some();
        let every = if busy { 100 } else { 500 };
        if self.desk_read.elapsed() >= Duration::from_millis(every) {
            self.desk_read = Instant::now();
            self.desk = DeskStatus::read(&self.paths);
        }
    }

    pub(crate) fn friends(&self) -> Vec<FriendState> {
        let mut list = self.state.as_ref().map(|s| s.friends.clone()).unwrap_or_default();
        let order = |s: Status| match s {
            Status::Online => 0,
            Status::Pending => 1,
            Status::Offline => 2,
        };
        list.sort_by_key(|f| (order(f.status), f.nickname.to_lowercase()));
        list
    }

    pub(crate) fn my_code(&self) -> String {
        match self.state.as_ref().map(|s| s.me.code.clone()).filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => {
                archmates::config::load_or_create_key(&self.paths).map(|k| k.public().to_string()).unwrap_or_default()
            }
        }
    }

    /// Opens the panel on `view` (or switches it there).
    pub(crate) fn open(&mut self, ctx: &egui::Context, view: View) {
        if let View::Chat(code) = &view {
            self.messages = chat::load(&self.paths, code);
            // Messages already there don't animate; only the ones arriving from now on.
            self.seen = self.messages.iter().map(|m| m.id.clone()).collect();
            let _ = chat::mark_read(&self.paths, code);
            self.status.lock().unwrap().clear();
            self.draft.clear();
        }
        if self.panel.is_none() {
            self.panel_glass = None;
            self.panel_tries = 0;
        }
        if self.panel.as_ref() != Some(&view) {
            self.panel_since = ctx.input(|i| i.time);
        }
        self.confirm_delete = false;
        self.add_status.clear();
        self.panel = Some(view);
    }

    /// Runs a command on the service in the background; its answer goes to `status`
    /// (unless `quiet` and it worked).
    pub(crate) fn send_command(
        &self,
        command: ipc::Command,
        waiting: &str,
        ctx: &egui::Context,
        cleanup: Option<PathBuf>,
        quiet: bool,
    ) {
        let status = self.status.clone();
        if !waiting.is_empty() {
            *status.lock().unwrap() = waiting.to_string();
        }
        let ctx = ctx.clone();
        self.rt.spawn(async move {
            let text = match ipc::request(&command).await {
                Ok(_) if quiet => None,
                Ok(a) => Some(format!("✔ {}", a.message)),
                Err(e) => Some(format!("✖ {e:#}")),
            };
            if let Some(file) = cleanup {
                let _ = std::fs::remove_file(file);
            }
            if let Some(text) = text {
                *status.lock().unwrap() = text;
            }
            ctx.request_repaint();
        });
    }

    pub(crate) fn avatar_texture(&mut self, ctx: &egui::Context, path: Option<&str>) -> Option<egui::TextureHandle> {
        let path = path?;
        self.avatars
            .entry(path.to_string())
            .or_insert_with(|| {
                // Saved without an extension: the format is read from the file itself.
                let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
                let img = img.resize_to_fill(128, 128, image::imageops::FilterType::Triangle).to_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied([128, 128], img.as_raw());
                Some(ctx.load_texture(format!("avatar:{path}"), color, egui::TextureOptions::LINEAR))
            })
            .clone()
    }

    pub(crate) fn picture(&mut self, ctx: &egui::Context, path: &str) -> Option<&Picture> {
        self.pictures.entry(path.to_string()).or_insert_with(|| load_picture(ctx, path)).as_ref()
    }
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.show.swap(false, Ordering::SeqCst) {
            // Started again while running: the widget comes above your windows until you
            // click elsewhere.
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.raised = true;
        } else if self.raised && ctx.input(|i| i.viewport().focused) == Some(false) {
            self.raised = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(crate::desktop::WIDGET_LEVEL));
        }
        let asked = instance::OPEN.lock().unwrap().take();
        if let Some(who) = asked {
            let wanted = who.to_lowercase();
            let friend = self.friends().into_iter().find(|f| f.code == who || f.nickname.to_lowercase() == wanted);
            if let Some(f) = friend {
                self.open(ctx, View::Chat(f.code));
            }
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
        self.read_desk();
        let busy = self.desk.state != DeskState::Idle || self.desk.playing.is_some();
        ctx.request_repaint_after(Duration::from_millis(if busy { 50 } else { 2000 }));

        let mut action: Option<Action> = None;
        self.desktop_widget(ctx, &mut action);
        if let Some(view) = self.panel.clone() {
            self.show_panel(ctx, view, &mut action);
        }
        if let Some(a) = action {
            self.apply(ctx, a);
        }
    }
}

impl App {
    pub(crate) fn apply(&mut self, ctx: &egui::Context, action: Action) {
        let friend_code = match &self.panel {
            Some(View::Chat(c)) => Some(c.clone()),
            _ => None,
        };
        match action {
            Action::Open(view) => self.open(ctx, view),
            Action::ClosePanel => {
                // A recording for this friend is kept: the widget shows it again when reopened.
                self.panel = None;
            }
            Action::Send(text) => {
                if let Some(to) = friend_code {
                    self.send_command(ipc::Command::SendMessage { to, text }, "Sending…", ctx, None, false);
                    self.last_reload = Instant::now() - Duration::from_secs(60);
                }
            }
            Action::SendFile(path) => {
                if let Some(to) = friend_code {
                    self.send_command(
                        ipc::Command::SendFile { to, path },
                        "Waiting for your friend to accept the file…",
                        ctx,
                        None,
                        false,
                    );
                }
            }
            Action::Answer(id, accept) => {
                let waiting = if accept { "Receiving…" } else { "…" };
                self.send_command(ipc::Command::AnswerFile { id, accept }, waiting, ctx, None, false);
            }
            Action::Voice(command) => {
                // Read the desk again right away, so the recorder follows without waiting.
                self.desk_read = Instant::now() - Duration::from_secs(60);
                let sending = matches!(command, ipc::Command::VoiceSendRecording);
                let waiting = if sending { "Sending the voice message…" } else { "" };
                if !sending {
                    self.status.lock().unwrap().clear();
                }
                self.send_command(command, waiting, ctx, None, !sending);
            }
            Action::ToggleDelete => self.confirm_delete = !self.confirm_delete,
            Action::DeleteChat(code) => {
                self.confirm_delete = false;
                let nickname =
                    self.friends().into_iter().find(|f| f.code == code).map(|f| f.nickname).unwrap_or_default();
                *self.status.lock().unwrap() = match chat::delete_conversation(&self.paths, &code, &nickname) {
                    Ok(d) => format!("✔ Conversation deleted ({} messages, {} files)", d.messages, d.files),
                    Err(e) => format!("✖ {e:#}"),
                };
                self.messages.clear();
                self.pictures.clear();
            }
            Action::RemoveFriend(code) => {
                if let Ok(mut list) = FriendList::load(&self.paths) {
                    if list.remove(&code) && list.save(&self.paths).is_ok() {
                        self.reload(true);
                        self.panel = None;
                    }
                }
            }
        }
    }
}

pub(crate) fn add_friend(paths: &Paths, code: &str, nickname: &str) -> anyhow::Result<()> {
    let nickname = nickname.trim();
    anyhow::ensure!(!nickname.is_empty(), "pick a nickname");
    let mut list = FriendList::load(paths)?;
    list.add(code.trim(), nickname)?;
    list.save(paths)
}

pub(crate) fn set_avatar(paths: &Paths, image: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(image)?;
    anyhow::ensure!(archmates::profile::image_kind(&bytes).is_some(), "not a PNG, JPEG, WebP or GIF image");
    anyhow::ensure!(bytes.len() <= 30 * 1024 * 1024, "the image is larger than 30 MB");
    archmates::config::write_atomic(&paths.avatar(), &bytes)
}
