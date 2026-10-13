//! LeetCode API access: wire format and HTTP client.

pub mod client;
pub mod graphql;

pub use client::{LeetCodeClient, ProblemQuery};
