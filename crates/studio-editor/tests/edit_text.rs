//! Draft text, local/project undo, pending-draft commit, time cells, and cross-row text moves.
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_editor::edit::draft::{
    SegmentDraft, TEXT_HISTORY_MAX_ENTRIES, TEXT_HISTORY_MAX_RETAINED_BYTES,
};
use studio_editor::edit::text::{
    CROSS_ROW_HISTORY_MAX_ENTRIES, CROSS_ROW_HISTORY_MAX_RETAINED_BYTES, CrossRowTextEdit,
    TextMoveError, push_cross_row_history,
};
use studio_editor::edit::time::{
    CommitTimeCellEdit, CommitTimeCellResult, TimeEditError, TimeEditState, TimeField,
    TimeRangeEdit, commit_time_cell_edit,
};
use studio_editor::edit::{EditError, EditorDoc, Effect, Outcome, TextSelection};
use studio_editor::track_filter::TrackFilter;

#[test]
fn an_empty_transcript_has_no_core_text_diagnostic() {
    let doc = doc(vec![seg("empty", "lane-1", 0.0, 1.0, "")]);
    let segment = doc.project().segment("empty").unwrap();
    assert!(doc.drafts().draft_lint(segment).is_none());
}

// ---- helpers ------------------------------------------------------------------------------

/// Types `text` into the row with the caret after `caret_chars` chars.
fn set_text_with_caret(doc: &mut EditorDoc, segment_id: &str, text: &str, caret_chars: usize) {
    ok(doc.edit_text(
        segment_id,
        text.to_string(),
        TextSelection::caret(caret_chars),
    ));
}

fn set_text_with_caret_at_end(doc: &mut EditorDoc, segment_id: &str, text: &str) {
    set_text_with_caret(doc, segment_id, text, text.chars().count());
}

fn is_dirty(doc: &EditorDoc) -> bool {
    doc.session().is_dirty
}

fn clear_dirty(doc: &mut EditorDoc) {
    doc.session_mut().is_dirty = false;
}

fn caret(doc: &EditorDoc, segment_id: &str) -> usize {
    doc.draft(segment_id).unwrap().selection().head
}

fn has_draft_changed(effects: &[Effect], segment_id: &str) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::DraftChanged { segment_id: id } if id == segment_id))
}

fn draft_with_text(text: &str) -> SegmentDraft {
    SegmentDraft::from_segment(&seg("segment", "lane-1", 0.0, 1.0, text))
}

fn cross_row_entry(index: usize, snapshot: String) -> CrossRowTextEdit {
    CrossRowTextEdit {
        source_segment_id: format!("source-{index}"),
        target_segment_id: format!("target-{index}"),
        source_before: snapshot.clone(),
        source_after: snapshot.clone(),
        target_before: snapshot.clone(),
        target_after: snapshot,
        source_before_caret: index,
        source_after_caret: index,
    }
}

fn cross_row_bytes(entries: &[CrossRowTextEdit]) -> usize {
    entries
        .iter()
        .map(CrossRowTextEdit::retained_utf8_bytes)
        .sum()
}

// ---- drafts -------------------------------------------------------------------------------

#[test]
fn segment_drafts_use_babel_timestamp_format() {
    let doc = stable_sample_doc();
    let first = doc.draft("annotation-1").unwrap();
    let long = doc.draft("annotation-5").unwrap();

    assert_eq!(first.time_value(TimeField::Start), "00:01.20");
    assert_eq!(first.time_value(TimeField::End), "00:03.40");
    assert_eq!(long.time_value(TimeField::Start), "00:21.08");
}

#[test]
fn segment_drafts_initialize_from_multiline_babel_text() {
    let doc = stable_sample_doc();
    assert_eq!(
        draft_text(&doc, "annotation-4"),
        text_of(&doc, "annotation-4")
    );
}

