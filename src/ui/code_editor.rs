//! The solution editor pane.
//!
//! Deliberately small: a monospace text area backed by a workspace file. The
//! file is the source of truth, so edits made here and edits made in nvim are
//! the same operation and neither side has to know about the other.

use egui::{RichText, TextEdit, Ui};

use crate::editor::{self, Buffer};
use crate::glyphs;
use crate::theme;

/// What the user asked the pane to do.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CodeEditorAction {
    pub open_in_nvim: bool,
    /// The text changed this frame (the caller decides when to persist).
    pub changed: bool,
    /// The user asked for an explicit language switch.
    pub language: Option<String>,
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

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
