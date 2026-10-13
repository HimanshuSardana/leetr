//! GraphQL query strings and wire types.
//!
//! Keeping the wire format in one place means a LeetCode schema change is a
//! local edit here plus the conversion in [`super::client`].

use serde::Deserialize;

pub const Q_USER_STATUS: &str = r#"
query globalData {
  userStatus {
    isSignedIn
    username
    isPremium
  }
}"#;

pub const Q_LANGUAGES: &str = r#"
query languageList {
  languageList {
    id
    name
    verboseName
  }
}"#;

/// Paginated problem list.
///
/// `filters` accepts the `QuestionListFilterInput` input object, e.g.
/// `{"status": "SOLVED"}` or `{"searchKeywords": "two"}`.
pub const Q_PROBLEM_LIST: &str = r#"
query problemsetQuestionList(
  $categorySlug: String
  $limit: Int
  $skip: Int
  $filters: QuestionListFilterInput
) {
  problemsetQuestionList: questionList(
    categorySlug: $categorySlug
    limit: $limit
    skip: $skip
    filters: $filters
  ) {
    total: totalNum
    questions: data {
      frontendQuestionId: questionFrontendId
      title
      titleSlug
      difficulty
      status
      isPaidOnly
      acRate
      topicTags {
        name
        slug
      }
    }
  }
}"#;

pub const Q_QUESTION: &str = r#"
query questionData($titleSlug: String!) {
  question(titleSlug: $titleSlug) {
    questionId
    questionFrontendId
    title
    titleSlug
    difficulty
    isPaidOnly
    content
    sampleTestCase
    exampleTestcases
    hints
    topicTags {
      name
      slug
    }
    codeSnippets {
      lang
      langSlug
      code
    }
  }
}"#;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// The standard GraphQL response envelope.
#[derive(Debug, Deserialize)]
#[serde(bound(deserialize = "T: Deserialize<'de>"))]
pub struct Envelope<T> {
    #[serde(default)]
    pub data: Option<T>,
    #[serde(default)]
    pub errors: Vec<GraphQlError>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQlError {
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserStatusData {
    pub user_status: WireUserStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireUserStatus {
    #[serde(default)]
    pub is_signed_in: bool,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub is_premium: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguagesData {
    pub language_list: Vec<WireLanguage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireLanguage {
    pub id: u32,
    pub name: String,
    pub verbose_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProblemListData {
    pub problemset_question_list: WireProblemList,
}

#[derive(Debug, Deserialize)]
pub struct WireProblemList {
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub questions: Vec<WireProblem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireProblem {
    pub frontend_question_id: String,
    pub title: String,
    pub title_slug: String,
    #[serde(default)]
    pub difficulty: String,
    /// `"ac"`, `"notac"` or `null` (only populated when signed in).
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_paid_only: bool,
    #[serde(default)]
    pub ac_rate: Option<f64>,
    #[serde(default)]
    pub topic_tags: Vec<WireTag>,
}

#[derive(Debug, Deserialize)]
pub struct WireTag {
    pub name: String,
    #[serde(default)]
    pub slug: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionData {
    pub question: WireQuestion,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireQuestion {
    pub question_id: String,
    pub question_frontend_id: String,
    pub title: String,
    pub title_slug: String,
    #[serde(default)]
    pub difficulty: String,
    #[serde(default)]
    pub is_paid_only: bool,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub sample_test_case: Option<String>,
    #[serde(default)]
    pub example_testcases: Option<String>,
    #[serde(default)]
    pub hints: Option<Vec<String>>,
    #[serde(default)]
    pub topic_tags: Vec<WireTag>,
    #[serde(default)]
    pub code_snippets: Vec<WireCodeSnippet>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCodeSnippet {
    #[serde(default)]
    pub lang: String,
    pub lang_slug: String,
    #[serde(default)]
    pub code: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_parses_data_and_errors() {
        let ok: Envelope<LanguagesData> = serde_json::from_str(
            r#"{"data":{"languageList":[{"id":11,"name":"python3","verboseName":"Python3"}]}}"#,
        )
        .unwrap();
        assert!(ok.errors.is_empty());
        assert_eq!(ok.data.unwrap().language_list[0].id, 11);

        let bad: Envelope<LanguagesData> =
            serde_json::from_str(r#"{"errors":[{"message":"boom"}]}"#).unwrap();
        assert!(bad.data.is_none());
        assert_eq!(bad.errors[0].message, "boom");
    }

    #[test]
    fn problem_list_wire_shape() {
        let raw = r#"{"data":{"problemsetQuestionList":{"total":4082,"questions":[{
            "frontendQuestionId":"1","title":"Two Sum","titleSlug":"two-sum",
            "difficulty":"Easy","status":null,"isPaidOnly":false,"acRate":55.1,
            "topicTags":[{"name":"Array","slug":"array"}]}]}}}"#;
        let parsed: Envelope<ProblemListData> = serde_json::from_str(raw).unwrap();
        let list = parsed.data.unwrap().problemset_question_list;
        assert_eq!(list.total, 4082);
        assert_eq!(list.questions[0].title_slug, "two-sum");
        assert_eq!(list.questions[0].topic_tags[0].slug, "array");
        assert!(list.questions[0].status.is_none());
    }

    #[test]
    fn question_wire_shape_tolerates_missing_optional_fields() {
        let raw = r#"{"data":{"question":{
            "questionId":"1","questionFrontendId":"1","title":"Two Sum",
            "titleSlug":"two-sum","difficulty":"Easy","isPaidOnly":false,
            "content":"<p>hi</p>","codeSnippets":[{"lang":"Rust","langSlug":"rust","code":"x"}]}}}"#;
        let parsed: Envelope<QuestionData> = serde_json::from_str(raw).unwrap();
        let q = parsed.data.unwrap().question;
        assert_eq!(q.question_id, "1");
        assert!(q.hints.is_none());
        assert!(q.sample_test_case.is_none());
        assert_eq!(q.code_snippets[0].lang_slug, "rust");
    }
}
