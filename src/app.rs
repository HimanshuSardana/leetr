//! The `eframe` application: top-level state, panel layout and background work.

use std::collections::BTreeSet;
use std::time::Duration;

use eframe::egui;

use std::sync::Arc;

use crate::bg::Bg;
use crate::config::{self, Config, Credentials};
use crate::editor::{Buffer as EditorBuffer, Workspace};
use crate::glyphs;
use crate::interop::{self, Command as InteropCommand, Shared as InteropShared, Snapshot};
use crate::leetcode::client::ProblemQuery;
use crate::leetcode::html::{self, Block};
use crate::leetcode::submit::submission_policy;
use crate::leetcode::{JudgeResult, LeetCodeClient, SubmitDecision, Verdict};
use crate::model::{Problem, ProblemPage, QuestionDetail, UserStatus};
use crate::nvim;
use crate::theme::Theme;
use crate::ui;
use crate::ui::code_editor::{self, JudgeView, LanguageChoice};
use crate::ui::palette::{PaletteEntry, PaletteOutcome, PaletteState};
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
    /// A finished judge run (sample tests or a submission).
    Judge {
        is_submission: bool,
        slug: String,
        result: Job<Box<JudgeResult>>,
        /// Carried through so the finished result can still show the warning.
        override_warning: Option<String>,
    },
    Detail {
        slug: String,
        result: Job<Box<QuestionDetail>>,
    },
    Session(Job<(UserStatus, BTreeSet<String>)>),
}

/// Which pane the central area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Statement,
    Code,
}

/// Cap on how many loaded problems the `Ctrl+P` palette will offer.
const MAX_PALETTE_PROBLEMS: usize = 1500;

/// Things the `Ctrl+K` palette can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    ToggleTheme,
    SelectProblem,
    ShowProblems,
    RefreshProblems,
    LoadMore,
    ClearFilters,
    TogglePremiumFilter,
    OpenOnLeetCode,
    CopyProblemLink,
    Quit,
}

impl Action {
    /// Stable, state-independent label used by the `Ctrl+K` palette.
    ///
    /// Kept as a method (rather than inline strings) so a test can assert that
    /// the labels users are likely to type actually match.
    fn search_label(self) -> &'static str {
        match self {
            Self::ToggleTheme => "Toggle theme",
            Self::SelectProblem => "Go to problem",
            Self::ShowProblems => "Show all problems",
            Self::RefreshProblems => "Reload problem list",
            Self::LoadMore => "Load more problems",
            Self::ClearFilters => "Clear problem filters",
            Self::TogglePremiumFilter => "Toggle premium-only problems",
            Self::OpenOnLeetCode => "Open on leetcode.com",
            Self::CopyProblemLink => "Copy problem link",
            Self::Quit => "Quit Leetr",
        }
    }
}

/// Every action the palette can offer, in display order.
///
/// Test-only: `action_catalog` filters this by applicability at runtime.
#[cfg(test)]
const ALL_ACTIONS: &[Action] = &[
    Action::ToggleTheme,
    Action::SelectProblem,
    Action::ShowProblems,
    Action::RefreshProblems,
    Action::LoadMore,
    Action::ClearFilters,
    Action::TogglePremiumFilter,
    Action::OpenOnLeetCode,
    Action::CopyProblemLink,
    Action::Quit,
];

/// One row of the `Ctrl+K` catalog: action, hint, icon, section, shortcut and
/// whether it applies right now.
type ActionRow = (
    Action,
    String,
    &'static str,
    &'static str,
    Option<&'static str>,
    bool,
);

/// Reduce a palette outcome to a chosen row index.
///
/// Choosing a row (or dismissing the palette) always closes it, so the palette
/// never lingers after the user has acted.
fn resolve_palette_choice(state: &mut PaletteState, outcome: &PaletteOutcome) -> Option<usize> {
    if outcome.chosen.is_some() || outcome.closed {
        state.close();
    }
    outcome.chosen
}

/// Transport form of the auth badge, for the control API.
fn auth_state_of(badge: &AuthBadge) -> interop::AuthState {
    match badge {
        AuthBadge::SignedIn { .. } => interop::AuthState::SignedIn,
        AuthBadge::Rejected => interop::AuthState::Rejected,
        AuthBadge::Checking => interop::AuthState::Checking,
        AuthBadge::BrowseOnly => interop::AuthState::BrowseOnly,
    }
}

/// Which auth badge to show.
///
/// The three "not signed in" cases are genuinely different, and conflating them
/// produced a status bar reading "browse-only | Signed in as (unknown)" with a
/// tooltip telling the user to add credentials they already had.
fn auth_badge_for(
    user: Option<&UserStatus>,
    credentials_configured: bool,
    solved: usize,
) -> AuthBadge {
    match user {
        Some(status) if status.signed_in => AuthBadge::SignedIn {
            username: status.username.clone(),
            solved,
        },
        // Checked, and LeetCode refused us.
        Some(_) => AuthBadge::Rejected,
        // Not checked yet.
        None if credentials_configured => AuthBadge::Checking,
        None => AuthBadge::BrowseOnly,
    }
}

