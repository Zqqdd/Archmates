//! The widget's glass on Windows: your wallpaper, blurred, drawn under it, like Hyprland's
//! blur under the Noctalia widget.
//!
//! Windows' own glass can't do it: its acrylic turns gray whenever the window is not the active
//! one (the widget never is), and its corners are 8 px at most (Noctalia's glass: 18). The
//! widget sits on the desktop, so what is behind it is the wallpaper: it is laid out as Windows
//! shows it on the widget's monitor (fill, fit, stretch, center, tile), blurred once, and the
//! part under the widget is drawn in its rounded card.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;

/// The wallpaper is laid out at a quarter of the monitor's size: it is blurred anyway.
const SCALE: u32 = 4;
/// Blur radius at that size (×4 on screen).
const SIGMA: f32 = 6.0;

pub struct Backdrop {
    texture: egui::TextureHandle,
    /// The monitor it was laid out for, in physical pixels: left, top, width, height.
    monitor: [i32; 4],
    /// Wallpaper file, its date, how it is shown, and the monitor: rebuilt when one changes.
    key: String,
    checked: Instant,
    /// Where the window was when last drawn.
    drawn_at: std::cell::Cell<[i32; 4]>,
    /// The wallpaper's average color: how bright the chat's acrylic is (theme::glass_hwnd).
    pub average: egui::Color32,
}

/// What decides the backdrop: wallpaper file and date, its layout, the monitor.
struct Source {
    path: PathBuf,
    style: Style,
    background: [u8; 3],
    monitor: [i32; 4],
    key: String,
}

#[derive(Clone, Copy, Debug)]
enum Style {
    Fill,
    Fit,
    Stretch,
    Center,
    Tile,
}

