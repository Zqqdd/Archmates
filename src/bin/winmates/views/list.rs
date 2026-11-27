//! The friends list (the Linux widget card): status lines, friends, copy code, add a friend.

use std::time::{Duration, Instant};

use archmates::service::{FriendState, Status};
use eframe::egui::{self, RichText, Sense, Vec2, vec2};

use crate::app::{Action, App, View, add_friend};
use crate::theme::{self, alpha};
use crate::widgets::status_text;

impl App {
    // ── Friends list (the Linux widget card) ──

    pub(crate) fn list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        let (service_ok, update_line) = {
            let b = self.background.lock().unwrap();
            (b.service_ok, b.update.clone())
        };
        let me = self.state.as_ref().map(|s| s.me.clone());
        if !service_ok {
            ui.label(RichText::new("Starting the service…").small().color(theme::PENDING));
        } else if me
            .as_ref()
            .is_some_and(|m| m.updated_ms > 0 && archmates::time::now_ms().saturating_sub(m.updated_ms) > 120_000)
        {
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
        egui::ScrollArea::vertical()
            .id_salt("friends")
            .max_height((ui.available_height() - bottom_space).max(80.0))
            .show(ui, |ui| {
                if friends.is_empty() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("No friends yet. Add one below with their code, and send them yours.")
                            .small()
                            .color(theme.muted()),
                    );
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
                if self
                    .pill(ui, if copied { "✓ Copied" } else { "Copy my code" })
                    .on_hover_text(self.my_code())
                    .clicked()
                {
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
                    ui.add(
                        egui::TextEdit::singleline(&mut self.add_nickname)
                            .hint_text("nickname")
                            .desired_width(ui.available_width()),
                    );
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
                ui.add(
                    egui::TextEdit::singleline(&mut self.add_code)
                        .hint_text("their friend code")
                        .desired_width(f32::INFINITY),
                );
            }
        });
    }

    pub(crate) fn friend_row(&mut self, ui: &mut egui::Ui, f: &FriendState) -> egui::Response {
        let theme = self.theme;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 48.0), Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.15);
        ui.painter().rect_filled(rect, 10, alpha(theme.fg, 0.07 * hover));
        let inner = rect.shrink2(vec2(6.0, 6.0)).translate(vec2(2.0 * hover, 0.0));
        let mut row = ui
            .new_child(egui::UiBuilder::new().max_rect(inner).layout(egui::Layout::left_to_right(egui::Align::Center)));
        self.avatar(&mut row, f, 36.0, false);
        row.add_space(4.0);
        row.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                let (d, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(d.center(), 4.0, self.dot_color(f.status));
                ui.label(RichText::new(&f.nickname).strong());
                if f.unread > 0 {
                    ui.label(
                        RichText::new(format!(" {} ", f.unread))
                            .small()
                            .strong()
                            .color(theme.on_accent())
                            .background_color(theme.accent),
                    );
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
}
