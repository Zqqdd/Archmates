//! The desktop widget (the main window), drawn like the Noctalia widget: title and online
//! count, status lines, your friends, and "Copy my code". Click a friend to open the panel.
//! Drag it anywhere on its card to move it; its height follows the number of friends.

use std::time::{Duration, Instant};

use archmates::service::{FriendState, Status};
use eframe::egui::{self, RichText, Sense, Vec2, vec2};

use crate::app::{Action, App, View};
use crate::theme::{self, Look, alpha, icons};
use crate::widgets::status_text;

/// Same width as the Noctalia widget.
pub const WIDGET_W: f32 = 300.0;

impl App {
    pub(crate) fn desktop_widget(&mut self, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            // The glass: the blurred wallpaper, then the card, rounded like Noctalia's (18 px).
            if let Some(hwnd) = self.widget_hwnd {
                if self.backdrop.as_mut().is_some_and(|b| b.outdated(hwnd)) {
                    self.backdrop = crate::backdrop::Backdrop::load(ctx, hwnd);
                    self.glass = self.backdrop.is_some();
                }
                if let Some(b) = &self.backdrop {
                    b.paint(ui, ui.max_rect(), hwnd, 18.0);
                }
            }
            theme::card(ui, &theme, self.glass, 18);
            // Move the widget by dragging its card. Registered first, so every button and
            // friend row drawn on top of it still gets its clicks.
            let drag = ui.interact(ui.max_rect(), ui.id().with("widget-drag"), Sense::drag());
            if drag.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            let content = egui::Frame::NONE.inner_margin(16).show(ui, |ui| {
                ui.set_width(WIDGET_W - 32.0);
                ui.spacing_mut().item_spacing.y = 10.0;
                self.widget_content(ui, ctx, action);
            });
            // The window takes the height of its content (more friends, taller widget).
            let height = content.response.rect.height().ceil();
            let current = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
            if current.is_some_and(|s| (s.y - height).abs() > 0.5 || (s.x - WIDGET_W).abs() > 0.5) {
                // Min = max = the content: the size follows it, and can't be dragged.
                let size = vec2(WIDGET_W, height);
                ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(size));
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            }
        });
    }

    fn widget_content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        let friends = self.friends();
        let online = friends.iter().filter(|f| f.status == Status::Online).count();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            theme::title(ui, &theme, crate::APP_NAME);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{online}/{} online", friends.len()))
                        .monospace()
                        .size(11.0)
                        .color(theme.muted()),
                );
            });
        });

        // Status lines, as on the Noctalia widget.
        let (service_ok, update_line) = {
            let b = self.background.lock().unwrap();
            (b.service_ok, b.update.clone())
        };
        let me = self.state.as_ref().map(|s| s.me.clone());
        let small = |text: &str, color| RichText::new(text).size(11.0).color(color);
        if !service_ok {
            ui.label(small("Starting the service…", theme::PENDING));
        } else if me
            .as_ref()
            .is_some_and(|m| m.updated_ms > 0 && archmates::time::now_ms().saturating_sub(m.updated_ms) > 120_000)
        {
            ui.label(small("Service stopped (status frozen)", theme::DANGER));
        }
        if update_line.contains("available") {
            ui.label(small(&update_line, theme::PENDING));
        }
        if self.paths.is_invisible() {
            ui.label(small("You are in invisible mode", theme::PENDING));
        }

        if friends.is_empty() {
            ui.label(small("No friends yet. Add one with their code, and send them yours.", theme.muted()));
        } else {
            ui.separator();
            ui.spacing_mut().item_spacing.y = 4.0;
            for f in &friends {
                if self.friend_row(ui, f).clicked() {
                    *action = Some(Action::Open(View::Chat(f.code.clone())));
                }
            }
            ui.spacing_mut().item_spacing.y = 10.0;
        }

        // Your code, as on Noctalia. Add a friend and settings (Windows has no command line for
        // them) only show while the pointer is on the widget, so at rest it looks the same.
        let on_widget = ctx.input(|i| i.pointer.hover_pos().is_some());
        let shown = ctx.animate_bool_with_time(ui.id().with("widget-tools"), on_widget, 0.15);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if shown > 0.0 {
                ui.scope(|ui| {
                    ui.multiply_opacity(shown);
                    if theme::icon_button(ui, &theme, icons::USER_PLUS, "Add a friend", Look::Quiet).clicked() {
                        *action = Some(Action::Open(View::AddFriend));
                    }
                    if theme::icon_button(ui, &theme, icons::SETTINGS, "Settings", Look::Quiet).clicked() {
                        *action = Some(Action::Open(View::Settings));
                    }
                });
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let copied = self.copied_at.is_some_and(|t| t.elapsed() < Duration::from_millis(1500));
                let (icon, text) = if copied { (icons::CHECK, "Copied") } else { (icons::COPY, "Copy my code") };
                let r = theme::quiet_button(ui, &theme, Some(icon), text).on_hover_text("Send it to your friend");
                if r.clicked() {
                    ctx.copy_text(self.my_code());
                    self.copied_at = Some(Instant::now());
                }
                if copied {
                    ctx.request_repaint_after(Duration::from_millis(200));
                }
            });
        });
    }

    /// One friend: picture, status dot, nickname, unread count, and who they are.
    pub(crate) fn friend_row(&mut self, ui: &mut egui::Ui, f: &FriendState) -> egui::Response {
        let theme = self.theme;
        let width = ui.available_width();
        let (rect, response) = ui.allocate_exact_size(vec2(width, 44.0), Sense::click());
        let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.15);
        ui.painter().rect_filled(rect.expand2(vec2(4.0, 0.0)), 10, alpha(theme.fg, 0.06 * hover));
        let mut row = ui
            .new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        row.spacing_mut().item_spacing.x = 10.0;
        self.avatar(&mut row, f, 36.0, false);
        row.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (d, _) = ui.allocate_exact_size(Vec2::splat(8.0), Sense::hover());
                ui.painter().circle_filled(d.center(), 4.0, self.dot_color(f.status));
                ui.label(RichText::new(&f.nickname).font(theme::bold(13.5)));
                if f.unread > 0 {
                    self.badge(ui, f.unread);
                }
                if !f.offers.is_empty() {
                    ui.label(
                        RichText::new(icons::FILE_DOWNLOAD.to_string())
                            .font(theme::icon_font(12.0))
                            .color(theme::PENDING),
                    );
                }
            });
            let sub = match (&f.profile, f.status) {
                (Some(p), Status::Online) => format!("{}@{} · {}", p.user, p.host, p.distro),
                _ => status_text(f),
            };
            ui.add(egui::Label::new(RichText::new(sub).size(11.0).color(theme.muted())).truncate());
        });
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        match &f.profile {
            Some(p) => response.on_hover_text(format!(
                "{}@{}\n{} · kernel {}\n{} · {} · {}\nup {}",
                p.user,
                p.host,
                p.distro,
                p.kernel,
                p.wm,
                p.terminal,
                p.shell,
                crate::widgets::duration(p.uptime_s)
            )),
            None => response,
        }
    }
}
