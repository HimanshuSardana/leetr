//! The GPUI application: top-level state, layout and background work.
//!
//! Leetr's UI is a retained-mode tree built with
//! [GPUI Kit](https://gpui-kit.com/): [`Button`], [`Input`], [`Textarea`],
//! [`TabBar`]/[`Tab`] and [`Spinner`] from `gpui-component`, laid out with
//! plain GPUI `div`s. Everything that is not rendering — the LeetCode client,
//! workspace files, nvim integration and the loopback control API — is shared
//! with the CLI and untouched by this module.
//!
//! Network calls never block the UI thread: jobs run on plain `std::thread`s
//! (see [`crate::bg`]) and a lightweight timer applies their results to the
//! view, which is GPUI's moral equivalent of the old per-frame drain.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::base::Selectable;
use gpui_kit::component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon};
// The complete Lucide catalog ships with the assets crate; the component's own
// `IconName` is only the small legacy subset.
use gpui_kit::assets::IconName;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab as KitTab, TabBar};
use gpui_kit::component::theme::Theme as KitTheme;
use gpui_kit::component::{IndexPath, Sizable as _};
use gpui_kit::prelude::*;
use gpui_kit::Focusable;
use gpui_kit::{
    div, img, px, rgb, AnyElement, App, ClickEvent, Context, Entity, FontWeight, IntoElement,
    KeyBinding, ObjectFit, Render, SharedString, Subscription, TitlebarOptions, Window,
    WindowOptions,
};

use crate::bg::Bg;
use crate::config::{self, Config, Credentials};
use crate::editor::{Buffer as EditorBuffer, Workspace};
use crate::interop::{self, Command as InteropCommand, Shared as InteropShared, Snapshot};
use crate::leetcode::client::ProblemQuery;
use crate::leetcode::html::{self, Block, SpanStyle};
use crate::leetcode::submit::submission_policy;
use crate::leetcode::{JudgeResult, LeetCodeClient, SubmitDecision, Verdict};
use crate::model::{Difficulty, Problem, ProblemPage, ProblemStatus, QuestionDetail, UserStatus};
use crate::nvim;
use crate::theme;
use crate::ui::code_editor::{self, JudgeView, LanguageChoice};
use crate::ui::palette::{self, PaletteEntry, MAX_ROWS};
use crate::ui::problem_list::ProblemListState;
use crate::ui::statement;
use crate::ui::status_bar::AuthBadge;

gpui_kit::actions!(
    leetr,
    [
        Quit,
        ToggleTheme,
        ActionPalette,
        ProblemPalette,
        CloseOverlay,
        OpenInNvim,
        RunTests,
        SubmitSolution,
        ToggleSidebar,
        ShowStatementTab,
        ShowCodeTab,
    ]
);

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

/// Which command palette is open. [`Command`] owns the query, the filter and
/// the highlight; the view supplies ranked rows and applies choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaletteKind {
    Actions,
    Problems,
}

/// One problem row offered by the problem palette.
#[derive(Debug, Clone)]
struct ProblemRow {
    slug: String,
    label: String,
}

/// Cap on how many loaded problems the `Ctrl+P` palette will offer.
const MAX_PALETTE_PROBLEMS: usize = 1500;

/// How many problem rows render before "show more" is needed.
const ROW_PAGE: usize = 200;

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

/// One row of the `Ctrl+K` catalog: action, hint, section and shortcut.
type ActionRow = (Action, String, &'static str, Option<&'static str>);

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
fn session_line(badge: &AuthBadge) -> (String, Option<u32>) {
    match badge {
        AuthBadge::SignedIn { username, solved } => {
            let text = if username.is_empty() {
                format!("signed in · {solved} solved")
            } else {
                format!("{username} · {solved} solved")
            };
            (text, Some(theme::SUCCESS_HEX))
        }
        AuthBadge::Rejected => ("session rejected".to_string(), Some(theme::DANGER_HEX)),
        AuthBadge::Checking => ("checking session…".to_string(), None),
        AuthBadge::BrowseOnly => ("browse-only".to_string(), Some(theme::WARNING_HEX)),
    }
}

/// Highlight language for the solution editor, from a LeetCode language slug.
///
/// Only the Python and C++ grammars are compiled in, so every other language —
/// and every unknown slug — decodes as plain text. The returned name must be a
/// name the language registry knows, or the editor would treat it as garbage
/// rather than as unhighlighted code; `text` is the registry's plain-text
/// language, and a test pins that.
///
/// C is highlighted with the C++ grammar: it is a superset that parses C, and
/// the alternative is no highlighting at all for a common LeetCode language.
pub fn editor_language(lang_slug: &str) -> &'static str {
    match lang_slug {
        "python" | "python3" | "py" | "pythondata" | "pandas" => "python",
        "cpp" | "c++" | "c" => "cpp",
        _ => "text",
    }
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

/// Non-UI state assembled before the GPUI application starts.
struct Bootstrap {
    config: Config,
    credentials: Credentials,
    status: String,
    error: Option<String>,
    client: LeetCodeClient,
    workspace: Workspace,
    interop_shared: Arc<InteropShared>,
    interop: Option<interop::RunningServer>,
}

