//! WinmatesSetup.exe: installs Winmates for your account (no admin rights).
//!
//! The three programs are embedded in the installer at build time:
//!   WINMATES_PAYLOAD=target/release cargo build --release --features setup --bin winmates-setup
//!
//! It copies them to %LOCALAPPDATA%\Programs\Winmates, adds the Start menu (and optional
//! desktop) shortcut, registers WinmatesUpdater to start with Windows, adds Winmates to
//! Settings → Apps (to uninstall it), then starts it. Running it again repairs / updates.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "../gui/theme.rs"]
mod theme;

use eframe::egui::{self, RichText};
use theme::{Theme, alpha};

const PROGRAMS: [(&str, &[u8]); 3] = [
    ("winmates.exe", include_bytes!(concat!(env!("WINMATES_PAYLOAD"), "/winmates.exe"))),
    ("winmates-updater.exe", include_bytes!(concat!(env!("WINMATES_PAYLOAD"), "/winmates-updater.exe"))),
    ("archmates.exe", include_bytes!(concat!(env!("WINMATES_PAYLOAD"), "/archmates.exe"))),
];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Winmates setup")
            .with_inner_size([440.0, 330.0])
            .with_resizable(false)
            .with_decorations(false)
            .with_transparent(true),
        ..Default::default()
    };
    eframe::run_native("Winmates setup", options, Box::new(|cc| Ok(Box::new(Setup::new(cc)))))
}

#[derive(PartialEq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Step {
    Welcome,
    Done,
    Failed(String),
}

struct Setup {
    theme: Theme,
    glass: bool,
    step: Step,
    autostart: bool,
    desktop: bool,
    shown_at: f64,
}

impl Setup {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let theme = Theme::detect();
        theme::install_fonts(&cc.egui_ctx);
        theme.apply(&cc.egui_ctx);
        let glass = theme::glass(cc, &theme);
        Self { theme, glass, step: Step::Welcome, autostart: true, desktop: false, shown_at: -1.0 }
    }

    fn install(&mut self) {
        #[cfg(windows)]
        {
            self.step = match archmates::platform::windows::install::install(&PROGRAMS, self.desktop, self.autostart) {
                Ok(()) => {
                    // Start it the way Windows will at every login: update check, service, window.
                    let _ = std::process::Command::new(archmates::platform::windows::install::updater()).spawn();
                    Step::Done
                }
                Err(e) => Step::Failed(format!("{e:#}")),
            };
        }
        #[cfg(not(windows))]
        {
            let _ = PROGRAMS;
            self.step = Step::Failed("WinmatesSetup only runs on Windows.".into());
        }
    }
}

impl eframe::App for Setup {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = ctx.input(|i| i.time);
        if self.shown_at < 0.0 {
            self.shown_at = now;
        }
        // Fade + rise in when it opens.
        let t = ((now - self.shown_at) / 0.35).clamp(0.0, 1.0) as f32;
        let ease = 1.0 - (1.0 - t).powi(3);
        if t < 1.0 {
            ctx.request_repaint();
        }
        let theme = self.theme;
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            theme::card(ui, &theme, self.glass);
            // Drag the window by its background.
            let bg = ui.interact(ui.max_rect(), ui.id().with("drag"), egui::Sense::drag());
            if bg.drag_started() {
                ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.set_opacity(ease);
            egui::Frame::NONE.inner_margin(egui::Margin { left: 24, right: 24, top: 20 + ((1.0 - ease) * 12.0) as i8, bottom: 20 }).show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::title(ui, &theme, "winmates");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button(ui, &theme, theme::icons::X, "Close", theme::Look::Quiet).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                });
                ui.add_space(14.0);
                match &self.step {
                    Step::Welcome => {
                        ui.label(RichText::new("Your friends, right on your desktop.").heading().strong());
                        ui.label(
                            RichText::new("Peer-to-peer and encrypted: see who is online, chat, send files and voice messages, with friends on Windows and on Linux.")
                                .color(theme.muted()),
                        );
                        ui.add_space(12.0);
                        ui.checkbox(&mut self.autostart, "Start with Windows (and update by itself)");
                        ui.checkbox(&mut self.desktop, "Add a desktop shortcut");
                        ui.add_space(4.0);
                        ui.label(RichText::new(format!("Installs version {} for your account only, no admin rights.", archmates::update::CURRENT)).small().color(theme.muted()));
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                            let install = egui::Button::new(RichText::new("Install").strong().color(theme.on_accent()))
                                .fill(theme.accent)
                                .min_size(egui::vec2(120.0, 34.0));
                            if ui.add(install).clicked() {
                                self.install();
                            }
                        });
                    }
                    Step::Done => {
                        ui.label(RichText::new("✔ Winmates is installed").heading().strong().color(theme.accent));
                        ui.label(RichText::new("It is starting now. Find it in the Start menu; it updates itself each time it starts.").color(theme.muted()));
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                            if ui.add(egui::Button::new("Close").min_size(egui::vec2(100.0, 32.0))).clicked() {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        });
                    }
                    Step::Failed(e) => {
                        ui.label(RichText::new("Installation failed").heading().strong().color(theme::DANGER));
                        ui.label(RichText::new(e.as_str()).color(alpha(theme.fg, 0.8)));
                        ui.with_layout(egui::Layout::bottom_up(egui::Align::Max), |ui| {
                            if ui.button("Try again").clicked() {
                                self.step = Step::Welcome;
                            }
                        });
                    }
                }
            });
        });
    }
}
