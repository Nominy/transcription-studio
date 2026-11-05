//! Empty text, whitespace and line-break rules.

use super::js::{is_blank, is_ws};
use super::{GoldLanguage, Reason, fixed, reasons};

/// Studio annotations always carry both timestamps, so Babel's `NULL_TIMESTAMPS` branch never fires.
pub(super) fn empty_content(_: &GoldLanguage, text: &str) -> Reason {
    if is_blank(text) {
        fixed(&reasons().empty_content)
    } else {
        None
    }
}

pub(super) fn double_spaces(_: &GoldLanguage, text: &str) -> Reason {
    if !is_blank(text) && text.contains("  ") {
        fixed(&reasons().double_spaces)
    } else {
        None
    }
}

pub(super) fn leading_trailing_spaces(_: &GoldLanguage, text: &str) -> Reason {
    if !is_blank(text) && (text.starts_with(is_ws) || text.ends_with(is_ws)) {
        fixed(&reasons().leading_trailing_spaces)
    } else {
        None
    }
}

pub(super) fn newline_characters(_: &GoldLanguage, text: &str) -> Reason {
    if text.contains(['\n', '\r']) {
        fixed(&reasons().newline_characters)
    } else {
        None
    }
}
