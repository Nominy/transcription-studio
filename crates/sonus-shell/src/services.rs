//! Process-wide service handles and the bridge from tokio-backed service futures to gpui.

use gpui_kit::*;
use studio_services::prefs::UiStorage;

/// Shared service clients. Read with `cx.global::<AppServices>()`; mutate storage with
/// `cx.update_global::<AppServices, _>(..)`.
pub struct AppServices {
    pub storage: UiStorage,
}

impl Global for AppServices {}

impl AppServices {
    pub fn from_env() -> Self {
        let storage = UiStorage::load_default();
        Self { storage }
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
