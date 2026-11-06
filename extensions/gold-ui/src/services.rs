//! Gold-owned inference handles and optional document-specific polish integration.

use std::{future::Future, pin::Pin, sync::Arc};

use exaudio_gold::polish::{
    AudioTrack, BackendHealth, DraftResult, DraftStreamEvent, DraftingConfig,
};
use exaudio_gold::prefs::{AiPreferences, AiSettings, ExecutionLocation};
use exaudio_gold::remote::{RemoteBackend, RemoteConfig, TimingTokenStore};
use exaudio_gold::{LocalBackend, LocalConfig};
use gpui_kit::*;
use studio_core::Project;
use studio_inference::InferenceBackend;
use studio_services::prefs::UiStorage;

pub type PolishFuture<'a, T> = Pin<Box<dyn Future<Output = anyhow::Result<T>> + Send + 'a>>;
pub type PolishEvents = Arc<dyn Fn(DraftStreamEvent) + Send + Sync>;

/// A document extension supplies its payload interpretation and network backend. Gold's
/// local timing, drafting and denoise never require this optional provider.
pub trait PolishProvider: Send + Sync {
    fn prepare_payload(
        &self,
        project: &Project,
        config: &DraftingConfig,
    ) -> anyhow::Result<serde_json::Value>;
    fn health(&self) -> PolishFuture<'_, BackendHealth>;
    fn generate_drafts(
        &self,
        payload: serde_json::Value,
        tracks: Vec<AudioTrack>,
        events: PolishEvents,
    ) -> PolishFuture<'_, DraftResult>;
}

/// Provider registration is independent of whether Gold's UI and backends are active.
#[derive(Default)]
struct RegisteredPolishProvider(Option<Arc<dyn PolishProvider>>);
impl Global for RegisteredPolishProvider {}

pub fn register_polish_provider(provider: Arc<dyn PolishProvider>, cx: &mut App) {
    cx.set_global(RegisteredPolishProvider(Some(provider.clone())));
    if cx.has_global::<GoldServices>() {
        cx.global_mut::<GoldServices>().polish = Some(provider);
    }
}

pub fn unregister_polish_provider(cx: &mut App) {
    if cx.has_global::<RegisteredPolishProvider>() {
        cx.global_mut::<RegisteredPolishProvider>().0 = None;
    }
    if cx.has_global::<GoldServices>() {
        cx.global_mut::<GoldServices>().polish = None;
    }
}

pub(crate) fn registered_polish_provider(cx: &App) -> Option<Arc<dyn PolishProvider>> {
    cx.try_global::<RegisteredPolishProvider>()
        .and_then(|registration| registration.0.clone())
}

pub struct GoldServices {
    pub ai: AiSettings,
    pub inference: InferenceBackends,
    pub polish: Option<Arc<dyn PolishProvider>>,
}

impl Global for GoldServices {}

impl GoldServices {
    pub fn new(storage: &UiStorage) -> Self {
        let ai = storage.ai_settings();
        Self {
            inference: InferenceBackends::new(&ai),
            ai,
            polish: None,
        }
    }

    /// Updates the active backends and persists through the shell's generic storage.
    pub fn update_ai(
        &mut self,
        storage: &mut UiStorage,
        change: impl FnOnce(&mut AiSettings),
    ) -> std::io::Result<()> {
        let mut next = storage.ai_settings();
        change(&mut next);
        self.ai = next.normalized();
        self.inference.apply_settings(&self.ai);
        storage.set_ai_settings(&self.ai)
    }
}

pub struct InferenceBackends {
    local: Arc<LocalBackend>,
    remote: Arc<RemoteBackend>,
    remote_settings: (String, String),
    timing_tokens: Arc<TimingTokenStore>,
}

impl InferenceBackends {
    fn new(ai: &AiSettings) -> Self {
        let timing_tokens = Arc::new(TimingTokenStore::load_default());
        Self {
            local: Arc::new(LocalBackend::new(LocalConfig::for_app())),
            remote: Arc::new(RemoteBackend::new(
                RemoteConfig::from_settings(ai),
                timing_tokens.clone(),
            )),
            remote_settings: remote_settings(ai),
            timing_tokens,
        }
    }

    pub fn backend(&self, location: ExecutionLocation) -> Arc<dyn InferenceBackend> {
        match location {
            ExecutionLocation::Local => self.local.clone(),
            ExecutionLocation::Remote => self.remote.clone(),
        }
    }

    pub fn local(&self) -> Arc<LocalBackend> {
        self.local.clone()
    }

    fn apply_settings(&mut self, ai: &AiSettings) {
        let settings = remote_settings(ai);
        if settings != self.remote_settings {
            self.remote = Arc::new(RemoteBackend::new(
                RemoteConfig::from_settings(ai),
                self.timing_tokens.clone(),
            ));
            self.remote_settings = settings;
        }
    }
}

fn remote_settings(ai: &AiSettings) -> (String, String) {
    (ai.remote_base_url.clone(), ai.remote_access_key.clone())
}
