//! Symbol glyphs painted directly by Leetr.
//!
//! Every symbol the UI draws is declared here so icon usage stays consistent
//! across the GPUI views. If you add an icon, add it here — not inline.

/// Status bars and lists: the session/problem is "on" (signed in, solved).
///
/// `⏺` (U+23FA) rather than the more obvious `●` (U+25CF), which the default
/// proportional font does not contain.
pub const DOT: &str = "⏺";

/// Problem-list status markers.
pub const SOLVED: &str = "⏺";
pub const ATTEMPTED: &str = "◑";
pub const TODO: &str = "·";

/// Toolbar: reload the problem list.
pub const REFRESH: &str = "⟳";

/// Premium-only problem marker.
pub const PREMIUM: &str = "★";

/// Bullet used by list rendering.
pub const BULLET: &str = "•";

/// Theme toggle icons. These are drawn with the **monospace** font.
pub const MOON: &str = "\u{1F319}";
pub const SUN: &str = "\u{2600}";

/// Leading icon in the command palettes' search field.
pub const SEARCH: &str = "\u{1F50D}";

/// Glyphs the UI paints with the proportional font.
pub const PROPORTIONAL: &[&str] = &[DOT, SOLVED, ATTEMPTED, TODO, REFRESH, PREMIUM, BULLET];

/// Glyphs the UI paints with the monospace font.
pub const MONOSPACE: &[&str] = &[MOON, SUN, SEARCH];

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert every declared glyph table is non-empty so the loops that
    /// consume them always mean something.
    #[test]
    fn glyph_tables_are_non_empty() {
        assert!(
            !PROPORTIONAL.is_empty() && !MONOSPACE.is_empty(),
            "glyph tables are empty — the loops would assert nothing"
        );
        for glyph in PROPORTIONAL.iter().chain(MONOSPACE.iter()) {
            assert!(!glyph.is_empty());
        }
    }

    #[test]
    fn marker_glyphs_are_single_symbols() {
        for glyph in [SOLVED, ATTEMPTED, TODO, REFRESH, PREMIUM, BULLET] {
            assert_eq!(
                glyph.chars().count(),
                1,
                "{glyph:?} should be a single character"
            );
        }
    }

    #[test]
    fn marker_glyphs_are_distinct() {
        let markers = [SOLVED, ATTEMPTED, TODO];
        for (i, a) in markers.iter().enumerate() {
            for b in markers.iter().skip(i + 1) {
                assert_ne!(a, b, "status markers must be visually distinct");
            }
        }
    }
}
