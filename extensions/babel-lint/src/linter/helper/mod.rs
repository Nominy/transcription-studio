//! Port of the Babel Helper extension's custom linter (page.ts `buildBuiltInCustomIssues`
//! over `createBuiltInLintRules`) with Helper's default settings (`DEFAULT_EXTENSION_SETTINGS`
//! in settings.ts): its default-disabled rules (curly-, angle- and square-bracket-tag
//! trailing punctuation) are not part of the port, highlighted words are on with
//! `DEFAULT_HIGHLIGHTED_WORDS`, non-Emotion projects.
//!
//! Rule order, reasons and severities, the highlighted words, the interjection corrections
//! and the ё exception dictionary come from `assets/helper-lint-data.json`, generated from
//! Helper's sources by `tools/lint-parity/generate.mjs`. Each rule reports whether a row has
//! at least one match; JS UTF-16 indexing and regex semantics are reproduced in [`text`].

mod capitalization;
mod dashes;
mod spacing;
mod text;
mod words;

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::Deserialize;

use super::LintSeverity;
use text::{Text, Token, tokenize};
use words::Literals;

/// page.ts `compactMatches` keeps a rule's first eight regex matches; rules that filter
/// matches afterwards only see those.
const COMPACTED_MATCH_LIMIT: usize = 8;

pub(crate) struct HelperEntry<'a> {
    pub annotation_id: &'a str,
    pub text: &'a str,
    pub speaker_key: &'a str,
}

pub(crate) struct HelperIssue {
    pub annotation_id: String,
    pub rule_id: &'static str,
    pub reason: &'static str,
    pub severity: LintSeverity,
}

/// Helper's issues for `entries` in time order: per entry, one issue per matching rule, in
/// rule order.
pub(crate) fn helper_issues(entries: &[HelperEntry<'_>]) -> Vec<HelperIssue> {
    let linter = &*LINTER;
    let mut issues = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let text = Text::new(entry.text);
        let row = Row {
            tokens: tokenize(&text),
            text,
            entries,
            index,
            tables: &linter.tables,
        };
        issues.extend(
            linter
                .rules
                .iter()
                .filter(|rule| (rule.check)(&row))
                .map(|rule| HelperIssue {
                    annotation_id: entry.annotation_id.to_string(),
                    rule_id: &rule.id,
                    reason: &rule.reason,
                    severity: rule.severity,
                }),
        );
    }
    issues
}

/// One entry being linted, with the transcript around it.
struct Row<'a> {
    text: Text<'a>,
    tokens: Vec<Token>,
    entries: &'a [HelperEntry<'a>],
    index: usize,
    tables: &'a Tables,
}

