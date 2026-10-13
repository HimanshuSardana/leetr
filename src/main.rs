//! Leetr — a lightweight `egui` LeetCode client with first-class nvim interoperability.

mod app;
mod config;
mod theme;
mod ui;

use anyhow::Result;

const HELP: &str = "\
leetr — a lightweight egui LeetCode client

USAGE:
    leetr [COMMAND]

COMMANDS:
    (none)          Launch the graphical client
    init            Write default config + credentials files and print their paths
    nvim-plugin     Print the path to the bundled nvim plugin
    -h, --help      Print this help
    -V, --version   Print the version

FILES:
    config       ~/.config/leetr/config.toml
    credentials  ~/.config/leetr/credentials.toml   (never committed)
    workspace    ~/.local/share/leetr/workspace/
";

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("-h") | Some("--help") => println!("{HELP}"),
        Some("-V") | Some("--version") => println!("leetr {}", env!("CARGO_PKG_VERSION")),
        Some("init") => {
            let paths = config::seed()?;
            println!("config       {}", paths.config.display());
            println!("credentials  {}", paths.credentials.display());
        }
        Some("nvim-plugin") => {
            println!("{}", env!("CARGO_MANIFEST_DIR"));
        }
        Some(other) => {
            eprintln!("error: unknown command `{other}`\n\n{HELP}");
            std::process::exit(2);
        }
        None => app::run()?,
    }
    Ok(())
}
