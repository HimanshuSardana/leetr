//! Configuration and credential handling.
//!
//! Two files live under the OS config dir (Linux: `~/.config/leetr/`):
//!
//! * `config.toml`      — user preferences, safe to share.
//! * `credentials.toml` — LeetCode session cookies, **never** committed.
//!
//! Environment variables `LEETCODE_SESSION` and `LEETCODE_CSRFTOKEN` take
//! precedence over the credentials file when set and non-empty.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::theme::Theme;

pub const APP_NAME: &str = "leetr";

/// Placeholder written into a freshly seeded credentials file.
pub const PLACEHOLDER: &str = "REPLACE_ME";

pub const CONFIG_TEMPLATE: &str = r#"# Leetr configuration.

# Colour scheme: "dark" or "light".
theme = "dark"

# Language slug used when a problem is opened for the first time.
# Any slug from LeetCode's languageList works, e.g. "python3", "rust", "cpp".
default_language = "python3"

# Port for the local control API used by the bundled nvim plugin.
interop_port = 9527

# Safety rail: when false (the default) leetr refuses to submit a solution to a
# problem that is not already marked as solved on your account.
allow_unsolved_submit = false

# Command used for "Open in nvim". Leave empty to auto-detect
# (tmux split when $TMUX is set, otherwise $TERMINAL, otherwise `nvim`).
# Examples: "nvim", "kitty -e nvim", "alacritty -e nvim"
editor_command = ""

# Where solution files are kept. Empty = the OS data dir
# (Linux: ~/.local/share/leetr/workspace).
workspace_dir = ""

# Number of problems fetched per page in the problem list.
problem_list_page_size = 50
"#;

pub const CREDENTIALS_TEMPLATE: &str = r#"# Leetr credentials — do NOT commit this file (it lives outside the repo).
#
# Leetr only needs these for authenticated features:
#   * knowing which problems you have already solved
#   * running sample tests
#   * submitting solutions
#
# Browsing problems and reading statements works without any credentials.
#
# ── How to obtain ────────────────────────────────────────────────────────────
#   1. Open https://leetcode.com and make sure you are logged in.
#   2. Open DevTools (F12) → Application → Storage → Cookies → https://leetcode.com
#   3. Copy the value of the `LEETCODE_SESSION` cookie and the `csrftoken` cookie
#      (the *cookie*, not the `x-csrftoken` request header) below.
#   4. Restart leetr — a "signed in" badge appears in the status bar.
#
# Environment variables LEETCODE_SESSION and LEETCODE_CSRFTOKEN override this
# file when they are set and non-empty.
# ─────────────────────────────────────────────────────────────────────────────

leetcode_session = "REPLACE_ME"
csrftoken = "REPLACE_ME"
"#;

/// Paths that `leetr init` created or found.
#[derive(Debug, Clone)]
pub struct FilePaths {
    pub config: PathBuf,
    pub credentials: PathBuf,
}

/// User preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub theme: Theme,
    pub default_language: String,
    pub interop_port: u16,
    pub allow_unsolved_submit: bool,
    pub editor_command: String,
    pub workspace_dir: String,
    pub problem_list_page_size: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            default_language: "python3".to_string(),
            interop_port: 9527,
            allow_unsolved_submit: false,
            editor_command: String::new(),
            workspace_dir: String::new(),
            problem_list_page_size: 50,
        }
    }
}

impl Config {
    /// Load `config.toml`, falling back to defaults when it does not exist.
    pub fn load() -> Result<Self> {
        Self::load_from(&config_path())
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let cfg = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// LeetCode session cookies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Credentials {
    pub leetcode_session: String,
    pub csrftoken: String,
}

impl Credentials {
    /// Read the credentials file, then apply environment overrides.
    pub fn load() -> Self {
        Self::load_from(&credentials_path())
    }

    pub fn load_from(path: &Path) -> Self {
        let mut creds = Self::from_file(path).unwrap_or_default();
        if let Ok(v) = std::env::var("LEETCODE_SESSION") {
            if !v.trim().is_empty() {
                creds.leetcode_session = v;
            }
        }
        if let Ok(v) = std::env::var("LEETCODE_CSRFTOKEN") {
            if !v.trim().is_empty() {
                creds.csrftoken = v;
            }
        }
        creds
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let creds = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(creds)
    }

    /// True when both cookies hold a real (non-placeholder) value.
    pub fn is_configured(&self) -> bool {
        is_real(&self.leetcode_session) && is_real(&self.csrftoken)
    }

    /// Value for the HTTP `Cookie` header.
    pub fn cookie_header(&self) -> String {
        format!(
            "LEETCODE_SESSION={}; csrftoken={}",
            self.leetcode_session, self.csrftoken
        )
    }
}

fn is_real(value: &str) -> bool {
    let v = value.trim();
    !v.is_empty() && v != PLACEHOLDER
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_NAME)
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn credentials_path() -> PathBuf {
    config_dir().join("credentials.toml")
}

/// Write default config + credentials templates if they are missing.
pub fn seed() -> Result<FilePaths> {
    seed_at(&config_dir())
}

pub fn seed_at(dir: &Path) -> Result<FilePaths> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let config = dir.join("config.toml");
    let credentials = dir.join("credentials.toml");