#[test]
fn transcript_text_edit_updates_draft_text_and_dirty_state() {
    let mut doc = stable_sample_doc();
    let edited = format!("{} edited", draft_text(&doc, "annotation-1"));
    set_text_with_caret_at_end(&mut doc, "annotation-1", &edited);

    assert_eq!(draft_text(&doc, "annotation-1"), edited);
    assert!(is_dirty(&doc));
    assert_eq!(text_of(&doc, "annotation-1"), "Здравствуйте.");
}

#[test]
fn transcript_text_undo_redo_stays_in_draft_without_project_history() {
    let mut doc = stable_sample_doc();
    let original = draft_text(&doc, "annotation-1");
    assert!(!doc.can_undo());

    let edited = format!("{original} edited");
    set_text_with_caret_at_end(&mut doc, "annotation-1", &edited);
    assert!(!doc.can_undo());

    let undo = ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), original);
    assert!(has_draft_changed(&undo.effects, "annotation-1"));
    assert!(!doc.can_undo());

    let redo = ok(doc.redo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), edited);
    assert!(has_draft_changed(&redo.effects, "annotation-1"));
    assert!(!doc.can_undo());
}

#[test]
fn read_only_project_blocks_transcript_text_and_structure_edits() {
    let mut doc = read_only_doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "first"),
        seg("b", "lane-1", 3.0, 5.0, "second"),
    ]);
    assert!(doc.is_read_only());

    let err = doc
        .edit_text("a", "first blocked".to_string(), TextSelection::caret(13))
        .unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    assert_eq!(draft_text(&doc, "a"), "first");
    assert!(!is_dirty(&doc));

    let err = doc.delete_segment("a").unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    assert_eq!(ids(&doc), vec!["a", "b"]);
    assert!(!is_dirty(&doc));
}

#[test]
fn draft_sync_preserves_unapplied_edits() {
    let mut doc = stable_sample_doc();
    let pending = format!("{} pending", draft_text(&doc, "annotation-1"));
    set_text_with_caret_at_end(&mut doc, "annotation-1", &pending);

    doc.with_workflow(|_| ());

    assert_eq!(draft_text(&doc, "annotation-1"), pending);
}

#[test]
fn unrelated_actions_preserve_unapplied_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "draft still being typed");

    ok(doc.split_segment_at("annotation-2", 4.3));

    assert_eq!(draft_text(&doc, "annotation-1"), "draft still being typed");
}

#[test]
fn find_matches_track_live_draft_text_edits() {
    let mut doc = stable_sample_doc();
    doc.open_find();
    doc.set_find_query("draft-only-token".to_string());
    assert!(doc.find_matches().is_empty());

    set_text_with_caret_at_end(&mut doc, "annotation-1", "draft-only-token");

    assert_eq!(doc.find_matches(), vec!["annotation-1".to_string()]);
    assert_eq!(doc.find_summary().map(|summary| summary.total), Some(1));
}

// ---- local text history bounds ------------------------------------------------------------

#[test]
fn thousands_of_edits_keep_only_the_latest_undo_chain_and_redo_it() {
    let edit_count = TEXT_HISTORY_MAX_ENTRIES + 2_000;
    let mut draft = draft_with_text("edit-0");
    for index in 1..=edit_count {
        draft.replace_text_from_user_edit(format!("edit-{index}"));
    }

    assert_eq!(draft.undo_depth(), TEXT_HISTORY_MAX_ENTRIES);

    for expected in (edit_count - TEXT_HISTORY_MAX_ENTRIES..edit_count).rev() {
        assert!(draft.undo());
        assert_eq!(draft.text(), format!("edit-{expected}"));
    }
    assert!(!draft.undo());

    for expected in edit_count - TEXT_HISTORY_MAX_ENTRIES + 1..=edit_count {
        assert!(draft.redo());
        assert_eq!(draft.text(), format!("edit-{expected}"));
    }
    assert!(!draft.redo());
}

