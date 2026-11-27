//! Pictures shown in the chat: a sharp copy, and a blurred one shown while the window is not
//! the active one.

use std::path::Path;

use eframe::egui::{self, Vec2, vec2};

/// A picture shown in the chat, sharp and blurred.
pub struct Picture {
    pub sharp: egui::TextureHandle,
    pub blurred: egui::TextureHandle,
    pub size: Vec2,
}

pub const IMAGE_EXT: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

pub fn is_image(path: &str) -> bool {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    IMAGE_EXT.contains(&ext.as_str()) && Path::new(path).exists()
}

/// Loads a chat picture, shrunk to fit 240×170, plus a heavily blurred copy.
pub fn load_picture(ctx: &egui::Context, path: &str) -> Option<Picture> {
    let img = image::open(path).ok()?;
    let small = img.resize(480, 340, image::imageops::FilterType::Triangle).to_rgba8();
    let (w, h) = small.dimensions();
    // Blur a tiny copy: cheap, and once stretched back it reveals nothing.
    let tiny = image::imageops::resize(&small, (w / 12).max(1), (h / 12).max(1), image::imageops::FilterType::Triangle);
    let tiny = image::imageops::blur(&tiny, 2.5);
    let sharp = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], small.as_raw());
    let (tw, th) = tiny.dimensions();
    let blurred = egui::ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], tiny.as_raw());
    let scale = (240.0 / w as f32).min(170.0 / h as f32).min(1.0) * 2.0;
    Some(Picture {
        sharp: ctx.load_texture(format!("pic:{path}"), sharp, egui::TextureOptions::LINEAR),
        blurred: ctx.load_texture(format!("pic-blur:{path}"), blurred, egui::TextureOptions::LINEAR),
        size: vec2(w as f32 * scale / 2.0, h as f32 * scale / 2.0),
    })
}
