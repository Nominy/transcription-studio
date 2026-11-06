//! Timeline / transport / helper-focus behaviour of `EditorDoc`: decoded-audio trim,
//! auto-insert and auto-segmentation, cut preview commit/loop, transport ticks and playback
//! follow, helper focus + ghost cursor (Space/Escape/Tab), row and problem navigation, find,
//! speed/rewind/speaker workflow, timeline-default persistence, window unfocus and Escape
//! precedence.
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_editor::edit::draft::parse_draft_times;
use studio_editor::edit::find::FindSummary;
use studio_editor::edit::structure::AuditionKey;
use studio_editor::edit::{
    EditError, EditorAction, EditorDoc, Effect, Outcome, PointerModifiers, TextSelection,
};
use studio_editor::timeline_model::TimeRange;
use studio_editor::track_filter::TrackFilter;
use studio_waveform::WaveformHitTarget;

// ---- helpers ----------------------------------------------------------------------------

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 0.0001,
        "expected {expected}, got {actual}"
    );
}

fn selected(doc: &EditorDoc) -> Option<&str> {
    doc.session().selected_segment_id.as_deref()
}

fn pending_segment(doc: &EditorDoc) -> Option<&str> {
    doc.helper()
        .pending
        .as_ref()
        .map(|restore| restore.segment_id.as_str())
}

fn set_draft_with_caret_at_end(doc: &mut EditorDoc, segment_id: &str, text: &str) {
    let caret = text.chars().count();
    ok(doc.edit_text(segment_id, text.to_string(), TextSelection::caret(caret)));
}

fn set_playhead(doc: &mut EditorDoc, seconds: f64) {
    doc.session_mut().playhead_seconds = seconds;
}

fn perform(doc: &mut EditorDoc, action: EditorAction) -> Outcome {
    ok(doc
        .perform(action, None)
        .unwrap_or_else(|| panic!("{action:?} should be handled by the document")))
}

fn has_reveal(outcome: &Outcome) -> bool {
    outcome
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::RevealSegment { .. }))
}

fn segment_start(doc: &EditorDoc, segment_id: &str) -> f64 {
    doc.project().segment(segment_id).unwrap().start_seconds
}

/// Old `large_two_lane_project`: a long opening row on `lane-1`, then rows alternating
/// `lane-2`/`lane-1` every second from 21s, each 0.72s long.
fn large_two_lane_doc(segment_count: usize) -> EditorDoc {
    let mut segments = vec![seg(
        "annotation-0000",
        "lane-1",
        0.0,
        12.0,
        "large project opening segment keeps playback inside one selected row",
    )];
    for index in 1..segment_count {
        let lane = if index % 2 == 0 { "lane-1" } else { "lane-2" };
        let start = 20.0 + index as f64;
        segments.push(seg(
            &format!("annotation-{index:04}"),
            lane,
            start,
            start + 0.72,
            &format!("large project row {index} with unique transcript text"),
        ));
    }
    doc(segments)
}

// ---- helper focus navigation (Shift+Left/Right) ------------------------------------------

#[test]
fn focus_adjacent_segment_focuses_next_row_from_start_without_seeking() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-1");
    set_playhead(&mut doc, 42.0);

    let outcome = ok(doc.focus_adjacent_segment(1));

    assert_eq!(selected(&doc), Some("annotation-2"));
    assert_eq!(doc.session().playhead_seconds, 42.0);
    assert_eq!(
        doc.helper()
            .last
            .as_ref()
            .map(|restore| (restore.segment_id.as_str(), restore.restore_offset)),
        Some(("annotation-2", 0))
    );
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "annotation-2".to_string(),
        caret: 0,
    }));
    assert_eq!(
        doc.draft("annotation-2").unwrap().selection(),
        TextSelection::caret(0)
    );
}

#[test]
fn focus_adjacent_segment_refuses_at_transcript_edge() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 1.0, "first"),
        seg("b", "lane-1", 2.0, 3.0, "second"),
    ]);
    let _ = doc.select_segment_without_seek("b");

    let error = doc.focus_adjacent_segment(1).unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(selected(&doc), Some("b"));
}

// ---- cut preview ----------------------------------------------------------------------

#[test]
fn cut_preview_commit_smart_splits_middle_segment_and_clears_overlay() {
    let mut doc = doc(vec![seg(
        "annotation-1",
        "lane-1",
        1.0,
        6.0,
        "one two three four five six",
    )]);
    let _ = doc.select_segment("annotation-1");
    set_overlay(&mut doc, "lane-1", 2.5, 4.0);

    ok(doc.commit_cut_preview(true));

    let project = doc.project();
    let left = project.segment("annotation-1").unwrap();
    let right = project.segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.start_seconds, 1.0);
    assert_eq!(left.end_seconds, 2.5);
    assert_eq!(left.text, "one two three");
    assert_eq!(right.start_seconds, 4.0);
    assert_eq!(right.end_seconds, 6.0);
    assert_eq!(right.text, "four five six");
    assert!(doc.session().overlay.is_none());
    assert_eq!(selected(&doc), Some("annotation-1__studio_1"));
    assert!(doc.session().is_dirty);
}

#[test]
fn cut_preview_commit_cuts_the_lane_it_was_drawn_on_not_the_selected_rows() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 1.0, 6.0, "one two three four five six"),
        seg("b", "lane-2", 1.0, 6.0, "uno dos tres cuatro cinco seis"),
    ]);
    let _ = doc.select_segment("a");
    set_overlay(&mut doc, "lane-2", 2.5, 4.0);

    ok(doc.commit_cut_preview(true));

    let project = doc.project();
    let untouched = project.segment("a").unwrap();
    assert_eq!((untouched.start_seconds, untouched.end_seconds), (1.0, 6.0));
    let left = project.segment("b").unwrap();
    let right = project.segment("b__studio_1").unwrap();
    assert_eq!((left.start_seconds, left.end_seconds), (1.0, 2.5));
    assert_eq!((right.start_seconds, right.end_seconds), (4.0, 6.0));
    assert_eq!(left.text, "uno dos tres");
    assert_eq!(right.text, "cuatro cinco seis");
    assert_eq!(selected(&doc), Some("b__studio_1"));
}

#[test]
fn cut_preview_commit_without_overlay_is_unavailable() {
    let mut doc = doc(vec![seg("annotation-1", "lane-1", 1.0, 6.0, "text")]);
    let _ = doc.select_segment("annotation-1");

    let error = doc.commit_cut_preview(true).unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(ids(&doc), vec!["annotation-1".to_string()]);
}

#[test]
fn document_minimum_cut_policy_does_not_prevent_looping() {
    let mut doc = doc(vec![seg("annotation-1", "lane-1", 1.0, 6.0, "one two")]);
    doc.session_mut()
        .environment
        .policy
        .minimum_cut_milliseconds = 1000;
    let _ = doc.select_segment("annotation-1");
    set_overlay(&mut doc, "lane-1", 2.0, 2.3);
    let before = doc.project().segments.clone();

    let error = doc.commit_cut_preview(false).unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(doc.project().segments, before);
    assert!(doc.session().overlay.is_some());
    ok(doc.loop_cut_preview());
    assert!(
        doc.session()
            .overlay
            .as_ref()
            .is_some_and(|overlay| overlay.looping)
    );
}

#[test]
fn local_document_can_cut_less_than_one_second() {
    let mut doc = doc(vec![seg("annotation-1", "lane-1", 1.0, 6.0, "one two")]);
    set_overlay(&mut doc, "lane-1", 2.0, 2.3);
    ok(doc.commit_cut_preview(false));
    assert_eq!(doc.project().segments.len(), 2);
}

#[test]
fn cut_preview_commit_is_undone_in_one_step() {
    let mut doc = doc(vec![seg(
        "annotation-1",
        "lane-1",
        1.0,
        6.0,
        "one two three four",
    )]);
    let _ = doc.select_segment("annotation-1");
    set_overlay(&mut doc, "lane-1", 2.5, 4.0);
    let before = doc.project().segments.clone();

    ok(doc.commit_cut_preview(true));
    ok(doc.undo());

    assert_eq!(doc.project().segments, before);
}

