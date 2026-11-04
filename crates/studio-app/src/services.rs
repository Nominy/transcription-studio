//! Process-wide service handles and the bridge from tokio-backed service futures to gpui.

use std::sync::Arc;

use gpui_kit::*;
use studio_inference::{LocalBackend, LocalConfig};
use studio_services::babel::BabelClient;
use studio_services::inference::{InferenceBackend, RemoteBackend, RemoteConfig, TimingTokenStore};
use studio_services::prefs::{AiSettings, ExecutionLocation, UiStorage};
use studio_services::review::ReviewBackend;

/// Shared service clients. Read with `cx.global::<AppServices>()`; mutate storage with
/// `cx.update_global::<AppServices, _>(..)`.
pub struct AppServices {
    pub babel: BabelClient,
    pub review: ReviewBackend,
    pub storage: UiStorage,
    /// AI settings (Settings → AI), persisted in `storage`; change through
    /// [`AppServices::update_ai`].
    pub ai: AiSettings,
    /// Gold inference backends (This PC / server), kept in step with `ai`.
    pub inference: InferenceBackends,
}

impl Global for AppServices {}

impl AppServices {
    pub fn from_env() -> Self {
        let storage = UiStorage::load_default();
        let ai = storage.ai_settings();
        Self {
            babel: BabelClient::from_env(),
            review: ReviewBackend::from_env(),
            inference: InferenceBackends::new(&ai),
            ai,
            storage,
        }
    }

    /// Applies `change` to the AI settings, normalizes them and writes them through to storage.
    pub fn update_ai(&mut self, change: impl FnOnce(&mut AiSettings)) -> std::io::Result<()> {
        let mut next = self.ai.clone();
        change(&mut next);
        self.ai = next.normalized();
        self.inference.apply_settings(&self.ai);
        self.storage.set_ai_settings(&self.ai)
    }
}

/// The local (Gold host page in WebView2) and remote (coordinator) inference backends.
pub struct InferenceBackends {
    local: Arc<LocalBackend>,
    remote: Arc<RemoteBackend>,
    /// Server address and access key `remote` was built with.
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

    /// The backend for `location` (`ai.execution_location` or `ai.denoise_location`).
    pub fn backend(&self, location: ExecutionLocation) -> Arc<dyn InferenceBackend> {
        match location {
            ExecutionLocation::Local => self.local.clone(),
            ExecutionLocation::Remote => self.remote.clone(),
        }
    }

    /// This PC: also model install/test/remove and the detailed host status for Settings.
    pub fn local(&self) -> Arc<LocalBackend> {
        self.local.clone()
    }

    /// Rebuilds the remote backend when its server address or key changed; requests already
    /// running keep the backend they started with.
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

/// Runs a `Send` service future on the services runtime and delivers its output to `on_done`
/// on the UI thread. The completion is dropped if the entity is gone.
pub fn spawn_service<T, F, R>(
    cx: &mut Context<T>,
    fut: F,
    on_done: impl FnOnce(&mut T, R, &mut Context<T>) + 'static,
) -> Task<()>
where
    T: 'static,
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let pending = studio_services::runtime::spawn(fut);
    cx.spawn(async move |this, cx| {
        let output = pending.await;
        this.update(cx, |this, cx| on_done(this, output, cx)).ok();
    })
}

/// Like [`spawn_service`], but the completion also gets the window (dialogs, notifications,
/// focus).
pub fn spawn_service_in<T, F, R>(
    window: &Window,
    cx: &mut Context<T>,
    fut: F,
    on_done: impl FnOnce(&mut T, R, &mut Window, &mut Context<T>) + 'static,
) -> Task<()>
where
    T: 'static,
    F: Future<Output = R> + Send + 'static,
    R: Send + 'static,
{
    let pending = studio_services::runtime::spawn(fut);
    cx.spawn_in(window, async move |this, cx| {
        let output = pending.await;
        this.update_in(cx, |this, window, cx| on_done(this, output, window, cx))
            .ok();
    })
}
