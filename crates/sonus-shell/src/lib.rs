//! The native editor shell. Domain behavior is supplied by document providers and owned contributions.
pub mod app;
pub mod contributions;
pub mod editor;
pub mod keymap;
pub mod root;
pub mod screens;
pub mod services;
pub mod tray;
pub mod zoom;

pub mod wasm_host;
pub mod wasm_ui;
pub use contributions::{
    CommandContribution, ContributionRegistry, ModeContribution, PanelContribution, PanelLocation,
};
pub use root::{ShellHandle, StudioRoot};
use sonus_extension::document::DocumentEnvironment;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug)]
pub struct DocumentOrigin {
    pub mode_id: String,
    pub provider_id: Option<String>,
    pub read_only: bool,
    pub native_path: Option<PathBuf>,
}
impl DocumentOrigin {
    pub fn local(path: Option<PathBuf>) -> Self {
        Self {
            mode_id: "sonus.local".into(),
            provider_id: None,
            read_only: false,
            native_path: path,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProviderEnvironment {
    pub audio_base_dir: Option<PathBuf>,
    /// Ordered local candidates supplied by the mode, keyed by lane ID.
    /// Canonical project source references are not rewritten.
    pub audio_sources: BTreeMap<String, Vec<PathBuf>>,
    pub document: DocumentEnvironment,
    pub values: BTreeMap<String, String>,
}
impl ProviderEnvironment {
    pub fn local(key: impl Into<String>, audio_base_dir: Option<PathBuf>) -> Self {
        Self {
            audio_base_dir,
            audio_sources: BTreeMap::new(),
            document: DocumentEnvironment::local(key),
            values: BTreeMap::new(),
        }
    }
}