#[test]
fn cut_preview_commit_is_refused_on_read_only_task() {
    let mut doc = read_only_doc(vec![seg("annotation-1", "lane-1", 1.0, 6.0, "text")]);
    let _ = doc.select_segment("annotation-1");
    set_overlay(&mut doc, "lane-1", 2.0, 4.0);

    let error = doc.commit_cut_preview(true).unwrap_err();

    assert!(matches!(error, EditError::ReadOnly { .. }));
    assert_eq!(ids(&doc), vec!["annotation-1".to_string()]);
}

fn set_viewport(doc: &mut EditorDoc, start_seconds: f64, end_seconds: f64) {
    doc.session_mut().viewport = TimeRange {
        start_seconds,
        end_seconds,
    };
}

fn assert_loop_range(doc: &EditorDoc, start_seconds: f64, end_seconds: f64) {
    let range = doc.session().loop_range().expect("the selection loops");
    assert_close(range.start_seconds, start_seconds);
    assert_close(range.end_seconds, end_seconds);
}

/// Drags on `lane-1` from `from` to `to` with the current modifiers.
fn drag_lane_1(doc: &mut EditorDoc, from: f64, to: f64) {
    doc.hover_waveform("lane-1", hit(WaveformHitTarget::Waveform, None, from));
    let _ = ok(doc.press_waveform("lane-1"));
    doc.hover_waveform("lane-1", hit(WaveformHitTarget::Waveform, None, to));
    let _ = ok(doc.release_waveform("lane-1"));
}

#[test]
fn l_loops_playback_over_the_selection_until_l_again() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    let before = doc.project().segments.clone();
    set_overlay(&mut doc, "lane-1", 2.0, 3.0);
    set_playhead(&mut doc, 0.0);

    perform(&mut doc, EditorAction::LoopCutPreviewOverlay);

    assert_loop_range(&doc, 2.0, 3.0);
    assert_close(doc.session().playhead_seconds, 2.0);
    assert_eq!(doc.project().segments, before);

    let _ = doc.toggle_playback();
    let speed = f64::from(doc.session().speed);
    set_playhead(&mut doc, 2.9);
    let outcome = doc.tick(0.2);
    assert!(doc.session().is_playing);
    assert_close(doc.session().playhead_seconds, 2.9 + 0.2 * speed - 1.0);
    assert!(outcome.has_effect(&Effect::RestartPlayback));

    let outcome = perform(&mut doc, EditorAction::LoopCutPreviewOverlay);

    assert_eq!(doc.session().loop_range(), None);
    assert_eq!(
        outcome.status.as_deref(),
        Some("Stopped looping the selection.")
    );
    assert_eq!(
        overlay_range(&doc).map(|range| range.end_seconds),
        Some(3.0)
    );
    set_playhead(&mut doc, 2.9);
    doc.tick(0.2);
    assert_close(doc.session().playhead_seconds, 2.9 + 0.2 * speed);
}

#[test]
fn resizing_or_moving_a_looping_selection_keeps_looping_over_the_new_range() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    set_viewport(&mut doc, 0.0, 10.0);
    set_overlay(&mut doc, "lane-1", 2.0, 4.0);
    perform(&mut doc, EditorAction::LoopCutPreviewOverlay);

    drag_lane_1(&mut doc, 4.0, 3.0);
    assert_loop_range(&doc, 2.0, 3.0);

    doc.set_modifiers(PointerModifiers {
        secondary: true,
        ..PointerModifiers::default()
    });
    drag_lane_1(&mut doc, 2.5, 4.5);
    doc.set_modifiers(PointerModifiers::default());
    assert_loop_range(&doc, 4.0, 5.0);

    // Playback runs into the moved range and wraps at its end.
    let _ = doc.toggle_playback();
    let speed = f64::from(doc.session().speed);
    set_playhead(&mut doc, 4.9);
    doc.tick(0.2);
    assert_close(doc.session().playhead_seconds, 4.9 + 0.2 * speed - 1.0);

    // A playhead the range shrank behind starts over at the loop start.
    set_playhead(&mut doc, 4.8);
    drag_lane_1(&mut doc, 5.0, 4.5);
    assert_loop_range(&doc, 4.0, 4.5);
    let outcome = doc.tick(0.1);
    assert_close(doc.session().playhead_seconds, 4.0);
    assert!(outcome.has_effect(&Effect::RestartPlayback));
}

#[test]
fn escape_cancels_a_looping_selection_and_playback_runs_on() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    set_overlay(&mut doc, "lane-1", 2.0, 3.0);
    perform(&mut doc, EditorAction::LoopCutPreviewOverlay);
    let _ = doc.toggle_playback();

    let outcome = perform(&mut doc, EditorAction::EscapePressed);

    assert!(doc.session().overlay.is_none());
    assert_eq!(doc.session().loop_range(), None);
    assert_eq!(
        outcome.status.as_deref(),
        Some("Selection cleared; stopped looping.")
    );
    assert!(
        doc.session().is_playing,
        "Escape only cancels the selection"
    );
    let speed = f64::from(doc.session().speed);
    set_playhead(&mut doc, 2.9);
    doc.tick(0.2);
    assert_close(doc.session().playhead_seconds, 2.9 + 0.2 * speed);
}

#[test]
fn a_newly_drawn_selection_starts_unlooped() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    set_viewport(&mut doc, 0.0, 10.0);
    set_overlay(&mut doc, "lane-1", 2.0, 3.0);
    perform(&mut doc, EditorAction::LoopCutPreviewOverlay);

    doc.set_modifiers(PointerModifiers {
        alt: true,
        ..PointerModifiers::default()
    });
    drag_lane_1(&mut doc, 5.0, 6.0);

    assert_eq!(
        overlay_range(&doc).map(|range| range.start_seconds),
        Some(5.0)
    );
    assert_eq!(doc.session().loop_range(), None);
}

// ---- decoded-audio timing -------------------------------------------------------------

#[test]
fn auto_segment_visible_silences_splits_long_silence_then_trims_segments() {
    let mut doc = doc(vec![seg(
        "annotation-1",
        "lane-1",
        1.0,
        4.0,
        "first island second island",
    )]);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(6.0, 100, &[(1.20, 1.60), (3.20, 3.60)]),
    );

    ok(doc.auto_segment_visible_silences());

    let mut lane_segments: Vec<_> = doc
        .project()
        .segments
        .iter()
        .filter(|segment| segment.lane_id == "lane-1")
        .collect();
    lane_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    assert_eq!(lane_segments.len(), 2);
    assert_eq!(lane_segments[0].id, "annotation-1");
    assert_eq!(lane_segments[1].id, "annotation-1__studio_1");
    assert_close(lane_segments[0].start_seconds, 1.195);
    assert_close(lane_segments[0].end_seconds, 1.595);
    assert_close(lane_segments[1].start_seconds, 3.195);
    assert_close(lane_segments[1].end_seconds, 3.595);
    assert_eq!(lane_segments[0].text, "first island");
    assert_eq!(lane_segments[1].text, "second island");
    assert_eq!(draft_text(&doc, "annotation-1"), "first island");
    assert_eq!(draft_text(&doc, "annotation-1__studio_1"), "second island");
    assert!(doc.session().is_dirty);
}

#[test]
fn auto_segment_with_nothing_to_change_is_unavailable() {
    // The row already hugs its only speech: no trim, merge, split, silent row or placement.
    let mut doc = doc(vec![seg("annotation-1", "lane-1", 1.195, 1.595, "speech")]);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(6.0, 100, &[(1.20, 1.60)]),
    );
    let before = doc.project().segments.clone();

    let error = doc.auto_segment_visible_silences().unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(doc.project().segments, before);
    assert!(!doc.can_undo());
}

