//! On-disk solution workspaces.
//!
//! Every problem gets a directory under the workspace root containing:
//!
//! * `solution.<ext>` — the file both Leetr and the user's editor edit,
//! * `problem.md`     — the statement rendered to text for offline reading,
//! * `meta.json`      — ids/title/language so the directory is self-describing.
//!
//! Because the file is shared, editing in nvim and editing in Leetr are the
//! same operation: whoever writes last wins, and the other side notices the
//! mtime change and reloads.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::leetcode::html;
use crate::model::QuestionDetail;

/// File extension for a LeetCode language slug.
///
/// Covers every language LeetCode currently offers (see the live test that
/// asserts none of them fall back to `txt`).
pub fn extension_for(lang_slug: &str) -> &'static str {
    match lang_slug {
        "cpp" => "cpp",
        "java" => "java",
        "python" | "python3" => "py",
        "javascript" => "js",
        "typescript" => "ts",
        "csharp" => "cs",
        "c" => "c",
        "golang" => "go",
        "kotlin" => "kt",
        "swift" => "swift",
        "rust" => "rs",
        "ruby" => "rb",
        "php" => "php",
        "dart" => "dart",
        "scala" => "scala",
        "elixir" => "ex",
        "erlang" => "erl",
        "racket" => "rkt",
        "bash" => "sh",
        "mysql" | "mssql" | "postgresql" | "oraclesql" => "sql",
        "pythondata" => "py",
        _ => "txt",
    }
}

/// Directory that holds one problem's files.
pub fn problem_dir(root: &Path, frontend_id: &str, slug: &str) -> PathBuf {
    root.join(format!("{frontend_id}-{slug}"))
}

/// Path of the solution file for a given language.
pub fn solution_path(root: &Path, frontend_id: &str, slug: &str, lang_slug: &str) -> PathBuf {
    problem_dir(root, frontend_id, slug).join(format!("solution.{}", extension_for(lang_slug)))
}

/// Input shown by "run code" for this problem.
pub fn sample_input(detail: &QuestionDetail) -> &str {
    &detail.sample_test_case
}

/// The statement as Markdown, for reading in the editor.
pub fn statement_markdown(detail: &QuestionDetail) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {} {}\n\n", detail.frontend_id, detail.title));
    out.push_str(&format!("- Difficulty: {}\n", detail.difficulty.label()));
    out.push_str(&format!(
        "- Link: https://leetcode.com/problems/{}/\n",
        detail.slug
    ));
    if detail.paid_only {
        out.push_str("- Premium only\n");
    }
    if !detail.tags.is_empty() {
        let tags = detail
            .tags
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("- Tags: {tags}\n"));
    }
    out.push_str("\n---\n\n");
    out.push_str(&html::to_plain_text(&html::parse(&detail.content_html)));
    out.push('\n');

    if !detail.hints.is_empty() {
        out.push_str("\n## Hints\n\n");
        for (index, hint) in detail.hints.iter().enumerate() {
            out.push_str(&format!(
                "{}. {}\n",
                index + 1,
                html::to_plain_text(&html::parse(hint))
            ));
        }
    }
    out
}

/// Metadata written next to the solution so the directory is self-describing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemMeta {
    pub question_id: String,
    pub frontend_id: String,
    pub title: String,
    pub slug: String,
    pub difficulty: String,
    pub language: String,
}

impl ProblemMeta {
    pub fn from_detail(detail: &QuestionDetail, lang_slug: &str) -> Self {
        Self {
            question_id: detail.question_id.clone(),
            frontend_id: detail.frontend_id.clone(),
            title: detail.title.clone(),
            slug: detail.slug.clone(),
            difficulty: detail.difficulty.label().to_string(),
            language: lang_slug.to_string(),
        }
    }
}

/// What [`Workspace::prepare`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub dir: PathBuf,
    pub solution: PathBuf,
    /// True when the solution file was created from the boilerplate.
    pub created_solution: bool,
}

