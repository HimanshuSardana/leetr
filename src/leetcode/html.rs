//! A tiny, dependency-light HTML → rich-text converter for problem statements.
//!
//! LeetCode serves statements as HTML. Rather than embed a browser (or pull in
//! a full HTML engine) we parse the small subset of tags LeetCode actually
//! emits — paragraphs, headings, lists, `<pre>` examples, inline
//! `<strong>`/`<em>`/`<code>`, tables and rules — into [`Block`]s that both the
//! egui statement pane and the CLI text renderer can consume.
//!
//! Malformed or unknown markup is tolerated: unknown tags are transparent
//! (their text still flows through) and unbalanced tags never panic.

use serde::{Deserialize, Serialize};

/// Inline emphasis applied to a run of text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpanStyle {
    #[default]
    Normal,
    Bold,
    Italic,
    BoldItalic,
    Code,
}

impl SpanStyle {
    fn combine(self, other: Self) -> Self {
        use SpanStyle::*;
        match (self, other) {
            (BoldItalic, _) | (_, BoldItalic) => BoldItalic,
            (Bold, Italic) | (Italic, Bold) => BoldItalic,
            (Bold, _) | (_, Bold) => Bold,
            (Italic, _) | (_, Italic) => Italic,
            (Code, _) | (_, Code) => Code,
            _ => Normal,
        }
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "strong" | "b" => Some(Self::Bold),
            "em" | "i" => Some(Self::Italic),
            "code" | "kbd" | "samp" | "tt" => Some(Self::Code),
            _ => None,
        }
    }
}

/// A styled run of text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    pub style: SpanStyle,
}

impl Span {
    fn new(text: impl Into<String>, style: SpanStyle) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// A block-level element of a statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Block {
    Paragraph(Vec<Span>),
    Heading(Vec<Span>),
    ListItem(Vec<Span>),
    /// A `<pre>` block — monospace, whitespace preserved.
    Code(String),
    Rule,
}

/// Parse statement HTML into renderable blocks.
pub fn parse(html: &str) -> Vec<Block> {
    Parser::new().run(html)
}

/// Render blocks back to plain text (used by the CLI and by `problem.md`).
pub fn to_plain_text(blocks: &[Block]) -> String {
    blocks
        .iter()
        .map(render_block)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_block(block: &Block) -> String {
    match block {
        Block::Paragraph(spans) | Block::Heading(spans) => spans_text(spans),
        Block::ListItem(spans) => format!("  • {}", spans_text(spans)),
        Block::Code(code) => code.trim_end().to_string(),
        Block::Rule => "────────────────────────".to_string(),
    }
}

fn spans_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

// ---------------------------------------------------------------------------
// Tokenizer
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Tag {
        name: String,
        closing: bool,
        self_closing: bool,
    },
    Text(String),
}

fn tokenize(html: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    let mut i = 0usize;

    while i < html.len() {
        if html.as_bytes()[i] == b'<' {
            if let Some(offset) = html[i..].find('>') {
                let raw = &html[i + 1..i + offset];
                if !text.is_empty() {
                    tokens.push(Token::Text(std::mem::take(&mut text)));
                }
                push_tag(&mut tokens, raw);
                i += offset + 1;
                continue;
            }
            // No closing '>' — treat the rest as text.
        }

        let ch = html[i..]
            .chars()
            .next()
            .expect("index is on a char boundary");
        text.push(ch);
        i += ch.len_utf8();
    }

    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    tokens
}

fn push_tag(tokens: &mut Vec<Token>, raw: &str) {
    let raw = raw.trim();
    // Comments, doctypes and processing instructions carry no text.
    if raw.starts_with('!') || raw.starts_with('?') {
        return;
    }

    let closing = raw.starts_with('/');
    let body = raw.trim_start_matches('/').trim_end_matches('/');
    let name = body
        .split(|c: char| c.is_whitespace())
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();

    if name.is_empty() {
        return;
    }

    tokens.push(Token::Tag {
        name,
        closing,
        self_closing: raw.ends_with('/'),
    });
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Tags that end the current line before and after their content.
fn is_block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "section"
            | "article"
            | "table"
            | "tr"
            | "li"
            | "ul"
            | "ol"
    )
}

#[derive(Default)]
struct Parser {
    blocks: Vec<Block>,
    line: Vec<Span>,
    style: SpanStyle,
    style_stack: Vec<SpanStyle>,
    in_pre: bool,
    pre: String,
    /// Nesting depth of `<ul>`/`<ol>`; `> 0` makes lines list items.
    list_depth: usize,
}

impl Parser {
    fn new() -> Self {
        Self::default()
    }

    fn run(mut self, html: &str) -> Vec<Block> {
        for token in tokenize(html) {
            match token {
                Token::Text(text) => self.text(&text),
                Token::Tag {
                    name,
                    closing,
                    self_closing,
                } => {
                    if closing {
                        self.close_tag(&name);
                    } else {
                        self.open_tag(&name, self_closing);
                    }
                }
            }
        }
        self.flush();
        self.blocks
    }

