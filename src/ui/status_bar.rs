//! Bottom status bar: auth badge + transient status message.
//!
//! Rendering lives in the GPUI root view (`crate::app`); this module keeps the
//! badge model.

/// What to show for the LeetCode session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AuthBadge {
    /// Credentials are configured and LeetCode accepted them.
    SignedIn { username: String, solved: usize },
    /// Credentials are configured but LeetCode *refused* the session — almost
    /// always an expired cookie. Worth its own state: telling someone who has
    /// credentials that none were found sends them to fix the wrong thing.
    Rejected,
    /// Credentials are configured and the check is still in flight.
    Checking,
    /// No credentials at all.
    #[default]
    BrowseOnly,
}
