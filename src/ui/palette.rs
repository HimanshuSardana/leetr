//! A fuzzy-search command palette, shared by `Ctrl+K` (actions) and `Ctrl+P`
//! (problem selector).
//!
//! The matching logic is a pure function so it can be tested exhaustively
//! without an egui context.

use egui::text::LayoutJob;
use egui::{Color32, FontId, Key, RichText, TextEdit, TextFormat, Ui};

/// Width of the palette popup.
const PALETTE_WIDTH: f32 = 620.0;
/// Height of the scrollable result list.
const LIST_HEIGHT: f32 = 330.0;
/// Upper bound on rendered rows, so a 4000-entry list stays instant.
const MAX_ROWS: usize = 200;

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Score a case-insensitive fuzzy subsequence match.
///
/// Returns `None` when `query` is not a subsequence of `haystack`. Higher
/// scores are better. Word-boundary matches and contiguous runs are rewarded;
/// gaps and long haystacks are penalised, so "ts" ranks `Two Sum` above
/// `Add Two Numbers`.
pub fn fuzzy_match(query: &str, haystack: &str) -> Option<i32> {
    let needle: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    if needle.is_empty() {
        return Some(0);
    }

    let hay: Vec<char> = haystack.to_lowercase().chars().collect();
    let mut score: i32 = 0;
    let mut matched = 0usize;
    let mut previous: Option<usize> = None;

    for (index, ch) in hay.iter().enumerate() {
        if matched >= needle.len() {
            break;
        }
        if *ch != needle[matched] {
            continue;
        }

        score += 10;

        match previous {
            // Contiguous run — strongly prefer it.
            Some(prev) if prev + 1 == index => score += 8,
            // Otherwise pay for the gap we skipped.
            Some(prev) => score -= ((index - prev - 1) as i32).min(10),
            // Very first matched character.
            None => score += 6,
        }

        // Matching the first letter of a word is a strong signal.
        if index == 0 || is_separator(hay[index - 1]) {
            score += 12;
        }

        previous = Some(index);
        matched += 1;
    }

    if matched != needle.len() {
        return None;
    }

    // Exact substring matches always beat scattered subsequences.
    let needle_string: String = needle.iter().collect();
    if hay.iter().collect::<String>().contains(&needle_string) {
        score += 30;
    }

    // Prefer shorter candidates when everything else ties.
    score -= (hay.len() / 8) as i32;

    Some(score)
}

fn is_separator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, '-' | '_' | '.' | '/' | '(' | '[')
}

/// Indices of `candidates` matching `query`, best match first.
///
/// An empty query returns every index in its original order.
pub fn rank<S: AsRef<str>>(query: &str, candidates: &[S]) -> Vec<usize> {
    if query.trim().is_empty() {
        return (0..candidates.len()).collect();
    }

    let mut scored: Vec<(usize, i32)> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, text)| fuzzy_match(query, text.as_ref()).map(|score| (index, score)))
        .collect();

    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(index, _)| index).collect()
}

// ---------------------------------------------------------------------------
// Palette data
// ---------------------------------------------------------------------------

/// One selectable row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub label: String,
    /// Secondary text shown after the label (and searched too).
    pub hint: String,
    /// Optional glyph, drawn with the monospace font (see [`crate::glyphs`]).
    pub icon: Option<String>,
}

impl PaletteEntry {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            hint: String::new(),
            icon: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    fn search_text(&self) -> String {
        if self.hint.is_empty() {
            self.label.clone()
        } else {
            format!("{} {}", self.label, self.hint)
        }
    }
}

/// Open/closed state and cursor of one palette.
#[derive(Debug, Default, Clone)]
pub struct PaletteState {
    open: bool,
    query: String,
    selected: usize,
    /// Set for one frame after opening so the text field takes focus.
    grab_focus: bool,
    /// Set when the selection moved by keyboard so the list scrolls to it.
    scroll_to_selection: bool,
}

