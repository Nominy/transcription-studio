//! UI language for the user-visible text this crate produces without a UI context
//! (status lines, action names). The shell sets it from its language setting; English is
//! the default.

use std::sync::atomic::{AtomicBool, Ordering};

static RUSSIAN: AtomicBool = AtomicBool::new(false);

/// Selects Russian (`true`) or English (`false`) for all later [`localized`] calls.
pub fn set_russian(russian: bool) {
    RUSSIAN.store(russian, Ordering::Relaxed);
}

/// The variant for the active language.
pub fn localized(en: &'static str, ru: &'static str) -> &'static str {
    if RUSSIAN.load(Ordering::Relaxed) {
        ru
    } else {
        en
    }
}

/// A static string in both languages; resolved with [`Text::get`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    pub en: &'static str,
    pub ru: &'static str,
}

impl Text {
    pub const fn new(en: &'static str, ru: &'static str) -> Self {
        Self { en, ru }
    }

    pub fn get(self) -> &'static str {
        localized(self.en, self.ru)
    }
}
