//! Non-editor screens (sign-in, dashboard with projects / sub-accounts / settings) and the
//! app-wide models they share with the tray.

pub mod auth;
pub mod dashboard;
pub mod local_files;
pub mod relay;
pub mod settings;
pub mod subaccounts;
pub mod task_lookup;
pub mod ui;
pub mod updates;

use gpui_kit::*;

use relay::RelayModel;
use updates::UpdaterModel;

/// App-lifetime models (relay process, updater) reachable from any screen and the tray.
pub struct ShellModels {
    pub relay: Entity<RelayModel>,
    pub updater: Entity<UpdaterModel>,
}

impl Global for ShellModels {}

impl ShellModels {
    pub fn init(cx: &mut App) {
        let relay = cx.new(|_| RelayModel::new());
        let updater = cx.new(|_| UpdaterModel::new());
        cx.set_global(Self { relay, updater });
    }

    pub fn relay(cx: &App) -> Entity<RelayModel> {
        cx.global::<Self>().relay.clone()
    }

    pub fn updater(cx: &App) -> Entity<UpdaterModel> {
        cx.global::<Self>().updater.clone()
    }
}
