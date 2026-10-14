//! Blocking HTTP client for LeetCode's GraphQL API.
//!
//! Everything is synchronous: the GUI runs these calls on a worker thread (see
//! the background-task plumbing added with the problem browser), and the CLI
//! calls them directly. No async runtime, which keeps the binary small.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER};
use serde::de::DeserializeOwned;

use crate::config::Credentials;
use crate::model::{
    CodeSnippet, Difficulty, Language, Problem, ProblemPage, ProblemStatus, QuestionDetail, Tag,
    UserStatus,
};

use super::graphql as g;

/// LeetCode's public origin.
pub const DEFAULT_BASE_URL: &str = "https://leetcode.com";

const USER_AGENT: &str = concat!(
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) ",
    "leetr/",
    env!("CARGO_PKG_VERSION")
);

/// LeetCode silently truncates `limit` to this many results per request, so
/// pagination must advance by the number actually returned.
pub const MAX_PAGE_SIZE: u32 = 100;

/// Cap on how many pages the solved-set lookups will walk.
const MAX_SOLVED_PAGES: usize = 30;
/// Cap on the full-scan fallback used by `fetch_solved_slugs`.
const MAX_SCAN_PAGES: usize = 50;

/// Which problems the list query should return.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProblemQuery {
    pub skip: u32,
    pub limit: u32,
    /// Restrict to problems with this status (requires credentials).
    pub status: Option<ProblemStatus>,
    /// Server-side keyword search.
    pub search: Option<String>,
}

impl ProblemQuery {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }

    fn filters_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        if let Some(status) = self.status {
            map.insert(
                "status".to_string(),
                serde_json::Value::String(status.as_filter().to_string()),
            );
        }
        if let Some(search) = self.search.as_ref().filter(|s| !s.trim().is_empty()) {
            map.insert(
                "searchKeywords".to_string(),
                serde_json::Value::String(search.trim().to_string()),
            );
        }
        serde_json::Value::Object(map)
    }
}

/// A cloneable LeetCode API client.
#[derive(Clone)]
pub struct LeetCodeClient {
    http: Client,
    credentials: Credentials,
    base_url: String,
}

impl LeetCodeClient {
    pub fn new(credentials: Credentials) -> Result<Self> {
        Self::with_base_url(credentials, DEFAULT_BASE_URL)
    }

