//! The Archmates look, for Winmates and its installer: the same frosted card, colors and
//! shapes as the Linux widget and Noctalia panel.
//!
//! Colors (bg / fg / accent), like the Linux widget: your pywal / wallust colors if you use them
//! (`~/.cache/wal/colors.json`, also written by winwal on Windows), else on Windows your accent
//! color and light / dark mode, else the built-in rice colors.

#![allow(dead_code)]

use eframe::egui::{self, Color32, CornerRadius, Stroke};

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    pub bg: Color32,
    pub fg: Color32,
    pub accent: Color32,
    pub dark: bool,
}

/// Same defaults as the Linux widget.
pub const FALLBACK: Theme = Theme {
    bg: Color32::from_rgb(0x14, 0x14, 0x18),
    fg: Color32::from_rgb(0xec, 0xe6, 0xe1),
    accent: Color32::from_rgb(0xf0, 0xa8, 0x68),
    dark: true,
};

pub const PENDING: Color32 = Color32::from_rgb(0xe6, 0xc0, 0x7b);
pub const DANGER: Color32 = Color32::from_rgb(0xf2, 0xb8, 0xb5);

fn hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() < 6 {
        return None;
    }
    let n = u32::from_str_radix(&s[..6], 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

fn luminance(c: Color32) -> f32 {
    (0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32) / 255.0
}

/// `c` with an opacity (0..1), like GTK's alpha(@color, a).
pub fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// Mix two colors (t = 0 → a, 1 → b).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)) as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn pywal() -> Option<Theme> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let txt = std::fs::read_to_string(std::path::Path::new(&home).join(".cache/wal/colors.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&txt).ok()?;
    let bg = hex(json["special"]["background"].as_str()?)?;
    let fg = hex(json["special"]["foreground"].as_str()?)?;
    let accent = hex(json["colors"]["color4"].as_str().or(json["colors"]["color1"].as_str())?)?;
    Some(Theme { bg, fg, accent, dark: luminance(bg) < 0.5 })
}

#[cfg(windows)]
fn windows_colors() -> Option<Theme> {
    use archmates::platform::reg;
    let light = reg::get_dword(
        reg::CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "AppsUseLightTheme",
    )
    .unwrap_or(0)
        == 1;
    let base = if light {
        Theme {
            bg: Color32::from_rgb(0xf4, 0xf1, 0xee),
            fg: Color32::from_rgb(0x1c, 0x1b, 0x1f),
            accent: FALLBACK.accent,
            dark: false,
        }
    } else {
        FALLBACK
    };
    // AccentColor is 0xAABBGGRR.
    let accent = reg::get_dword(reg::CURRENT_USER, r"Software\Microsoft\Windows\DWM", "AccentColor")
        .map(|v| Color32::from_rgb(v as u8, (v >> 8) as u8, (v >> 16) as u8));
    // A very dark accent on a dark card (or light on light) would be unreadable: keep ours.
    let accent = accent.filter(|a| (luminance(*a) - luminance(base.bg)).abs() > 0.25).unwrap_or(base.accent);
    Some(Theme { accent, ..base })
}

impl Theme {
    pub fn detect() -> Self {
        if let Some(t) = pywal() {
            return t;
        }
        #[cfg(windows)]
        if let Some(t) = windows_colors() {
            return t;
        }
        FALLBACK
    }

    /// Text on the accent color (send button, badges).
    pub fn on_accent(&self) -> Color32 {
        if luminance(self.accent) > 0.55 { Color32::from_rgb(0x14, 0x14, 0x18) } else { Color32::WHITE }
    }

    pub fn muted(&self) -> Color32 {
        alpha(self.fg, 0.68)
    }

    /// Fonts, rounding, colors and spacing of every egui widget.
    pub fn apply(&self, ctx: &egui::Context) {
        let mut visuals = if self.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        visuals.override_text_color = Some(self.fg);
        visuals.panel_fill = Color32::TRANSPARENT;
        visuals.window_fill = mix(self.bg, self.fg, 0.04);
        visuals.window_stroke = Stroke::new(1.0_f32, alpha(self.fg, 0.22));
        visuals.window_corner_radius = CornerRadius::same(16);
        visuals.menu_corner_radius = CornerRadius::same(12);
        visuals.extreme_bg_color = alpha(self.bg, 0.55);
        visuals.faint_bg_color = alpha(self.fg, 0.06);
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = alpha(self.accent, 0.45);
        visuals.selection.stroke = Stroke::new(1.0_f32, self.accent);
        visuals.text_cursor.stroke = Stroke::new(2.0_f32, self.accent);
        let r = CornerRadius::same(10);
        let w = &mut visuals.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0_f32, alpha(self.fg, 0.18));
        w.noninteractive.fg_stroke = Stroke::new(1.0_f32, self.fg);
        w.inactive.weak_bg_fill = alpha(self.fg, 0.08);
        w.inactive.bg_fill = alpha(self.fg, 0.10);
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0_f32, self.fg);
        w.hovered.weak_bg_fill = alpha(self.fg, 0.18);
        w.hovered.bg_fill = alpha(self.fg, 0.18);
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.0_f32, self.fg);
        w.active.weak_bg_fill = alpha(self.accent, 0.40);
        w.active.bg_fill = alpha(self.accent, 0.40);
        w.active.bg_stroke = Stroke::NONE;
        w.open.weak_bg_fill = alpha(self.fg, 0.14);
        for v in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
            v.corner_radius = r;
            v.expansion = 0.0;
        }
        ctx.set_visuals(visuals);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
            s.spacing.button_padding = egui::vec2(10.0, 5.0);
            s.spacing.interact_size.y = 30.0;
            s.animation_time = 0.18;
            use egui::{FontFamily, FontId, TextStyle};
            s.text_styles = [
                (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
            ]
            .into();
        });
    }
}

/// Windows: frosted-glass window (acrylic, like Hyprland's blur behind the Linux widget) with
/// rounded corners. Returns true when the glass is on (the card can then be more transparent).
pub fn glass(cc: &eframe::CreationContext<'_>, theme: &Theme) -> bool {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = cc.window_handle() else { return false };
        if let RawWindowHandle::Win32(h) = handle.as_raw() {
            use windows_sys::Win32::Graphics::Dwm::{
                DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
            };
            let pref = DWMWCP_ROUND;
            unsafe {
                DwmSetWindowAttribute(
                    h.hwnd.get() as _,
                    DWMWA_WINDOW_CORNER_PREFERENCE as _,
                    (&pref as *const i32).cast(),
                    4,
                );
            }
        }
        let tint = (theme.bg.r(), theme.bg.g(), theme.bg.b(), if theme.dark { 110 } else { 90 });
        window_vibrancy::apply_acrylic(&handle, Some(tint)).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = (cc, theme);
        false
    }
}

/// The card behind everything: translucent like `.card` on Linux (alpha 0.35 over the blur),
/// more opaque when there is no glass behind the window.
pub fn card(ui: &mut egui::Ui, theme: &Theme, glass: bool) {
    let rect = ui.max_rect();
    let (fill, radius) = if glass { (alpha(theme.bg, 0.35), 0) } else { (alpha(theme.bg, 0.92), 18) };
    ui.painter().rect(
        rect,
        CornerRadius::same(radius),
        fill,
        Stroke::new(1.0_f32, alpha(theme.fg, 0.22)),
        egui::StrokeKind::Inside,
    );
}

/// The little title: icon + lowercase monospace name in the accent color, as on Linux.
pub fn title(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(egui::RichText::new(format!("👥  {text}")).monospace().strong().size(14.0).color(theme.accent));
}