impl Bootstrap {
    fn load() -> Self {
        let mut error = None;
        if let Err(err) = config::seed() {
            error = Some(format!("could not write config files: {err}"));
        }

        let config = Config::load().unwrap_or_else(|err| {
            error = Some(format!("{err}"));
            Config::default()
        });
        let credentials = Credentials::load();

        let client = match LeetCodeClient::new(credentials.clone()) {
            Ok(client) => client,
            Err(err) => {
                error = Some(format!("could not build the HTTP client: {err}"));
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

        Self {
            config,
            credentials,
            status: "Loading problems…".to_string(),
            error,
            client,
            workspace,
            interop_shared,
            interop,
        }
    }
}

/// The root GPUI view: all former `eframe` state, rendered with GPUI Kit.
pub struct LeetrView {
    config: Config,
    credentials: Credentials,
    status: String,
    error: Option<String>,

    client: LeetCodeClient,
    bg: Bg<ApiEvent>,

    problems: Vec<Problem>,
    total: u32,
    list_state: ProblemListState,
    list_loading: bool,
    list_error: Option<String>,
    row_cap: usize,

    selected: Option<String>,
    detail: Option<QuestionDetail>,
    detail_blocks: Vec<Block>,
    detail_loading: bool,
    detail_error: Option<String>,

    user: Option<UserStatus>,
    solved: BTreeSet<String>,

    actions_cmd: Entity<CommandState>,
    problems_cmd: Entity<CommandState>,
    /// Which command palette is open, if any.
    open_palette: Option<PaletteKind>,
    /// A global shortcut asked for a palette. Opening one focuses its field,
    /// and focusing needs a `Window`, which an action listener does not get —
    /// so the request is applied on the next paint.
    pending_palette: Option<PaletteKind>,

    workspace: Workspace,
    code: Option<EditorBuffer>,
    languages: Vec<LanguageChoice>,
    tab: Tab,
    pending_code: Option<String>,
    judge: Option<JudgeView>,

    interop_shared: Arc<InteropShared>,
    interop: Option<interop::RunningServer>,

    search_input: Entity<InputState>,
    code_input: Entity<EditorState>,
    /// LeetCode slug the solution editor is highlighting as.
    editor_lang_slug: String,
    /// Language requested without a window (control API); applied on next paint.
    pending_lang: Option<String>,
    _subscriptions: Vec<Subscription>,
    /// Text waiting to be pushed into the solution field on the next paint.
    code_input_needs_sync: Option<String>,
    /// Set by the Quit action row; the render loop turns it into `cx.quit()`.
    quit_requested: bool,
    /// The problem sidebar is hidden.
    sidebar_collapsed: bool,
}

impl LeetrView {
    fn new(
        bootstrap: Bootstrap,
        search_input: Entity<InputState>,
        actions_cmd: Entity<CommandState>,
        problems_cmd: Entity<CommandState>,
        code_input: Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Keep the search field and the filter model in sync.
        let search_sub = cx.subscribe(
            &search_input,
            |this: &mut Self, emitter, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.list_state.search = emitter.read(cx).value().to_string();
                    this.row_cap = ROW_PAGE;
                    cx.notify();
                }
            },
        );
        // Edits in the solution editor flow into the workspace file, which is
        // the source of truth shared with nvim.
        let code_sub = cx.subscribe(
            &code_input,
            |this: &mut Self, emitter, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = emitter.read(cx).value().to_string();
                    if let Some(buffer) = this.code.as_mut() {
                        buffer.set_text(text);
                        if let Err(err) = buffer.save() {
                            this.error = Some(format!("could not save the solution: {err}"));
                        }
                    }
                    cx.notify();
                }
            },
        );

        // Background threads post into `bg`; this timer applies their results
        // on the UI thread, replacing the old per-frame drain.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let Some(view) = this.upgrade() else {
                break;
            };
            view.update(cx, |view, cx| {
                view.drain_events();
                view.apply_interop_commands();
                view.poll_workspace_file();
                view.publish_snapshot();
                cx.notify();
            });
        })
        .detach();

        let default_lang = bootstrap.config.default_language.clone();
        let mut view = Self {
            config: bootstrap.config,
            credentials: bootstrap.credentials,
            status: bootstrap.status,
            error: bootstrap.error,
            client: bootstrap.client,
            bg: Bg::new(),
            problems: Vec::new(),
            total: 0,
            list_state: ProblemListState::default(),
            list_loading: false,
            list_error: None,
            row_cap: ROW_PAGE,
            selected: None,
            detail: None,
            detail_blocks: Vec::new(),
            detail_loading: false,
            detail_error: None,
            user: None,
            solved: BTreeSet::new(),
            actions_cmd,
            problems_cmd,
            open_palette: None,
            pending_palette: None,
            workspace: bootstrap.workspace,
            code: None,
            languages: Vec::new(),
            tab: Tab::Statement,
            pending_code: None,
            judge: None,
            interop_shared: bootstrap.interop_shared,
            interop: bootstrap.interop,
            search_input,
            code_input,
            editor_lang_slug: default_lang,
            pending_lang: None,
            _subscriptions: vec![search_sub, code_sub],
            code_input_needs_sync: None,
            quit_requested: false,
            sidebar_collapsed: false,
        };

        view.refresh_session();
        view.load_first_page();
        view
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

    fn spawn_problems(&mut self, skip: u32, replace: bool) {
        self.list_loading = true;
        self.list_error = None;

        let client = self.client.clone();
        let limit = self.config.problem_list_page_size.clamp(1, 100);
        let status_filter = if replace {
            self.list_state.status
        } else {
            None
        };
        self.bg.spawn(move || {
            let result = client
                .fetch_problems(&ProblemQuery {
                    skip,
                    limit,
                    status: status_filter,
                    search: None,
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
        self.tab = Tab::Statement;
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
                            self.status = format!("{} · {}", detail.frontend_id, detail.title);
                            self.detail = Some(*detail);
                            self.detail_error = None;
                            if let Some(detail) = self.detail.clone() {
                                self.prepare_workspace(&detail, &lang);
                            }
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

    /// Recreate the solution editor for `lang_slug`, seeded with `text`.
    ///
    /// The component editor takes its language once at construction, so a
    /// switch builds a fresh state (the workspace file stays the source of
    /// truth, and `set_value` emits no change event, so this never loops).
    fn reset_editor(
        &mut self,
        lang_slug: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let language = editor_language(lang_slug);
        let seed = text.to_string();
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .default_value(seed)
        });
        let sub = cx.subscribe(
            &state,
            |this: &mut Self, emitter, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = emitter.read(cx).value().to_string();
                    if let Some(buffer) = this.code.as_mut() {
                        buffer.set_text(text);
                        if let Err(err) = buffer.save() {
                            this.error = Some(format!("could not save the solution: {err}"));
                        }
                    }
                    cx.notify();
                }
            },
        );
        self.code_input = state;
        if let Some(slot) = self._subscriptions.get_mut(1) {
            *slot = sub;
        } else {
            self._subscriptions.push(sub);
        }
        self.editor_lang_slug = lang_slug.to_string();
        self.pending_lang = None;
    }

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
                // `set_value` needs a `Window`, which this tick does not
                // have; the code pane pushes the text (and language) on its
                // next paint.
                if self.editor_lang_slug != buffer.lang_slug {
                    self.pending_lang = Some(buffer.lang_slug.clone());
                }
                self.code_input_needs_sync = Some(buffer.text.clone());
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
                if self.editor_lang_slug != buffer.lang_slug {
                    self.pending_lang = Some(buffer.lang_slug.clone());
                }
                self.code_input_needs_sync = Some(buffer.text.clone());
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
                self.code_input_needs_sync = Some(buffer.text.clone());
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
                                    self.code_input_needs_sync = Some(buffer.text.clone());
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
    /// Called on every timer tick: the payload is a LeetCode-sized solution, so
    /// the clone is cheap and it removes a whole class of staleness bugs.
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

    // -- palettes ------------------------------------------------------------

    /// Which command palette is open. [`Command`] owns the query, the filter
    /// and the highlight; the view supplies ranked rows and applies choices.
    fn open_actions_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette = Some(PaletteKind::Actions);
        self.actions_cmd.update(cx, |state, cx| {
            state.set_query("", window, cx);
        });
        let handle = self.actions_cmd.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn open_problems_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette = Some(PaletteKind::Problems);
        self.problems_cmd.update(cx, |state, cx| {
            state.set_query("", window, cx);
        });
        let handle = self.problems_cmd.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    fn close_palette(&mut self) {
        self.open_palette = None;
    }

    /// One confirmed command-palette row.
    fn confirm_palette(
        &mut self,
        kind: PaletteKind,
        path: IndexPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match kind {
            PaletteKind::Actions => {
                let query = self.actions_cmd.read(cx).query(cx).to_string();
                let groups = self.action_groups(&query);
                let action = groups
                    .get(path.section)
                    .and_then(|(_, rows)| rows.get(path.row))
                    .map(|(action, _)| *action);
                if let Some(action) = action {
                    self.open_palette = None;
                    match action {
                        Action::SelectProblem => self.open_problems_palette(window, cx),
                        other => self.run_action(other, cx),
                    }
                }
            }
            PaletteKind::Problems => {
                let query = self.problems_cmd.read(cx).query(cx).to_string();
                let groups = self.problem_rows(&query);
                let slug = groups
                    .get(path.section)
                    .and_then(|(_, rows)| rows.get(path.row))
                    .map(|row| row.slug.clone());
                if let Some(slug) = slug {
                    self.open_palette = None;
                    self.open_problem(slug);
                }
            }
        }
        cx.notify();
    }

    fn action_catalog(&self) -> Vec<ActionRow> {
        let mut rows = vec![
            (
                Action::ToggleTheme,
                format!("Switch to {} mode", self.config.theme.toggled().label()),
                "Theme",
                None,
            ),
            (
                Action::SelectProblem,
                "Fuzzy jump to any loaded problem".to_string(),
                "Go",
                Some("Ctrl P"),
            ),
            (
                Action::ShowProblems,
                "Back to the full problem list".to_string(),
                "Go",
                None,
            ),
            (
                Action::RefreshProblems,
                "Fetch the first page again".to_string(),
                "Problems",
                None,
            ),
            (
                Action::LoadMore,
                "Append the next page".to_string(),
                "Problems",
                None,
            ),
            (
                Action::ClearFilters,
                "Reset search, status and difficulty".to_string(),
                "Problems",
                None,
            ),
            (
                Action::TogglePremiumFilter,
                "Hide or show premium-only problems".to_string(),
                "Problems",
                None,
            ),
        ];
        if self.selected.is_some() {
            rows.push((
                Action::OpenOnLeetCode,
                "Open the current problem in a browser".to_string(),
                "Problem",
                None,
            ));
            rows.push((
                Action::CopyProblemLink,
                "Copy the problem URL".to_string(),
                "Problem",
                None,
            ));
        }
        rows.push((Action::Quit, "Close the window".to_string(), "App", None));
        rows
    }

    /// Action rows grouped by section, in catalog order.
    /// Action rows matching `query`, grouped by section for browsing and flat
    /// when ranked.
    ///
    /// The palette runs with the component's own filtering off (`.filterable(false)`)
    /// so that ranking stays fuzzy — which means the rows have to be filtered
    /// here, on every render, or typing in the field would do nothing.
    fn action_groups(&self, query: &str) -> Vec<(String, Vec<(Action, String)>)> {
        let catalog = self.action_catalog();
        let entries: Vec<PaletteEntry> = catalog
            .iter()
            .map(|(action, hint, section, _)| {
                PaletteEntry::new(action.search_label())
                    .with_hint(hint.clone())
                    .with_section(*section)
            })
            .collect();
        let matches = palette::visible_matches(query, &entries, None);

        if query.trim().is_empty() {
            let mut groups: Vec<(String, Vec<(Action, String)>)> = Vec::new();
            for index in matches {
                let (action, hint, section, _) = &catalog[index];
                match groups.last_mut() {
                    Some((name, rows)) if name == *section => rows.push((*action, hint.clone())),
                    _ => groups.push((section.to_string(), vec![(*action, hint.clone())])),
                }
            }
            groups
        } else {
            let rows = matches
                .into_iter()
                .map(|index| {
                    let (action, hint, _, _) = &catalog[index];
                    (*action, hint.clone())
                })
                .collect();
            vec![("Commands".to_string(), rows)]
        }
    }

    /// Problem rows matching `query`/`chip`, grouped by status for browsing
    /// and flat when ranked. Mirrors the palette's grouped-browse contract.
    fn problem_rows(&self, query: &str) -> Vec<(String, Vec<ProblemRow>)> {
        let mut entries = Vec::new();
        let mut slugs = Vec::new();
        for problem in self.problems.iter().take(MAX_PALETTE_PROBLEMS) {
            let section = match problem.status {
                ProblemStatus::Solved => "Solved",
                ProblemStatus::Attempted => "Attempted",
                ProblemStatus::Todo => "Todo",
            };
            entries.push(
                PaletteEntry::new(format!("{}. {}", problem.id, problem.title))
                    .with_hint(format!(
                        "{} · {}",
                        problem.difficulty.label(),
                        problem.status.label()
                    ))
                    .with_section(section)
                    .with_tags([
                        problem.difficulty.label().to_lowercase(),
                        problem.status.label().to_lowercase(),
                    ]),
            );
            slugs.push(problem.slug.clone());
        }
        let matches: Vec<usize> = palette::visible_matches(query, &entries, None)
            .into_iter()
            .take(MAX_ROWS)
            .collect();
        if query.trim().is_empty() {
            // Cluster by section in first-appearance order; the match order
            // within a section is preserved.
            let mut groups: Vec<(String, Vec<ProblemRow>)> = Vec::new();
            for index in matches {
                let section = entries[index].section.clone().unwrap_or_default();
                let row = ProblemRow {
                    slug: slugs[index].clone(),
                    label: entries[index].label.clone(),
                };
                match groups.last_mut() {
                    Some((name, rows)) if *name == section => rows.push(row),
                    _ => groups.push((section, vec![row])),
                }
            }
            groups
        } else {
            vec![(
                "Problems".to_string(),
                matches
                    .into_iter()
                    .map(|index| ProblemRow {
                        slug: slugs[index].clone(),
                        label: entries[index].label.clone(),
                    })
                    .collect(),
            )]
        }
    }

    fn run_action(&mut self, action: Action, cx: &mut App) {
        match action {
            Action::ToggleTheme => self.toggle_theme(cx),
            // The problem selector opens the other palette, which needs a
            // window; the confirm handler does that directly and never lands
            // here.
            Action::SelectProblem => {
                self.status = "use Ctrl+P for the problem selector".to_string();
            }
            Action::ShowProblems => self.close_problem(),
            Action::RefreshProblems => self.load_first_page(),
            Action::LoadMore => {
                self.row_cap = self.row_cap.saturating_add(ROW_PAGE);
                self.load_next_page();
            }
            Action::ClearFilters => self.list_state.clear_filters(),
            Action::TogglePremiumFilter => {
                self.list_state.hide_premium = !self.list_state.hide_premium;
            }
            Action::OpenOnLeetCode => {
                if let Some(slug) = self.selected.clone() {
                    let url = problem_url(&slug);
                    if let Err(err) = open_in_browser(&url) {
                        self.error = Some(err);
                    } else {
                        self.status = format!("opened {url}");
                    }
                }
            }
            Action::CopyProblemLink => {
                if let Some(slug) = self.selected.as_deref() {
                    self.status = format!("link: {}", problem_url(slug));
                }
            }
            Action::Quit => {
                // Handled via the Quit action binding; kept here for symmetry.
                self.quit_requested = true;
            }
        }
    }

    fn toggle_theme(&mut self, cx: &mut App) {
        let next = self.config.theme.toggled();
        self.config.theme = next;
        KitTheme::change(next.mode(), None, cx);
        if let Err(err) = self.config.save() {
            self.error = Some(format!("could not save the theme: {err}"));
        } else {
            self.status = format!("{} mode", next.label());
        }
    }
}

