//! Domain types.
//!
//! These are deliberately decoupled from the GraphQL wire format: the API
//! schema lives in [`crate::leetcode::graphql`], and the client converts wire
//! structs into these types. If LeetCode reshapes its schema, only the wire
//! structs and the conversions have to change.

use serde::{Deserialize, Serialize};

/// Problem difficulty, with a lossy fallback for unknown values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Difficulty {
    #[default]
    Unknown,
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub fn from_wire(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "easy" => Self::Easy,
            "medium" => Self::Medium,
            "hard" => Self::Hard,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "—",
            Self::Easy => "Easy",
            Self::Medium => "Medium",
            Self::Hard => "Hard",
        }
    }
}

/// The signed-in user's relationship to a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProblemStatus {
    #[default]
    Todo,
    Attempted,
    Solved,
}

impl ProblemStatus {
    /// LeetCode reports `"ac"` (accepted), `"notac"` (attempted) or `null`.
    pub fn from_wire(value: Option<&str>) -> Self {
        match value {
            Some("ac") => Self::Solved,
            Some("notac") => Self::Attempted,
            _ => Self::Todo,
        }
    }

    /// The value expected by LeetCode's `UserQuestionStatus` GraphQL enum.
    ///
    /// Note: the enum is *not* named after the wire `status` values
    /// (`"ac"` / `"notac"`). The live schema expects `AC`, `TRIED` and
    /// `NOT_STARTED` — verified against the API.
    pub fn as_filter(self) -> &'static str {
        match self {
            Self::Todo => "NOT_STARTED",
            Self::Attempted => "TRIED",
            Self::Solved => "AC",
        }
    }

    /// Compact marker used in the problem list.
    ///
    /// These glyphs are deliberately chosen from what egui's *default* fonts
    /// actually contain. `✓` (U+2713) and friends are not present and render
    /// as tofu boxes — see `ui::glyph_tests`.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Solved => crate::glyphs::SOLVED,
            Self::Attempted => crate::glyphs::ATTEMPTED,
            Self::Todo => crate::glyphs::TODO,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Solved => "solved",
            Self::Attempted => "attempted",
            Self::Todo => "todo",
        }
    }
}

/// Session information for the configured credentials.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct UserStatus {
    pub signed_in: bool,
    pub username: String,
    pub premium: bool,
}

impl UserStatus {
    /// Best-effort display name for the status bar.
    pub fn display_name(&self) -> &str {
        if self.username.is_empty() {
            "(unknown)"
        } else {
            &self.username
        }
    }
}

/// A topic tag such as "Array" or "Dynamic Programming".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub name: String,
    pub slug: String,
}

/// A row in the problem list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Problem {
    /// The public frontend id, e.g. `"1"` (may be `"1"` or `"100001"`).
    pub id: String,
    pub title: String,
    pub slug: String,
    pub difficulty: Difficulty,
    pub paid_only: bool,
    pub status: ProblemStatus,
    pub ac_rate: Option<f64>,
    pub tags: Vec<Tag>,
}

impl Problem {
    /// Text used by quick-search / fuzzy matching.
    pub fn search_haystack(&self) -> String {
        format!("{} {} {}", self.id, self.title, self.slug).to_ascii_lowercase()
    }
}

/// A page of problems returned by the paginated list query.
#[derive(Debug, Clone, PartialEq)]
pub struct ProblemPage {
    pub total: u32,
    pub problems: Vec<Problem>,
}

/// A programming language LeetCode accepts submissions in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Language {
    pub id: u32,
    pub name: String,
    pub verbose_name: String,
}

/// Starter code for one language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSnippet {
    pub lang: String,
    pub lang_slug: String,
    pub code: String,
}

/// A fully loaded problem statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionDetail {
    /// The internal question id used by the judge REST endpoints.
    pub question_id: String,
    pub frontend_id: String,
    pub title: String,
    pub slug: String,
    pub difficulty: Difficulty,
    pub paid_only: bool,
    /// Raw statement HTML.
    pub content_html: String,
    /// The sample input used by "run code".
    pub sample_test_case: String,
    /// All example inputs, newline separated.
    pub example_testcases: String,
    pub hints: Vec<String>,
    pub tags: Vec<Tag>,
    pub snippets: Vec<CodeSnippet>,
}

