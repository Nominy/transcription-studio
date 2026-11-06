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
    CommandContribution, ContributionRegistry, ModeContribution, ModeLaunchContext,
    PanelContribution, PanelLocation,
};
pub mod providers;
use exaudio_extension::document::DocumentEnvironment;
pub use providers::{
    ProviderContribution, ProviderInstance, ProviderRegistry, ProviderSetup, ProviderSetupFactory,
    ProviderSignOutFn, ProviderStatus, ProviderStatusFn,
};
pub use root::{ShellHandle, StudioRoot};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug)]
pub struct DocumentOrigin {
    pub mode_id: String,
    /// Provider account the document was opened through; `None` for provider-less modes.
    /// Save/leave handlers and recovery policy are keyed by the mode, never by this.
    pub instance: Option<ProviderInstance>,
    pub read_only: bool,
    pub native_path: Option<PathBuf>,
}
impl DocumentOrigin {
    pub fn local(path: Option<PathBuf>) -> Self {
        Self {
            mode_id: "exaudio.local".into(),
            instance: None,
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
