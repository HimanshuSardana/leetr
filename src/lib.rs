//! Leetr — a lightweight `egui` LeetCode client with first-class nvim
//! interoperability.
//!
//! The crate is split into a library and a thin binary so the API client,
//! statement renderer and configuration handling can be integration-tested
//! against the live LeetCode API (see `tests/live_api.rs`).

pub mod app;
pub mod bg;
pub mod config;
pub mod editor;
pub mod glyphs;
pub mod leetcode;
pub mod model;
pub mod nvim;
pub mod theme;
pub mod ui;
