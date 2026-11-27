//! Settings, and the "delete conversation" confirmation.

use archmates::service::control as launcher;
use archmates::{chat, ipc, platform, stats, update};
use eframe::egui::{self, Color32, RichText};

use crate::APP_NAME;
use crate::app::{App, set_avatar};
use crate::pictures::IMAGE_EXT;
use crate::startup::check_for_update;
use crate::theme::{self, alpha};

impl App {
    // ── Settings ──

    pub(crate) fn settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let theme = self.theme;
        let section = |ui: &mut egui::Ui, title: &str| {
            ui.add_space(8.0);
            ui.label(RichText::new(title).small().strong().color(theme.accent));
        };
        egui::ScrollArea::vertical().id_salt("settings").show(ui, |ui| {
            section(ui, "STARTUP");
            let label = if cfg!(windows) { "Start with Windows (and update by itself)" } else { "Start the service when I log in (systemd)" };
            if ui.checkbox(&mut self.autostart, label).changed() {
                self.settings_status = match platform::set_autostart(self.autostart) {
                    Ok(()) => "✓ Saved".into(),
                    Err(e) => {
                        self.autostart = platform::autostart_enabled();
                        format!("✗ {e:#}")
                    }
                };
            }
            if cfg!(windows) && ui.checkbox(&mut self.open_at_login, format!("Open {APP_NAME} when I log in")).changed() {
                launcher::set_open_at_login(&self.paths, self.open_at_login);
            }

            section(ui, "PRIVACY");
            let mut invisible = self.paths.is_invisible();
            if ui.checkbox(&mut invisible, "Invisible mode (friends see you offline)").changed() {
                if invisible {
                    let _ = std::fs::create_dir_all(&self.paths.data);
                    let _ = std::fs::write(self.paths.invisible(), b"");
                } else {
                    let _ = std::fs::remove_file(self.paths.invisible());
                }
            }
            if ui.checkbox(&mut self.share_stats, "Share my RAM and storage usage").changed() {
                let _ = stats::set_sharing(&self.paths, self.share_stats);
            }
            ui.label(RichText::new("Pictures in the chat are blurred while the window is not active. Voice messages are deleted at every restart.").small().color(theme.muted()));

            section(ui, "PROFILE PICTURE");
            ui.horizontal(|ui| {
                if self.pill(ui, "Choose…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("Pictures", &IMAGE_EXT).pick_file() {
                        self.settings_status = match set_avatar(&self.paths, &path) {
                            Ok(()) => "✓ Picture set. Your friends get it within a minute.".into(),
                            Err(e) => format!("✗ {e:#}"),
                        };
                    }
                }
                if self.pill(ui, "Use my account picture").clicked() {
                    let _ = std::fs::remove_file(self.paths.avatar());
                    self.settings_status = "✓ Back to your account picture".into();
                }
            });

            section(ui, "FILES");
            let dir = archmates::transfer::download_dir().join(archmates::transfer::DOWNLOAD_FOLDER);
            if ui.link(RichText::new(dir.display().to_string()).small()).clicked() {
                let _ = std::fs::create_dir_all(&dir);
                platform::open_path(&dir);
            }

            section(ui, "VERSION");
            let (line, offer, checking) = {
                let b = self.background.lock().unwrap();
                (b.update.clone(), b.update_offer.clone(), b.checking)
            };
            ui.label(format!("{APP_NAME} {}", update::CURRENT));
            if !line.is_empty() {
                ui.label(RichText::new(line).small().color(theme.muted()));
            }
            ui.horizontal(|ui| {
                if checking {
                    ui.spinner();
                } else if self.pill(ui, "Check for updates").clicked() {
                    let b = self.background.clone();
                    let ctx = ctx.clone();
                    self.rt.spawn(async move {
                        check_for_update(&b).await;
                        ctx.request_repaint();
                    });
                }
                if let Some(latest) = offer {
                    if self.pill(ui, &format!("Install {latest}")).clicked() {
                        let b = self.background.clone();
                        let ctx = ctx.clone();
                        b.lock().unwrap().update = format!("Installing {latest}…");
                        self.rt.spawn(async move {
                            let text = match update::install(&latest).await {
                                Ok(()) => format!("✓ {latest} installed. Restart the app to use it."),
                                Err(e) => format!("✗ {e:#}"),
                            };
                            let mut g = b.lock().unwrap();
                            g.update = text;
                            g.update_offer = None;
                            ctx.request_repaint();
                        });
                    }
                }
            });
            if cfg!(windows) {
                ui.label(RichText::new("WinmatesUpdater checks for a new version at every login.").small().color(theme.muted()));
            }

            ui.add_space(14.0);
            let quit = egui::Button::new(RichText::new(format!("Quit {APP_NAME}")).color(theme::DANGER)).fill(alpha(theme::DANGER, 0.12));
            if ui.add(quit).on_hover_text("Stops the service too: your friends will see you offline").clicked() {
                self.rt.block_on(async {
                    let _ = ipc::request(&ipc::Command::Quit).await;
                });
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.label(RichText::new(format!("Closing the window keeps {APP_NAME} running in the background.")).small().color(theme.muted()));
            if !self.settings_status.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(&self.settings_status).small());
            }
        });
    }

    /// "Delete conversation?" confirmation.
    pub(crate) fn delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(code) = self.confirm_delete.clone() else { return };
        let nickname = self.friends().into_iter().find(|f| f.code == code).map(|f| f.nickname).unwrap_or_default();
        let theme = self.theme;
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::new("delete-chat")).show(ctx, |ui| {
            ui.set_width(300.0);
            ui.label(RichText::new(format!("Delete your conversation with {nickname}?")).strong());
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "Every message and voice message, and every file {nickname} sent you ({}), is deleted from this PC for good. Files you sent stay where they are.",
                    archmates::transfer::received_dir(&nickname).display()
                ))
                .small()
                .color(theme.muted()),
            );
            ui.add_space(4.0);
            ui.label(RichText::new("This cannot be undone.").small().color(theme::DANGER));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let delete = egui::Button::new(RichText::new("Delete").strong().color(Color32::WHITE)).fill(Color32::from_rgb(0xc4, 0x3c, 0x3c));
                    if ui.add(delete).clicked() {
                        let result = chat::delete_conversation(&self.paths, &code, &nickname);
                        *self.status.lock().unwrap() = match result {
                            Ok(d) => format!("✓ Conversation deleted ({} messages, {} files)", d.messages, d.files),
                            Err(e) => format!("✗ {e:#}"),
                        };
                        self.messages.clear();
                        self.pictures.clear();
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        });
        if close || modal.should_close() {
            self.confirm_delete = None;
        }
    }
}
