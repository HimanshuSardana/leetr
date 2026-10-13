//! The problem list panel: search box, filters and a virtualised list.

use egui::text::LayoutJob;
use egui::{Color32, FontId, RichText, TextEdit, TextFormat, Ui};

use crate::glyphs;
use crate::model::{Difficulty, Problem, ProblemStatus};
use crate::theme;

/// How tall a single list row is.
const ROW_HEIGHT: f32 = 22.0;

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
}

impl ProblemListAction {
    pub fn is_empty(&self) -> bool {
        self.open.is_none() && !self.load_more && self.server_search.is_none() && !self.refresh
    }
}

/// Draw the panel. Returns whatever the user requested.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    state: &mut ProblemListState,
    problems: &[Problem],
    total: u32,
    loading: bool,
    error: Option<&str>,
    has_more: bool,
    selected: Option<&str>,
) -> ProblemListAction {
    let mut action = ProblemListAction::default();

    egui::Panel::top("problems-controls").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong("Problems");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(glyphs::REFRESH)
                    .on_hover_text("Reload the problem list")
                    .clicked()
                {
                    action.refresh = true;
                }
                if loading {
                    ui.spinner();
                }
                ui.label(
                    RichText::new(format!("{} / {total}", problems.len()))
                        .weak()
                        .small(),
                );
            });
        });

        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut state.search)
                    .hint_text("Filter loaded problems…")
                    .desired_width(f32::INFINITY),
            );
        });

        ui.horizontal_wrapped(|ui| {
            for (label, value) in [
                ("All", None),
                ("Easy", Some(Difficulty::Easy)),
                ("Medium", Some(Difficulty::Medium)),
                ("Hard", Some(Difficulty::Hard)),
            ] {
                if ui
                    .selectable_label(state.difficulty == value, label)
                    .clicked()
                {
                    state.difficulty = value;
                }
            }

            ui.separator();

            for (label, value) in [
                ("Any", None),
                ("Solved", Some(ProblemStatus::Solved)),
                ("Attempted", Some(ProblemStatus::Attempted)),
                ("Todo", Some(ProblemStatus::Todo)),
            ] {
                if ui.selectable_label(state.status == value, label).clicked() {
                    state.status = value;
                }
            }

            ui.separator();
            if ui
                .selectable_label(state.hide_premium, "No premium")
                .on_hover_text("Hide premium-only problems")
                .clicked()
            {
                state.hide_premium = !state.hide_premium;
            }

            if state.is_filtered() && ui.small_button("clear").clicked() {
                state.clear_filters();
            }
        });

        if let Some(err) = error {
            ui.colored_label(theme::DANGER, err);
        }
        ui.add_space(4.0);
    });

    let matching = state.matching(problems);
    let show_server_search = !state.search.trim().is_empty() && matching.is_empty() && !loading;

    egui::Panel::bottom("problems-footer").show(ui, |ui| {
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if state.is_filtered() {
                ui.label(
                    RichText::new(format!(
                        "{} of {} loaded match",
                        matching.len(),
                        problems.len()
                    ))
                    .weak()
                    .small(),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if has_more {
                    if ui.button("Load more").clicked() {
                        action.load_more = true;
                    }
                } else if !problems.is_empty() {
                    ui.label(RichText::new("all loaded").weak().small());
                }
            });
        });
        ui.add_space(2.0);
    });

    egui::CentralPanel::no_frame().show(ui, |ui| {
        if problems.is_empty() && loading {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.spinner();
                ui.label(RichText::new("Loading problems…").weak());
            });
            return;
        }

        if matching.is_empty() {
            ui.add_space(12.0);
            ui.label(RichText::new("No problems match these filters.").weak());
            if show_server_search {
                ui.add_space(6.0);
                if ui
                    .button(format!("Search LeetCode for “{}”", state.search.trim()))
                    .clicked()
                {
                    action.server_search = Some(state.search.trim().to_string());
                }
            }
            return;
        }

        let row_width = ui.available_width();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, matching.len(), |ui, range| {
                for index in range {
                    let problem = &problems[matching[index]];
                    let is_selected = selected == Some(problem.slug.as_str());
                    let job = row_job(problem, row_width);
                    if ui.selectable_label(is_selected, job).clicked() {
                        action.open = Some(problem.slug.clone());
                    }
                }
            });
    });

    action
}

/// One list row as a single rich-text job, so the whole row stays clickable.
fn row_job(problem: &Problem, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width.max(160.0);

    let weak = Color32::from_rgb(0x8a, 0x8f, 0x98);
    let text_color = Color32::from_rgb(0xc8, 0xce, 0xd6);
    let status_color = match problem.status {
        ProblemStatus::Solved => theme::SUCCESS,
        ProblemStatus::Attempted => theme::WARNING,
        ProblemStatus::Todo => weak,
    };

    append(
        &mut job,
        &format!("{:>5}", problem.id),
        FontId::monospace(12.0),
        weak,
    );
    append(&mut job, "  ", FontId::proportional(13.0), weak);
    append(
        &mut job,
        problem.status.marker(),
        FontId::proportional(13.0),
        status_color,
    );
    append(&mut job, " ", FontId::proportional(13.0), weak);
    append(
        &mut job,
        &format!("{:<6}", problem.difficulty.label()),
        FontId::proportional(13.0),
        theme::difficulty_color(problem.difficulty),
    );
    append(&mut job, " ", FontId::proportional(13.0), weak);
    append(
        &mut job,
        &problem.title,
        FontId::proportional(14.0),
        text_color,
    );

    if problem.paid_only {
        append(
            &mut job,
            &format!("  {}", glyphs::PREMIUM),
            FontId::proportional(12.0),
            theme::ACCENT,
        );
    }
    if let Some(rate) = problem.ac_rate {
        append(
            &mut job,
            &format!("  {rate:.0}%"),
            FontId::proportional(11.0),
            weak,
        );
    }

    job
}

fn append(job: &mut LayoutJob, text: &str, font_id: FontId, color: Color32) {
    job.append(
        text,
        0.0,
        TextFormat {
            font_id,
            color,
            ..Default::default()
        },
    );
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
    fn row_job_contains_the_problem_title_and_difficulty() {
        let job = row_job(&sample()[0], 400.0);
        assert!(job.text.contains("Two Sum"));
        assert!(job.text.contains("Easy"));
        assert!(job.text.contains('1'));
    }
}