// -- rendering -----------------------------------------------------------

const MUTED_DARK: u32 = 0x979EA9;
const BORDER_DARK: u32 = 0x2D3239;

impl LeetrView {
    /// One navigable row in the sidebar.
    ///
    /// Kept as a small builder rather than inline closures so the same item
    /// shape (label, icon, active state, trailing hint) is used everywhere.
    fn sidebar_item(
        view: &Entity<Self>,
        label: String,
        icon: Icon,
        active: bool,
        suffix: Option<String>,
        on_click: impl Fn(&mut Self) + 'static + Clone,
    ) -> SidebarMenuItem {
        let view = view.clone();
        let mut item = SidebarMenuItem::new(label)
            .icon(icon)
            .active(active)
            .on_click(move |_, window, cx| {
                view.update(cx, |this, _| on_click(this));
                window.refresh();
            });
        if let Some(text) = suffix {
            item = item.suffix(move |_, _| {
                div()
                    .text_xs()
                    .text_color(rgb(MUTED_DARK))
                    .child(text.clone())
            });
        }
        item
    }

    /// Status icon for a problem row, coloured by difficulty.
    fn status_icon(status: ProblemStatus, difficulty: Difficulty) -> Icon {
        let name = match status {
            ProblemStatus::Solved => IconName::CircleCheck,
            ProblemStatus::Attempted => IconName::CircleDot,
            ProblemStatus::Todo => IconName::Circle,
        };
        Icon::new(name)
            .size_4()
            .text_color(rgb(theme::difficulty_hex(difficulty)))
    }

