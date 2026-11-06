//! Babel's native linter configured for the `gold-non-bg` transcription variant ("Gold"), ported
//! rule for rule from Babel's `lintAnnotations`. Per-language configuration and rule tables come
//! from `assets/babel-lint-data.json` ([`crate::data::LINT_DATA_JSON`]), generated from Babel by
//! `tools/lint-parity/generate.mjs`.
//!
//! Babel runs on JavaScript strings; the ports keep its semantics (JS whitespace, UTF-16 lengths
//! where they reach a reason or a threshold) while indexing Rust strings by byte.

/// Characters JavaScript's `\s` and `String.prototype.trim` treat as whitespace, as the body of a
/// regex character class.
macro_rules! ws_chars {
    () => {
        r"\t\n\x0B\x0C\r \xA0\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}"
    };
}

mod brackets;
mod cjk;
mod curly_tags;
mod dashes;
mod emphasis;
mod foreign;
mod heuristics;
mod js;
mod spacing;
mod style_tags;
mod transcript;

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, OnceLock};

use regex::Regex;
use serde::Deserialize;

use super::LintSeverity;
use js::{WS, regex};

pub(crate) struct BracketType {
    pub open: &'static str,
    pub close: &'static str,
}

/// One Babel transcription language's gold-non-bg linter configuration.
pub(crate) struct GoldLanguage {
    /// Upper-case Babel language code, e.g. `"RU"`.
    pub language: &'static str,
    pub bracket_types: Vec<BracketType>,
    pub bracket_tags: Vec<&'static str>,
    pub style_tags: Vec<&'static str>,
    pub foreign_style_tag: Option<&'static str>,
    pub mis_keyword: &'static str,
    pub pro_keyword: &'static str,
    pub unintelligible_tag: &'static str,
    pub min_distance_seconds: f64,
    checks: Vec<EnabledRule>,
    bracket_tag_set: HashSet<&'static str>,
    style_tag_set: HashSet<&'static str>,
    distance_reason: String,
    patterns: OnceLock<Patterns>,
}

pub(crate) struct GoldAnnotation<'a> {
    pub id: &'a str,
    pub text: &'a str,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub track_id: &'a str,
    /// Studio's imported consensus data: the segment still has unresolved low-confidence words.
    pub unresolved_low_confidence: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GoldIssue {
    pub annotation_id: String,
    pub rule_id: &'static str,
    pub reason: String,
    pub severity: LintSeverity,
}

/// Every language Gold has a configuration for.
pub(crate) fn gold_languages() -> &'static [GoldLanguage] {
    &LANGUAGES
}

/// The Gold configuration for a Babel language code (ASCII case-insensitive).
pub(crate) fn gold_language(code: &str) -> Option<&'static GoldLanguage> {
    LANGUAGES
        .iter()
        .find(|language| language.language.eq_ignore_ascii_case(code))
}

