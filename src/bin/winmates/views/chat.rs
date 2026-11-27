//! A conversation (the Noctalia panel): profile, RAM / storage, file offers, messages, pictures, voice, composer.

use std::path::Path;
use std::time::Duration;

use archmates::service::FriendState;
use archmates::{chat, platform, voice};
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2, pos2, vec2};

use crate::app::{Action, App};
use crate::pictures::is_image;
use crate::theme::{self, alpha, mix};
use crate::widgets::status_text;
use archmates::time::{ago, gb};

impl App {
    // ── Friend panel (the Noctalia panel) ──

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
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{}@{} · {}", p.user, p.host, p.distro)).small().color(theme.muted()),
                        )
                        .truncate(),
                    );
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
                ui.label(
                    RichText::new(format!(
                        "📎 {} wants to send you {} ({:.1} MB)",
                        friend.nickname,
                        o.name,
                        o.size as f64 / 1_048_576.0
                    ))
                    .small(),
                );
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
                    for (i, (name, used, total)) in
                        [("RAM", s.ram_used, s.ram_total), ("Disk", s.disk_used, s.disk_total)].into_iter().enumerate()
                    {
                        let ui = &mut cols[i];
                        let pct = if total > 0 { used as f32 / total as f32 } else { 0.0 };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).small().color(theme.muted()));
                            ui.label(RichText::new(format!("{} / {} GB", gb(used), gb(total))).small());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new(format!("{:.0}%", pct * 100.0)).small().strong().color(theme.accent),
                                );
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
        egui::ScrollArea::vertical()
            .id_salt(("chat", code))
            .stick_to_bottom(true)
            .max_height(messages_height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
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
        let layout = if mine {
            egui::Layout::right_to_left(egui::Align::TOP)
        } else {
            egui::Layout::left_to_right(egui::Align::TOP)
        };
        ui.add_space(4.0 + (1.0 - arrive) * 10.0);
        ui.with_layout(layout, |ui| {
            ui.set_opacity(arrive);
            let fill = if mine { alpha(theme.accent, 0.25) } else { alpha(theme.fg, 0.10) };
            egui::Frame::NONE.fill(fill).corner_radius(12).inner_margin(egui::Margin::symmetric(10, 8)).show(
                ui,
                |ui| {
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
                },
            );
        });
    }

    pub(crate) fn voice_bubble(&mut self, ui: &mut egui::Ui, m: &chat::Message, action: &mut Option<Action>) {
        let theme = self.theme;
        let secs = m.voice.unwrap_or(0);
        let path = m.file.clone().unwrap_or_default();
        if !voice::available(&path) {
            ui.label(
                RichText::new(format!("🎤 Voice message · {} · expired", voice::format_secs(secs)))
                    .size(13.0)
                    .color(theme.muted()),
            );
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
            let button = egui::Button::new(RichText::new(if playing { "■" } else { "▶" }).color(theme.on_accent()))
                .fill(theme.accent)
                .corner_radius(14)
                .min_size(Vec2::splat(28.0));
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
                ui.painter().line_segment(
                    [pos2(x, rect.center().y - h / 2.0), pos2(x, rect.center().y + h / 2.0)],
                    Stroke::new(2.5_f32, color),
                );
            }
            ui.label(RichText::new(voice::format_secs(secs)).small().color(theme.muted()));
            if playing {
                ui.ctx().request_repaint_after(Duration::from_millis(40));
            }
        });
    }

    pub(crate) fn composer(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        friend: &FriendState,
        action: &mut Option<Action>,
    ) {
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
                ui.painter().circle_filled(
                    d.center(),
                    4.0 + pulse * 2.0,
                    mix(theme::DANGER, Color32::from_rgb(0xe0, 0x4f, 0x4f), pulse),
                );
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
}
