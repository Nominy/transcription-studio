//! Local ("This PC") inference for Studio: Gold's own runtime hosted in a hidden WebView2.
//!
//! [`LocalBackend`] implements the shared `InferenceBackend` by driving Gold's host inference
//! page (`chrome-extension://<id>/inference.html`, host API v1) loaded from Gold's unpacked
//! host build, which [`bundle`] downloads on first use from the release pinned in
//! `gold-inference.json`.

mod backend;
pub mod bundle;
pub mod number_markup;
pub mod polish;
pub mod prefs;
pub mod protocol;
pub mod remote;
pub mod rpc;

#[cfg(windows)]
pub mod host;

pub use backend::{LocalBackend, LocalConfig};

/// Virtual host the audio staging folder is served at (allowed by Gold's host manifest).
pub const AUDIO_HOST: &str = "studio-audio.example";
