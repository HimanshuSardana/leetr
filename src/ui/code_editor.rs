//! The solution editor pane.
//!
//! Deliberately small: a monospace text area backed by a workspace file. The
//! file is the source of truth, so edits made here and edits made in nvim are
//! the same operation and neither side has to know about the other.

use egui::{RichText, TextEdit, Ui};

use crate::editor::{self, Buffer};
use crate::glyphs;
use crate::leetcode::Verdict;
use crate::theme;

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
            detail,
        }
    }
}

/// Colour for a verdict.
pub fn verdict_color(verdict: Verdict) -> egui::Color32 {
    if verdict.is_accepted() {
        theme::SUCCESS
    } else if verdict == Verdict::Pending {
        theme::WARNING
    } else {
        theme::DANGER
    }
}

/// Languages offered in the picker (the problem's snippets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageChoice {
    pub slug: String,
    pub label: String,
}

/// Draw the pane.
pub fn show(
    ui: &mut Ui,
    buffer: &mut Buffer,
    languages: &[LanguageChoice],
    dirty: bool,
    reloaded_from_disk: bool,
    judge: Option<&JudgeView>,
    can_submit: bool,
) -> CodeEditorAction {
    let mut action = CodeEditorAction::default();

    egui::Panel::top("code-header").show(ui, |ui| {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.strong("Solution");
            ui.separator();

            let current = languages
                .iter()
                .find(|choice| choice.slug == buffer.lang_slug)
                .map(|choice| choice.label.clone())
                .unwrap_or_else(|| buffer.lang_slug.clone());

            egui::ComboBox::from_id_salt("leetr-language")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for choice in languages {
                        let selected = choice.slug == buffer.lang_slug;
                        if ui.selectable_label(selected, &choice.label).clicked() && !selected {
                            action.language = Some(choice.slug.clone());
                        }
                    }
                });

            if dirty {
                ui.label(
                    RichText::new(format!("{} unsaved", glyphs::DOT))
                        .color(theme::WARNING)
                        .small(),
                );
            } else if reloaded_from_disk {
                ui.label(
                    RichText::new("reloaded from disk")
                        .color(theme::SUCCESS)
                        .small(),
                );
            }

            let running = judge.is_some_and(|view| view.running);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let submit = ui
                    .add_enabled(!running, egui::Button::new("Submit"))
                    .on_hover_text(if can_submit {
                        "Submit this solution to LeetCode"
                    } else {
                        "Only problems you have already solved can be submitted"
                    })
                    .on_disabled_hover_text("Waiting for the judge…");
                if submit.clicked() {
                    action.submit = true;
                }

                if ui
                    .add_enabled(!running, egui::Button::new("Run tests"))
                    .on_hover_text("Run the sample test cases")
                    .clicked()
                {
                    action.run = true;
                }

                if running {
                    ui.spinner();
                }

                if ui
                    .button("Open in nvim")
                    .on_hover_text("Open this file in your editor; saving there updates Leetr")
                    .clicked()
                {
                    action.open_in_nvim = true;
                }
                if ui
                    .button(format!("{} Reload", glyphs::REFRESH))
                    .on_hover_text("Discard in-app edits and re-read the file from disk")
                    .clicked()
                {
                    action.changed = false;
                    if let Ok(text) = std::fs::read_to_string(buffer.path()) {
                        buffer.set_text(text);
                    }
                }
            });
        });

        let full_path = buffer.path().display().to_string();
        ui.horizontal(|ui| {
            ui.label(RichText::new(&full_path).weak().small().monospace())
                .on_hover_text(&full_path);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} lines", buffer.text.lines().count().max(1)))
                        .weak()
                        .small(),
                );
            });
        });
        ui.add_space(4.0);
        ui.separator();
    });

    if let Some(judge) = judge {
        egui::Panel::bottom("leetr-judge")
            .resizable(true)
            .default_size(130.0)
            .size_range(egui::Rangef::new(60.0, 400.0))
            .show(ui, |ui| judge_panel(ui, judge));
    }

    egui::CentralPanel::no_frame().show(ui, |ui| {
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let response = ui.add(
                    TextEdit::multiline(&mut buffer.text)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(28)
                        .lock_focus(true),
                );
                if response.changed() {
                    action.changed = true;
                }
            });
    });

    action
}

fn judge_panel(ui: &mut Ui, judge: &JudgeView) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let what = if judge.is_submission {
            "Submission"
        } else {
            "Sample tests"
        };
        ui.strong(what);

        if judge.running {
            ui.spinner();
            ui.label(RichText::new("waiting for the judge…").weak());
        } else if let Some(err) = judge.error.as_ref() {
            ui.label(RichText::new(err).color(theme::DANGER));
        } else {
            ui.label(
                RichText::new(judge.verdict.label())
                    .color(verdict_color(judge.verdict))
                    .strong(),
            );
            if !judge.summary.is_empty() && judge.summary != judge.verdict.label() {
                ui.label(RichText::new(&judge.summary).weak().small());
            }
        }
    });

    if judge.detail.is_empty() {
        ui.add_space(2.0);
        return;
    }

    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (label, value) in &judge.detail {
                ui.label(RichText::new(label).weak().small());
                egui::Frame::default()
                    .fill(ui.visuals().code_bg_color)
                    .corner_radius(4.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(value).monospace().size(12.0));
                    });
                ui.add_space(4.0);
            }
        });
}

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
