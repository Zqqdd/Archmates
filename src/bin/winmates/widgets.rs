//! Small drawing helpers shared by the views: avatars, status dots, bars, buttons.

use archmates::service::{FriendState, Status};
use archmates::time::ago;
use eframe::egui::{self, Color32, RichText, Sense, Stroke, Vec2, vec2};

use crate::app::App;
use crate::theme::{self, alpha, mix};

pub fn status_text(f: &FriendState) -> String {
    match f.status {
        Status::Online => "online".into(),
        Status::Pending => "hasn't added you yet".into(),
        Status::Offline => match f.last_seen_ms {
            Some(ms) => format!("last seen {}", ago(ms)),
            None => "never seen".into(),
        },
    }
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

impl App {
    pub fn dot_color(&self, status: Status) -> Color32 {
        match status {
            Status::Online => self.theme.accent,
            Status::Pending => theme::PENDING,
            Status::Offline => alpha(self.theme.fg, 0.35),
        }
    }

    /// Round picture (or the first letter), faded when offline, with a status dot.
    pub fn avatar(&mut self, ui: &mut egui::Ui, f: &FriendState, size: f32, with_dot: bool) {
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
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    letter,
                    egui::FontId::proportional(size * 0.42),
                    alpha(self.theme.fg, 0.8 * faded),
                );
            }
        }
        if with_dot {
            let c = rect.right_bottom() - vec2(size * 0.14, size * 0.14);
            ui.painter().circle(
                c,
                size * 0.13,
                self.dot_color(f.status),
                Stroke::new(2.0_f32, mix(self.theme.bg, self.theme.fg, 0.08)),
            );
        }
    }

    /// A thin bar like the Linux widget's levelbar, sliding to its value.
    pub fn bar(&self, ui: &mut egui::Ui, id: egui::Id, frac: f32, width: f32) {
        let frac = ui.ctx().animate_value_with_time(id, frac.clamp(0.0, 1.0), 0.6);
        let (rect, _) = ui.allocate_exact_size(vec2(width, 5.0), Sense::hover());
        ui.painter().rect_filled(rect, 3, alpha(self.theme.fg, 0.15));
        let fill = egui::Rect::from_min_size(rect.min, vec2(rect.width() * frac, rect.height()));
        let color = if frac > 0.85 { theme::DANGER } else { self.theme.accent };
        ui.painter().rect_filled(fill, 3, color);
    }

    pub fn ghost(&self, ui: &mut egui::Ui, text: &str, hint: &str) -> egui::Response {
        ui.add(egui::Button::new(RichText::new(text).size(15.0)).frame(false).min_size(Vec2::splat(30.0)))
            .on_hover_text(hint)
    }

    pub fn pill(&self, ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.add(egui::Button::new(RichText::new(text).small()).fill(alpha(self.theme.fg, 0.08)).corner_radius(10))
    }

    pub fn accent_button(&self, ui: &mut egui::Ui, text: &str) -> egui::Response {
        ui.add(
            egui::Button::new(RichText::new(text).strong().color(self.theme.on_accent()))
                .fill(self.theme.accent)
                .corner_radius(10)
                .min_size(vec2(34.0, 32.0)),
        )
    }
}