/// Babel `lintAnnotations`: every per-annotation rule for each annotation in input order, then the
/// per-track rules over each track's time-sorted neighbours, then the transcript-wide rules.
pub(crate) fn lint_annotations(
    language: &GoldLanguage,
    annotations: &[GoldAnnotation<'_>],
) -> Vec<GoldIssue> {
    let mut out = Vec::new();
    for annotation in annotations {
        for rule in &language.checks {
            if let Check::Annotation(check) = rule.check
                && let Some(reason) = check(language, annotation.text)
            {
                rule.issues(&mut out).push(annotation.id, reason);
            }
        }
    }

    if language
        .checks
        .iter()
        .any(|rule| matches!(rule.check, Check::Pair(_)))
    {
        let mut track_index = HashMap::new();
        let mut tracks: Vec<Vec<&GoldAnnotation<'_>>> = Vec::new();
        for annotation in annotations {
            let index = *track_index.entry(annotation.track_id).or_insert_with(|| {
                tracks.push(Vec::new());
                tracks.len() - 1
            });
            tracks[index].push(annotation);
        }
        for track in &mut tracks {
            track.sort_by(|left, right| {
                left.start_seconds
                    .partial_cmp(&right.start_seconds)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            for pair in track.windows(2) {
                for rule in &language.checks {
                    if let Check::Pair(check) = rule.check {
                        check(language, pair[0], pair[1], &mut rule.issues(&mut out));
                    }
                }
            }
        }
    }

    for rule in &language.checks {
        if let Check::Global(check) = rule.check {
            check(language, annotations, &mut rule.issues(&mut out));
        }
    }
    out
}

type Reason = Option<Cow<'static, str>>;

#[derive(Clone, Copy)]
enum Check {
    Annotation(fn(&GoldLanguage, &str) -> Reason),
    Pair(fn(&GoldLanguage, &GoldAnnotation<'_>, &GoldAnnotation<'_>, &mut Issues<'_>)),
    Global(fn(&GoldLanguage, &[GoldAnnotation<'_>], &mut Issues<'_>)),
}

/// Every Babel rule enabled for gold-non-bg in some language, by id.
const RULES: [(&str, Check); 45] = [
    ("empty-content", Check::Annotation(spacing::empty_content)),
    (
        "multiple-asterisks",
        Check::Annotation(emphasis::multiple_asterisks),
    ),
    (
        "unpaired-asterisks",
        Check::Annotation(emphasis::unpaired_asterisks),
    ),
    (
        "empty-emphasis",
        Check::Annotation(emphasis::empty_emphasis),
    ),
    (
        "punctuation-in-emphasis",
        Check::Annotation(emphasis::punctuation_in_emphasis),
    ),
    (
        "spaces-in-emphasis",
        Check::Annotation(emphasis::spaces_in_emphasis),
    ),
    (
        "bracket-validation",
        Check::Annotation(brackets::bracket_validation),
    ),
    (
        "style-tag-validation",
        Check::Annotation(style_tags::style_tag_validation),
    ),
    (
        "curly-bracket-validation",
        Check::Annotation(brackets::curly_bracket_validation),
    ),
    ("double-spaces", Check::Annotation(spacing::double_spaces)),
    (
        "nss-tag-spacing",
        Check::Annotation(brackets::nss_tag_spacing),
    ),
    (
        "cjk-half-width-space",
        Check::Annotation(cjk::cjk_half_width_space),
    ),
    (
        "leading-trailing-spaces",
        Check::Annotation(spacing::leading_trailing_spaces),
    ),
    (
        "overlapping-segments",
        Check::Pair(transcript::overlapping_segments),
    ),
    (
        "distance-between-annotations",
        Check::Pair(transcript::distance_between_annotations),
    ),
    ("triple-dash", Check::Annotation(dashes::triple_dash)),
    (
        "spaced-double-dash",
        Check::Annotation(dashes::spaced_double_dash),
    ),
    (
        "double-dash-punctuation",
        Check::Annotation(dashes::double_dash_punctuation),
    ),
    (
        "double-dash-missing-space",
        Check::Annotation(dashes::double_dash_missing_space),
    ),
    (
        "single-dash-missing-space",
        Check::Annotation(dashes::single_dash_missing_space),
    ),
    (
        "filler-word-spelling",
        Check::Annotation(heuristics::filler_word_spelling),
    ),
    (
        "unresolved-low-confidence",
        Check::Global(transcript::unresolved_low_confidence),
    ),
    (
        "newline-characters",
        Check::Annotation(spacing::newline_characters),
    ),
    (
        "consecutive-punctuation",
        Check::Annotation(heuristics::consecutive_punctuation),
    ),
    (
        "intra-segment-duplicate",
        Check::Annotation(heuristics::intra_segment_duplicate),
    ),
    (
        "duplicated-segments",
        Check::Global(transcript::duplicated_segments),
    ),
    (
        "duplicated-segments-adjacent",
        Check::Global(transcript::duplicated_segments_adjacent),
    ),
    (
        "unbracketed-annotation-tags",
        Check::Annotation(curly_tags::unbracketed_annotation_tags),
    ),
    (
        "malformed-curly-bracket-tags",
        Check::Annotation(curly_tags::malformed_curly_bracket_tags),
    ),
    (
        "curly-tag-spacing",
        Check::Annotation(curly_tags::curly_tag_spacing),
    ),
    (
        "arabic-foreign-tag-marker",
        Check::Annotation(curly_tags::arabic_foreign_tag_marker),
    ),
    (
        "pro-tag-numerals",
        Check::Annotation(curly_tags::pro_tag_numerals),
    ),
    (
        "full-width-punctuation",
        Check::Annotation(cjk::full_width_punctuation),
    ),
    (
        "traditional-chinese",
        Check::Annotation(cjk::traditional_chinese),
    ),
    (
        "banned-characters",
        Check::Annotation(heuristics::banned_characters),
    ),
    ("emoji", Check::Annotation(heuristics::emoji)),
    (
        "consecutive-capitals",
        Check::Annotation(heuristics::consecutive_capitals),
    ),
    (
        "consecutive-chars",
        Check::Annotation(heuristics::consecutive_chars),
    ),
    (
        "excessive-digits",
        Check::Annotation(heuristics::excessive_digits),
    ),
    ("banned-words", Check::Annotation(heuristics::banned_words)),
    ("em-dash", Check::Annotation(dashes::em_dash)),
    (
        "parentheses-usage",
        Check::Annotation(heuristics::parentheses_usage),
    ),
    (
        "unpaired-quotes",
        Check::Annotation(heuristics::unpaired_quotes),
    ),
    (
        "tag-only-punctuation",
        Check::Annotation(brackets::tag_only_punctuation),
    ),
    (
        "foreign-tag-consistency",
        Check::Global(foreign::foreign_tag_consistency),
    ),
];

struct EnabledRule {
    id: &'static str,
    severity: LintSeverity,
    check: Check,
}

impl EnabledRule {
    fn issues<'o>(&self, out: &'o mut Vec<GoldIssue>) -> Issues<'o> {
        Issues {
            out,
            rule_id: self.id,
            severity: self.severity,
        }
    }
}

/// Issue sink for one rule.
struct Issues<'o> {
    out: &'o mut Vec<GoldIssue>,
    rule_id: &'static str,
    severity: LintSeverity,
}

impl Issues<'_> {
    fn push(&mut self, annotation_id: &str, reason: impl Into<String>) {
        self.out.push(GoldIssue {
            annotation_id: annotation_id.to_owned(),
            rule_id: self.rule_id,
            reason: reason.into(),
            severity: self.severity,
        });
    }
}

#[derive(Deserialize)]
struct LintData {
    registry: Vec<RegistryEntry>,
    reasons: Reasons,
    languages: Vec<LanguageData>,
    hyphenated_words: HashSet<Box<str>>,
    language_codes: HashSet<Box<str>>,
    filler_corrections: HashMap<Box<str>, Box<str>>,
    filler_suggestions: HashMap<Box<str>, Box<str>>,
    capitals_exempt_languages: Vec<Box<str>>,
    banned_words: HashMap<Box<str>, Vec<BannedWord>>,
    terminal_punctuation: HashMap<Box<str>, Vec<Box<str>>>,
    default_terminal_punctuation: Vec<Box<str>>,
    comma_by_language: HashMap<Box<str>, Box<str>>,
    traditional_chinese: Box<str>,
}

#[derive(Deserialize)]
struct RegistryEntry {
    id: Box<str>,
    /// Checked against the port's rule kinds by the tests.
    #[cfg(test)]
    #[serde(rename = "type")]
    kind: Box<str>,
    severity: Box<str>,
}

/// Babel's fixed rule reasons (`e3`).
#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Reasons {
    empty_content: Box<str>,
    double_spaces: Box<str>,
    leading_trailing_spaces: Box<str>,
    nss_tag_spacing: Box<str>,
    cjk_half_width_space: Box<str>,
    multiple_asterisks: Box<str>,
    unpaired_asterisks: Box<str>,
    empty_emphasis: Box<str>,
    punctuation_inside_emphasis: Box<str>,
    spaces_inside_emphasis: Box<str>,
    invalid_bracket_tag: Box<str>,
    unclosed_bracket: Box<str>,
    empty_bracket: Box<str>,
    empty_curly_bracket: Box<str>,
    unclosed_curly_bracket: Box<str>,
    bare_curly_comment: Box<str>,
    curly_tag_spacing: Box<str>,
    bracket_spacing: Box<str>,
    overlapping_segments: Box<str>,
    style_tag_empty_name: Box<str>,
    style_tag_whitespace_in_syntax: Box<str>,
    style_tag_invalid_name: Box<str>,
    style_tag_unmatched: Box<str>,
    style_tag_empty: Box<str>,
    style_tag_spacing: Box<str>,
    style_tag_punctuation_outside: Box<str>,
    style_tag_stray_bracket: Box<str>,
    triple_dash: Box<str>,
    spaced_double_dash: Box<str>,
    double_dash_punctuation: Box<str>,
    double_dash_missing_space: Box<str>,
    single_dash_missing_space: Box<str>,
    unresolved_low_confidence: Box<str>,
    newline_characters: Box<str>,
    consecutive_punctuation: Box<str>,
    duplicated_segments: Box<str>,
    duplicated_segments_adjacent: Box<str>,
    arabic_foreign_tag_missing_lang_marker: Box<str>,
    full_width_punctuation: Box<str>,
    banned_characters: Box<str>,
    emoji: Box<str>,
    em_dash: Box<str>,
    pro_tag_numerals: Box<str>,
    tag_only_punctuation: Box<str>,
    parentheses: Box<str>,
    unpaired_quotes: Box<str>,
    consecutive_capitals: Box<str>,
    consecutive_chars: Box<str>,
    excessive_digits: Box<str>,
    banned_words: Box<str>,
}

#[derive(Deserialize)]
struct LanguageData {
    language: Box<str>,
    rules: Vec<Box<str>>,
    bracket_types: Vec<BracketData>,
    bracket_tags: Vec<Box<str>>,
    style_tags: Vec<Box<str>>,
    foreign_style_tag: Option<Box<str>>,
    mis_keyword: Box<str>,
    pro_keyword: Box<str>,
    unintelligible_tag: Box<str>,
    min_distance_seconds: f64,
}

#[derive(Deserialize)]
struct BracketData {
    open: Box<str>,
    close: Box<str>,
}

#[derive(Deserialize)]
struct BannedWord {
    word: Box<str>,
    romanization: Option<Box<str>>,
}

static DATA: LazyLock<LintData> = LazyLock::new(|| {
    serde_json::from_str(crate::data::LINT_DATA_JSON)
        .unwrap_or_else(|error| panic!("bundled babel-lint-data.json is invalid: {error}"))
});

static LANGUAGES: LazyLock<Vec<GoldLanguage>> = LazyLock::new(|| {
    let data = data();
    data.languages
        .iter()
        .map(|language| GoldLanguage::new(data, language))
        .collect()
});

fn data() -> &'static LintData {
    &DATA
}

