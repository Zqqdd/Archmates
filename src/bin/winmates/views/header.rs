//! The header: title, back, settings, minimize / close, and dragging the window.

use archmates::service::Status;
use eframe::egui::{self, RichText, Sense};

use crate::APP_NAME;
use crate::app::{Action, App, View};
use crate::theme::{self};

impl App {
    pub(crate) fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, action: &mut Option<Action>) {
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
                    ui.label(
                        RichText::new(format!("{online}/{} online", friends.len()))
                            .monospace()
                            .size(11.0)
                            .color(theme.muted()),
                    );
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
}
