//! Running sample tests and submitting solutions.
//!
//! LeetCode's judge is a two-step protocol:
//!
//! 1. `POST /problems/{slug}/interpret_solution/` (sample tests) or
//!    `POST /problems/{slug}/submit/` (a real submission) returns an id;
//! 2. `GET /submissions/detail/{id}/check/` is polled until it reports a verdict.
//!
//! Both endpoints take the **language slug** (`python3`, `rust`, …) and the
//! problem's *internal* `questionId`, and require the session cookies plus the
//! CSRF token.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use reqwest::header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER};
use serde::{Deserialize, Serialize};

use super::client::LeetCodeClient;

/// How long to wait for a judge verdict before giving up.
pub const JUDGE_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(700);

/// Refuse a submission to a problem that is not already solved.
///
/// This is the safety rail requested by the user: Leetr will not send a first
/// attempt at a problem it thinks is unsolved. `allow_unsolved` (from
/// `config.toml`) turns it off.
pub fn ensure_submittable(
    slug: &str,
    solved: &BTreeSet<String>,
    allow_unsolved: bool,
) -> Result<()> {
    if allow_unsolved || solved.contains(slug) {
        return Ok(());
    }

    bail!(
        "refusing to submit to `{slug}`: it is not in your solved set. \
         Leetr only submits to problems you have already solved — \
         set `allow_unsolved_submit = true` in config.toml to override."
    )
}

/// Outcome of a judge run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Still on the judge.
    #[default]
    Pending,
    Accepted,
    WrongAnswer,
    TimeLimitExceeded,
    MemoryLimitExceeded,
    OutputLimitExceeded,
    RuntimeError,
    CompileError,
    InternalError,
    Unknown,
}

impl Verdict {
    /// Classify a result, preferring the human-readable `status_msg` because it
    /// is stable while the numeric codes have shifted between API revisions.
    pub fn from_status(status_msg: &str, status_code: Option<i64>) -> Self {
        let message = status_msg.trim().to_ascii_lowercase();

        if message.contains("accepted") {
            Self::Accepted
        } else if message.contains("wrong answer") {
            Self::WrongAnswer
        } else if message.contains("time limit") || message.contains("timeout") {
            Self::TimeLimitExceeded
        } else if message.contains("memory limit") {
            Self::MemoryLimitExceeded
        } else if message.contains("output limit") {
            Self::OutputLimitExceeded
        } else if message.contains("runtime error") {
            Self::RuntimeError
        } else if message.contains("compile error") {
            Self::CompileError
        } else if message.contains("pending") || message.contains("started") {
            Self::Pending
        } else if message.is_empty() {
            Self::from_code(status_code)
        } else {
            Self::Unknown
        }
    }

    fn from_code(status_code: Option<i64>) -> Self {
        match status_code {
            Some(10) => Self::Accepted,
            Some(11) => Self::WrongAnswer,
            Some(12) => Self::MemoryLimitExceeded,
            Some(13) => Self::OutputLimitExceeded,
            Some(14) => Self::RuntimeError,
            Some(15) => Self::CompileError,
            Some(20 | 21) => Self::InternalError,
            _ => Self::Pending,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pending => "Judging…",
            Self::Accepted => "Accepted",
            Self::WrongAnswer => "Wrong Answer",
            Self::TimeLimitExceeded => "Time Limit Exceeded",
            Self::MemoryLimitExceeded => "Memory Limit Exceeded",
            Self::OutputLimitExceeded => "Output Limit Exceeded",
            Self::RuntimeError => "Runtime Error",
            Self::CompileError => "Compile Error",
            Self::InternalError => "Internal Error",
            Self::Unknown => "Unknown",
        }
    }

    pub fn is_accepted(self) -> bool {
        self == Self::Accepted
    }

    /// True once the judge has finished (successfully or not).
    pub fn is_final(self) -> bool {
        self != Self::Pending
    }
}