/// The workspace root, plus helpers to lay files out inside it.
#[derive(Debug, Clone)]
pub struct Workspace {
    root: PathBuf,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn from_config(config: &crate::config::Config) -> Self {
        Self::new(config.workspace_root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn dir_for(&self, frontend_id: &str, slug: &str) -> PathBuf {
        problem_dir(&self.root, frontend_id, slug)
    }

    /// Create the problem directory and the statement/metadata files.
    ///
    /// The solution file is only written when it does not already exist, so
    /// re-opening a problem never clobbers the user's work.
    pub fn prepare(&self, detail: &QuestionDetail, lang_slug: &str) -> Result<Prepared> {
        let dir = self.dir_for(&detail.frontend_id, &detail.slug);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

        let statement = dir.join("problem.md");
        write_if_changed(&statement, &statement_markdown(detail))?;

        let meta = ProblemMeta::from_detail(detail, lang_slug);
        let meta_text = serde_json::to_string_pretty(&meta)?;
        write_if_changed(&dir.join("meta.json"), &format!("{meta_text}\n"))?;

        let solution = solution_path(&self.root, &detail.frontend_id, &detail.slug, lang_slug);
        let created_solution = !solution.exists();
        if created_solution {
            // Fall back to an empty file rather than failing: a missing snippet
            // (e.g. premium problems) should not block editing.
            let boilerplate = detail.snippet_for(lang_slug).unwrap_or_default();
            std::fs::write(&solution, boilerplate)
                .with_context(|| format!("writing {}", solution.display()))?;
        }

        Ok(Prepared {
            dir,
            solution,
            created_solution,
        })
    }
}

/// Write `contents` only when it differs, to avoid pointless mtime churn.
fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == contents {
            return Ok(());
        }
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// The text currently being edited for one problem + language.
#[derive(Debug, Clone)]
pub struct Buffer {
    pub slug: String,
    pub frontend_id: String,
    pub lang_slug: String,
    pub path: PathBuf,
    pub text: String,
    /// mtime recorded when we last read or wrote the file.
    last_mtime: Option<SystemTime>,
}

impl Buffer {
    /// Read the solution file, falling back to `fallback` when unreadable.
    pub fn load(
        path: PathBuf,
        slug: impl Into<String>,
        frontend_id: impl Into<String>,
        lang_slug: impl Into<String>,
        fallback: &str,
    ) -> Self {
        let (text, last_mtime) = match std::fs::read_to_string(&path) {
            Ok(text) => (text, mtime(&path)),
            Err(_) => (fallback.to_string(), None),
        };

        Self {
            slug: slug.into(),
            frontend_id: frontend_id.into(),
            lang_slug: lang_slug.into(),
            path,
            text,
            last_mtime,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True when `self.text` differs from what is on disk.
    pub fn is_dirty(&self) -> bool {
        match std::fs::read_to_string(&self.path) {
            Ok(on_disk) => on_disk != self.text,
            Err(_) => !self.text.is_empty(),
        }
    }

    pub fn save(&mut self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&self.path, &self.text)
            .with_context(|| format!("writing {}", self.path.display()))?;
        self.last_mtime = mtime(&self.path);
        Ok(())
    }

    /// Reload from disk when the file changed underneath us (e.g. `:w` in nvim).
    ///
    /// Returns `true` only when the *contents* actually differ: an mtime bump
    /// with identical text (which can happen when the app itself just wrote the
    /// file) must not be reported as an external edit.
    pub fn reload_if_changed(&mut self) -> Result<bool> {
        let current = mtime(&self.path);
        if current.is_none() || current == self.last_mtime {
            return Ok(false);
        }

        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        self.last_mtime = current;

        if text == self.text {
            return Ok(false);
        }

        self.text = text;
        Ok(true)
    }

    /// Adopt `text` (e.g. pushed from the editor over the control API).
    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CodeSnippet, Difficulty, Tag};

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("leetr-editor-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn detail() -> QuestionDetail {
        QuestionDetail {
            question_id: "1".into(),
            frontend_id: "1".into(),
            title: "Two Sum".into(),
            slug: "two-sum".into(),
            difficulty: Difficulty::Easy,
            paid_only: false,
            content_html: "<p>Given <code>nums</code>.</p><ul><li>a</li></ul>".into(),
            sample_test_case: "[2,7,11,15]\n9".into(),
            example_testcases: String::new(),
            hints: vec!["use a hash map".into()],
            tags: vec![Tag {
                name: "Array".into(),
                slug: "array".into(),
            }],
            snippets: vec![
                CodeSnippet {
                    lang: "Python3".into(),
                    lang_slug: "python3".into(),
                    code: "class Solution:\n    pass\n".into(),
                },
                CodeSnippet {
                    lang: "Rust".into(),
                    lang_slug: "rust".into(),
                    code: "impl Solution {}\n".into(),
                },
            ],
        }
    }

    #[test]
    fn every_current_leetcode_language_has_a_real_extension() {
        // The full list from the live `languageList` query.
        let slugs = [
            "cpp",
            "java",
            "python3",
            "python",
            "javascript",
            "typescript",
            "csharp",
            "c",
            "golang",
            "kotlin",
            "swift",
            "rust",
            "ruby",
            "php",
            "dart",
            "scala",
            "elixir",
            "erlang",
            "racket",
            "bash",
            "mysql",
            "mssql",
            "postgresql",
            "oraclesql",
            "pythondata",
        ];
        for slug in slugs {
            assert_ne!(
                extension_for(slug),
                "txt",
                "language {slug:?} must map to a real file extension"
            );
        }
        assert_eq!(extension_for("python3"), "py");
        assert_eq!(extension_for("rust"), "rs");
        assert_eq!(extension_for("mysql"), "sql");
    }

    #[test]
    fn unknown_language_falls_back_to_txt() {
        assert_eq!(extension_for("brainfuck"), "txt");
        assert_eq!(extension_for(""), "txt");
    }

    #[test]
    fn solution_paths_live_under_the_problem_directory() {
        let root = Path::new("/ws");
        assert_eq!(
            solution_path(root, "1", "two-sum", "rust"),
            PathBuf::from("/ws/1-two-sum/solution.rs")
        );
        assert_eq!(
            problem_dir(root, "100001", "some-problem"),
            PathBuf::from("/ws/100001-some-problem")
        );
    }

    #[test]
    fn prepare_creates_statement_meta_and_boilerplate() {
        let root = tmpdir("prepare");
        let ws = Workspace::new(root.clone());
        let prepared = ws.prepare(&detail(), "python3").unwrap();

        assert!(prepared.created_solution);
        assert!(prepared.solution.ends_with("1-two-sum/solution.py"));
        assert_eq!(
            std::fs::read_to_string(&prepared.solution).unwrap(),
            "class Solution:\n    pass\n"
        );

        let statement =
            std::fs::read_to_string(ws.dir_for("1", "two-sum").join("problem.md")).unwrap();
        assert!(statement.contains("# 1 Two Sum"));
        assert!(statement.contains("Difficulty: Easy"));
        assert!(statement.contains("https://leetcode.com/problems/two-sum/"));
        assert!(statement.contains("Given nums."));
        assert!(statement.contains("## Hints"));

        let meta: ProblemMeta = serde_json::from_str(
            &std::fs::read_to_string(ws.dir_for("1", "two-sum").join("meta.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta.question_id, "1");
        assert_eq!(meta.language, "python3");
    }

    #[test]
    fn prepare_never_clobbers_an_existing_solution() {
        let root = tmpdir("noclobber");
        let ws = Workspace::new(root);
        let first = ws.prepare(&detail(), "python3").unwrap();
        std::fs::write(&first.solution, "my hard work").unwrap();

        let second = ws.prepare(&detail(), "python3").unwrap();
        assert!(!second.created_solution);
        assert_eq!(
            std::fs::read_to_string(&second.solution).unwrap(),
            "my hard work"
        );
    }

    #[test]
    fn prepare_works_without_a_boilerplate_snippet() {
        let root = tmpdir("nosnippet");
        let ws = Workspace::new(root);
        // "cpp" has no snippet in the fixture.
        let prepared = ws.prepare(&detail(), "cpp").unwrap();
        assert!(prepared.created_solution);
        assert_eq!(std::fs::read_to_string(&prepared.solution).unwrap(), "");
    }

    #[test]
    fn buffer_reloads_when_the_file_changes_on_disk() {
        let root = tmpdir("reload");
        let ws = Workspace::new(root.clone());
        let prepared = ws.prepare(&detail(), "rust").unwrap();

        let mut buffer = Buffer::load(
            prepared.solution.clone(),
            "two-sum",
            "1",
            "rust",
            "fallback",
        );
        assert_eq!(buffer.text, "impl Solution {}\n");
        assert!(!buffer.reload_if_changed().unwrap(), "nothing changed yet");

        // Simulate the user saving in nvim.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&prepared.solution, "// edited in nvim\n").unwrap();

        assert!(buffer.reload_if_changed().unwrap());
        assert_eq!(buffer.text, "// edited in nvim\n");
        assert!(!buffer.reload_if_changed().unwrap(), "no further change");
    }

    #[test]
    fn a_touched_but_identical_file_is_not_a_reload() {
        let root = tmpdir("touch");
        let ws = Workspace::new(root);
        let prepared = ws.prepare(&detail(), "rust").unwrap();
        let mut buffer = Buffer::load(prepared.solution.clone(), "two-sum", "1", "rust", "");

        // Same bytes, newer mtime.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let same = std::fs::read_to_string(&prepared.solution).unwrap();
        std::fs::write(&prepared.solution, same).unwrap();

        assert!(
            !buffer.reload_if_changed().unwrap(),
            "an mtime bump with identical contents must not report an external edit"
        );

        // Now genuinely different.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&prepared.solution, "// actually different\n").unwrap();
        assert!(buffer.reload_if_changed().unwrap());
    }

    #[test]
    fn buffer_save_writes_and_clears_dirty() {
        let root = tmpdir("save");
        let ws = Workspace::new(root);
        let prepared = ws.prepare(&detail(), "rust").unwrap();
        let mut buffer = Buffer::load(prepared.solution.clone(), "two-sum", "1", "rust", "");

        buffer.text = "fn main() {}\n".into();
        assert!(buffer.is_dirty());
        buffer.save().unwrap();
        assert!(!buffer.is_dirty());
        assert_eq!(
            std::fs::read_to_string(&prepared.solution).unwrap(),
            "fn main() {}\n"
        );
    }

    #[test]
    fn buffer_falls_back_when_the_file_is_missing() {
        let buffer = Buffer::load(
            PathBuf::from("/nonexistent/solution.rs"),
            "slug",
            "1",
            "rust",
            "// boilerplate",
        );
        assert_eq!(buffer.text, "// boilerplate");
    }

    #[test]
    fn sample_input_is_the_api_sample() {
        assert_eq!(sample_input(&detail()), "[2,7,11,15]\n9");
    }
}