    if !config.exists() {
        std::fs::write(&config, CONFIG_TEMPLATE)
            .with_context(|| format!("writing {}", config.display()))?;
    }
    if !credentials.exists() {
        std::fs::write(&credentials, CREDENTIALS_TEMPLATE)
            .with_context(|| format!("writing {}", credentials.display()))?;
    }

    Ok(FilePaths {
        config,
        credentials,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("leetr-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_config_is_sane() {
        let cfg = Config::default();
        assert_eq!(cfg.theme, Theme::Dark);
        assert_eq!(cfg.default_language, "python3");
        assert_eq!(cfg.interop_port, 9527);
        assert!(
            !cfg.allow_unsolved_submit,
            "solved-guard must default to on"
        );
        assert_eq!(cfg.problem_list_page_size, 50);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let dir = tmpdir("cfg-rt");
        let path = dir.join("config.toml");
        let cfg = Config {
            theme: Theme::Light,
            default_language: "rust".into(),
            interop_port: 1234,
            allow_unsolved_submit: true,
            ..Config::default()
        };
        cfg.save_to(&path).unwrap();

        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded, cfg);
        assert_eq!(loaded.theme, Theme::Light);
    }

    #[test]
    fn missing_theme_falls_back_to_dark() {
        let dir = tmpdir("cfg-no-theme");
        let path = dir.join("config.toml");
        std::fs::write(&path, "default_language = \"go\"\n").unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.theme, Theme::Dark);
    }

    #[test]
    fn seeded_config_template_parses_back_to_defaults() {
        let dir = tmpdir("cfg-template");
        let paths = seed_at(&dir).unwrap();
        let cfg = Config::load_from(&paths.config).unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn missing_config_yields_defaults() {
        let dir = tmpdir("cfg-missing");
        let loaded = Config::load_from(&dir.join("nope.toml")).unwrap();
        assert_eq!(loaded, Config::default());
    }

    #[test]
    fn partial_config_uses_defaults_for_missing_keys() {
        let dir = tmpdir("cfg-partial");
        let path = dir.join("config.toml");
        std::fs::write(&path, "default_language = \"go\"\n").unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.default_language, "go");
        assert_eq!(loaded.interop_port, 9527);
        assert!(!loaded.allow_unsolved_submit);
    }

    #[test]
    fn placeholder_credentials_are_not_configured() {
        let creds = Credentials {
            leetcode_session: PLACEHOLDER.into(),
            csrftoken: PLACEHOLDER.into(),
        };
        assert!(!creds.is_configured());

        let creds = Credentials {
            leetcode_session: "abc".into(),
            csrftoken: "def".into(),
        };
        assert!(creds.is_configured());
    }

    #[test]
    fn empty_credentials_are_not_configured() {
        assert!(!Credentials::default().is_configured());
        let creds = Credentials {
            leetcode_session: "abc".into(),
            csrftoken: "  ".into(),
        };
        assert!(!creds.is_configured());
    }

    #[test]
    fn cookie_header_contains_both_cookies() {
        let creds = Credentials {
            leetcode_session: "sess".into(),
            csrftoken: "tok".into(),
        };
        assert_eq!(
            creds.cookie_header(),
            "LEETCODE_SESSION=sess; csrftoken=tok"
        );
    }

    #[test]
    fn seed_writes_templates_once_and_does_not_clobber() {
        let dir = tmpdir("seed");
        let paths = seed_at(&dir).unwrap();
        assert!(paths.config.exists());
        assert!(paths.credentials.exists());

        // User edits the file; a second seed must not overwrite it.
        std::fs::write(
            &paths.credentials,
            "leetcode_session = \"mine\"\ncsrftoken = \"t\"\n",
        )
        .unwrap();
        seed_at(&dir).unwrap();
        let text = std::fs::read_to_string(&paths.credentials).unwrap();
        assert!(
            text.contains("mine"),
            "seed must not clobber existing credentials"
        );
    }

    #[test]
    fn seeded_credentials_file_contains_instructions_and_placeholders() {
        let dir = tmpdir("seed-template");
        let paths = seed_at(&dir).unwrap();
        let text = std::fs::read_to_string(&paths.credentials).unwrap();
        assert!(text.contains("LEETCODE_SESSION"));
        assert!(text.contains("csrftoken"));
        assert!(
            text.contains("DevTools"),
            "must explain how to obtain cookies"
        );
        let creds = Credentials::from_file(&paths.credentials).unwrap();
        assert!(!creds.is_configured(), "seeded creds must be placeholders");
    }
}
