//! The problem list panel: brand, quick actions, search, filters and a
//! virtualised list.
//!
//! This panel *is* the app's sidebar, in the manner of a modern workspace app:
//! a white lettermark with the session line, a "Quick actions" entry that opens
//! the `Ctrl+K` palette, a soft search field, text-only filters, and
//! single-line rows that lead with a status dot and truncate long titles rather
//! than wrapping or showing dense metadata.

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Label, Layout, RichText, Sense, Stroke,
    StrokeKind, TextEdit, TextFormat, Ui, UiBuilder, Vec2,
};

use crate::glyphs;
use crate::model::{Difficulty, Problem, ProblemStatus};
use crate::theme::{self, Theme};

/// Height of a single list row. Roomy, like a modern sidebar.
const ROW_HEIGHT: f32 = 34.0;
/// Horizontal padding inside the panel.
const SIDE_PADDING: f32 = 10.0;
/// Radius of the small leading dot.
const DOT_RADIUS: f32 = 3.5;

/// The chrome drawn at the top of the panel.
pub struct Header<'a> {
    /// Session line under the app name, e.g. `alice · 152 solved`.
    pub session: &'a str,
    /// Colour for the session line; `None` falls back to the muted text colour.
    pub session_color: Option<Color32>,
    pub theme: Theme,
}

/// Filters applied to the loaded problem list.
#[derive(Debug, Clone, Default)]
pub struct ProblemListState {
    pub search: String,
    pub status: Option<ProblemStatus>,
    pub difficulty: Option<Difficulty>,
    pub hide_premium: bool,
}

impl ProblemListState {
    /// Indices into `problems` that match the current filters.
    pub fn matching(&self, problems: &[Problem]) -> Vec<usize> {
        let needle = self.search.trim().to_ascii_lowercase();

        problems
            .iter()
            .enumerate()
            .filter(|(_, problem)| {
                if self.hide_premium && problem.paid_only {
                    return false;
                }
                if let Some(status) = self.status {
                    if problem.status != status {
                        return false;
                    }
                }
                if let Some(difficulty) = self.difficulty {
                    if problem.difficulty != difficulty {
                        return false;
                    }
                }
                needle.is_empty() || problem.search_haystack().contains(&needle)
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// True when any filter is active.
    pub fn is_filtered(&self) -> bool {
        !self.search.trim().is_empty()
            || self.status.is_some()
            || self.difficulty.is_some()
            || self.hide_premium
    }

    pub fn clear_filters(&mut self) {
        self.search.clear();
        self.status = None;
        self.difficulty = None;
        self.hide_premium = false;
    }
}

/// What the user asked the list to do this frame.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProblemListAction {
    pub open: Option<String>,
    pub load_more: bool,
    pub server_search: Option<String>,
    pub refresh: bool,
    /// Return to the full-width list, dropping the open problem.
    pub show_all: bool,
    /// Open the `Ctrl+K` command palette from the "Quick actions" row.
    pub open_actions: bool,
    /// Flip the colour scheme from the header toggle.
    pub toggle_theme: bool,
}

impl ProblemListAction {
    pub fn is_empty(&self) -> bool {
        self.open.is_none()
            && !self.load_more
            && self.server_search.is_none()
            && !self.refresh
            && !self.show_all
            && !self.open_actions
            && !self.toggle_theme
    }
}

/// Draw the panel. Returns whatever the user requested.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    state: &mut ProblemListState,
    header: &Header<'_>,
    problems: &[Problem],
    total: u32,
    loading: bool,
    error: Option<&str>,
    has_more: bool,
    selected: Option<&str>,
) -> ProblemListAction {
    let mut action = ProblemListAction::default();

    egui::Panel::top("problems-controls")
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: SIDE_PADDING as i8,
            right: SIDE_PADDING as i8,
            top: 12,
            bottom: 6,
        }))
        .show(ui, |ui| {
            chrome(ui, header, &mut action);
            ui.add_space(12.0);
            header_row(ui, problems.len(), total, loading, &mut action);
            ui.add_space(10.0);
            search_field(ui, state);
            ui.add_space(8.0);
            filters(ui, state);

            if let Some(err) = error {
                ui.add_space(6.0);
                ui.label(RichText::new(err).color(theme::DANGER).small());
            }
        });

    let matching = state.matching(problems);
    let show_server_search = !state.search.trim().is_empty() && matching.is_empty() && !loading;

    egui::Panel::bottom("problems-footer")
        .frame(egui::Frame::NONE.inner_margin(egui::Margin {
            left: SIDE_PADDING as i8,
            right: SIDE_PADDING as i8,
            top: 4,
            bottom: 8,
        }))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if has_more && quiet_button(ui, "Load more").clicked() {
                    action.load_more = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if state.is_filtered() {
                        format!("{} of {}", matching.len(), problems.len())
                    } else {
                        format!("{}", problems.len())
                    };
                    ui.label(RichText::new(label).weak().small());
                });
            });
        });

    egui::CentralPanel::no_frame().show(ui, |ui| {
        if problems.is_empty() && loading {
            centered_message(ui, Some(()), "Loading problems…");
            return;
        }

        if matching.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("No problems match these filters.").weak());
            if show_server_search {
                ui.add_space(8.0);
                if quiet_button(
                    ui,
                    &format!("Search LeetCode for “{}”", state.search.trim()),
                )
                .clicked()
                {
                    action.server_search = Some(state.search.trim().to_string());
                }
            }
            return;
        }

        let width = ui.available_width();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, matching.len(), |ui, range| {
                for index in range {
                    let problem = &problems[matching[index]];
                    let is_selected = selected == Some(problem.slug.as_str());
                    if problem_row(ui, problem, is_selected, width).clicked() {
                        action.open = Some(problem.slug.clone());
                    }
                }
            });
    });

    action
}

