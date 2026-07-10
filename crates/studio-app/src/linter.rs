mod low_confidence;
mod tags;
mod text_hygiene;
mod timing;

use studio_core::{Project, Segment};

use crate::{BabelProjectLintTagConfig, resolve_babel_project_lint_tag_config};
use low_confidence::{LOW_CONFIDENCE_REASON, has_unresolved_low_confidence_word};
use tags::{
    TAG_ONLY_PUNCTUATION_REASON, UNBRACKETED_TAG_REASON, has_tag_only_punctuation,
    has_unbracketed_annotation_tag_with_tags, malformed_curly_annotation_tag_with_tags,
    validate_curly_brackets, validate_square_brackets_with_tags, validate_style_tags_with_tags,
};
use text_hygiene::{
    BANNED_CHARACTERS_REASON, BANNED_WORDS_REASON, CONSECUTIVE_CAPITALS_REASON,
    CONSECUTIVE_CHARS_REASON, CONSECUTIVE_PUNCTUATION_REASON, DOUBLE_DASH_MISSING_SPACE_REASON,
    DOUBLE_DASH_PUNCTUATION_REASON, EM_DASH_REASON, EMPTY_EMPHASIS_REASON, EXCESSIVE_DIGITS_REASON,
    INTRA_SEGMENT_DUPLICATE_SUBSTRING_LENGTH, MULTIPLE_ASTERISKS_REASON, NEWLINE_CHARACTERS_REASON,
    PARENTHESES_REASON, PUNCTUATION_INSIDE_EMPHASIS_REASON, SPACED_DOUBLE_DASH_REASON,
    TRIPLE_DASH_REASON, UNPAIRED_ASTERISKS_REASON, UNPAIRED_QUOTES_REASON,
    banned_characters_in_text, banned_words_in_text, consecutive_capitals_in_text,
    consecutive_chars_in_text, emoji_in_text, excessive_digits_in_text,
    has_consecutive_punctuation, has_double_dash_missing_space, has_double_dash_punctuation,
    has_em_dash, has_empty_emphasis, has_multiple_asterisks, has_newline, has_parentheses,
    has_punctuation_inside_emphasis, has_spaced_double_dash, has_unpaired_asterisk,
    has_unpaired_quotes, intra_segment_duplicate_in_text, unsupported_unicode_in_text,
};
use timing::{
    DISTANCE_BETWEEN_SEGMENTS_REASON, OVERLAPPING_SEGMENTS_REASON, has_too_close_next_segment,
    overlaps_same_lane, pairwise_overlap_issue_count,
};

pub use tags::{RUSSIAN_SQUARE_BRACKET_TAGS, RUSSIAN_STYLE_TAGS};

const EMPTY_CONTENT_REASON: &str = "Segment cannot have empty text.";
const DOUBLE_SPACES_REASON: &str = "Double spaces are not allowed in text.";
const LEADING_TRAILING_SPACES_REASON: &str =
    "Extra spaces at the end or beginning of segments are not allowed.";
const NO_ANGLE_BRACKETS_REASON: &str =
    "Angle brackets <> are not allowed in silver tier transcriptions.";
const NO_ASTERISKS_REASON: &str =
    "Asterisks for emphasis are not needed for silver tier transcriptions.";
const NO_CURLY_BRACKETS_REASON: &str = "Silver tier does not use {PRO:} or {MIS:} tags.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintStatus {
    pub severity: LintSeverity,
    pub message: String,
    pub kind: LintKind,
}

impl LintStatus {
    pub fn pass(message: impl Into<String>) -> Self {
        Self {
            severity: LintSeverity::Pass,
            message: message.into(),
            kind: LintKind::Pass,
        }
    }

    fn error(kind: LintKind, message: impl Into<String>) -> Self {
        Self {
            severity: LintSeverity::Error,
            message: message.into(),
            kind,
        }
    }

    fn warning(kind: LintKind, message: impl Into<String>) -> Self {
        Self {
            severity: LintSeverity::Warning,
            message: message.into(),
            kind,
        }
    }

