//! Native Babel authentication: Clerk phone-code login, persisted cookie store,
//! session-JWT minting for Babel API calls, startup auth gate, and the
//! `--babel-auth-*` CLI.
//!
//! Env: `STUDIO_BABEL_AUTH_STORE` (store file override), `STUDIO_BABEL_CLERK_URL`
//! (default `https://clerk.babel.audio`), `STUDIO_BABEL_SERVICE_BASE_URL`
//! (default `https://dashboard.babel.audio`).

mod clerk;
pub mod cli;
pub mod flow;
mod store;

use std::env;

pub use clerk::{
    NeedsSignIn, PhoneCodeLoginComplete, PhoneCodeLoginStart, active_session_id,
    babel_backend_cookie_header, mint_babel_authorization, start_login, start_phone_code_login,
    verify_login, verify_phone_code_login,
};
pub use store::{
    AuthCookie, AuthStore, cookie_header_for_url, detach_store_to_executable, detach_store_to_path,
    read_store, reset_store, save_store, store_from_str, store_path, store_path_next_to_executable,
};

const DEFAULT_CLERK_URL: &str = "https://clerk.babel.audio";
const DEFAULT_BABEL_BASE_URL: &str = "https://dashboard.babel.audio";
pub(crate) const STUDIO_BABEL_AUTH_STORE: &str = "STUDIO_BABEL_AUTH_STORE";
const STUDIO_BABEL_CLERK_URL: &str = "STUDIO_BABEL_CLERK_URL";
pub(crate) const STUDIO_BABEL_SERVICE_BASE_URL: &str = "STUDIO_BABEL_SERVICE_BASE_URL";

/// Clerk frontend API base (`STUDIO_BABEL_CLERK_URL`, trailing `/` trimmed).
pub fn clerk_url() -> String {
    env_url(STUDIO_BABEL_CLERK_URL).unwrap_or_else(|| DEFAULT_CLERK_URL.to_string())
}

/// Babel dashboard base (`STUDIO_BABEL_SERVICE_BASE_URL`, trailing `/` trimmed).
pub fn babel_base_url() -> String {
    env_url(STUDIO_BABEL_SERVICE_BASE_URL).unwrap_or_else(|| DEFAULT_BABEL_BASE_URL.to_string())
}

fn env_url(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
}

fn now_unix_seconds() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}
