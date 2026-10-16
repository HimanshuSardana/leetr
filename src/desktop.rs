//! Installing Leetr into graphical app launchers.
//!
//! Quickshell's launcher — like every freedesktop menu, dock and launcher —
//! reads XDG `.desktop` entries from `~/.local/share/applications/`. This module
//! writes Leetr's entry plus its icon, so the app shows up there.
//!
//! Absolute paths are used throughout, because the binary is frequently *not*
//! on `PATH` (and `Exec=leetr` would then fail silently from the launcher's
//! point of view).

use std::path::{Path, PathBuf};

/// The desktop entry template. `__EXEC__` and `__ICON__` are filled in at
/// install time with the absolute binary and icon paths.
pub const DESKTOP_SOURCE: &str = include_str!("../contrib/leetr.desktop");

/// The menu icon, embedded so it works from an installed binary too.
pub const ICON_SOURCE: &[u8] = include_bytes!("../contrib/leetr.png");

pub const DESKTOP_FILE_NAME: &str = "leetr.desktop";
pub const ICON_FILE_NAME: &str = "leetr.png";

/// Where this user's desktop entries live (Linux: `~/.local/share/applications`).
pub fn applications_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("applications"))
}

/// Where loose user icons live (Linux: `~/.local/share/icons`).
pub fn icons_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("icons"))
}

pub fn default_desktop_path() -> Option<PathBuf> {
    applications_dir().map(|dir| dir.join(DESKTOP_FILE_NAME))
}

pub fn default_icon_path() -> Option<PathBuf> {
    icons_dir().map(|dir| dir.join(ICON_FILE_NAME))
}

/// How `Exec` should invoke this binary.
///
/// The absolute path of the running executable, double-quoted. An entry that
/// says `Exec=leetr` only works when the binary is on `PATH`, which it often
/// isn't — so resolve it at install time instead.
pub fn executable() -> String {
    match std::env::current_exe() {
        Ok(path) => {
            let text = path.display().to_string();
            if text.contains('"') {
                "leetr".to_string()
            } else {
                format!("\"{text}\"")
            }
        }
        Err(_) => "leetr".to_string(),
    }
}

/// Render the desktop template with concrete paths.
pub fn render_desktop(executable: &str, icon_path: &str) -> String {
    DESKTOP_SOURCE
        .replace("__EXEC__", executable)
        .replace("__ICON__", icon_path)
}

/// What got written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallOutcome {
    pub desktop_written: bool,
    pub icon_written: bool,
}

impl InstallOutcome {
    pub fn is_up_to_date(self) -> bool {
        !self.desktop_written && !self.icon_written
    }
}

/// Write (or refresh) the entry and the icon.
///
/// Each file is only rewritten when its contents would change, so running the
/// installer repeatedly is harmless and cheap.
pub fn install(desktop_path: &Path, icon_path: &Path) -> Result<InstallOutcome, String> {
    if let Some(parent) = desktop_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("creating {}: {err}", parent.display()))?;
    }
    if let Some(parent) = icon_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("creating {}: {err}", parent.display()))?;
    }

    let desktop = render_desktop(&executable(), &icon_path.display().to_string());
    let desktop_written = write_if_changed(desktop_path, desktop.as_bytes())?;
    let icon_written = write_if_changed(icon_path, ICON_SOURCE)?;

    Ok(InstallOutcome {
        desktop_written,
        icon_written,
    })
}