/// A lane's words in time order, the way a reader sees the transcript.
fn lane_words(doc: &EditorDoc, lane: &str) -> Vec<String> {
    let mut rows: Vec<_> = doc
        .project()
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane)
        .collect();
    rows.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    rows.iter()
        .flat_map(|segment| segment.text.split_whitespace().map(str::to_string))
        .collect()
}

#[test]
fn auto_segment_merges_close_same_speaker_rows_before_splitting_on_silence() {
    // Two rows 0.3s apart whose shared speech has its pause inside the second row: Helper
    // merges them first, so the pause (not the old row boundary) decides the rows.
    let mut doc = doc(vec![
        seg("a", "lane-1", 1.0, 2.0, "Раз два."),
        seg("b", "lane-1", 2.3, 6.0, "Три четыре. Пять шесть."),
        seg("other", "lane-2", 2.0, 3.0, "Чужой текст."),
    ]);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(8.0, 100, &[(1.0, 3.5), (4.8, 6.0)]),
    );
    let words = lane_words(&doc, "lane-1");

    let outcome = ok(doc.auto_segment_visible_silences());

    let mut rows: Vec<_> = doc
        .project()
        .segments
        .iter()
        .filter(|segment| segment.lane_id == "lane-1")
        .collect();
    rows.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    assert_eq!(rows.len(), 2, "merged then split once: {rows:?}");
    assert!(rows[0].end_seconds <= 3.6 && rows[1].start_seconds >= 4.7);
    assert_eq!(
        lane_words(&doc, "lane-1"),
        words,
        "no word lost, added or reordered"
    );
    assert_eq!(text_of(&doc, "other"), "Чужой текст.");
    for row in &rows {
        assert_eq!(draft_text(&doc, &row.id), row.text);
    }
    assert!(
        outcome
            .status
            .as_deref()
            .is_some_and(|status| status.contains("merged 1"))
    );
}

#[test]
fn auto_segment_removes_silent_rows_and_stitches_their_text_onto_the_nearest_row() {
    // `stray` has no audio and sits 1.5s after `speech` (too far to merge): its text moves to
    // `speech` and the row goes. Text placement must not overwrite the stitched words.
    let mut doc = doc(vec![
        seg("speech", "lane-1", 1.0, 2.0, "hello there"),
        seg("stray", "lane-1", 3.5, 4.0, "stray words"),
    ]);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(6.0, 100, &[(1.0, 2.0)]),
    );

    ok(doc.auto_segment_visible_silences());

    assert_eq!(ids(&doc), vec!["speech".to_string()]);
    assert_eq!(text_of(&doc, "speech"), "hello there stray words");
    assert_eq!(draft_text(&doc, "speech"), "hello there stray words");
}

#[test]
fn auto_segment_stitches_an_earlier_silent_row_in_front_and_drops_empty_silent_rows() {
    let mut doc = doc(vec![
        seg("lead", "lane-1", 0.2, 0.6, "first"),
        seg("speech", "lane-1", 2.0, 3.0, "second third"),
        seg("empty", "lane-1", 5.0, 5.5, ""),
    ]);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(8.0, 100, &[(2.0, 3.0)]),
    );

    ok(doc.auto_segment_visible_silences());

    assert_eq!(ids(&doc), vec!["speech".to_string()]);
    assert_eq!(text_of(&doc, "speech"), "first second third");
}

#[test]
fn auto_segment_keeps_a_silent_row_with_text_and_no_same_speaker_neighbour() {
    let mut doc = doc(vec![
        seg("alone", "lane-1", 1.0, 2.0, "only words"),
        seg("other", "lane-2", 1.0, 2.0, "other speaker"),
    ]);
    install_audio(&mut doc, "lane-1", audio_with_speech(4.0, 100, &[]));
    install_audio(
        &mut doc,
        "lane-2",
        audio_with_speech(4.0, 100, &[(1.0, 2.0)]),
    );

    let outcome = ok(doc.auto_segment_visible_silences());

    assert_eq!(text_of(&doc, "alone"), "only words");
    assert_eq!(text_of(&doc, "other"), "other speaker");
    assert!(
        outcome
            .status
            .as_deref()
            .is_some_and(|status| status.contains("kept 1 silent segment"))
    );
}

#[test]
fn auto_insert_at_playhead_creates_padded_empty_segment_around_nearest_uncovered_speech() {
    let mut doc = stable_sample_doc();
    let original_count = doc.project().segments.len();
    set_playhead(&mut doc, 8.5);
    install_audio(
        &mut doc,
        "track-speaker-1",
        audio_with_speech(12.0, 100, &[(8.4, 8.8)]),
    );

    ok(doc.auto_insert_segment_at_playhead());

    let project = doc.project();
    assert_eq!(project.segments.len(), original_count + 1);
    let created = project
        .segments
        .iter()
        .find(|segment| segment.id.starts_with("track-speaker-1__studio_"))
        .expect("auto-insert should create a derived segment in the decoded lane");
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.speaker_id, "speaker-1");
    assert_eq!(created.text, "");
    assert_close(created.start_seconds, 8.35);
    assert_close(created.end_seconds, 8.85);
    assert_eq!(selected(&doc), Some(created.id.as_str()));
    assert_eq!(doc.session().playhead_seconds, 8.5);
    assert!(doc.session().is_dirty);
}

#[test]
fn auto_insert_at_playhead_skips_speech_already_covered_by_visible_row() {
    let mut doc = stable_sample_doc();
    let original_count = doc.project().segments.len();
    set_playhead(&mut doc, 2.0);
    install_audio(
        &mut doc,
        "track-speaker-1",
        audio_with_speech(8.0, 100, &[(1.8, 2.2)]),
    );

    let error = doc.auto_insert_segment_at_playhead().unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(doc.project().segments.len(), original_count);
}

#[test]
fn auto_insert_at_playhead_merges_adjacent_same_lane_rows_and_selects_result() {
    let mut doc = doc(vec![
        seg("annotation-1", "lane-1", 1.0, 1.5, "before"),
        seg("annotation-next", "lane-1", 2.5, 3.0, "after"),
    ]);
    set_playhead(&mut doc, 2.0);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(4.0, 100, &[(1.55, 2.45)]),
    );

    ok(doc.auto_insert_segment_at_playhead());

    let lane_segments: Vec<_> = doc
        .project()
        .segments
        .iter()
        .filter(|segment| segment.lane_id == "lane-1")
        .collect();
    assert_eq!(lane_segments.len(), 1);
    let merged = lane_segments[0];
    assert_eq!(merged.id, "lane-1__studio_1");
    assert_eq!(merged.start_seconds, 1.0);
    assert_eq!(merged.end_seconds, 3.0);
    assert_eq!(merged.text, "before after");
    assert_eq!(selected(&doc), Some("lane-1__studio_1"));
    assert_eq!(draft_text(&doc, "lane-1__studio_1"), "before after");
}

#[test]
fn trim_current_segment_to_audio_moves_boundaries_with_padding_and_refreshes_draft() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-1");
    install_audio(
        &mut doc,
        "track-speaker-1",
        audio_with_speech(10.0, 100, &[(1.50, 2.90)]),
    );

    ok(doc.trim_current_segment_to_audio());

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_close(segment.start_seconds, 1.495);
    assert_close(segment.end_seconds, 2.895);
    let (draft_start, draft_end) =
        parse_draft_times(segment, doc.draft("annotation-1").unwrap()).unwrap();
    assert_close(draft_start, segment.start_seconds);
    assert_close(draft_end, segment.end_seconds);
    assert!(!doc.has_pending_drafts());
    assert!(doc.session().is_dirty);
}

