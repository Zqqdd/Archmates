//! The panel: a second window, opened from the desktop widget, like the Noctalia friend panel
//! (400 × 560, centered on the screen, the same glass). It shows a conversation, the "add a
//! friend" form or the settings, and closes with its ✕ (the widget stays on the desktop).

use eframe::egui::{self, RichText, Sense, vec2};

use crate::app::{Action, App, View, add_friend};
use crate::theme::{self, Look, icons};
use crate::widgets::ease_out;

pub const PANEL_W: f32 = 400.0;
pub const PANEL_H: f32 = 560.0;

impl App {
    pub(crate) fn show_panel(&mut self, ctx: &egui::Context, view: View, action: &mut Option<Action>) {
        let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(vec2(1920.0, 1080.0));
        let builder = egui::ViewportBuilder::default()
            .with_title(crate::desktop::panel_title())
            .with_app_id("winmates")
            .with_inner_size([PANEL_W, PANEL_H])
            .with_min_inner_size([340.0, 420.0])
            .with_position(((monitor - vec2(PANEL_W, PANEL_H)) / 2.0).to_pos2())
            .with_decorations(false)
            .with_transparent(true);
        ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("winmates-panel"), builder, |ctx, _class| {
            if ctx.input(|i| i.viewport().close_requested()) {
                *action = Some(Action::ClosePanel);
            }
            #[cfg(windows)]
            if self.panel_glass.is_none() && self.panel_tries < 60 {
                // Its window exists once drawn: give it the same acrylic as the widget.
                self.panel_tries += 1;
                if let Some(hwnd) = crate::desktop::panel_hwnd() {
                    self.panel_glass = Some(theme::glass_hwnd(hwnd, &self.theme));
                }
                ctx.request_repaint();
            }
            let glass = self.panel_glass.unwrap_or(false);
            let theme = self.theme;
            // "On the page" = the panel is the active window and not minimized.
            let on_page =
                ctx.input(|i| i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false));
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                theme::card(ui, &theme, glass);
                let full = ui.max_rect();
                // Opens like a Noctalia panel: fades in while rising a little.
                let t = ease_out(((ui.input(|i| i.time) - self.panel_since) / 0.22) as f32);
                if t < 1.0 {
                    ctx.request_repaint();
                }
                let area = full.shrink(16.0).translate(vec2(0.0, (1.0 - t) * 12.0));
                let mut content = ui
                    .new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
                content.set_opacity(t);
                content.spacing_mut().item_spacing.y = 12.0;
                match &view {
                    View::Chat(code) => self.conversation(&mut content, ctx, code, on_page, action),
                    View::AddFriend => self.add_friend_view(&mut content, ctx, action),
                    View::Settings => self.settings(&mut content, ctx, action),
                }
                // Resize from the bottom-right corner (the window has no frame).
                let grip = egui::Rect::from_min_size(full.right_bottom() - vec2(16.0, 16.0), egui::Vec2::splat(16.0));
                let r = ui
                    .interact(grip, ui.id().with("grip"), Sense::drag())
                    .on_hover_cursor(egui::CursorIcon::ResizeSouthEast);
                if r.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(egui::ResizeDirection::SouthEast));
                }
            });
        });
    }

    /// A panel's top row: drags the window, ends with the ✕. `right` adds buttons before it.
    pub(crate) fn panel_top(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        height: f32,
        action: &mut Option<Action>,
        left: impl FnOnce(&mut Self, &mut egui::Ui),
        right: impl FnOnce(&mut Self, &mut egui::Ui, &mut Option<Action>),
    ) {
        let theme = self.theme;
        // Exactly `height` tall, whatever its content. The drag zone is registered before the
        // buttons, so they keep their clicks.
        let rect = egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), height));
        let drag = ui.interact(rect, ui.id().with("panel-drag"), Sense::drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        let mut row = ui
            .new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        row.spacing_mut().item_spacing.x = 12.0;
        left(self, &mut row);
        row.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if theme::icon_button(ui, &theme, icons::X, "Close", Look::Quiet).clicked() {
                *action = Some(Action::ClosePanel);
            }
            right(self, ui, action);
        });
        ui.allocate_rect(rect, Sense::hover());
    }

    pub(crate) fn add_friend_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
        let theme = self.theme;
        self.panel_top(
            ui,
            ctx,
            32.0,
            action,
            |_, ui| {
                ui.label(RichText::new("Add a friend").font(theme::bold(17.0)));
            },
            |_, _, _| {},
        );
        ui.label(
            RichText::new("Friends add each other with their codes: send them yours, add theirs below.")
                .size(12.0)
                .color(theme.muted()),
        );
        ui.label(RichText::new("YOUR CODE").font(theme::bold(11.0)).color(theme.accent));
        ui.horizontal(|ui| {
            let code = self.my_code();
            let w = ui.available_width() - 110.0;
            ui.add_sized(
                [w, 30.0],
                egui::Label::new(RichText::new(&code).monospace().size(11.0).color(theme.muted())).truncate(),
            )
            .on_hover_text(&code);
            let copied = self.copied_at.is_some_and(|t| t.elapsed().as_millis() < 1500);
            let (icon, text) = if copied { (icons::CHECK, "Copied") } else { (icons::COPY, "Copy") };
            if theme::quiet_button(ui, &theme, Some(icon), text).clicked() {
                ctx.copy_text(code);
                self.copied_at = Some(std::time::Instant::now());
            }
        });
        ui.label(RichText::new("THEIR CODE").font(theme::bold(11.0)).color(theme.accent));
        ui.add(
            egui::TextEdit::singleline(&mut self.add_code)
                .hint_text("paste your friend's code")
                .margin(egui::Margin::symmetric(10, 8))
                .desired_width(f32::INFINITY),
        );
        ui.label(RichText::new("NICKNAME").font(theme::bold(11.0)).color(theme.accent));
        let nick = ui.add(
            egui::TextEdit::singleline(&mut self.add_nickname)
                .hint_text("how you call them")
                .margin(egui::Margin::symmetric(10, 8))
                .desired_width(f32::INFINITY),
        );
        let enter = nick.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            if theme::primary_button(ui, &theme, "Add").clicked() || enter {
                self.add_status = match add_friend(&self.paths, &self.add_code, &self.add_nickname) {
                    Ok(()) => {
                        self.add_code.clear();
                        self.add_nickname.clear();
                        self.reload(true);
                        "✔ Added. They show up online once they add you too.".into()
                    }
                    Err(e) => format!("✖ {e:#}"),
                };
            }
        });
        if !self.add_status.is_empty() {
            let color = if self.add_status.starts_with('✖') { theme::DANGER } else { theme.muted() };
            ui.label(RichText::new(&self.add_status).size(11.0).color(color));
        }
    }
}