type Check = fn(&Row<'_>) -> bool;

struct Rule {
    id: String,
    reason: String,
    severity: LintSeverity,
    check: Check,
}

/// The data tables rules consult.
struct Tables {
    interjections: Literals,
    highlighted_words: Literals,
    yo_exceptions: HashSet<String>,
}

struct Linter {
    rules: Vec<Rule>,
    tables: Tables,
}

const HELPER_LINT_DATA_JSON: &str = include_str!("../../../assets/helper-lint-data.json");

#[derive(Deserialize)]
struct HelperLintData {
    rules: Vec<RuleData>,
    highlighted_words: Vec<String>,
    interjection_corrections: Vec<InterjectionCorrection>,
    yo_exceptions: YoExceptions,
}

#[derive(Deserialize)]
struct RuleData {
    id: String,
    reason: String,
    severity: Severity,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    Error,
    Warning,
}

/// Only the variants decide whether a row matches; the canonical spelling is the fix.
#[derive(Deserialize)]
struct InterjectionCorrection {
    variants: Vec<String>,
}

#[derive(Deserialize)]
struct YoExceptions {
    words: Vec<String>,
}

static LINTER: LazyLock<Linter> = LazyLock::new(|| {
    let data: HelperLintData = serde_json::from_str(HELPER_LINT_DATA_JSON)
        .expect("embedded helper-lint-data.json is valid");
    Linter {
        rules: data
            .rules
            .into_iter()
            .map(|rule| Rule {
                check: rule_check(&rule.id),
                severity: match rule.severity {
                    Severity::Error => LintSeverity::Error,
                    Severity::Warning => LintSeverity::Warning,
                },
                id: rule.id,
                reason: rule.reason,
            })
            .collect(),
        tables: Tables {
            interjections: Literals::interjections(
                data.interjection_corrections
                    .iter()
                    .flat_map(|correction| correction.variants.iter().map(String::as_str)),
            ),
            highlighted_words: Literals::highlighted_words(
                data.highlighted_words.iter().map(String::as_str),
            ),
            yo_exceptions: data.yo_exceptions.words.into_iter().collect(),
        },
    }
});

fn rule_check(id: &str) -> Check {
    match id {
        "leading-trailing-spaces" => spacing::leading_trailing_spaces,
        "double-spaces" => spacing::double_spaces,
        "comma-spacing" => spacing::comma_spacing,
        "period-spacing" => spacing::period_spacing,
        "angle-tag-spacing" => spacing::angle_tag_spacing,
        "square-bracket-tag-spacing" => spacing::square_bracket_tag_spacing,
        "curly-tag-spacing" => spacing::curly_tag_spacing,
        "quote-balance" => spacing::quote_balance,
        "unicode-quotes" => spacing::unicode_quotes,
        "unicode-dashes" => spacing::unicode_dashes,
        "free-mid-sentence-double-dash" => dashes::free_mid_sentence_double_dash,
        "double-dash-punctuation" => dashes::double_dash_punctuation,
        "single-dash-punctuation" => dashes::single_dash_punctuation,
        "terminal-punctuation" => dashes::terminal_punctuation,
        "normalized-stutters" => words::normalized_stutters,
        "incorrect-interjection-forms" => words::incorrect_interjection_forms,
        "unnecessary-yo" => words::unnecessary_yo,
        "highlighted-words" => words::highlighted_words,
        "sentence-boundary-capitalization" => capitalization::sentence_boundary_capitalization,
        "polite-pronoun-case" => capitalization::polite_pronoun_case,
        "segment-start-capitalization" => capitalization::segment_start_capitalization,
        _ => panic!(
            "helper-lint-data.json lists rule {id:?}, which the Studio port does not implement"
        ),
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::{HelperEntry, LintSeverity, helper_issues};

    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        segments: Vec<Segment>,
        helper: Vec<Issue>,
    }

    #[derive(Deserialize)]
    struct Segment {
        id: String,
        text: String,
        start: f64,
        track: String,
    }

    #[derive(Deserialize)]
    struct Issue {
        id: String,
        reason: String,
        severity: String,
    }

    fn severity_name(severity: LintSeverity) -> &'static str {
        match severity {
            LintSeverity::Pass => "pass",
            LintSeverity::Warning => "warning",
            LintSeverity::Error => "error",
        }
    }

    #[test]
    fn matches_helper_custom_linter_on_parity_corpus() {
        let Some(json) =
            crate::data::lint_parity_json("matches_helper_custom_linter_on_parity_corpus")
        else {
            return;
        };
        let fixture: Fixture = serde_json::from_str(&json).expect("parity fixture is valid");
        let mut failures = Vec::new();
        for case in &fixture.cases {
            let mut segments = case.segments.iter().collect::<Vec<_>>();
            segments.sort_by(|left, right| left.start.total_cmp(&right.start));
            let entries = segments
                .iter()
                .map(|segment| HelperEntry {
                    annotation_id: &segment.id,
                    text: &segment.text,
                    speaker_key: &segment.track,
                })
                .collect::<Vec<_>>();
            let actual = helper_issues(&entries)
                .into_iter()
                .map(|issue| {
                    format!(
                        "{} {} {}",
                        issue.annotation_id,
                        severity_name(issue.severity),
                        issue.reason
                    )
                })
                .collect::<Vec<_>>();
            let expected = case
                .helper
                .iter()
                .map(|issue| format!("{} {} {}", issue.id, issue.severity, issue.reason))
                .collect::<Vec<_>>();
            if actual != expected {
                failures.push(format!(
                    "{}\n  expected {expected:#?}\n  actual   {actual:#?}",
                    case.name
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} cases differ from Helper:\n{}",
            failures.len(),
            fixture.cases.len(),
            failures.join("\n")
        );
    }
}