fn reasons() -> &'static Reasons {
    &data().reasons
}

/// A fixed reason as a rule result.
fn fixed(reason: &'static str) -> Reason {
    Some(Cow::Borrowed(reason))
}

/// Babel `e4`: the distance rule's reason for a minimum gap in seconds.
fn distance_reason(seconds: f64) -> String {
    let milliseconds = 1000.0 * seconds;
    if milliseconds.fract() == 0.0 && milliseconds < 1000.0 {
        format!("Segments must be at least {milliseconds} ms apart.")
    } else {
        let plural = if seconds == 1.0 { "" } else { "s" };
        format!("Segments must be at least {seconds} second{plural} apart.")
    }
}

impl GoldLanguage {
    fn new(data: &'static LintData, raw: &'static LanguageData) -> Self {
        let checks = raw
            .rules
            .iter()
            .map(|id| {
                let check = RULES
                    .iter()
                    .find(|(rule, _)| *rule == &**id)
                    .map(|(_, check)| *check)
                    .unwrap_or_else(|| {
                        panic!("babel-lint-data.json enables Gold rule `{id}`, which has no port")
                    });
                let severity = data
                    .registry
                    .iter()
                    .find(|entry| entry.id == *id)
                    .map(|entry| severity(&entry.severity))
                    .unwrap_or_else(|| panic!("Gold rule `{id}` is missing from the registry"));
                EnabledRule {
                    id: rule_id(id),
                    severity,
                    check,
                }
            })
            .collect();
        let bracket_tags: Vec<&'static str> = raw.bracket_tags.iter().map(|tag| &**tag).collect();
        let style_tags: Vec<&'static str> = raw.style_tags.iter().map(|tag| &**tag).collect();
        Self {
            language: &raw.language,
            bracket_types: raw
                .bracket_types
                .iter()
                .map(|bracket| BracketType {
                    open: &bracket.open,
                    close: &bracket.close,
                })
                .collect(),
            bracket_tag_set: bracket_tags.iter().copied().collect(),
            style_tag_set: style_tags.iter().copied().collect(),
            bracket_tags,
            style_tags,
            foreign_style_tag: raw.foreign_style_tag.as_deref(),
            mis_keyword: &raw.mis_keyword,
            pro_keyword: &raw.pro_keyword,
            unintelligible_tag: &raw.unintelligible_tag,
            min_distance_seconds: raw.min_distance_seconds,
            checks,
            distance_reason: distance_reason(raw.min_distance_seconds),
            patterns: OnceLock::new(),
        }
    }