    pub fn with_base_url(credentials: Credentials, base_url: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(USER_AGENT)
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            http,
            credentials,
            base_url,
        })
    }

    /// True when session cookies are present (not that they are still valid).
    pub fn is_authenticated(&self) -> bool {
        self.credentials.is_configured()
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Exposed so the judge (run/submit) endpoints can reuse the same
    /// connection pool and auth headers.
    pub(crate) fn http(&self) -> &Client {
        &self.http
    }

    pub(crate) fn credentials(&self) -> &Credentials {
        &self.credentials
    }

    /// Who, if anyone, the configured session belongs to.
    pub fn fetch_user_status(&self) -> Result<UserStatus> {
        let data: g::UserStatusData = self.graphql(g::Q_USER_STATUS, serde_json::json!({}))?;
        let wire = data.user_status;
        Ok(UserStatus {
            signed_in: wire.is_signed_in,
            username: wire.username.unwrap_or_default(),
            premium: wire.is_premium.unwrap_or(false),
        })
    }

    /// Every language LeetCode accepts submissions in.
    pub fn fetch_languages(&self) -> Result<Vec<Language>> {
        let data: g::LanguagesData = self.graphql(g::Q_LANGUAGES, serde_json::json!({}))?;
        Ok(data
            .language_list
            .into_iter()
            .map(|l| Language {
                id: l.id,
                name: l.name,
                verbose_name: l.verbose_name,
            })
            .collect())
    }

    /// One page of the problem list.
    ///
    /// The per-problem `status` field is only populated when credentials are
    /// configured. `query.limit` is clamped to [`MAX_PAGE_SIZE`]: LeetCode
    /// silently returns at most that many rows.
    pub fn fetch_problems(&self, query: &ProblemQuery) -> Result<ProblemPage> {
        if query.limit == 0 {
            bail!("limit must be greater than zero");
        }
        let limit = query.limit.min(MAX_PAGE_SIZE);

        let variables = serde_json::json!({
            "categorySlug": "",
            "skip": query.skip,
            "limit": limit,
            "filters": query.filters_json(),
        });

        let data: g::ProblemListData = self.graphql(g::Q_PROBLEM_LIST, variables)?;
        let wire = data.problemset_question_list;

        Ok(ProblemPage {
            total: wire.total,
            problems: wire.questions.into_iter().map(problem_from_wire).collect(),
        })
    }

    /// Slugs of every problem the signed-in user has solved.
    ///
    /// Requires credentials. The fast path asks LeetCode to filter by the
    /// `AC` status; if that ever stops working (the enum has changed before)
    /// it falls back to scanning the list and reading each problem's own
    /// `status` field, which is populated for authenticated requests.
    pub fn fetch_solved_slugs(&self) -> Result<BTreeSet<String>> {
        if let Ok(solved) = self.solved_via_status_filter() {
            return Ok(solved);
        }
        self.solved_via_scan()
    }

    fn solved_via_status_filter(&self) -> Result<BTreeSet<String>> {
        self.collect_pages(Some(ProblemStatus::Solved), MAX_SOLVED_PAGES)
    }

    /// Fallback: page through the whole list and keep the accepted ones.
    fn solved_via_scan(&self) -> Result<BTreeSet<String>> {
        self.collect_pages(None, MAX_SCAN_PAGES)
    }

    /// Walk the problem list, keeping slugs that match `filter`.
    ///
    /// Pages advance by however many rows LeetCode actually returned, because
    /// the server clamps `limit` to [`MAX_PAGE_SIZE`].
    fn collect_pages(
        &self,
        status: Option<ProblemStatus>,
        max_pages: usize,
    ) -> Result<BTreeSet<String>> {
        let mut collected = BTreeSet::new();
        let mut skip = 0u32;
        let mut total = u32::MAX;

        for _ in 0..max_pages {
            if skip >= total {
                break;
            }

            let page = self.fetch_problems(&ProblemQuery {
                skip,
                limit: MAX_PAGE_SIZE,
                status,
                search: None,
            })?;
            total = page.total;

            let fetched = page.problems.len() as u32;
            if fetched == 0 {
                break;
            }

            collected.extend(
                page.problems
                    .into_iter()
                    .filter(|p| status.is_none() || p.status == ProblemStatus::Solved)
                    .map(|p| p.slug),
            );
            skip += fetched;
        }

        Ok(collected)
    }

    /// The full statement, snippets and metadata for one problem.
    pub fn fetch_question(&self, slug: &str) -> Result<QuestionDetail> {
        let slug = slug.trim();
        if slug.is_empty() {
            bail!("a problem slug is required");
        }

        let variables = serde_json::json!({ "titleSlug": slug });
        let data: g::QuestionData = self.graphql(g::Q_QUESTION, variables)?;
        let q = data.question;

        Ok(QuestionDetail {
            question_id: q.question_id,
            frontend_id: q.question_frontend_id,
            title: q.title,
            slug: q.title_slug,
            difficulty: Difficulty::from_wire(&q.difficulty),
            paid_only: q.is_paid_only,
            content_html: q.content.unwrap_or_default(),
            sample_test_case: q.sample_test_case.unwrap_or_default(),
            example_testcases: q.example_testcases.unwrap_or_default(),
            hints: q.hints.unwrap_or_default(),
            tags: q.topic_tags.into_iter().map(tag_from_wire).collect(),
            snippets: q
                .code_snippets
                .into_iter()
                .map(|s| CodeSnippet {
                    lang: s.lang,
                    lang_slug: s.lang_slug,
                    code: s.code,
                })
                .collect(),
        })
    }

    /// POST a GraphQL query and unwrap the `data` field.
    fn graphql<T: DeserializeOwned>(&self, query: &str, variables: serde_json::Value) -> Result<T> {
        let url = format!("{}/graphql/", self.base_url);
        let body = serde_json::json!({ "query": query, "variables": variables });

        let mut request = self
            .http
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .header(REFERER, format!("{}/", self.base_url))
            .header(ORIGIN, &self.base_url)
            .header("X-Requested-With", "XMLHttpRequest")
            .json(&body);

        if self.credentials.is_configured() {
            request = request
                .header(COOKIE, self.credentials.cookie_header())
                .header("x-csrftoken", self.credentials.csrftoken.clone());
        }

        let response = request
            .send()
            .with_context(|| format!("POST {url} (is the network available?)"))?;
        let status = response.status();
        let text = response.text().context("reading the response body")?;

        if !status.is_success() {
            bail!("LeetCode returned HTTP {status}: {}", truncate(&text, 300));
        }

        let envelope: g::Envelope<T> = serde_json::from_str(&text)
            .with_context(|| format!("decoding the GraphQL response: {}", truncate(&text, 300)))?;

        if !envelope.errors.is_empty() {
            let messages = envelope
                .errors
                .iter()
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>()
                .join("; ");
            bail!("LeetCode GraphQL error: {messages}");
        }

        envelope
            .data
            .ok_or_else(|| anyhow!("LeetCode returned an empty response"))
    }
}