impl PaletteState {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open with a clean query.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.grab_focus = true;
        self.scroll_to_selection = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.grab_focus = false;
        self.scroll_to_selection = false;
    }

    pub fn query(&self) -> &str {
        &self.query
    }
}

/// What the user did with the palette this frame.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PaletteOutcome {
    /// Index into the `entries` slice that was passed in.
    pub chosen: Option<usize>,
    pub closed: bool,
    /// The caller-supplied fallback action was triggered (see `empty_action`).
    pub empty_action: bool,
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Draw the palette as a modal overlay. Returns the user's choice, if any.
pub fn show(
    ctx: &egui::Context,
    state: &mut PaletteState,
    id: &str,
    placeholder: &str,
    entries: &[PaletteEntry],
    empty_action: Option<&str>,
) -> PaletteOutcome {
    if !state.open {
        return PaletteOutcome::default();
    }

    let texts: Vec<String> = entries.iter().map(PaletteEntry::search_text).collect();
    let matches = rank(&state.query, &texts);
    if state.selected >= matches.len() {
        state.selected = matches.len().saturating_sub(1);
    }

    let screen = ctx.viewport_rect();
    let position = egui::pos2(
        (screen.center().x - PALETTE_WIDTH / 2.0).max(screen.left() + 8.0),
        screen.top() + 76.0,
    );
    let modal_id = egui::Id::new(("leetr-palette", id));

    // Keyboard navigation is read before the widgets so a single key press
    // cannot both move the cursor and activate a row.
    let (moved_up, moved_down, activate) = ctx.input(|i| {
        (
            i.key_pressed(Key::ArrowUp) || (i.key_pressed(Key::Tab) && i.modifiers.shift),
            i.key_pressed(Key::ArrowDown) || (i.key_pressed(Key::Tab) && !i.modifiers.shift),
            i.key_pressed(Key::Enter),
        )
    });

    if moved_up && !matches.is_empty() {
        state.selected = state.selected.checked_sub(1).unwrap_or(matches.len() - 1);
        state.scroll_to_selection = true;
    }
    if moved_down && !matches.is_empty() {
        state.selected = (state.selected + 1) % matches.len();
        state.scroll_to_selection = true;
    }

    // Build on `Modal::default_area` rather than a bare `Area`: it carries
    // `UiKind::Modal` and `Order::Foreground`, without which the modal is not
    // drawn above the panels.
    let response = egui::Modal::new(modal_id)
        .backdrop_color(Color32::from_black_alpha(120))
        .area(egui::Modal::default_area(modal_id).fixed_pos(position))
        .show(ctx, |ui| {
            let mut outcome = PaletteOutcome::default();
            let mut empty_action_clicked = false;
            ui.set_width(PALETTE_WIDTH);

            let edit = ui.add(
                TextEdit::singleline(&mut state.query)
                    .id(modal_id.with("query"))
                    .hint_text(placeholder)
                    .desired_width(f32::INFINITY),
            );
            if std::mem::take(&mut state.grab_focus) {
                edit.request_focus();
            }
            if edit.changed() {
                state.selected = 0;
            }

            ui.add_space(4.0);
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if matches.is_empty() {
                        ui.add_space(8.0);
                        match empty_action {
                            Some(label) => {
                                if ui.button(label).clicked() {
                                    empty_action_clicked = true;
                                }
                            }
                            None => {
                                ui.label(RichText::new("No matches").weak());
                            }
                        }
                        ui.add_space(8.0);
                        return;
                    }

                    for (row, &entry_index) in matches.iter().enumerate().take(MAX_ROWS) {
                        let entry = &entries[entry_index];
                        let is_selected = row == state.selected;
                        let job = row_job(entry, ui, is_selected);

                        let response = ui.selectable_label(is_selected, job);
                        if is_selected && state.scroll_to_selection {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            outcome.chosen = Some(entry_index);
                        }
                    }
                });

            state.scroll_to_selection = false;

            if activate {
                if let Some(&entry_index) = matches.get(state.selected) {
                    outcome.chosen = Some(entry_index);
                }
            }
            if empty_action_clicked || (activate && matches.is_empty() && empty_action.is_some()) {
                outcome.empty_action = true;
            }

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                // Deliberately plain text: arrow and return glyphs are absent
                // from egui's default fonts (see `crate::glyphs`).
                ui.label(
                    RichText::new("Up/Down navigate · Enter select · Esc close")
                        .weak()
                        .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} of {}", matches.len(), entries.len()))
                            .weak()
                            .small(),
                    );
                });
            });

            outcome
        });

    let should_close = response.should_close();
    let mut outcome = response.inner;
    if should_close {
        outcome.closed = true;
    }
    if outcome.closed {
        state.close();
    }
    outcome
}

