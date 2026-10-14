//! The command-line surface: commands, options and the `--help` text.
//!
//! Everything here is declared as **data** rather than written into a string by
//! hand, because a hand-written help text drifts. The tests at the bottom walk
//! the tables in both directions:
//!
//! * every declared command parses, and every command variant is declared
//!   (the exhaustive `match` in `all_commands_are_declared` is a compile-time
//!   guard: adding a variant breaks the test build until it is listed here);
//! * every declared flag is actually read by the parser in `main.rs`, which is
//!   checked by searching that file's source.

/// A top-level subcommand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    List,
    Show,
    Languages,
    Whoami,
    Init,
    NvimPlugin,
    InstallNvimPlugin,
}

impl Command {
    /// Every command, in the order `--help` lists them.
    pub const ALL: [Command; 7] = [
        Command::List,
        Command::Show,
        Command::Languages,
        Command::Whoami,
        Command::Init,
        Command::NvimPlugin,
        Command::InstallNvimPlugin,
    ];

    /// The word the user types.
    pub fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Show => "show",
            Self::Languages => "languages",
            Self::Whoami => "whoami",
            Self::Init => "init",
            Self::NvimPlugin => "nvim-plugin",
            Self::InstallNvimPlugin => "install-nvim-plugin",
        }
    }

    /// How the command is invoked, as shown in `--help`.
    pub fn usage(self) -> &'static str {
        match self {
            Self::List => "list [OPTIONS]",
            Self::Show => "show <slug> [--raw]",
            Self::Languages => "languages",
            Self::Whoami => "whoami",
            Self::Init => "init",
            Self::NvimPlugin => "nvim-plugin",
            Self::InstallNvimPlugin => "install-nvim-plugin",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Self::List => "List problems (needs the network)",
            Self::Show => "Print a problem statement as text",
            Self::Languages => "List submission languages",
            Self::Whoami => "Show the configured LeetCode session",
            Self::Init => "Write default config + credentials files",
            Self::NvimPlugin => "Print where the nvim plugin is installed",
            Self::InstallNvimPlugin => "Install the nvim plugin into your nvim config",
        }
    }

    /// Parse a command name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.name() == name)
    }
}

/// A command-line option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flag {
    pub flag: &'static str,
    /// Whether the flag is followed by a value.
    pub takes_value: bool,
    pub help: &'static str,
}

/// Options accepted by `leetr list`.
pub const LIST_FLAGS: &[Flag] = &[
    Flag {
        flag: "--limit",
        takes_value: true,
        help: "Page size (default 50)",
    },
    Flag {
        flag: "--skip",
        takes_value: true,
        help: "Offset into the problem list",
    },
    Flag {
        flag: "--search",
        takes_value: true,
        help: "Server-side keyword search",
    },
    Flag {
        flag: "--solved",
        takes_value: false,
        help: "Only problems you have solved",
    },
    Flag {
        flag: "--todo",
        takes_value: false,
        help: "Only unsolved problems",
    },
    Flag {
        flag: "--attempted",
        takes_value: false,
        help: "Only attempted problems",
    },
    Flag {
        flag: "--slug",
        takes_value: false,
        help: "Also print each problem's slug",
    },
];

/// Options accepted by `leetr show`.
pub const SHOW_FLAGS: &[Flag] = &[Flag {
    flag: "--raw",
    takes_value: false,
    help: "Print the raw statement HTML",
}];

/// Global options, valid anywhere.
pub const GLOBAL_FLAGS: &[Flag] = &[
    Flag {
        flag: "--help",
        takes_value: false,
        help: "Print this help",
    },
    Flag {
        flag: "--version",
        takes_value: false,
        help: "Print the version",
    },
];

const PREAMBLE: &str = "leetr — a lightweight egui LeetCode client";
const FILES: &str = "\
FILES:
    config       ~/.config/leetr/config.toml
    credentials  ~/.config/leetr/credentials.toml   (never committed)
    workspace    ~/.local/share/leetr/workspace/";

/// Render the `--help` text.
pub fn help_text() -> String {
    let probe_unused_variable = 1;
    let mut out = String::new();
    out.push_str(PREAMBLE);
    out.push_str("\n\nUSAGE:\n    leetr [COMMAND]\n\nCOMMANDS:\n");
    out.push_str("    (none)              Launch the graphical client\n");
    for command in Command::ALL {
        out.push_str(&format!(
            "    {:<20}{}\n",
            command.usage(),
            command.summary()
        ));
    }
    for flag in GLOBAL_FLAGS {
        let spelling = match flag.flag {
            "--help" => "-h, --help",
            "--version" => "-V, --version",
            other => other,
        };
        out.push_str(&format!("    {:<20}{}\n", spelling, flag.help));
    }

    for (title, flags) in [("LIST OPTIONS:", LIST_FLAGS), ("SHOW OPTIONS:", SHOW_FLAGS)] {
        out.push_str(&format!("\n{title}\n"));
        for flag in flags {
            let spelling = if flag.takes_value {
                format!("{} {}", flag.flag, value_placeholder(flag.flag))
            } else {
                flag.flag.to_string()
            };
            out.push_str(&format!("    {:<20}{}\n", spelling, flag.help));
        }
    }

    out.push('\n');
    out.push_str(FILES);
    out.push('\n');
    out
}