fn problem_from_wire(w: g::WireProblem) -> Problem {
    Problem {
        id: w.frontend_question_id,
        title: w.title,
        slug: w.title_slug,
        difficulty: Difficulty::from_wire(&w.difficulty),
        paid_only: w.is_paid_only,
        status: ProblemStatus::from_wire(w.status.as_deref()),
        ac_rate: w.ac_rate,
        tags: w.topic_tags.into_iter().map(tag_from_wire).collect(),
    }
}

fn tag_from_wire(w: g::WireTag) -> Tag {
    Tag {
        name: w.name,
        slug: w.slug,
    }
}

fn truncate(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> LeetCodeClient {
        LeetCodeClient::new(Credentials::default()).unwrap()
    }

    #[test]
    fn base_url_is_trimmed() {
        let c = LeetCodeClient::with_base_url(Credentials::default(), "http://localhost:1234/")
            .unwrap();
        assert_eq!(c.base_url(), "http://localhost:1234");
    }

    #[test]
    fn unauthenticated_client_reports_so() {
        assert!(!client().is_authenticated());
    }

    #[test]
    fn authenticated_client_reports_so() {
        let creds = Credentials {
            leetcode_session: "s".into(),
            csrftoken: "t".into(),
        };
        assert!(LeetCodeClient::new(creds).unwrap().is_authenticated());
    }

    #[test]
    fn empty_filters_when_no_status_or_search() {
        let q = ProblemQuery::new(50);
        assert_eq!(q.filters_json(), serde_json::json!({}));
    }

    #[test]
    fn filters_include_status_and_search() {
        let q = ProblemQuery {
            skip: 0,
            limit: 10,
            status: Some(ProblemStatus::Solved),
            search: Some("  two sum ".into()),
        };
        let filters = q.filters_json();
        assert_eq!(filters["status"], "AC");
        assert_eq!(filters["searchKeywords"], "two sum");
    }

    #[test]
    fn blank_search_is_omitted() {
        let q = ProblemQuery {
            search: Some("   ".into()),
            ..ProblemQuery::new(10)
        };
        assert!(q.filters_json().get("searchKeywords").is_none());
    }

    #[test]
    fn zero_limit_is_rejected_before_hitting_the_network() {
        let err = client().fetch_problems(&ProblemQuery::new(0)).unwrap_err();
        assert!(err.to_string().contains("limit"));
    }

    #[test]
    fn page_size_constant_matches_leetcodes_server_cap() {
        // LeetCode returns at most 100 rows regardless of the requested limit;
        // paging relies on this constant being the real cap.
        assert_eq!(MAX_PAGE_SIZE, 100);
    }

    #[test]
    fn empty_slug_is_rejected_before_hitting_the_network() {
        let err = client().fetch_question("  ").unwrap_err();
        assert!(err.to_string().contains("slug"));
    }

    #[test]
    fn truncate_keeps_short_text_and_elides_long_text() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("  hi  ", 10), "hi");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }

    /// A closed port: nothing listens there, so the request fails at the
    /// transport layer. This is the "no network" path, which the GUI shows in
    /// red, and it must produce a usable message rather than a panic or a hang.
    #[test]
    fn a_transport_failure_surfaces_a_usable_error() {
        let client =
            LeetCodeClient::with_base_url(Credentials::default(), "http://127.0.0.1:1").unwrap();

        let error = client
            .fetch_problems(&ProblemQuery::new(1))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("127.0.0.1:1"),
            "should name the endpoint: {error}"
        );
        assert!(
            error.to_lowercase().contains("network") || error.to_lowercase().contains("connect"),
            "should hint at connectivity: {error}"
        );

        // The other entry points must fail the same way rather than panicking.
        assert!(client.fetch_question("two-sum").is_err());
        assert!(client.fetch_languages().is_err());
        assert!(client.fetch_user_status().is_err());
        assert!(client.fetch_solved_slugs().is_err());
    }

    #[test]
    fn wire_problem_conversion_maps_status_and_tags() {
        let wire: g::WireProblem = serde_json::from_str(
            r#"{"frontendQuestionId":"1","title":"Two Sum","titleSlug":"two-sum",
                "difficulty":"Easy","status":"ac","isPaidOnly":false,"acRate":55.5,
                "topicTags":[{"name":"Array","slug":"array"}]}"#,
        )
        .unwrap();
        let p = problem_from_wire(wire);
        assert_eq!(p.status, ProblemStatus::Solved);
        assert_eq!(p.difficulty, Difficulty::Easy);
        assert_eq!(p.tags[0].name, "Array");
        assert_eq!(p.ac_rate, Some(55.5));
    }
}
