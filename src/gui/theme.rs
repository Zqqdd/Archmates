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
        visuals.extreme_bg_color = alpha(self.bg, 0.45);
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
        // Also the border of text fields (focused: the accent, like Noctalia's input).
        w.inactive.bg_stroke = Stroke::new(1.0_f32, alpha(self.fg, 0.25));
        w.inactive.fg_stroke = Stroke::new(1.0_f32, self.fg);
        w.hovered.weak_bg_fill = alpha(self.fg, 0.18);
        w.hovered.bg_fill = alpha(self.fg, 0.18);
        w.hovered.bg_stroke = Stroke::new(1.0_f32, alpha(self.fg, 0.40));
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

/// The Noctalia panel's icons (Tabler icon font subset, see scripts/make-icons.py).
#[path = "icons.rs"]
pub mod icons;

/// Font family of the icons.
pub fn icon_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("icons".into()))
}

/// Bold text (names, titles), in the system font like Noctalia.
pub fn bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("bold".into()))
}

/// Bold monospace (the widget title).
pub fn mono_bold(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name("mono-bold".into()))
}

/// The desktop's own fonts, as Noctalia draws with them: "sans-serif" from fontconfig on
/// Linux (Noto Sans…), Segoe UI on Windows; and the bold monospace of the widget title.
fn system_font(kind: &str) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        let dir = std::path::PathBuf::from(std::env::var_os("WINDIR")?).join("Fonts");
        let files: &[&str] = match kind {
            "regular" => &["segoeui.ttf"],
            "bold" => &["seguisb.ttf", "segoeuib.ttf"],
            _ => &["consolab.ttf"],
        };
        files.iter().find_map(|f| std::fs::read(dir.join(f)).ok())
    }
    #[cfg(not(windows))]
    {
        let pattern = match kind {
            "regular" => "sans-serif",
            "bold" => "sans-serif:semibold",
            _ => "DejaVu Sans Mono:bold",
        };
        let out = std::process::Command::new("fc-match").args(["-f", "%{file}", pattern]).output().ok()?;
        let path = String::from_utf8(out.stdout).ok()?;
        // Collections (.ttc) and variable fonts may not draw well: only plain font files.
        if !(path.ends_with(".ttf") || path.ends_with(".otf")) {
            return None;
        }
        std::fs::read(path).ok()
    }
}

/// Adds the system fonts and the icon font (also as a fallback, so icon characters show in
/// any text). Without system fonts, egui's own are used (bold then falls back to regular).
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontFamily};
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "archmates-icons".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/icons/archmates-icons.ttf"))),
    );
    let mut add = |name: &str, kind: &str| {
        let data = system_font(kind)?;
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(data)));
        Some(name.to_string())
    };
    let regular = add("system", "regular");
    let bold = add("system-bold", "bold");
    let mono_bold = add("system-mono-bold", "mono");
    let proportional = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let monospace = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();
    if let Some(r) = &regular {
        fonts.families.entry(FontFamily::Proportional).or_default().insert(0, r.clone());
    }
    let with =
        |first: Option<String>, rest: &[String]| first.into_iter().chain(rest.iter().cloned()).collect::<Vec<_>>();
    let base = fonts.families[&FontFamily::Proportional].clone();
    fonts.families.insert(FontFamily::Name("bold".into()), with(bold, &base));
    fonts.families.insert(FontFamily::Name("mono-bold".into()), with(mono_bold, &monospace));
    let _ = proportional;
    fonts.families.insert(FontFamily::Name("icons".into()), vec!["archmates-icons".into()]);
    for family in fonts.families.values_mut() {
        if !family.iter().any(|f| f == "archmates-icons") {
            family.push("archmates-icons".into());
        }
    }
    ctx.set_fonts(fonts);
}

/// Windows: frosted-glass window (acrylic, like Hyprland's blur behind the Linux widget) with
/// rounded corners. Returns true when the glass is on (the card can then be more transparent).
pub fn glass(cc: &eframe::CreationContext<'_>, theme: &Theme) -> bool {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(handle) = cc.window_handle() else { return false };
        match handle.as_raw() {
            RawWindowHandle::Win32(h) => glass_hwnd(h.hwnd.get(), theme, None),
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (cc, theme);
        false
    }
}

