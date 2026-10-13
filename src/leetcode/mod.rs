//! LeetCode API access: wire format, HTTP client and statement rendering.

pub mod client;
pub mod graphql;
pub mod html;
pub mod submit;

pub use client::{LeetCodeClient, ProblemQuery};
pub use submit::{ensure_submittable, JudgeResult, Verdict};