// ---------------------------------------------------------------------------
// Header, search, filters
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Chrome, search, filters
// ---------------------------------------------------------------------------

/// The brand block, the theme toggle and the "Quick actions" entry.
fn chrome(ui: &mut Ui, header: &Header<'_>, action: &mut ProblemListAction) {
    ui.horizontal(|ui| {
        // The brand doubles as a "home" affordance: clicking it drops the open
        // problem and returns to the full-width list.
        let brand = ui
            .horizontal(|ui| {
                let mark = lettermark(ui);
                ui.add_space(10.0);
                let name = ui
                    .vertical(|ui| {
                        ui.add_space(1.0);
                        let title = ui.add(
                            egui::Label::new(RichText::new("Leetr").size(15.0).strong())
                                .sense(Sense::click()),
                        );
                        ui.label(
                            RichText::new(header.session)
                                .color(
                                    header
                                        .session_color
                                        .unwrap_or_else(|| ui.visuals().weak_text_color()),
                                )
                                .size(11.5),
                        );
                        title
                    })
                    .inner;
                mark.union(name)
            })
            .inner
            .on_hover_text("Show all problems");
        if brand.clicked() {
            action.show_all = true;
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let toggle = ui
                .add(
                    egui::Button::new(RichText::new(header.theme.icon()).monospace().size(14.0))
                        .frame(false),
                )
                .on_hover_text(format!("Switch to {} mode", header.theme.toggled().label()));
            if toggle.clicked() {
                action.toggle_theme = true;
            }
        });
    });

    ui.add_space(12.0);
    if quick_actions_row(ui).clicked() {
        action.open_actions = true;
    }
    ui.add_space(8.0);
    divider(ui);
}

/// A rounded, dark square holding a white `L`.
///
/// The fill is fixed rather than themed: the reference mark is a dark tile with
/// a light glyph, and a theme-following tile would put a white `L` on a pale
/// square in light mode.
fn lettermark(ui: &mut Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let radius = CornerRadius::same(8);
    ui.painter()
        .rect_filled(rect, radius, Color32::from_rgb(0x24, 0x28, 0x2f));
    ui.painter().rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, Color32::from_rgb(0x3a, 0x40, 0x49)),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        "L",
        FontId::proportional(16.0),
        Color32::WHITE,
    );
    response
}

