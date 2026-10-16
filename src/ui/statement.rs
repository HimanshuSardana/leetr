//! The statement pane: renders a loaded problem, its examples and constraints.
//!
//! Rendering lives in the GPUI views (`crate::app`): this module converts the
//! parsed HTML blocks into plain text the views can lay out.

use crate::leetcode::html::{Block, Span, SpanStyle};
use crate::model::QuestionDetail;

/// Plain-text rendering of one inline span run.
pub fn inline_text(spans: &[Span]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

/// Plain-text rendering of one block.
pub fn block_text(block: &Block) -> String {
    match block {
        Block::Paragraph(spans) | Block::Heading(spans) | Block::ListItem(spans) => {
            inline_text(spans)
        }
        Block::Code(code) => code.trim_end().to_string(),
        Block::Image { alt, url } => {
            if alt.trim().is_empty() {
                format!("[image: {url}]")
            } else {
                format!("[image {}: {url}]", alt.trim())
            }
        }
        Block::Rule => "---".to_string(),
    }
}

/// Plain-text rendering of a whole statement.
pub fn blocks_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(block_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Whether a span run should be drawn in the code style.
pub fn is_code(span: &Span) -> bool {
    matches!(span.style, SpanStyle::Code)
}

/// Whether a span run should be drawn emphasised.
pub fn is_emphasis(span: &Span) -> bool {
    matches!(
        span.style,
        SpanStyle::Bold | SpanStyle::Italic | SpanStyle::BoldItalic
    )
}

/// Human-readable header line, e.g. `"1. Two Sum"`.
pub fn title_line(detail: &QuestionDetail) -> String {
    format!("{}. {}", detail.frontend_id, detail.title)
}

/// Placeholder shown when no problem is selected.
pub fn empty_message(loading: bool) -> &'static str {
    if loading {
        "Loading statement…"
    } else {
        "Select a problem to read its statement."
    }
}

/// Placeholder when the statement body is missing.
pub fn missing_body_message(detail: &QuestionDetail) -> &'static str {
    if detail.paid_only {
        "This is a premium problem — the statement is not available."
    } else {
        "LeetCode returned an empty statement."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leetcode::html;

    #[test]
    fn inline_text_preserves_the_text() {
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
        assert_eq!(inline_text(&spans), "hello bold");
    }

    #[test]
    fn span_style_helpers_classify() {
        let code = Span {
            text: "x".into(),
            style: SpanStyle::Code,
        };
        let bold = Span {
            text: "b".into(),
            style: SpanStyle::Bold,
        };
        let normal = Span {
            text: "n".into(),
            style: SpanStyle::Normal,
        };
        assert!(is_code(&code));
        assert!(!is_code(&bold));
        assert!(is_emphasis(&bold));
        assert!(!is_emphasis(&normal));
    }

    #[test]
    fn statement_blocks_render_to_text() {
        let blocks =
            html::parse("<p>Intro with <code>x</code>.</p><ul><li>a</li></ul><pre>code here</pre>");
        let text = blocks_text(&blocks);
        assert!(text.contains("Intro with"));
        assert!(text.contains("code here"));
        for block in &blocks {
            assert!(!block_text(block).is_empty() || matches!(block, Block::Rule));
        }
    }

    #[test]
    fn empty_and_missing_messages_are_present() {
        assert!(!empty_message(true).is_empty());
        assert!(!empty_message(false).is_empty());
        let mut detail = QuestionDetail {
            question_id: "1".into(),
            frontend_id: "1".into(),
            title: "Two Sum".into(),
            slug: "two-sum".into(),
            difficulty: crate::model::Difficulty::Easy,
            paid_only: true,
            content_html: String::new(),
            sample_test_case: String::new(),
            example_testcases: String::new(),
            hints: vec![],
            tags: vec![],
            snippets: vec![],
        };
        assert!(missing_body_message(&detail).contains("premium"));
        detail.paid_only = false;
        assert!(!missing_body_message(&detail).is_empty());
        assert_eq!(title_line(&detail), "1. Two Sum");
    }
}