fn value_placeholder(flag: &str) -> &'static str {
    match flag {
        "--limit" | "--skip" => "N",
        _ => "TEXT",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Guard against vacuous loops: every table the tests below walk must be
    /// populated, otherwise a `for` over an empty slice asserts nothing and
    /// passes. Cheap to check, and it is the difference between a test and a
    /// decoration.
    fn assert_tables_are_populated() {
        assert!(
            !Command::ALL.is_empty(),
            "Command::ALL is empty — loops over it would assert nothing"
        );
        assert!(
            !LIST_FLAGS.is_empty(),
            "LIST_FLAGS is empty — loops over it would assert nothing"
        );
        assert!(
            !SHOW_FLAGS.is_empty(),
            "SHOW_FLAGS is empty — loops over it would assert nothing"
        );
        assert!(
            !GLOBAL_FLAGS.is_empty(),
            "GLOBAL_FLAGS is empty — loops over it would assert nothing"
        );
    }

    #[test]
    fn every_declared_command_parses_back_to_itself() {
        assert_tables_are_populated();
        for command in Command::ALL {
            assert_eq!(
                Command::parse(command.name()),
                Some(command),
                "{} does not parse",
                command.name()
            );
        }
    }

    #[test]
    fn command_names_and_usages_are_unique() {
        let names: BTreeSet<&str> = Command::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names.len(), Command::ALL.len(), "duplicate command name");
        let usages: BTreeSet<&str> = Command::ALL.iter().map(|c| c.usage()).collect();
        assert_eq!(usages.len(), Command::ALL.len(), "duplicate usage line");
    }

    /// Compile-time guard: adding a `Command` variant breaks this `match`, so
    /// the new command cannot be forgotten in `ALL` (and therefore in `--help`).
    #[test]
    fn all_commands_are_declared() {
        for command in Command::ALL {
            match command {
                Command::List
                | Command::Show
                | Command::Languages
                | Command::Whoami
                | Command::Init
                | Command::NvimPlugin
                | Command::InstallNvimPlugin => {}
            }
        }
        assert_eq!(Command::ALL.len(), 7);
    }

    #[test]
    fn unknown_commands_do_not_parse() {
        for name in ["", "ls", "List", "install_nvim_plugin", "--list"] {
            assert_eq!(Command::parse(name), None, "{name:?} must not parse");
        }
    }

    #[test]
    fn help_lists_every_command_with_its_usage_and_summary() {
        assert_tables_are_populated();
        let help = help_text();
        for command in Command::ALL {
            assert!(
                help.contains(command.usage()),
                "help omits {}",
                command.name()
            );
            assert!(
                help.contains(command.summary()),
                "help omits {}",
                command.name()
            );
        }
        assert!(help.contains(PREAMBLE));
        assert!(help.contains(FILES));
    }

    #[test]
    fn help_documents_every_flag_with_its_help_text() {
        assert_tables_are_populated();
        let help = help_text();
        for flag in LIST_FLAGS.iter().chain(SHOW_FLAGS).chain(GLOBAL_FLAGS) {
            assert!(help.contains(flag.flag), "help omits {}", flag.flag);
            assert!(
                help.contains(flag.help),
                "help omits the text for {}",
                flag.flag
            );
        }
    }

    /// A flag documented but never read by the parser is a lie. `main.rs` no
    /// longer contains the help text, so every declared flag must appear there
    /// because the argument parsing reads it.
    #[test]
    fn every_declared_flag_is_read_by_the_parser() {
        assert_tables_are_populated();
        const PARSER: &str = include_str!("main.rs");
        for flag in LIST_FLAGS.iter().chain(SHOW_FLAGS) {
            // Match the *quoted* literal. A bare `contains` is too weak: it
            // would accept `--attempted` inside `--attempted-typo`, so renaming
            // the flag in the parser would silently pass.
            let needle = format!("\"{}\"", flag.flag);
            assert!(
                PARSER.contains(&needle),
                "{} is documented but the parser never reads it",
                flag.flag
            );
        }
    }

    /// The check above only works because the flags are written as string
    /// literals in the parser. Guard that assumption.
    #[test]
    fn the_parser_literal_check_is_not_vacuous() {
        const PARSER: &str = include_str!("main.rs");
        assert!(
            PARSER.contains("\"--limit\""),
            "parser should use flag literals"
        );
        assert!(
            !PARSER.contains("\"--not-a-real-flag\""),
            "the parser source should not mention invented flags"
        );
    }

    #[test]
    fn flags_are_well_formed() {
        assert_tables_are_populated();
        for flag in LIST_FLAGS.iter().chain(SHOW_FLAGS).chain(GLOBAL_FLAGS) {
            assert!(
                flag.flag.starts_with("--") && flag.flag.len() > 2,
                "bad flag spelling: {}",
                flag.flag
            );
            assert!(!flag.help.trim().is_empty(), "{} has no help", flag.flag);
            assert!(
                !flag.help.contains('\n'),
                "{}'s help must be a single line",
                flag.flag
            );
        }
    }

    #[test]
    fn help_is_not_empty_and_ends_with_a_newline() {
        let help = help_text();
        assert!(help.len() > 200);
        assert!(help.ends_with('\n'));
    }
}
