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

use egui::{
    Align2, Color32, CornerRadius, FontId, Key, Margin, RichText, Sense, Stroke, StrokeKind,
    TextEdit,
};

use crate::glyphs;

/// Width of the palette popup.
const PALETTE_WIDTH: f32 = 660.0;
/// Height of the scrollable result list.
const LIST_HEIGHT: f32 = 380.0;
/// Upper bound on rendered rows, so a 4000-entry list stays instant.
const MAX_ROWS: usize = 200;
/// Height of one result row.
const ROW_HEIGHT: f32 = 40.0;
/// Height of a key cap / shortcut badge.
const CAP_HEIGHT: f32 = 20.0;

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

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Colours that differ between the two themes.
struct Colors {
    border: Color32,
    selection_fill: Color32,
    hover_fill: Color32,
    chip_fill: Color32,
    chip_border: Color32,
    chip_active_fill: Color32,
    cap_fill: Color32,
    cap_border: Color32,
}

fn colors(dark: bool) -> Colors {
    if dark {
        Colors {
            border: Color32::from_rgb(0x2a, 0x2f, 0x37),
            selection_fill: Color32::from_rgb(0x2a, 0x35, 0x49),
            hover_fill: Color32::from_rgb(0x20, 0x25, 0x2c),
            chip_fill: Color32::from_rgb(0x22, 0x26, 0x2c),
            chip_border: Color32::from_rgb(0x33, 0x39, 0x42),
            chip_active_fill: Color32::from_rgb(0x2a, 0x35, 0x49),
            cap_fill: Color32::from_rgb(0x1c, 0x20, 0x26),
            cap_border: Color32::from_rgb(0x39, 0x3f, 0x49),
        }
    } else {
        Colors {
            border: Color32::from_rgb(0xe6, 0xe8, 0xec),
            selection_fill: Color32::from_rgb(0xe9, 0xef, 0xf9),
            hover_fill: Color32::from_rgb(0xf2, 0xf4, 0xf7),
            chip_fill: Color32::from_rgb(0xf3, 0xf4, 0xf7),
            chip_border: Color32::from_rgb(0xdc, 0xdf, 0xe4),
            chip_active_fill: Color32::from_rgb(0xe4, 0xec, 0xfa),
            cap_fill: Color32::from_rgb(0xf1, 0xf2, 0xf5),
            cap_border: Color32::from_rgb(0xd5, 0xd9, 0xdf),
        }
    }
}

/// Size a key cap needs to fit `text`.
fn key_cap_size(ui: &egui::Ui, text: &str) -> egui::Vec2 {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        FontId::monospace(11.0),
        Color32::PLACEHOLDER,
    );
    egui::vec2(galley.size().x + 16.0, CAP_HEIGHT)
}

/// Paint a bordered key cap.
fn paint_key_cap(ui: &egui::Ui, rect: egui::Rect, text: &str, border: Color32, fill: Color32) {
    let radius = CornerRadius::same(5);
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::monospace(11.0),
        ui.visuals().weak_text_color(),
    );
}

