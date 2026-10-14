//! The statement pane: renders a loaded problem, its examples and constraints.

use egui::text::LayoutJob;
use egui::{FontId, RichText, TextFormat, Ui};

use crate::glyphs;
use crate::leetcode::html::{Block, Span, SpanStyle};
use crate::model::QuestionDetail;
use crate::theme;

const BODY_SIZE: f32 = 14.0;
const HEADING_SIZE: f32 = 17.0;
const CODE_SIZE: f32 = 13.0;

/// Draw the pane.
pub fn show(
    ui: &mut Ui,
    detail: Option<&QuestionDetail>,
    blocks: &[Block],
    loading: bool,
    error: Option<&str>,
) {
    if let Some(err) = error {
        ui.add_space(8.0);
        ui.colored_label(
            theme::DANGER,
            format!("Could not load the statement: {err}"),
        );
        return;
    }

    let Some(detail) = detail else {
        ui.vertical_centered(|ui| {
            ui.add_space(40.0);
            if loading {
                ui.spinner();
                ui.label(RichText::new("Loading statement…").weak());
            } else {
                ui.label(RichText::new("Select a problem to read its statement.").weak());
            }
        });
        return;
    };

    egui::Panel::top("statement-header").show(ui, |ui| {
        header(ui, detail);
    });

    egui::CentralPanel::no_frame().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(6.0);

                if loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Refreshing…").weak().small());
                    });
                }

                if detail.content_html.trim().is_empty() {
                    ui.label(
                        RichText::new(if detail.paid_only {
                            "This is a premium problem — the statement is not available."
                        } else {
                            "LeetCode returned an empty statement."
                        })
                        .color(theme::WARNING),
                    );
                }

                for block in blocks {
                    render_block(ui, block);
                }

                if !detail.hints.is_empty() {
                    ui.add_space(10.0);
                    ui.separator();
                    egui::CollapsingHeader::new(format!("Hints ({})", detail.hints.len()))
                        .default_open(false)
                        .show(ui, |ui| {
                            for (index, hint) in detail.hints.iter().enumerate() {
                                let hint_blocks = crate::leetcode::html::parse(hint);
                                ui.label(RichText::new(format!("{}.", index + 1)).weak().small());
                                for block in &hint_blocks {
                                    render_block(ui, block);
                                }
                                ui.add_space(6.0);
                            }
                        });
                }

                if !detail.snippets.is_empty() {
                    ui.add_space(10.0);
                    ui.separator();
                    egui::CollapsingHeader::new("Starter code")
                        .default_open(false)
                        .show(ui, |ui| {
                            for snippet in &detail.snippets {
                                egui::CollapsingHeader::new(&snippet.lang)
                                    .id_salt(&snippet.lang_slug)
                                    .default_open(false)
                                    .show(ui, |ui| {
                                        code_block(ui, &snippet.code);
                                    });
                            }
                        });
                }

                ui.add_space(16.0);
            });
    });
}

fn header(ui: &mut Ui, detail: &QuestionDetail) {
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&detail.frontend_id).weak().monospace());
        ui.heading(&detail.title);
    });

    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(detail.difficulty.label())
                .color(theme::difficulty_color(detail.difficulty))
                .strong(),
        );

        if detail.paid_only {
            ui.label(RichText::new("★ premium").color(theme::ACCENT).small());
        }

        for tag in &detail.tags {
            ui.label(RichText::new(&tag.name).weak().small());
        }
    });
    ui.add_space(4.0);
    ui.separator();
}

fn render_block(ui: &mut Ui, block: &Block) {
    match block {
        Block::Paragraph(spans) => {
            if !spans.is_empty() {
                ui.label(inline_job(spans, BODY_SIZE, ui));
            }
            ui.add_space(5.0);
        }
        Block::Heading(spans) => {
            ui.add_space(4.0);
            ui.label(inline_job(spans, HEADING_SIZE, ui));
            ui.add_space(3.0);
        }
        Block::ListItem(spans) => {
            // The marker ("• " or "3. ") is carried in the spans.
            ui.horizontal_wrapped(|ui| {
                ui.add_space(8.0);
                ui.label(inline_job(spans, BODY_SIZE, ui));
            });
        }
        Block::Code(code) => code_block(ui, code),
        Block::Image { alt, url } => {
            // Deliberately not fetched — that would mean an image decoder and
            // an HTTP loader. But it must be visible: several statements carry
            // a diagram the prose refers to.
            let label = if alt.trim().is_empty() {
                format!("{} image", glyphs::BULLET)
            } else {
                format!("{} image: {}", glyphs::BULLET, alt.trim())
            };
            ui.label(RichText::new(label).weak().italics().small())
                .on_hover_text(url.as_str());
            ui.label(RichText::new(url.as_str()).weak().small().monospace());
            ui.add_space(4.0);
        }
        Block::Rule => {
            ui.add_space(2.0);
            ui.separator();
            ui.add_space(2.0);
        }
    }
}

