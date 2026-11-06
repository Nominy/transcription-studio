//! The active editor keymap (defaults + persisted user overrides) and keystroke helpers.

use gpui_kit::*;
use studio_editor::edit::keymap::{HOTKEY_STORAGE_KEY, KeyFocus};
use studio_editor::edit::{EditorAction, Keymap};

use crate::services::AppServices;

pub const IS_MAC: bool = cfg!(target_os = "macos");

const HOST_OS: OperatingSystem = if IS_MAC {
    OperatingSystem::Mac
} else if cfg!(target_os = "windows") {
    OperatingSystem::Windows
} else {
    OperatingSystem::Linux
};

/// Global holding the user's keymap. Mutate through [`ActiveKeymap::update`] so overrides
/// are persisted.
pub struct ActiveKeymap(pub Keymap);

impl Global for ActiveKeymap {}

impl ActiveKeymap {
    /// Loads overrides from UI storage (call after [`AppServices`] is installed).
    pub fn init(cx: &mut App) {
        let keymap = cx
            .global::<AppServices>()
            .storage
            .value(HOTKEY_STORAGE_KEY)
            .map(Keymap::from_json)
            .unwrap_or_default();
        cx.set_global(Self(keymap));
        // Only the editor owns Tab; shared pages retain normal keyboard focus traversal.
        cx.bind_keys([
            KeyBinding::new("tab", NoAction, Some("ExaudioEditor")),
            KeyBinding::new("shift-tab", NoAction, Some("ExaudioEditor")),
        ]);
    }

    pub fn resolve(cx: &App, keystroke: &Keystroke, focus: KeyFocus) -> Option<EditorAction> {
        cx.global::<Self>()
            .0
            .resolve(&keystroke_string(keystroke), focus, IS_MAC)
    }

    /// Applies `f` to the keymap and persists the result.
    pub fn update<R>(cx: &mut App, f: impl FnOnce(&mut Keymap) -> R) -> anyhow::Result<R> {
        let result = f(&mut cx.global_mut::<Self>().0);
        let json = cx.global::<Self>().0.to_json();
        cx.global_mut::<AppServices>()
            .storage
            .set_value(HOTKEY_STORAGE_KEY, json)?;
        Ok(result)
    }
}

/// Canonical gpui keystroke text (`ctrl-shift-z`) as understood by [`Keymap`].
pub fn keystroke_string(keystroke: &Keystroke) -> String {
    keystroke.unparse(HOST_OS)
}
