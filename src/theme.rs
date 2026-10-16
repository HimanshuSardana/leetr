//! Colour schemes.
//!
//! Leetr ships a curated **dark mode** (the default for a coding tool) and a
//! matching light mode. On GPUI Kit these map onto the component library's
//! [`ThemeMode`](gpui_kit::component::theme::ThemeMode); the hex constants
//! below drive Leetr-specific accents (difficulty badges, verdict colours).

use serde::{Deserialize, Serialize};

use crate::model::Difficulty;

/// LeetCode's signature orange, used as the accent colour in both themes.
pub const ACCENT_HEX: u32 = 0xFFA116;
/// Green used for accepted verdicts and success states.
pub const SUCCESS_HEX: u32 = 0x3FB950;
/// Red used for errors and rejected verdicts.
pub const DANGER_HEX: u32 = 0xE5534B;
/// Yellow used for warnings.
pub const WARNING_HEX: u32 = 0xD29922;

/// Colour used for a problem's difficulty badge, as a packed `0xRRGGBB`.
pub fn difficulty_hex(difficulty: Difficulty) -> u32 {
    match difficulty {
        Difficulty::Easy => 0x2CBB8A,
        Difficulty::Medium => 0xE8B339,
        Difficulty::Hard => 0xEF4A4A,
        Difficulty::Unknown => 0x8A8F98,
    }
}

/// A concrete colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Light text on a dark background.
    #[default]
    Dark,
    /// Dark text on a light background.
    Light,
}

impl Theme {
    /// Human-readable name, used in menus and status messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }

    /// Small glyph shown on the theme toggle.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Dark => crate::glyphs::MOON,
            Self::Light => crate::glyphs::SUN,
        }
    }

    /// The other theme.
    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    /// Corresponding GPUI window appearance, so the OS chrome matches.
    pub fn appearance(self) -> gpui_kit::WindowAppearance {
        match self {
            Self::Dark => gpui_kit::WindowAppearance::Dark,
            Self::Light => gpui_kit::WindowAppearance::Light,
        }
    }

    /// Corresponding gpui-component theme mode.
    pub fn mode(self) -> gpui_kit::component::theme::ThemeMode {
        match self {
            Self::Dark => gpui_kit::component::theme::ThemeMode::Dark,
            Self::Light => gpui_kit::component::theme::ThemeMode::Light,
        }
    }

    /// Apply this theme to the running GPUI app.
    pub fn apply(self, cx: &mut gpui_kit::App) {
        gpui_kit::component::theme::Theme::change(self.mode(), None, cx);
    }
}

/// Convert a packed `0xRRGGBB` into a GPUI colour.
pub fn rgb(hex: u32) -> gpui_kit::Rgba {
    gpui_kit::rgb(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_is_the_default() {
        assert_eq!(Theme::default(), Theme::Dark);
    }

    #[test]
    fn toggling_flips_between_the_two_themes() {
        assert_eq!(Theme::Dark.toggled(), Theme::Light);
        assert_eq!(Theme::Light.toggled(), Theme::Dark);
        assert_eq!(Theme::Dark.toggled().toggled(), Theme::Dark);
    }

    #[test]
    fn labels_and_icons_are_present() {
        for theme in [Theme::Dark, Theme::Light] {
            assert!(!theme.label().is_empty());
            assert!(!theme.icon().is_empty());
        }
    }

    #[test]
    fn difficulty_colours_are_distinct() {
        let colours = [
            difficulty_hex(Difficulty::Easy),
            difficulty_hex(Difficulty::Medium),
            difficulty_hex(Difficulty::Hard),
            difficulty_hex(Difficulty::Unknown),
        ];
        for (i, a) in colours.iter().enumerate() {
            for b in colours.iter().skip(i + 1) {
                assert_ne!(a, b, "difficulty colours must be distinguishable");
            }
        }
    }

    #[test]
    fn theme_serialises_lowercase() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Wrapper {
            theme: Theme,
        }

        let text = toml::to_string(&Wrapper { theme: Theme::Dark }).unwrap();
        assert_eq!(text.trim(), "theme = \"dark\"");

        let text = toml::to_string(&Wrapper {
            theme: Theme::Light,
        })
        .unwrap();
        assert_eq!(text.trim(), "theme = \"light\"");

        let back: Wrapper = toml::from_str(&text).unwrap();
        assert_eq!(back.theme, Theme::Light);
    }

    #[test]
    fn theme_deserialises_from_config_text() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            theme: Theme,
        }

        let w: Wrapper = toml::from_str("theme = \"dark\"").unwrap();
        assert_eq!(w.theme, Theme::Dark);

        let w: Wrapper = toml::from_str("theme = \"light\"").unwrap();
        assert_eq!(w.theme, Theme::Light);
    }
}
