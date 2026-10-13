//! Command-line entry point.

use anyhow::{bail, Result};
use leetr::app;
use leetr::config::{self, Credentials};
use leetr::leetcode::client::ProblemQuery;
use leetr::leetcode::{html, LeetCodeClient};
use leetr::model::{ProblemStatus, QuestionDetail};
use leetr::nvim;

const HELP: &str = "\
leetr — a lightweight egui LeetCode client

USAGE:
    leetr [COMMAND]

COMMANDS:
    (none)                  Launch the graphical client
    list [OPTIONS]          List problems (needs the network)
    show <slug> [--raw]     Print a problem statement as text (--raw: HTML)
    languages               List submission languages
    whoami                  Show the configured LeetCode session
    init                    Write default config + credentials files
    nvim-plugin             Print the path to the bundled nvim plugin
    install-nvim-plugin     Install the nvim plugin into your nvim config
    -h, --help              Print this help
    -V, --version           Print the version

LIST OPTIONS:
    --limit N               Page size (default 50)
    --skip N                Offset into the problem list
    --search TEXT           Server-side keyword search
    --solved                Only problems you have solved
    --todo                  Only unsolved problems
    --attempted             Only attempted problems
    --slug                  Also print each problem's slug

FILES:
    config       ~/.config/leetr/config.toml
    credentials  ~/.config/leetr/credentials.toml   (never committed)
    workspace    ~/.local/share/leetr/workspace/
";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(String::as_str) {
        None => app::run()?,
        Some("-h") | Some("--help") => println!("{HELP}"),
        Some("-V") | Some("--version") => println!("leetr {}", env!("CARGO_PKG_VERSION")),
        Some("init") => {
            let paths = config::seed()?;
            println!("config       {}", paths.config.display());
            println!("credentials  {}", paths.credentials.display());
        }
        Some("nvim-plugin") => {
            // The plugin is embedded in the binary (`include_str!`), so there is
            // no meaningful "source path" to print — CARGO_MANIFEST_DIR is the
            // machine that built the binary and usually does not exist for the
            // person running it.
            let dest = nvim::default_plugin_path()
                .ok_or_else(|| anyhow::anyhow!("could not locate your config directory"))?;
            println!("{}", dest.display());
            println!("the plugin is embedded in this binary; install it with `leetr install-nvim-plugin`");
        }
        Some("install-nvim-plugin") => cmd_install_nvim_plugin()?,
        Some(_) => run_command(&args)?,
    }

    Ok(())
}

/// Copy the embedded plugin into the user's nvim config.
fn cmd_install_nvim_plugin() -> Result<()> {
    let dest = nvim::default_plugin_path()
        .ok_or_else(|| anyhow::anyhow!("could not locate your config directory"))?;

    match nvim::install_plugin(&dest) {
        Ok(true) => println!("installed {}", dest.display()),
        Ok(false) => println!("already up to date: {}", dest.display()),
        Err(err) => bail!("{err}"),
    }

    println!();
    print!("{}", nvim::install_hint());
    Ok(())
}

fn run_command(args: &[String]) -> Result<()> {
    let client = LeetCodeClient::new(Credentials::load())?;

    match args[0].as_str() {
        "list" => cmd_list(&client, &args[1..]),
        "show" => cmd_show(&client, &args[1..]),
        "languages" => cmd_languages(&client),
        "whoami" => cmd_whoami(&client),
        other => {
            eprintln!("error: unknown command `{other}`\n\n{HELP}");
            std::process::exit(2);
        }
    }
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|a| a == name)?;
    args.get(index + 1).cloned()
}

fn flag_u32(args: &[String], name: &str, default: u32) -> Result<u32> {
    match flag_value(args, name) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<u32>()
            .map_err(|_| anyhow::anyhow!("`{name}` expects a number, got `{raw}`")),
    }
}

