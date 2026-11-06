//! Emphasis asterisk rules (`*word*`).

use super::js::{has_letter_or_number, is_blank, is_ws};
use super::{GoldLanguage, Reason, fixed, reasons};

/// The texts between asterisk pairs, when the text has paired, single asterisks.
fn emphasized(text: &str) -> Option<impl Iterator<Item = &str>> {
    if text.contains("**") || text.matches('*').count() % 2 == 1 {
        return None;
    }
    Some(text.split('*').skip(1).step_by(2))
}

pub(super) fn multiple_asterisks(_: &GoldLanguage, text: &str) -> Reason {
    if text.contains("**") {
        fixed(&reasons().multiple_asterisks)
    } else {
        None
    }
}

pub(super) fn unpaired_asterisks(_: &GoldLanguage, text: &str) -> Reason {
    if !text.contains("**") && text.matches('*').count() % 2 == 1 {
        fixed(&reasons().unpaired_asterisks)
    } else {
        None
    }
}

pub(super) fn empty_emphasis(_: &GoldLanguage, text: &str) -> Reason {
    if emphasized(text)?.any(|inner| !has_letter_or_number(inner)) {
        fixed(&reasons().empty_emphasis)
    } else {
        None
    }
}

pub(super) fn punctuation_in_emphasis(_: &GoldLanguage, text: &str) -> Reason {
    if emphasized(text)?.any(|inner| inner.ends_with(['.', ',', '!', '?', ';', ':'])) {
        fixed(&reasons().punctuation_inside_emphasis)
    } else {
        None
    }
}

pub(super) fn spaces_in_emphasis(_: &GoldLanguage, text: &str) -> Reason {
    if emphasized(text)?
        .any(|inner| !is_blank(inner) && (inner.starts_with(is_ws) || inner.ends_with(is_ws)))
    {
        fixed(&reasons().spaces_inside_emphasis)
    } else {
        None
    }
}
