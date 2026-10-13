//! Bottom status bar: auth badge + transient status message.

use crate::config::{Config, Credentials};
use crate::theme;

pub fn show(
    ui: &mut egui::Ui,
    config: &Config,
    credentials: &Credentials,
    status: &str,
    error: Option<&str>,
) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if credentials.is_configured() {
            ui.label(egui::RichText::new("● signed in").color(theme::SUCCESS));
        } else {
            ui.label(egui::RichText::new("● browse-only").color(theme::WARNING))
                .on_hover_text(
                    "No credentials found. Add them to ~/.config/leetr/credentials.toml \
                     to run tests and submit solutions.",
                );
        }

        ui.separator();

        if let Some(err) = error {
            ui.label(egui::RichText::new(err).color(theme::DANGER));
        } else {
            ui.label(status);
        }

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} · lang {} · api :{}",
                    config.theme.label(),
                    config.default_language,
                    config.interop_port
                ))
                .weak()
                .small(),
            );
        });
    });
    ui.add_space(2.0);
}
