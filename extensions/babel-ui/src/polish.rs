//! Optional Gold polish transport. This capability works on generic local projects too;
//! installing it never marks a document as Babel-origin or selects its task route.

use exaudio_babel_services::review::{ReviewBackend, build_draft_generate_payload};
use exaudio_gold::polish::{AudioTrack, BackendHealth, DraftResult, DraftingConfig};
use exaudio_gold_ui::services::{
    PolishEvents, PolishFuture, PolishProvider, register_polish_provider,
    unregister_polish_provider,
};
use gpui_kit::*;
use std::sync::Arc;
use studio_core::Project;

struct BabelPolishProvider(ReviewBackend);

impl PolishProvider for BabelPolishProvider {
    fn prepare_payload(
        &self,
        project: &Project,
        config: &DraftingConfig,
    ) -> anyhow::Result<serde_json::Value> {
        build_draft_generate_payload(project, config)
    }

    fn health(&self) -> PolishFuture<'_, BackendHealth> {
        Box::pin(self.0.health())
    }

    fn generate_drafts(
        &self,
        payload: serde_json::Value,
        tracks: Vec<AudioTrack>,
        events: PolishEvents,
    ) -> PolishFuture<'_, DraftResult> {
        Box::pin(
            self.0
                .generate_drafts(payload, tracks, move |event| events(event)),
        )
    }
}

/// Registers only the polish service, even while Gold is disabled. Local inference and
/// the document's platform identity remain independent.
pub fn install(cx: &mut App) {
    register_polish_provider(Arc::new(BabelPolishProvider(ReviewBackend::from_env())), cx);
}

pub fn uninstall(cx: &mut App) {
    unregister_polish_provider(cx);
}