    fn render_sidebar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let matching = self.list_state.matching(&self.problems);
        let shown = matching.len().min(self.row_cap);
        let view = cx.entity();

        // ---- header: just the filter field ----
        //
        // The brand, the session line, the theme button and the quick-actions
        // button used to live here. The sidebar is a list now: identity and
        // theme are in the status bar and the command palette.
        let header = div()
            .flex()
            .flex_col()
            .w_full()
            .gap_2()
            .child(Input::new(&self.search_input).id("problem-search"));

        // ---- "Filter" group: status, level, premium ----
        let status = self.list_state.status;
        let difficulty = self.list_state.difficulty;
        let mut filters = vec![
            Self::sidebar_item(
                &view,
                "All problems".to_string(),
                Icon::new(IconName::ListTodo).size_4(),
                status.is_none(),
                Some(format!("{}", self.total)),
                |this| {
                    this.list_state.status = None;
                    this.row_cap = ROW_PAGE;
                },
            ),
            Self::sidebar_item(
                &view,
                "Solved".to_string(),
                Icon::new(IconName::CircleCheck)
                    .size_4()
                    .text_color(rgb(theme::SUCCESS_HEX)),
                status == Some(ProblemStatus::Solved),
                None,
                |this| {
                    this.list_state.status = Some(ProblemStatus::Solved);
                    this.row_cap = ROW_PAGE;
                },
            ),
            Self::sidebar_item(
                &view,
                "Attempted".to_string(),
                Icon::new(IconName::CircleDot)
                    .size_4()
                    .text_color(rgb(theme::WARNING_HEX)),
                status == Some(ProblemStatus::Attempted),
                None,
                |this| {
                    this.list_state.status = Some(ProblemStatus::Attempted);
                    this.row_cap = ROW_PAGE;
                },
            ),
            Self::sidebar_item(
                &view,
                "Todo".to_string(),
                Icon::new(IconName::Circle)
                    .size_4()
                    .text_color(rgb(MUTED_DARK)),
                status == Some(ProblemStatus::Todo),
                None,
                |this| {
                    this.list_state.status = Some(ProblemStatus::Todo);
                    this.row_cap = ROW_PAGE;
                },
            ),
        ];