#[test]
fn trim_current_segment_extends_outward_then_trims_to_speech_edges() {
    let mut doc = doc(vec![seg(
        "annotation-1",
        "lane-1",
        2.0,
        3.0,
        "continuous speech",
    )]);
    let _ = doc.select_segment("annotation-1");
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(5.0, 1000, &[(1.397, 3.603)]),
    );

    ok(doc.trim_current_segment_to_audio());

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_close(segment.start_seconds, 1.392);
    assert_close(segment.end_seconds, 3.607);
    assert_eq!(segment.text, "continuous speech");
    assert!(doc.session().is_dirty);
}

#[test]
fn trim_current_segment_outward_pass_respects_same_lane_neighbor_guards() {
    let mut doc = doc(vec![
        seg("annotation-1", "lane-1", 2.0, 3.0, "middle"),
        seg("annotation-prev", "lane-1", 1.0, 1.35, "previous"),
        seg("annotation-next", "lane-1", 3.70, 4.0, "next"),
    ]);
    let _ = doc.select_segment("annotation-1");
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(5.0, 1000, &[(1.0, 4.2)]),
    );

    ok(doc.trim_current_segment_to_audio());

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_close(segment.start_seconds, 1.36);
    assert_close(segment.end_seconds, 3.69);
}

#[test]
fn trim_visible_segments_uses_decoded_audio_and_skips_lanes_without_audio() {
    let mut doc = stable_sample_doc();
    install_audio(
        &mut doc,
        "track-speaker-1",
        audio_with_speech(10.0, 100, &[(1.50, 2.90), (5.70, 7.05)]),
    );

    ok(doc.trim_visible_segments_to_audio());

    let project = doc.project();
    let first = project.segment("annotation-1").unwrap();
    let third = project.segment("annotation-3").unwrap();
    assert_close(first.start_seconds, 1.495);
    assert_close(first.end_seconds, 2.895);
    assert_close(third.start_seconds, 5.695);
    assert_close(third.end_seconds, 7.045);
    assert_eq!(
        project.segment("annotation-2").unwrap().start_seconds,
        3.5,
        "lanes without decoded audio are skipped"
    );
}

#[test]
fn trim_visible_segments_never_grows_two_neighbours_into_each_other() {
    // Speech runs through the gap: both rows' outward passes want the same span.
    let mut doc = doc(vec![
        seg("left", "lane-1", 1.0, 1.9, "раз"),
        seg("right", "lane-1", 2.1, 3.0, "два"),
    ]);
    install_audio(
        &mut doc,
        "lane-1",
        audio_with_speech(5.0, 1000, &[(0.5, 3.5)]),
    );

    ok(doc.trim_visible_segments_to_audio());

    let project = doc.project();
    let left = project.segment("left").unwrap();
    let right = project.segment("right").unwrap();
    assert!(
        left.end_seconds < right.start_seconds,
        "rows overlap: left ends {} after right starts {}",
        left.end_seconds,
        right.start_seconds
    );
}

// ---- window unfocus -------------------------------------------------------------------

#[test]
fn window_unfocus_cancels_captured_pointer_drags_and_audition_holds() {
    let mut doc = stable_sample_doc();
    doc.set_modifiers(PointerModifiers {
        secondary: true,
        ..PointerModifiers::default()
    });
    doc.hover_waveform(
        "track-speaker-2",
        hit(WaveformHitTarget::Waveform, None, 2.0),
    );
    doc.press_autoscroll("track-speaker-2");
    doc.set_audition_key(AuditionKey::BeforeStart, true);

    assert!(doc.pointer().autoscroll.is_some());
    assert!(
        doc.pointer()
            .held_audition_keys
            .contains(&AuditionKey::BeforeStart)
    );
    assert!(doc.pointer().modifiers.secondary);

    ok(doc.window_unfocused());

    assert!(doc.pointer().autoscroll.is_none());
    assert!(doc.pointer().held_audition_keys.is_empty());
    assert_eq!(doc.pointer().modifiers, PointerModifiers::default());
}

// ---- row navigation -------------------------------------------------------------------

#[test]
fn adjacent_segment_navigation_skips_collapsed_lanes() {
    let mut doc = stable_sample_doc();
    doc.toggle_lane_collapse("track-speaker-2");
    let _ = doc.select_segment("annotation-1");

    let _ = doc.select_adjacent_segment(1);

    let selected_id = selected(&doc).expect("a row stays selected").to_string();
    assert_eq!(
        doc.project().segment(&selected_id).unwrap().lane_id,
        "track-speaker-1"
    );
}

#[test]
fn adjacent_segment_navigation_seeks_to_timeline_bounds_past_first_and_last_rows() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-1");

    let _ = doc.select_adjacent_segment(-1);

    assert_eq!(doc.session().playhead_seconds, 0.0);
    assert_eq!(selected(&doc), None);

    let last_segment_id = doc
        .transcript_rows()
        .last()
        .expect("sample should contain rows")
        .segment_id
        .clone();
    let _ = doc.select_segment(&last_segment_id);
    let project_end = TimeRange::for_project(doc.project()).end_seconds;

    let _ = doc.select_adjacent_segment(1);

    assert_eq!(doc.session().playhead_seconds, project_end);
    assert_eq!(selected(&doc), None);
}

#[test]
fn selected_segment_navigation_recenters_next_row_even_when_visible() {
    let mut doc = large_two_lane_doc(80);
    let _ = doc.select_segment_without_seek("annotation-0018");
    let start = segment_start(&doc, "annotation-0018");
    set_playhead(&mut doc, start);

    let outcome = doc.select_adjacent_segment(1);

    assert_eq!(selected(&doc), Some("annotation-0019"));
    assert!(outcome.has_effect(&Effect::RevealSegment {
        segment_id: "annotation-0019".to_string(),
        center: true,
    }));
}

// ---- transport ------------------------------------------------------------------------

#[test]
fn tick_advances_playhead_only_while_playing_and_honours_speed() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-2");
    let _ = doc.set_speed(2.0);

    let _ = doc.tick(0.016);
    assert_eq!(doc.session().playhead_seconds, 3.5);
    assert!(!doc.needs_ticks());

    let outcome = doc.toggle_playback();
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert!(doc.needs_ticks());
    let _ = doc.tick(0.016);

    assert!((doc.session().playhead_seconds - 3.532).abs() < 1e-9);
}

#[test]
fn tick_uses_elapsed_frame_time() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-2");
    let _ = doc.toggle_playback();

    let _ = doc.tick(0.016);
    let first = doc.session().playhead_seconds;
    let _ = doc.tick(0.048);
    let second = doc.session().playhead_seconds;

    assert!((first - 3.516).abs() < 1e-9);
    assert!((second - 3.564).abs() < 1e-9);
    assert!(second - first < 0.05);
}

#[test]
fn playback_start_only_recenters_when_playhead_is_outside_or_past_midpoint() {
    let mut doc = stable_sample_doc();
    let viewport = TimeRange {
        start_seconds: 20.0,
        end_seconds: 25.0,
    };
    doc.session_mut().viewport = viewport;
    set_playhead(&mut doc, 21.0);

    let _ = doc.toggle_playback();
    assert_eq!(doc.session().viewport, viewport);

    let outcome = doc.toggle_playback();
    assert!(outcome.has_effect(&Effect::StopPlayback));
    doc.session_mut().viewport = viewport;
    set_playhead(&mut doc, 24.0);
    let _ = doc.toggle_playback();

    let viewport = doc.session().viewport;
    assert!((viewport.start_seconds - 21.5).abs() < 0.000_001);
    assert!((viewport.end_seconds - 26.5).abs() < 0.000_001);
}

#[test]
fn playback_follow_reveals_row_under_playhead_once_per_row() {
    let mut doc = large_two_lane_doc(300);
    let far_segment_id = doc.transcript_rows()[200].segment_id.clone();
    let _ = doc.select_segment(&far_segment_id);
    let _ = doc.toggle_playback();

    let first = doc.tick(0.016);

    assert!(first.has_effect(&Effect::RevealSegment {
        segment_id: far_segment_id.clone(),
        center: false,
    }));
    assert_eq!(selected(&doc), Some(far_segment_id.as_str()));

    let second = doc.tick(0.016);
    assert!(
        !has_reveal(&second),
        "follow must not re-reveal while playback stays within the same row"
    );
}