#[test]
fn text_history_is_bounded_by_utf8_bytes() {
    let mut draft = draft_with_text("initial");
    for index in 0..80 {
        draft.replace_text_from_user_edit(format!("{index}:{}", "я".repeat(32 * 1024)));
    }

    let depth = draft.undo_depth();
    assert!(depth < TEXT_HISTORY_MAX_ENTRIES);

    let mut retained = 0;
    let mut expected_index = 78;
    while draft.undo() {
        retained += draft.text().len();
        assert!(draft.text().starts_with(&format!("{expected_index}:")));
        expected_index -= 1;
    }
    assert_eq!(78 - expected_index, depth);
    assert!(retained <= TEXT_HISTORY_MAX_RETAINED_BYTES);
}

#[test]
fn newest_oversized_snapshot_remains_immediately_undoable() {
    let mut draft = draft_with_text("initial");
    draft.replace_text_from_user_edit("small edit".to_string());
    let oversized = "x".repeat(TEXT_HISTORY_MAX_RETAINED_BYTES + 1);
    draft.replace_text_from_user_edit(oversized.clone());
    draft.replace_text_from_user_edit("current".to_string());

    assert_eq!(draft.undo_depth(), 1);
    assert!(draft.undo());
    assert_eq!(draft.text(), oversized);
}

#[test]
fn cross_row_history_keeps_the_newest_fixed_count() {
    let mut entries = Vec::new();
    for index in 0..5_000 {
        push_cross_row_history(&mut entries, cross_row_entry(index, String::new()));
    }

    assert_eq!(entries.len(), CROSS_ROW_HISTORY_MAX_ENTRIES);
    assert_eq!(
        entries.first().unwrap().source_segment_id,
        format!("source-{}", 5_000 - CROSS_ROW_HISTORY_MAX_ENTRIES)
    );
    assert_eq!(entries.last().unwrap().source_segment_id, "source-4999");
    assert!(cross_row_bytes(&entries) <= CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
}

#[test]
fn cross_row_history_is_bounded_by_all_retained_utf8_snapshots() {
    let mut entries = Vec::new();
    for index in 0..20 {
        push_cross_row_history(&mut entries, cross_row_entry(index, "я".repeat(32 * 1024)));
    }

    assert!(entries.len() < CROSS_ROW_HISTORY_MAX_ENTRIES);
    assert!(cross_row_bytes(&entries) <= CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
    assert_eq!(entries.last().unwrap().source_segment_id, "source-19");
}

#[test]
fn cross_row_history_keeps_one_newest_oversized_move() {
    let mut entries = Vec::new();
    push_cross_row_history(&mut entries, cross_row_entry(0, String::new()));
    push_cross_row_history(
        &mut entries,
        cross_row_entry(1, "x".repeat(CROSS_ROW_HISTORY_MAX_RETAINED_BYTES)),
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source_segment_id, "source-1");
    assert!(cross_row_bytes(&entries) > CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
}

// ---- cross-row text moves -----------------------------------------------------------------

#[test]
fn helper_text_move_before_caret_targets_previous_same_lane_segment() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-3");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "previous");
    set_text_with_caret(&mut doc, "annotation-3", "move me stay", 7);
    clear_dirty(&mut doc);

    let outcome = ok(doc.move_text_to_previous_row("annotation-3"));

    assert_eq!(draft_text(&doc, "annotation-1"), "previous move me");
    assert_eq!(draft_text(&doc, "annotation-3"), "stay");
    assert_eq!(caret(&doc, "annotation-3"), 0);
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(has_draft_changed(&outcome.effects, "annotation-3"));
    assert!(is_dirty(&doc));
}

#[test]
fn helper_text_move_after_caret_targets_next_same_lane_segment() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "next");
    set_text_with_caret(&mut doc, "annotation-1", "keep move me", 4);
    clear_dirty(&mut doc);

    ok(doc.move_text_to_next_row("annotation-1"));

    assert_eq!(draft_text(&doc, "annotation-1"), "keep");
    assert_eq!(caret(&doc, "annotation-1"), "keep".chars().count());
    assert_eq!(draft_text(&doc, "annotation-3"), "move me next");
    assert!(is_dirty(&doc));
}