    fn is_arabic(&self) -> bool {
        self.language == "AR"
    }

    fn patterns(&self) -> &Patterns {
        self.patterns.get_or_init(|| Patterns::new(self))
    }
}

/// The `'static` id from the port table for a rule id read from the data.
fn rule_id(id: &str) -> &'static str {
    RULES
        .iter()
        .map(|(rule, _)| *rule)
        .find(|rule| *rule == id)
        .unwrap_or_else(|| panic!("unknown Gold rule `{id}`"))
}

fn severity(name: &str) -> LintSeverity {
    match name {
        "error" => LintSeverity::Error,
        "warning" => LintSeverity::Warning,
        other => panic!("unknown Gold severity `{other}`"),
    }
}

/// Language-specific regexes, compiled on the language's first lint.
struct Patterns {
    /// `(?<!\{)(KEYWORDS)\s*:`, case-insensitive.
    unbracketed_keyword: Regex,
    /// The unintelligible tag inside each bracket type, case-insensitive.
    bracketed_unintelligible: Vec<Regex>,
    /// The bare unintelligible tag, case-insensitive.
    unintelligible: Regex,
    unbracketed_reason: String,
    /// `{KEYWORD:` openings.
    curly_tag_opening: Regex,
    /// Babel's well-formed `{KEYWORD: word}` tag.
    curly_tag: Regex,
    /// Arabic's right-to-left `{word :KEYWORD}` tag.
    arabic_curly_tag: Regex,
    /// A `{PRO: …}` tag (either direction for Arabic).
    pro_tag: Regex,
    /// `FOREIGN="XX"` style tag names.
    coded_foreign_tag_name: Option<Regex>,
    /// A complete `<FOREIGN> … </FOREIGN>` block, capturing its content.
    foreign_block: Option<Regex>,
    /// An opening or closing foreign style tag with an optional `="XX"` code.
    foreign_tag: Option<Regex>,
    /// Each bracket type's `[…]` span.
    bracket_spans: Vec<Regex>,
    /// Terminal punctuation tokens plus the comma, in Babel's matching order.
    punctuation_tokens: Vec<&'static str>,
    /// Banned words and romanizations in Babel's report order, case-insensitive.
    banned_words: Vec<(&'static str, Regex)>,
}

impl Patterns {
    fn new(language: &GoldLanguage) -> Self {
        let data = data();
        let keywords = keyword_alternation(language);
        let unintelligible = regex::escape(language.unintelligible_tag);
        let word = format!("[^{{}}{ws}](?:[^{{}}]*[^{{}}{ws}])?", ws = ws_chars!());
        let pro = regex::escape(language.pro_keyword);
        let pro_tag = if language.is_arabic() {
            format!(r"\{{(?:{pro}{WS}*:[^}}]*|[^}}]*:{WS}*{pro})\}}")
        } else {
            format!(r"\{{{pro}{WS}*:[^}}]*\}}")
        };
        let foreign = language.foreign_style_tag.map(regex::escape);
        let code = language.language.to_ascii_lowercase();
        let mut punctuation_tokens: Vec<&'static str> = data
            .terminal_punctuation
            .get(language.language)
            .unwrap_or(&data.default_terminal_punctuation)
            .iter()
            .map(|token| &**token)
            .collect();
        punctuation_tokens.push(
            data.comma_by_language
                .get(language.language)
                .map_or(",", |comma| &**comma),
        );
        Self {
            unbracketed_keyword: regex(&format!(r"(?i)(?:^|[^{{]){keywords}{WS}*:")),
            bracketed_unintelligible: language
                .bracket_types
                .iter()
                .map(|bracket| {
                    regex(&format!(
                        "(?i){}{unintelligible}{}",
                        regex::escape(bracket.open),
                        regex::escape(bracket.close)
                    ))
                })
                .collect(),
            unintelligible: regex(&format!("(?i){unintelligible}")),
            unbracketed_reason: format!(
                "Annotation tag outside brackets. Use [{}], {{{}: word}} for mispronunciations, or {{{}: word}} for pronunciations.",
                language.unintelligible_tag, language.mis_keyword, language.pro_keyword
            ),
            curly_tag_opening: regex(&format!(r"\{{{keywords}:")),
            curly_tag: regex(&format!(r"^\{{{keywords}:{WS}*{word}\}}$")),
            arabic_curly_tag: regex(&format!(r"^\{{{word}{WS}*:{keywords}\}}$")),
            pro_tag: regex(&pro_tag),
            coded_foreign_tag_name: foreign
                .as_ref()
                .map(|tag| regex(&format!(r#"^{tag}="([A-Z]{{2}})"$"#))),
            foreign_block: foreign.as_ref().map(|tag| {
                regex(&format!(
                    r#"<{WS}*{tag}(?:="[A-Z]{{2}}")?{WS}*>((?s:.)*?)<{WS}*/{WS}*{tag}{WS}*>"#
                ))
            }),
            foreign_tag: foreign
                .as_ref()
                .map(|tag| regex(&format!(r#"<(/?){tag}(?:{WS}*={WS}*"([^"]*)")?{WS}*>"#))),
            bracket_spans: language
                .bracket_types
                .iter()
                .map(|bracket| {
                    let close = regex::escape(bracket.close);
                    regex(&format!(
                        "{}[^{close}]*{close}",
                        regex::escape(bracket.open)
                    ))
                })
                .collect(),
            punctuation_tokens,
            banned_words: data
                .banned_words
                .get(code.as_str())
                .into_iter()
                .flatten()
                .flat_map(|entry| std::iter::once(&entry.word).chain(&entry.romanization))
                .map(|word| (&**word, regex(&format!("(?i){}", regex::escape(word)))))
                .collect(),
        }
    }
}

/// Babel `cn`: the language's MIS and PRO keywords plus the English ones, as an alternation.
fn keyword_alternation(language: &GoldLanguage) -> String {
    let mut keywords: Vec<&str> = Vec::with_capacity(4);
    for keyword in [language.mis_keyword, language.pro_keyword, "MIS", "PRO"] {
        if !keywords.contains(&keyword) {
            keywords.push(keyword);
        }
    }
    let escaped: Vec<String> = keywords.into_iter().map(regex::escape).collect();
    format!("(?:{})", escaped.join("|"))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Deserialize)]
    struct Fixture {
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        name: String,
        language: String,
        segments: Vec<Segment>,
        gold: Vec<ExpectedIssue>,
    }

    #[derive(Deserialize)]
    struct Segment {
        id: String,
        text: String,
        start: f64,
        end: f64,
        track: String,
    }

    #[derive(Deserialize, Debug, PartialEq)]
    struct ExpectedIssue {
        id: String,
        reason: String,
        severity: String,
    }

    #[test]
    fn matches_babel_on_every_parity_case() {
        let Some(json) = crate::data::lint_parity_json("matches_babel_on_every_parity_case") else {
            return;
        };
        let fixture: Fixture = serde_json::from_str(&json).unwrap();
        let mut failures = Vec::new();
        for case in &fixture.cases {
            let language = gold_language(&case.language)
                .unwrap_or_else(|| panic!("{}: unknown language", case.name));
            let annotations: Vec<GoldAnnotation<'_>> = case
                .segments
                .iter()
                .map(|segment| GoldAnnotation {
                    id: &segment.id,
                    text: &segment.text,
                    start_seconds: segment.start,
                    end_seconds: segment.end,
                    track_id: &segment.track,
                    unresolved_low_confidence: false,
                })
                .collect();
            let actual: Vec<ExpectedIssue> = lint_annotations(language, &annotations)
                .into_iter()
                .map(|issue| ExpectedIssue {
                    id: issue.annotation_id,
                    reason: issue.reason,
                    severity: match issue.severity {
                        LintSeverity::Error => "error",
                        LintSeverity::Warning => "warning",
                        LintSeverity::Pass => "pass",
                    }
                    .to_owned(),
                })
                .collect();
            if actual != case.gold {
                failures.push(format!(
                    "{}\n  expected {:?}\n  actual   {:?}",
                    case.name, case.gold, actual
                ));
            }
        }
        assert!(
            failures.is_empty(),
            "{} of {} cases differ:\n{}",
            failures.len(),
            fixture.cases.len(),
            failures.join("\n")
        );
    }

    #[test]
    fn every_enabled_rule_has_a_port_of_the_registry_kind() {
        for language in &data().languages {
            for id in &language.rules {
                let (_, check) = RULES
                    .iter()
                    .find(|(rule, _)| *rule == &**id)
                    .unwrap_or_else(|| panic!("{}: rule `{id}` has no port", language.language));
                let entry = data()
                    .registry
                    .iter()
                    .find(|entry| entry.id == *id)
                    .unwrap();
                let kind = match check {
                    Check::Annotation(_) => "per-annotation",
                    Check::Pair(_) => "per-track",
                    Check::Global(_) => "global",
                };
                assert_eq!(kind, &*entry.kind, "rule `{id}` kind");
            }
        }
        for (id, _) in RULES {
            assert!(
                data()
                    .languages
                    .iter()
                    .any(|language| language.rules.iter().any(|rule| &**rule == id)),
                "ported rule `{id}` is not enabled for any language"
            );
        }
    }

    #[test]
    fn unresolved_low_confidence_flags_marked_annotations_in_input_order() {
        let language = gold_language("en").unwrap();
        let annotation = |id, unresolved_low_confidence| GoldAnnotation {
            id,
            text: "Fine.",
            start_seconds: 0.0,
            end_seconds: 1.0,
            track_id: id,
            unresolved_low_confidence,
        };
        let issues = lint_annotations(
            language,
            &[
                annotation("b", true),
                annotation("a", false),
                annotation("c", true),
            ],
        );
        let flagged: Vec<(&str, &str)> = issues
            .iter()
            .filter(|issue| issue.rule_id == "unresolved-low-confidence")
            .map(|issue| (issue.annotation_id.as_str(), issue.reason.as_str()))
            .collect();
        let reason = &*reasons().unresolved_low_confidence;
        assert_eq!(flagged, [("b", reason), ("c", reason)]);
    }
}