// ---- helper focus: Escape / Space / ghost cursor ------------------------------------------

#[test]
fn helper_focus_escape_workflow_matches_four_state_toggle() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment_without_seek("annotation-1");
    set_playhead(&mut doc, 0.30);
    doc.session_mut().is_playing = true;

    // Focused row + playing: Escape pauses only.
    let outcome = ok(doc.transcript_escape("annotation-1"));
    assert!(!doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StopPlayback));
    assert_eq!(selected(&doc), Some("annotation-1"));

    // Focused row + paused: Escape blurs, leaves a ghost cursor and resumes.
    let outcome = ok(doc.transcript_escape("annotation-1"));
    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::BlurText));
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert_eq!(pending_segment(&doc), Some("annotation-1"));

    // Unfocused + playing: Escape restores the caret into the row under the playhead.
    set_playhead(&mut doc, 3.75);
    set_draft_with_caret_at_end(&mut doc, "annotation-2", "a bcdefghi");
    let outcome = doc.escape();
    assert!(!doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StopPlayback));
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "annotation-2".to_string(),
        caret: 2,
    }));
    assert_eq!(selected(&doc), Some("annotation-2"));
    assert_eq!(
        doc.helper()
            .last
            .as_ref()
            .map(|restore| (restore.segment_id.as_str(), restore.restore_offset)),
        Some(("annotation-2", 2))
    );
    assert_eq!(
        doc.draft("annotation-2").unwrap().selection(),
        TextSelection::caret(2)
    );

    // Unfocused + paused: Escape bootstraps a synthetic ghost cursor and resumes.
    let outcome = doc.escape();
    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert_eq!(selected(&doc), Some("annotation-2"));
    assert_eq!(
        doc.helper()
            .pending
            .as_ref()
            .map(|restore| restore.synthetic),
        Some(true)
    );
}

#[test]
fn space_toggles_helper_playback_and_ghost_cursor_like_escape() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment_without_seek("annotation-1");
    set_playhead(&mut doc, 2.0);

    let outcome = perform(&mut doc, EditorAction::PlayPause);
    assert!(
        doc.session().is_playing,
        "space from paused resumes playback"
    );
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert_eq!(pending_segment(&doc), Some("annotation-1"));
    assert!(doc.ghost_cursor("annotation-1").is_some());

    set_playhead(&mut doc, 3.75);
    set_draft_with_caret_at_end(&mut doc, "annotation-2", "a bcdefghi");
    let outcome = perform(&mut doc, EditorAction::PlayPause);

    assert!(!doc.session().is_playing, "space while playing pauses");
    assert!(outcome.has_effect(&Effect::StopPlayback));
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "annotation-2".to_string(),
        caret: 2,
    }));
    assert_eq!(
        doc.helper().focused_segment_id.as_deref(),
        Some("annotation-2")
    );
    assert!(doc.helper().pending.is_none());
}

#[test]
fn space_plays_the_selected_segment_from_its_start_when_the_playhead_is_outside_it() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 2.0, 4.0, "one"),
        seg("b", "lane-1", 10.5, 15.4, "two"),
    ]);
    let _ = doc.select_segment_without_seek("b");
    set_playhead(&mut doc, 0.0);

    let outcome = perform(&mut doc, EditorAction::PlayPause);

    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert_eq!(doc.session().playhead_seconds, 10.5);
    assert_eq!(selected(&doc), Some("b"));
    assert_eq!(pending_segment(&doc), Some("b"));

    // After playing the row to its end the playhead sits on the boundary: Space replays it.
    let _ = perform(&mut doc, EditorAction::PlayPause);
    assert!(!doc.session().is_playing);
    let _ = doc.select_segment_without_seek("b");
    set_playhead(&mut doc, 15.4);
    let _ = perform(&mut doc, EditorAction::PlayPause);
    assert!(doc.session().is_playing);
    assert_eq!(doc.session().playhead_seconds, 10.5);
    assert_eq!(pending_segment(&doc), Some("b"));
}

#[test]
fn space_keeps_the_playhead_when_it_is_inside_the_selected_segment() {
    let mut doc = doc(vec![seg("b", "lane-1", 10.5, 15.4, "two")]);
    let _ = doc.select_segment_without_seek("b");
    set_playhead(&mut doc, 12.0);

    let _ = perform(&mut doc, EditorAction::PlayPause);

    assert!(doc.session().is_playing);
    assert_eq!(doc.session().playhead_seconds, 12.0);
    assert_eq!(pending_segment(&doc), Some("b"));
}

#[test]
fn helper_ghost_cursor_renders_only_on_pending_row_at_restore_offset_while_paused() {
    let mut doc = stable_sample_doc();
    set_draft_with_caret_at_end(&mut doc, "annotation-2", "abcdefghij");
    let _ = doc.select_segment_without_seek("annotation-2");
    set_playhead(&mut doc, 3.75);
    ok(doc.transcript_escape("annotation-2"));
    let _ = doc.toggle_playback();
    assert!(!doc.session().is_playing);

    let restore_offset = doc.helper().pending.as_ref().unwrap().restore_offset;
    let ghost = doc
        .ghost_cursor("annotation-2")
        .expect("pending helper restore exposes a ghost cursor");

    assert_eq!(ghost.segment_id, "annotation-2");
    assert_eq!(ghost.char_offset, restore_offset);
    assert!(
        doc.ghost_cursor("annotation-1").is_none(),
        "ghost cursor only renders on the pending restore row"
    );
}

#[test]
fn helper_ghost_cursor_is_hidden_on_the_focused_row() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment_without_seek("annotation-2");
    set_playhead(&mut doc, 3.75);
    ok(doc.transcript_escape("annotation-2"));
    assert!(doc.ghost_cursor("annotation-2").is_some());

    doc.set_text_focus(Some("annotation-2"));

    assert!(doc.ghost_cursor("annotation-2").is_none());
}

#[test]
fn helper_ghost_cursor_tracks_playhead_while_playing() {
    let mut doc = stable_sample_doc();
    set_draft_with_caret_at_end(&mut doc, "annotation-2", "abcdefghij");
    let segment = doc.project().segment("annotation-2").unwrap().clone();
    set_playhead(&mut doc, segment.start_seconds);
    ok(doc.transcript_escape("annotation-2"));
    assert!(doc.session().is_playing);

    let first = doc
        .ghost_cursor("annotation-2")
        .expect("playing restore exposes a ghost cursor");
    set_playhead(
        &mut doc,
        segment.start_seconds + (segment.end_seconds - segment.start_seconds) * 0.6,
    );
    let second = doc
        .ghost_cursor("annotation-2")
        .expect("playing restore keeps exposing a ghost cursor");

    assert!(
        second.char_offset > first.char_offset,
        "playing ghost cursor moves with the live playhead"
    );
}

#[test]
fn helper_ghost_cursor_snaps_playback_offset_to_word_boundary() {
    let mut doc = stable_sample_doc();
    set_draft_with_caret_at_end(&mut doc, "annotation-2", "alpha beta gamma");
    let segment = doc.project().segment("annotation-2").unwrap().clone();
    set_playhead(&mut doc, segment.start_seconds);
    ok(doc.transcript_escape("annotation-2"));
    set_playhead(
        &mut doc,
        segment.start_seconds + (segment.end_seconds - segment.start_seconds) * 0.5,
    );

    let ghost = doc
        .ghost_cursor("annotation-2")
        .expect("playing restore exposes a ghost cursor");

    assert_eq!(
        ghost.char_offset, 6,
        "raw proportional offset inside `beta` snaps to the nearest word start"
    );
}