        // Level is a submenu: the parent shows the active level, the children
        // pick one — the component's own collapsible-item pattern.
        let level_label = match difficulty {
            None => "All levels".to_string(),
            Some(level) => level.label().to_string(),
        };
        filters.push(
            SidebarMenuItem::new("Level")
                .click_to_toggle(true)
                .icon(Icon::new(IconName::Gauge).size_4())
                .suffix(move |_, _| {
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED_DARK))
                        .child(level_label.clone())
                })
                .children(
                    [
                        None,
                        Some(Difficulty::Easy),
                        Some(Difficulty::Medium),
                        Some(Difficulty::Hard),
                    ]
                    .into_iter()
                    .map(|level| {
                        let label = match level {
                            None => "All levels".to_string(),
                            Some(level) => level.label().to_string(),
                        };
                        let hex = level.map_or(MUTED_DARK, theme::difficulty_hex);
                        Self::sidebar_item(
                            &view,
                            label,
                            Icon::new(IconName::Circle).size_3().text_color(rgb(hex)),
                            difficulty == level,
                            None,
                            move |this| {
                                this.list_state.difficulty = level;
                                this.row_cap = ROW_PAGE;
                            },
                        )
                    })
                    .collect::<Vec<_>>(),
                ),
        );

        let premium = self.list_state.hide_premium;
        filters.push(Self::sidebar_item(
            &view,
            "Premium only".to_string(),
            Icon::new(IconName::Star)
                .size_4()
                .text_color(rgb(theme::ACCENT_HEX)),
            premium,
            None,
            move |this| {
                this.list_state.hide_premium = !this.list_state.hide_premium;
                this.row_cap = ROW_PAGE;
            },
        ));

        // ---- "Problems" group ----
        let mut problem_items = Vec::new();
        for &index in matching.iter().take(shown) {
            let problem = &self.problems[index];
            let slug = problem.slug.clone();
            let active = self.selected.as_deref() == Some(slug.as_str());
            let label = format!("{}. {}", problem.id, problem.title);
            let icon = Self::status_icon(problem.status, problem.difficulty);
            let item_view = view.clone();
            // The dot's colour already says easy/medium/hard, so the row
            // carries no difficulty text.
            problem_items.push(
                SidebarMenuItem::new(label)
                    .icon(icon)
                    .active(active)
                    .on_click(move |_, window, cx| {
                        let slug = slug.clone();
                        item_view.update(cx, |this, _| this.open_problem(slug));
                        window.refresh();
                    }),
            );
        }
        let remaining = matching.len().saturating_sub(shown);
        if remaining > 0 {
            problem_items.push(Self::sidebar_item(
                &view,
                format!("Show {remaining} more"),
                Icon::new(IconName::ChevronsDown)
                    .size_4()
                    .text_color(rgb(MUTED_DARK)),
                false,
                None,
                |this| {
                    this.row_cap = this.row_cap.saturating_add(ROW_PAGE);
                    this.load_next_page();
                },
            ));
        }

        // ---- "Current problem" group ----
        let mut current_items = Vec::new();
        if let Some(slug) = self.selected.clone() {
            let running = self.judge.as_ref().is_some_and(|judge| judge.running);
            let has_code = self.code.is_some();
            if has_code {
                current_items.push(Self::sidebar_item(
                    &view,
                    "Run sample tests".to_string(),
                    Icon::new(IconName::Zap).size_4(),
                    false,
                    Some("Ctrl '".to_string()),
                    move |this| this.start_judge(false),
                ));
                current_items.push(Self::sidebar_item(
                    &view,
                    "Submit solution".to_string(),
                    Icon::new(if running {
                        IconName::LoaderCircle
                    } else {
                        IconName::CircleCheckBig
                    })
                    .size_4(),
                    false,
                    Some("Ctrl ↵".to_string()),
                    move |this| this.start_judge(true),
                ));
                current_items.push(Self::sidebar_item(
                    &view,
                    "Open in nvim".to_string(),
                    Icon::new(IconName::SquareCode).size_4(),
                    false,
                    Some("Ctrl O".to_string()),
                    move |this| this.open_in_nvim(),
                ));
            }
            current_items.push(Self::sidebar_item(
                &view,
                "Open on leetcode.com".to_string(),
                Icon::new(IconName::ExternalLink).size_4(),
                false,
                None,
                move |this| {
                    let url = problem_url(&slug);
                    match open_in_browser(&url) {
                        Ok(()) => this.status = format!("opened {url}"),
                        Err(err) => this.error = Some(err),
                    }
                },
            ));
        }

        // ---- footer: counts ----
        let footer = if self.list_loading {
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(Spinner::new())
                .child(div().text_xs().child("Loading…"))
        } else {
            let line = if self.list_state.is_filtered() {
                format!("{shown} matching · {} loaded", self.problems.len())
            } else {
                format!("{shown} shown · {} total", self.total)
            };
            div().text_xs().text_color(rgb(MUTED_DARK)).child(line)
        };

        // ---- "App" group ----
        let app_view = view.clone();
        let app_items = vec![
            Self::sidebar_item(
                &app_view,
                "Reload problem list".to_string(),
                Icon::new(IconName::RefreshCw).size_4(),
                false,
                None,
                move |this| this.load_first_page(),
            ),
            Self::sidebar_item(
                &app_view,
                "Clear filters".to_string(),
                Icon::new(IconName::ListFilter).size_4(),
                false,
                None,
                move |this| {
                    this.list_state.clear_filters();
                    this.row_cap = ROW_PAGE;
                },
            ),
        ];

        let mut sidebar = Sidebar::new("leetr-sidebar")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(self.sidebar_collapsed)
            .header(header)
            .child(SidebarGroup::new("Filter").children(filters));
        if !problem_items.is_empty() {
            sidebar = sidebar.child(SidebarGroup::new("Problems").children(problem_items));
        }
        if !current_items.is_empty() {
            sidebar = sidebar.child(SidebarGroup::new("Current problem").children(current_items));
        }
        sidebar = sidebar.child(SidebarGroup::new("App").children(app_items));
        sidebar.footer(footer).into_any_element()
    }

    fn render_main(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(detail) = self.detail.clone() else {
            let message = if self.detail_loading {
                "Loading statement…"
            } else if let Some(err) = self.detail_error.as_deref() {
                let _ = err;
                "Could not load the statement."
            } else {
                statement::empty_message(false)
            };
            return div()
                .flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .child(div().text_sm().text_color(rgb(MUTED_DARK)).child(message))
                .into_any_element();
        };

        // The collapse control is the tab bar's prefix rather than a sibling, so
        // it sits inside the bar's background and stays reachable with the
        // sidebar hidden. It is a plain button rather than
        // `SidebarToggleButton` so its surface can be set explicitly: in dark
        // mode it reads as a black tile against the bar.
        let toggle_view = cx.entity();
        let toggle_icon = if self.sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };
        let toggle_surface = if cx.theme().is_dark() {
            rgb(0x000000).into()
        } else {
            gpui_kit::transparent_black()
        };
        let sidebar_toggle = Button::new("sidebar-toggle")
            .ghost()
            .small()
            .bg(toggle_surface)
            .icon(Icon::new(toggle_icon).size_4())
            .accessibility_label("Toggle sidebar")
            .on_click(move |_, window, cx| {
                toggle_view.update(cx, |this, _| {
                    this.sidebar_collapsed = !this.sidebar_collapsed;
                });
                window.refresh();
            });

        let tabs = TabBar::new("leetr-tabs")
            .w_full()
            .py_1()
            .prefix(sidebar_toggle)
            .selected_index(match self.tab {
                Tab::Statement => 0,
                Tab::Code => 1,
            })
            .child(
                KitTab::new()
                    .label("Statement")
                    .selected(self.tab == Tab::Statement)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.tab = Tab::Statement;
                        cx.notify();
                    })),
            )
            .child(
                KitTab::new()
                    .label("Code")
                    .selected(self.tab == Tab::Code)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.tab = Tab::Code;
                        cx.notify();
                    })),
            );

        let tab = self.tab;
        let pane = match tab {
            Tab::Statement => self.render_statement(&detail),
            Tab::Code => self.render_code(cx),
        };

        // The tab bar is flush with the pane's edges — no outer padding or
        // gap — so its background spans the full width. The padding belongs to
        // the content underneath it.
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(tabs)
            .child(div().flex().flex_col().flex_1().p_3().gap_2().child(pane))
            .into_any_element()
    }

    fn render_statement(&self, detail: &QuestionDetail) -> AnyElement {
        let header = div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::BOLD)
                    .child(detail.title.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(theme::difficulty_hex(detail.difficulty)))
                            .child(detail.difficulty.label().to_string()),
                    )
                    .children(detail.tags.iter().map(|tag| {
                        div()
                            .text_xs()
                            .text_color(rgb(MUTED_DARK))
                            .child(tag.name.clone())
                    })),
            );

        let mut body = div()
            .flex()
            .flex_col()
            .id("statement-scroll")
            .overflow_y_scroll()
            .flex_1()
            .gap_2()
            .child(header);
        if self.detail_loading {
            body = body.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(Spinner::new())
                    .child(div().text_sm().child("Loading statement…")),
            );
        } else if detail.content_html.trim().is_empty() {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(rgb(theme::WARNING_HEX))
                    .child(statement::missing_body_message(detail).to_string()),
            );
        } else {
            for (index, block) in self.detail_blocks.iter().enumerate() {
                body = body.child(render_block(block, index));
            }
            if !detail.hints.is_empty() {
                body = body.child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child(format!("Hints ({})", detail.hints.len())),
                );
                for (index, hint) in detail.hints.iter().enumerate() {
                    let text = html::to_plain_text(&html::parse(hint));
                    body = body.child(div().text_sm().child(format!("{}. {text}", index + 1)));
                }
            }
        }
        body.into_any_element()
    }

    fn render_code(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(buffer) = self.code.clone() else {
            return div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(MUTED_DARK))
                        .child("Open a problem to get a solution buffer."),
                )
                .into_any_element();
        };

        // The problem's snippet list can run to a dozen languages, which as a
        // row of buttons wrapped onto extra lines. It is a dropdown now.
        let current = self
            .languages
            .iter()
            .find(|choice| choice.slug == buffer.lang_slug)
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| buffer.lang_slug.clone());
        let choices = self.languages.clone();
        let menu_view = cx.entity();
        let language_picker = DropdownButton::new("language-picker")
            .button(
                Button::new("language-picker-button")
                    .label(current)
                    .dropdown_caret(true),
            )
            .dropdown_menu(move |menu, _window, _cx| {
                let mut menu = menu;
                for choice in &choices {
                    let slug = choice.slug.clone();
                    let selected = slug == choice.slug;
                    let view = menu_view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(choice.label.clone())
                            .checked(selected)
                            .on_click(move |_, _window, cx| {
                                let slug = slug.clone();
                                view.update(cx, |this, _| this.switch_language(slug));
                            }),
                    );
                }
                menu
            });

        let lang_row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(language_picker);

        let judge = match self.judge.clone() {
            None => div()
                .text_xs()
                .text_color(rgb(MUTED_DARK))
                .child("Ctrl+' runs the sample tests · Ctrl+Enter submits · Ctrl+O opens nvim")
                .into_any_element(),
            Some(view) => render_judge(&view),
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .gap_2()
            .child(lang_row)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .flex_col()
                    .id("solution-editor")
                    .overflow_hidden()
                    .child(Editor::new(&self.code_input).size_full()),
            )
            .child(judge)
            .into_any_element()
    }

    fn render_status_bar(&self) -> AnyElement {
        let (session_text, session_color) = session_line(&self.auth_badge());
        let badge_text = format!("{} {session_text}", crate::glyphs::DOT);
        let badge_color = session_color.unwrap_or(MUTED_DARK);
        let message = match self.error.as_deref() {
            Some(err) => err.to_string(),
            None => self.status.clone(),
        };
        let message_color = match self.error {
            Some(_) => theme::DANGER_HEX,
            None => MUTED_DARK,
        };
        let port = self.interop.as_ref().map(|s| s.port()).unwrap_or(0);

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .px_3()
            .py_1()
            .border_t_1()
            .border_color(rgb(BORDER_DARK))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(badge_color))
                            .child(badge_text),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(message_color))
                            .child(message),
                    ),
            )
            .child(div().text_xs().text_color(rgb(MUTED_DARK)).child(format!(
                "{} · lang {} · api :{port}",
                self.config.theme.label(),
                self.config.default_language,
            )))
            .into_any_element()
    }

    fn render_palette(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(kind) = self.open_palette else {
            return div().into_any_element();
        };
        // Reading the query establishes the host's dependency on the command
        // state, so typing re-renders the overlay with freshly ranked rows.
        let view = cx.entity();
        let overlay = match kind {
            PaletteKind::Actions => {
                let cancel_view = view.clone();
                let confirm_view = view.clone();
                let query = self.actions_cmd.read(cx).query(cx).to_string();
                let groups = self.action_groups(&query);
                let mut command = Command::new(&self.actions_cmd)
                    .filterable(false)
                    .placeholder("Type a command…")
                    .footer(|_, _, _| palette_footer("↑↓ navigate · Enter run · Esc close"))
                    .on_cancel(move |window, cx| {
                        cancel_view.update(cx, |this, _| {
                            this.close_palette();
                        });
                        window.refresh();
                    });
                command = command.on_confirm(move |path, window, cx| {
                    confirm_view.update(cx, |this, ctx| {
                        this.confirm_palette(PaletteKind::Actions, path, window, ctx);
                    });
                    window.refresh();
                });
                for (section, rows) in groups {
                    let mut group = CommandGroup::new().label(section);
                    for (action, _) in rows {
                        group = group.item(CommandItem::new().label(action.search_label()));
                    }
                    command = command.group(group);
                }
                command.into_any_element()
            }
            PaletteKind::Problems => {
                let query = self.problems_cmd.read(cx).query(cx).to_string();
                let groups = self.problem_rows(&query);
                let supplied: usize = groups.iter().map(|(_, rows)| rows.len()).sum();
                let cancel_view = view.clone();
                let confirm_view = view.clone();
                let mut command = Command::new(&self.problems_cmd)
                    .filterable(false)
                    .placeholder("Jump to a problem…")
                    .footer(|_, _, _| palette_footer("↑↓ navigate · Enter open · Esc close"))
                    .on_cancel(move |window, cx| {
                        cancel_view.update(cx, |this, _| {
                            this.close_palette();
                        });
                        window.refresh();
                    });
                command = command.on_confirm(move |path, window, cx| {
                    confirm_view.update(cx, |this, ctx| {
                        this.confirm_palette(PaletteKind::Problems, path, window, ctx);
                    });
                    window.refresh();
                });
                for (section, rows) in &groups {
                    let mut group = CommandGroup::new().label(section.clone());
                    for row in rows {
                        group = group.item(CommandItem::new().label(row.label.clone()));
                    }
                    command = command.group(group);
                }

                // Status and difficulty are filtered from the sidebar; the
                // palette is only a jump-to, so it carries no filter chips.
                let mut panel = div().flex().flex_col().gap_1().child(command);
                if supplied == 0 && !query.trim().is_empty() {
                    let server_query = query.clone();
                    panel = panel.child(
                        Button::new("palette-server-search")
                            .label(format!("Search LeetCode for “{query}”"))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.close_palette();
                                this.search_problems(server_query.clone());
                                cx.notify();
                            })),
                    );
                }
                panel.into_any_element()
            }
        };

        div()
            .absolute()
            .top(px(76.))
            .left(px(0.))
            .right(px(0.))
            .flex()
            .justify_center()
            .child(div().flex().flex_col().w(px(660.)).child(overlay))
            .into_any_element()
    }
}

