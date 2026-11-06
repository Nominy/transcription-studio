//! Optional, language-neutral transcript diagnostics, never an editor invariant.
//!
//! Only ASCII SPACE and TAB are normalized, and only through explicit quick fixes.
//! NBSP, narrow/thin/ideographic spaces and all line separators are preserved.
//! Punctuation has no universally erroneous form: ellipses, CJK punctuation,
//! repeated expressive marks and stutters are deliberately not checked.
//! Timing and empty-text findings never offer destructive or speculative fixes.

use exaudio_extension::{
    document::{
        Diagnostic, DiagnosticCollection, DiagnosticProvider, DocumentEdit, DocumentProviders,
        QuickFix, Severity, TextRange,
    },
    manifest::ExtensionManifest,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use studio_core::{Project, Segment};

pub const ID: &str = "exaudio.lint-basic";
pub const MANIFEST_JSON: &str = include_str!("../manifest.json");

pub fn manifest() -> Result<ExtensionManifest, serde_json::Error> {
    serde_json::from_str(MANIFEST_JSON)
}

/// Pass this contribution to the host's owner-scoped provider registry. The host
/// attaches it only to enabled Local documents and drops its collection on disable.
pub fn providers(language: Language) -> DocumentProviders {
    DocumentProviders {
        diagnostics: vec![Arc::new(BasicChecks { language })],
        ..Default::default()
    }
}

/// UI language of the messages this provider emits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Language {
    #[default]
    En,
    Ru,
}

impl Language {
    fn pick(self, en: &'static str, ru: &'static str) -> &'static str {
        match self {
            Language::En => en,
            Language::Ru => ru,
        }
    }
}

#[derive(Debug, Default)]
pub struct BasicChecks {
    pub language: Language,
}

fn finding(segment: &Segment, rule: &str, severity: Severity, message: &str) -> Diagnostic {
    Diagnostic {
        source: ID.into(),
        rule: format!("{ID}.{rule}"),
        severity,
        segment_id: segment.id.clone(),
        range: None,
        message: message.into(),
        fixes: Vec::new(),
    }
}

fn valid_timing(segment: &Segment) -> bool {
    segment.start_seconds.is_finite()
        && segment.end_seconds.is_finite()
        && segment.start_seconds >= 0.0
        && segment.end_seconds >= segment.start_seconds
}

fn horizontal(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t')
}

fn line_separator(c: char) -> bool {
    matches!(
        c,
        '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}' | '\u{000b}' | '\u{000c}'
    )
}
fn whitespace(segment: &Segment, language: Language, revision: u64, out: &mut Vec<Diagnostic>) {
    let bytes = segment.text.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if !horizontal(bytes[cursor]) {
            cursor += 1;
            continue;
        }
        let start = cursor;
        while cursor < bytes.len() && horizontal(bytes[cursor]) {
            cursor += 1;
        }
        // Only transcript boundaries, not line indentation or trailing Markdown
        // line-break spaces, are trimmed. Internal line breaks are never edited.
        let (rule, message, replacement) = if start == 0 {
            (
                "leading-whitespace",
                language.pick("Starts with a space", "Начинается с пробела"),
                "",
            )
        } else if cursor == bytes.len() {
            (
                "trailing-whitespace",
                language.pick("Ends with a space", "Заканчивается пробелом"),
                "",
            )
        } else if cursor - start > 1
            && !segment.text[..start]
                .chars()
                .next_back()
                .is_some_and(line_separator)
            && !segment.text[cursor..]
                .chars()
                .next()
                .is_some_and(line_separator)
        {
            (
                "repeated-whitespace",
                language.pick("Double spaces", "Двойные пробелы"),
                " ",
            )
        } else {
            continue;
        };
        // Every endpoint abuts an ASCII byte, hence is a UTF-8 boundary.
        let range = TextRange { start, end: cursor };
        let mut diagnostic = finding(segment, rule, Severity::Hint, message);
        diagnostic.range = Some(range);
        diagnostic.fixes.push(QuickFix {
            title: language
                .pick("Remove extra spaces", "Убрать лишние пробелы")
                .into(),
            expected_revision: revision,
            edits: vec![DocumentEdit::ReplaceText {
                segment_id: segment.id.clone(),
                range: Some(range),
                text: replacement.into(),
            }],
        });
        out.push(diagnostic);
    }
}

impl DiagnosticProvider for BasicChecks {
    fn diagnose(&self, project: &Project, revision: u64) -> DiagnosticCollection {
        let mut diagnostics = Vec::new();
        let mut lanes: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, segment) in project.segments.iter().enumerate() {
            whitespace(segment, self.language, revision, &mut diagnostics);
            if segment.text.trim().is_empty() {
                diagnostics.push(finding(
                    segment,
                    "empty-transcript",
                    Severity::Warning,
                    self.language.pick("Empty segment", "Пустой сегмент"),
                ));
            }
            if !valid_timing(segment) {
                diagnostics.push(finding(
                    segment,
                    "invalid-timing",
                    Severity::Error,
                    self.language
                        .pick("End time is before start time", "Конец раньше начала"),
                ));
            } else if segment.end_seconds > segment.start_seconds {
                lanes.entry(&segment.lane_id).or_default().push(index);
            }
        }
        let mut overlapping = BTreeSet::new();
        for indices in lanes.values_mut() {
            indices.sort_by(|&a, &b| {
                project.segments[a]
                    .start_seconds
                    .total_cmp(&project.segments[b].start_seconds)
                    .then(a.cmp(&b))
            });
            let mut longest: Option<usize> = None;
            for &index in indices.iter() {
                let segment = &project.segments[index];
                if let Some(previous) = longest {
                    let earlier = &project.segments[previous];
                    if segment.start_seconds < earlier.end_seconds {
                        overlapping.insert(previous);
                        overlapping.insert(index);
                    }
                    if segment.end_seconds <= earlier.end_seconds {
                        continue;
                    }
                }
                longest = Some(index);
            }
        }
        for index in overlapping {
            diagnostics.push(finding(
                &project.segments[index],
                "same-lane-overlap",
                Severity::Warning,
                self.language.pick(
                    "Overlaps another segment on this track",
                    "Пересекается с другим сегментом на дорожке",
                ),
            ));
        }
        DiagnosticCollection {
            source: ID.into(),
            revision,
            diagnostics,
        }
    }
}
