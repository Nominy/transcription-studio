pub mod auth;
pub mod dashboard;
pub mod error_task;
pub mod local_files;
pub mod relay;
pub mod review_settings;
pub mod settings;
pub mod subaccounts;
pub mod task_lookup;

use gpui_kit::*;
use relay::RelayModel;

/// Feature-owned relay state; the shell continues to own its updater.
pub struct ShellModels {
    pub relay: Entity<RelayModel>,
}

impl Global for ShellModels {}

impl ShellModels {
    pub fn init(cx: &mut App) {
        let relay = cx.new(|_| RelayModel::new());
        let weak = relay.downgrade();
        cx.on_app_quit(move |cx| {
            weak.update(cx, |relay, _| relay.shutdown()).ok();
            async {}
        })
        .detach();
        cx.set_global(Self { relay });
    }

    pub fn relay(cx: &App) -> Entity<RelayModel> {
        cx.global::<Self>().relay.clone()
    }
}