    pub fn is_assertable_warning(&self) -> bool {
        self.severity == LintSeverity::Warning && self.kind != LintKind::UnresolvedLowConfidence
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintSeverity {
    Pass,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLintIssue {
    pub annotation_id: String,
    pub reason: String,
    pub severity: LintSeverity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerLintSnapshot {
    pub revision: u64,
    pub issues: Vec<ServerLintIssue>,
}

impl ServerLintSnapshot {
    pub fn new(revision: u64, issues: Vec<ServerLintIssue>) -> Self {
        Self { revision, issues }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinterRuleType {
    PerAnnotation,
    PerTrack,
    Global,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinterRule {
    pub id: &'static str,
    pub rule_type: LinterRuleType,
    pub severity: LintSeverity,
}

const RECOVERED_LINTER_REGISTRY: &[LinterRule] = &[
    LinterRule {
        id: "empty-content",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "multiple-asterisks",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "unpaired-asterisks",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "empty-emphasis",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "punctuation-in-emphasis",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "bracket-validation",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "style-tag-validation",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "curly-bracket-validation",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "double-spaces",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "leading-trailing-spaces",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "overlapping-segments",
        rule_type: LinterRuleType::PerTrack,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "distance-between-annotations",
        rule_type: LinterRuleType::PerTrack,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "triple-dash",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "spaced-double-dash",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "double-dash-punctuation",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "double-dash-missing-space",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "unresolved-low-confidence",
        rule_type: LinterRuleType::Global,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "newline-characters",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "consecutive-punctuation",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "intra-segment-duplicate",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "duplicated-segments",
        rule_type: LinterRuleType::Global,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "duplicated-segments-adjacent",
        rule_type: LinterRuleType::Global,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "unbracketed-annotation-tags",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "malformed-curly-bracket-tags",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "banned-characters",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "emoji",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
    LinterRule {
        id: "consecutive-capitals",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "consecutive-chars",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "excessive-digits",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "banned-words",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "parentheses-usage",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "unpaired-quotes",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Warning,
    },
    LinterRule {
        id: "tag-only-punctuation",
        rule_type: LinterRuleType::PerAnnotation,
        severity: LintSeverity::Error,
    },
];

pub fn recovered_linter_registry() -> &'static [LinterRule] {
    RECOVERED_LINTER_REGISTRY
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LintKind {
    Pass,
    BabelServer,
    InvalidTimeRange,
    UnknownTrack,
    EmptyText,
    DoubleSpaces,
    LeadingTrailingSpaces,
    Overlap,
    DistanceBetweenSegments,
    MultipleAsterisks,
    UnpairedAsterisks,
    EmptyEmphasis,
    PunctuationInsideEmphasis,
    TripleDash,
    SpacedDoubleDash,
    DoubleDashPunctuation,
    DoubleDashMissingSpace,
    ConsecutivePunctuation,
    BannedCharacters,
    Emoji,
    EmDash,
    UnsupportedUnicode,
    ConsecutiveCapitals,
    ConsecutiveChars,
    ExcessiveDigits,
    BannedWords,
    Parentheses,
    UnpairedQuotes,
    BracketValidation,
    StyleTagValidation,
    CurlyBracketValidation,
    MalformedCurlyBracketTag,
    NoAngleBrackets,
    NoAsterisks,
    NoCurlyBrackets,
    UnresolvedLowConfidence,
    NewlineCharacters,
    UnbracketedAnnotationTag,
    IntraSegmentDuplicate,
    DuplicatedSegments,
    DuplicatedSegmentsAdjacent,
    TagOnlyPunctuation,
    LongSegment,
}

#[cfg(test)]
pub(crate) fn lint_segment(project: &Project, segment: &Segment) -> LintStatus {
    lint_segment_issues(project, segment)
        .into_iter()
        .next()
        .unwrap_or_else(|| LintStatus::pass("OK"))
}

pub(crate) fn lint_segment_issues(project: &Project, segment: &Segment) -> Vec<LintStatus> {
    let config = resolve_babel_project_lint_tag_config(project);
    lint_segment_issues_with_config(project, segment, config)
}

pub(crate) fn server_lint_issues_for_segment(
    snapshot: Option<&ServerLintSnapshot>,
    segment: &Segment,
) -> Vec<LintStatus> {
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };

    snapshot
        .issues
        .iter()
        .filter(|issue| server_lint_issue_matches_segment(issue, segment))
        .map(|issue| LintStatus {
            severity: issue.severity,
            message: issue.reason.clone(),
            kind: LintKind::BabelServer,
        })
        .collect()
}

pub(crate) fn server_lint_issue_matches_segment(
    issue: &ServerLintIssue,
    segment: &Segment,
) -> bool {
    issue.annotation_id == segment.id
        || segment
            .source_annotation_id
            .as_deref()
            .is_some_and(|source_id| issue.annotation_id == source_id)
}

fn lint_segment_issues_with_config(
    project: &Project,
    segment: &Segment,
    config: BabelProjectLintTagConfig,
) -> Vec<LintStatus> {
    let mut issues = lint_identity_issues(project, segment);
    if config.rule_enabled("empty-content") && segment.text.trim().is_empty() {
        issues.push(LintStatus::error(LintKind::EmptyText, EMPTY_CONTENT_REASON));
    }
    if config.rule_enabled("multiple-asterisks") && has_multiple_asterisks(&segment.text) {
        issues.push(LintStatus::error(
            LintKind::MultipleAsterisks,
            MULTIPLE_ASTERISKS_REASON,
        ));
    }
    if config.rule_enabled("unpaired-asterisks") && has_unpaired_asterisk(&segment.text) {
        issues.push(LintStatus::error(
            LintKind::UnpairedAsterisks,
            UNPAIRED_ASTERISKS_REASON,
        ));
    }
    if config.rule_enabled("empty-emphasis") && has_empty_emphasis(&segment.text) {
        issues.push(LintStatus::error(
            LintKind::EmptyEmphasis,
            EMPTY_EMPHASIS_REASON,
        ));
    }
    if config.rule_enabled("punctuation-in-emphasis")
        && has_punctuation_inside_emphasis(&segment.text)
    {
        issues.push(LintStatus::error(
            LintKind::PunctuationInsideEmphasis,
            PUNCTUATION_INSIDE_EMPHASIS_REASON,
        ));
    }
    if config.rule_enabled("bracket-validation")
        && let Some(message) =
            validate_square_brackets_with_tags(&segment.text, config.bracket_tags)
    {
        issues.push(LintStatus::error(LintKind::BracketValidation, message));
    }
    if config.rule_enabled("style-tag-validation")
        && let Some(message) = validate_style_tags_with_tags(&segment.text, config.style_tags)
    {
        issues.push(LintStatus::error(LintKind::StyleTagValidation, message));
    }
    if config.rule_enabled("curly-bracket-validation")
        && let Some(message) = validate_curly_brackets(&segment.text)
    {
        issues.push(LintStatus::error(LintKind::CurlyBracketValidation, message));
    }
    if config.rule_enabled("double-spaces") && segment.text.contains("  ") {
        issues.push(LintStatus::error(
            LintKind::DoubleSpaces,
            DOUBLE_SPACES_REASON,
        ));
    }
    if config.rule_enabled("leading-trailing-spaces")
        && (segment.text != segment.text.trim_start() || segment.text != segment.text.trim_end())
    {
        issues.push(LintStatus::error(
            LintKind::LeadingTrailingSpaces,
            LEADING_TRAILING_SPACES_REASON,
        ));
    }
    if config.rule_enabled("overlapping-segments") {
        for _ in 0..pairwise_overlap_issue_count(project, segment) {
            issues.push(LintStatus::error(
                LintKind::Overlap,
                OVERLAPPING_SEGMENTS_REASON,
            ));
        }
    }
    if config.rule_enabled("distance-between-annotations")
        && has_too_close_next_segment(project, segment)
    {
        issues.push(LintStatus::error(
            LintKind::DistanceBetweenSegments,
            DISTANCE_BETWEEN_SEGMENTS_REASON,
        ));
    }
    if config.rule_enabled("triple-dash") && segment.text.contains("---") {
        issues.push(LintStatus::error(LintKind::TripleDash, TRIPLE_DASH_REASON));
    }
    if config.rule_enabled("spaced-double-dash") && has_spaced_double_dash(&segment.text) {
        issues.push(LintStatus::error(
            LintKind::SpacedDoubleDash,
            SPACED_DOUBLE_DASH_REASON,
        ));
    }
    if config.rule_enabled("double-dash-punctuation") && has_double_dash_punctuation(&segment.text)
    {
        issues.push(LintStatus::error(
            LintKind::DoubleDashPunctuation,
            DOUBLE_DASH_PUNCTUATION_REASON,
        ));
    }
    if config.rule_enabled("double-dash-missing-space")
        && has_double_dash_missing_space(&segment.text)
    {
        issues.push(LintStatus::error(
            LintKind::DoubleDashMissingSpace,
            DOUBLE_DASH_MISSING_SPACE_REASON,
        ));
    }
    if config.rule_enabled("no-angle-brackets")
        && (segment.text.contains('<') || segment.text.contains('>'))
    {
        issues.push(LintStatus::error(
            LintKind::NoAngleBrackets,
            NO_ANGLE_BRACKETS_REASON,
        ));
    }
    if config.rule_enabled("no-asterisks") && segment.text.contains('*') {
        issues.push(LintStatus::error(
            LintKind::NoAsterisks,
            NO_ASTERISKS_REASON,
        ));
    }
    if config.rule_enabled("no-curly-brackets")
        && (segment.text.contains('{') || segment.text.contains('}'))
    {
        issues.push(LintStatus::error(
            LintKind::NoCurlyBrackets,
            NO_CURLY_BRACKETS_REASON,
        ));
    }
    if config.rule_enabled("consecutive-punctuation") && has_consecutive_punctuation(&segment.text)
    {
        issues.push(LintStatus::warning(
            LintKind::ConsecutivePunctuation,
            CONSECUTIVE_PUNCTUATION_REASON,
        ));
    }
    if config.rule_enabled("intra-segment-duplicate")
        && let Some(duplicate) = intra_segment_duplicate_in_text(&segment.text)
    {
        issues.push(LintStatus::warning(
            LintKind::IntraSegmentDuplicate,
            format!(
                "Repeated {INTRA_SEGMENT_DUPLICATE_SUBSTRING_LENGTH}-char substring: {duplicate}"
            ),
        ));
    }
    if config.rule_enabled("duplicated-segments") && has_duplicated_long_segment(project, segment) {
        issues.push(LintStatus::warning(
            LintKind::DuplicatedSegments,
            "Exact duplicate of another segment in this transcript.",
        ));
    }
    if config.rule_enabled("duplicated-segments-adjacent")
        && has_adjacent_mid_length_duplicate(project, segment)
    {
        issues.push(LintStatus::warning(
            LintKind::DuplicatedSegmentsAdjacent,
            "Possible duplicate of the adjacent segment.",
        ));
    }
    if config.rule_enabled("banned-characters")
        && let Some(characters) = banned_characters_in_text(&segment.text)
    {
        issues.push(LintStatus::error(
            LintKind::BannedCharacters,
            format!("{BANNED_CHARACTERS_REASON} Found: {characters}"),
        ));
    }
    if config.rule_enabled("emoji")
        && let Some(emoji) = emoji_in_text(&segment.text)
    {
        issues.push(LintStatus::error(
            LintKind::Emoji,
            format!("Emoji are not allowed in transcriptions. Found: {emoji}"),
        ));
    }
    if config.rule_enabled("em-dash") && has_em_dash(&segment.text) {
        issues.push(LintStatus::error(LintKind::EmDash, EM_DASH_REASON));
    }
    if config.rule_enabled("unsupported-unicode")
        && let Some(characters) = unsupported_unicode_in_text(&segment.text)
    {
        issues.push(LintStatus::error(
            LintKind::UnsupportedUnicode,
            format!("Unsupported Unicode character(s) detected. Found: {characters}"),
        ));
    }
    if config.rule_enabled("consecutive-capitals")
        && let Some(matches) = consecutive_capitals_in_text(&segment.text)
    {
        issues.push(LintStatus::warning(
            LintKind::ConsecutiveCapitals,
            format!(
                "{CONSECUTIVE_CAPITALS_REASON} Found 5+ consecutive uppercase letters: {matches}"
            ),
        ));
    }
    if config.rule_enabled("consecutive-chars")
        && let Some(matches) = consecutive_chars_in_text(&segment.text)
    {
        issues.push(LintStatus::warning(
            LintKind::ConsecutiveChars,
            format!("{CONSECUTIVE_CHARS_REASON} Found 5+ consecutive identical chars: {matches}"),
        ));
    }
    if config.rule_enabled("excessive-digits")
        && let Some(matches) = excessive_digits_in_text(&segment.text)
    {
        issues.push(LintStatus::warning(
            LintKind::ExcessiveDigits,
            format!("{EXCESSIVE_DIGITS_REASON} Found 13+ digit sequence: {matches}"),
        ));
    }
    if config.rule_enabled("banned-words")
        && let Some(words) = banned_words_in_text(&segment.text)
    {
        issues.push(LintStatus::warning(
            LintKind::BannedWords,
            format!("{BANNED_WORDS_REASON} Found: {words}"),
        ));
    }
    if config.rule_enabled("parentheses-usage") && has_parentheses(&segment.text) {
        issues.push(LintStatus::warning(
            LintKind::Parentheses,
            PARENTHESES_REASON,
        ));
    }
    if config.rule_enabled("unpaired-quotes") && has_unpaired_quotes(&segment.text) {
        issues.push(LintStatus::warning(
            LintKind::UnpairedQuotes,
            UNPAIRED_QUOTES_REASON,
        ));
    }
    if config.rule_enabled("malformed-curly-bracket-tags")
        && let Some(tag) =
            malformed_curly_annotation_tag_with_tags(&segment.text, config.curly_tags)
    {
        issues.push(LintStatus::error(
            LintKind::MalformedCurlyBracketTag,
            format!(
                "Malformed or disallowed curly-bracket tag \"{tag}\". Use {{MIS: word}} or {{PRO: word}} — exact tag, single colon, no surrounding spaces, no nested tags."
            ),
        ));
    }
    if config.rule_enabled("unresolved-low-confidence")
        && has_unresolved_low_confidence_word(segment)
    {
        issues.push(LintStatus::warning(
            LintKind::UnresolvedLowConfidence,
            LOW_CONFIDENCE_REASON,
        ));
    }
    if config.rule_enabled("newline-characters") && has_newline(&segment.text) {
        issues.push(LintStatus::error(
            LintKind::NewlineCharacters,
            NEWLINE_CHARACTERS_REASON,
        ));
    }
    if config.rule_enabled("unbracketed-annotation-tags")
        && has_unbracketed_annotation_tag_with_tags(&segment.text, config.curly_tags)
    {
        issues.push(LintStatus::warning(
            LintKind::UnbracketedAnnotationTag,
            UNBRACKETED_TAG_REASON,
        ));
    }
    if config.rule_enabled("tag-only-punctuation") && has_tag_only_punctuation(&segment.text) {
        issues.push(LintStatus::error(
            LintKind::TagOnlyPunctuation,
            TAG_ONLY_PUNCTUATION_REASON,
        ));
    }
    if config.rule_enabled("long-segment") && segment.end_seconds - segment.start_seconds > 45.0 {
        issues.push(LintStatus::warning(LintKind::LongSegment, "long segment"));
    }

    issues
}

pub(crate) fn lint_region_issues(project: &Project, segment: &Segment) -> Vec<LintStatus> {
    let mut issues = lint_identity_issues(project, segment);
    if overlaps_same_lane(project, segment) {
        issues.push(LintStatus::error(
            LintKind::Overlap,
            OVERLAPPING_SEGMENTS_REASON,
        ));
    }

    issues
}

fn lint_identity_issues(project: &Project, segment: &Segment) -> Vec<LintStatus> {
    let mut issues = Vec::new();
    if !segment.start_seconds.is_finite()
        || !segment.end_seconds.is_finite()
        || segment.start_seconds >= segment.end_seconds
    {
        issues.push(LintStatus::error(
            LintKind::InvalidTimeRange,
            "start before end is required",
        ));
    }
    if !project.lanes.iter().any(|lane| lane.id == segment.lane_id) {
        issues.push(LintStatus::error(LintKind::UnknownTrack, "unknown track"));
    }

    issues
}

fn has_duplicated_long_segment(project: &Project, segment: &Segment) -> bool {
    let content = segment.text.trim();
    if content.len() < 30 {
        return false;
    }

    sorted_transcription_segments(project)
        .into_iter()
        .take_while(|candidate| candidate.id != segment.id)
        .any(|candidate| candidate.text.trim() == content)
}

fn has_adjacent_mid_length_duplicate(project: &Project, segment: &Segment) -> bool {
    let content = segment.text.trim();
    if content.len() < 20 || content.len() >= 30 {
        return false;
    }

    let sorted = sorted_transcription_segments(project);
    sorted.windows(2).any(|window| {
        let current = window[0];
        let next = window[1];
        next.id == segment.id && current.text.trim() == content
    })
}

fn sorted_transcription_segments(project: &Project) -> Vec<&Segment> {
    let mut segments = project
        .segments
        .iter()
        .filter(|segment| !segment.text.trim().is_empty() && segment.start_seconds.is_finite())
        .collect::<Vec<_>>();
    segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    segments
}

pub(crate) fn warning_key(segment: &Segment, lint: &LintStatus) -> Option<String> {
    lint.is_assertable_warning()
        .then(|| format!("{}::{}", segment.id, lint.message))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use studio_core::{AudioLane, Project, Segment, Speaker};

    use super::{LintKind, lint_segment, lint_segment_issues};

    #[test]
    fn linter_registry_matches_recovered_babel_rule_contract() {
        let registry = super::recovered_linter_registry()
            .iter()
            .map(|rule| (rule.id, rule.rule_type, rule.severity))
            .collect::<Vec<_>>();

        assert_eq!(
            registry,
            vec![
                (
                    "empty-content",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "multiple-asterisks",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "unpaired-asterisks",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "empty-emphasis",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "punctuation-in-emphasis",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "bracket-validation",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "style-tag-validation",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "curly-bracket-validation",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "double-spaces",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "leading-trailing-spaces",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "overlapping-segments",
                    super::LinterRuleType::PerTrack,
                    super::LintSeverity::Error
                ),
                (
                    "distance-between-annotations",
                    super::LinterRuleType::PerTrack,
                    super::LintSeverity::Error
                ),
                (
                    "triple-dash",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "spaced-double-dash",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "double-dash-punctuation",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "double-dash-missing-space",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "unresolved-low-confidence",
                    super::LinterRuleType::Global,
                    super::LintSeverity::Warning
                ),
                (
                    "newline-characters",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "consecutive-punctuation",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "intra-segment-duplicate",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "duplicated-segments",
                    super::LinterRuleType::Global,
                    super::LintSeverity::Warning
                ),
                (
                    "duplicated-segments-adjacent",
                    super::LinterRuleType::Global,
                    super::LintSeverity::Warning
                ),
                (
                    "unbracketed-annotation-tags",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "malformed-curly-bracket-tags",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "banned-characters",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "emoji",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
                (
                    "consecutive-capitals",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "consecutive-chars",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "excessive-digits",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "banned-words",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "parentheses-usage",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "unpaired-quotes",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Warning
                ),
                (
                    "tag-only-punctuation",
                    super::LinterRuleType::PerAnnotation,
                    super::LintSeverity::Error
                ),
            ]
        );
    }

    #[test]
    fn linter_accepts_known_babel_style_tag_pairs() {
        let lint = lint_text("<смех-в-речи> М-да, </смех-в-речи> на самом деле.");

        assert_eq!(lint.kind, LintKind::Pass);
    }

    #[test]
    fn linter_rejects_style_tags_without_surrounding_spaces() {
        let lint = lint_text("<смех-в-речи>М-да</смех-в-речи>, на самом деле.");

        assert_eq!(lint.kind, LintKind::StyleTagValidation);
        assert_eq!(
            lint.message,
            "Style tags must have spaces around them: 'word <tag> content </tag> word'"
        );
    }

    #[test]
    fn linter_rejects_unknown_babel_style_tags() {
        let lint = lint_text("<не-babel>да</не-babel>");

        assert_eq!(lint.kind, LintKind::StyleTagValidation);
    }

    #[test]
    fn linter_rejects_mismatched_babel_style_tag_pairs() {
        let lint = lint_text("<речь-нараспев>да</смех-в-речи>");

        assert_eq!(lint.kind, LintKind::StyleTagValidation);
    }

    #[test]
    fn linter_rejects_stray_babel_style_close_tags() {
        let lint = lint_text("да</смех-в-речи>");

        assert_eq!(lint.kind, LintKind::StyleTagValidation);
    }

    #[test]
    fn linter_rejects_unclosed_babel_style_tags() {
        let lint = lint_text("<смех-в-речи>да");

        assert_eq!(lint.kind, LintKind::StyleTagValidation);
    }

    #[test]
    fn linter_reports_recovered_babel_banned_character_error() {
        let lint = lint_text("word = word");

        assert_eq!(lint.kind, LintKind::BannedCharacters);
        assert_eq!(lint.severity, super::LintSeverity::Error);
        assert_eq!(lint.message, "Banned character(s) detected. Found: \"=\"");
    }

    #[test]
    fn linter_reports_recovered_babel_warning_hygiene_rules() {
        let cases = [
            (
                "ABCDE",
                LintKind::ConsecutiveCapitals,
                "Consecutive capital letters detected. Found 5+ consecutive uppercase letters: \"ABCDE\"",
            ),
            (
                "heyyyyy",
                LintKind::ConsecutiveChars,
                "Consecutive repeated character(s) detected. Found 5+ consecutive identical chars: \"yyyyy\"",
            ),
            (
                "1234567890123",
                LintKind::ExcessiveDigits,
                "Excessive digit sequence detected. Found 13+ digit sequence: \"1234567890123\"",
            ),
            (
                "word хуй word",
                LintKind::BannedWords,
                "Banned word(s) detected. Found: \"хуй\"",
            ),
        ];

        for (text, kind, message) in cases {
            let lint = lint_text(text);
            assert_eq!(lint.kind, kind);
            assert_eq!(lint.severity, super::LintSeverity::Warning);
            assert_eq!(lint.message, message);
        }
    }

    #[test]
    fn linter_marks_recovered_babel_soft_hygiene_rules_as_warnings() {
        for text in ["word,, word", "word (aside)", "word \"quote"] {
            let lint = lint_text(text);
            assert_eq!(lint.severity, super::LintSeverity::Warning);
        }
    }

    #[test]
    fn russian_gold_allows_gold_only_tags_that_russian_transcription_rejects() {
        let text = "[музыка] <подражание> да </подражание>";
        let gold_issues = lint_issues_with_project_metadata(
            text,
            &[(
                crate::META_BABEL_PROJECT_ID,
                "0bf29f0c-371a-506a-8b77-5fffd14e5762",
            )],
        );
        let transcription_kinds = lint_kinds_with_project_metadata(
            text,
            &[(
                crate::META_BABEL_PROJECT_ID,
                "d6031436-1fd5-4c79-9b8f-2a7452fc8c4b",
            )],
        );

        assert_eq!(gold_issues, Vec::new());
        assert!(transcription_kinds.contains(&LintKind::BracketValidation));
        assert!(transcription_kinds.contains(&LintKind::StyleTagValidation));
    }

    #[test]
    fn russian_silver_uses_silver_markup_rules_without_style_tag_validation() {
        for (text, expected_kind) in [
            ("<не-babel>да</не-babel>", LintKind::NoAngleBrackets),
            ("да *нет*", LintKind::NoAsterisks),
            ("{MIS: слово}", LintKind::NoCurlyBrackets),
        ] {
            let kinds = lint_kinds_with_project_metadata(
                text,
                &[(
                    crate::META_BABEL_PROJECT_ID,
                    "73f8e130-9fb4-5c17-b142-5a2e75d89b6d",
                )],
            );

            assert_eq!(kinds, vec![expected_kind]);
        }
    }

    fn lint_issues_with_project_metadata(
        text: &str,
        metadata: &[(&str, &str)],
    ) -> Vec<super::LintStatus> {
        let project = project_with_segment_text(text, metadata);

        lint_segment_issues(&project, &project.segments[0])
    }

    fn lint_kinds_with_project_metadata(text: &str, metadata: &[(&str, &str)]) -> Vec<LintKind> {
        lint_issues_with_project_metadata(text, metadata)
            .into_iter()
            .map(|issue| issue.kind)
            .collect()
    }

    fn lint_text(text: &str) -> super::LintStatus {
        let project = project_with_segment_text(text, &[]);

        lint_segment(&project, &project.segments[0])
    }

    fn project_with_segment_text(text: &str, metadata: &[(&str, &str)]) -> Project {
        Project {
            id: "task".to_string(),
            name: "Task".to_string(),
            speakers: vec![Speaker {
                id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
            }],
            lanes: vec![AudioLane {
                id: "lane-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
                source_path: "speaker_1.wav".to_string(),
            }],
            segments: vec![Segment {
                id: "segment-1".to_string(),
                source_annotation_id: None,
                speaker_id: "speaker-1".to_string(),
                lane_id: "lane-1".to_string(),
                start_seconds: 0.0,
                end_seconds: 1.0,
                text: text.to_string(),
                metadata: BTreeMap::new(),
            }],
            metadata: metadata
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        }
    }
}
