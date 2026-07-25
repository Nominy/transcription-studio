//! Process-wide shared HTTP client.
//!
//! Building a `reqwest::Client` allocates a fresh rustls connector, root
//! certificate store, and connection pool. Doing that per request (live
//! Babel tRPC calls, review-backend autosaves, Clerk JWT mints) churned
//! memory and file handles until the process froze, so every plain
//! `reqwest::Client::new()` site in this module tree goes through the one
//! shared client below instead.

use std::sync::LazyLock;

static SHARED_HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

/// The one shared `reqwest::Client` for the process lifetime.
///
/// `reqwest::Client` is an `Arc` internally, so cloning the returned handle
/// is cheap and every clone shares the same connection pool and TLS state.
/// Only valid for the default client configuration; a site that needs custom
/// builder options must get its own dedicated static.
pub(super) fn shared_http_client() -> reqwest::Client {
    SHARED_HTTP_CLIENT.clone()
}

#[cfg(test)]
mod tests {
    use super::shared_http_client;

    #[test]
    fn shared_http_client_returns_one_reusable_handle() {
        let first = shared_http_client();
        let second = shared_http_client();
        // Both clones share the same underlying connection pool.
        drop(first);
        let _request = second.get("https://example.invalid");
    }
}
