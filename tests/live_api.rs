//! Live integration tests against the public LeetCode API.
//!
//! These are `#[ignore]`d so a plain `cargo test` stays offline and fast.
//! Run them explicitly:
//!
//! ```text
//! cargo test --test live_api -- --ignored --nocapture
//! ```
//!
//! The authenticated test needs credentials in `~/.config/leetr/credentials.toml`
//! (or `LEETCODE_SESSION` / `LEETCODE_CSRFTOKEN`); it is skipped, not failed,
//! when they are absent.

use leetr::config::Credentials;
use leetr::leetcode::client::ProblemQuery;
use leetr::leetcode::{html, LeetCodeClient};
use leetr::model::{Difficulty, ProblemStatus};

fn public_client() -> LeetCodeClient {
    LeetCodeClient::new(Credentials::default()).expect("building the client")
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn fetches_a_page_of_problems() {
    let page = public_client()
        .fetch_problems(&ProblemQuery::new(5))
        .expect("problem list");

    assert!(
        page.total > 1000,
        "expected a large problem set, got {}",
        page.total
    );
    assert_eq!(page.problems.len(), 5);
    for problem in &page.problems {
        assert!(!problem.title.is_empty());
        assert!(!problem.slug.is_empty());
        assert_ne!(problem.difficulty, Difficulty::Unknown);
    }
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn paging_returns_different_problems() {
    let client = public_client();
    let first = client.fetch_problems(&ProblemQuery::new(3)).unwrap();
    let second = client
        .fetch_problems(&ProblemQuery {
            skip: 3,
            ..ProblemQuery::new(3)
        })
        .unwrap();

    assert_ne!(first.problems[0].slug, second.problems[0].slug);
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn server_side_search_filters_results() {
    let page = public_client()
        .fetch_problems(&ProblemQuery {
            search: Some("two sum".into()),
            ..ProblemQuery::new(10)
        })
        .expect("search");

    assert!(!page.problems.is_empty());
    assert!(
        page.problems
            .iter()
            .any(|p| p.slug == "two-sum" || p.title.to_lowercase().contains("two sum")),
        "expected two-sum in the results, got {:?}",
        page.problems.iter().map(|p| &p.slug).collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn fetches_the_two_sum_statement() {
    let detail = public_client().fetch_question("two-sum").expect("two-sum");

    assert_eq!(detail.frontend_id, "1");
    assert_eq!(detail.slug, "two-sum");
    assert_eq!(detail.difficulty, Difficulty::Easy);
    assert!(!detail.paid_only);
    assert!(
        !detail.snippets.is_empty(),
        "expected starter code snippets"
    );
    assert!(
        detail.snippet_for("python3").is_some(),
        "expected a python3 snippet"
    );

    let text = html::to_plain_text(&html::parse(&detail.content_html));
    assert!(text.contains("You are given an array of integers"));
    assert!(text.contains("Constraints"));
    assert!(!text.contains("<p>"), "raw HTML leaked: {text}");
    assert!(!text.contains("</"), "raw HTML leaked: {text}");
    assert!(!text.contains("&lt;"), "entities leaked: {text}");
    assert!(!detail.sample_test_case.is_empty());
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn fetches_languages() {
    let languages = public_client().fetch_languages().expect("languages");
    assert!(languages.len() > 10);

    let python = languages
        .iter()
        .find(|l| l.name == "python3")
        .expect("python3 must be offered");
    assert_eq!(python.id, 11);
    assert_eq!(python.verbose_name, "Python3");

    assert!(languages.iter().any(|l| l.name == "rust"));
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn unsigned_client_reports_not_signed_in() {
    let status = public_client().fetch_user_status().expect("userStatus");
    assert!(!status.signed_in);
}

#[test]
#[ignore = "hits the live LeetCode API"]
fn authenticated_session_can_read_the_solved_set() {
    let credentials = Credentials::load();
    if !credentials.is_configured() {
        eprintln!(
            "skipping: no credentials at {}",
            leetr::config::credentials_path().display()
        );
        return;
    }

    let client = LeetCodeClient::new(credentials).expect("client");
    let status = client.fetch_user_status().expect("userStatus");
    assert!(
        status.signed_in,
        "credentials are configured but LeetCode rejected the session"
    );
    eprintln!("signed in as {}", status.display_name());

    let solved = client.fetch_solved_slugs().expect("solved set");
    eprintln!("{} problems solved", solved.len());

    // The solved set must be usable for the submit safety guard.
    let solved_page = client
        .fetch_problems(&ProblemQuery {
            status: Some(ProblemStatus::Solved),
            ..ProblemQuery::new(5)
        })
        .expect("solved page");
    assert!(solved_page.total as usize == solved.len());
}

/// Drives the real statement pane with real content, headlessly.
#[test]
#[ignore = "hits the live LeetCode API"]
fn statement_pane_renders_real_content() {
    let detail = public_client().fetch_question("two-sum").expect("two-sum");
    let blocks = html::parse(&detail.content_html);
    assert!(
        blocks.len() > 5,
        "expected a rich statement, got {} blocks",
        blocks.len()
    );

    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        leetr::ui::statement::show(ui, Some(&detail), &blocks, false, None);
    });
    output.textures_delta.clear();
}

/// Drives the real problem-list pane against a page of live problems.
#[test]
#[ignore = "hits the live LeetCode API"]
fn problem_list_pane_renders_real_content() {
    let page = public_client()
        .fetch_problems(&ProblemQuery::new(25))
        .expect("problem list");
    let mut state = leetr::ui::problem_list::ProblemListState::default();

    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let _ = leetr::ui::problem_list::show(
            ui,
            &mut state,
            &page.problems,
            page.total,
            false,
            None,
            true,
            Some(page.problems[0].slug.as_str()),
        );
    });
    output.textures_delta.clear();

    assert_eq!(state.matching(&page.problems).len(), page.problems.len());
}