#[test]
fn helper_text_move_uses_timeline_order_for_visual_first_row() {
    // `later` is first in project order but last on the timeline.
    let mut doc = doc(vec![
        seg("later", "lane-1", 5.0, 6.0, "later row"),
        seg("first", "lane-1", 1.0, 2.0, "first visible"),
    ]);
    doc.select_segment("first");
    doc.set_selection("first", TextSelection::caret(13));

    let err = doc.move_text_to_previous_row("first").unwrap_err();

    assert!(matches!(
        err,
        EditError::TextMove(TextMoveError::NoAdjacentSegment)
    ));
    assert_eq!(draft_text(&doc, "first"), "first visible");
    assert_eq!(draft_text(&doc, "later"), "later row");
    assert!(!is_dirty(&doc));
}

#[test]
fn helper_text_move_refuses_to_empty_source_in_either_direction() {
    let mut before = stable_sample_doc();
    before.select_segment("annotation-3");
    set_text_with_caret_at_end(&mut before, "annotation-1", "previous");
    set_text_with_caret(&mut before, "annotation-3", "whole row", 9);
    clear_dirty(&mut before);

    let err = before
        .move_text_to_previous_row("annotation-3")
        .unwrap_err();
    assert!(matches!(
        err,
        EditError::TextMove(TextMoveError::WouldEmptySource)
    ));
    assert_eq!(draft_text(&before, "annotation-3"), "whole row");
    assert_eq!(draft_text(&before, "annotation-1"), "previous");
    assert!(!is_dirty(&before));

    let mut after = stable_sample_doc();
    after.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut after, "annotation-3", "next");
    set_text_with_caret(&mut after, "annotation-1", "whole row", 0);
    clear_dirty(&mut after);

    let err = after.move_text_to_next_row("annotation-1").unwrap_err();
    assert!(matches!(
        err,
        EditError::TextMove(TextMoveError::WouldEmptySource)
    ));
    assert_eq!(draft_text(&after, "annotation-1"), "whole row");
    assert_eq!(draft_text(&after, "annotation-3"), "next");
    assert!(!is_dirty(&after));
}

#[test]
fn helper_text_move_undo_redo_restores_both_rows_atomically() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-3");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "previous");
    set_text_with_caret(&mut doc, "annotation-3", "move me stay", 7);

    ok(doc.move_text_to_previous_row("annotation-3"));
    let undo = ok(doc.undo_text("annotation-3"));

    assert_eq!(draft_text(&doc, "annotation-1"), "previous");
    assert_eq!(draft_text(&doc, "annotation-3"), "move me stay");
    assert_eq!(caret(&doc, "annotation-3"), 7);
    assert!(has_draft_changed(&undo.effects, "annotation-1"));
    assert!(has_draft_changed(&undo.effects, "annotation-3"));

    ok(doc.redo_text("annotation-3"));

    assert_eq!(draft_text(&doc, "annotation-1"), "previous move me");
    assert_eq!(draft_text(&doc, "annotation-3"), "stay");
    assert_eq!(caret(&doc, "annotation-3"), 0);
}

// ---- time cells ---------------------------------------------------------------------------

#[test]
fn time_cell_commit_rules_match_babel_submit_and_blur() {
    let invalid_submit = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: Some(TimeField::Start),
        field: TimeField::Start,
        draft_value: "00:03.40".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: false,
    });
    assert_eq!(
        invalid_submit,
        CommitTimeCellResult::Handled {
            active_field: Some(TimeField::Start),
            draft_value: "00:03.40".to_string(),
            error: Some(TimeEditError::StartNotBeforeEnd),
            next_times: None,
        }
    );

    let invalid_blur = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: Some(TimeField::Start),
        field: TimeField::Start,
        draft_value: "not a time".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: true,
    });
    assert_eq!(
        invalid_blur,
        CommitTimeCellResult::Handled {
            active_field: None,
            draft_value: "00:01.20".to_string(),
            error: None,
            next_times: None,
        }
    );

    let valid_submit = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: Some(TimeField::End),
        field: TimeField::End,
        draft_value: "00:03.75".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: false,
    });
    assert_eq!(
        valid_submit,
        CommitTimeCellResult::Handled {
            active_field: None,
            draft_value: "00:03.75".to_string(),
            error: None,
            next_times: Some(TimeRangeEdit {
                start_seconds: 1.2,
                end_seconds: 3.75,
            }),
        }
    );
}

