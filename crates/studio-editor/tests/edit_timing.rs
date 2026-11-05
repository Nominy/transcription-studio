//! Word-timing consumers on `EditorDoc`: smart split (Ctrl+Shift+Click, cut preview S),
//! Alt+Click word seek, the timed ghost cursor, Gold Draft row replacement / Apply draft /
//! Restore original and the empty-row-only segment transcription guards.
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_core::SegmentUpdate;
use studio_core::timing::{LaneTimings, TimingToken};
use studio_editor::edit::{
    DraftedRow, EditError, EditorAction, EditorDoc, Effect, PointerModifiers, TextSelection,
    TranscriptionRefusal, remove_word_final_ellipsis,
};
use studio_waveform::WaveformHitTarget;

fn install_timings(doc: &mut EditorDoc, lane: &str, words: &[(&str, f64, f64)]) {
    doc.install_word_timings([LaneTimings {
        lane_id: lane.to_string(),
        source_id: "pcm-sha".to_string(),
        tokens: words
            .iter()
            .map(|(text, start, end)| TimingToken {
                text: text.to_string(),
                start_seconds: *start,
                end_seconds: *end,
            })
            .collect(),
    }]);
}

#[test]
fn smart_split_puts_completed_words_left_using_timings() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 6.0, "Раз, два три. Четыре")]);
    // "три" ends long after the time-ratio midpoint would suggest.
    install_timings(
        &mut doc,
        "lane-1",
        &[
            ("раз", 0.1, 0.4),
            ("два", 0.5, 0.8),
            ("три", 1.0, 1.3),
            ("четыре", 5.0, 5.5),
        ],
    );

    ok(doc.smart_split_segment_at("a", 3.0));

    assert_eq!(text_of(&doc, "a"), "Раз, два три.");
    assert_eq!(text_of(&doc, "a__studio_1"), "Четыре");
}

#[test]
fn ctrl_shift_click_smart_splits_and_ctrl_click_keeps_ratio_split() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 6.0, "one two three four")]);
    install_timings(
        &mut doc,
        "lane-1",
        &[
            ("one", 0.1, 0.3),
            ("two", 0.4, 0.6),
            ("three", 0.7, 0.9),
            ("four", 5.0, 5.5),
        ],
    );
    doc.set_modifiers(PointerModifiers {
        shift: true,
        alt: false,
        secondary: true,
    });
    doc.hover_waveform("lane-1", hit(WaveformHitTarget::RegionBody, Some("a"), 3.0));
    ok(doc.press_waveform("lane-1"));
    assert_eq!(text_of(&doc, "a"), "one two three");
    assert_eq!(text_of(&doc, "a__studio_1"), "four");

    let mut plain = edit_support::doc(vec![seg("a", "lane-1", 0.0, 6.0, "one two three four")]);
    plain.set_modifiers(PointerModifiers {
        shift: false,
        alt: false,
        secondary: true,
    });
    plain.hover_waveform("lane-1", hit(WaveformHitTarget::RegionBody, Some("a"), 3.0));
    ok(plain.press_waveform("lane-1"));
    assert_eq!(text_of(&plain, "a"), "one two");
}

#[test]
fn smart_split_falls_back_to_word_ratio_without_timings() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 4.0, "one {two three} four")]);
    ok(doc.smart_split_segment_at("a", 2.0));
    assert_eq!(text_of(&doc, "a"), "one {two three}");
    assert_eq!(text_of(&doc, "a__studio_1"), "four");
}

#[test]
fn cut_preview_smart_split_uses_timings_at_the_cut_midpoint() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 8.0, "раз два три четыре")]);
    install_timings(
        &mut doc,
        "lane-1",
        &[
            ("раз", 0.2, 0.5),
            ("два", 0.6, 0.9),
            ("три", 1.0, 1.4),
            ("четыре", 7.0, 7.5),
        ],
    );
    let _ = doc.select_segment("a");
    set_overlay(&mut doc, "lane-1", 2.0, 6.0);

    ok(doc.commit_cut_preview(true));

    assert_eq!(text_of(&doc, "a"), "раз два три");
    assert_eq!(text_of(&doc, "a__studio_1"), "четыре");
}

