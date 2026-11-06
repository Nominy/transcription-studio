use exaudio_extension::document::{
    DiagnosticCollection, DiagnosticProvider, DocumentEdit, Severity,
};
use exaudio_lint_basic::{BasicChecks, ID};
use studio_core::{Project, Segment};

fn segment(id: &str, lane: &str, text: &str, start: f64, end: f64) -> Segment {
    Segment {
        id: id.into(),
        source_annotation_id: None,
        speaker_id: format!("speaker-{lane}"),
        lane_id: lane.into(),
        start_seconds: start,
        end_seconds: end,
        text: text.into(),
        metadata: Default::default(),
    }
}
fn project(segments: Vec<Segment>) -> Project {
    Project {
        id: "test".into(),
        name: "Test".into(),
        speakers: vec![],
        lanes: vec![],
        segments,
        metadata: Default::default(),
    }
}
fn check(text: &str) -> DiagnosticCollection {
    BasicChecks::default().diagnose(&project(vec![segment("one", "a", text, 0.0, 1.0)]), 41)
}

#[test]
fn manifest_is_valid_and_provider_is_opt_in() {
    let manifest = exaudio_lint_basic::manifest().unwrap();
    manifest.validate().unwrap();
    manifest.check_host_compatibility().unwrap();
    assert_eq!(manifest.id, ID);
    assert!(manifest.modes.is_empty());
    assert!(
        exaudio_extension::document::DocumentEnvironment::local("test")
            .providers
            .diagnostics
            .is_empty()
    );
    let providers = exaudio_lint_basic::providers(exaudio_lint_basic::Language::En);
    assert_eq!(providers.diagnostics.len(), 1);
    assert_eq!(
        providers.diagnostics[0]
            .diagnose(&project(vec![]), 2)
            .source,
        ID
    );
}

#[test]
fn fixes_are_revision_bound_utf8_byte_ranges_not_character_offsets() {
    let result = check(" \té🙂  字\t ");
    assert_eq!(result.revision, 41);
    assert_eq!(result.diagnostics.len(), 3);
    let mut text = " \té🙂  字\t ".to_string();
    for issue in result.diagnostics.iter().rev() {
        assert_eq!(issue.source, ID);
        assert!(issue.rule.starts_with(&format!("{ID}.")));
        assert_eq!(issue.severity, Severity::Hint);
        assert_eq!(issue.fixes.len(), 1);
        let fix = &issue.fixes[0];
        assert_eq!(fix.expected_revision, 41);
        assert_eq!(fix.edits.len(), 1);
        let DocumentEdit::ReplaceText {
            segment_id,
            range: Some(range),
            text: replacement,
        } = &fix.edits[0]
        else {
            panic!("expected bounded text replacement")
        };
        assert_eq!(segment_id, "one");
        assert!(text.is_char_boundary(range.start));
        assert!(text.is_char_boundary(range.end));
        assert_eq!(Some(*range), issue.range);
        text.replace_range(range.start..range.end, replacement);
    }
    assert_eq!(text, "é🙂 字");
}

#[test]
fn unicode_spacing_and_punctuation_have_no_language_policy() {
    for text in [
        "\u{a0}mot\u{a0}",
        "mot\u{202f}!",
        "a\u{2009}\u{2009}b",
        "　字　",
        "Wait...",
        "えっ！？",
        "w-w-well?!",
        "？！……",
        "a b",
    ] {
        assert!(
            check(text).diagnostics.is_empty(),
            "unexpected finding for {text:?}"
        );
    }
}

#[test]
fn legitimate_linebreaks_and_indentation_are_not_collapsed() {
    for newline in ["\n", "\r\n", "\r", "\u{85}", "\u{2028}", "\u{2029}"] {
        let text = format!("one  {newline}\t  two{newline}three");
        assert!(
            check(&text).diagnostics.is_empty(),
            "unexpected finding for {text:?}"
        );
    }
}

#[test]
fn empty_transcripts_are_non_destructive_warnings() {
    for text in ["", "\n", "\u{a0}", "　"] {
        let result = check(text);
        assert_eq!(result.diagnostics.len(), 1);
        let issue = &result.diagnostics[0];
        assert_eq!(issue.rule, format!("{ID}.empty-transcript"));
        assert_eq!(issue.severity, Severity::Warning);
        assert!(issue.fixes.is_empty());
    }
}

#[test]
fn invalid_timing_is_error_without_guessing_a_repair() {
    for (start, end) in [
        (f64::NAN, 1.0),
        (0.0, f64::INFINITY),
        (f64::NEG_INFINITY, 1.0),
        (-1.0, 2.0),
        (0.0, -1.0),
        (2.0, 1.0),
    ] {
        let result = BasicChecks::default()
            .diagnose(&project(vec![segment("one", "a", "text", start, end)]), 0);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].severity, Severity::Error);
        assert!(result.diagnostics[0].fixes.is_empty());
    }
}

#[test]
fn overlap_is_lane_scoped_unsorted_nested_and_non_destructive() {
    let result = BasicChecks::default().diagnose(
        &project(vec![
            segment("nested", "a", "text", 2.0, 3.0),
            segment("other-speaker", "b", "text", 1.0, 9.0),
            segment("outer", "a", "text", 0.0, 10.0),
            segment("later-nested", "a", "text", 5.0, 6.0),
            segment("touching", "a", "text", 10.0, 11.0),
            segment("point", "a", "text", 3.0, 3.0),
        ]),
        3,
    );
    let ids: Vec<_> = result
        .diagnostics
        .iter()
        .map(|d| d.segment_id.as_str())
        .collect();
    assert_eq!(ids, ["nested", "outer", "later-nested"]);
    for issue in result.diagnostics {
        assert_eq!(issue.rule, format!("{ID}.same-lane-overlap"));
        assert_eq!(issue.severity, Severity::Warning);
        assert!(issue.fixes.is_empty());
    }
}

#[test]
fn different_speakers_on_same_lane_still_overlap() {
    let a = segment("one", "a", "one", 0.0, 2.0);
    let mut b = segment("two", "a", "two", 1.0, 3.0);
    b.speaker_id = "different".into();
    assert_eq!(
        BasicChecks::default()
            .diagnose(&project(vec![a, b]), 1)
            .diagnostics
            .len(),
        2
    );
}