#[test]
fn time_cell_commit_is_ignored_when_field_is_not_active() {
    let result = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: None,
        field: TimeField::Start,
        draft_value: "00:02.00".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: false,
    });
    assert_eq!(result, CommitTimeCellResult::Ignored);
}

#[test]
fn submitting_time_cell_edit_updates_boundary_without_flushing_text_draft() {
    let mut doc = stable_sample_doc();
    let original_text = text_of(&doc, "annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "text draft still being edited");
    clear_dirty(&mut doc);

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:01.25".to_string()));
    assert!(!is_dirty(&doc));

    ok(doc.commit_time_edit("annotation-1", TimeField::Start, false));

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.25);
    assert_eq!(segment.end_seconds, 3.4);
    assert_eq!(segment.text, original_text);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:01.25");
    assert_eq!(draft.text(), "text draft still being edited");
    assert_eq!(*draft.time_edit(), TimeEditState::default());
    assert!(is_dirty(&doc));
}

#[test]
fn submitting_time_cell_edit_preserves_timeline_viewport() {
    let mut doc = stable_sample_doc();
    doc.session_mut().viewport.start_seconds = 1.0;
    doc.session_mut().viewport.end_seconds = 6.0;
    doc.session_mut().playhead_seconds = 80.0;
    let before_viewport = doc.session().viewport;

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:01.25".to_string()));
    ok(doc.commit_time_edit("annotation-1", TimeField::Start, false));

    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.25
    );
    assert_eq!(doc.session().playhead_seconds, 80.0);
    assert_eq!(doc.session().viewport, before_viewport);
}

#[test]
fn invalid_time_cell_submit_stays_local_and_reports_babel_error() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:03.40".to_string()));
    let err = doc
        .commit_time_edit("annotation-1", TimeField::Start, false)
        .unwrap_err();

    assert!(matches!(
        err,
        EditError::Time(TimeEditError::StartNotBeforeEnd)
    ));
    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.2);
    assert_eq!(segment.end_seconds, 3.4);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:03.40");
    assert_eq!(draft.time_edit().active_field, Some(TimeField::Start));
    assert_eq!(
        draft.time_edit().error,
        Some(TimeEditError::StartNotBeforeEnd)
    );
    assert!(draft.time_error(TimeField::Start).is_some());
    assert!(!is_dirty(&doc));
}

#[test]
fn invalid_time_cell_blur_reverts_silently() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "not a time".to_string()));
    ok(doc.commit_time_edit("annotation-1", TimeField::Start, true));

    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:01.20");
    assert_eq!(draft.time_edit().active_field, None);
    assert_eq!(draft.time_error(TimeField::Start), None);
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.2
    );
    assert!(!is_dirty(&doc));
}

#[test]
fn escape_discards_a_typed_valid_time_even_when_the_cell_blurs_afterwards() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:02.00".to_string()));
    doc.cancel_time_edit("annotation-1", TimeField::Start);
    // Leaving the cell after Escape must not apply the abandoned value.
    ok(doc.commit_time_edit("annotation-1", TimeField::Start, true));

    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:01.20");
    assert_eq!(draft.time_edit().active_field, None);
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.2
    );
    assert!(!doc.can_undo());
    assert!(!is_dirty(&doc));
}

#[test]
fn escape_after_a_rejected_enter_clears_the_error_and_shows_the_stored_time() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::End, " ".to_string()));
    assert!(
        doc.commit_time_edit("annotation-1", TimeField::End, false)
            .is_err()
    );
    doc.cancel_time_edit("annotation-1", TimeField::End);

    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::End), "00:03.40");
    assert_eq!(draft.time_error(TimeField::End), None);
    assert_eq!(draft.time_edit().active_field, None);
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().end_seconds,
        3.4
    );
}

