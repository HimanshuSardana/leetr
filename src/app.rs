//! The `eframe` application: top-level state, panel layout and background work.

use std::collections::BTreeSet;
use std::time::Duration;

use eframe::egui;

use crate::bg::Bg;
use crate::config::{self, Config, Credentials};
use crate::leetcode::client::ProblemQuery;
use crate::leetcode::html::{self, Block};
use crate::leetcode::LeetCodeClient;
use crate::model::{Problem, ProblemPage, QuestionDetail, UserStatus};
use crate::theme::Theme;
use crate::ui;
use crate::ui::problem_list::{ProblemListAction, ProblemListState};
use crate::ui::status_bar::AuthBadge;

/// Background jobs report `Result`s with `String` errors so they stay `Send`.
type Job<T> = Result<T, String>;

/// Messages posted by background jobs.
enum ApiEvent {
    /// A page of problems. `skip == 0` replaces the list, otherwise it appends.
    Problems {
        skip: u32,
        result: Job<ProblemPage>,
    },
    Detail {
        slug: String,
        result: Job<Box<QuestionDetail>>,
    },
    Session(Job<(UserStatus, BTreeSet<String>)>),
}

pub struct LeetrApp {
    pub config: Config,
    pub credentials: Credentials,
    pub status: String,
    pub error: Option<String>,

    client: LeetCodeClient,
    bg: Bg<ApiEvent>,

    problems: Vec<Problem>,
    total: u32,
    list_state: ProblemListState,
    list_loading: bool,
    list_error: Option<String>,

    selected: Option<String>,
    detail: Option<QuestionDetail>,
    detail_blocks: Vec<Block>,
    detail_loading: bool,
    detail_error: Option<String>,

    user: Option<UserStatus>,
    solved: BTreeSet<String>,
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

        let client = match LeetCodeClient::new(credentials.clone()) {
            Ok(client) => client,
            Err(err) => {
                error = Some(format!("could not build the HTTP client: {err}"));
                // `with_base_url` only fails if TLS setup fails, which cannot
                // happen with rustls defaults; fall back to an unusable client.
                LeetCodeClient::new(Credentials::default()).expect("default client")
            }
        };

        let mut app = Self {
            config,
            credentials,
            status: "Loading problems…".to_string(),
            error,
            client,
            bg: Bg::new(),
            problems: Vec::new(),
            total: 0,
            list_state: ProblemListState::default(),
            list_loading: false,
            list_error: None,
            selected: None,
            detail: None,
            detail_blocks: Vec::new(),
            detail_loading: false,
            detail_error: None,
            user: None,
            solved: BTreeSet::new(),
        };