#[test]
fn alt_click_seeks_to_the_clicked_word_and_noops_without_timings() {
    let mut doc = doc(vec![seg("a", "lane-1", 1.0, 5.0, "раз два три")]);
    let before = doc.session().playhead_seconds;
    let outcome = ok(doc.seek_to_word("a", 5));
    assert!(outcome.status.unwrap().contains("no word timestamps"));
    assert_eq!(doc.session().playhead_seconds, before);

    install_timings(&mut doc, "lane-1", &[("два", 2.25, 2.5), ("три", 3.0, 3.4)]);
    ok(doc.seek_to_word("a", 5));
    assert_eq!(doc.session().playhead_seconds, 2.25);
    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("a"));
}

#[test]
fn pausing_focuses_the_timed_ghost_cursor_not_a_proportional_estimate() {
    // "один" fills most of the row: the time ratio alone would put the caret on "два".
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 10.0, "один два три")]);
    install_timings(
        &mut doc,
        "lane-1",
        &[("один", 0.0, 8.0), ("два", 8.2, 8.6), ("три", 8.8, 9.5)],
    );
    doc.select_segment_without_seek("a");
    ok(doc.perform(EditorAction::PlayPause, None).unwrap());
    assert!(doc.session().is_playing);
    doc.session_mut().playhead_seconds = 6.0;
    let ghost = doc.ghost_cursor("a").unwrap().char_offset;
    assert!(
        ghost <= 4,
        "the ghost is inside the spoken word, got {ghost}"
    );

    let outcome = ok(doc.perform(EditorAction::PlayPause, None).unwrap());

    assert!(!doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "a".to_string(),
        caret: ghost,
    }));
}

#[test]
fn repeated_escape_keeps_the_ghost_and_caret_on_the_timed_word() {
    // "один" is spoken for most of the row, so the time ratio runs far ahead of the speech:
    // a proportional estimate would sit on "три"/"четыре" while "один" is still spoken.
    let text = "один два три четыре пять шесть";
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 10.0, text)]);
    install_timings(
        &mut doc,
        "lane-1",
        &[
            ("один", 0.0, 8.0),
            ("два", 8.1, 8.4),
            ("три", 8.5, 8.8),
            ("четыре", 8.9, 9.3),
            ("пять", 9.4, 9.6),
            ("шесть", 9.7, 9.9),
        ],
    );
    doc.select_segment_without_seek("a");
    // Space from the timeline starts playback with a ghost cursor at the row start.
    ok(doc.perform(EditorAction::PlayPause, None).unwrap());
    for playhead in [1.0, 2.5, 4.0, 5.5, 7.0] {
        doc.session_mut().playhead_seconds = playhead;
        let ghost = doc.ghost_cursor("a").unwrap().char_offset;
        assert!(
            ghost <= 4,
            "at {playhead}s the ghost stays on the spoken \"один\", got {ghost}"
        );
        // Escape while playing (row blurred): pauses and puts the caret on the ghost.
        let outcome = ok(doc.perform(EditorAction::EscapePressed, None).unwrap());
        assert!(!doc.session().is_playing);
        assert!(outcome.has_effect(&Effect::FocusText {
            segment_id: "a".to_string(),
            caret: ghost,
        }));
        // Escape in the focused row while paused: blurs, leaves a ghost and plays on.
        ok(doc
            .perform(EditorAction::TranscriptEscape, Some("a"))
            .unwrap());
        assert!(doc.session().is_playing);
    }
    doc.session_mut().playhead_seconds = 8.6;
    assert_eq!(
        doc.ghost_cursor("a").unwrap().char_offset,
        10,
        "inside \"три\" once it is spoken"
    );
}

