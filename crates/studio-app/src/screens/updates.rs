//! Update check / install state shared by Settings (survives screen switches).

use gpui_kit::*;
use studio_services::updater::{self, UpdateState};

use crate::services::spawn_service;

/// This binary's version (`STUDIO_BUILD_VERSION` from packaging, else the crate version).
pub const APP_VERSION: &str = match option_env!("STUDIO_BUILD_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

pub struct UpdaterModel {
    state: UpdateState,
    task: Option<Task<()>>,
}

impl UpdaterModel {
    pub fn new() -> Self {
        Self {
            state: UpdateState::new(APP_VERSION),
            task: None,
        }
    }

    pub fn state(&self) -> &UpdateState {
        &self.state
    }

    pub fn check(&mut self, cx: &mut Context<Self>) {
        if !self.state.begin_check() {
            return;
        }
        let feed_url = self.state.feed_url.clone();
        self.task = Some(spawn_service(
            cx,
            updater::check_for_updates(feed_url),
            |this, result, cx| {
                this.task = None;
                this.state
                    .finish_check(result.map_err(|error| format!("{error:#}")));
                cx.notify();
            },
        ));
        cx.notify();
    }

    pub fn install(&mut self, cx: &mut Context<Self>) {
        if !self.state.begin_install() {
            return;
        }
        let feed_url = self.state.feed_url.clone();
        self.task = Some(spawn_service(
            cx,
            updater::install_available_update(feed_url),
            |this, result, cx| {
                this.task = None;
                this.state
                    .finish_install(result.map_err(|error| format!("{error:#}")));
                cx.notify();
            },
        ));
        cx.notify();
    }
}