/// The compact session line shown under the app name in the problem panel.
fn session_line(badge: &AuthBadge) -> (String, Option<egui::Color32>) {
    match badge {
        AuthBadge::SignedIn { username, solved } => {
            let text = if username.is_empty() {
                format!("signed in · {solved} solved")
            } else {
                format!("{username} · {solved} solved")
            };
            (text, Some(crate::theme::SUCCESS))
        }
        AuthBadge::Rejected => ("session rejected".to_string(), Some(crate::theme::DANGER)),
        AuthBadge::Checking => ("checking session…".to_string(), None),
        AuthBadge::BrowseOnly => ("browse-only".to_string(), Some(crate::theme::WARNING)),
    }
}

/// Should a plain `Tab` switch from the statement pane to the code pane?
///
/// Tab is the standard "move focus" key, so it only doubles as a pane switch
/// when nothing has keyboard focus. Without the second condition, pressing Tab
/// while typing in the problem filter teleports the user to the code pane
/// mid-query (verified in the GUI: 31,699 pixels of view change).
fn tab_switches_panes(current: Tab, nothing_focused: bool) -> bool {
    current == Tab::Statement && nothing_focused
}

/// Canonical URL for a problem.
fn problem_url(slug: &str) -> String {
    format!("https://leetcode.com/problems/{slug}/")
}