#[test]
fn helper_ghost_cursor_advances_to_active_row_during_playback() {
    let mut doc = stable_sample_doc();
    let start = segment_start(&doc, "annotation-1");
    let _ = doc.select_segment_without_seek("annotation-1");
    set_playhead(&mut doc, start);
    ok(doc.transcript_escape("annotation-1"));
    assert_eq!(pending_segment(&doc), Some("annotation-1"));
    set_playhead(&mut doc, 4.0);

    let _ = doc.tick(0.016);

    assert_eq!(
        pending_segment(&doc),
        Some("annotation-2"),
        "the ghost cursor switches rows when playback exits its original row"
    );
    assert_eq!(selected(&doc), Some("annotation-2"));
    assert!(doc.ghost_cursor("annotation-1").is_none());
    assert!(doc.ghost_cursor("annotation-2").is_some());
}

#[test]
fn helper_ghost_cursor_parks_on_latest_lane_row_during_playback_gaps() {
    let mut doc = stable_sample_doc();
    let start = segment_start(&doc, "annotation-1");
    let _ = doc.select_segment_without_seek("annotation-1");
    set_playhead(&mut doc, start);
    ok(doc.transcript_escape("annotation-1"));
    set_playhead(&mut doc, 7.5);

    let _ = doc.tick(0.016);

    assert_eq!(
        pending_segment(&doc),
        Some("annotation-3"),
        "a gap after speech leaves the ghost at the latest row in its tracked lane"
    );
    let ghost = doc
        .ghost_cursor("annotation-3")
        .expect("the latest same-lane row renders the ghost cursor");
    assert_eq!(
        ghost.char_offset,
        draft_text(&doc, "annotation-3").chars().count(),
        "the cursor clamps to the completed row's end while playback is in the gap"
    );
}

#[test]
fn helper_ghost_cursor_pending_target_follows_segment_navigation() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-1");
    let _ = doc.helper_toggle_playback();
    let _ = doc.toggle_playback();
    assert!(!doc.session().is_playing);
    assert_eq!(pending_segment(&doc), Some("annotation-1"));

    let _ = doc.select_adjacent_segment(1);

    assert_eq!(pending_segment(&doc), Some("annotation-2"));
    assert!(
        doc.ghost_cursor("annotation-2").is_some(),
        "navigation moves the ghost cursor to the newly selected row"
    );
}

/// Fixture lanes: Speaker 1 rows annotation-1 (1.2–3.4 s) and annotation-3 (5.4–7.2 s);
/// Speaker 2 rows annotation-2 (3.5–5.1 s) and annotation-4 (10.9 s–).
fn play_ghost_from(doc: &mut EditorDoc, segment_id: &str, playhead: f64) {
    let _ = doc.select_segment_without_seek(segment_id);
    set_playhead(doc, playhead);
    let _ = doc.helper_toggle_playback();
    assert!(doc.session().is_playing);
    assert_eq!(pending_segment(doc), Some(segment_id));
}

fn play_to(doc: &mut EditorDoc, seconds: f64) {
    set_playhead(doc, seconds);
    let _ = doc.tick(0.01);
}

#[test]
fn helper_ghost_cursor_tab_switches_lane_and_playback_keeps_it_there() {
    let mut doc = stable_sample_doc();
    play_ghost_from(&mut doc, "annotation-1", 2.0);

    let outcome = perform(&mut doc, EditorAction::SwitchGhostCursorLane);
    assert_eq!(
        pending_segment(&doc),
        Some("annotation-2"),
        "before the other lane speaks, its first row takes the ghost"
    );
    assert_eq!(selected(&doc), Some("annotation-2"));
    assert!(doc.ghost_cursor("annotation-2").is_some());
    assert!(outcome.has_effect(&Effect::RevealSegment {
        segment_id: "annotation-2".to_string(),
        center: true,
    }));
    assert!(doc.session().is_playing, "switching lanes keeps playing");

    play_to(&mut doc, 6.0);
    assert_eq!(
        pending_segment(&doc),
        Some("annotation-2"),
        "the switched ghost stays on its lane while the other lane speaks"
    );
    play_to(&mut doc, 11.0);
    assert_eq!(pending_segment(&doc), Some("annotation-4"));

    let _ = perform(&mut doc, EditorAction::SwitchGhostCursorLane);
    assert_eq!(
        pending_segment(&doc),
        Some("annotation-3"),
        "back on Speaker 1: its latest row already started"
    );
}

#[test]
fn helper_ghost_cursor_follows_the_speaking_lane_again_after_a_new_ghost() {
    let mut doc = stable_sample_doc();
    play_ghost_from(&mut doc, "annotation-1", 2.0);
    let _ = perform(&mut doc, EditorAction::SwitchGhostCursorLane);
    let _ = doc.helper_toggle_playback();
    assert!(!doc.session().is_playing);

    ok(doc.transcript_escape("annotation-2"));
    play_to(&mut doc, 6.0);

    assert_eq!(
        pending_segment(&doc),
        Some("annotation-3"),
        "the lane switch ends with its ghost"
    );
}

#[test]
fn helper_ghost_cursor_lane_switch_needs_another_visible_lane() {
    let mut doc = stable_sample_doc();
    ok(doc.switch_speaker_workflow_lane("Speaker 1"));
    play_ghost_from(&mut doc, "annotation-1", 2.0);

    let error = doc.switch_ghost_cursor_lane(None).unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(pending_segment(&doc), Some("annotation-1"));
}

#[test]
fn ghost_cursor_lane_switch_without_a_cursor_is_unavailable() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment_without_seek("annotation-1");

    let error = doc.switch_ghost_cursor_lane(None).unwrap_err();

    assert!(matches!(error, EditError::Unavailable(_)));
    assert_eq!(selected(&doc), Some("annotation-1"));
}

fn focused_caret(outcome: &Outcome) -> Option<(&str, usize)> {
    outcome.effects.iter().find_map(|effect| match effect {
        Effect::FocusText { segment_id, caret } => Some((segment_id.as_str(), *caret)),
        _ => None,
    })
}

fn tab_in_text(doc: &mut EditorDoc, segment_id: &str) -> Outcome {
    doc.set_text_focus(Some(segment_id));
    ok(doc
        .perform(EditorAction::SwitchGhostCursorLane, Some(segment_id))
        .expect("the document handles the lane switch"))
}

/// annotation-5 (Speaker 1, 65.08–65.41 s) is spoken during annotation-4 (Speaker 2,
/// 10.93–109.29 s).
#[test]
fn tab_while_stopped_moves_the_writing_cursor_to_the_other_lane_at_the_playhead() {
    let mut doc = stable_sample_doc();
    set_playhead(&mut doc, 65.2);

    let outcome = tab_in_text(&mut doc, "annotation-5");
    let (segment_id, caret) = focused_caret(&outcome).expect("focus moves");
    assert_eq!(segment_id, "annotation-4");
    let text_len = draft_text(&doc, "annotation-4").chars().count();
    assert!(
        caret > 0 && caret < text_len,
        "the caret lands mid-row, where 65.2 s is spoken (got {caret} of {text_len})"
    );
    assert_eq!(
        doc.helper().focused_segment_id.as_deref(),
        Some("annotation-4")
    );
    assert_eq!(selected(&doc), Some("annotation-4"));
    assert!(!doc.session().is_playing);
    assert_eq!(
        doc.session().playhead_seconds,
        65.2,
        "switching never seeks"
    );

    let outcome = tab_in_text(&mut doc, "annotation-4");
    assert_eq!(
        focused_caret(&outcome).map(|(id, _)| id),
        Some("annotation-5")
    );
}

#[test]
fn tab_in_a_row_away_from_the_playhead_uses_that_row_s_time() {
    let mut doc = stable_sample_doc();
    set_playhead(&mut doc, 0.0);

    let outcome = tab_in_text(&mut doc, "annotation-4");

    assert_eq!(
        focused_caret(&outcome),
        Some((
            "annotation-3",
            draft_text(&doc, "annotation-3").chars().count()
        )),
        "from annotation-4's start, Speaker 1's latest started row, caret at its end"
    );
}

