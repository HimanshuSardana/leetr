//! Bottom status bar: auth badge + transient status message.

use crate::config::Config;
use crate::glyphs;
use crate::theme;

/// What to show for the LeetCode session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthBadge {
    /// Credentials are configured and LeetCode accepted them.
    SignedIn { username: String, solved: usize },
    /// Either no credentials, or they have not been verified yet.
    #[default]
    BrowseOnly,
}

pub fn show(
    ui: &mut egui::Ui,
    config: &Config,
    auth: &AuthBadge,
    status: &str,
    error: Option<&str>,
) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        match auth {
            AuthBadge::SignedIn { username, solved } => {
                ui.label(
                    egui::RichText::new(format!("{} signed in", glyphs::DOT)).color(theme::SUCCESS),
                );
                if !username.is_empty() {
                    ui.label(egui::RichText::new(username).weak().small());
                }
                ui.label(
                    egui::RichText::new(format!("{solved} solved"))
                        .weak()
                        .small(),
                );
            }
            AuthBadge::BrowseOnly => {
                ui.label(
                    egui::RichText::new(format!("{} browse-only", glyphs::DOT))
                        .color(theme::WARNING),
                )
                .on_hover_text(
                    "No credentials found. Add them to ~/.config/leetr/credentials.toml \
                         to run tests and submit solutions.",
                );
            }
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
