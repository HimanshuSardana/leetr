//! A fuzzy-search command palette, shared by `Ctrl+K` (actions) and `Ctrl+P`
//! (problem selector).
//!
//! The matching logic is a pure function so it can be tested exhaustively
//! without an egui context.
//!
//! Visually it follows the "spotlight" convention: a borderless search field
//! with a key cap on the right, a row of filter chips, small-caps section
//! headers, and rows whose trailing edge carries either a shortcut key cap or a
//! muted hint.

//!
//! Rendering lives in the GPUI views (`crate::app`): this module keeps the
//! matching, ranking and open/close state so it stays unit-testable.

/// Upper bound on rendered rows, so a 4000-entry list stays instant.
pub const MAX_ROWS: usize = 200;

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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaletteEntry {
    pub label: String,
    /// Secondary text shown after the label (and searched too).
    pub hint: String,
    /// Optional glyph, drawn with the monospace font (see [`crate::glyphs`]).
    pub icon: Option<String>,
    /// Group heading this row sits under. Consecutive rows sharing a section
    /// are drawn beneath one small-caps header.
    pub section: Option<String>,
    /// Keyboard shortcut, drawn as a key cap on the right of the row.
    pub shortcut: Option<String>,
    /// Chip ids this row answers to (used by the filter chips).
    pub tags: Vec<String>,
}

impl PaletteEntry {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ..Self::default()
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

    pub fn with_section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }

    pub fn with_shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn with_tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
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

/// A filter chip drawn above the results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteChip {
    pub id: String,
    pub label: String,
}

impl PaletteChip {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

/// Everything the palette needs besides the entries themselves.
#[derive(Debug, Clone, Default)]
pub struct PaletteConfig<'a> {
    /// Stable id (keeps the modal and its widgets apart per palette).
    pub id: &'a str,
    /// Placeholder shown in the empty search field.
    pub placeholder: &'a str,
    /// Key cap drawn at the right of the search field, e.g. `"Ctrl P"`.
    pub search_badge: Option<&'a str>,
    /// Filter chips shown between the search field and the results.
    pub chips: Vec<PaletteChip>,
    /// Fallback action offered when nothing matches (see `empty_action`).
    pub empty_action: Option<&'a str>,
}

/// Open/closed state and cursor of one palette.
#[derive(Debug, Default, Clone)]
pub struct PaletteState {
    open: bool,
    query: String,
    selected: usize,
    /// Active filter chip id; `None` means "All".
    chip: Option<String>,
    /// Set for one frame after opening so the text field takes focus.
    grab_focus: bool,
    /// Set when the selection moved by keyboard so the list scrolls to it.
    scroll_to_selection: bool,
}