fn row_job(entry: &PaletteEntry, ui: &Ui, is_selected: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = ui.available_width().max(200.0);

    let text_color = if is_selected {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    let hint_color = ui.visuals().weak_text_color();

    if let Some(icon) = &entry.icon {
        job.append(
            icon,
            0.0,
            TextFormat {
                font_id: FontId::monospace(14.0),
                color: text_color,
                ..Default::default()
            },
        );
        job.append(
            " ",
            0.0,
            TextFormat {
                font_id: FontId::proportional(14.0),
                color: text_color,
                ..Default::default()
            },
        );
    }

    job.append(
        &entry.label,
        0.0,
        TextFormat {
            font_id: FontId::proportional(14.0),
            color: text_color,
            ..Default::default()
        },
    );

    if !entry.hint.is_empty() {
        job.append(
            &format!("  {}", entry.hint),
            0.0,
            TextFormat {
                font_id: FontId::proportional(12.0),
                color: hint_color,
                ..Default::default()
            },
        );
    }

    job
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs;

    fn entry(label: &str, hint: &str) -> PaletteEntry {
        PaletteEntry::new(label).with_hint(hint)
    }

    #[test]
    fn empty_query_matches_everything_in_order() {
        let texts = vec!["one".to_string(), "two".to_string()];
        assert_eq!(rank("", &texts), vec![0, 1]);
        assert_eq!(rank("   ", &texts), vec![0, 1]);
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert!(fuzzy_match("xyz", "two sum").is_none());
        assert!(fuzzy_match("mt", "two sum").is_none());
        assert!(rank("zzz", &["two-sum".to_string(), "matrix".to_string()]).is_empty());
    }

    #[test]
    fn subsequences_match() {
        assert!(fuzzy_match("ts", "two sum").is_some());
        assert!(fuzzy_match("twosum", "two sum").is_some());
        assert!(fuzzy_match("t", "Two Sum").is_some());
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(fuzzy_match("TWO", "two"), fuzzy_match("two", "two"));
        assert!(fuzzy_match("TwoSum", "two-sum").is_some());
    }

    #[test]
    fn exact_substring_outranks_scattered_subsequence() {
        let exact = fuzzy_match("sum", "two sum").unwrap();
        let scattered = fuzzy_match("sum", "s u m").unwrap_or(i32::MIN);
        assert!(exact > scattered);
    }

    #[test]
    fn word_boundary_matches_outrank_mid_word_matches() {
        // Same gap in both, but the second character starts a word in the first.
        let boundary = fuzzy_match("tz", "t z").unwrap();
        let embedded = fuzzy_match("tz", "txz").unwrap();
        assert!(
            boundary > embedded,
            "boundary={boundary} embedded={embedded}"
        );
    }

    #[test]
    fn contiguity_is_rewarded() {
        let contiguous = fuzzy_match("sum", "sum").unwrap();
        let gapped = fuzzy_match("sum", "s---u---m").unwrap();
        assert!(contiguous > gapped);
    }

    #[test]
    fn ranking_puts_the_strongest_match_first() {
        let texts = vec![
            "Add Two Numbers".to_string(),
            "Two Sum".to_string(),
            "Median of Two Sorted Arrays".to_string(),
        ];

        // "Two Sum" is a prefix match and shorter than the others, so it wins.
        let ranked = rank("two", &texts);
        assert_eq!(
            ranked.first(),
            Some(&1),
            "Two Sum should rank first: {ranked:?}"
        );
        assert_eq!(ranked.len(), 3, "all three contain 'two': {ranked:?}");
    }

    #[test]
    fn subsequence_matching_requires_every_character_in_order() {
        // There is no second 'u' after the 's' in "add two numbers".
        assert!(fuzzy_match("twosum", "add two numbers").is_none());
        // ...whereas "two sum" has them in order.
        assert!(fuzzy_match("twosum", "two sum").is_some());
    }

    #[test]
    fn ranking_is_by_score_then_original_order() {
        let texts = vec!["a".to_string(), "a".to_string(), "a".to_string()];
        assert_eq!(rank("a", &texts), vec![0, 1, 2]);
    }

    #[test]
    fn search_text_includes_the_hint() {
        let e = entry("Two Sum", "Easy");
        assert!(e.search_text().contains("Two Sum"));
        assert!(e.search_text().contains("Easy"));
    }

    #[test]
    fn palette_state_opens_clean_and_closes_clean() {
        let mut state = PaletteState::default();
        assert!(!state.is_open());

        state.open();
        assert!(state.is_open());
        assert_eq!(state.query(), "");
        assert_eq!(state.selected, 0);

        state.query = "abc".into();
        state.selected = 5;
        state.close();
        assert!(!state.is_open());
        assert_eq!(state.query(), "");
        assert_eq!(state.selected, 0);
        assert!(!state.grab_focus);
    }

    #[test]
    fn palette_renders_without_panicking() {
        let entries = vec![
            entry("Toggle theme", "Dark → Light").with_icon(glyphs::MOON),
            entry("Refresh problems", ""),
        ];
        let ctx = egui::Context::default();
        let mut state = PaletteState::default();
        state.open();

        for query in ["", "theme", "zzzz"] {
            state.open();
            state.query = query.to_string();
            let mut output = ctx.run_ui(Default::default(), |_ui| {
                let outcome = show(&ctx, &mut state, "test", "Search…", &entries, None);
                assert!(outcome.chosen.is_none(), "nothing should be chosen");
            });
            output.textures_delta.clear();
        }
    }

    fn key_event(key: Key) -> egui::RawInput {
        egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn enter_selects_the_top_match() {
        let entries = vec![
            entry("Toggle theme", "switch to Light mode"),
            entry("Quit Leetr", ""),
        ];
        let ctx = egui::Context::default();
        let mut state = PaletteState::default();
        state.open();
        state.query = "theme".into();

        let mut chosen = None;
        let mut output = ctx.run_ui(key_event(Key::Enter), |_ui| {
            chosen = show(&ctx, &mut state, "t", "…", &entries, None).chosen;
        });
        output.textures_delta.clear();

        assert_eq!(chosen, Some(0), "Enter must activate the highlighted row");
    }

    #[test]
    fn arrow_keys_move_the_selection() {
        let entries = vec![entry("alpha", ""), entry("beta", ""), entry("gamma", "")];
        let ctx = egui::Context::default();
        let mut state = PaletteState::default();
        state.open();

        let mut output = ctx.run_ui(key_event(Key::ArrowDown), |_ui| {
            show(&ctx, &mut state, "t", "…", &entries, None);
        });
        output.textures_delta.clear();
        assert_eq!(state.selected, 1);

        let mut output = ctx.run_ui(key_event(Key::ArrowUp), |_ui| {
            show(&ctx, &mut state, "t", "…", &entries, None);
        });
        output.textures_delta.clear();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn closing_clears_the_state() {
        let entries = vec![entry("Quit", "")];
        let ctx = egui::Context::default();
        let mut state = PaletteState::default();
        state.open();

        let mut output = ctx.run_ui(Default::default(), |_ui| {
            let mut outcome = show(&ctx, &mut state, "test", "Search…", &entries, None);
            outcome.closed = true;
            if outcome.closed {
                state.close();
            }
        });
        output.textures_delta.clear();
        assert!(!state.is_open());
    }
}