impl Backdrop {
    /// None when there is no wallpaper image (a plain color, Spotlight…) or off Windows.
    pub fn load(ctx: &egui::Context, hwnd: isize) -> Option<Self> {
        let source = source(hwnd)?;
        let image = layout(&source)?;
        let image = image::imageops::fast_blur(&image, SIGMA);
        let size = [image.width() as usize, image.height() as usize];
        let mut sum = [0u64; 3];
        for p in image.pixels() {
            for (s, v) in sum.iter_mut().zip(p.0) {
                *s += v as u64;
            }
        }
        let n = (image.pixels().len() as u64).max(1);
        let average = egui::Color32::from_rgb((sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8);
        let color = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        Some(Self {
            average,
            texture: ctx.load_texture("wallpaper", color, egui::TextureOptions::LINEAR),
            monitor: source.monitor,
            key: source.key,
            checked: Instant::now(),
            drawn_at: Default::default(),
        })
    }

    /// Every second: a new wallpaper, or the widget moved to another monitor?
    pub fn outdated(&mut self, hwnd: isize) -> bool {
        if self.checked.elapsed() < Duration::from_secs(1) {
            return false;
        }
        self.checked = Instant::now();
        source(hwnd).is_none_or(|s| s.key != self.key)
    }

    /// Draws the part of the wallpaper under the window, in `rect` (the whole window).
    pub fn paint(&self, ui: &egui::Ui, rect: egui::Rect, hwnd: isize, radius: f32) {
        let Some(window) = window_rect(hwnd) else { return };
        // Being dragged: follow the window frame by frame.
        if self.drawn_at.replace(window) != window {
            ui.ctx().request_repaint();
        }
        let [mx, my, mw, mh] = self.monitor.map(|v| v as f32);
        let [wx, wy, ww, wh] = window.map(|v| v as f32);
        let uv = egui::Rect::from_min_max(
            egui::pos2((wx - mx) / mw, (wy - my) / mh),
            egui::pos2((wx + ww - mx) / mw, (wy + wh - my) / mh),
        );
        egui::Image::new((self.texture.id(), rect.size())).uv(uv).corner_radius(radius).paint_at(ui, rect);
    }
}

/// The wallpaper as Windows shows it on that monitor, at 1/SCALE.
fn layout(source: &Source) -> Option<image::RgbaImage> {
    use image::imageops::{self, FilterType};
    let wallpaper = image::ImageReader::open(&source.path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let (w, h) = ((source.monitor[2] as u32 / SCALE).max(1), (source.monitor[3] as u32 / SCALE).max(1));
    let [r, g, b] = source.background;
    let mut canvas = image::RgbaImage::from_pixel(w, h, image::Rgba([r, g, b, 255]));
    let filter = FilterType::Triangle;
    match source.style {
        Style::Fill => return Some(wallpaper.resize_to_fill(w, h, filter).to_rgba8()),
        Style::Stretch => return Some(wallpaper.resize_exact(w, h, filter).to_rgba8()),
        Style::Fit => {
            let fitted = wallpaper.resize(w, h, filter).to_rgba8();
            let (x, y) = ((w - fitted.width()) / 2, (h - fitted.height()) / 2);
            imageops::overlay(&mut canvas, &fitted, x as i64, y as i64);
        }
        Style::Center | Style::Tile => {
            // Shown at its own size: a quarter of it here.
            let (iw, ih) = ((wallpaper.width() / SCALE).max(1), (wallpaper.height() / SCALE).max(1));
            let small = wallpaper.resize_exact(iw, ih, filter).to_rgba8();
            if let Style::Center = source.style {
                imageops::overlay(&mut canvas, &small, (w as i64 - iw as i64) / 2, (h as i64 - ih as i64) / 2);
            } else {
                for y in (0..h).step_by(ih as usize) {
                    for x in (0..w).step_by(iw as usize) {
                        imageops::overlay(&mut canvas, &small, x as i64, y as i64);
                    }
                }
            }
        }
    }
    Some(canvas)
}

#[cfg(windows)]
fn source(hwnd: isize) -> Option<Source> {
    use archmates::platform::reg;
    let desktop = r"Control Panel\Desktop";
    // The image Windows actually shows (a copy it keeps), else the file you picked.
    let transcoded = std::env::var_os("APPDATA")
        .map(|d| PathBuf::from(d).join(r"Microsoft\Windows\Themes\TranscodedWallpaper"))
        .filter(|p| p.is_file());
    let picked = reg::get_string(reg::CURRENT_USER, desktop, "WallPaper")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_file());
    let path = transcoded.or(picked)?;
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
    let tile = reg::get_string(reg::CURRENT_USER, desktop, "TileWallpaper").unwrap_or_default();
    let style = match reg::get_string(reg::CURRENT_USER, desktop, "WallpaperStyle").unwrap_or_default().trim() {
        "6" => Style::Fit,
        "2" => Style::Stretch,
        "0" if tile.trim() == "1" => Style::Tile,
        "0" => Style::Center,
        // 10 (fill), 22 (span), unset.
        _ => Style::Fill,
    };
    let background = reg::get_string(reg::CURRENT_USER, r"Control Panel\Colors", "Background")
        .and_then(|s| {
            let v: Vec<u8> = s.split_whitespace().filter_map(|n| n.parse().ok()).collect();
            (v.len() == 3).then(|| [v[0], v[1], v[2]])
        })
        .unwrap_or([0, 0, 0]);
    let monitor = monitor_rect(hwnd)?;
    let key = format!("{}|{modified:?}|{style:?}|{background:?}|{monitor:?}", path.display());
    Some(Source { path, style, background, monitor, key })
}

#[cfg(not(windows))]
fn source(_hwnd: isize) -> Option<Source> {
    None
}

/// The window, in physical pixels: left, top, width, height.
#[cfg(windows)]
fn window_rect(hwnd: isize) -> Option<[i32; 4]> {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    (unsafe { GetWindowRect(hwnd as _, &mut r) } != 0).then(|| [r.left, r.top, r.right - r.left, r.bottom - r.top])
}

#[cfg(not(windows))]
fn window_rect(_hwnd: isize) -> Option<[i32; 4]> {
    None
}

/// The monitor the window is on, in physical pixels: left, top, width, height.
#[cfg(windows)]
fn monitor_rect(hwnd: isize) -> Option<[i32; 4]> {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
    };
    unsafe {
        let monitor = MonitorFromWindow(hwnd as _, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let r = info.rcMonitor;
        Some([r.left, r.top, r.right - r.left, r.bottom - r.top])
    }
}
