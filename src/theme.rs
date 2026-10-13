//! Colour schemes.
//!
//! Leetr ships a curated **dark mode** (the default for a coding tool) and a
//! matching light mode. Both are applied to the whole `egui` context so every
//! panel, popup and widget — including future command palettes — picks them up.

use egui::style::Selection;
use egui::{Color32, Stroke, Visuals};
use serde::{Deserialize, Serialize};

/// LeetCode's signature orange, used as the accent colour in both themes.
pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0xa1, 0x16);

/// Green used for accepted verdicts and success states.
pub const SUCCESS: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);

/// Red used for errors and rejected verdicts.
pub const DANGER: Color32 = Color32::from_rgb(0xe5, 0x53, 0x4b);

/// Yellow used for warnings.
pub const WARNING: Color32 = Color32::from_rgb(0xd2, 0x99, 0x22);

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
            Self::Dark => "\u{1F319}", // 🌙
            Self::Light => "\u{2600}", // ☀
        }
    }

    /// The other theme.
    pub fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    /// Apply this theme to the whole egui context.
    ///
    /// Both themes' visuals are (re)installed so a later switch is instant, then
    /// the preference is set to the chosen one.
    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_visuals_of(egui::Theme::Dark, dark_visuals());
        ctx.set_visuals_of(egui::Theme::Light, light_visuals());
        ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.button_padding = egui::vec2(8.0, 4.0);
            style.spacing.interact_size = egui::vec2(28.0, 24.0);
        });
        ctx.set_theme(match self {
            Self::Dark => egui::ThemePreference::Dark,
            Self::Light => egui::ThemePreference::Light,
        });
    }
}

fn dark_visuals() -> Visuals {
    let background = Color32::from_rgb(0x17, 0x19, 0x1d);
    let surface = Color32::from_rgb(0x1f, 0x22, 0x27);
    let raised = Color32::from_rgb(0x27, 0x2b, 0x32);
    let hovered = Color32::from_rgb(0x32, 0x37, 0x3f);
    let active = Color32::from_rgb(0x3d, 0x43, 0x4d);
    let border = Color32::from_rgb(0x2d, 0x32, 0x39);
    let sunken = Color32::from_rgb(0x10, 0x12, 0x15);
    let text = Color32::from_rgb(0xdd, 0xe1, 0xe6);
    let weak_text = Color32::from_rgb(0x97, 0x9e, 0xa9);

    let mut v = Visuals::dark();
    v.dark_mode = true;
    v.panel_fill = background;
    v.window_fill = surface;
    v.faint_bg_color = surface;
    v.extreme_bg_color = sunken;
    v.text_edit_bg_color = Some(sunken);
    v.code_bg_color = Color32::from_rgb(0x12, 0x14, 0x17);
    v.weak_text_color = Some(weak_text);
    v.hyperlink_color = ACCENT;
    v.window_stroke = Stroke::new(1.0, border);
    v.selection = Selection {
        bg_fill: Color32::from_rgb(0x2b, 0x47, 0x69),
        stroke: Stroke::new(1.0, ACCENT),
    };

    v.widgets.noninteractive.bg_fill = surface;
    v.widgets.noninteractive.weak_bg_fill = surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);

    v.widgets.inactive.bg_fill = raised;
    v.widgets.inactive.weak_bg_fill = raised;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);

    v.widgets.hovered.bg_fill = hovered;
    v.widgets.hovered.weak_bg_fill = hovered;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.65));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.active.bg_fill = active;
    v.widgets.active.weak_bg_fill = active;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    v.widgets.open.bg_fill = surface;
    v.widgets.open.weak_bg_fill = surface;
    v.widgets.open.bg_stroke = Stroke::new(1.0, border);

    v
}

fn light_visuals() -> Visuals {
    let background = Color32::from_rgb(0xf7, 0xf8, 0xfa);
    let surface = Color32::WHITE;
    let raised = Color32::from_rgb(0xed, 0xef, 0xf2);
    let hovered = Color32::from_rgb(0xe3, 0xe6, 0xea);
    let active = Color32::from_rgb(0xd6, 0xda, 0xe0);
    let border = Color32::from_rgb(0xd3, 0xd7, 0xdd);
    let text = Color32::from_rgb(0x1c, 0x1f, 0x24);
    let weak_text = Color32::from_rgb(0x5f, 0x67, 0x72);

    let mut v = Visuals::light();
    v.dark_mode = false;
    v.panel_fill = background;
    v.window_fill = surface;
    v.faint_bg_color = Color32::from_rgb(0xf0, 0xf2, 0xf5);
    v.extreme_bg_color = surface;
    v.text_edit_bg_color = Some(surface);
    v.code_bg_color = Color32::from_rgb(0xf2, 0xf3, 0xf5);
    v.weak_text_color = Some(weak_text);
    // Darker orange so links stay legible on white.
    v.hyperlink_color = Color32::from_rgb(0xb2, 0x6a, 0x00);
    v.window_stroke = Stroke::new(1.0, border);
    v.selection = Selection {
        bg_fill: Color32::from_rgb(0xbb, 0xd6, 0xfb),
        stroke: Stroke::new(1.0, Color32::from_rgb(0xb2, 0x6a, 0x00)),
    };

    v.widgets.noninteractive.bg_fill = surface;
    v.widgets.noninteractive.weak_bg_fill = surface;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);

    v.widgets.inactive.bg_fill = raised;
    v.widgets.inactive.weak_bg_fill = raised;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, border);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);

    v.widgets.hovered.bg_fill = hovered;
    v.widgets.hovered.weak_bg_fill = hovered;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xb2, 0x6a, 0x00));
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, text);

    v.widgets.active.bg_fill = active;
    v.widgets.active.weak_bg_fill = active;
    v.widgets.active.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0xb2, 0x6a, 0x00));
    v.widgets.active.fg_stroke = Stroke::new(1.0, text);

    v.widgets.open.bg_fill = surface;
    v.widgets.open.weak_bg_fill = surface;
    v.widgets.open.bg_stroke = Stroke::new(1.0, border);

    v
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
    fn every_theme_has_contrasting_visuals() {
        let dark = dark_visuals();
        let light = light_visuals();
        assert!(dark.dark_mode);
        assert!(!light.dark_mode);
        assert_ne!(dark.panel_fill, light.panel_fill);
        assert_ne!(dark.widgets.noninteractive.fg_stroke.color, dark.panel_fill);
        assert_ne!(
            light.widgets.noninteractive.fg_stroke.color,
            light.panel_fill
        );
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