#[test]
fn time_cell_boundary_edit_is_one_project_undo_step() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::End, "00:03.30".to_string()));
    ok(doc.commit_time_edit("annotation-1", TimeField::End, false));
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().end_seconds,
        3.3
    );

    ok(doc.undo());

    assert_eq!(
        doc.project().segment("annotation-1").unwrap().end_seconds,
        3.4
    );
    assert_eq!(
        doc.draft("annotation-1")
            .unwrap()
            .time_value(TimeField::End),
        "00:03.40"
    );
}

// ---- commit / project undo ----------------------------------------------------------------

#[test]
fn applying_text_only_preserves_precise_timing() {
    let mut doc = doc(vec![seg("a", "lane-1", 1.23456, 3.45678, "Original.")]);
    set_text_with_caret_at_end(&mut doc, "a", "Only text changed.");

    ok(doc.commit_draft("a"));

    let segment = doc.project().segment("a").unwrap();
    assert_eq!(segment.start_seconds, 1.23456);
    assert_eq!(segment.end_seconds, 3.45678);
    assert_eq!(segment.text, "Only text changed.");
    assert!(is_dirty(&doc));
    assert!(!doc.has_pending_drafts());
}

#[test]
fn applying_text_and_timing_undoes_as_one_step() {
    let mut doc = stable_sample_doc();
    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:01.50".to_string()));
    ok(doc.set_time_draft("annotation-1", TimeField::End, "00:03.75".to_string()));
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Updated together.");

    ok(doc.commit_draft("annotation-1"));
    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.5);
    assert_eq!(segment.end_seconds, 3.75);
    assert_eq!(segment.text, "Updated together.");

    ok(doc.undo());

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.2);
    assert_eq!(segment.end_seconds, 3.4);
    assert_eq!(segment.text, "Здравствуйте.");
    assert_eq!(draft_text(&doc, "annotation-1"), "Здравствуйте.");
}

#[test]
fn committing_draft_with_invalid_typed_time_is_refused() {
    let mut doc = stable_sample_doc();
    ok(doc.set_time_draft("annotation-1", TimeField::Start, "bogus".to_string()));

    let err = doc.commit_draft("annotation-1").unwrap_err();

    assert!(matches!(err, EditError::Draft { .. }));
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.2
    );
    assert!(doc.pending_draft_segment_ids().is_err());
    assert!(doc.has_pending_drafts());
}

#[test]
fn global_undo_is_blocked_losslessly_with_pending_transcript_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Committed history step.");
    ok(doc.commit_draft("annotation-1"));
    let committed = text_of(&doc, "annotation-1");

    set_text_with_caret(
        &mut doc,
        "annotation-1",
        "Pending local edit.",
        "Pending".chars().count(),
    );
    set_text_with_caret(
        &mut doc,
        "annotation-1",
        "Pending! local edit.",
        "Pending!".chars().count(),
    );

    let err = doc.undo().unwrap_err();

    assert!(matches!(err, EditError::PendingDrafts { action: "Undo" }));
    assert_eq!(text_of(&doc, "annotation-1"), committed);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.text(), "Pending! local edit.");
    assert_eq!(
        draft.selection(),
        TextSelection::caret("Pending!".chars().count())
    );
    assert!(draft.can_undo());

    ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), "Pending local edit.");
}

#[test]
fn global_redo_is_blocked_losslessly_with_pending_transcript_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Redo history step.");
    ok(doc.commit_draft("annotation-1"));
    ok(doc.undo());
    let undone = text_of(&doc, "annotation-1");
    assert!(doc.can_redo());

    set_text_with_caret_at_end(&mut doc, "annotation-1", "Pending redo-safe draft.");

    let err = doc.redo().unwrap_err();

    assert!(matches!(err, EditError::PendingDrafts { action: "Redo" }));
    assert_eq!(text_of(&doc, "annotation-1"), undone);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.text(), "Pending redo-safe draft.");
    assert_eq!(
        draft.selection(),
        TextSelection::caret("Pending redo-safe draft.".chars().count())
    );
    assert!(doc.can_redo());
}