/// Indices of `entries` visible for a query and chip filter.
///
/// Ranking is by [`rank`]. With an empty query the rows are a grouped browse
/// view, so each section is kept together (in first-appearance order) while the
/// order *within* a section is preserved. The returned indices always point at
/// the original entries, so a choice maps straight back.
fn visible_matches(query: &str, entries: &[PaletteEntry], chip: Option<&str>) -> Vec<usize> {
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

/// Draw the palette as a modal overlay. Returns the user's choice, if any.
pub fn show(
    ctx: &egui::Context,
    state: &mut PaletteState,
    entries: &[PaletteEntry],
    config: &PaletteConfig<'_>,
) -> PaletteOutcome {
    if !state.open {
        return PaletteOutcome::default();
    }

    let grouped = state.query.trim().is_empty();
    let matches = visible_matches(&state.query, entries, state.chip.as_deref());

    if state.selected >= matches.len() {
        state.selected = matches.len().saturating_sub(1);
    }

    let screen = ctx.viewport_rect();
    let position = egui::pos2(
        (screen.center().x - PALETTE_WIDTH / 2.0).max(screen.left() + 8.0),
        screen.top() + 76.0,
    );
    let modal_id = egui::Id::new(("leetr-palette", config.id));

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

    // A rounded, shadowed card with a borderless search field and an
    // accent-tinted selection reads as a modern launcher rather than a stock
    // egui popup.
    let popup_frame = egui::Frame::popup(ctx.global_style().as_ref())
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(Margin::ZERO)
        .shadow(egui::epaint::Shadow {
            offset: [0, 12],
            blur: 32,
            spread: 0,
            color: Color32::from_black_alpha(110),
        });

    let response = egui::Modal::new(modal_id)
        .backdrop_color(Color32::from_black_alpha(150))
        .frame(popup_frame)
        .area(egui::Modal::default_area(modal_id).fixed_pos(position))
        .show(ctx, |ui| {
            let mut outcome = PaletteOutcome::default();
            let mut empty_action_clicked = false;
            let mut requested_chip: Option<Option<String>> = None;
            ui.set_width(PALETTE_WIDTH);

            let palette_colors = colors(ui.visuals().dark_mode);

            // -- search -------------------------------------------------
            let search = egui::Frame::NONE
                .inner_margin(Margin::symmetric(16, 13))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let galley = ui.painter().layout_no_wrap(
                            glyphs::SEARCH.to_owned(),
                            FontId::monospace(14.0),
                            ui.visuals().weak_text_color(),
                        );
                        let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
                        ui.painter()
                            .galley(rect.min, galley, ui.visuals().weak_text_color());
                        ui.add_space(10.0);

                        // Size the field so the key cap still fits on the right.
                        let cap_size = config.search_badge.map(|text| key_cap_size(ui, text));
                        let reserved = cap_size.map_or(0.0, |size| size.x + 10.0);
                        let edit_width = (ui.available_width() - reserved).max(80.0);
                        let edit = ui.add(
                            TextEdit::singleline(&mut state.query)
                                .id(modal_id.with("query"))
                                .hint_text(RichText::new(config.placeholder).size(15.0))
                                .frame(egui::Frame::NONE)
                                .margin(Margin::ZERO)
                                .font(FontId::proportional(15.0))
                                .desired_width(edit_width),
                        );
                        if std::mem::take(&mut state.grab_focus) {
                            edit.request_focus();
                        }
                        if edit.changed() {
                            state.selected = 0;
                        }

                        if let (Some(text), Some(size)) = (config.search_badge, cap_size) {
                            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                            paint_key_cap(
                                ui,
                                rect,
                                text,
                                palette_colors.cap_border,
                                palette_colors.cap_fill,
                            );
                        }
                    });
                });
            ui.painter().hline(
                search.response.rect.x_range(),
                search.response.rect.bottom(),
                Stroke::new(1.0, palette_colors.border),
            );

            // -- chips --------------------------------------------------
            if !config.chips.is_empty() {
                let chips = egui::Frame::NONE
                    .inner_margin(Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                            for chip in &config.chips {
                                // The chip with id "all" means "no filter", so it
                                // reads as active whenever no chip is selected.
                                let active = state
                                    .chip
                                    .as_deref()
                                    .map_or(chip.id == "all", |current| current == chip.id);
                                if chip_button(ui, &chip.label, active, &palette_colors).clicked() {
                                    requested_chip = Some(if active || chip.id == "all" {
                                        None
                                    } else {
                                        Some(chip.id.clone())
                                    });
                                }
                            }
                        });
                    });
                ui.painter().hline(
                    chips.response.rect.x_range(),
                    chips.response.rect.bottom(),
                    Stroke::new(1.0, palette_colors.border),
                );
            }

            if let Some(next) = requested_chip {
                state.chip = next;
                state.selected = 0;
            }

            // -- results ------------------------------------------------
            egui::ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(6.0);

                    if matches.is_empty() {
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| match config.empty_action {
                            Some(label) => {
                                if ui
                                    .add(egui::Button::new(label).corner_radius(8.0))
                                    .clicked()
                                {
                                    empty_action_clicked = true;
                                }
                            }
                            None => {
                                ui.label(RichText::new("No matches").weak());
                            }
                        });
                        ui.add_space(12.0);
                        return;
                    }

                    let mut last_section: Option<&str> = None;
                    for (row, &entry_index) in matches.iter().enumerate().take(MAX_ROWS) {
                        let entry = &entries[entry_index];

                        if grouped {
                            let section = entry.section.as_deref().unwrap_or("");
                            if !section.is_empty() && last_section != Some(section) {
                                if last_section.is_some() {
                                    ui.add_space(8.0);
                                }
                                section_header(ui, section);
                                last_section = Some(section);
                            }
                        }

                        let is_selected = row == state.selected;
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ROW_HEIGHT),
                            egui::Sense::click(),
                        );

                        if ui.is_rect_visible(rect) {
                            let pill = rect.shrink2(egui::vec2(6.0, 2.0));
                            if is_selected {
                                ui.painter().rect_filled(
                                    pill,
                                    CornerRadius::same(8),
                                    palette_colors.selection_fill,
                                );
                            } else if response.hovered() {
                                ui.painter().rect_filled(
                                    pill,
                                    CornerRadius::same(8),
                                    palette_colors.hover_fill,
                                );
                            }

                            let text_color = if is_selected {
                                ui.visuals().strong_text_color()
                            } else {
                                ui.visuals().text_color()
                            };
                            let hint_color = ui.visuals().weak_text_color();
                            let inner = rect.shrink2(egui::vec2(16.0, 0.0));

                            // The trailing element is a shortcut key cap when the
                            // row has one, otherwise the muted hint text.
                            let hint_font = FontId::proportional(12.0);
                            let cap_size =
                                entry.shortcut.as_deref().map(|text| key_cap_size(ui, text));
                            let hint_width = if let Some(size) = cap_size {
                                size.x + 16.0
                            } else if entry.hint.is_empty() {
                                0.0
                            } else {
                                ui.painter()
                                    .layout_no_wrap(
                                        entry.hint.clone(),
                                        hint_font.clone(),
                                        hint_color,
                                    )
                                    .size()
                                    .x
                                    + 16.0
                            };

                            // The entry's icon carries real information — for
                            // Ctrl+P it is the solved / attempted / todo marker.
                            // NOTE: this must stay. An earlier rewrite of this
                            // row dropped it and silently removed those markers;
                            // no test caught it because nothing asserted painted
                            // output, only that rendering did not panic.
                            let mut label_left = inner.left();
                            if let Some(icon) = entry.icon.as_deref() {
                                let galley = ui.painter().layout_no_wrap(
                                    icon.to_owned(),
                                    FontId::monospace(14.0),
                                    text_color,
                                );
                                let position = egui::pos2(
                                    inner.left(),
                                    inner.center().y - galley.size().y / 2.0,
                                );
                                label_left += galley.size().x + 8.0;
                                ui.painter().galley(position, galley, text_color);
                            }

                            let label = ui.painter().layout(
                                entry.label.clone(),
                                FontId::proportional(14.5),
                                text_color,
                                (inner.width() - hint_width - (label_left - inner.left()))
                                    .max(40.0),
                            );
                            ui.painter().galley(
                                egui::pos2(label_left, inner.center().y - label.size().y / 2.0),
                                label,
                                text_color,
                            );

                            if let (Some(text), Some(size)) = (entry.shortcut.as_deref(), cap_size)
                            {
                                let cap = egui::Rect::from_min_size(
                                    egui::pos2(
                                        inner.right() - size.x,
                                        inner.center().y - size.y / 2.0,
                                    ),
                                    size,
                                );
                                paint_key_cap(
                                    ui,
                                    cap,
                                    text,
                                    palette_colors.cap_border,
                                    palette_colors.cap_fill,
                                );
                            } else if !entry.hint.is_empty() {
                                ui.painter().text(
                                    egui::pos2(inner.right(), inner.center().y),
                                    Align2::RIGHT_CENTER,
                                    &entry.hint,
                                    hint_font,
                                    hint_color,
                                );
                            }
                        }

                        if is_selected && state.scroll_to_selection {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        if response.clicked() {
                            outcome.chosen = Some(entry_index);
                        }
                    }

                    ui.add_space(6.0);
                });

            state.scroll_to_selection = false;

            if activate {
                if let Some(&entry_index) = matches.get(state.selected) {
                    outcome.chosen = Some(entry_index);
                }
            }
            if empty_action_clicked
                || (activate && matches.is_empty() && config.empty_action.is_some())
            {
                outcome.empty_action = true;
            }

            // -- footer ------------------------------------------------
            let footer = egui::Frame::NONE
                .inner_margin(Margin::symmetric(16, 10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let hint = RichText::new("Up/Down navigate · Enter select · Esc close")
                            .weak()
                            .small();
                        ui.label(hint);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{} of {}", matches.len(), entries.len()))
                                    .weak()
                                    .small(),
                            );
                        });
                    });
                });
            ui.painter().hline(
                footer.response.rect.x_range(),
                footer.response.rect.top(),
                Stroke::new(1.0, palette_colors.border),
            );

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