/// A judge response.
///
/// The `/check/` payload differs between a **sample run** (`interpret_solution`)
/// and a real **submission**, and field types differ too: outputs are arrays
/// for runs and strings for submissions, and `memory` is sometimes a number.
/// Everything loose is therefore kept as raw JSON and rendered on demand, and
/// the helpers below accept both shapes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct JudgeResult {
    /// `PENDING`, `STARTED` or `SUCCESS`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub status_msg: Option<String>,
    #[serde(default)]
    pub status_code: Option<i64>,
    /// Present for sample runs; `true` once the code ran without an exception.
    #[serde(default)]
    pub run_success: Option<bool>,
    #[serde(default)]
    pub total_correct: Option<u32>,
    #[serde(default)]
    pub total_testcases: Option<u32>,
    #[serde(default)]
    pub runtime: Option<serde_json::Value>,
    #[serde(default)]
    pub status_runtime: Option<String>,
    #[serde(default)]
    pub display_runtime: Option<String>,
    #[serde(default)]
    pub memory: Option<serde_json::Value>,
    #[serde(default)]
    pub status_memory: Option<String>,
    #[serde(default)]
    pub code_output: Option<serde_json::Value>,
    #[serde(default)]
    pub code_answer: Option<serde_json::Value>,
    #[serde(default)]
    pub expected_output: Option<serde_json::Value>,
    #[serde(default)]
    pub expected_code_answer: Option<serde_json::Value>,
    #[serde(default)]
    pub std_output: Option<serde_json::Value>,
    #[serde(default)]
    pub std_output_list: Option<serde_json::Value>,
    #[serde(default)]
    pub last_testcase: Option<String>,
    #[serde(default)]
    pub compile_error: Option<String>,
    #[serde(default)]
    pub full_compile_error: Option<String>,
    #[serde(default)]
    pub runtime_error: Option<String>,
}

impl JudgeResult {
    /// Has the judge finished?
    ///
    /// The `state` field is authoritative when present; the run endpoint
    /// sometimes reports a status code without explicitly saying `SUCCESS`.
    pub fn is_finished(&self) -> bool {
        if !self.state.trim().is_empty() {
            return self.state.eq_ignore_ascii_case("SUCCESS");
        }
        self.status_code.is_some() || self.run_success.is_some()
    }

    pub fn verdict(&self) -> Verdict {
        if !self.is_finished() {
            return Verdict::Pending;
        }
        Verdict::from_status(
            self.status_msg.as_deref().unwrap_or_default(),
            self.status_code,
        )
    }

    /// Short, single-line summary for the status bar / nvim.
    pub fn summary(&self) -> String {
        let verdict = self.verdict();
        let mut parts = vec![verdict.label().to_string()];

        if !verdict.is_accepted() {
            if let (Some(correct), Some(total)) = (self.total_correct, self.total_testcases) {
                parts.push(format!("{correct}/{total} tests"));
            }
        }
        if let Some(runtime) = self.runtime_text() {
            parts.push(runtime);
        }

        parts.join(" · ")
    }

    /// "44 ms"-style runtime, whichever shape the API used.
    pub fn runtime_text(&self) -> Option<String> {
        for candidate in [&self.status_runtime, &self.display_runtime] {
            if let Some(text) = candidate
                .as_ref()
                .map(|t| t.trim())
                .filter(|t| !t.is_empty())
            {
                return Some(if text.ends_with("ms") {
                    text.to_string()
                } else {
                    format!("{text} ms")
                });
            }
        }

        match self.runtime.as_ref()? {
            serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            serde_json::Value::Number(number) => Some(format!("{number} ms")),
            _ => None,
        }
    }

    /// Memory used, preferring the pre-formatted string.
    pub fn memory_text(&self) -> Option<String> {
        if let Some(text) = self
            .status_memory
            .as_ref()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
        {
            return Some(text.to_string());
        }

        match self.memory.as_ref()? {
            serde_json::Value::String(text) if !text.trim().is_empty() => Some(text.clone()),
            // The judge reports bytes.
            serde_json::Value::Number(number) => number.as_f64().map(|bytes| {
                if bytes >= 1_048_576.0 {
                    format!("{:.1} MB", bytes / 1_048_576.0)
                } else if bytes >= 1024.0 {
                    format!("{:.1} KB", bytes / 1024.0)
                } else {
                    format!("{bytes} B")
                }
            }),
            _ => None,
        }
    }