    fn text(&mut self, raw: &str) {
        // `<>` and `&nbsp;` etc. arrive as text — decode entities either way.
        let decoded = html_escape::decode_html_entities(raw);

        if self.in_pre {
            self.pre.push_str(&decoded);
            return;
        }

        let collapsed = collapse_whitespace(&decoded);
        if collapsed.is_empty() {
            return;
        }

        if let Some(last) = self.line.last_mut() {
            if last.style == self.style {
                last.text.push_str(&collapsed);
                return;
            }
        }
        self.line.push(Span::new(collapsed, self.style));
    }

    fn open_tag(&mut self, tag: &str, self_closing: bool) {
        match tag {
            "pre" => {
                self.flush();
                self.in_pre = true;
                self.pre.clear();
                return;
            }
            "br" => {
                if self.in_pre {
                    self.pre.push('\n');
                } else {
                    self.flush();
                }
                return;
            }
            "hr" => {
                self.flush();
                self.blocks.push(Block::Rule);
                return;
            }
            "ul" | "ol" => {
                self.flush();
                self.list_depth += 1;
                return;
            }
            "li" => {
                self.flush();
                return;
            }
            "td" | "th" => {
                if !self.line.is_empty() {
                    self.line.push(Span::new("  |  ", SpanStyle::Normal));
                }
                return;
            }
            "sup" => {
                // `10<sup>4</sup>` reads better as `10^4` in a plain text pane.
                // No early return: the shared code below keeps the style stack
                // balanced with the matching `</sup>`.
                if self.in_pre {
                    self.pre.push('^');
                } else {
                    self.line.push(Span::new("^", self.style));
                }
            }
            _ => {}
        }

        if is_block_tag(tag) {
            self.flush();
        }

        if self_closing {
            return;
        }

        if let Some(style) = SpanStyle::from_tag(tag) {
            self.style_stack.push(self.style);
            self.style = self.style.combine(style);
        } else if is_emphasis_container(tag) {
            // Track nesting so the matching close tag restores the style.
            self.style_stack.push(self.style);
        }
    }

    fn close_tag(&mut self, tag: &str) {
        match tag {
            "pre" => {
                if self.in_pre {
                    self.in_pre = false;
                    let code = std::mem::take(&mut self.pre);
                    if !code.trim().is_empty() {
                        self.blocks.push(Block::Code(code));
                    }
                }
                return;
            }
            "ul" | "ol" => {
                self.flush();
                self.list_depth = self.list_depth.saturating_sub(1);
                return;
            }
            "li" | "tr" => {
                self.flush();
                return;
            }
            _ => {}
        }

        if is_block_tag(tag) {
            self.flush();
        }

        if SpanStyle::from_tag(tag).is_some() || is_emphasis_container(tag) {
            self.style = self.style_stack.pop().unwrap_or(SpanStyle::Normal);
        }
    }

    /// Emit the pending line as a block.
    fn flush(&mut self) {
        if self.line.is_empty() {
            return;
        }

        let mut spans = std::mem::take(&mut self.line);

        // Drop the whitespace that HTML used purely for wrapping.
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        spans.retain(|s| !s.text.is_empty());

        if spans.is_empty() {
            return;
        }

        if self.list_depth > 0 {
            self.blocks.push(Block::ListItem(spans));
        } else {
            self.blocks.push(Block::Paragraph(spans));
        }
    }
}

fn is_emphasis_container(tag: &str) -> bool {
    matches!(tag, "span" | "font" | "sup" | "sub" | "a")
}

