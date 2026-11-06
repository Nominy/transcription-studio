//! Process-wide shared HTTP client.
//!
//! Building a `reqwest::Client` allocates a fresh rustls connector, root
//! certificate store, and connection pool. Doing that per request churned
//! memory and file handles until the process froze, so every default-config
//! request in this crate goes through the one shared client below.

use std::sync::LazyLock;

static SHARED_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// The shared `reqwest::Client` (an `Arc` internally; cloning is cheap and
/// shares the connection pool). Only for the default client configuration.
pub fn client() -> reqwest::Client {
    SHARED_CLIENT.clone()
}

/// Collapses whitespace and truncates a response body for error messages.
pub fn body_snippet(body: &str) -> String {
    let snippet: String = body
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect();
    if snippet.is_empty() {
        "<empty response body>".to_string()
    } else {
        snippet
    }
}
