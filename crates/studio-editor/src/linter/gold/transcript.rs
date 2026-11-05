//! Per-track timing rules and transcript-wide rules.

use std::collections::HashSet;

use super::js::{trim, utf16_len};
use super::{GoldAnnotation, GoldLanguage, Issues, reasons};

pub(super) fn overlapping_segments(
    _: &GoldLanguage,
    earlier: &GoldAnnotation<'_>,
    later: &GoldAnnotation<'_>,
    issues: &mut Issues<'_>,
) {
    if earlier.end_seconds > later.start_seconds {
        let reason = &*reasons().overlapping_segments;
        issues.push(earlier.id, reason);
        issues.push(later.id, reason);
    }
}

/// Babel `validateDistanceBetweenAnnotations`. Neighbours on one track share Babel's speaker key
/// (the track), so every pair is a same-speaker pair.
pub(super) fn distance_between_annotations(
    language: &GoldLanguage,
    earlier: &GoldAnnotation<'_>,
    later: &GoldAnnotation<'_>,
    issues: &mut Issues<'_>,
) {
    if later.start_seconds - earlier.end_seconds < language.min_distance_seconds {
        issues.push(earlier.id, language.distance_reason.as_str());
    }
}

pub(super) fn unresolved_low_confidence(
    _: &GoldLanguage,
    annotations: &[GoldAnnotation<'_>],
    issues: &mut Issues<'_>,
) {
    for annotation in annotations
        .iter()
        .filter(|annotation| annotation.unresolved_low_confidence)
    {
        issues.push(annotation.id, &*reasons().unresolved_low_confidence);
    }
}

/// Babel `e0` order: annotations stably sorted by start time.
fn by_start<'s, 'a>(annotations: &'s [GoldAnnotation<'a>]) -> Vec<&'s GoldAnnotation<'a>> {
    let mut sorted: Vec<&GoldAnnotation<'a>> = annotations.iter().collect();
    sorted.sort_by(|left, right| {
        left.start_seconds
            .partial_cmp(&right.start_seconds)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    sorted
}

/// Babel `e0` `duplicated-segments`: a 30+ character text repeating an earlier one.
pub(super) fn duplicated_segments(
    _: &GoldLanguage,
    annotations: &[GoldAnnotation<'_>],
    issues: &mut Issues<'_>,
) {
    let mut seen = HashSet::new();
    for annotation in by_start(annotations) {
        let text = trim(annotation.text);
        if utf16_len(text) >= 30 && !seen.insert(text) {
            issues.push(annotation.id, &*reasons().duplicated_segments);
        }
    }
}

/// Babel `e0` `duplicated-segments-adjacent`: a 20–29 character text repeating its predecessor.
pub(super) fn duplicated_segments_adjacent(
    _: &GoldLanguage,
    annotations: &[GoldAnnotation<'_>],
    issues: &mut Issues<'_>,
) {
    for pair in by_start(annotations).windows(2) {
        let text = trim(pair[0].text);
        if (20..30).contains(&utf16_len(text)) && text == trim(pair[1].text) {
            issues.push(pair[1].id, &*reasons().duplicated_segments_adjacent);
        }
    }
}