impl PaletteState {
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open with a clean query and no chip filter.
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.chip = None;
        self.grab_focus = true;
        self.scroll_to_selection = false;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.chip = None;
        self.grab_focus = false;
        self.scroll_to_selection = false;
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The active chip id, if any.
    pub fn active_chip(&self) -> Option<&str> {
        self.chip.as_deref()
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

impl PaletteState {
    /// Replace the query (GPUI input drives this) and reset the cursor.
    pub fn set_query(&mut self, query: String) {
        self.query = query;
        self.selected = 0;
    }

    /// Activate a filter chip; `None` means "All".
    pub fn set_chip(&mut self, chip: Option<String>) {
        self.chip = chip;
        self.selected = 0;
    }

    /// Move the highlight, wrapping around `count` visible rows.
    pub fn move_next(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        self.selected = (self.selected + 1) % count;
    }

    /// Move the highlight up, wrapping around `count` visible rows.
    pub fn move_prev(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
    }

    /// Clamp a stale highlight after the match set shrank.
    pub fn clamp_selection(&mut self, count: usize) {
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    /// Index of the highlighted row.
    pub fn selected(&self) -> usize {
        self.selected
    }
}

/// Indices of `entries` visible for a query and chip filter.
///
/// Ranking is by [`rank`]. With an empty query the rows are a grouped browse
/// view, so each section is kept together (in first-appearance order) while the
/// order *within* a section is preserved. The returned indices always point at
/// the original entries, so a choice maps straight back.
pub fn visible_matches(query: &str, entries: &[PaletteEntry], chip: Option<&str>) -> Vec<usize> {
    let texts: Vec<String> = entries.iter().map(PaletteEntry::search_text).collect();
    let mut matches = rank(query, &texts);

    if let Some(chip) = chip {
        matches.retain(|&index| entries[index].tags.iter().any(|tag| tag == chip));
    }

    if query.trim().is_empty() {
        let mut order: Vec<&str> = Vec::new();
        for entry in entries {
            if let Some(section) = entry.section.as_deref() {
                if !order.contains(&section) {
                    order.push(section);
                }
            }
        }
        matches.sort_by_key(|&index| {
            let section = entries[index].section.as_deref().unwrap_or("");
            order
                .iter()
                .position(|s| *s == section)
                .unwrap_or(usize::MAX)
        });
    }

    matches
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn tagged(label: &str, section: &str, tags: &[&str]) -> PaletteEntry {
        PaletteEntry::new(label)
            .with_section(section)
            .with_tags(tags.iter().map(|tag| (*tag).to_string()))
    }

    #[test]
    fn an_empty_query_groups_rows_by_section() {
        let entries = vec![
            tagged("a", "Todo", &["todo"]),
            tagged("b", "Solved", &["solved"]),
            tagged("c", "Todo", &["todo"]),
            tagged("d", "Solved", &["solved"]),
        ];
        // "Solved" first appears after "Todo", but within each section the
        // original order is kept.
        assert_eq!(visible_matches("", &entries, None), vec![0, 2, 1, 3]);
    }

    #[test]
    fn a_chip_filter_narrows_by_tag() {
        let entries = vec![
            tagged("a", "Solved", &["easy", "solved"]),
            tagged("b", "Todo", &["hard", "todo"]),
            tagged("c", "Solved", &["hard", "solved"]),
        ];
        // Grouping still applies with a chip: "Solved" first appears before
        // "Todo", so the hard rows come back as [c, b].
        assert_eq!(visible_matches("", &entries, Some("hard")), vec![2, 1]);
        assert_eq!(visible_matches("", &entries, Some("solved")), vec![0, 2]);
        assert!(visible_matches("", &entries, Some("medium")).is_empty());
    }

    #[test]
    fn a_query_and_a_chip_compose() {
        let entries = vec![
            tagged("Two Sum", "Solved", &["easy", "solved"]),
            tagged("Two Sum II", "Solved", &["medium", "solved"]),
            tagged("Median", "Todo", &["hard", "todo"]),
        ];
        // "two" ranks both Two Sum rows; the chip then keeps only the easy one.
        let matched = visible_matches("two", &entries, Some("easy"));
        assert_eq!(matched, vec![0]);
    }

    #[test]
    fn a_query_does_not_group_rows() {
        // With a query the list is a ranked flat view, not a grouped one:
        // "a" matches every row and they come back in score order.
        let entries = vec![
            tagged("alpha", "Todo", &[]),
            tagged("beta", "Solved", &[]),
            tagged("gamma", "Todo", &[]),
        ];
        assert_eq!(visible_matches("a", &entries, None), vec![0, 1, 2]);
    }

    #[test]
    fn palette_state_opens_clean_and_closes_clean() {
        let mut state = PaletteState::default();
        assert!(!state.is_open());

        state.open();
        assert!(state.is_open());
        assert_eq!(state.query(), "");
        assert_eq!(state.selected, 0);
        assert_eq!(state.active_chip(), None);

        state.query = "abc".into();
        state.selected = 5;
        state.chip = Some("easy".into());
        state.close();
        assert!(!state.is_open());
        assert_eq!(state.query(), "");
        assert_eq!(state.selected, 0);
        assert_eq!(state.active_chip(), None);
        assert!(!state.grab_focus);
    }

    #[test]
    fn navigation_wraps_around_the_visible_rows() {
        let mut state = PaletteState::default();
        state.open();
        state.move_next(3);
        assert_eq!(state.selected(), 1);
        state.move_next(3);
        state.move_next(3);
        assert_eq!(state.selected(), 0);
        state.move_prev(3);
        assert_eq!(state.selected(), 2);
        state.set_query("two".into());
        assert_eq!(state.query(), "two");
        assert_eq!(state.selected(), 0);
        state.set_chip(Some("easy".into()));
        assert_eq!(state.active_chip(), Some("easy"));
        state.clamp_selection(1);
        state.move_next(0);
        state.move_prev(0);
    }
}
