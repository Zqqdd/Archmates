//! The window: its state (`App`), what clicks ask for (`Action`), and each frame.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archmates::config::{FriendList, Paths};
use archmates::service::control as launcher;
use archmates::service::{FriendState, StateFile, Status};
use archmates::{chat, ipc, platform, stats, update, voice};
use eframe::egui::{self, Sense, Vec2, vec2};

use crate::instance;
use crate::pictures::{Picture, load_picture};
use crate::startup::Background;
use crate::theme::{self, Theme};
use crate::widgets::ease_out;

#[derive(Clone, PartialEq)]
pub enum View {
    List,
    Chat(String),
    Settings,
}

/// What a click asks for, applied once the frame is drawn.
pub enum Action {
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

pub struct App {
    pub(crate) paths: Paths,
    pub(crate) rt: tokio::runtime::Handle,
    pub(crate) background: Arc<Mutex<Background>>,
    pub(crate) show: Arc<AtomicBool>,
    pub(crate) theme: Theme,
    pub(crate) theme_checked: Instant,
    pub(crate) glass: bool,
    pub(crate) view: View,
    /// When the current view appeared, and from which side (for the slide).
    pub(crate) view_since: f64,
    pub(crate) view_forward: bool,
    pub(crate) state: Option<StateFile>,
    pub(crate) messages: Vec<chat::Message>,
    /// Messages already on screen: only new ones get the "arrive" animation.
    pub(crate) seen: HashSet<String>,
    pub(crate) last_reload: Instant,
    pub(crate) draft: String,
    /// Feedback line under the message field ("sent", errors…), filled by background tasks.
    pub(crate) status: Arc<Mutex<String>>,
    pub(crate) add_open: bool,
    pub(crate) add_code: String,
    pub(crate) add_nickname: String,
    pub(crate) add_status: String,
    pub(crate) copied_at: Option<Instant>,
    pub(crate) avatars: HashMap<String, Option<egui::TextureHandle>>,
    pub(crate) pictures: HashMap<String, Option<Picture>>,
    #[cfg(feature = "voice")]
    pub(crate) recorder: Option<voice::Recorder>,
    #[cfg(feature = "voice")]
    pub(crate) player: Option<(String, voice::Player)>,
    pub(crate) confirm_delete: Option<String>,
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
        if let View::Chat(code) = &self.view {
            self.messages = chat::load(&self.paths, code);
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

    pub(crate) fn go(&mut self, ctx: &egui::Context, view: View, forward: bool) {
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
    pub(crate) fn send_command(
        &self,
        command: ipc::Command,
        waiting: &str,
        ctx: &egui::Context,
        cleanup: Option<PathBuf>,
    ) {
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

    pub(crate) fn avatar_texture(&mut self, ctx: &egui::Context, path: Option<&str>) -> Option<egui::TextureHandle> {
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
                let mut child = ui
                    .new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
                child.set_opacity(t);
                match self.view.clone() {
                    View::List => self.list(&mut child, ctx, &mut action),
                    View::Chat(code) => self.conversation(&mut child, ctx, &code, on_page, &mut action),
                    View::Settings => self.settings(&mut child, ctx),
                }
            });
            // Resize from the bottom-right corner (the window has no frame).
            let grip = egui::Rect::from_min_size(full.right_bottom() - vec2(16.0, 16.0), Vec2::splat(16.0));
            let r = ui
                .interact(grip, ui.id().with("grip"), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeSouthEast);
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
    pub(crate) fn apply(&mut self, ctx: &egui::Context, action: Action) {
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
                    self.send_command(
                        ipc::Command::SendFile { to, path },
                        "Waiting for your friend to accept the file…",
                        ctx,
                        None,
                    );
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
                            let tmp = self.paths.cache.join(format!("recording-{}.wav", archmates::time::now_ms()));
                            match archmates::config::write_atomic(&tmp, &wav) {
                                Ok(()) => {
                                    let path = tmp.to_string_lossy().into_owned();
                                    self.send_command(
                                        ipc::Command::SendVoice { to, path },
                                        "Sending the voice message…",
                                        ctx,
                                        Some(tmp),
                                    );
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
            Action::StartRecording
            | Action::CancelRecording
            | Action::SendRecording
            | Action::Play(..)
            | Action::StopPlaying => {}
        }
    }
}

pub(crate) fn add_friend(paths: &Paths, code: &str, nickname: &str) -> anyhow::Result<()> {
    let nickname = nickname.trim();
    anyhow::ensure!(!nickname.is_empty(), "pick a nickname");
    let mut list = FriendList::load(paths)?;
    anyhow::ensure!(!list.friends.iter().any(|f| f.nickname == nickname), "you already have a friend named {nickname}");
    list.add(code, nickname)?;
    list.save(paths)
}

pub(crate) fn set_avatar(paths: &Paths, image: &Path) -> anyhow::Result<()> {
    let bytes = std::fs::read(image)?;
    anyhow::ensure!(archmates::profile::image_kind(&bytes).is_some(), "not a PNG, JPEG, WebP or GIF image");
    anyhow::ensure!(bytes.len() <= 30 * 1024 * 1024, "the image is larger than 30 MB");
    archmates::config::write_atomic(&paths.avatar(), &bytes)
}