#[test]
fn helper_playhead_row_deduction_only_considers_visible_lanes() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment_without_seek("annotation-3");
    set_playhead(&mut doc, 4.0);

    let _ = doc.escape();
    assert_eq!(
        pending_segment(&doc),
        Some("annotation-2"),
        "with all lanes visible the playhead row wins"
    );

    let _ = doc.toggle_playback();
    assert!(!doc.session().is_playing);
    ok(doc.switch_speaker_workflow_lane("Speaker 1"));
    assert!(
        doc.helper().pending.is_none(),
        "hiding the pending row's lane drops its ghost cursor"
    );
    let _ = doc.select_segment_without_seek("annotation-3");
    set_playhead(&mut doc, 4.0);

    let _ = doc.escape();

    assert_eq!(
        pending_segment(&doc),
        Some("annotation-3"),
        "rows on hidden lanes must not capture the ghost cursor"
    );
}

// ---- find -----------------------------------------------------------------------------

#[test]
fn find_query_selects_transcript_matches_and_wraps() {
    let mut doc = large_two_lane_doc(6);

    let _ = perform(&mut doc, EditorAction::OpenFind);
    assert!(doc.find_state().open);

    let outcome = doc.set_find_query("large project row".to_string());
    assert_eq!(doc.find_state().query, "large project row");
    assert_eq!(selected(&doc), Some("annotation-0001"));
    assert!(outcome.has_effect(&Effect::RevealSegment {
        segment_id: "annotation-0001".to_string(),
        center: true,
    }));
    assert_eq!(
        doc.find_summary(),
        Some(FindSummary {
            current: 1,
            total: 5,
        })
    );

    let _ = doc.find_previous();
    assert_eq!(selected(&doc), Some("annotation-0005"));
    assert_eq!(
        doc.find_summary(),
        Some(FindSummary {
            current: 5,
            total: 5,
        })
    );

    let _ = doc.find_next();
    assert_eq!(selected(&doc), Some("annotation-0001"));

    let _ = doc.set_find_query("row 3".to_string());
    assert_eq!(selected(&doc), Some("annotation-0003"));
    assert_eq!(
        doc.find_summary(),
        Some(FindSummary {
            current: 1,
            total: 1,
        })
    );

    let _ = doc.set_find_query("not in transcript".to_string());
    assert_eq!(
        doc.find_summary(),
        Some(FindSummary {
            current: 0,
            total: 0,
        })
    );
}

#[test]
fn find_next_and_previous_shortcuts_move_between_matches() {
    let mut doc = large_two_lane_doc(6);
    let _ = doc.open_find();
    let _ = doc.set_find_query("large project row".to_string());

    let outcome = perform(&mut doc, EditorAction::FindNext);
    assert_eq!(selected(&doc), Some("annotation-0002"));
    assert!(outcome.has_effect(&Effect::RevealSegment {
        segment_id: "annotation-0002".to_string(),
        center: true,
    }));

    let _ = perform(&mut doc, EditorAction::FindPrevious);
    assert_eq!(selected(&doc), Some("annotation-0001"));
}

#[test]
fn find_matches_track_live_draft_text_edits() {
    let mut doc = large_two_lane_doc(6);
    let _ = doc.open_find();
    let _ = doc.set_find_query("draft-only-token".to_string());
    assert_eq!(
        doc.find_summary(),
        Some(FindSummary {
            current: 0,
            total: 0,
        })
    );

    set_draft_with_caret_at_end(&mut doc, "annotation-0001", "draft-only-token");

    assert_eq!(doc.find_matches(), vec!["annotation-0001".to_string()]);
    assert_eq!(
        doc.find_summary(),
        Some(FindSummary {
            current: 1,
            total: 1,
        })
    );
}

// ---- speed / rewind / speaker workflow ------------------------------------------------

#[test]
fn playback_speed_rewind_and_speaker_workflow_update_session() {
    let mut doc = stable_sample_doc();
    let _ = doc.set_speed(1.0);
    set_playhead(&mut doc, 3.75);

    let outcome = perform(&mut doc, EditorAction::IncreasePlaybackSpeed);
    assert_eq!(doc.session().speed, 1.5);
    assert_eq!(doc.session().playhead_seconds, 3.75);
    assert!(outcome.has_effect(&Effect::SpeedChanged(1.5)));

    let outcome = perform(&mut doc, EditorAction::DecreasePlaybackSpeed);
    assert_eq!(doc.session().speed, 1.0);
    assert_eq!(doc.session().playhead_seconds, 3.75);
    assert!(outcome.has_effect(&Effect::SpeedChanged(1.0)));

    let _ = perform(&mut doc, EditorAction::RewindPlaybackOneSecond);
    assert_eq!(doc.session().playhead_seconds, 2.75);

    let _ = perform(&mut doc, EditorAction::SwitchSpeakerWorkflowLane1);
    let lane_id = |doc: &EditorDoc, label: &str| {
        doc.project()
            .lanes
            .iter()
            .find(|lane| lane.label == label)
            .unwrap()
            .id
            .clone()
    };
    let speaker_one = lane_id(&doc, "Speaker 1");
    let speaker_two = lane_id(&doc, "Speaker 2");
    let session = doc.session();
    assert!(matches!(
        &session.track_filter,
        TrackFilter::Lane { label, .. } if label == "Speaker 1"
    ));
    assert!(!session.muted_lane_ids.contains(&speaker_one));
    assert!(session.muted_lane_ids.contains(&speaker_two));
    assert!(!session.collapsed_lane_ids.contains(&speaker_one));
    assert!(session.collapsed_lane_ids.contains(&speaker_two));

    let _ = perform(&mut doc, EditorAction::ResetSpeakerWorkflow);
    let session = doc.session();
    assert_eq!(session.track_filter, TrackFilter::All);
    assert!(!session.muted_lane_ids.contains(&speaker_one));
    assert!(!session.muted_lane_ids.contains(&speaker_two));
    assert!(!session.collapsed_lane_ids.contains(&speaker_one));
    assert!(!session.collapsed_lane_ids.contains(&speaker_two));
}

#[test]
fn rewind_past_the_visible_window_brings_the_playhead_into_view() {
    let mut doc = stable_sample_doc();
    let window = |start_seconds: f64| TimeRange {
        start_seconds,
        end_seconds: start_seconds + 6.0,
    };
    set_playhead(&mut doc, 50.5);
    doc.session_mut().viewport = window(50.0);

    let _ = perform(&mut doc, EditorAction::RewindPlaybackOneSecond);

    let viewport = doc.session().viewport;
    assert_eq!(doc.session().playhead_seconds, 49.5);
    assert!(
        viewport.start_seconds < 49.5 && 49.5 < viewport.end_seconds,
        "paused rewind out of view shifts the timeline to the playhead ({viewport:?})"
    );

    doc.session_mut().viewport = window(45.0);
    let _ = perform(&mut doc, EditorAction::RewindPlaybackOneSecond);
    assert_eq!(
        doc.session().viewport,
        window(45.0),
        "a playhead still in view keeps the view"
    );
}

// ---- timeline defaults ----------------------------------------------------------------