fn code_block(ui: &mut Ui, code: &str) {
    let fill = ui.visuals().code_bg_color;
    egui::Frame::default()
        .fill(fill)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(code.trim_end()).monospace().size(CODE_SIZE));
        });
    ui.add_space(6.0);
}

/// Build a wrapping rich-text job for one run of styled spans.
fn inline_job(spans: &[Span], size: f32, ui: &Ui) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width().max(120.0);

    let text_color = ui.visuals().text_color();
    let strong_color = ui.visuals().strong_text_color();
    let code_bg = ui.visuals().code_bg_color;

    for span in spans {
        let format = match span.style {
            SpanStyle::Normal => TextFormat {
                font_id: FontId::proportional(size),
                color: text_color,
                ..Default::default()
            },
            SpanStyle::Bold => TextFormat {
                font_id: FontId::proportional(size),
                color: strong_color,
                ..Default::default()
            },
            SpanStyle::Italic => TextFormat {
                font_id: FontId::proportional(size),
                color: text_color,
                italics: true,
                ..Default::default()
            },
            SpanStyle::BoldItalic => TextFormat {
                font_id: FontId::proportional(size),
                color: strong_color,
                italics: true,
                ..Default::default()
            },
            SpanStyle::Code => TextFormat {
                font_id: FontId::monospace(size * 0.93),
                color: theme::ACCENT,
                background: code_bg,
                ..Default::default()
            },
        };
        job.append(&span.text, 0.0, format);
    }

    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leetcode::html;

    /// Run one egui pass and consume its texture deltas.
    ///
    /// `Context::run_ui` hands back a `FullOutput`; dropping it without
    /// handling `textures_delta` panics inside epaint.
    fn run(ctx: &egui::Context, mut add_contents: impl FnMut(&mut Ui)) {
        let mut output = ctx.run_ui(Default::default(), |ui| add_contents(ui));
        output.textures_delta.clear();
    }

    #[test]
    fn inline_job_preserves_the_text() {
        let ctx = egui::Context::default();
        run(&ctx, |ui| {
            let spans = vec![
                Span {
                    text: "hello ".into(),
                    style: SpanStyle::Normal,
                },
                Span {
                    text: "bold".into(),
                    style: SpanStyle::Bold,
                },
            ];
            let job = inline_job(&spans, BODY_SIZE, ui);
            assert_eq!(job.text, "hello bold");
            assert_eq!(job.sections.len(), 2);
        });
    }

    #[test]
    fn inline_job_marks_up_emphasis_and_code() {
        let ctx = egui::Context::default();
        run(&ctx, |ui| {
            let spans = vec![
                Span {
                    text: "i".into(),
                    style: SpanStyle::Italic,
                },
                Span {
                    text: "b".into(),
                    style: SpanStyle::Bold,
                },
                Span {
                    text: "nums".into(),
                    style: SpanStyle::Code,
                },
            ];
            let job = inline_job(&spans, BODY_SIZE, ui);

            assert!(job.sections[0].format.italics);
            assert!(!job.sections[1].format.italics);
            assert_eq!(
                job.sections[2].format.font_id.family,
                egui::FontFamily::Monospace
            );
            assert_eq!(
                job.sections[2].format.background,
                ui.visuals().code_bg_color
            );
        });
    }

    #[test]
    fn statement_blocks_render_without_panicking() {
        let blocks =
            html::parse("<p>Intro with <code>x</code>.</p><ul><li>a</li></ul><pre>code here</pre>");
        let ctx = egui::Context::default();
        run(&ctx, |ui| {
            for block in &blocks {
                render_block(ui, block);
            }
        });
    }

    #[test]
    fn show_handles_every_state_without_panicking() {
        let blocks = html::parse("<p>hi</p>");
        let ctx = egui::Context::default();

        run(&ctx, |ui| show(ui, None, &[], false, None));
        run(&ctx, |ui| show(ui, None, &[], true, None));
        run(&ctx, |ui| show(ui, None, &blocks, false, Some("boom")));
    }
}
