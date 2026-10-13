//! Opening the user's editor on a workspace file.
//!
//! The interesting part is deciding *how* to launch it, which is pure logic so
//! it can be tested without spawning anything:
//!
//! 1. an explicit `editor_command` from the config always wins,
//! 2. otherwise, inside tmux, open a horizontal split,
//! 3. otherwise, use `$TERMINAL` if set,
//! 4. otherwise, run `nvim` directly.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The bundled nvim plugin, embedded so it works from an installed binary too.
pub const PLUGIN_SOURCE: &str = include_str!("../contrib/nvim/leetr.lua");

/// How to launch the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// `tmux split-window -h -c <dir> nvim <file>`
    Tmux,
    /// A terminal emulator, e.g. `["kitty", "-e"]`, followed by `nvim <file>`.
    Terminal(Vec<String>),
    /// Run the editor directly.
    Direct(Vec<String>),
}

impl Launch {
    pub fn describe(&self) -> String {
        match self {
            Self::Tmux => "tmux split-window".to_string(),
            Self::Terminal(parts) => format!("{} (terminal)", parts.join(" ")),
            Self::Direct(parts) => parts.join(" "),
        }
    }
}

/// Decide how to launch the editor.
pub fn plan(editor_command: &str, tmux: Option<&str>, terminal: Option<&str>) -> Launch {
    let editor_command = editor_command.trim();

    if !editor_command.is_empty() {
        return Launch::Direct(split_words(editor_command));
    }
    if tmux.is_some_and(|t| !t.trim().is_empty()) {
        return Launch::Tmux;
    }
    if let Some(terminal) = terminal.map(str::trim).filter(|t| !t.is_empty()) {
        return Launch::Terminal(split_words(terminal));
    }

    Launch::Direct(vec!["nvim".to_string()])
}

/// Decide how to launch the editor, reading the real environment.
pub fn plan_from_env(editor_command: &str) -> Launch {
    let tmux = std::env::var("TMUX").ok();
    let terminal = std::env::var("TERMINAL").ok();
    plan(editor_command, tmux.as_deref(), terminal.as_deref())
}

/// Open `file` in the user's editor.
///
/// The caller's environment is inherited, which is what makes `$TMUX` and
/// terminal multiplexing work.
pub fn open(file: &Path, editor_command: &str) -> Result<String, String> {
    let launch = plan_from_env(editor_command);
    let file = file.display().to_string();

    let mut command = match &launch {
        Launch::Tmux => {
            let mut command = Command::new("tmux");
            command.arg("split-window").arg("-h");
            if let Some(dir) = file.rsplit_once('/').map(|(dir, _)| dir) {
                command.arg("-c").arg(dir);
            }
            command.arg("nvim").arg(&file);
            command
        }
        Launch::Terminal(parts) => {
            let (program, args) = parts.split_first().ok_or("empty terminal command")?;
            let mut command = Command::new(program);
            command.args(args);
            command.arg("nvim").arg(&file);
            command
        }
        Launch::Direct(parts) => {
            let (program, args) = parts.split_first().ok_or("empty editor command")?;
            let mut command = Command::new(program);
            command.args(args).arg(&file);
            command
        }
    };

    command
        .stdin(std::process::Stdio::null())
        .spawn()
        .map(|_| launch.describe())
        .map_err(|err| format!("could not launch `{}`: {err}", launch.describe()))
}

/// Split a command string on whitespace.
///
/// Deliberately simple: no quoting or escapes, which is documented in the
/// config template. Enough for `kitty -e nvim` style values.
fn split_words(command: &str) -> Vec<String> {
    command.split_whitespace().map(str::to_string).collect()
}

/// Where the plugin should live for the user's nvim.
pub fn default_plugin_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("nvim").join("lua").join("leetr.lua"))
}

/// Write the bundled plugin to `dest`.
///
/// Returns `Ok(false)` when the file is already up to date, so running the
/// installer repeatedly is harmless.
pub fn install_plugin(dest: &Path) -> Result<bool, String> {
    if let Ok(existing) = std::fs::read_to_string(dest) {
        if existing == PLUGIN_SOURCE {
            return Ok(false);
        }
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("creating {}: {err}", parent.display()))?;
    }
    std::fs::write(dest, PLUGIN_SOURCE)
        .map_err(|err| format!("writing {}: {err}", dest.display()))?;
    Ok(true)
}

