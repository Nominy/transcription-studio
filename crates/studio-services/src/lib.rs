//! Generic UI-free native services. Constructing these never activates an extension.
//! Async APIs run on the shared tokio runtime in [`runtime`].

pub mod http;
pub mod paths;
pub mod prefs;
pub mod recovery;
pub mod runtime;
pub mod updater;
