//! Symbol glyphs painted directly by Leetr.
//!
//! egui renders a "tofu" box for any glyph missing from the active font, and
//! the default **proportional** font is missing many common symbols (`✓`,
//! `●`, `☀`, `🌙`, `🔒`). Worse, coverage differs per font family: the
//! monospace fallback has the emoji/dingbat set while the proportional one
//! does not.
//!
//! Every symbol the UI draws is therefore declared here, and the test at the
//! bottom of this file asserts that each one really exists in the font family
//! it is drawn with. If you add an icon, add it here — not inline.

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

    /// Assert every declared glyph really exists in the default font set.
    ///
    /// `Context::run_ui` returns a `FullOutput`; dropping it without handling
    /// `textures_delta` panics inside epaint, so it is cleared here.
    #[test]
    fn every_ui_glyph_exists_in_the_font_it_is_drawn_with() {
        // Without this the loops below assert nothing if a table is emptied.
        assert!(
            !PROPORTIONAL.is_empty() && !MONOSPACE.is_empty(),
            "glyph tables are empty — the loops would assert nothing"
        );

        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |_ui| {
            let proportional = egui::FontId::proportional(14.0);
            let monospace = egui::FontId::monospace(14.0);

            ctx.fonts_mut(|fonts| {
                for glyph in PROPORTIONAL {
                    for ch in glyph.chars() {
                        assert!(
                            fonts.has_glyph(&proportional, ch),
                            "glyph {ch:?} (U+{:04X}) from {glyph:?} is missing from the \
                             proportional font — it would render as a tofu box",
                            ch as u32
                        );
                    }
                }

                for glyph in MONOSPACE {
                    for ch in glyph.chars() {
                        assert!(
                            fonts.has_glyph(&monospace, ch),
                            "glyph {ch:?} (U+{:04X}) from {glyph:?} is missing from the \
                             monospace font — it would render as a tofu box",
                            ch as u32
                        );
                    }
                }
            });
        });
        output.textures_delta.clear();
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
