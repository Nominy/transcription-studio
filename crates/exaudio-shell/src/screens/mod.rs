pub mod accounts;
pub mod extensions;
pub mod formats;
pub mod home;
pub mod local_files;
pub mod recovery;
pub mod settings;
pub mod ui;
pub mod updates;
use gpui_kit::*;
use updates::UpdaterModel;
pub struct ShellModels {
    pub updater: Entity<UpdaterModel>,
}
impl Global for ShellModels {}
impl ShellModels {
    pub fn init(cx: &mut App) {
        let updater = cx.new(UpdaterModel::new);
        cx.set_global(Self { updater });
    }
    pub fn updater(cx: &App) -> Entity<UpdaterModel> {
        cx.global::<Self>().updater.clone()
    }
}