/// Collapse every run of whitespace into a single space.
///
/// Leading/trailing spaces are kept (as a single space) so that a space used
/// to separate two inline elements is not lost; the line boundaries are
/// trimmed later in [`Parser::flush`].
fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(blocks: &[Block]) -> String {
        to_plain_text(blocks)
    }

    #[test]
    fn paragraphs_are_split_and_whitespace_collapsed() {
        let blocks = parse("<p>Hello\n   world</p>\n<p>Second</p>");
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph(vec![Span::new("Hello world", SpanStyle::Normal)]),
                Block::Paragraph(vec![Span::new("Second", SpanStyle::Normal)]),
            ]
        );
    }

    #[test]
    fn inline_emphasis_is_tracked() {
        let blocks = parse("<p>a <strong>bold</strong> b <em>it</em> <code>c</code></p>");
        let Block::Paragraph(spans) = &blocks[0] else {
            panic!("expected paragraph");
        };
        let rendered: Vec<(&str, SpanStyle)> =
            spans.iter().map(|s| (s.text.as_str(), s.style)).collect();
        assert!(rendered.contains(&("bold", SpanStyle::Bold)));
        assert!(rendered.contains(&("it", SpanStyle::Italic)));
        assert!(rendered.contains(&("c", SpanStyle::Code)));
    }

    #[test]
    fn nested_emphasis_combines() {
        let blocks = parse("<p><strong><em>both</em></strong></p>");
        let Block::Paragraph(spans) = &blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(spans
            .iter()
            .any(|s| s.text == "both" && s.style == SpanStyle::BoldItalic));
    }

    #[test]
    fn emphasis_does_not_leak_across_elements() {
        let blocks = parse("<p><strong>bold</strong></p><p>plain</p>");
        let Block::Paragraph(second) = &blocks[1] else {
            panic!("expected paragraph");
        };
        assert_eq!(second[0].style, SpanStyle::Normal);
    }

    #[test]
    fn pre_blocks_preserve_newlines_and_strip_inner_tags() {
        let html = "<p>x</p><pre>\n<strong>Input:</strong> nums = [2,7]\n<strong>Output:</strong> [0,1]\n</pre>";
        let blocks = parse(html);
        let code = blocks
            .iter()
            .find_map(|b| match b {
                Block::Code(c) => Some(c.clone()),
                _ => None,
            })
            .expect("expected a code block");
        assert!(code.contains('\n'), "newlines must survive in <pre>");
        assert!(code.contains("Input: nums = [2,7]"));
        assert!(code.contains("Output: [0,1]"));
    }

    #[test]
    fn entities_are_decoded() {
        let blocks = parse("<p>&lt;html&gt; &amp; &quot;quotes&quot;&nbsp;done</p>");
        let text = text_of(&blocks);
        assert!(text.contains("<html> & \"quotes\" done"), "got: {text}");
    }

    #[test]
    fn entities_are_decoded_inside_pre() {
        let blocks = parse("<pre>if a &lt; b &amp;&amp; c</pre>");
        let Block::Code(code) = &blocks[0] else {
            panic!("expected code block");
        };
        assert_eq!(code, "if a < b && c");
    }

    #[test]
    fn lists_become_list_items() {
        let blocks = parse("<ul><li>one</li><li>two</li></ul>");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], Block::ListItem(_)));
        assert_eq!(text_of(&blocks), "  • one\n  • two");
    }

    #[test]
    fn rules_become_rule_blocks() {
        let blocks = parse("<p>a</p><hr><p>b</p>");
        assert!(blocks.iter().any(|b| matches!(b, Block::Rule)));
    }

    #[test]
    fn superscript_becomes_caret_notation() {
        let blocks = parse("<p>between 10<sup>4</sup> and 10<sup>5</sup></p>");
        assert!(text_of(&blocks).contains("10^4"));
        assert!(text_of(&blocks).contains("10^5"));
    }

    #[test]
    fn unknown_tags_are_transparent() {
        let blocks = parse("<p><custom-thing>kept</custom-thing></p>");
        assert!(text_of(&blocks).contains("kept"));
    }

    #[test]
    fn malformed_markup_does_not_panic() {
        for html in [
            "<p>unclosed",
            "</p>stray close",
            "<p><strong>x</p></strong>",
            "<",
            "<>",
            "<p",
            "",
            "<!-- comment --><p>after</p>",
            "<p>a<br>b</p>",
        ] {
            let blocks = parse(html);
            let _ = text_of(&blocks);
        }
    }

    #[test]
    fn table_cells_get_separators() {
        let html = "<table><tr><td>a</td><td>b</td></tr></table>";
        let text = text_of(&parse(html));
        assert!(text.contains('a') && text.contains('b'));
        assert!(
            text.contains('|'),
            "cells should be visually separated: {text}"
        );
    }

    #[test]
    fn real_two_sum_statement_renders() {
        // Trimmed from the live API response for `two-sum`.
        let html = r#"<p>You are given an array of integers <code>nums</code>&nbsp;and an integer <code>target</code>, return <em>indices of the two numbers such that they add up to <code>target</code></em>.</p>

<p>You may assume that each input would have <strong><em>exactly</em> one solution</strong>, and you may not use the <em>same</em> element twice.</p>

<p>&nbsp;</p>
<p><strong class="example">Example 1:</strong></p>

<pre>
<strong>Input:</strong> nums = [2,7,11,15], target = 9
<strong>Output:</strong> [0,1]
<strong>Explanation:</strong> Because nums[0] + nums[1] == 9, we return [0, 1].
</pre>

<p><strong>Constraints:</strong></p>
<ul>
	<li><code>2 &lt;= nums.length &lt;= 10<sup>4</sup></code></li>
	<li><code>-10<sup>9</sup> &lt;= nums[i] &lt;= 10<sup>9</sup></code></li>
</ul>"#;

        let blocks = parse(html);
        let text = text_of(&blocks);

        assert!(text.contains("You are given an array of integers"));
        assert!(text.contains("Example 1:"));
        assert!(text.contains("Input: nums = [2,7,11,15], target = 9"));
        assert!(text.contains("Constraints:"));
        // The `<` inside constraints must be decoded, and `10^4` superscripted.
        assert!(text.contains("2 <= nums.length <= 10^4"), "got: {text}");
        // No raw markup or undecoded entities should survive. A bare `<` is
        // legitimate here — it comes from decoding `&lt;` in the constraints.
        for leak in ["<p>", "</", "&lt;", "&nbsp;", "&quot;"] {
            assert!(!text.contains(leak), "`{leak}` leaked into output: {text}");
        }
    }

    #[test]
    fn to_plain_text_joins_blocks_with_single_newlines() {
        let blocks = parse("<p>one</p><p>two</p>");
        assert_eq!(text_of(&blocks), "one\ntwo");
    }
}