fn render_block(block: &Block, index: usize) -> AnyElement {
    match block {
        Block::Paragraph(spans) => render_spans(spans, false),
        Block::Heading(spans) => div()
            .text_lg()
            .font_weight(FontWeight::BOLD)
            .child(render_spans(spans, false))
            .into_any_element(),
        Block::ListItem(spans) => div()
            .flex()
            .flex_row()
            .gap_2()
            .child(div().text_sm().child("•"))
            .child(render_spans(spans, false))
            .into_any_element(),
        Block::Code(code) => div()
            .w_full()
            .rounded_md()
            .bg(rgb(0x121417))
            .p_2()
            .child(div().text_xs().child(code.trim_end().to_string()))
            .into_any_element(),
        Block::Image { alt, url } => render_image(alt, url, index),
        Block::Rule => div()
            .h_px()
            .w_full()
            .bg(rgb(BORDER_DARK))
            .into_any_element(),
    }
    .into_any_element()
}

/// A statement diagram.
///
/// Fetched through the app's [`gpui_kit::http_client::HttpClient`] (see
/// [`crate::img`]) and cached by GPUI. The `.id(..)` is required for the loading
/// state to appear at all.
///
/// A failure shows the URL rather than nothing, so a diagram the prose refers to
/// is never silently missing — the guarantee the old placeholder text provided.
/// The dimming layer behind an open palette.
///
/// Heavier than the library's dialog scrim (`theme().overlay`, 5% light / 20%
/// dark): a dialog covers most of the window, while a palette is a panel in the
/// middle of it, so the dimming has to read as one at a glance. Kept in the
/// theme's terms (black, theme-dependent strength) so light mode is not turned
/// into a dark room.
fn palette_scrim(dark: bool) -> gpui_kit::Hsla {
    gpui_kit::hsla(0., 0., 0., if dark { 0.5 } else { 0.35 })
}