impl QuestionDetail {
    /// The boilerplate for a language slug, if LeetCode provided one.
    pub fn snippet_for(&self, lang_slug: &str) -> Option<&str> {
        self.snippets
            .iter()
            .find(|s| s.lang_slug == lang_slug)
            .map(|s| s.code.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_parses_wire_values() {
        assert_eq!(Difficulty::from_wire("Easy"), Difficulty::Easy);
        assert_eq!(Difficulty::from_wire("medium"), Difficulty::Medium);
        assert_eq!(Difficulty::from_wire(" HARD "), Difficulty::Hard);
        assert_eq!(Difficulty::from_wire("Epic"), Difficulty::Unknown);
        assert_eq!(Difficulty::from_wire(""), Difficulty::Unknown);
    }

    #[test]
    fn problem_status_parses_wire_values() {
        assert_eq!(ProblemStatus::from_wire(Some("ac")), ProblemStatus::Solved);
        assert_eq!(
            ProblemStatus::from_wire(Some("notac")),
            ProblemStatus::Attempted
        );
        assert_eq!(ProblemStatus::from_wire(None), ProblemStatus::Todo);
        assert_eq!(ProblemStatus::from_wire(Some("weird")), ProblemStatus::Todo);
    }

    #[test]
    fn problem_status_markers_are_single_visible_glyphs() {
        for status in [
            ProblemStatus::Solved,
            ProblemStatus::Attempted,
            ProblemStatus::Todo,
        ] {
            let marker = status.marker();
            assert!(!marker.is_empty(), "{status:?} needs a marker");
            assert!(
                marker.chars().all(|c| !c.is_ascii_whitespace()),
                "{status:?} marker must be visible"
            );
        }
        // Distinct so the list is readable at a glance.
        assert_ne!(ProblemStatus::Solved.marker(), ProblemStatus::Todo.marker());
        assert_ne!(
            ProblemStatus::Attempted.marker(),
            ProblemStatus::Todo.marker()
        );
    }

    #[test]
    fn problem_status_filter_matches_leetcode_enum() {
        // These are the values `UserQuestionStatus` actually accepts; the
        // wire `status` values ("ac"/"notac") are different.
        assert_eq!(ProblemStatus::Solved.as_filter(), "AC");
        assert_eq!(ProblemStatus::Attempted.as_filter(), "TRIED");
        assert_eq!(ProblemStatus::Todo.as_filter(), "NOT_STARTED");
    }

    #[test]
    fn search_haystack_is_lowercase_and_searchable() {
        let problem = Problem {
            id: "1".into(),
            title: "Two Sum".into(),
            slug: "two-sum".into(),
            difficulty: Difficulty::Easy,
            paid_only: false,
            status: ProblemStatus::Todo,
            ac_rate: None,
            tags: vec![],
        };
        let hay = problem.search_haystack();
        assert!(hay.contains("two sum"));
        assert!(hay.contains("two-sum"));
        assert_eq!(hay, hay.to_ascii_lowercase());
    }

    #[test]
    fn user_status_display_name_falls_back() {
        let status = UserStatus::default();
        assert_eq!(status.display_name(), "(unknown)");
        let status = UserStatus {
            signed_in: true,
            username: "alice".into(),
            premium: false,
        };
        assert_eq!(status.display_name(), "alice");
        assert!(status.signed_in);
    }

    #[test]
    fn snippet_lookup_by_language_slug() {
        let detail = QuestionDetail {
            question_id: "1".into(),
            frontend_id: "1".into(),
            title: "Two Sum".into(),
            slug: "two-sum".into(),
            difficulty: Difficulty::Easy,
            paid_only: false,
            content_html: String::new(),
            sample_test_case: String::new(),
            example_testcases: String::new(),
            hints: vec![],
            tags: vec![],
            snippets: vec![
                CodeSnippet {
                    lang: "Python3".into(),
                    lang_slug: "python3".into(),
                    code: "class Solution: pass".into(),
                },
                CodeSnippet {
                    lang: "Rust".into(),
                    lang_slug: "rust".into(),
                    code: "impl Solution {}".into(),
                },
            ],
        };
        assert_eq!(detail.snippet_for("rust"), Some("impl Solution {}"));
        assert_eq!(detail.snippet_for("python3"), Some("class Solution: pass"));
        assert_eq!(detail.snippet_for("java"), None);
    }
}
