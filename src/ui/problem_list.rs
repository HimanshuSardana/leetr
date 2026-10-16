//! The problem list panel: brand, quick actions, search, filters and a
//! virtualised list.
//!
//! This panel *is* the app's sidebar, in the manner of a modern workspace app:
//! a white lettermark with the session line, a "Quick actions" entry that opens
//! the `Ctrl+K` palette, a soft search field, text-only filters, and
//! single-line rows that lead with a status dot and truncate long titles rather
//! than wrapping or showing dense metadata.

use crate::model::{Difficulty, Problem, ProblemStatus};

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

/// Tooltip text for a problem row: id, title, difficulty, status, acceptance,
///
/// Kept pure so the details the rows omit stay tested.
pub fn hover_text(problem: &Problem) -> String {
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

/// Packed `0xRRGGBB` of the leading status dot.
pub fn status_dot_hex(status: ProblemStatus) -> u32 {
    match status {
        ProblemStatus::Solved => crate::theme::SUCCESS_HEX,
        ProblemStatus::Attempted => crate::theme::WARNING_HEX,
        ProblemStatus::Todo => 0x4A4F57,
    }
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
    fn the_status_dot_distinguishes_every_state() {
        let solved = status_dot_hex(ProblemStatus::Solved);
        let attempted = status_dot_hex(ProblemStatus::Attempted);
        let todo = status_dot_hex(ProblemStatus::Todo);
        assert_ne!(solved, attempted);
        assert_ne!(solved, todo);
        assert_ne!(attempted, todo);
    }
}