/// The key-hint strip at the bottom of a palette.
///
/// Padded beyond the library default so the hints do not sit flush against the
/// list above them or the panel edge below.
fn palette_footer(hints: &'static str) -> AnyElement {
    div()
        .w_full()
        .px_3()
        .py_3()
        .text_xs()
        .text_color(rgb(MUTED_DARK))
        .child(hints)
        .into_any_element()
}

fn render_image(alt: &str, url: &str, index: usize) -> AnyElement {
    let caption = alt.trim().to_string();
    let fallback_url = url.to_string();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            img(SharedString::from(url.to_string()))
                .id(SharedString::from(format!("statement-image-{index}")))
                .w_full()
                .max_h(px(420.))
                .object_fit(ObjectFit::Contain)
                .with_loading(|| {
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED_DARK))
                        .child("Loading image…")
                        .into_any_element()
                })
                .with_fallback(move || {
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(theme::WARNING_HEX))
                                .child("Could not load this image"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(MUTED_DARK))
                                .child(fallback_url.clone()),
                        )
                        .into_any_element()
                }),
        )
        .when(!caption.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(rgb(MUTED_DARK))
                    .child(caption.clone()),
            )
        })
        .into_any_element()
}

fn render_spans(spans: &[crate::leetcode::html::Span], _heading: bool) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .children(
            spans
                .iter()
                .map(|span| {
                    let mut text = div().text_sm().child(span.text.clone());
                    match span.style {
                        SpanStyle::Bold | SpanStyle::BoldItalic => {
                            text = text.font_weight(FontWeight::BOLD);
                        }
                        SpanStyle::Code => {
                            text = text.text_color(rgb(theme::ACCENT_HEX)).text_xs();
                        }
                        SpanStyle::Normal | SpanStyle::Italic => {}
                    }
                    text
                })
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

fn render_judge(view: &JudgeView) -> AnyElement {
    let color = crate::ui::code_editor::verdict_hex(view.verdict);
    let mut panel = div().flex().flex_col().gap_1();
    if view.running {
        panel = panel.child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(Spinner::new())
                .child(div().text_sm().child(view.summary.clone())),
        );
        return panel.into_any_element();
    }
    panel = panel.child(
        div()
            .text_sm()
            .font_weight(FontWeight::BOLD)
            .text_color(rgb(color))
            .child(view.summary.clone()),
    );
    if let Some(warning) = view.warning.as_deref() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(theme::WARNING_HEX))
                .child(warning.to_string()),
        );
    }
    if let Some(error) = view.error.as_deref() {
        panel = panel.child(
            div()
                .text_xs()
                .text_color(rgb(theme::DANGER_HEX))
                .child(error.to_string()),
        );
    }
    for (key, value) in &view.detail {
        panel = panel.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(MUTED_DARK))
                        .child(key.clone()),
                )
                .child(div().text_xs().child(value.clone())),
        );
    }
    panel.into_any_element()
}

impl Render for LeetrView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Deferred work that needs a window: quitting, and pushing workspace
        // text (and language) into the solution editor.
        if self.quit_requested {
            cx.quit();
        }
        if let Some(lang) = self.pending_lang.clone() {
            let text = self
                .code
                .as_ref()
                .map(|buffer| buffer.text.clone())
                .or(self.code_input_needs_sync.clone())
                .unwrap_or_default();
            self.code_input_needs_sync = None;
            self.reset_editor(&lang, &text, window, cx);
        } else if let Some(text) = self.code_input_needs_sync.take() {
            self.code_input.update(cx, |state, cx| {
                state.set_value(text, window, cx);
            });
        }

        // A global shortcut asked for a palette; focusing its field needs the
        // window, so it is applied here.
        if let Some(kind) = self.pending_palette.take() {
            match kind {
                PaletteKind::Actions => self.open_actions_palette(window, cx),
                PaletteKind::Problems => self.open_problems_palette(window, cx),
            }
        }

        let overlay_open = self.open_palette.is_some();
        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .when(!self.sidebar_collapsed, |row| {
                        row.child(self.render_sidebar(cx))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .h_full()
                            .flex_col()
                            .child(self.render_main(cx)),
                    ),
            )
            .child(self.render_status_bar())
            .when(overlay_open, |root| {
                // Dim the window behind the palette and swallow the click that
                // dismisses it. Both layers sit after the app's own children,
                // so they paint over everything.
                root.child(
                    div()
                        .id("palette-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(palette_scrim(cx.theme().is_dark()))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.close_palette();
                            cx.notify();
                        })),
                )
                // The command palette owns focus and the keyboard while open.
                .child(self.render_palette(window, cx))
            })
    }
}

