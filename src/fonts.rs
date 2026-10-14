//! Runtime font loading.
//!
//! The interface is drawn in **Iosevka Nerd Font** when it is installed.
//!
//! The font is deliberately *not* bundled: a single face is ~14 MB, which would
//! more than double the binary for a purely cosmetic default. Instead it is
//! discovered on the system and egui's bundled fonts are kept as **fallbacks**,
//! so a machine without Iosevka still renders every glyph — the fallbacks carry
//! the emoji and dingbats the UI paints (see [`crate::glyphs`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{FontData, FontDefinitions, FontFamily};

/// Font files to look for, best match first.
///
/// The base "Iosevka Nerd Font" is preferred; the Mono and Propo variants are
/// accepted as reasonable stand-ins when only those are installed.
const CANDIDATES: &[&str] = &[
    "IosevkaNerdFont-Regular",
    "IosevkaNerdFontMono-Regular",
    "IosevkaNerdFontPropo-Regular",
];

/// Name the font is registered under inside egui.
const FAMILY: &str = "iosevka";

/// Directories scanned for the font, in order.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    dirs
}

/// Find an installed Iosevka Nerd Font, if any.
pub fn find() -> Option<PathBuf> {
    search_dirs().iter().find_map(|dir| best_in(dir, 0))
}

/// The best Iosevka candidate under `dir`, ranked by [`CANDIDATES`].
///
/// Walks nested directories (font packages commonly nest one or two levels,
/// e.g. `/usr/share/fonts/TTF/`), but stops descending at [`MAX_DEPTH`] so a
/// symlink loop cannot hang startup.
fn best_in(dir: &Path, depth: usize) -> Option<PathBuf> {
    const MAX_DEPTH: usize = 3;
    if depth > MAX_DEPTH {
        return None;
    }

    let mut best: Option<(usize, PathBuf)> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some((rank, found)) = best_in(&path, depth + 1).and_then(|p| rank_of(&p)) {
                let better = best.as_ref().is_none_or(|(current, _)| rank < *current);
                if better {
                    best = Some((rank, found));
                }
            }
            continue;
        }

        if let Some((rank, found)) = rank_of(&path) {
            let better = best.as_ref().is_none_or(|(current, _)| rank < *current);
            if better {
                best = Some((rank, found));
            }
        }
    }

    best.map(|(_, path)| path)
}

/// Rank a path against [`CANDIDATES`], if its file stem matches one.
fn rank_of(path: &Path) -> Option<(usize, PathBuf)> {
    let stem = path.file_stem()?.to_str()?;
    CANDIDATES
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(stem))
        .map(|rank| (rank, path.to_path_buf()))
}

/// Install Iosevka Nerd Font into `ctx` if it can be found.
///
/// Returns the path that was loaded. egui's bundled fonts stay in the family
/// lists behind Iosevka, so missing glyphs still fall back instead of rendering
/// as tofu boxes.
pub fn install(ctx: &egui::Context) -> Option<PathBuf> {
    let path = find()?;
    let bytes = std::fs::read(&path).ok()?;

    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert(FAMILY.to_owned(), Arc::new(FontData::from_owned(bytes)));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, FAMILY.to_owned());
    }
    ctx.set_fonts(fonts);
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique scratch directory, removed when the guard drops.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "leetr-fonts-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn touch(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"").unwrap();
    }

    #[test]
    fn candidate_names_are_distinct() {
        let mut names = CANDIDATES.to_vec();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn the_base_variant_is_preferred_over_mono_and_propo() {
        let dir = TempDir::new("prefer");
        touch(dir.path(), "IosevkaNerdFontPropo-Regular.ttf");
        touch(dir.path(), "IosevkaNerdFont-Regular.ttf");
        touch(dir.path(), "IosevkaNerdFontMono-Regular.ttf");

        let found = best_in(dir.path(), 0).unwrap();
        assert_eq!(
            found.file_name().unwrap().to_str().unwrap(),
            "IosevkaNerdFont-Regular.ttf"
        );
    }

    #[test]
    fn unrelated_fonts_are_ignored() {
        let dir = TempDir::new("unrelated");
        touch(dir.path(), "DejaVuSans.ttf");
        touch(dir.path(), "Iosevka-Regular.ttf");
        assert!(best_in(dir.path(), 0).is_none());
    }

    #[test]
    fn nested_font_directories_are_searched() {
        let dir = TempDir::new("nested");
        let nested = dir.path().join("TTF");
        std::fs::create_dir_all(&nested).unwrap();
        touch(&nested, "IosevkaNerdFontMono-Regular.ttf");

        let found = best_in(dir.path(), 0).unwrap();
        assert_eq!(
            found.file_name().unwrap().to_str().unwrap(),
            "IosevkaNerdFontMono-Regular.ttf"
        );
    }

    #[test]
    fn install_keeps_the_default_fallbacks_and_never_panics() {
        let ctx = egui::Context::default();
        // On a machine without the font this is `None`; on this one it loads.
        // Either way it must not panic and the families must stay populated.
        let _loaded = install(&ctx);

        let mut families = Vec::new();
        // Fonts only exist once a frame has run.
        let mut output = ctx.run_ui(Default::default(), |_ui| {
            families = ctx.fonts_mut(|fonts| fonts.families());
        });
        output.textures_delta.clear();

        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            assert!(
                families.contains(&family),
                "{family:?} must have at least the bundled fonts"
            );
        }
    }
}