        app.refresh_session();
        app.load_first_page();
        app
    }

    // -- background work ----------------------------------------------------

    fn refresh_session(&mut self) {
        if !self.client.is_authenticated() {
            return;
        }
        let client = self.client.clone();
        self.bg.spawn(move || {
            let result: Job<(UserStatus, BTreeSet<String>)> = (|| {
                let status = client.fetch_user_status().map_err(|e| e.to_string())?;
                let solved = client.fetch_solved_slugs().map_err(|e| e.to_string())?;
                Ok((status, solved))
            })();
            ApiEvent::Session(result)
        });
    }

    fn spawn_problems(&mut self, skip: u32, overwrite: bool) {
        if overwrite {
            self.problems.clear();
            self.total = 0;
        }
        self.list_loading = true;
        self.list_error = None;

        let client = self.client.clone();
        let limit = self.config.problem_list_page_size.clamp(1, 100);
        self.bg.spawn(move || {
            let result = client
                .fetch_problems(&ProblemQuery {
                    skip,
                    limit,
                    ..ProblemQuery::default()
                })
                .map_err(|e| e.to_string());
            ApiEvent::Problems { skip, result }
        });
    }

    fn load_first_page(&mut self) {
        self.spawn_problems(0, true);
    }

    fn load_next_page(&mut self) {
        if self.list_loading || self.problems.len() >= self.total as usize {
            return;
        }
        let skip = self.problems.len() as u32;
        self.list_loading = true;
        self.list_error = None;

        let client = self.client.clone();
        let limit = self.config.problem_list_page_size.clamp(1, 100);
        self.bg.spawn(move || {
            let result = client
                .fetch_problems(&ProblemQuery {
                    skip,
                    limit,
                    ..ProblemQuery::default()
                })
                .map_err(|e| e.to_string());
            ApiEvent::Problems { skip, result }
        });
    }

    /// Replace the list with a server-side keyword search.
    fn search_problems(&mut self, query: String) {
        self.problems.clear();
        self.total = 0;
        self.list_loading = true;
        self.list_error = None;
        self.status = format!("Searching LeetCode for “{query}”…");

        let client = self.client.clone();
        let limit = self.config.problem_list_page_size.clamp(1, 100);
        self.bg.spawn(move || {
            let result = client
                .fetch_problems(&ProblemQuery {
                    skip: 0,
                    limit,
                    status: None,
                    search: Some(query),
                })
                .map_err(|e| e.to_string());
            ApiEvent::Problems { skip: 0, result }
        });
    }

    fn open_problem(&mut self, slug: String) {
        self.selected = Some(slug.clone());
        self.detail = None;
        self.detail_blocks.clear();
        self.detail_error = None;
        self.detail_loading = true;
        self.status = format!("Loading {slug}…");

        let client = self.client.clone();
        self.bg.spawn(move || {
            let result = client
                .fetch_question(&slug)
                .map(Box::new)
                .map_err(|e| e.to_string());
            ApiEvent::Detail { slug, result }
        });
    }

    fn drain_events(&mut self) {
        for event in self.bg.drain() {
            match event {
                ApiEvent::Problems { skip, result } => {
                    self.list_loading = false;
                    match result {
                        Ok(page) => {
                            self.total = page.total;
                            if skip == 0 {
                                self.problems = page.problems;
                            } else {
                                self.problems.extend(page.problems);
                            }
                            self.list_error = None;
                            self.status = format!(
                                "{} problems loaded ({} total)",
                                self.problems.len(),
                                self.total
                            );
                        }
                        Err(err) => {
                            self.list_error = Some(err.clone());
                            self.status = "Could not load problems".to_string();
                        }
                    }
                }
                ApiEvent::Detail { slug, result } => {
                    // Ignore a response for a problem the user has moved on from.
                    if self.selected.as_deref() != Some(slug.as_str()) {
                        continue;
                    }
                    self.detail_loading = false;
                    match result {
                        Ok(detail) => {
                            self.detail_blocks = html::parse(&detail.content_html);
                            self.status = format!("{} · {}", detail.frontend_id, detail.title);
                            self.detail = Some(*detail);
                            self.detail_error = None;
                        }
                        Err(err) => {
                            self.detail_error = Some(err.clone());
                            self.status = format!("Could not load {slug}");
                        }
                    }
                }
                ApiEvent::Session(result) => match result {
                    Ok((status, solved)) => {
                        self.status = format!(
                            "Signed in as {} — {} solved",
                            status.display_name(),
                            solved.len()
                        );
                        self.user = Some(status);
                        self.solved = solved;
                    }
                    Err(err) => {
                        self.error = Some(format!("session check failed: {err}"));
                    }
                },
            }
        }
    }

    fn auth_badge(&self) -> AuthBadge {
        match &self.user {
            Some(user) if user.signed_in => AuthBadge::SignedIn {
                username: user.username.clone(),
                solved: self.solved.len(),
            },
            _ => AuthBadge::BrowseOnly,
        }
    }

    // -- theme --------------------------------------------------------------

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

    // -- panels -------------------------------------------------------------

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
                    .button(theme_button_job(theme))
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

    fn handle_list_action(&mut self, action: ProblemListAction) {
        if action.refresh {
            self.load_first_page();
        }
        if let Some(query) = action.server_search {
            self.search_problems(query);
        }
        if action.load_more {
            self.load_next_page();
        }
        if let Some(slug) = action.open {
            self.open_problem(slug);
        }
    }
}

impl eframe::App for LeetrApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();

        if self.list_loading || self.detail_loading {
            ui.ctx().request_repaint_after(Duration::from_millis(80));
        }

        egui::Panel::top("leetr-toolbar").show(ui, |ui| {
            ui.add_space(2.0);
            self.toolbar(ui);
            ui.add_space(2.0);
        });

        egui::Panel::bottom("leetr-status").show(ui, |ui| {
            let badge = self.auth_badge();
            ui::status_bar::show(
                ui,
                &self.config,
                &badge,
                &self.status,
                self.error.as_deref(),
            );
        });

        let mut action = ProblemListAction::default();
        let has_more = (self.problems.len() as u32) < self.total;

        egui::Panel::left("leetr-problems")
            .resizable(true)
            .default_size(420.0)
            .size_range(egui::Rangef::new(280.0, 720.0))
            .show(ui, |ui| {
                action = ui::problem_list::show(
                    ui,
                    &mut self.list_state,
                    &self.problems,
                    self.total,
                    self.list_loading,
                    self.list_error.as_deref(),
                    has_more,
                    self.selected.as_deref(),
                );
            });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            ui::statement::show(
                ui,
                self.detail.as_ref(),
                &self.detail_blocks,
                self.detail_loading,
                self.detail_error.as_deref(),
            );
        });

        self.handle_list_action(action);
    }
}

/// Label for the theme toggle.
///
/// The sun/moon glyphs only exist in the default *monospace* font's fallback,
/// so the icon and the label are rendered as two sections with different fonts
/// (`Color32::PLACEHOLDER` keeps the button's own text colour).
fn theme_button_job(theme: Theme) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        theme.icon(),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::monospace(14.0),
            color: egui::Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job.append(
        &format!(" {}", theme.label()),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(14.0),
            color: egui::Color32::PLACEHOLDER,
            ..Default::default()
        },
    );
    job
}

/// Launch the GUI. Blocks until the window is closed.
pub fn run() -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Leetr")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([840.0, 520.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Leetr",
        options,
        Box::new(|cc| Ok(Box::new(LeetrApp::new(cc)))),
    )
    .map_err(|err| anyhow::anyhow!("failed to start the window: {err}"))
}