/// The full-width "Quick actions" row with its `Ctrl K` key cap.
fn quick_actions_row(ui: &mut Ui) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    if response.hovered() {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(0.0, 1.0)),
            CornerRadius::same(7),
            ui.visuals().widgets.hovered.weak_bg_fill,
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Quick actions",
        FontId::proportional(13.0),
        ui.visuals().text_color(),
    );

    // Right-aligned key cap.
    let galley = ui.painter().layout_no_wrap(
        "Ctrl K".to_owned(),
        FontId::monospace(11.0),
        ui.visuals().weak_text_color(),
    );
    let size = Vec2::new(galley.size().x + 16.0, 20.0);
    let cap = egui::Rect::from_min_size(
        egui::pos2(rect.right() - size.x, rect.center().y - size.y / 2.0),
        size,
    );
    let radius = CornerRadius::same(5);
    ui.painter()
        .rect_filled(cap, radius, ui.visuals().widgets.inactive.weak_bg_fill);
    ui.painter().rect_stroke(
        cap,
        radius,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        StrokeKind::Inside,
    );
    ui.painter().text(
        cap.center(),
        Align2::CENTER_CENTER,
        "Ctrl K",
        FontId::monospace(11.0),
        ui.visuals().weak_text_color(),
    );

    response
}

/// A hairline separator spanning the panel.
fn divider(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
}

/// The section title, problem count and reload control.
fn header_row(
    ui: &mut Ui,
    loaded: usize,
    total: u32,
    loading: bool,
    action: &mut ProblemListAction,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Problems").size(13.0).strong());
        ui.add_space(2.0);

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if quiet_button(ui, glyphs::REFRESH)
                .on_hover_text("Reload the problem list")
                .clicked()
            {
                action.refresh = true;
            }
            if loading {
                ui.spinner();
            } else {
                ui.label(
                    RichText::new(if total > 0 {
                        format!("{loaded}/{total}")
                    } else {
                        String::new()
                    })
                    .weak()
                    .small(),
                );
            }
        });
    });
}

fn search_field(ui: &mut Ui, state: &mut ProblemListState) {
    let fill = ui.visuals().faint_bg_color;
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin {
            left: 8,
            right: 8,
            top: 6,
            bottom: 6,
        })
        .show(ui, |ui| {
            ui.add(
                TextEdit::singleline(&mut state.search)
                    // The surrounding Frame provides the soft background.
                    .frame(egui::Frame::NONE)
                    .margin(egui::Margin::ZERO)
                    .hint_text("Filter loaded problems")
                    .desired_width(f32::INFINITY),
            );
        });
}

fn filters(ui: &mut Ui, state: &mut ProblemListState) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;

        for (label, value) in [
            ("All", None),
            ("Easy", Some(Difficulty::Easy)),
            ("Medium", Some(Difficulty::Medium)),
            ("Hard", Some(Difficulty::Hard)),
        ] {
            if quiet_toggle(ui, state.difficulty == value, label).clicked() {
                state.difficulty = value;
            }
        }

        for (label, value) in [
            ("Any", None),
            ("Solved", Some(ProblemStatus::Solved)),
            ("Attempted", Some(ProblemStatus::Attempted)),
            ("Todo", Some(ProblemStatus::Todo)),
        ] {
            if quiet_toggle(ui, state.status == value, label).clicked() {
                state.status = value;
            }
        }

        if quiet_toggle(ui, state.hide_premium, "No premium")
            .on_hover_text("Hide premium-only problems")
            .clicked()
        {
            state.hide_premium = !state.hide_premium;
        }

        if state.is_filtered() && quiet_button(ui, "clear").clicked() {
            state.clear_filters();
        }
    });
}

/// A text-only filter toggle: the active one is brighter, nothing else moves.
fn quiet_toggle(ui: &mut Ui, selected: bool, label: &str) -> egui::Response {
    let color = if selected {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().weak_text_color()
    };
    let text = RichText::new(label).color(color).size(12.5);
    let text = if selected { text.strong() } else { text };

    ui.add(
        egui::Button::new(text)
            .frame(false)
            .min_size(Vec2::new(0.0, 16.0)),
    )
}

fn quiet_button(ui: &mut Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).weak().size(12.5))
            .frame(false)
            .min_size(Vec2::new(0.0, 16.0)),
    )
}