/// One filter chip.
fn chip_button(ui: &mut egui::Ui, label: &str, active: bool, colors: &Colors) -> egui::Response {
    let text_color = if active {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };

    let galley =
        ui.painter()
            .layout_no_wrap(label.to_owned(), FontId::proportional(12.5), text_color);
    let size = egui::vec2(galley.size().x + 22.0, 26.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let fill = if active {
            colors.chip_active_fill
        } else {
            colors.chip_fill
        };
        let border = if active {
            ui.visuals().selection.stroke.color
        } else {
            colors.chip_border
        };
        let radius = CornerRadius::same(7);
        ui.painter().rect_filled(rect, radius, fill);
        ui.painter()
            .rect_stroke(rect, radius, Stroke::new(1.0, border), StrokeKind::Inside);
        ui.painter().galley(
            egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            ),
            galley,
            text_color,
        );
    }

    response
}

/// A small-caps section heading.
fn section_header(ui: &mut egui::Ui, section: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    ui.painter().text(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        section.to_uppercase(),
        FontId::proportional(11.0),
        ui.visuals().weak_text_color(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs;

    fn entry(label: &str, hint: &str) -> PaletteEntry {
        PaletteEntry::new(label).with_hint(hint)
    }

    fn config(id: &str) -> PaletteConfig<'_> {
        PaletteConfig {
            id,
            placeholder: "Search…",
            ..PaletteConfig::default()
        }
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
    fn palette_renders_without_panicking() {
        let entries = vec![
            entry("Toggle theme", "Dark → Light").with_icon(glyphs::MOON),
            entry("Refresh problems", ""),
        ];
        let ctx = egui::Context::default();
        let mut state = PaletteState::default();

        for query in ["", "theme", "zzzz"] {
            state.open();
            state.query = query.to_string();
            let mut output = ctx.run_ui(Default::default(), |_ui| {
                let outcome = show(&ctx, &mut state, &entries, &config("test"));
                assert!(outcome.chosen.is_none(), "nothing should be chosen");
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn palette_renders_with_chips_sections_and_shortcuts() {
        let entries = vec![
            entry("1. Two Sum", "Easy · solved")
                .with_icon(glyphs::SOLVED)
                .with_section("Solved")
                .with_tags(["easy", "solved"]),
            entry("4. Median", "Hard · todo")
                .with_icon(glyphs::TODO)
                .with_section("Todo")
                .with_tags(["hard", "todo"]),
            PaletteEntry::new("Quit Leetr")
                .with_section("App")
                .with_shortcut("Ctrl Q"),
        ];
        let chips = vec![
            PaletteChip::new("all", "All"),
            PaletteChip::new("easy", "Easy"),
            PaletteChip::new("solved", "Solved"),
        ];
        let cfg = PaletteConfig {
            id: "problems",
            placeholder: "Jump to a problem…",
            search_badge: Some("Ctrl P"),
            chips,
            empty_action: None,
        };

        let ctx = egui::Context::default();
        let mut state = PaletteState::default();

        for chip in [None, Some("easy".to_string()), Some("solved".to_string())] {
            state.open();
            state.chip = chip;
            let mut output = ctx.run_ui(Default::default(), |_ui| {
                let _ = show(&ctx, &mut state, &entries, &cfg);
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
            chosen = show(&ctx, &mut state, &entries, &config("t")).chosen;
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
            show(&ctx, &mut state, &entries, &config("t"));
        });
        output.textures_delta.clear();
        assert_eq!(state.selected, 1);

        let mut output = ctx.run_ui(key_event(Key::ArrowUp), |_ui| {
            show(&ctx, &mut state, &entries, &config("t"));
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
            let mut outcome = show(&ctx, &mut state, &entries, &config("test"));
            outcome.closed = true;
            if outcome.closed {
                state.close();
            }
        });
        output.textures_delta.clear();
        assert!(!state.is_open());
    }
}