/// Register every global shortcut on the application.
///
/// These are *app-level* action listeners ([`App::on_action`]), not handlers on
/// the view's own element. GPUI dispatches an action along the path from the
/// window root down to the focused node and then bubbles back up, so an element
/// listener only runs when focus happens to be inside that element — which is
/// why the shortcuts used to work only while the code editor had focus, and did
/// nothing at all with nothing focused. App-level listeners run after that walk
/// regardless of where focus is.
///
/// Registered once, when the window is created.
fn bind_global_actions(cx: &mut App, view: &Entity<LeetrView>) {
    fn listener<A: gpui_kit::Action + 'static>(
        cx: &mut App,
        view: &Entity<LeetrView>,
        handler: impl Fn(&mut LeetrView, &mut App) + 'static,
    ) {
        let view = view.clone();
        cx.on_action(move |_: &A, cx: &mut App| {
            view.update(cx, |this, cx| handler(this, cx));
        });
    }

    listener::<ToggleTheme>(cx, view, |this, cx| {
        this.toggle_theme(cx);
    });
    // These need a window to move focus, so they are deferred to the next paint.
    listener::<ActionPalette>(cx, view, |this, _| {
        this.pending_palette = Some(PaletteKind::Actions);
    });
    listener::<ProblemPalette>(cx, view, |this, _| {
        this.pending_palette = Some(PaletteKind::Problems);
    });
    listener::<CloseOverlay>(cx, view, |this, _| this.close_palette());
    listener::<Quit>(cx, view, |_, cx| cx.quit());
    listener::<OpenInNvim>(cx, view, |this, _| this.open_in_nvim());
    listener::<RunTests>(cx, view, |this, _| this.start_judge(false));
    listener::<SubmitSolution>(cx, view, |this, _| this.start_judge(true));
    listener::<ToggleSidebar>(cx, view, |this, _| {
        this.sidebar_collapsed = !this.sidebar_collapsed;
    });
}

/// Start the GPUI application.
pub fn run() -> anyhow::Result<()> {
    let bootstrap = Bootstrap::load();
    let theme = bootstrap.config.theme;
    // Every icon GPUI Kit draws resolves through an `AssetSource`, so one has
    // to be registered or icons render as nothing. `AllAssets` is the complete
    // Lucide catalog: the curated `Assets` bundle covers what the components
    // use internally but omits most of the icons the sidebar rows ask for, and
    // a missing icon fails silently.
    let mut application = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    // Statement diagrams are remote files; GPUI fetches them through the app's
    // `HttpClient`. Its built-in default returns an error, so Leetr supplies a
    // blocking one (see `crate::img`).
    if let Some(client) = crate::img::image_client() {
        application = application.with_http_client(client);
    }
    application.run(move |cx: &mut App| {
        // This must be called before using any GPUI Component features.
        gpui_kit::init(cx);
        // Draw the interface in Iosevka Nerd Font when it is installed; the
        // bundled fallbacks render otherwise.
        crate::fonts::install(cx);
        KitTheme::change(theme.mode(), None, cx);

        cx.bind_keys([
            KeyBinding::new("ctrl-k", ActionPalette, None),
            KeyBinding::new("ctrl-p", ProblemPalette, None),
            KeyBinding::new("ctrl-o", OpenInNvim, None),
            KeyBinding::new("ctrl-'", RunTests, None),
            KeyBinding::new("ctrl-enter", SubmitSolution, None),
            KeyBinding::new("ctrl-b", ToggleSidebar, None),
            KeyBinding::new("escape", CloseOverlay, None),
            // Alt+1 / Alt+2 switch panes. The plain digits are unusable: a
            // binding without a modifier also matches while the code editor is
            // focused (on Linux GPUI does not prefer character input over a
            // binding), so typing "1" in a solution would jump to the
            // statement. A context predicate cannot express "not in the
            // editor" either — GPUI's `!Input` never matches an empty context
            // stack, which is the state with nothing focused.
            KeyBinding::new("alt-1", ShowStatementTab, None),
            KeyBinding::new("alt-2", ShowCodeTab, None),
        ]);

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Leetr".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search problems…"));
            let actions_cmd = cx.new(|cx| CommandState::new(window, cx));
            let problems_cmd = cx.new(|cx| CommandState::new(window, cx));
            let lang = editor_language(&bootstrap.config.default_language);
            let code = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(lang)
                    .default_value("")
            });
            cx.new(|cx| LeetrView::new(bootstrap, search, actions_cmd, problems_cmd, code, cx))
        });
        match opened {
            Ok((_, view)) => bind_global_actions(cx, &view),
            Err(err) => eprintln!("failed to open window: {err}"),
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::palette;

    /// The languages the editor claims to highlight must really have a grammar
    /// compiled in.
    ///
    /// This is the regression test for a real bug: the editor shipped with no
    /// syntax highlighting at all because no `tree-sitter-*` feature was
    /// enabled, and nothing failed — the language simply fell back to plain
    /// text. Asserting the grammar exists makes that failure visible.
    #[test]
    fn every_highlighted_language_has_a_compiled_in_grammar() {
        use gpui_kit::component::highlighter::LanguageRegistry;

        // Only these two grammars are compiled in (see Cargo.toml).
        const HIGHLIGHTED: &[&str] = &["python", "cpp"];
        // Slugs that must map onto them, including C, which rides the C++
        // grammar.
        const ALIASES: &[&str] = &[
            "python",
            "python3",
            "py",
            "pythondata",
            "pandas",
            "cpp",
            "c++",
            "c",
        ];

        let registry = LanguageRegistry::singleton();
        for slug in ALIASES {
            let name = editor_language(slug);
            assert!(
                HIGHLIGHTED.contains(&name),
                "{slug} maps to {name:?}, which is not a compiled-in grammar"
            );
        }
        for name in HIGHLIGHTED {
            let config = registry
                .language(name)
                .unwrap_or_else(|| panic!("{name:?} is not a language the registry knows"));
            assert!(
                config.has_grammar(),
                "{name:?} has no tree-sitter grammar — the `tree-sitter-{name}` feature is \
                 probably missing from Cargo.toml"
            );
        }
    }

    #[test]
    fn languages_without_a_grammar_render_as_plain_text() {
        use gpui_kit::component::highlighter::LanguageRegistry;

        // Languages LeetCode offers that this build deliberately does not
        // highlight.
        for slug in [
            "rust",
            "java",
            "golang",
            "javascript",
            "typescript",
            "csharp",
            "ruby",
            "swift",
            "kotlin",
            "scala",
            "php",
            "elixir",
            "bash",
            "mysql",
        ] {
            assert_eq!(
                editor_language(slug),
                "text",
                "{slug} has no grammar and must fall back to plain text"
            );
        }
        // Unknown slugs too.
        assert_eq!(editor_language("dart"), "text");
        assert_eq!(editor_language("racket"), "text");

        // The fallback itself has to be a language the registry knows, or the
        // editor would treat the name as garbage rather than as plain text.
        assert!(
            LanguageRegistry::singleton().language("text").is_some(),
            "the plain-text fallback must be a registered language"
        );
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