/// The `require("leetr")` line to add to an nvim config.
pub fn setup_snippet() -> &'static str {
    "require(\"leetr\").setup()"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_editor_command_wins_over_everything() {
        let launch = plan("kitty -e nvim", Some("/tmp/tmux"), Some("xterm"));
        assert_eq!(
            launch,
            Launch::Direct(vec!["kitty".into(), "-e".into(), "nvim".into()])
        );
    }

    #[test]
    fn tmux_is_used_when_no_explicit_command_is_set() {
        assert_eq!(
            plan("", Some("/tmp/tmux-0/default,1,0"), Some("kitty")),
            Launch::Tmux
        );
    }

    #[test]
    fn terminal_is_used_outside_tmux() {
        assert_eq!(
            plan("", None, Some("alacritty -e")),
            Launch::Terminal(vec!["alacritty".into(), "-e".into()])
        );
    }

    #[test]
    fn nvim_is_the_last_resort() {
        assert_eq!(plan("", None, None), Launch::Direct(vec!["nvim".into()]));
    }

    #[test]
    fn blank_environment_values_are_ignored() {
        assert_eq!(
            plan("", Some("  "), Some("")),
            Launch::Direct(vec!["nvim".into()])
        );
        assert_eq!(
            plan("   ", Some(""), None),
            Launch::Direct(vec!["nvim".into()])
        );
    }

    #[test]
    fn launch_descriptions_are_human_readable() {
        assert_eq!(plan("", Some("x"), None).describe(), "tmux split-window");
        assert_eq!(plan("nvim", None, None).describe(), "nvim");
        assert!(plan("", None, Some("kitty -e"))
            .describe()
            .contains("kitty"));
    }

    #[test]
    fn embedded_plugin_defines_the_expected_commands() {
        for command in [
            "LeetrStatus",
            "LeetrOpen",
            "LeetrPull",
            "LeetrPush",
            "LeetrPort",
            "LeetrRun",
            "LeetrSubmit",
            "LeetrResult",
            "leetcode",
        ] {
            let expected = if command == "leetcode" {
                "leetr.nvim"
            } else {
                command
            };
            assert!(
                PLUGIN_SOURCE.contains(expected),
                "plugin should mention {expected}"
            );
        }
    }

    /// Actually load the plugin in neovim and check its commands exist.
    ///
    /// Skipped (not failed) when nvim is not installed, so CI images without it
    /// still pass.
    #[test]
    fn plugin_loads_and_registers_commands_in_neovim() {
        let Some(nvim) = find_nvim() else {
            eprintln!("skipping: nvim is not on PATH");
            return;
        };

        let dir = std::env::temp_dir().join(format!("leetr-nvim-load-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let plugin = dir.join("leetr.lua");
        std::fs::write(&plugin, PLUGIN_SOURCE).unwrap();

        let script = r#"lua
local ok, m = pcall(require, "leetr")
if not ok then print("LOAD FAILED: " .. tostring(m)); vim.cmd("cq") end
m.setup({ auto_push = false })
local cmds = vim.api.nvim_get_commands({})
local want = { "LeetrStatus", "LeetrOpen", "LeetrPull", "LeetrPush", "LeetrPort", "LeetrRun", "LeetrSubmit", "LeetrResult" }
local missing = {}
for _, c in ipairs(want) do
  if not cmds[c] then table.insert(missing, c) end
end
if #missing > 0 then print("MISSING: " .. table.concat(missing, ",")); vim.cmd("cq") end
print("PLUGIN_OK")
vim.cmd("q")"#;

        let output = Command::new(nvim)
            .args(["--headless", "-u", "NONE"])
            .arg("--cmd")
            .arg(format!("set rtp+={}", dir.display()))
            .arg("--cmd")
            .arg(format!(
                "lua package.path = '{}/?.lua;' .. package.path",
                dir.display()
            ))
            .arg("-c")
            .arg(script)
            .output();

        let _ = std::fs::remove_dir_all(&dir);

        let output = output.expect("running nvim");
        // nvim's `print` goes to stderr in headless mode, so check both.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let combined = format!("{stdout}{stderr}");
        assert!(
            output.status.success() && combined.contains("PLUGIN_OK"),
            "nvim could not load the plugin.\nstatus: {:?}\nstdout: {stdout}\nstderr: {stderr}",
            output.status.code()
        );
    }

    fn find_nvim() -> Option<&'static str> {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join("nvim"))
            .find(|candidate| candidate.is_file())
            .map(|_| "nvim")
    }

    #[test]
    fn installing_the_plugin_writes_it_and_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("leetr-nvim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dest = dir.join("nvim/lua/leetr.lua");

        assert!(install_plugin(&dest).unwrap(), "first install writes");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), PLUGIN_SOURCE);
        assert!(!install_plugin(&dest).unwrap(), "second install is a no-op");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installing_over_a_stale_copy_updates_it() {
        let dir = std::env::temp_dir().join(format!("leetr-nvim-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let dest = dir.join("leetr.lua");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&dest, "-- old version").unwrap();

        assert!(install_plugin(&dest).unwrap());
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), PLUGIN_SOURCE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_plugin_path_points_at_the_nvim_config() {
        if let Some(path) = default_plugin_path() {
            assert!(
                path.ends_with("nvim/lua/leetr.lua"),
                "got {}",
                path.display()
            );
        }
    }

    #[test]
    fn setup_snippet_is_valid_lua() {
        assert_eq!(setup_snippet(), "require(\"leetr\").setup()");
    }

    #[test]
    fn splitting_is_whitespace_based() {
        assert_eq!(
            split_words("  a   b  "),
            vec!["a".to_string(), "b".to_string()]
        );
        assert!(split_words("").is_empty());
    }
}