fn write_if_changed(path: &Path, contents: &[u8]) -> Result<bool, String> {
    if let Ok(existing) = std::fs::read(path) {
        if existing == contents {
            return Ok(false);
        }
    }
    std::fs::write(path, contents).map_err(|err| format!("writing {}: {err}", path.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Parse `Name=Value` lines out of a rendered entry, ignoring comments and
    /// the section header, so tests can assert on the structure.
    fn fields(rendered: &str) -> BTreeSet<String> {
        rendered
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                    None
                } else {
                    line.split_once('=').map(|(key, _)| key.trim().to_string())
                }
            })
            .collect()
    }

    #[test]
    fn the_template_renders_without_leftover_placeholders() {
        let rendered = render_desktop("/opt/leetr/leetr", "/home/u/.local/share/icons/leetr.png");
        assert!(!rendered.contains("__EXEC__"), "Exec was not substituted");
        assert!(!rendered.contains("__ICON__"), "Icon was not substituted");
        assert!(rendered.contains("/opt/leetr/leetr"));
        assert!(rendered.contains("/home/u/.local/share/icons/leetr.png"));
    }

    #[test]
    fn the_rendered_entry_has_everything_a_launcher_needs() {
        let rendered = render_desktop("\"/opt/leetr/leetr\"", "/home/u/icons/leetr.png");
        let keys = fields(&rendered);

        for required in ["Type", "Name", "Exec", "Icon", "GenericName", "Comment"] {
            assert!(
                keys.contains(required),
                "{required} is missing from the entry"
            );
        }
        assert!(rendered.contains("Type=Application"));
        assert!(rendered.contains("Name=Leetr"));
        assert!(
            rendered.contains("\"/opt/leetr/leetr\""),
            "Exec must be the absolute, quoted binary path"
        );

        // Must not be hidden: NoDisplay would keep it out of every launcher.
        for line in rendered.lines() {
            let line = line.trim();
            assert!(
                !(line.starts_with("NoDisplay") && line.contains("true")),
                "the entry must not hide itself from launchers"
            );
        }
    }

    #[test]
    fn the_entry_is_searchable_by_the_terms_a_user_would_type() {
        let rendered = render_desktop("leetr", "leetr.png").to_lowercase();
        for term in ["leet", "leetcode", "problem", "judge"] {
            assert!(
                rendered.contains(term),
                "nothing searchable as {term:?} — the launcher matches name/generic/keywords"
            );
        }
    }

    #[test]
    fn executable_is_a_usable_command_line() {
        let exe = executable();
        assert!(
            !exe.trim().is_empty(),
            "the Exec line must never be empty (a launcher fails silently)"
        );
        // Either an absolute (quoted) path or the bare name as a fallback.
        assert!(
            exe.starts_with('"') || !exe.contains('/'),
            "unexpected Exec shape: {exe:?}"
        );
    }

    #[test]
    fn install_writes_both_files_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("leetr-desktop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let desktop = dir.join("applications").join(DESKTOP_FILE_NAME);
        let icon = dir.join("icons").join(ICON_FILE_NAME);

        let outcome = install(&desktop, &icon).expect("first install writes");
        assert!(outcome.desktop_written && outcome.icon_written);
        assert_eq!(std::fs::read(&icon).unwrap(), ICON_SOURCE);
        let parsed = std::fs::read_to_string(&desktop).unwrap();
        assert!(!parsed.contains("__EXEC__") && !parsed.contains("__ICON__"));

        let outcome = install(&desktop, &icon).expect("second install is a no-op");
        assert!(
            outcome.is_up_to_date(),
            "re-running must not rewrite identical files"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_updates_a_stale_entry() {
        let dir = std::env::temp_dir().join(format!("leetr-desktop-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let desktop = dir.join(DESKTOP_FILE_NAME);
        let icon = dir.join(ICON_FILE_NAME);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&desktop, b"[Desktop Entry]\nType=Application\nName=Leetr\n").unwrap();
        std::fs::write(&icon, b"stale").unwrap();

        let outcome = install(&desktop, &icon).expect("refresh");
        assert!(outcome.desktop_written && outcome.icon_written);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_paths_land_where_desktop_launchers_look() {
        let desktop = default_desktop_path().expect("a data dir exists");
        let icon = default_icon_path().expect("a data dir exists");
        assert!(
            desktop.ends_with("applications/leetr.desktop"),
            "got {}",
            desktop.display()
        );
        assert!(icon.ends_with("icons/leetr.png"), "got {}", icon.display());
    }
}
