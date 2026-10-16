//! The solution editor pane.
//!
//! Deliberately small: a workspace-file-backed solution buffer plus the judge
//! verdict view model. The file is the source of truth, so edits made here and
//! edits made in nvim are the same operation and neither side has to know
//! about the other. Rendering lives in the GPUI views (`crate::app`).

use crate::editor;
use crate::leetcode::Verdict;

/// What the user asked the pane to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CodeEditorAction {
    pub open_in_nvim: bool,
    /// The text changed this frame (the caller decides when to persist).
    pub changed: bool,
    /// The user asked for an explicit language switch.
    pub language: Option<String>,
    /// Run the sample tests.
    pub run: bool,
    /// Submit the solution.
    pub submit: bool,
}

/// The latest judge result, rendered under the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JudgeView {
    /// True for a real submission, false for a sample run.
    pub is_submission: bool,
    pub running: bool,
    pub verdict: Verdict,
    pub summary: String,
    pub error: Option<String>,
    /// Shown when the request proceeded despite the safety rail being off.
    pub warning: Option<String>,
    /// Extra rows: last test case, produced/expected output, compile errors…
    pub detail: Vec<(String, String)>,
}

impl Default for JudgeView {
    fn default() -> Self {
        Self {
            is_submission: false,
            running: false,
            verdict: Verdict::Pending,
            summary: String::new(),
            error: None,
            warning: None,
            detail: Vec::new(),
        }
    }
}

impl JudgeView {
    /// Build a view from a finished judge result.
    pub fn from_result(result: &crate::leetcode::JudgeResult, is_submission: bool) -> Self {
        let mut detail = Vec::new();
        if let Some(text) = result.error_text() {
            detail.push(("Error".to_string(), text));
        }
        if let Some(case) = result
            .last_testcase
            .as_ref()
            .filter(|t| !t.trim().is_empty())
        {
            detail.push(("Input".to_string(), case.clone()));
        }
        if let Some(output) = result.output_text() {
            detail.push(("Output".to_string(), output));
        }
        if let Some(expected) = result.expected_text() {
            detail.push(("Expected".to_string(), expected));
        }
        if let Some(memory) = result.memory_text() {
            detail.push(("Memory".to_string(), memory));
        }

        Self {
            is_submission,
            running: false,
            verdict: result.verdict(),
            summary: result.summary(),
            error: None,
            warning: None,
            detail,
        }
    }
}

/// Packed `0xRRGGBB` for a verdict.
pub fn verdict_hex(verdict: Verdict) -> u32 {
    if verdict.is_accepted() {
        crate::theme::SUCCESS_HEX
    } else if verdict == Verdict::Pending {
        crate::theme::WARNING_HEX
    } else {
        crate::theme::DANGER_HEX
    }
}

/// Languages offered in the picker (the problem's snippets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageChoice {
    pub slug: String,
    pub label: String,
}

/// Draw the pane.
/// Build the language picker options from a problem's snippets.
pub fn language_choices(detail: &crate::model::QuestionDetail) -> Vec<LanguageChoice> {
    let mut choices: Vec<LanguageChoice> = detail
        .snippets
        .iter()
        .map(|snippet| LanguageChoice {
            slug: snippet.lang_slug.clone(),
            label: if snippet.lang.is_empty() {
                snippet.lang_slug.clone()
            } else {
                snippet.lang.clone()
            },
        })
        .collect();
    choices.sort_by_key(|choice| choice.label.to_lowercase());
    choices
}

/// The language to open a problem with: the configured default when the
/// problem offers it, otherwise the first available snippet.
pub fn preferred_language(detail: &crate::model::QuestionDetail, configured: &str) -> String {
    if detail.snippet_for(configured).is_some() {
        return configured.to_string();
    }
    detail
        .snippets
        .first()
        .map(|snippet| snippet.lang_slug.clone())
        .unwrap_or_else(|| configured.to_string())
}

/// Filename shown in the header, e.g. `solution.rs`.
pub fn file_name(lang_slug: &str) -> String {
    format!("solution.{}", editor::extension_for(lang_slug))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CodeSnippet, Difficulty, QuestionDetail};

    fn detail(slugs: &[&str]) -> QuestionDetail {
        QuestionDetail {
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
            snippets: slugs
                .iter()
                .map(|slug| CodeSnippet {
                    lang: slug.to_uppercase(),
                    lang_slug: (*slug).to_string(),
                    code: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn language_choices_are_sorted_and_labelled() {
        let choices = language_choices(&detail(&["rust", "cpp", "python3"]));
        let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["CPP", "PYTHON3", "RUST"]);
    }

    #[test]
    fn a_language_without_a_name_falls_back_to_its_slug() {
        let mut problem = detail(&["rust"]);
        problem.snippets[0].lang.clear();
        let choices = language_choices(&problem);
        assert_eq!(choices[0].label, "rust");
    }

    #[test]
    fn preferred_language_uses_the_configured_default_when_available() {
        let problem = detail(&["rust", "python3"]);
        assert_eq!(preferred_language(&problem, "python3"), "python3");
    }

    #[test]
    fn preferred_language_falls_back_to_the_first_snippet() {
        let problem = detail(&["rust", "python3"]);
        assert_eq!(preferred_language(&problem, "haskell"), "rust");
    }

    #[test]
    fn preferred_language_keeps_the_configured_value_when_there_are_no_snippets() {
        let problem = detail(&[]);
        assert_eq!(preferred_language(&problem, "python3"), "python3");
    }

    #[test]
    fn file_names_use_the_language_extension() {
        assert_eq!(file_name("rust"), "solution.rs");
        assert_eq!(file_name("python3"), "solution.py");
    }
}
