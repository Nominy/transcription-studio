//! UI-free services: Babel/review backends, auth, persistence, updates.
//! Async APIs run on the shared tokio runtime in [`runtime`].

pub mod audio_fetch;
pub mod auth;
pub mod babel;
pub mod http;
pub mod paths;
pub mod prefs;
pub mod recovery;
pub mod relay;
pub mod review;
pub mod runtime;
pub mod subaccounts;
pub mod updater;