#[test]
fn apply_pending_drafts_commits_multiple_rows_as_one_undo_step() {
    let mut doc = stable_sample_doc();
    let first = doc.project().segments[0].id.clone();
    let second = doc.project().segments[1].id.clone();
    let original_first = text_of(&doc, &first);
    let original_second = text_of(&doc, &second);
    set_text_with_caret_at_end(&mut doc, &first, "Grouped draft one.");
    set_text_with_caret_at_end(&mut doc, &second, "Grouped draft two.");
    assert_eq!(
        doc.pending_draft_segment_ids().unwrap(),
        vec![first.clone(), second.clone()]
    );

    ok(doc.commit_pending_drafts("Save"));

    assert_eq!(text_of(&doc, &first), "Grouped draft one.");
    assert_eq!(text_of(&doc, &second), "Grouped draft two.");
    assert!(!doc.has_pending_drafts());

    ok(doc.undo());

    assert_eq!(text_of(&doc, &first), original_first);
    assert_eq!(text_of(&doc, &second), original_second);
}

#[test]
fn commit_pending_drafts_refuses_invalid_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Valid text.");
    ok(doc.set_time_draft("annotation-2", TimeField::End, "nope".to_string()));

    let err = doc.commit_pending_drafts("Save").unwrap_err();

    assert!(matches!(err, EditError::Draft { action: "Save", .. }));
    assert_eq!(text_of(&doc, "annotation-1"), "Здравствуйте.");
}

// ---- commit on Enter / blur ---------------------------------------------------------------

#[test]
fn typing_in_a_focused_row_leaves_the_project_untouched() {
    let mut doc = stable_sample_doc();
    ok(doc.set_text_focus(Some("annotation-1")));
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Typed, not committed.");

    assert_eq!(text_of(&doc, "annotation-1"), "Здравствуйте.");
    assert_eq!(draft_text(&doc, "annotation-1"), "Typed, not committed.");
    assert!(doc.has_pending_drafts());
    assert!(!doc.can_undo());
    let segment = doc.project().segment("annotation-1").unwrap();
    assert!(doc.drafts().draft_lint(segment).is_none());
}

#[test]
fn enter_commits_the_typed_text_as_one_undo_step_and_drops_the_local_history() {
    let mut doc = stable_sample_doc();
    ok(doc.set_text_focus(Some("annotation-1")));
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Typed then");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Typed then committed.");

    let outcome = ok(doc.blur_text("annotation-1"));

    assert_eq!(text_of(&doc, "annotation-1"), "Typed then committed.");
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(!doc.has_pending_drafts());
    assert_eq!(doc.helper().focused_segment_id, None);
    assert!(!doc.draft("annotation-1").unwrap().can_undo());
    assert_eq!(ok(doc.commit_pending_drafts("Save")), Outcome::none());

    ok(doc.undo());

    assert_eq!(text_of(&doc, "annotation-1"), "Здравствуйте.");
    assert_eq!(draft_text(&doc, "annotation-1"), "Здравствуйте.");
    assert!(!doc.can_undo());
}

#[test]
fn blurring_an_unchanged_row_adds_no_undo_step() {
    let mut doc = stable_sample_doc();
    ok(doc.set_text_focus(Some("annotation-1")));

    let outcome = ok(doc.set_text_focus(None));

    assert_eq!(outcome, Outcome::none());
    assert!(!doc.can_undo());
    assert!(!is_dirty(&doc));
    assert_eq!(doc.helper().focused_segment_id, None);
}

#[test]
fn focusing_another_row_commits_the_row_left_behind() {
    let mut doc = stable_sample_doc();
    ok(doc.set_text_focus(Some("annotation-1")));
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Left behind.");

    ok(doc.set_text_focus(Some("annotation-2")));

    assert_eq!(text_of(&doc, "annotation-1"), "Left behind.");
    assert_eq!(
        doc.helper().focused_segment_id.as_deref(),
        Some("annotation-2")
    );
    assert!(!doc.has_pending_drafts());
}

