//! One language per string. Every user-facing Babel string goes through `localized`
//! (static text) or `localized_fmt` (templates); errors use `failed` / `with_detail`.

use exaudio_babel_domain::i18n;
use gpui_kit::*;

pub use exaudio_babel_domain::i18n::{count, fill, with_detail};

pub fn is_russian(cx: &App) -> bool {
    let russian = exaudio_shell::screens::ui::localized(cx, "en", "ru") == "ru";
    i18n::set_russian(russian);
    russian
}

/// Picks EN or RU from the shell language and mirrors the choice into the domain crates.
pub fn localized(cx: &App, en: &'static str, ru: &'static str) -> &'static str {
    if is_russian(cx) { ru } else { en }
}

/// `localized` for templates: every `{name}` in the chosen text is replaced from `args`.
pub fn localized_fmt(
    cx: &App,
    en: &'static str,
    ru: &'static str,
    args: &[(&str, &str)],
) -> String {
    fill(localized(cx, en, ru), args)
}

/// A plain headline followed by the technical text after an em dash, for single-line
/// surfaces (status bar, toasts). Banners and dialogs use the shell's `error_block`.
pub fn failed(
    cx: &App,
    en: &'static str,
    ru: &'static str,
    error: impl std::fmt::Display,
) -> String {
    with_detail(localized(cx, en, ru), error)
}