fn centered_message(ui: &mut Ui, spinner: Option<()>, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(24.0);
        if spinner.is_some() {
            ui.spinner();
        }
        ui.label(RichText::new(text).weak());
    });
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// One problem row: a status dot, a single truncated title, and a muted
/// difficulty on the right. No ids, no acceptance rate, no wrapping.
fn problem_row(ui: &mut Ui, problem: &Problem, selected: bool, width: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let visuals = ui.visuals();
    let background = if selected {
        Some(visuals.selection.bg_fill.gamma_multiply(0.55))
    } else if response.hovered() {
        Some(visuals.widgets.hovered.weak_bg_fill)
    } else {
        None
    };
    if let Some(fill) = background {
        ui.painter().rect_filled(
            rect.shrink2(Vec2::new(4.0, 2.0)),
            CornerRadius::same(6),
            fill,
        );
    }

    // Leading status dot, the row's only marker.
    ui.painter().circle_filled(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        DOT_RADIUS,
        status_dot_color(problem.status),
    );

    // Title, truncated rather than wrapped.
    let title_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 30.0, rect.top()),
        egui::pos2(rect.right() - 62.0, rect.bottom()),
    );
    let title_color = if selected {
        visuals.strong_text_color()
    } else {
        visuals.text_color()
    };
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(title_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    child.add(
        Label::new(RichText::new(&problem.title).color(title_color).size(13.5))
            .truncate()
            .selectable(false),
    );

    // Muted difficulty on the right.
    let meta_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - 62.0, rect.top()),
        egui::pos2(rect.right() - 10.0, rect.bottom()),
    );
    let mut meta = ui.new_child(
        UiBuilder::new()
            .max_rect(meta_rect)
            .layout(Layout::right_to_left(Align::Center)),
    );
    meta.add(
        Label::new(
            RichText::new(problem.difficulty.label())
                .color(theme::difficulty_color(problem.difficulty).gamma_multiply(0.85))
                .size(11.0),
        )
        .selectable(false),
    );

    response.on_hover_text(hover_text(problem))
}