/// Hand a URL to the platform opener. Best effort — failures are surfaced in
/// the status bar rather than aborting.
fn open_in_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let candidates: &[&str] = &["open"];
    #[cfg(target_os = "windows")]
    let candidates: &[&str] = &["explorer"];
    #[cfg(all(unix, not(target_os = "macos")))]
    let candidates: &[&str] = &["xdg-open", "gio", "sensible-browser"];

    for command in candidates {
        let mut process = std::process::Command::new(command);
        if *command == "gio" {
            process.arg("open");
        }
        match process.arg(url).spawn() {
            Ok(_) => return Ok(()),
            Err(_) => continue,
        }
    }

    Err(format!("could not find a browser opener for {url}"))
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

    actions_palette: PaletteState,
    problems_palette: PaletteState,

    workspace: Workspace,
    code: Option<EditorBuffer>,
    languages: Vec<LanguageChoice>,
    tab: Tab,
    reloaded_from_disk: bool,
    sidebar_collapsed: bool,
    /// Code pushed over the API before its problem finished loading.
    pending_code: Option<String>,
    judge: Option<JudgeView>,

    interop_shared: Arc<InteropShared>,
    interop: Option<interop::RunningServer>,
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

        // Draw the whole interface in Iosevka Nerd Font when it is installed.
        crate::fonts::install(&cc.egui_ctx);

        let client = match LeetCodeClient::new(credentials.clone()) {
            Ok(client) => client,
            Err(err) => {
                error = Some(format!("could not build the HTTP client: {err}"));
                // `with_base_url` only fails if TLS setup fails, which cannot
                // happen with rustls defaults; fall back to an unusable client.
                LeetCodeClient::new(Credentials::default()).expect("default client")
            }
        };

        let workspace = Workspace::from_config(&config);
        let interop_shared = Arc::new(InteropShared::new());

        let bind = format!("127.0.0.1:{}", config.interop_port);
        let interop = match interop::start(&bind, Arc::clone(&interop_shared)) {
            Ok(server) => Some(server),
            Err(err) => {
                // A busy port must not stop the app from starting.
                match interop::start("127.0.0.1:0", Arc::clone(&interop_shared)) {
                    Ok(server) => {
                        error = Some(format!(
                            "{err}; control API moved to port {}",
                            server.port()
                        ));
                        Some(server)
                    }
                    Err(err) => {
                        error = Some(format!("control API unavailable: {err}"));
                        None
                    }
                }
            }
        };

        // Wake the UI whenever the control API queues a command: egui only runs
        // a frame when asked to, so an idle app would never apply them.
        {
            let ctx = cc.egui_ctx.clone();
            interop_shared.set_waker(Arc::new(move || ctx.request_repaint()));
        }

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
            actions_palette: PaletteState::default(),
            problems_palette: PaletteState::default(),
            workspace,
            code: None,
            languages: Vec::new(),
            tab: Tab::Statement,
            sidebar_collapsed: false,
            reloaded_from_disk: false,
            pending_code: None,
            judge: None,
            interop_shared,
            interop,
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

    /// Return to the full-width problem list, dropping the open problem.
    fn close_problem(&mut self) {
        if self.selected.is_none() {
            return;
        }
        self.selected = None;
        self.detail = None;
        self.detail_blocks.clear();
        self.detail_error = None;
        self.detail_loading = false;
        self.code = None;
        self.languages.clear();
        self.judge = None;
        self.tab = Tab::Statement;
        self.status = "Showing all problems".to_string();
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
                            self.languages = code_editor::language_choices(&detail);
                            let lang = code_editor::preferred_language(
                                &detail,
                                &self.config.default_language,
                            );
                            self.prepare_workspace(&detail, &lang);
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
                ApiEvent::Judge {
                    is_submission,
                    slug,
                    result,
                    override_warning,
                } => match result {
                    Ok(result) => {
                        let mut view = JudgeView::from_result(&result, is_submission);
                        view.warning = override_warning;
                        self.status = view.summary.clone();
                        if self.error.is_none() {
                            self.error = None;
                        }
                        // A successful first submission means the problem is
                        // now solved; keep the guard's data honest.
                        if is_submission && result.verdict() == Verdict::Accepted {
                            self.solved.insert(slug.clone());
                        }
                        self.judge = Some(view);
                    }
                    Err(err) => {
                        self.status = "judge request failed".to_string();
                        self.judge = Some(JudgeView {
                            is_submission,
                            error: Some(err),
                            warning: override_warning,
                            ..JudgeView::default()
                        });
                    }
                },
                ApiEvent::Session(result) => match result {
                    Ok((status, solved)) => {
                        if status.signed_in {
                            self.status = format!(
                                "Signed in as {} — {} solved",
                                status.display_name(),
                                solved.len()
                            );
                            self.solved = solved;
                        } else {
                            // Do not claim to be signed in when we are not: an
                            // expired cookie reported "Signed in as (unknown)".
                            //
                            // Deliberately NOT written to `status`: that is a
                            // transient slot which routine messages ("50
                            // problems loaded") legitimately overwrite, and a
                            // standing condition does not belong there. The badge
                            // and its tooltip carry it in the UI, and `auth` in
                            // the control API carries it to tooling.
                            self.solved.clear();
                        }
                        // `Some(_)` with `signed_in == false` means "checked and
                        // refused", which the badge renders distinctly.
                        self.user = Some(status);
                    }
                    Err(err) => {
                        self.error = Some(format!("session check failed: {err}"));
                    }
                },
            }
        }
    }

    fn auth_badge(&self) -> AuthBadge {
        auth_badge_for(
            self.user.as_ref(),
            self.credentials.is_configured(),
            self.solved.len(),
        )
    }

    // -- workspace / editor --------------------------------------------------

    /// Create the problem's files and load its buffer, applying any code that
    /// arrived over the control API while the problem was still loading.
    fn prepare_workspace(&mut self, detail: &QuestionDetail, lang_slug: &str) {
        match self.workspace.prepare(detail, lang_slug) {
            Ok(prepared) => {
                let boilerplate = detail
                    .snippet_for(lang_slug)
                    .unwrap_or_default()
                    .to_string();
                let mut buffer = EditorBuffer::load(
                    prepared.solution.clone(),
                    &detail.slug,
                    &detail.frontend_id,
                    lang_slug,
                    &boilerplate,
                );

                if let Some(pushed) = self.pending_code.take() {
                    buffer.set_text(pushed);
                    if let Err(err) = buffer.save() {
                        self.error = Some(format!("could not save pushed code: {err}"));
                    }
                }

                self.status = if prepared.created_solution {
                    format!("created {}", prepared.solution.display())
                } else {
                    format!("editing {}", prepared.solution.display())
                };
                self.code = Some(buffer);
            }
            Err(err) => {
                self.code = None;
                self.error = Some(format!("could not prepare the workspace: {err}"));
            }
        }
    }

    fn switch_language(&mut self, lang_slug: String) {
        let Some(detail) = self.detail.clone() else {
            return;
        };
        if self
            .code
            .as_ref()
            .is_some_and(|buffer| buffer.lang_slug == lang_slug)
        {
            return;
        }

        match self.workspace.prepare(&detail, &lang_slug) {
            Ok(prepared) => {
                let boilerplate = detail
                    .snippet_for(&lang_slug)
                    .unwrap_or_default()
                    .to_string();
                let buffer = EditorBuffer::load(
                    prepared.solution,
                    &detail.slug,
                    &detail.frontend_id,
                    &lang_slug,
                    &boilerplate,
                );
                self.status = format!("switched to {lang_slug}");
                self.code = Some(buffer);
            }
            Err(err) => self.error = Some(format!("could not switch language: {err}")),
        }
    }

    /// Pick up edits made to the workspace file outside the app (nvim `:w`).
    fn poll_workspace_file(&mut self) {
        let Some(buffer) = self.code.as_mut() else {
            return;
        };
        match buffer.reload_if_changed() {
            Ok(true) => {
                self.reloaded_from_disk = true;
                self.status = "solution reloaded from disk".to_string();
            }
            Ok(false) => {}
            Err(err) => self.error = Some(format!("could not read the solution file: {err}")),
        }
    }

    /// Everything the judge needs about the current buffer.
    fn judge_request(&self) -> Option<(String, String, String, String, String)> {
        let detail = self.detail.as_ref()?;
        let buffer = self.code.as_ref()?;
        Some((
            detail.slug.clone(),
            detail.question_id.clone(),
            buffer.lang_slug.clone(),
            buffer.text.clone(),
            detail.sample_test_case.clone(),
        ))
    }

    fn start_judge(&mut self, is_submission: bool) {
        if !self.client.is_authenticated() {
            self.judge = Some(JudgeView {
                is_submission,
                error: Some(format!(
                    "no credentials configured — add them to {}",
                    config::credentials_path().display()
                )),
                ..JudgeView::default()
            });
            self.status = "not signed in".to_string();
            return;
        }

        let Some((slug, question_id, lang_slug, code, sample_input)) = self.judge_request() else {
            self.status = "open a problem first".to_string();
            return;
        };

        let mut override_warning: Option<String> = None;

        if is_submission {
            // Refuse to submit to a problem that is not already solved. This is
            // the safety rail requested when the project was set up.
            if self.solved.is_empty() {
                self.judge = Some(JudgeView {
                    is_submission,
                    error: Some(
                        "your solved set has not loaded yet, so Leetr refuses to submit \
                         (it will never submit to an unsolved problem)"
                            .to_string(),
                    ),
                    ..JudgeView::default()
                });
                self.status = "submission blocked".to_string();
                return;
            }
            // The decision also carries a warning when the guard is off, so
            // submitting to an unsolved problem is never silent.
            match submission_policy(&slug, &self.solved, self.config.allow_unsolved_submit) {
                SubmitDecision::Allowed => {}
                SubmitDecision::AllowedByOverride(warning) => override_warning = Some(warning),
                SubmitDecision::Refused(reason) => {
                    self.judge = Some(JudgeView {
                        is_submission,
                        error: Some(reason),
                        ..JudgeView::default()
                    });
                    self.status = "submission blocked".to_string();
                    return;
                }
            }
        }

        self.judge = Some(JudgeView {
            is_submission,
            running: true,
            warning: override_warning.clone(),
            summary: if is_submission {
                "Submitting…".to_string()
            } else {
                "Running sample tests…".to_string()
            },
            ..JudgeView::default()
        });
        self.status = self
            .judge
            .as_ref()
            .map(|j| j.summary.clone())
            .unwrap_or_default();

        let client = self.client.clone();
        self.bg.spawn(move || {
            let outcome: Job<Box<JudgeResult>> = (|| {
                let id = if is_submission {
                    client
                        .submit_solution(&slug, &question_id, &lang_slug, &code)
                        .map(|id| id.to_string())
                        .map_err(|err| err.to_string())?
                } else {
                    client
                        .run_sample(&slug, &question_id, &lang_slug, &code, &sample_input)
                        .map_err(|err| err.to_string())?
                };
                client
                    .wait_for_submission(&id)
                    .map(Box::new)
                    .map_err(|err| err.to_string())
            })();

            ApiEvent::Judge {
                is_submission,
                slug,
                result: outcome,
                override_warning,
            }
        });
    }

    fn open_in_nvim(&mut self) {
        let Some(buffer) = self.code.as_ref() else {
            self.status = "open a problem first".to_string();
            return;
        };
        // Make sure the editor sees what is on screen.
        let path = buffer.path().to_path_buf();
        if let Some(buffer) = self.code.as_mut() {
            if let Err(err) = buffer.save() {
                self.error = Some(format!("could not save before opening nvim: {err}"));
                return;
            }
        }

        match nvim::open(&path, &self.config.editor_command) {
            Ok(how) => self.status = format!("opened {path:?} with {how}"),
            Err(err) => self.error = Some(err),
        }
    }

    // -- control API ---------------------------------------------------------

    fn apply_interop_commands(&mut self) {
        for command in self.interop_shared.take_commands() {
            match command {
                InteropCommand::Open { slug } => self.open_problem(slug),
                InteropCommand::Run => self.start_judge(false),
                InteropCommand::Submit => self.start_judge(true),
                InteropCommand::SetCode {
                    slug,
                    language,
                    code,
                } => {
                    if let Some(language) = language.filter(|lang| !lang.trim().is_empty()) {
                        self.switch_language(language);
                    }

                    let same_problem = match (slug.as_deref(), self.selected.as_deref()) {
                        (Some(incoming), Some(selected)) => incoming == selected,
                        (None, _) => true,
                        (Some(_), None) => false,
                    };

                    if !same_problem {
                        if let Some(slug) = slug {
                            // The problem is not loaded yet; stash the code for
                            // `prepare_workspace` to apply.
                            self.pending_code = Some(code);
                            self.open_problem(slug);
                            continue;
                        }
                    }

                    match self.code.as_mut() {
                        Some(buffer) => {
                            buffer.set_text(code);
                            match buffer.save() {
                                Ok(()) => {
                                    self.status = "solution synced from the editor".to_string();
                                    self.error = None;
                                }
                                Err(err) => {
                                    self.error = Some(format!("could not save the solution: {err}"))
                                }
                            }
                        }
                        None => self.pending_code = Some(code),
                    }
                }
            }
        }
    }

    /// Publish what the control API can read.
    ///
    /// Called once per frame: the payload is a LeetCode-sized solution, so the
    /// clone is cheap and it removes a whole class of staleness bugs.
    fn publish_snapshot(&self) {
        let snapshot = Snapshot {
            slug: self.selected.clone(),
            title: self.detail.as_ref().map(|detail| detail.title.clone()),
            language: self.code.as_ref().map(|buffer| buffer.lang_slug.clone()),
            file: self
                .code
                .as_ref()
                .map(|buffer| buffer.path().display().to_string()),
            code: self
                .code
                .as_ref()
                .map(|buffer| buffer.text.clone())
                .unwrap_or_default(),
            solved: self
                .selected
                .as_ref()
                .is_some_and(|slug| self.solved.contains(slug)),
            ready: self.code.is_some(),
            status: self.status.clone(),
            auth: auth_state_of(&self.auth_badge()),
            version: interop::API_VERSION.to_string(),
            judge: self.judge.as_ref().map(|judge| interop::JudgeSummary {
                is_submission: judge.is_submission,
                running: judge.running,
                verdict: judge.verdict.label().to_string(),
                summary: judge.summary.clone(),
                error: judge.error.clone(),
                warning: judge.warning.clone(),
            }),
        };
        self.interop_shared.publish(snapshot);
    }

    fn code_pane(&mut self, ui: &mut egui::Ui) {
        let languages = self.languages.clone();
        let Some(buffer) = self.code.as_mut() else {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(egui::RichText::new("Open a problem to get a solution buffer.").weak());
            });
            return;
        };

        let dirty = buffer.is_dirty();
        let reloaded = self.reloaded_from_disk;
        let can_submit = self.config.allow_unsolved_submit
            || self
                .selected
                .as_ref()
                .is_some_and(|slug| self.solved.contains(slug));
        let judge = self.judge.clone();
        let action = code_editor::show(
            ui,
            buffer,
            &languages,
            dirty,
            reloaded,
            judge.as_ref(),
            can_submit,
        );
        self.reloaded_from_disk = false;

        if action.run {
            self.start_judge(false);
        }
        if action.submit {
            self.start_judge(true);
        }

        if let Some(language) = action.language {
            self.switch_language(language);
        }
        if action.open_in_nvim {
            self.open_in_nvim();
        }
        if action.changed {
            if let Some(buffer) = self.code.as_mut() {
                if let Err(err) = buffer.save() {
                    self.error = Some(format!("could not save the solution: {err}"));
                }
            }
        }
    }

    // -- command palettes ---------------------------------------------------

    /// Every action reachable from `Ctrl+K`, in display order.
    ///
    /// Labels are the verb ("Toggle theme") and hints are the detail the user
    /// might remember instead ("switch to Light mode"), because both are
    /// searched by the fuzzy matcher.
    fn action_catalog(&self) -> Vec<(Action, PaletteEntry)> {
        let theme = self.config.theme;
        let has_selection = self.selected.is_some();
        let has_more = (self.problems.len() as u32) < self.total;
        let slug = self.selected.clone().unwrap_or_default();

        // (action, hint, icon, section, shortcut, enabled)
        let catalog: [ActionRow; 10] = [
            (
                Action::SelectProblem,
                "Ctrl+P problem selector".to_string(),
                glyphs::BULLET,
                "Quick actions",
                Some("Ctrl P"),
                true,
            ),
            (
                Action::ShowProblems,
                "back to the full list".to_string(),
                glyphs::BULLET,
                "Quick actions",
                None,
                has_selection,
            ),
            (
                Action::RefreshProblems,
                "refetch the first page".to_string(),
                glyphs::REFRESH,
                "Problem list",
                None,
                true,
            ),
            (
                Action::LoadMore,
                "fetch the next page".to_string(),
                glyphs::BULLET,
                "Problem list",
                None,
                has_more,
            ),
            (
                Action::ClearFilters,
                "reset search, difficulty and status".to_string(),
                glyphs::BULLET,
                "Problem list",
                None,
                self.list_state.is_filtered(),
            ),
            (
                Action::TogglePremiumFilter,
                if self.list_state.hide_premium {
                    "currently hidden".to_string()
                } else {
                    "currently shown".to_string()
                },
                glyphs::PREMIUM,
                "Problem list",
                None,
                true,
            ),
            (
                Action::OpenOnLeetCode,
                slug.clone(),
                glyphs::BULLET,
                "Current problem",
                None,
                has_selection,
            ),
            (
                Action::CopyProblemLink,
                slug,
                glyphs::BULLET,
                "Current problem",
                None,
                has_selection,
            ),
            (
                Action::ToggleTheme,
                format!("switch to {} mode", theme.toggled().label()),
                theme.toggled().icon(),
                "Application",
                None,
                true,
            ),
            (
                Action::Quit,
                String::new(),
                glyphs::BULLET,
                "Application",
                None,
                true,
            ),
        ];

        catalog
            .into_iter()
            .filter(|row| row.5)
            .map(|(action, hint, icon, section, shortcut, _)| {
                let mut entry = PaletteEntry::new(action.search_label())
                    .with_hint(hint)
                    .with_icon(icon)
                    .with_section(section);
                if let Some(shortcut) = shortcut {
                    entry = entry.with_shortcut(shortcut);
                }
                (action, entry)
            })
            .collect()
    }

    fn run_action(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::ToggleTheme => {
                let next = self.config.theme.toggled();
                self.set_theme(next, ctx);
            }
            Action::SelectProblem => self.problems_palette.open(),
            Action::ShowProblems => self.close_problem(),
            Action::RefreshProblems => self.load_first_page(),
            Action::LoadMore => self.load_next_page(),
            Action::ClearFilters => self.list_state.clear_filters(),
            Action::TogglePremiumFilter => {
                self.list_state.hide_premium = !self.list_state.hide_premium;
            }
            Action::OpenOnLeetCode => {
                if let Some(slug) = self.selected.clone() {
                    let url = problem_url(&slug);
                    if let Err(err) = open_in_browser(&url) {
                        self.error = Some(err);
                    }
                }
            }
            Action::CopyProblemLink => {
                if let Some(slug) = self.selected.clone() {
                    let url = problem_url(&slug);
                    ctx.copy_text(url.clone());
                    self.status = format!("Copied {url}");
                }
            }
            Action::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Draw `Ctrl+K` (actions) and `Ctrl+P` (problem selector).
    fn show_palettes(&mut self, ctx: &egui::Context) {
        // -- Ctrl+K: actions ------------------------------------------------
        let catalog = self.action_catalog();
        let entries: Vec<PaletteEntry> = catalog.iter().map(|(_, e)| e.clone()).collect();

        let outcome = ui::palette::show(
            ctx,
            &mut self.actions_palette,
            &entries,
            &ui::palette::PaletteConfig {
                id: "actions",
                placeholder: "Type a command…",
                search_badge: Some("Ctrl K"),
                chips: Vec::new(),
                empty_action: None,
            },
        );
        if let Some(index) = resolve_palette_choice(&mut self.actions_palette, &outcome) {
            if let Some((action, _)) = catalog.get(index) {
                self.run_action(*action, ctx);
            }
        }

        // -- Ctrl+P: problem selector ---------------------------------------
        if self.problems_palette.is_open() {
            let query = self.problems_palette.query().to_string();
            let cap = self.problems.len().min(MAX_PALETTE_PROBLEMS);
            let entries: Vec<PaletteEntry> = self.problems[..cap]
                .iter()
                .map(|problem| {
                    PaletteEntry::new(format!("{}. {}", problem.id, problem.title))
                        .with_hint(format!(
                            "{} · {}",
                            problem.difficulty.label(),
                            problem.status.label()
                        ))
                        .with_icon(problem.status.marker().to_string())
                        .with_section(problem.status.label())
                        .with_tags([
                            problem.difficulty.label().to_ascii_lowercase(),
                            problem.status.label().to_string(),
                        ])
                })
                .collect();

            let search_label = if query.trim().len() >= 2 {
                Some(format!("Search LeetCode for “{}”", query.trim()))
            } else {
                None
            };

            let outcome = ui::palette::show(
                ctx,
                &mut self.problems_palette,
                &entries,
                &ui::palette::PaletteConfig {
                    id: "problems",
                    placeholder: "Jump to a problem…",
                    search_badge: Some("Ctrl P"),
                    chips: vec![
                        ui::palette::PaletteChip::new("all", "All"),
                        ui::palette::PaletteChip::new("easy", "Easy"),
                        ui::palette::PaletteChip::new("medium", "Medium"),
                        ui::palette::PaletteChip::new("hard", "Hard"),
                        ui::palette::PaletteChip::new("solved", "Solved"),
                        ui::palette::PaletteChip::new("attempted", "Attempted"),
                        ui::palette::PaletteChip::new("todo", "Todo"),
                    ],
                    empty_action: search_label.as_deref(),
                },
            );

            if outcome.empty_action {
                self.search_problems(query.trim().to_string());
                self.problems_palette.close();
            }
            if let Some(index) = resolve_palette_choice(&mut self.problems_palette, &outcome) {
                if let Some(problem) = self.problems.get(index) {
                    let slug = problem.slug.clone();
                    self.open_problem(slug);
                }
            }
        }
    }

    /// True while any palette is open, so global shortcuts stand down.
    fn palette_open(&self) -> bool {
        self.actions_palette.is_open() || self.problems_palette.is_open()
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

    fn handle_list_action(&mut self, action: ProblemListAction, ctx: &egui::Context) {
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
        if action.show_all {
            self.close_problem();
        }
        if action.open_actions {
            self.actions_palette.open();
        }
        if action.toggle_theme {
            let next = self.config.theme.toggled();
            self.set_theme(next, ctx);
        }
    }
}

impl eframe::App for LeetrApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.apply_interop_commands();
        self.poll_workspace_file();

        if self.list_loading || self.detail_loading {
            ui.ctx().request_repaint_after(Duration::from_millis(80));
        }

        // With a solution open, keep ticking so an edit saved outside the app
        // (nvim `:w`) is noticed. Without this, egui would idle and the mtime
        // check in `poll_workspace_file` would never run.
        if self.code.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(400));
        }

        // Global shortcuts.
        //
        // `consume_key` matches against the *event's own* modifiers rather than
        // `InputState::modifiers` (the live key state). That matters: with a
        // fast synthetic or human key sequence the modifier can already be
        // released by the time the frame runs, in which case checking
        // `i.modifiers` silently misses the shortcut. Consuming the event also
        // stops a focused `TextEdit` from acting on it (egui binds Ctrl+K to
        // "delete to end of line").
        //
        // `Modifiers::COMMAND` is Ctrl on Linux/Windows and Cmd on macOS.
        // Shortcuts stand down while a palette is open so it keeps focus.
        if !self.palette_open() {
            let open_actions = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::K));
            let open_problems = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::P));
            let open_nvim = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O));
            let run_tests = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Quote));
            let submit_solution = ui.ctx().input_mut(|i| {
                i.consume_key(
                    egui::Modifiers {
                        command: true,
                        ..Default::default()
                    },
                    egui::Key::Enter,
                )
            });
            // Tab switches panes, but only when no widget has keyboard focus.
            // Tab is the standard "move focus" key, so claiming it while the
            // user is typing in the filter box teleports them to the code pane
            // mid-query. This runs before widgets are drawn, so the check has
            // to happen here rather than relying on the text field to refuse it.
            let nothing_focused = ui.ctx().memory(|memory| memory.focused().is_none());
            // Only consume the key when it is actually ours — otherwise the
            // focused widget must still receive it.
            let tab_to_code = if tab_switches_panes(self.tab, nothing_focused) {
                ui.ctx()
                    .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
            } else {
                false
            };
            let toggle_sidebar = ui
                .ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::B));

            if open_actions {
                self.actions_palette.open();
            }
            if open_problems {
                self.problems_palette.open();
            }
            if open_nvim {
                self.open_in_nvim();
            }
            if run_tests && self.code.is_some() {
                self.start_judge(false);
            }
            if submit_solution && self.code.is_some() {
                self.start_judge(true);
            }
            if tab_to_code {
                self.tab = Tab::Code;
            }
            if toggle_sidebar && self.selected.is_some() {
                self.sidebar_collapsed = !self.sidebar_collapsed;
                self.status = if self.sidebar_collapsed {
                    "Sidebar hidden".to_string()
                } else {
                    "Sidebar shown".to_string()
                };
            }
        }

        egui::Panel::bottom("leetr-status").show(ui, |ui| {
            let badge = self.auth_badge();
            let api_port = self
                .interop
                .as_ref()
                .map(|server| server.port())
                .unwrap_or(self.config.interop_port);
            ui::status_bar::show(
                ui,
                &self.config,
                &badge,
                &self.status,
                self.error.as_deref(),
                api_port,
            );
        });

        // The problem list is the app's sidebar. Its chrome (brand, session
        // line, quick actions) is drawn by the panel itself.
        let badge = self.auth_badge();
        let (session, session_color) = session_line(&badge);
        let header = ui::problem_list::Header {
            session: &session,
            session_color,
            theme: self.config.theme,
        };

        let mut action = ProblemListAction::default();
        let has_more = (self.problems.len() as u32) < self.total;

        if self.selected.is_none() {
            // Nothing is open, so the list spans the whole window.
            egui::CentralPanel::default_margins().show(ui, |ui| {
                action = ui::problem_list::show(
                    ui,
                    &mut self.list_state,
                    &header,
                    &self.problems,
                    self.total,
                    self.list_loading,
                    self.list_error.as_deref(),
                    has_more,
                    None,
                );
            });
        } else {
            // A problem is open: the list shrinks to a sidebar on the left and
            // the statement/code fills the wider column beside it.
            //
            // `show_collapsible` slides the panel off-screen and clips its
            // contents while doing so, and keeps a thin grab handle so the user
            // can drag it back open. Ctrl+B flips the same flag.
            //
            // Note the polarity: the panel wants `is_expanded`, our field is
            // `sidebar_collapsed`. Passing the field directly renders the
            // sidebar collapsed (invisible) from the start.
            let mut expanded = !self.sidebar_collapsed;
            egui::Panel::left("leetr-problems")
                .resizable(true)
                .default_size(340.0)
                .size_range(egui::Rangef::new(240.0, 560.0))
                .show_collapsible(ui, &mut expanded, |ui| {
                    action = ui::problem_list::show(
                        ui,
                        &mut self.list_state,
                        &header,
                        &self.problems,
                        self.total,
                        self.list_loading,
                        self.list_error.as_deref(),
                        has_more,
                        self.selected.as_deref(),
                    );
                });

            // The panel flips `expanded` itself when the user drags the resize
            // handle closed or double-clicks the edge; mirror that back.
            if expanded == self.sidebar_collapsed {
                self.sidebar_collapsed = !expanded;
                self.status = if self.sidebar_collapsed {
                    "Sidebar hidden".to_string()
                } else {
                    "Sidebar shown".to_string()
                };
            }

            egui::CentralPanel::default_margins().show(ui, |ui| match self.tab {
                Tab::Statement => ui::statement::show(
                    ui,
                    self.detail.as_ref(),
                    &self.detail_blocks,
                    self.detail_loading,
                    self.detail_error.as_deref(),
                ),
                Tab::Code => self.code_pane(ui),
            });
        }

        let ctx = ui.ctx().clone();
        self.handle_list_action(action, &ctx);
        self.show_palettes(&ctx);
        self.publish_snapshot();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::palette;
    use egui::{Event, Key, Modifiers, RawInput};

    /// Build a one-shot `RawInput` whose *live* modifier state is `live` while
    /// the key event itself carries `event_modifiers`.
    fn raw_input(key: Key, event_modifiers: Modifiers, live: Modifiers) -> RawInput {
        RawInput {
            events: vec![
                Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: event_modifiers,
                },
                // The modifier is released before the frame runs — exactly what
                // a fast (synthetic or human) key sequence looks like.
                Event::ModifiersChanged(live),
            ],
            ..Default::default()
        }
    }

    /// Regression test for a real bug.
    ///
    /// `Ctrl+K` did nothing because the check used `InputState::modifiers`
    /// (the *live* key state), which had already been reset to `NONE` by the
    /// time the frame ran. `consume_key` matches the event's own modifiers.
    #[test]
    fn consume_key_matches_the_event_modifiers_not_the_live_key_state() {
        let ctx = egui::Context::default();

        let mut naive = false;
        let mut robust = false;
        let mut output = ctx.run_ui(
            raw_input(Key::K, Modifiers::COMMAND, Modifiers::NONE),
            |_ui| {
                naive = ctx.input(|i| i.modifiers.command && i.key_pressed(Key::K));
                robust = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K));
            },
        );
        output.textures_delta.clear();

        assert!(
            !naive,
            "the live modifier state is stale here — checking it is the bug"
        );
        assert!(
            robust,
            "consume_key must match on the event's own modifiers"
        );
    }

    #[test]
    fn plain_keys_do_not_trigger_command_shortcuts() {
        let ctx = egui::Context::default();
        let mut triggered = false;
        let mut output = ctx.run_ui(raw_input(Key::K, Modifiers::NONE, Modifiers::NONE), |_ui| {
            triggered = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K));
        });
        output.textures_delta.clear();
        assert!(!triggered, "K without a modifier must not open the palette");
    }

    #[test]
    fn a_shortcut_fires_only_once_per_press() {
        let ctx = egui::Context::default();
        let mut first = false;
        let mut second = false;
        let mut output = ctx.run_ui(
            raw_input(Key::P, Modifiers::COMMAND, Modifiers::NONE),
            |_ui| {
                first = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::P));
                second = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::P));
            },
        );
        output.textures_delta.clear();

        assert!(first);
        assert!(!second, "a consumed shortcut must not fire twice");
    }

    #[test]
    fn choosing_a_row_index_dismisses_the_palette() {
        let mut state = PaletteState::default();
        state.open();

        let outcome = PaletteOutcome {
            chosen: Some(3),
            ..Default::default()
        };
        assert_eq!(resolve_palette_choice(&mut state, &outcome), Some(3));
        assert!(!state.is_open(), "choosing must close the palette");
    }

    #[test]
    fn closing_without_choosing_leaves_nothing_selected() {
        let mut state = PaletteState::default();
        state.open();

        let outcome = PaletteOutcome {
            closed: true,
            ..Default::default()
        };
        assert_eq!(resolve_palette_choice(&mut state, &outcome), None);
        assert!(!state.is_open());
    }

    #[test]
    fn an_open_palette_with_no_action_stays_open() {
        let mut state = PaletteState::default();
        state.open();

        assert_eq!(
            resolve_palette_choice(&mut state, &PaletteOutcome::default()),
            None
        );
        assert!(state.is_open(), "typing must not close the palette");
    }

    #[test]
    fn every_badge_maps_to_a_distinct_auth_state() {
        use interop::AuthState;
        assert_eq!(
            auth_state_of(&AuthBadge::SignedIn {
                username: "a".into(),
                solved: 1
            }),
            AuthState::SignedIn
        );
        assert_eq!(auth_state_of(&AuthBadge::Rejected), AuthState::Rejected);
        assert_eq!(auth_state_of(&AuthBadge::Checking), AuthState::Checking);
        assert_eq!(auth_state_of(&AuthBadge::BrowseOnly), AuthState::BrowseOnly);
        // "expired cookie" and "no credentials" must never be conflated, since
        // they need different fixes.
        assert_ne!(AuthState::Rejected, AuthState::BrowseOnly);
    }

    #[test]
    fn an_expired_session_is_not_reported_as_signed_in() {
        let refused = UserStatus {
            signed_in: false,
            username: String::new(),
            premium: false,
        };
        // Credentials present, session refused -> Rejected, NOT BrowseOnly
        // ("no credentials found") and emphatically not SignedIn.
        assert_eq!(auth_badge_for(Some(&refused), true, 0), AuthBadge::Rejected);
    }

    #[test]
    fn the_auth_badge_distinguishes_every_state() {
        let ok = UserStatus {
            signed_in: true,
            username: "alice".into(),
            premium: false,
        };
        let refused = UserStatus {
            signed_in: false,
            username: String::new(),
            premium: false,
        };

        assert_eq!(
            auth_badge_for(Some(&ok), true, 42),
            AuthBadge::SignedIn {
                username: "alice".into(),
                solved: 42
            }
        );
        assert_eq!(auth_badge_for(Some(&refused), true, 0), AuthBadge::Rejected);
        // Configured but not yet checked is its own state: saying "browse-only"
        // here would again tell the user to add credentials they have.
        assert_eq!(auth_badge_for(None, true, 0), AuthBadge::Checking);
        assert_eq!(auth_badge_for(None, false, 0), AuthBadge::BrowseOnly);
    }

    #[test]
    fn tab_only_switches_panes_when_nothing_is_focused() {
        // Focused text field: Tab belongs to the widget, not to us.
        assert!(!tab_switches_panes(Tab::Statement, false));
        assert!(!tab_switches_panes(Tab::Code, false));

        // Nothing focused: the shortcut is ours on the statement pane only.
        assert!(tab_switches_panes(Tab::Statement, true));
        assert!(
            !tab_switches_panes(Tab::Code, true),
            "Tab must not toggle back out of the code pane"
        );
    }

    #[test]
    fn problem_urls_point_at_leetcode() {
        assert_eq!(
            problem_url("two-sum"),
            "https://leetcode.com/problems/two-sum/"
        );
    }

    #[test]
    fn every_action_has_a_distinct_non_empty_label() {
        let mut labels: Vec<&str> = ALL_ACTIONS.iter().map(|a| a.search_label()).collect();
        assert!(labels.iter().all(|l| !l.trim().is_empty()));
        let count = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), count, "action labels must be unique");
    }

    /// The labels have to actually match what a user would type, otherwise the
    /// palette looks broken even though the plumbing works.
    #[test]
    fn searching_for_theme_finds_the_toggle() {
        let labels: Vec<&str> = ALL_ACTIONS.iter().map(|a| a.search_label()).collect();
        let ranked = palette::rank("theme", &labels);
        assert_eq!(
            ranked.first(),
            Some(&0),
            "Toggle theme should rank first: {ranked:?}"
        );
    }

    #[test]
    fn searching_for_expected_words_finds_their_actions() {
        let labels: Vec<&str> = ALL_ACTIONS.iter().map(|a| a.search_label()).collect();
        for (query, expected) in [
            ("theme", Action::ToggleTheme),
            ("reload", Action::RefreshProblems),
            ("more", Action::LoadMore),
            ("filters", Action::ClearFilters),
            ("premium", Action::TogglePremiumFilter),
            ("copy", Action::CopyProblemLink),
            ("quit", Action::Quit),
        ] {
            let ranked = palette::rank(query, &labels);
            let found = ranked.first().map(|&index| ALL_ACTIONS[index]);
            assert_eq!(
                found,
                Some(expected),
                "query {query:?} should surface {expected:?}, got {found:?}"
            );
        }
    }
}
