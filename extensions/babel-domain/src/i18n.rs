//! UI language for user-facing domain and service messages.
//!
//! The domain and service crates have no access to the shell's settings, so the Babel UI
//! mirrors the shell language here (`exaudio_babel_ui::text::localized` does it on every
//! call). Messages built before the first UI render fall back to Russian, matching the
//! shell default.

use std::sync::atomic::{AtomicBool, Ordering};

static RUSSIAN: AtomicBool = AtomicBool::new(true);

/// Mirrors the shell language; called by the Babel UI whenever it localizes a string.
pub fn set_russian(russian: bool) {
    RUSSIAN.store(russian, Ordering::Relaxed);
}

pub fn is_russian() -> bool {
    RUSSIAN.load(Ordering::Relaxed)
}

/// Picks the text for the current UI language.
pub fn localized(en: &'static str, ru: &'static str) -> &'static str {
    if is_russian() { ru } else { en }
}

/// `localized` for templates: every `{name}` in the chosen text is replaced from `args`.
pub fn localized_fmt(en: &'static str, ru: &'static str, args: &[(&str, &str)]) -> String {
    fill(localized(en, ru), args)
}

/// Replaces `{name}` placeholders in `template` with the paired values.
pub fn fill(template: &str, args: &[(&str, &str)]) -> String {
    let mut text = template.to_owned();
    for (name, value) in args {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

/// Russian plural form for `count`: `(1 сегмент, 2 сегмента, 5 сегментов)`.
pub fn plural_ru(count: u64, one: &str, few: &str, many: &str) -> String {
    let n = count % 100;
    let form = if (11..=14).contains(&n) {
        many
    } else {
        match n % 10 {
            1 => one,
            2..=4 => few,
            _ => many,
        }
    };
    format!("{count} {form}")
}

/// English plural form for `count`: `(1 segment, 5 segments)`.
pub fn plural_en(count: u64, one: &str, many: &str) -> String {
    if count == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {many}")
    }
}

/// Count with the right noun form for the current language.
pub fn count(count: u64, en: (&str, &str), ru: (&str, &str, &str)) -> String {
    if is_russian() {
        plural_ru(count, ru.0, ru.1, ru.2)
    } else {
        plural_en(count, en.0, en.1)
    }
}

/// `"{headline} — {detail}"`: a plain sentence followed by the technical text, for
/// single-line surfaces (status bar, toasts) that cannot show a Details disclosure.
pub fn with_detail(headline: &str, detail: impl std::fmt::Display) -> String {
    let detail = detail.to_string();
    if detail.trim().is_empty() {
        headline.to_owned()
    } else {
        format!("{headline} — {detail}")
    }
}