/// Tooltip with the details that the quiet rows deliberately omit.
fn hover_text(problem: &Problem) -> String {
    let mut parts = vec![format!("{}. {}", problem.id, problem.title)];
    parts.push(format!(
        "{} · {}",
        problem.difficulty.label(),
        problem.status.label()
    ));

    if let Some(rate) = problem.ac_rate {
        parts.push(format!("{rate:.1}% acceptance"));
    }
    if problem.paid_only {
        parts.push("premium only".to_string());
    }
    if !problem.tags.is_empty() {
        parts.push(
            problem
                .tags
                .iter()
                .map(|tag| tag.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    parts.push(problem.slug.clone());
    parts.join("\n")
}

/// Colour of the leading dot.
fn status_dot_color(status: ProblemStatus) -> Color32 {
    match status {
        ProblemStatus::Solved => theme::SUCCESS,
        ProblemStatus::Attempted => theme::WARNING,
        ProblemStatus::Todo => Color32::from_rgb(0x4a, 0x4f, 0x57),
    }
}

/// Kept for the empty-state test that a row job is still buildable.
#[allow(dead_code)]
fn row_job(problem: &Problem, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width.max(160.0);
    job.append(
        &problem.title,
        0.0,
        TextFormat {
            font_id: FontId::proportional(13.5),
            color: Color32::WHITE,
            ..Default::default()
        },
    );
    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Tag;

    fn problem(
        id: &str,
        title: &str,
        slug: &str,
        difficulty: Difficulty,
        status: ProblemStatus,
    ) -> Problem {
        Problem {
            id: id.into(),
            title: title.into(),
            slug: slug.into(),
            difficulty,
            paid_only: false,
            status,
            ac_rate: None,
            tags: vec![Tag {
                name: "Array".into(),
                slug: "array".into(),
            }],
        }
    }

    fn sample() -> Vec<Problem> {
        vec![
            problem(
                "1",
                "Two Sum",
                "two-sum",
                Difficulty::Easy,
                ProblemStatus::Solved,
            ),
            problem(
                "4",
                "Median of Two Sorted Arrays",
                "median-of-two-sorted-arrays",
                Difficulty::Hard,
                ProblemStatus::Todo,
            ),
            problem(
                "11",
                "Container With Most Water",
                "container-with-most-water",
                Difficulty::Medium,
                ProblemStatus::Attempted,
            ),
        ]
    }

    #[test]
    fn no_filters_matches_everything_in_order() {
        let state = ProblemListState::default();
        assert_eq!(state.matching(&sample()), vec![0, 1, 2]);
        assert!(!state.is_filtered());
    }

    #[test]
    fn search_matches_title_slug_and_id() {
        let mut state = ProblemListState {
            search: "median".into(),
            ..Default::default()
        };
        assert_eq!(state.matching(&sample()), vec![1]);

        state.search = "container-with".into();
        assert_eq!(state.matching(&sample()), vec![2]);

        state.search = "11".into();
        assert_eq!(state.matching(&sample()), vec![2]);

        state.search = "  TWO SUM ".into();
        assert_eq!(state.matching(&sample()), vec![0]);
    }

    #[test]
    fn difficulty_and_status_filters_compose() {
        let mut state = ProblemListState {
            difficulty: Some(Difficulty::Medium),
            ..Default::default()
        };
        assert_eq!(state.matching(&sample()), vec![2]);

        state.status = Some(ProblemStatus::Solved);
        assert!(state.matching(&sample()).is_empty());
        assert!(state.is_filtered());
    }

    #[test]
    fn status_filter_selects_matching_problems() {
        let state = ProblemListState {
            status: Some(ProblemStatus::Attempted),
            ..Default::default()
        };
        assert_eq!(state.matching(&sample()), vec![2]);
    }

    #[test]
    fn premium_filter_hides_paid_problems() {
        let mut problems = sample();
        problems[0].paid_only = true;

        let state = ProblemListState {
            hide_premium: true,
            ..Default::default()
        };
        assert_eq!(state.matching(&problems), vec![1, 2]);
    }

    #[test]
    fn clearing_filters_restores_everything() {
        let mut state = ProblemListState {
            search: "two".into(),
            status: Some(ProblemStatus::Todo),
            difficulty: Some(Difficulty::Hard),
            hide_premium: true,
        };
        state.clear_filters();
        assert!(!state.is_filtered());
        assert_eq!(state.matching(&sample()), vec![0, 1, 2]);
    }

    #[test]
    fn the_status_dot_distinguishes_every_state() {
        let solved = status_dot_color(ProblemStatus::Solved);
        let attempted = status_dot_color(ProblemStatus::Attempted);
        let todo = status_dot_color(ProblemStatus::Todo);
        assert_ne!(solved, attempted);
        assert_ne!(solved, todo);
        assert_ne!(attempted, todo);
        // The unsolved marker is deliberately the dimmest, so a long list of
        // not-yet-attempted problems reads as quiet.
        let brightness = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
        assert!(brightness(todo) < brightness(solved));
        assert!(brightness(todo) < brightness(attempted));
    }

    #[test]
    fn hover_text_carries_the_details_the_rows_omit() {
        let mut problem = sample()[0].clone();
        problem.ac_rate = Some(55.4);
        problem.paid_only = true;

        let text = hover_text(&problem);
        assert!(text.contains("1. Two Sum"));
        assert!(text.contains("Easy · solved"));
        assert!(text.contains("55.4% acceptance"));
        assert!(text.contains("premium only"));
        assert!(text.contains("Array"));
        assert!(text.contains("two-sum"));
    }

    #[test]
    fn hover_text_omits_missing_details() {
        let text = hover_text(&sample()[1]);
        assert!(!text.contains("acceptance"));
        assert!(!text.contains("premium only"));
    }

    #[test]
    fn rows_render_without_panicking_in_every_state() {
        let ctx = egui::Context::default();
        let problems = sample();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for problem in &problems {
                problem_row(ui, problem, false, 400.0);
                problem_row(ui, problem, true, 400.0);
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn the_panel_renders_in_loading_empty_and_loaded_states() {
        let ctx = egui::Context::default();
        let problems = sample();
        let mut state = ProblemListState::default();
        let header = Header {
            session: "browse-only",
            session_color: None,
            theme: Theme::Dark,
        };

        for (list, loading, error) in [
            (&[][..], true, None),
            (&problems[..], false, None),
            (&problems[..], false, Some("boom")),
            (&[][..], false, None),
        ] {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let _ = show(
                    ui, &mut state, &header, list, 42, loading, error, true, None,
                );
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn the_row_job_helper_still_builds() {
        // Guards the retained helper used by the empty-state path.
        let job = row_job(&sample()[0], 400.0);
        assert!(job.text.contains("Two Sum"));
    }
}