fn cmd_list(client: &LeetCodeClient, args: &[String]) -> Result<()> {
    let limit = flag_u32(args, "--limit", 50)?;
    let skip = flag_u32(args, "--skip", 0)?;
    let search = flag_value(args, "--search");

    let status = if args.iter().any(|a| a == "--solved") {
        Some(ProblemStatus::Solved)
    } else if args.iter().any(|a| a == "--todo") {
        Some(ProblemStatus::Todo)
    } else if args.iter().any(|a| a == "--attempted") {
        Some(ProblemStatus::Attempted)
    } else {
        None
    };

    if status.is_some() && !client.is_authenticated() {
        eprintln!(
            "warning: status filters need credentials ({})",
            config::credentials_path().display()
        );
    }

    let page = client.fetch_problems(&ProblemQuery {
        skip,
        limit,
        status,
        search,
    })?;

    let with_slug = args.iter().any(|a| a == "--slug");
    for problem in &page.problems {
        let marker = problem.status.marker();
        let paid = if problem.paid_only { " [premium]" } else { "" };
        if with_slug {
            println!(
                "{:>5}  [{}] {:<6}  {} ({}){}",
                problem.id,
                marker,
                problem.difficulty.label(),
                problem.title,
                problem.slug,
                paid
            );
        } else {
            println!(
                "{:>5}  [{}] {:<6}  {}{}",
                problem.id,
                marker,
                problem.difficulty.label(),
                problem.title,
                paid
            );
        }
    }

    println!(
        "\nshowing {} of {} problems",
        page.problems.len(),
        page.total
    );
    Ok(())
}

fn cmd_show(client: &LeetCodeClient, args: &[String]) -> Result<()> {
    let slug = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .ok_or_else(|| anyhow::anyhow!("usage: leetr show <slug> [--raw]"))?;

    let detail = client.fetch_question(slug)?;

    if args.iter().any(|a| a == "--raw") {
        println!("{}", detail.content_html);
    } else {
        print_statement(&detail);
    }
    Ok(())
}

fn print_statement(detail: &QuestionDetail) {
    println!("{}  {}", detail.frontend_id, detail.title);
    let tags = detail
        .tags
        .iter()
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if tags.is_empty() {
        println!("{}", detail.difficulty.label());
    } else {
        println!("{}  ·  {}", detail.difficulty.label(), tags);
    }
    println!("{}", "─".repeat(60));
    println!(
        "{}",
        html::to_plain_text(&html::parse(&detail.content_html))
    );

    if !detail.hints.is_empty() {
        println!("\nHints:");
        for hint in &detail.hints {
            println!("  • {}", html::to_plain_text(&html::parse(hint)));
        }
    }

    if !detail.snippets.is_empty() {
        let languages = detail
            .snippets
            .iter()
            .map(|s| s.lang_slug.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!("\nStarter code available for: {languages}");
    }
}

fn cmd_languages(client: &LeetCodeClient) -> Result<()> {
    let languages = client.fetch_languages()?;
    for language in &languages {
        println!(
            "{:<4}  {:<12}  {}",
            language.id, language.name, language.verbose_name
        );
    }
    println!("\n{} languages", languages.len());
    Ok(())
}

fn cmd_whoami(client: &LeetCodeClient) -> Result<()> {
    if !client.is_authenticated() {
        println!("not signed in — no credentials configured");
        println!("add them to {}", config::credentials_path().display());
        return Ok(());
    }

    let status = client.fetch_user_status()?;
    if !status.signed_in {
        bail!(
            "credentials found, but LeetCode says the session is not signed in \
             (the cookies have probably expired)"
        );
    }

    println!(
        "signed in as {}{}",
        status.display_name(),
        if status.premium { " (premium)" } else { "" }
    );

    match client.fetch_solved_slugs() {
        Ok(solved) => println!("{} problems solved", solved.len()),
        Err(err) => println!("could not fetch the solved set: {err}"),
    }

    Ok(())
}
