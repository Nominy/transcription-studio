//! Interface zoom (Ctrl+= / Ctrl+- / Ctrl+0), like zooming a web page.
//!
//! The zoom scales the theme's base font size, which gpui-component installs as the window's
//! rem size every frame. Everything sized in rems (component paddings, text, our layout
//! constants via [`ui_scale`]) grows and shrinks with it. The level is persisted.

use gpui_kit::component::Theme;
use gpui_kit::*;

use crate::services::AppServices;

actions!(studio, [ZoomIn, ZoomOut, ZoomReset]);

const STORAGE_KEY: &str = "studio.uiZoom.v1";
/// gpui-component's default base font sizes at 100%.
const BASE_FONT_SIZE: f32 = 16.;
const BASE_MONO_FONT_SIZE: f32 = 13.;
/// Zoom levels, as in a browser.
pub const LEVELS: [f32; 11] = [0.67, 0.75, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 1.75, 2.0, 2.5];

/// Current interface zoom (1.0 = 100%).
pub struct UiZoom(pub f32);

impl Global for UiZoom {}

/// Registers the zoom shortcuts and applies the saved level. Call after [`AppServices`] and
/// the theme are installed.
pub fn init(cx: &mut App) {
    #[cfg(target_os = "macos")]
    let keys = ["cmd-=", "cmd-+", "cmd-shift-=", "cmd--", "cmd-0"];
    #[cfg(not(target_os = "macos"))]
    let keys = ["ctrl-=", "ctrl-+", "ctrl-shift-=", "ctrl--", "ctrl-0"];
    cx.bind_keys([
        KeyBinding::new(keys[0], ZoomIn, None),
        KeyBinding::new(keys[1], ZoomIn, None),
        KeyBinding::new(keys[2], ZoomIn, None),
        KeyBinding::new(keys[3], ZoomOut, None),
        KeyBinding::new(keys[4], ZoomReset, None),
    ]);
    cx.on_action(|_: &ZoomIn, cx| step(1, cx));
    cx.on_action(|_: &ZoomOut, cx| step(-1, cx));
    cx.on_action(|_: &ZoomReset, cx| set(1.0, cx));

    let saved = cx
        .global::<AppServices>()
        .storage
        .value(STORAGE_KEY)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .map_or(1.0, clamp);
    cx.set_global(UiZoom(saved));
    reapply(cx);
}

/// Current zoom level.
pub fn level(cx: &App) -> f32 {
    cx.global::<UiZoom>().0
}

/// Moves to the next larger (`direction > 0`) or smaller level.
pub fn step(direction: i32, cx: &mut App) {
    let current = level(cx);
    let next = if direction > 0 {
        LEVELS
            .iter()
            .copied()
            .find(|level| *level > current + 0.001)
    } else {
        LEVELS
            .iter()
            .rev()
            .copied()
            .find(|level| *level < current - 0.001)
    };
    if let Some(next) = next {
        set(next, cx);
    }
}

/// Sets and persists the zoom level.
pub fn set(zoom: f32, cx: &mut App) {
    let zoom = clamp(zoom);
    cx.set_global(UiZoom(zoom));
    reapply(cx);
    let value = format!("{zoom}");
    if let Err(error) = cx
        .global_mut::<AppServices>()
        .storage
        .set_value(STORAGE_KEY, value)
    {
        eprintln!("failed to save interface zoom: {error}");
    }
}

/// Re-applies the zoom to the theme's font sizes (call after the theme was reloaded).
pub fn reapply(cx: &mut App) {
    let zoom = level(cx);
    Theme::update(cx, |theme| {
        theme.font_size = px(BASE_FONT_SIZE * zoom);
        theme.mono_font_size = px(BASE_MONO_FONT_SIZE * zoom);
    });
}

/// Multiplier for layout constants written in 100%-zoom pixels:
/// `RAIL_WIDTH * ui_scale(window)`.
pub fn ui_scale(window: &Window) -> f32 {
    window.rem_size() / px(BASE_FONT_SIZE)
}

fn clamp(zoom: f32) -> f32 {
    zoom.clamp(LEVELS[0], LEVELS[LEVELS.len() - 1])
}