#[test]
fn timeline_defaults_persist_in_project_metadata_and_hydrate_on_reload() {
    let mut doc = stable_sample_doc();
    doc.set_zoom(2.0);
    let _ = doc.set_speed(1.5);
    doc.set_track_filter(TrackFilter::lane("track-speaker-2", "Speaker 2"));
    doc.toggle_lane_collapse("track-speaker-2");
    let _ = doc.toggle_lane_mute("track-speaker-1");
    doc.set_lane_vertical_zoom("track-speaker-2", 2.5);
    let expected_vertical_zoom = *doc
        .session()
        .lane_vertical_zoom
        .get("track-speaker-2")
        .expect("vertical zoom is set before reload");

    let reloaded = EditorDoc::from_project(
        doc.project().clone(),
        exaudio_extension::document::DocumentEnvironment::local("test"),
    );

    let session = reloaded.session();
    assert!((session.zoom - 2.0).abs() < f32::EPSILON);
    assert!((session.speed - 1.5).abs() < f32::EPSILON);
    assert_eq!(
        session.track_filter,
        TrackFilter::lane("track-speaker-2", "Speaker 2")
    );
    assert!(session.collapsed_lane_ids.contains("track-speaker-2"));
    assert!(session.muted_lane_ids.contains("track-speaker-1"));
    assert_eq!(
        session.lane_vertical_zoom.get("track-speaker-2").copied(),
        Some(expected_vertical_zoom)
    );
}

// ---- initial viewport -----------------------------------------------------------------

#[test]
fn first_audio_opens_a_segmentless_timeline_on_its_first_thirty_seconds() {
    let mut doc = doc(Vec::new());
    let collapsed = doc.session().viewport;
    assert!(collapsed.end_seconds <= collapsed.start_seconds);

    install_audio(&mut doc, "lane-1", audio_with_speech(116.0, 100, &[]));

    let viewport = doc.session().viewport;
    assert_close(viewport.start_seconds, 0.0);
    assert!(
        (viewport.end_seconds - 30.0).abs() < 0.01,
        "the view opens on the first 30 s of 116 s ({viewport:?})"
    );
    assert!(doc.session().zoom < 1.0);

    // A second track's audio keeps the view.
    install_audio(&mut doc, "lane-2", audio_with_speech(90.0, 100, &[]));
    assert_eq!(doc.session().viewport, viewport);
}

#[test]
fn first_audio_shorter_than_thirty_seconds_shows_the_whole_file() {
    let mut doc = doc(Vec::new());

    install_audio(&mut doc, "lane-1", audio_with_speech(12.0, 100, &[]));

    let viewport = doc.session().viewport;
    assert_close(viewport.start_seconds, 0.0);
    assert!(
        (viewport.end_seconds - 12.0).abs() < 0.01,
        "a 12 s file is shown whole ({viewport:?})"
    );
}

#[test]
fn audio_arriving_after_a_zoom_change_keeps_that_zoom() {
    let mut doc = doc(Vec::new());
    // 1000 px / (0.5 × 200 px/s) = 10 s visible.
    doc.set_zoom(0.5);

    install_audio(&mut doc, "lane-1", audio_with_speech(116.0, 100, &[]));

    let viewport = doc.session().viewport;
    assert!((doc.session().zoom - 0.5).abs() < f32::EPSILON);
    assert_close(viewport.start_seconds, 0.0);
    assert!(
        (viewport.end_seconds - 10.0).abs() < 0.01,
        "the user's zoom decides the visible seconds ({viewport:?})"
    );
}

#[test]
fn audio_growing_a_segment_bounded_timeline_shows_the_zoomed_seconds_from_the_start() {
    let mut doc = doc(vec![seg("a", "lane-1", 1.0, 3.0, "text")]);
    assert!((doc.session().viewport.end_seconds - 3.0).abs() < 0.001);

    install_audio(&mut doc, "lane-1", audio_with_speech(116.0, 100, &[]));

    let viewport = doc.session().viewport;
    assert!((doc.session().zoom - 1.0).abs() < f32::EPSILON);
    assert_close(viewport.start_seconds, 0.0);
    assert!(
        (viewport.end_seconds - 5.0).abs() < 0.01,
        "zoom 1 shows 5 s per 1000 px once the audio extends the bounds ({viewport:?})"
    );
}

// ---- Escape precedence ----------------------------------------------------------------

fn stable_doc_with_looping_selection() -> EditorDoc {
    let mut doc = stable_sample_doc();
    set_overlay(&mut doc, "track-speaker-1", 3.5, 5.1);
    perform(&mut doc, EditorAction::LoopCutPreviewOverlay);
    assert!(doc.session().loop_range().is_some());
    doc
}

#[test]
fn escape_closes_create_region_dialog_before_cancelling_looping_selection() {
    let mut doc = stable_doc_with_looping_selection();
    doc.open_create_region("track-speaker-1", 12.0, 12.8);

    let _ = perform(&mut doc, EditorAction::EscapePressed);

    assert!(doc.create_region().is_none());
    assert!(doc.session().loop_range().is_some());

    let _ = perform(&mut doc, EditorAction::EscapePressed);

    assert!(doc.session().overlay.is_none());
    assert!(doc.session().loop_range().is_none());
}

#[test]
fn escape_closes_find_bar_before_dialogs_and_selection() {
    let mut doc = stable_doc_with_looping_selection();
    doc.open_create_region("track-speaker-1", 12.0, 12.8);
    let _ = doc.open_find();

    let _ = doc.escape();

    assert!(!doc.find_state().open);
    assert!(doc.create_region().is_some());
    assert!(doc.session().loop_range().is_some());
}

#[test]
fn captured_escape_closes_find_bar_before_create_region_input_reset() {
    let mut doc = stable_sample_doc();
    doc.open_create_region("track-speaker-1", 12.0, 12.8);
    let _ = doc.open_find();
    assert!(doc.find_state().open);
    assert!(doc.create_region().is_some());

    let _ = perform(&mut doc, EditorAction::CreateRegionEscape);

    assert!(!doc.find_state().open);
    assert!(doc.create_region().is_some());
}

#[test]
fn escape_cancels_autoscroll_before_closing_find_bar() {
    let mut doc = stable_doc_with_looping_selection();
    let _ = doc.open_find();
    doc.hover_waveform(
        "track-speaker-1",
        hit(WaveformHitTarget::Waveform, None, 2.0),
    );
    doc.press_autoscroll("track-speaker-1");
    assert!(doc.pointer().autoscroll.is_some());
    assert!(doc.needs_ticks());

    let _ = doc.escape();

    assert!(doc.pointer().autoscroll.is_none());
    assert!(doc.find_state().open);
    assert!(doc.session().loop_range().is_some());

    let _ = doc.escape();
    assert!(!doc.find_state().open);
    assert!(doc.session().loop_range().is_some());
}

#[test]
fn transcript_rows_cache_hit_returns_same_rows_until_selection_or_session_changes() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "first"),
        seg("b", "lane-1", 2.0, 4.0, "second"),
    ]);
    doc.select_segment("a");
    let first = doc.transcript_rows();
    assert!(std::rc::Rc::ptr_eq(&first, &doc.transcript_rows()));

    doc.toggle_playback();
    let playing = doc.transcript_rows();
    doc.tick(0.1);
    assert!(
        std::rc::Rc::ptr_eq(&playing, &doc.transcript_rows()),
        "ticks inside the selected row must reuse the cached rows"
    );

    // 0.1s + 8 × 0.25s puts the playhead at 2.1s, inside row `b`.
    for _ in 0..8 {
        doc.tick(0.25);
    }
    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("b"));
    let after_boundary = doc.transcript_rows();
    assert!(!std::rc::Rc::ptr_eq(&playing, &after_boundary));
    assert!(
        after_boundary
            .iter()
            .any(|row| row.segment_id == "b" && row.is_selected)
    );

    doc.select_segment_without_seek("a");
    let reselected = doc.transcript_rows();
    assert!(!std::rc::Rc::ptr_eq(&after_boundary, &reselected));
    assert!(
        reselected
            .iter()
            .any(|row| row.segment_id == "a" && row.is_selected)
    );

    let diff_view = doc.session().diff_view;
    doc.session_mut().diff_view = !diff_view;
    let after_session_change = doc.transcript_rows();
    assert!(!std::rc::Rc::ptr_eq(&reselected, &after_session_change));
    assert!(std::rc::Rc::ptr_eq(
        &after_session_change,
        &doc.transcript_rows()
    ));
}