#[test]
fn draft_rows_replace_timed_lanes_and_restore_needs_unchanged_structure() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "old one"),
        seg("b", "lane-1", 2.5, 4.0, "old two"),
        seg("c", "lane-2", 1.0, 3.0, "peer kept"),
    ]);

    let created = doc
        .replace_lane_rows(
            &["lane-1".to_string()],
            vec![
                DraftedRow {
                    lane_id: "lane-1".into(),
                    start_seconds: 0.2,
                    end_seconds: 1.8,
                    text: "new one".into(),
                },
                DraftedRow {
                    lane_id: "lane-1".into(),
                    start_seconds: 2.4,
                    end_seconds: 3.9,
                    text: "new two".into(),
                },
            ],
        )
        .unwrap();

    assert_eq!(created.len(), 2);
    assert!(!doc.project().has_segment("a") && !doc.project().has_segment("b"));
    assert_eq!(text_of(&doc, &created[0]), "new one");
    assert_eq!(text_of(&doc, "c"), "peer kept");

    // One undo step restores the whole lane.
    ok(doc.undo());
    assert_eq!(text_of(&doc, "a"), "old one");
    ok(doc.redo());

    let snapshot = doc.transcription_row_snapshot();
    ok(doc.edit_text(&created[0], "polished".into(), TextSelection::caret(8)));
    ok(doc.commit_pending_drafts("Apply"));
    ok(doc.restore_row_texts(&snapshot));
    assert_eq!(text_of(&doc, &created[0]), "new one");

    ok(doc.split_segment_at(&created[1], 3.0));
    assert!(matches!(
        doc.restore_row_texts(&snapshot),
        Err(EditError::Unavailable(_))
    ));
}

#[test]
fn an_applied_draft_shows_in_its_rows_and_a_later_commit_keeps_it() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "draft one"),
        seg("b", "lane-1", 2.5, 4.0, "draft two"),
    ]);
    ok(doc.edit_text("b", "typed two".into(), TextSelection::caret(9)));

    doc.edit_workflow("Apply draft", |workflow| {
        workflow.apply_segment_edits(vec![SegmentUpdate {
            segment_id: "a".into(),
            start_seconds: 0.0,
            end_seconds: 2.0,
            text: "polished one".into(),
        }])
    })
    .unwrap();

    assert_eq!(doc.draft("a").unwrap().text(), "polished one");
    // An unapplied edit of another row survives the outside edit.
    assert_eq!(doc.draft("b").unwrap().text(), "typed two");
    ok(doc.commit_pending_drafts("Gold Draft"));
    assert_eq!(text_of(&doc, "a"), "polished one");
    assert_eq!(text_of(&doc, "b"), "typed two");
}

#[test]
fn segment_transcription_only_fills_a_still_current_still_empty_row() {
    let mut doc = doc(vec![
        seg("full", "lane-1", 0.0, 1.0, "text"),
        seg("empty", "lane-1", 2.0, 3.0, ""),
        seg("other", "lane-2", 2.0, 3.0, ""),
    ]);
    assert_eq!(
        doc.transcription_target(Some("full")),
        Err(TranscriptionRefusal::NotEmpty)
    );
    let target = doc.transcription_target(Some("empty")).unwrap();

    assert_eq!(
        doc.apply_segment_transcription(&target, Some("other"), "привет"),
        Err(TranscriptionRefusal::NoLongerCurrent)
    );
    ok(doc.edit_text("empty", "typed".into(), TextSelection::caret(5)));
    assert_eq!(
        doc.apply_segment_transcription(&target, Some("empty"), "привет"),
        Err(TranscriptionRefusal::NoLongerEmpty)
    );
    ok(doc.edit_text("empty", String::new(), TextSelection::caret(0)));
    doc.apply_segment_transcription(&target, Some("empty"), "  ...привет мир ")
        .unwrap();
    assert_eq!(text_of(&doc, "empty"), "...Привет мир");

    let mut resized = edit_support::doc(vec![seg("empty", "lane-1", 2.0, 3.0, "")]);
    let target = resized.transcription_target(Some("empty")).unwrap();
    ok(resized.split_segment_at("empty", 2.5));
    assert_eq!(
        resized.apply_segment_transcription(&target, Some("empty"), "текст"),
        Err(TranscriptionRefusal::BoundariesChanged)
    );
}

/// Gold's rule for CTC draft rows and refined rows: an ellipsis glued to the end of a word is
/// dropped; a leading one (before the next word) or a free-standing one stays.
#[test]
fn word_final_ellipsis_is_removed_only_after_a_word() {
    assert_eq!(
        remove_word_final_ellipsis("Ну... так вот… 10… ...и всё ... да"),
        "Ну так вот 10 ...и всё ... да"
    );
    assert_eq!(remove_word_final_ellipsis("wait...no"), "wait...no");
    assert_eq!(remove_word_final_ellipsis("so......"), "so...");
    assert_eq!(remove_word_final_ellipsis("hm…?"), "hm?");
}
