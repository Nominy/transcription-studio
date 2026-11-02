//! Process-wide service handles and the bridge from tokio-backed service futures to gpui.

use gpui_kit::*;
use studio_services::babel::BabelClient;
use studio_services::prefs::UiStorage;
use studio_services::review::ReviewBackend;

/// Shared service clients. Read with `cx.global::<AppServices>()`; mutate storage with
/// `cx.update_global::<AppServices, _>(..)`.
pub struct AppServices {
    pub babel: BabelClient,
    pub review: ReviewBackend,
    pub storage: UiStorage,
    pub drafting: DraftingSettings,
}

impl Global for AppServices {}

impl AppServices {
    pub fn from_env() -> Self {
        Self {
            babel: BabelClient::from_env(),
            review: ReviewBackend::from_env(),
            storage: UiStorage::load_default(),
            drafting: DraftingSettings::default(),
        }
    }
}

/// Drafting backend inputs edited in Settings and read by the capability panel
/// (`DraftingConfig::from_inputs`). Kept in memory only: the API key is never persisted, and an
/// empty key falls back to `STUDIO_DRAFT_OPENROUTER_API_KEY` / `OPENROUTER_API_KEY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftingSettings {
    pub openrouter_api_key: String,
    pub model: String,
    pub service_tier: String,
    pub reasoning_effort: String,
}

impl Default for DraftingSettings {
    fn default() -> Self {
        Self {
            openrouter_api_key: String::new(),
            model: String::new(),
            service_tier: "default".to_string(),
            reasoning_effort: "default".to_string(),
        }
    }
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