/// The same glass on any window of this program, by its handle (used for the friend panel,
/// whose handle eframe does not give out: it is found by its title).
#[cfg(windows)]
pub fn glass_hwnd(hwnd: isize, theme: &Theme, luminosity: Option<Color32>) -> bool {
    use raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };
    struct Hwnd(std::num::NonZeroIsize);
    impl HasWindowHandle for Hwnd {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            // Safety: the window outlives this call (it is one of ours, on screen).
            Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(self.0))) })
        }
    }
    let Some(handle) = std::num::NonZeroIsize::new(hwnd) else { return false };
    let pref = DWMWCP_ROUND;
    // Windows draws its border over the window's edge: it gets the card's rim color (Noctalia's
    // on_surface_variant/0.55), so the rim follows Windows' rounded corners.
    let rim = mix(theme.bg, theme.fg, 0.42);
    let rim = rim.r() as u32 | (rim.g() as u32) << 8 | (rim.b() as u32) << 16;
    unsafe {
        DwmSetWindowAttribute(hwnd as _, DWMWA_WINDOW_CORNER_PREFERENCE as _, (&pref as *const i32).cast(), 4);
        DwmSetWindowAttribute(hwnd as _, DWMWA_BORDER_COLOR as _, (&rim as *const u32).cast(), 4);
    }
    // The "accent" acrylic, not the Windows 11 system backdrop (window_vibrancy's acrylic there):
    // that one turns to a flat gray whenever the window is not the active one. (The plain accent
    // blur draws black.) Its tint color, even nearly transparent, sets how bright the acrylic
    // is: `luminosity`, the wallpaper's average color, makes it about as bright as what is
    // behind, like the widget's blur; the card then tints both the same way.
    // A little lighter: the acrylic comes out a bit darker than the color it is given.
    let base = luminosity.map_or(theme.bg, |c| mix(c, Color32::WHITE, 0.18));
    let tint = (base.r(), base.g(), base.b(), 1);
    accent(hwnd, ACCENT_ACRYLIC, tint) || window_vibrancy::apply_acrylic(Hwnd(handle), Some(tint)).is_ok()
}

#[cfg(windows)]
const ACCENT_ACRYLIC: u32 = 4;

/// SetWindowCompositionAttribute (undocumented, user32): the "accent" glass of Windows 10,
/// still there on 11, and unlike the system backdrop it stays on when the window is inactive.
#[cfg(windows)]
fn accent(hwnd: isize, state: u32, tint: (u8, u8, u8, u8)) -> bool {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
    #[repr(C)]
    struct Policy {
        state: u32,
        flags: u32,
        color: u32,
        animation: u32,
    }
    #[repr(C)]
    struct Data {
        attrib: u32,
        data: *mut std::ffi::c_void,
        size: usize,
    }
    type Swca = unsafe extern "system" fn(isize, *mut Data) -> i32;
    unsafe {
        let user32 = LoadLibraryA(c"user32.dll".as_ptr().cast());
        if user32.is_null() {
            return false;
        }
        let Some(f) = GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr().cast()) else {
            return false;
        };
        let swca: Swca = std::mem::transmute(f);
        let mut policy = Policy {
            state,
            flags: 2,
            color: tint.0 as u32 | (tint.1 as u32) << 8 | (tint.2 as u32) << 16 | (tint.3.max(1) as u32) << 24,
            animation: 0,
        };
        let mut data = Data { attrib: 0x13, data: (&mut policy as *mut Policy).cast(), size: size_of::<Policy>() };
        swca(hwnd, &mut data) != 0
    }
}

/// The card behind everything, like the Noctalia glass (`surface/0.18` and a thin rim): a tint
/// over the blur, a soft light from the top edge, more opaque when there is no glass behind.
/// `radius`: the window's own corners when it has glass (0 if Windows rounds it), else 18.
pub fn card(ui: &mut egui::Ui, theme: &Theme, glass: bool, radius: u8) {
    let rect = ui.max_rect();
    // Noctalia's glass: surface/0.18 over the blur.
    // The same over the widget's own blur and over Windows' acrylic (left untinted for it).
    let fill = if glass { alpha(theme.bg, 0.22) } else { alpha(theme.bg, 0.94) };
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(radius), fill);
    if glass {
        painter.add(top_light(rect, radius as f32));
    }
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        // Noctalia's rim: on_surface_variant/0.55.
        Stroke::new(1.0_f32, alpha(theme.fg, 0.42)),
        egui::StrokeKind::Inside,
    );
}