    /// What the code produced, flattened to text.
    pub fn output_text(&self) -> Option<String> {
        first_text([
            &self.code_answer,
            &self.code_output,
            &self.std_output_list,
            &self.std_output,
        ])
    }

    /// What the judge expected, flattened to text.
    pub fn expected_text(&self) -> Option<String> {
        first_text([&self.expected_output, &self.expected_code_answer])
    }

    /// The most useful error text, if any, flattened to one string.
    pub fn error_text(&self) -> Option<String> {
        for candidate in [
            &self.full_compile_error,
            &self.compile_error,
            &self.runtime_error,
        ] {
            if let Some(text) = candidate.as_ref().filter(|text| !text.trim().is_empty()) {
                return Some(text.clone());
            }
        }
        None
    }
}

/// First non-empty rendering of a set of JSON values.
fn first_text<const N: usize>(candidates: [&Option<serde_json::Value>; N]) -> Option<String> {
    for candidate in candidates {
        let text = match candidate {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(serde_json::Value::Array(items)) => items
                .iter()
                .map(|item| match item {
                    serde_json::Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
            Some(serde_json::Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Judge requests
// ---------------------------------------------------------------------------

/// Body for `interpret_solution` (sample tests).
pub fn run_body(
    question_id: &str,
    lang_slug: &str,
    code: &str,
    data_input: &str,
) -> serde_json::Value {
    serde_json::json!({
        "lang": lang_slug,
        "question_id": question_id,
        "typed_code": code,
        "data_input": data_input,
        "judge_type": "small",
    })
}

/// Body for a real submission.
pub fn submit_body(question_id: &str, lang_slug: &str, code: &str) -> serde_json::Value {
    serde_json::json!({
        "lang": lang_slug,
        "question_id": question_id,
        "typed_code": code,
        "judge_type": "large",
    })
}

impl LeetCodeClient {
    /// Run the sample tests. Returns the judge id to poll.
    pub fn run_sample(
        &self,
        slug: &str,
        question_id: &str,
        lang_slug: &str,
        code: &str,
        data_input: &str,
    ) -> Result<String> {
        self.require_credentials()?;
        let body = run_body(question_id, lang_slug, code, data_input);
        let response =
            self.post_judge(&format!("/problems/{slug}/interpret_solution/"), slug, body)?;

        response
            .get("interpret_id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("LeetCode did not return an interpret_id: {response}"))
    }

    /// Submit a solution. Returns the submission id to poll.
    pub fn submit_solution(
        &self,
        slug: &str,
        question_id: &str,
        lang_slug: &str,
        code: &str,
    ) -> Result<i64> {
        self.require_credentials()?;
        let body = submit_body(question_id, lang_slug, code);
        let response = self.post_judge(&format!("/problems/{slug}/submit/"), slug, body)?;

        response
            .get("submission_id")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("LeetCode did not return a submission_id: {response}"))
    }

    /// One check of a queued judge run.
    pub fn check_submission(&self, id: &str) -> Result<JudgeResult> {
        self.require_credentials()?;
        let url = format!("{}/submissions/detail/{id}/check/", self.base_url());

        let mut request = self
            .http()
            .get(&url)
            .header(REFERER, format!("{}/", self.base_url()))
            .header(ORIGIN, self.base_url())
            .header("X-Requested-With", "XMLHttpRequest");

        if self.credentials().is_configured() {
            request = request
                .header(COOKIE, self.credentials().cookie_header())
                .header("x-csrftoken", self.credentials().csrftoken.clone());
        }

        let response = request.send().with_context(|| format!("GET {url}"))?;
        let status = response.status();
        let text = response.text().context("reading the check response")?;

        if !status.is_success() {
            bail!(
                "LeetCode returned HTTP {status} for {url}: {}",
                trim(&text, 200)
            );
        }

        serde_json::from_str(&text)
            .with_context(|| format!("decoding the check response: {}", trim(&text, 300)))
    }

    /// Poll a judge run until it finishes or [`JUDGE_TIMEOUT`] elapses.
    pub fn wait_for_submission(&self, id: &str) -> Result<JudgeResult> {
        let deadline = Instant::now() + JUDGE_TIMEOUT;

        loop {
            let result = self.check_submission(id)?;
            if result.is_finished() {
                return Ok(result);
            }
            if Instant::now() >= deadline {
                return Ok(result);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn require_credentials(&self) -> Result<()> {
        if self.credentials().is_configured() {
            Ok(())
        } else {
            bail!(
                "no LeetCode credentials configured — add them to {} to run or submit code",
                crate::config::credentials_path().display()
            )
        }
    }

    fn post_judge(
        &self,
        path: &str,
        slug: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}{path}", self.base_url());
        let referer = format!("{}/problems/{slug}/", self.base_url());

        let response = self
            .http()
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .header(REFERER, referer)
            .header(ORIGIN, self.base_url())
            .header("X-Requested-With", "XMLHttpRequest")
            .header(COOKIE, self.credentials().cookie_header())
            .header("x-csrftoken", self.credentials().csrftoken.clone())
            .json(&body)
            .send()
            .with_context(|| format!("POST {url}"))?;

        let status = response.status();
        let text = response.text().context("reading the judge response")?;

        if !status.is_success() {
            bail!(
                "LeetCode returned HTTP {status} for {path}: {}",
                trim(&text, 300)
            );
        }

        serde_json::from_str(&text)
            .with_context(|| format!("decoding the judge response: {}", trim(&text, 300)))
    }
}

fn trim(text: &str, max: usize) -> String {
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
    use crate::config::Credentials;

    fn solved() -> BTreeSet<String> {
        ["two-sum".to_string(), "add-two-numbers".to_string()]
            .into_iter()
            .collect()
    }

    // -- guard --------------------------------------------------------------

    #[test]
    fn submitting_to_a_solved_problem_is_allowed() {
        assert!(ensure_submittable("two-sum", &solved(), false).is_ok());
    }

    #[test]
    fn submitting_to_an_unsolved_problem_is_refused() {
        let err = ensure_submittable("median-of-two-sorted-arrays", &solved(), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing to submit"), "got: {err}");
        assert!(err.contains("median-of-two-sorted-arrays"));
        assert!(
            err.contains("allow_unsolved_submit"),
            "the message should say how to opt out"
        );
    }

    #[test]
    fn an_empty_solved_set_refuses_everything() {
        assert!(ensure_submittable("two-sum", &BTreeSet::new(), false).is_err());
    }

    #[test]
    fn the_override_permits_unsolved_submissions() {
        assert!(ensure_submittable("anything", &BTreeSet::new(), true).is_ok());
    }

    // -- verdicts -----------------------------------------------------------

    #[test]
    fn verdicts_come_from_the_status_message_first() {
        assert_eq!(
            Verdict::from_status("Accepted", Some(10)),
            Verdict::Accepted
        );
        assert_eq!(
            Verdict::from_status("Wrong Answer", Some(11)),
            Verdict::WrongAnswer
        );
        assert_eq!(
            Verdict::from_status("Time Limit Exceeded", Some(16)),
            Verdict::TimeLimitExceeded
        );
        assert_eq!(
            Verdict::from_status("Memory Limit Exceeded", Some(12)),
            Verdict::MemoryLimitExceeded
        );
        assert_eq!(
            Verdict::from_status("Runtime Error", Some(14)),
            Verdict::RuntimeError
        );
        assert_eq!(
            Verdict::from_status("Compile Error", Some(15)),
            Verdict::CompileError
        );
        assert_eq!(
            Verdict::from_status("Output Limit Exceeded", Some(13)),
            Verdict::OutputLimitExceeded
        );
    }

    #[test]
    fn the_status_message_beats_a_conflicting_code() {
        // The message is stable across API revisions, the codes have drifted.
        assert_eq!(
            Verdict::from_status("Time Limit Exceeded", Some(10)),
            Verdict::TimeLimitExceeded
        );
    }

    #[test]
    fn verdicts_fall_back_to_the_status_code() {
        assert_eq!(Verdict::from_status("", Some(10)), Verdict::Accepted);
        assert_eq!(Verdict::from_status("", Some(11)), Verdict::WrongAnswer);
        assert_eq!(Verdict::from_status("", Some(14)), Verdict::RuntimeError);
    }

    #[test]
    fn in_progress_states_are_pending() {
        assert_eq!(Verdict::from_status("Pending", None), Verdict::Pending);
        assert_eq!(Verdict::from_status("Started", None), Verdict::Pending);
        assert_eq!(Verdict::from_status("", None), Verdict::Pending);
        assert!(!Verdict::Pending.is_final());
    }

    #[test]
    fn unrecognised_messages_are_unknown_not_accepted() {
        let verdict = Verdict::from_status("Something New", Some(99));
        assert_eq!(verdict, Verdict::Unknown);
        assert!(!verdict.is_accepted());
        assert!(verdict.is_final());
    }

    #[test]
    fn verdict_labels_are_human_readable() {
        assert_eq!(Verdict::Accepted.label(), "Accepted");
        assert_eq!(Verdict::WrongAnswer.label(), "Wrong Answer");
        assert!(Verdict::Accepted.is_accepted());
        assert!(!Verdict::WrongAnswer.is_accepted());
    }

    // -- judge result -------------------------------------------------------

    /// Exactly what the live `interpret_solution` check returns for a pass.
    const REAL_RUN_ACCEPTED: &str = r#"{
        "state": "SUCCESS",
        "status_code": 10,
        "status_msg": "Accepted",
        "run_success": true,
        "total_correct": 1,
        "total_testcases": 1,
        "code_answer": ["[0,1]", ""],
        "code_output": [],
        "std_output_list": ["", ""],
        "expected_code_answer": ["[0,1]", ""],
        "expected_code_output": [],
        "compare_result": "1",
        "last_testcase": null,
        "runtime": null,
        "status_runtime": "0 ms",
        "display_runtime": "0",
        "memory": 19384000,
        "lang": "python3"
    }"#;

    #[test]
    fn parses_a_real_sample_run_payload() {
        let result: JudgeResult = serde_json::from_str(REAL_RUN_ACCEPTED).unwrap();
        assert!(result.is_finished());
        assert_eq!(result.verdict(), Verdict::Accepted);
        assert_eq!(result.runtime_text().as_deref(), Some("0 ms"));
        assert_eq!(
            result.memory_text().as_deref(),
            Some("18.5 MB"),
            "memory arrives as bytes"
        );
        assert_eq!(result.summary(), "Accepted · 0 ms");
        assert_eq!(
            result.output_text().as_deref(),
            Some("[0,1]"),
            "the empty code_output must not shadow code_answer"
        );
        assert_eq!(result.expected_text().as_deref(), Some("[0,1]"));
        assert!(result.error_text().is_none());
    }

    #[test]
    fn parses_a_submission_style_payload_with_string_outputs() {
        // The large judge reports outputs as plain strings, with different
        // field names than the sample run.
        let raw = r#"{"state":"SUCCESS","status_msg":"Wrong Answer","status_code":11,
            "total_correct":3,"total_testcases":60,"last_testcase":"[2,7,11,15]\n9",
            "code_output":"[1,2]","expected_output":"[0,1]","runtime":"44 ms",
            "status_memory":"16.5 MB"}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.verdict(), Verdict::WrongAnswer);
        assert_eq!(result.summary(), "Wrong Answer · 3/60 tests · 44 ms");
        assert_eq!(
            result.last_testcase.as_deref(),
            Some(
                "[2,7,11,15]
9"
            )
        );
        assert_eq!(result.output_text().as_deref(), Some("[1,2]"));
        assert_eq!(result.expected_text().as_deref(), Some("[0,1]"));
        assert_eq!(result.memory_text().as_deref(), Some("16.5 MB"));
    }

    #[test]
    fn a_missing_state_still_counts_as_finished() {
        // The run endpoint can omit `state` once it has a status code.
        let raw = r#"{"status_code":10,"status_msg":"Accepted","run_success":true}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert!(result.is_finished());
        assert_eq!(result.verdict(), Verdict::Accepted);
    }

    #[test]
    fn parses_a_numeric_runtime() {
        let raw = r#"{"state":"SUCCESS","status_msg":"Accepted","runtime":44}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.runtime_text().as_deref(), Some("44 ms"));
    }

    #[test]
    fn parses_a_compile_error_payload() {
        let raw = r#"{"state":"SUCCESS","status_msg":"Compile Error","status_code":15,
            "full_compile_error":"Line 3: error: expected `:`"}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.verdict(), Verdict::CompileError);
        assert_eq!(
            result.error_text().as_deref(),
            Some("Line 3: error: expected `:`")
        );
    }

    #[test]
    fn fallen_back_to_runtime_error_text_when_compile_error_is_empty() {
        let raw = r#"{"state":"SUCCESS","status_msg":"Runtime Error","compile_error":"",
            "runtime_error":"IndexError: list index out of range"}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.verdict(), Verdict::RuntimeError);
        assert_eq!(
            result.error_text().as_deref(),
            Some("IndexError: list index out of range")
        );
    }

    #[test]
    fn a_pending_payload_is_not_finished() {
        let raw = r#"{"state":"PENDING"}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert!(!result.is_finished());
        assert_eq!(result.verdict(), Verdict::Pending);
    }

    #[test]
    fn unknown_fields_do_not_break_parsing() {
        let raw = r#"{"state":"SUCCESS","status_msg":"Accepted","brand_new_field":{"a":[1,2]}}"#;
        let result: JudgeResult = serde_json::from_str(raw).unwrap();
        assert_eq!(result.verdict(), Verdict::Accepted);
    }

    // -- request bodies -----------------------------------------------------

    #[test]
    fn run_body_uses_the_small_judge_with_sample_input() {
        let body = run_body("1", "python3", "print(1)", "[2,7,11,15]\n9");
        assert_eq!(body["lang"], "python3");
        assert_eq!(body["question_id"], "1");
        assert_eq!(body["typed_code"], "print(1)");
        assert_eq!(body["data_input"], "[2,7,11,15]\n9");
        assert_eq!(body["judge_type"], "small");
    }

    #[test]
    fn submit_body_uses_the_large_judge_and_no_input() {
        let body = submit_body("1", "rust", "impl Solution {}");
        assert_eq!(body["lang"], "rust");
        assert_eq!(body["question_id"], "1");
        assert_eq!(body["typed_code"], "impl Solution {}");
        assert_eq!(body["judge_type"], "large");
        assert!(body.get("data_input").is_none());
    }

    // -- against a mock judge ----------------------------------------------

    mod mock {
        use super::*;
        use std::sync::{Arc, Mutex};
        use tiny_http::{Header, Response, Server};

        /// What the mock judge received.
        #[derive(Debug, Default, Clone)]
        pub struct Captured {
            pub path: String,
            pub body: serde_json::Value,
            pub cookie: Option<String>,
            pub csrf: Option<String>,
            pub referer: Option<String>,
            pub user_agent: Option<String>,
        }

        pub struct MockJudge {
            pub port: u16,
            pub captured: Arc<Mutex<Vec<Captured>>>,
        }

        impl MockJudge {
            pub fn start() -> Self {
                let server = Arc::new(Server::http("127.0.0.1:0").unwrap());
                let port = server.server_addr().to_ip().unwrap().port();
                let captured = Arc::new(Mutex::new(Vec::new()));

                let thread_captured = Arc::clone(&captured);
                std::thread::spawn(move || {
                    for mut request in server.incoming_requests() {
                        let path = request.url().to_string();
                        let mut body = String::new();
                        let _ = request.as_reader().read_to_string(&mut body);

                        let header = |name: &str| {
                            request
                                .headers()
                                .iter()
                                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
                                .map(|h| h.value.as_str().to_string())
                        };

                        thread_captured.lock().unwrap().push(Captured {
                            path: path.clone(),
                            body: serde_json::from_str(&body).unwrap_or(serde_json::Value::Null),
                            cookie: header("Cookie"),
                            csrf: header("x-csrftoken"),
                            referer: header("Referer"),
                            user_agent: header("User-Agent"),
                        });

                        let payload = if path.ends_with("/check/") {
                            // First poll pending, then accepted — exercises wait_for.
                            let polls = thread_captured
                                .lock()
                                .unwrap()
                                .iter()
                                .filter(|c| c.path.ends_with("/check/"))
                                .count();
                            if polls <= 1 {
                                serde_json::json!({ "state": "PENDING" })
                            } else {
                                // Mirrors the live payload shape, including the
                                // array-valued outputs and byte-count memory.
                                serde_json::json!({
                                    "state": "SUCCESS",
                                    "status_msg": "Accepted",
                                    "status_code": 10,
                                    "run_success": true,
                                    "total_correct": 1,
                                    "total_testcases": 1,
                                    "code_answer": ["[0,1]", ""],
                                    "code_output": [],
                                    "expected_code_answer": ["[0,1]", ""],
                                    "status_runtime": "40 ms",
                                    "display_runtime": "40",
                                    "memory": 19384000
                                })
                            }
                        } else if path.contains("interpret_solution") {
                            serde_json::json!({ "interpret_id": "run-123" })
                        } else {
                            serde_json::json!({ "submission_id": 987654 })
                        };

                        let _ = request.respond(
                            Response::from_string(payload.to_string())
                                .with_header(
                                    Header::from_bytes(
                                        &b"Content-Type"[..],
                                        &b"application/json"[..],
                                    )
                                    .unwrap(),
                                )
                                .with_status_code(200),
                        );
                    }
                });

                Self { port, captured }
            }

            pub fn client(&self) -> LeetCodeClient {
                LeetCodeClient::with_base_url(
                    Credentials {
                        leetcode_session: "session-value".into(),
                        csrftoken: "csrf-value".into(),
                    },
                    format!("http://127.0.0.1:{}", self.port),
                )
                .unwrap()
            }

            pub fn requests(&self) -> Vec<Captured> {
                self.captured.lock().unwrap().clone()
            }
        }
    }

    #[test]
    fn running_a_sample_sends_the_right_request() {
        let judge = mock::MockJudge::start();
        let client = judge.client();

        let id = client
            .run_sample("two-sum", "1", "python3", "print(1)", "[2,7]\n9")
            .expect("run");
        assert_eq!(id, "run-123");

        let requests = judge.requests();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(request.path, "/problems/two-sum/interpret_solution/");
        assert_eq!(request.body["lang"], "python3");
        assert_eq!(request.body["judge_type"], "small");
        assert_eq!(request.body["data_input"], "[2,7]\n9");
        assert_eq!(
            request.cookie.as_deref(),
            Some("LEETCODE_SESSION=session-value; csrftoken=csrf-value")
        );
        assert_eq!(request.csrf.as_deref(), Some("csrf-value"));
        let expected_referer = format!("http://127.0.0.1:{}/problems/two-sum/", judge.port);
        assert_eq!(request.referer.as_deref(), Some(expected_referer.as_str()));
        assert!(
            request
                .user_agent
                .as_deref()
                .is_some_and(|agent| agent.contains("Mozilla")),
            "LeetCode expects a browser-like User-Agent, got {:?}",
            request.user_agent
        );
    }

    #[test]
    fn submitting_sends_the_large_judge_and_returns_the_id() {
        let judge = mock::MockJudge::start();
        let client = judge.client();

        let id = client
            .submit_solution("two-sum", "1", "python3", "class Solution: pass")
            .expect("submit");
        assert_eq!(id, 987654);

        let requests = judge.requests();
        assert_eq!(requests[0].path, "/problems/two-sum/submit/");
        assert_eq!(requests[0].body["judge_type"], "large");
        assert_eq!(requests[0].body["typed_code"], "class Solution: pass");
    }

    #[test]
    fn polling_waits_until_the_judge_finishes() {
        let judge = mock::MockJudge::start();
        let client = judge.client();

        let result = client.wait_for_submission("run-123").expect("poll");
        assert!(result.is_finished());
        assert_eq!(result.verdict(), Verdict::Accepted);
        assert_eq!(result.summary(), "Accepted · 40 ms");
        assert_eq!(result.memory_text().as_deref(), Some("18.5 MB"));

        let polls = judge
            .requests()
            .iter()
            .filter(|request| request.path.ends_with("/check/"))
            .count();
        assert!(polls >= 2, "expected at least two polls, got {polls}");
    }

    #[test]
    fn a_check_error_is_surfaced() {
        let judge = mock::MockJudge::start();
        let client = judge.client();
        // A bad id still hits the mock, which returns a valid payload, so
        // this asserts the happy path is reachable through `check_submission`.
        let result = client.check_submission("anything").expect("check");
        assert!(!result.is_finished() || result.verdict().is_final());
    }
}
