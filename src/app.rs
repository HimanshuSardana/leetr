//! The `eframe` application: top-level state and panel layout.

use eframe::egui;

use crate::config::{self, Config, Credentials};
use crate::theme::Theme;
use crate::ui;

pub struct LeetrApp {
    pub config: Config,
    pub credentials: Credentials,
    pub status: String,
    pub error: Option<String>,
}

impl LeetrApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut error = None;
        if let Err(err) = config::seed() {
            error = Some(format!("could not write config files: {err}"));
        }

        let config = Config::load().unwrap_or_else(|err| {
            error = Some(format!("{err}"));
            Config::default()
        });
        let credentials = Credentials::load();

        // Restore the persisted colour scheme before the first frame.
        config.theme.apply(&cc.egui_ctx);

        let status = if credentials.is_configured() {
            "Ready — credentials loaded".to_string()
        } else {
            "Ready — no credentials (browse-only mode)".to_string()
        };

        Self {
            config,
            credentials,
            status,
            error,
        }
    }

    /// Switch colour scheme, persist the choice and report it in the status bar.
    pub fn set_theme(&mut self, theme: Theme, ctx: &egui::Context) {
        self.config.theme = theme;
        theme.apply(ctx);
        match self.config.save() {
            Ok(()) => {
                self.error = None;
                self.status = format!("{} mode enabled", theme.label());
            }
            Err(err) => self.error = Some(format!("could not save config: {err}")),
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let theme = self.config.theme;
        let mut requested_theme = None;

        ui.horizontal(|ui| {
            ui.heading("Leetr");
            ui.separator();
            ui.label(
                egui::RichText::new("lightweight LeetCode client")
                    .weak()
                    .small(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).weak());
                ui.separator();

                let toggle = ui
                    .button(format!("{} {}", theme.icon(), theme.label()))
                    .on_hover_text(format!("Switch to {} mode", theme.toggled().label()));
                if toggle.clicked() {
                    requested_theme = Some(theme.toggled());
                }
            });
        });

        if let Some(next) = requested_theme {
            let ctx = ui.ctx().clone();
            self.set_theme(next, &ctx);
        }
    }

    fn problem_panel(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Problems");
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Problem list lands in the next feature.").weak());
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.strong("Statement");
        ui.separator();
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Problem statements land in the next feature.").weak());
    }
}

impl eframe::App for LeetrApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("leetr-toolbar").show(ui, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });

        egui::Panel::bottom("leetr-status").show(ui, |ui| {
            ui::status_bar::show(
                ui,
                &self.config,
                &self.credentials,
                &self.status,
                self.error.as_deref(),
            );
        });

        egui::Panel::left("leetr-problems")
            .resizable(true)
            .default_size(320.0)
            .size_range(egui::Rangef::new(220.0, 520.0))
            .show(ui, |ui| {
                self.problem_panel(ui);
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            self.central(ui);
        });
    }
}

/// Launch the GUI. Blocks until the window is closed.
pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Leetr")
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Leetr",
        options,
        Box::new(|cc| Ok(Box::new(LeetrApp::new(cc)))),
    )
    .map_err(|err| anyhow::anyhow!("failed to start the window: {err}"))
}