#[test]
fn helper_focus_moves_commit_the_row_left_behind() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
    ]);
    doc.select_segment("a");
    ok(doc.set_text_focus(Some("a")));
    set_text_with_caret_at_end(&mut doc, "a", "one more");

    let outcome = ok(doc.focus_adjacent_segment(1));

    assert_eq!(text_of(&doc, "a"), "one more");
    assert!(has_draft_changed(&outcome.effects, "a"));
    assert_eq!(doc.helper().focused_segment_id.as_deref(), Some("b"));

    set_text_with_caret_at_end(&mut doc, "b", "two more");
    ok(doc.edit_segment_text("a"));

    assert_eq!(text_of(&doc, "b"), "two more");
    assert_eq!(doc.helper().focused_segment_id.as_deref(), Some("a"));
}

#[test]
fn escape_blur_commits_the_typed_text_before_playing() {
    let mut doc = stable_sample_doc();
    ok(doc.set_text_focus(Some("annotation-1")));
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Escaped.");

    let outcome = ok(doc.transcript_escape("annotation-1"));

    assert!(outcome.has_effect(&Effect::BlurText));
    assert!(doc.session().is_playing);
    assert_eq!(text_of(&doc, "annotation-1"), "Escaped.");
    assert!(!doc.has_pending_drafts());
}

#[test]
fn blur_commits_a_row_that_lost_the_helper_focus_while_hidden() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-2", 3.0, 5.0, "two"),
    ]);
    ok(doc.set_text_focus(Some("a")));
    set_text_with_caret_at_end(&mut doc, "a", "one hidden");
    doc.set_track_filter(TrackFilter::lane("lane-2", "Speaker 2"));
    assert_eq!(doc.helper().focused_segment_id, None);

    ok(doc.blur_text("a"));

    assert_eq!(text_of(&doc, "a"), "one hidden");
    assert!(!doc.has_pending_drafts());
}

#[test]
fn blur_of_a_removed_row_is_a_no_op() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 2.0, "one")]);
    ok(doc.set_text_focus(Some("a")));
    doc.select_segment("a");
    ok(doc.delete_selected());

    let outcome = ok(doc.blur_text("a"));

    assert_eq!(outcome, Outcome::none());
}

// ---- timeline tooltip: edit a row's text ----------------------------------------------------

#[test]
fn edit_segment_text_selects_row_without_seeking_and_focuses_draft_end() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
    ]);
    doc.select_segment("a");
    doc.session_mut().playhead_seconds = 1.0;
    set_text_with_caret(&mut doc, "b", "héllo wörld", 0);

    let outcome = ok(doc.edit_segment_text("b"));

    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("b"));
    assert_eq!(doc.session().playhead_seconds, 1.0);
    assert_eq!(doc.helper().focused_segment_id.as_deref(), Some("b"));
    assert_eq!(
        doc.draft("b").expect("draft").selection(),
        TextSelection::caret(11)
    );
    assert!(outcome.has_effect(&Effect::RevealSegment {
        segment_id: "b".to_string(),
        center: false,
    }));
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "b".to_string(),
        caret: 11,
    }));
}

#[test]
fn edit_segment_text_refuses_read_only_tasks_and_missing_rows() {
    let mut read_only = read_only_doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
    ]);
    read_only.select_segment("a");
    assert!(matches!(
        read_only.edit_segment_text("b"),
        Err(EditError::ReadOnly { .. })
    ));
    assert_eq!(
        read_only.session().selected_segment_id.as_deref(),
        Some("a")
    );
    assert_eq!(read_only.helper().focused_segment_id, None);

    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 2.0, "one")]);
    assert!(matches!(
        doc.edit_segment_text("gone"),
        Err(EditError::NoDraft(id)) if id == "gone"
    ));
    assert_eq!(doc.helper().focused_segment_id, None);
}