/// The card's light: a white gradient from the top edge, fading out at 45 % of the height,
/// in the card's rounded shape (a square one would light up the transparent corners).
fn top_light(rect: egui::Rect, radius: f32) -> egui::Mesh {
    let height = rect.height() * 0.45;
    let r = radius.min(height).min(rect.width() / 2.0);
    let bottom = rect.top() + height;
    let light = |y: f32| alpha(Color32::WHITE, 0.06 * (1.0 - (y - rect.top()) / height).clamp(0.0, 1.0));
    // The outline, left to right over the top: a convex shape, drawn as a fan from its middle.
    let mut outline = vec![egui::pos2(rect.left(), bottom)];
    let corner = |center: egui::Pos2, from: f32, outline: &mut Vec<egui::Pos2>| {
        for i in 0..=8 {
            let a = from + i as f32 / 8.0 * std::f32::consts::FRAC_PI_2;
            outline.push(center + r * egui::vec2(a.cos(), a.sin()));
        }
    };
    corner(egui::pos2(rect.left() + r, rect.top() + r), std::f32::consts::PI, &mut outline);
    corner(egui::pos2(rect.right() - r, rect.top() + r), -std::f32::consts::FRAC_PI_2, &mut outline);
    outline.push(egui::pos2(rect.right(), bottom));
    let mut mesh = egui::Mesh::default();
    let middle = egui::pos2(rect.center().x, rect.top() + height / 2.0);
    mesh.colored_vertex(middle, light(middle.y));
    for p in &outline {
        mesh.colored_vertex(*p, light(p.y));
    }
    let n = outline.len() as u32;
    for i in 1..n {
        mesh.add_triangle(0, i, i + 1);
    }
    // Closes the fan along the bottom edge.
    mesh.add_triangle(0, n, 1);
    mesh
}

/// The little title, as on the Noctalia widget: users icon + name in monospace, accent color.
pub fn title(ui: &mut egui::Ui, theme: &Theme, text: &str) {
    ui.label(egui::RichText::new(icons::USERS.to_string()).font(icon_font(15.0)).color(theme.accent));
    ui.label(egui::RichText::new(text).font(mono_bold(14.0)).color(theme.accent));
}

/// How an icon button looks, after the Noctalia panel's buttons.
#[derive(Clone, Copy, PartialEq)]
pub enum Look {
    /// No background until hovered (a neutral one: Noctalia's "ghost" turns green in some themes).
    Quiet,
    /// Filled with the accent color (send).
    Primary,
    /// A round accent button (play / stop a voice message).
    Round,
}

/// An icon button from the Noctalia icon set.
pub fn icon_button(ui: &mut egui::Ui, theme: &Theme, icon: char, hint: &str, look: Look) -> egui::Response {
    let size = if look == Look::Round { egui::Vec2::splat(28.0) } else { egui::Vec2::splat(32.0) };
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
    let painter = ui.painter();
    let (color, glyph) = match look {
        Look::Quiet => {
            if hover > 0.0 {
                painter.rect_filled(rect, 8, alpha(theme.fg, 0.10 * hover));
            }
            (theme.fg, 16.0)
        }
        Look::Primary => {
            painter.rect_filled(rect, 8, mix(theme.accent, theme.fg, 0.12 * hover));
            (theme.on_accent(), 15.0)
        }
        Look::Round => {
            painter.circle_filled(rect.center(), 14.0, mix(theme.accent, theme.fg, 0.12 * hover));
            (theme.on_accent(), 13.0)
        }
    };
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, icon, icon_font(glyph), color);
    response.on_hover_text(hint).on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A quiet button with an icon and a label ("Copy my code", "Decline"…), like Noctalia's.
pub fn quiet_button(ui: &mut egui::Ui, theme: &Theme, icon: Option<char>, text: &str) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, theme.fg);
    let icon_w = if icon.is_some() { 14.0 + 6.0 } else { 0.0 };
    let size = egui::vec2(galley.size().x + icon_w + 20.0, 30.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
    let painter = ui.painter();
    if hover > 0.0 {
        painter.rect_filled(rect, 8, alpha(theme.fg, 0.10 * hover));
    }
    let mut x = rect.left() + 10.0;
    if let Some(icon) = icon {
        painter.text(
            egui::pos2(x + 7.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icon,
            icon_font(14.0),
            theme.fg,
        );
        x += icon_w;
    }
    painter.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, theme.fg);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A filled accent button with a label ("Accept", "Add"), like Noctalia's primary buttons.
pub fn primary_button(ui: &mut egui::Ui, theme: &Theme, text: &str) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, theme.on_accent());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 24.0, 30.0), egui::Sense::click());
    let hover = ui.ctx().animate_bool_with_time(response.id, response.hovered(), 0.12);
    ui.painter().rect_filled(rect, 8, mix(theme.accent, theme.fg, 0.12 * hover));
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, theme.on_accent());
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
