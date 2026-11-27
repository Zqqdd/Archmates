//! A conversation, laid out like the Noctalia friend panel: picture, name, status, ⚙ and ✕;
//! RAM / storage; messages (text, pictures, files, voice) and file offers; the message field,
//! or the recorder while a voice message is being recorded or listened to.

use std::path::Path;
use std::time::Duration;

use archmates::ipc::Command;
use archmates::service::FriendState;
use archmates::voice::desk::DRAFT_ID;
use archmates::voice::{self, DeskState};
use archmates::{chat, platform};
use eframe::egui::{self, Color32, RichText, Sense, Vec2, vec2};

use crate::app::{Action, App};
use crate::pictures::is_image;
use crate::theme::{self, Look, alpha, icons, mix};
use crate::widgets::status_text;
use archmates::time::{ago, gb};

/// Widest a message gets (as in the Noctalia panel).
const BUBBLE_MAX: f32 = 270.0;
const FIELD_H: f32 = 34.0;

/// What a bubble holds, measured before drawing so the bubble fits its content.
enum Content {
    Text(std::sync::Arc<egui::Galley>),
    Picture(String, Vec2),
    File(String),
    Voice,
    ExpiredVoice,
}

impl App {
    pub(crate) fn conversation(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        code: &str,
        on_page: bool,
        action: &mut Option<Action>,
    ) {
        let theme = self.theme;
        let Some(friend) = self.friends().into_iter().find(|f| f.code == code) else {
            ui.label(RichText::new("Friend not found.").color(theme::DANGER));
            return;
        };
        // Mark as read once per new unread count: `unread` only changes when the service
        // publishes again, and writing the file every frame meanwhile is useless.
        let key = (friend.code.clone(), friend.unread);
        if on_page && friend.unread > 0 && self.marked_read.as_ref() != Some(&key) {
            let _ = chat::mark_read(&self.paths, &friend.code);
            self.marked_read = Some(key);
        }

        // Picture, name, who they are, status; ⚙ and ✕.
        self.panel_top(
            ui,
            ctx,
            52.0,
            action,
            |app, ui| {
                app.avatar(ui, &friend, 52.0, false);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(&friend.nickname).font(theme::bold(17.0)));
                    if let Some(p) = &friend.profile {
                        let text = format!("{}@{} · {}", p.user, p.host, p.distro);
                        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(theme.muted())).truncate());
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (d, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                        ui.painter().circle_filled(d.center(), 4.0, app.dot_color(friend.status));
                        ui.label(RichText::new(status_text(&friend)).size(11.0).color(theme.muted()));
                    });
                });
            },
            |_, ui, action| {
                if theme::icon_button(ui, &theme, icons::SETTINGS, "Delete conversation", Look::Quiet).clicked() {
                    *action = Some(Action::ToggleDelete);
                }
            },
        );

        if self.confirm_delete {
            self.delete_box(ui, &friend, action);
        }

        // RAM and storage.
        match friend.profile.as_ref().and_then(|p| p.stats.clone()) {
            Some(s) => {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    let w = (ui.available_width() - 14.0) / 2.0;
                    for (name, used, total) in [("RAM", s.ram_used, s.ram_total), ("Disk", s.disk_used, s.disk_total)] {
                        ui.allocate_ui_with_layout(vec2(w, 22.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 4.0;
                            self.stat(ui, &friend.code, name, used, total, w);
                        });
                    }
                });
            }
            None => {
                ui.label(RichText::new("RAM and storage not shared").size(11.0).color(theme.muted()));
            }
        }
        ui.separator();

        // The conversation fills what the message field and the status line leave.
        let status = self.status.lock().unwrap().clone();
        let desk_mine = self.desk.is_for(&friend.code);
        let error = desk_mine.then(|| self.desk.error.clone()).flatten();
        let has_status = !status.is_empty() || error.is_some();
        let bottom = FIELD_H + 12.0 + if has_status { 14.0 + 12.0 } else { 0.0 };
        let messages = self.messages.clone();
        let focus = ctx.animate_bool_with_time(egui::Id::new("on-page"), on_page, 0.25);
        egui::ScrollArea::vertical()
            .id_salt(("chat", &friend.code))
            .stick_to_bottom(true)
            .max_height((ui.available_height() - bottom).max(60.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if messages.is_empty() {
                    ui.label(RichText::new("No messages yet. Say hi!").size(12.0).color(theme.muted()));
                }
                for m in &messages {
                    self.bubble(ui, ctx, m, focus, action);
                }
                for o in &friend.offers {
                    self.offer_card(ui, &friend, o, action);
                }
            });

        if desk_mine {
            self.recorder(ui, ctx, action);
        } else {
            self.composer(ui, &friend, action);
        }
        match error {
            Some(e) => {
                ui.label(RichText::new(format!("✖ {e}")).size(11.0).color(theme::DANGER));
            }
            None if !status.is_empty() => {
                let color = if status.starts_with('✖') { theme::DANGER } else { theme.muted() };
                ui.label(RichText::new(status).size(11.0).color(color));
            }
            None => {}
        }
    }

    fn stat(&self, ui: &mut egui::Ui, code: &str, name: &str, used: u64, total: u64, width: f32) {
        let theme = self.theme;
        let pct = if total > 0 { used as f32 / total as f32 } else { 0.0 };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(name).size(11.0).color(theme.muted()));
            ui.label(RichText::new(format!("{} / {} GB", gb(used), gb(total))).size(11.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let color = if pct > 0.85 { theme::DANGER } else { theme.accent };
                ui.label(RichText::new(format!("{:.0}%", pct * 100.0)).font(theme::bold(11.0)).color(color));
            });
        });
        self.bar(ui, egui::Id::new(("bar", name, code)), pct, width);
    }

    fn delete_box(&mut self, ui: &mut egui::Ui, friend: &FriendState, action: &mut Option<Action>) {
        let theme = self.theme;
        egui::Frame::NONE.fill(alpha(theme::DANGER, 0.15)).corner_radius(12).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.label(
                RichText::new(format!(
                    "Delete your conversation with {} for good? Every message, voice message and file they sent you \
                     is deleted from this PC. This cannot be undone.",
                    friend.nickname
                ))
                .size(12.0),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let delete = egui::Button::new(RichText::new("Delete").size(12.0).color(Color32::WHITE))
                    .fill(Color32::from_rgb(0xc4, 0x3c, 0x3c))
                    .corner_radius(8)
                    .min_size(vec2(0.0, 30.0));
                if ui.add(delete).clicked() {
                    *action = Some(Action::DeleteChat(friend.code.clone()));
                }
                if theme::quiet_button(ui, &theme, None, "Remove friend")
                    .on_hover_text("Also removes them from your friends")
                    .clicked()
                {
                    *action = Some(Action::RemoveFriend(friend.code.clone()));
                }
                if theme::quiet_button(ui, &theme, None, "Cancel").clicked() {
                    *action = Some(Action::ToggleDelete);
                }
            });
        });
    }

    fn offer_card(
        &mut self,
        ui: &mut egui::Ui,
        friend: &FriendState,
        o: &archmates::service::Offer,
        action: &mut Option<Action>,
    ) {
        let theme = self.theme;
        egui::Frame::NONE.fill(alpha(theme::PENDING, 0.18)).corner_radius(12).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(icons::FILE_DOWNLOAD.to_string()).font(theme::icon_font(16.0)).color(theme::PENDING),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} wants to send you {} ({:.1} MB)",
                            friend.nickname,
                            o.name,
                            o.size as f64 / 1_048_576.0
                        ))
                        .size(12.0),
                    )
                    .wrap(),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                if theme::primary_button(ui, &theme, "Accept").clicked() {
                    *action = Some(Action::Answer(o.id.clone(), true));
                }
                if theme::quiet_button(ui, &theme, None, "Decline").clicked() {
                    *action = Some(Action::Answer(o.id.clone(), false));
                }
            });
        });
    }

    pub(crate) fn bubble(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        m: &chat::Message,
        focus: f32,
        action: &mut Option<Action>,
    ) {
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

        // Measure first: the bubble is as wide as what it holds (up to 270 px).
        let file = m.file.as_deref();
        let content = if m.voice.is_some() {
            if file.is_some_and(voice::available) { Content::Voice } else { Content::ExpiredVoice }
        } else if let Some(path) = file.filter(|f| is_image(f)) {
            match self.picture(ctx, path).map(|p| p.size) {
                Some(size) => Content::Picture(path.to_string(), size),
                None => Content::File(path.to_string()),
            }
        } else if let Some(path) = file {
            Content::File(path.to_string())
        } else {
            let text = if mine && !m.delivered { format!("{}  ⏳", m.text) } else { m.text.clone() };
            Content::Text(ui.painter().layout(text, egui::FontId::proportional(13.0), theme.fg, BUBBLE_MAX))
        };
        let time = ui.painter().layout_no_wrap(ago(m.ms), egui::FontId::proportional(9.0), alpha(theme.fg, 0.6));
        let inner = match &content {
            Content::Text(g) => g.size().x,
            Content::Picture(_, size) => size.x,
            Content::File(_) => {
                ui.painter().layout(m.text.clone(), egui::FontId::proportional(13.0), theme.fg, BUBBLE_MAX).size().x
            }
            Content::Voice => 28.0 + 8.0 + 130.0 + 8.0 + 28.0,
            Content::ExpiredVoice => BUBBLE_MAX,
        }
        .max(time.size().x)
        .min(BUBBLE_MAX)
        .ceil();

        ui.add_space((1.0 - arrive) * 10.0);
        ui.horizontal(|ui| {
            ui.set_opacity(arrive);
            if mine {
                ui.add_space((ui.available_width() - inner - 16.0).max(0.0));
            }
            let fill = if mine { alpha(theme.accent, 0.22) } else { alpha(theme.fg, 0.10) };
            egui::Frame::NONE.fill(fill).corner_radius(12).inner_margin(8).show(ui, |ui| {
                ui.set_width(inner);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    match content {
                        Content::Voice => self.voice_bubble(ui, m, action),
                        Content::ExpiredVoice => {
                            let text =
                                format!("🎤 Voice message · {} · expired", voice::format_secs(m.voice.unwrap_or(0)));
                            ui.add(egui::Label::new(RichText::new(text).size(13.0).color(theme.muted())).wrap());
                        }
                        Content::Picture(path, size) => {
                            if let Some(pic) = self.picture(ctx, &path) {
                                // Cross-fade: blurred when the panel is not the active window.
                                let (rect, r) = ui.allocate_exact_size(size, Sense::click());
                                egui::Image::new((pic.blurred.id(), size)).corner_radius(8).paint_at(ui, rect);
                                egui::Image::new((pic.sharp.id(), size))
                                    .corner_radius(8)
                                    .tint(Color32::WHITE.gamma_multiply(focus))
                                    .paint_at(ui, rect);
                                if r.on_hover_text("Open").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    platform::open_path(Path::new(&path));
                                }
                            }
                        }
                        Content::File(path) => {
                            if ui.link(RichText::new(&m.text).size(13.0)).on_hover_text(&path).clicked() {
                                platform::open_path(Path::new(&path));
                            }
                        }
                        Content::Text(galley) => {
                            ui.add(egui::Label::new(galley).selectable(true));
                        }
                    }
                    ui.label(time);
                });
            });
        });
    }

    /// Play / stop, waveform, length. The service plays it (the same desk for every app).
    pub(crate) fn voice_bubble(&mut self, ui: &mut egui::Ui, m: &chat::Message, action: &mut Option<Action>) {
        let theme = self.theme;
        let path = m.file.clone().unwrap_or_default();
        let progress = self.desk.progress_of(&m.id);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let (icon, hint) = if progress.is_some() {
                (icons::PLAYER_STOP_FILLED, "Stop")
            } else {
                (icons::PLAYER_PLAY_FILLED, "Play")
            };
            if theme::icon_button(ui, &theme, icon, hint, Look::Round).clicked() {
                *action = Some(Action::Voice(if progress.is_some() {
                    Command::VoicePlayStop
                } else {
                    Command::VoicePlay { id: m.id.clone(), path: path.clone() }
                }));
            }
            self.waveform(ui, m.ms, progress.unwrap_or(0.0));
            ui.label(RichText::new(voice::format_secs(m.voice.unwrap_or(0))).size(11.0).color(theme.muted()));
        });
    }

    /// The message field: 📎, 🎤, the field, send. Widths are worked out from the row, so the
    /// send button always fits inside the panel, whatever its size.
    fn composer(&mut self, ui: &mut egui::Ui, friend: &FriendState, action: &mut Option<Action>) {
        let theme = self.theme;
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), FIELD_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if theme::icon_button(ui, &theme, icons::PAPERCLIP, "Send a file", Look::Quiet).clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        *action = Some(Action::SendFile(path.to_string_lossy().into_owned()));
                    }
                }
                if theme::icon_button(ui, &theme, icons::MICROPHONE, "Record a voice message (5 min max)", Look::Quiet)
                    .clicked()
                {
                    *action = Some(Action::Voice(Command::VoiceRecord { to: friend.code.clone() }));
                }
                let field_w = (ui.available_width() - 32.0 - 6.0).max(60.0);
                let field = egui::TextEdit::singleline(&mut self.draft)
                    .hint_text(format!("Message {}…", friend.nickname))
                    .margin(egui::Margin::symmetric(10, 8))
                    .vertical_align(egui::Align::Center);
                let field = ui.add_sized([field_w, FIELD_H], field);
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (theme::icon_button(ui, &theme, icons::SEND, "Send", Look::Primary).clicked() || enter)
                    && !self.draft.trim().is_empty()
                {
                    *action = Some(Action::Send(std::mem::take(&mut self.draft)));
                    field.request_focus();
                }
            },
        );
    }

    /// Recording: ✕, pulsing dot, time, level, ■ (stop and listen), send.
    /// Recorded: 🗑, play / stop, waveform, length, send.
    fn recorder(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        let desk = self.desk.clone();
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), FIELD_H),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if desk.state == DeskState::Recording {
                    ctx.request_repaint_after(Duration::from_millis(40));
                    if theme::icon_button(ui, &theme, icons::X, "Cancel", Look::Quiet).clicked() {
                        *action = Some(Action::Voice(Command::VoiceDiscard));
                    }
                    let pulse = (ctx.input(|i| i.time) * std::f64::consts::PI * 2.0).sin() as f32 * 0.5 + 0.5;
                    let (d, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
                    ui.painter().circle_filled(d.center(), 5.0, mix(alpha(theme::DANGER, 0.45), theme::DANGER, pulse));
                    let secs = desk.secs as u32;
                    ui.label(RichText::new(format!("{} / 5:00", voice::format_secs(secs))).monospace().size(12.0));
                    let w = (ui.available_width() - 2.0 * (32.0 + 6.0)).max(30.0);
                    self.bar(ui, egui::Id::new("mic-level"), (desk.level * 1.6).min(1.0), w);
                    if theme::icon_button(ui, &theme, icons::PLAYER_STOP_FILLED, "Stop and listen", Look::Quiet)
                        .clicked()
                    {
                        *action = Some(Action::Voice(Command::VoiceStop));
                    }
                } else {
                    let sending = desk.state == DeskState::Sending;
                    if theme::icon_button(ui, &theme, icons::TRASH, "Discard", Look::Quiet).clicked() && !sending {
                        *action = Some(Action::Voice(Command::VoiceDiscard));
                    }
                    let progress = desk.progress_of(DRAFT_ID);
                    let (icon, hint) = if progress.is_some() {
                        (icons::PLAYER_STOP_FILLED, "Stop")
                    } else {
                        (icons::PLAYER_PLAY_FILLED, "Listen")
                    };
                    if theme::icon_button(ui, &theme, icon, hint, Look::Round).clicked() {
                        *action = Some(Action::Voice(match (&progress, &desk.draft) {
                            (Some(_), _) => Command::VoicePlayStop,
                            (None, Some(path)) => Command::VoicePlay { id: DRAFT_ID.into(), path: path.clone() },
                            (None, None) => Command::VoicePlayStop,
                        }));
                    }
                    self.waveform(ui, 0, progress.unwrap_or(0.0));
                    let w = (ui.available_width() - 32.0 - 6.0).max(0.0);
                    let secs = voice::format_secs(desk.secs as u32);
                    let label = if sending { format!("{secs} · sending…") } else { secs };
                    ui.allocate_ui_with_layout(
                        vec2(w, FIELD_H),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(RichText::new(label).size(11.0).color(theme.muted()));
                        },
                    );
                }
                if theme::icon_button(ui, &theme, icons::SEND, "Send the voice message", Look::Primary).clicked()
                    && desk.state != DeskState::Sending
                {
                    *action = Some(Action::Voice(Command::VoiceSendRecording));
                }
            },
        );
    }
}
