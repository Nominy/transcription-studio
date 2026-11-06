//! Structural edits and waveform pointer semantics of `studio_editor::edit`: split/merge/delete/
//! insert rows, waveform press/drag/release (boundary resize, create-by-drag, cut-preview overlay
//! drags), audition, the add-region and edit-annotation dialogs, the track cell, annotation type
//! changes and transcript row clicks.
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_editor::edit::create_region::{
    CreateRegionDialog, CreateRegionTimeField, track_options,
};
use studio_editor::edit::format::format_time;
use studio_editor::edit::structure::{
    AuditionKey, CREATE_PREVIEW_SEGMENT_ID, OverlayDragTarget, SplitError,
};
use studio_editor::edit::time::{TimeEditError, TimeField};
use studio_editor::edit::{
    EditError, EditorAction, EditorDoc, Effect, Outcome, PointerModifiers, TextSelection,
};
use studio_editor::timeline_model::TimeRange;
use studio_waveform::WaveformHitTarget;

const SHIFT: PointerModifiers = PointerModifiers {
    shift: true,
    alt: false,
    secondary: false,
};
const ALT: PointerModifiers = PointerModifiers {
    shift: false,
    alt: true,
    secondary: false,
};
const SECONDARY: PointerModifiers = PointerModifiers {
    shift: false,
    alt: false,
    secondary: true,
};

fn lane_of(doc: &EditorDoc, segment_id: &str) -> String {
    doc.project()
        .segment(segment_id)
        .unwrap_or_else(|| panic!("segment {segment_id} exists"))
        .lane_id
        .clone()
}

fn selected(doc: &EditorDoc) -> Option<String> {
    doc.session().selected_segment_id.clone()
}

fn segment_count(doc: &EditorDoc) -> usize {
    doc.project().segments.len()
}

fn set_viewport(doc: &mut EditorDoc, start_seconds: f64, end_seconds: f64) {
    doc.session_mut().viewport = TimeRange {
        start_seconds,
        end_seconds,
    };
}

fn has_play_range(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::PlayRange { .. }))
}

// ---- read-only ------------------------------------------------------------------------

#[test]
fn read_only_project_refuses_structure_edits() {
    let mut doc = read_only_doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one two"),
        seg("b", "lane-1", 2.5, 4.0, "three"),
    ]);
    let before = doc.project().clone();

    let err = doc.delete_segment("a").unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    let err = doc.split_segment_at("a", 1.0).unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    let err = doc.merge_below("a").unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    let err = doc.create_segment_before("b").unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    let err = doc.change_segment_track("a", "lane-2").unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));

    assert_eq!(doc.project(), &before);
    assert!(!doc.session().is_dirty);
}

// ---- audition -------------------------------------------------------------------------

#[test]
fn waveform_shift_click_auditions_full_region_without_starting_drag() {
    let mut doc = doc(vec![seg("a", "lane-1", 1.0, 3.0, "hello")]);
    doc.set_modifiers(SHIFT);
    doc.hover_waveform("lane-1", hit(WaveformHitTarget::RegionBody, Some("a"), 1.5));

    let outcome = ok(doc.press_waveform("lane-1"));

    assert!(outcome.has_effect(&Effect::PlayRange {
        start_seconds: 1.0,
        end_seconds: 3.0,
    }));
    assert!(!doc.session().is_playing);
    assert_eq!(doc.session().playhead_seconds, 1.0);
    assert!(doc.pointer().waveform_drag.is_none());
    assert!(doc.pointer().create_drag.is_none());
    assert_eq!(selected(&doc).as_deref(), Some("a"));
}

#[test]
fn waveform_held_audition_keys_play_500ms_boundary_windows() {
    let cases = [
        (AuditionKey::BeforeStart, 0.5, 1.0),
        (AuditionKey::AfterStart, 1.0, 1.5),
        (AuditionKey::BeforeEnd, 2.5, 3.0),
        (AuditionKey::AfterEnd, 3.0, 3.5),
    ];

    for (key, expected_start, expected_end) in cases {
        let mut doc = doc(vec![seg("a", "lane-1", 1.0, 3.0, "hello")]);
        doc.set_audition_key(key, true);
        doc.hover_waveform("lane-1", hit(WaveformHitTarget::RegionBody, Some("a"), 2.0));

        let outcome = ok(doc.press_waveform("lane-1"));

        assert!(
            outcome.has_effect(&Effect::PlayRange {
                start_seconds: expected_start,
                end_seconds: expected_end,
            }),
            "{key:?} should audition {expected_start}..{expected_end}, got {:?}",
            outcome.effects
        );
        assert!(doc.pointer().waveform_drag.is_none());
    }
}

#[test]
fn repeated_waveform_region_click_does_not_audition_region() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");

    for _ in 0..2 {
        doc.hover_waveform(
            &lane_id,
            hit(WaveformHitTarget::RegionBody, Some("annotation-1"), 1.5),
        );
        let pressed = ok(doc.press_waveform(&lane_id));
        let released = ok(doc.release_waveform(&lane_id));
        assert!(!has_play_range(&pressed.effects));
        assert!(!has_play_range(&released.effects));
    }

    assert!(!doc.session().is_playing);
    assert!(doc.pointer().waveform_drag.is_none());
}

// ---- split ----------------------------------------------------------------------------

#[test]
fn native_split_at_time_has_no_domain_gap() {
    let mut doc = stable_sample_doc();

    let _ = ok(doc.split_segment_at("annotation-1", 2.0));

    let left = doc.project().segment("annotation-1").unwrap();
    let right = doc.project().segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.end_seconds, 2.0);
    assert_eq!(right.start_seconds, 2.0);
}

#[test]
fn split_uses_given_time_even_when_playhead_is_inside_the_segment() {
    let mut doc = stable_sample_doc();
    let _ = doc.select_segment("annotation-1");
    doc.session_mut().playhead_seconds = 2.75;

    let _ = ok(doc.split_segment_at("annotation-1", 2.0));

    let left = doc.project().segment("annotation-1").unwrap();
    let right = doc.project().segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.end_seconds, 2.0);
    assert_eq!(right.start_seconds, 2.0);
}

#[test]
fn split_allocates_text_by_word_ratio_and_selects_right_row() {
    let mut stable_project = stable_sample_doc().project().clone();
    stable_project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = stable_project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 0.0;
    segment.end_seconds = 4.0;
    segment.text = "one two three four five".to_string();
    let mut doc = EditorDoc::from_project(
        stable_project,
        exaudio_extension::document::DocumentEnvironment::local("test"),
    );

    let outcome = ok(doc.split_segment_at("annotation-1", 2.0));

    assert_eq!(text_of(&doc, "annotation-1"), "one two three");
    assert_eq!(text_of(&doc, "annotation-1__studio_1"), "four five");
    assert_eq!(draft_text(&doc, "annotation-1"), "one two three");
    assert_eq!(draft_text(&doc, "annotation-1__studio_1"), "four five");
    assert_eq!(selected(&doc).as_deref(), Some("annotation-1__studio_1"));
    assert!(outcome.has_effect(&Effect::DraftChanged {
        segment_id: "annotation-1".to_string(),
    }));
    assert!(doc.session().is_dirty);
}

#[test]
fn split_rejects_tail_guard_before_workflow_mutation() {
    let mut doc = stable_sample_doc();
    let before = doc.project().segment("annotation-1").unwrap().clone();

    let err = doc.split_segment_at("annotation-1", 3.31).unwrap_err();

    assert!(matches!(err, EditError::Split(SplitError::TooCloseToEnd)));
    assert_eq!(doc.project().segment("annotation-1").unwrap(), &before);
    assert!(doc.project().segment("annotation-1__studio_1").is_none());
    assert!(!doc.session().is_dirty);
}

#[test]
fn split_rejects_points_outside_the_segment() {
    let mut doc = stable_sample_doc();
    let before = doc.project().clone();

    let err = doc.split_segment_at("annotation-1", 0.5).unwrap_err();

    assert!(matches!(err, EditError::Split(SplitError::OutsideSegment)));
    assert_eq!(doc.project(), &before);
}

#[test]
fn split_at_playhead_action_splits_the_selected_row_with_status_and_undo() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 4.0, "one two three four")]);
    doc.select_segment_without_seek("a");
    doc.session_mut().playhead_seconds = 2.0;

    let outcome = ok(doc
        .perform(EditorAction::SplitAtPlayhead, None)
        .expect("doc action"));

    assert_eq!(text_of(&doc, "a"), "one two");
    assert_eq!(text_of(&doc, "a__studio_1"), "three four");
    assert_eq!(doc.project().segment("a").unwrap().end_seconds, 2.0);
    assert_eq!(selected(&doc).as_deref(), Some("a__studio_1"));
    assert!(outcome.status.is_some(), "split reports what it did");
    assert!(doc.session().is_dirty);

    ok(doc.undo());
    assert_eq!(ids(&doc), vec!["a"]);
    assert_eq!(text_of(&doc, "a"), "one two three four");
}

#[test]
fn split_at_playhead_action_needs_the_playhead_inside_the_selected_row() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
    ]);
    let before = doc.project().clone();
    doc.session_mut().selected_segment_id = None;

    let err = doc
        .perform(EditorAction::SplitAtPlayhead, None)
        .expect("doc action")
        .unwrap_err();
    assert!(matches!(err, EditError::NoSelection { .. }));

    doc.select_segment_without_seek("b");
    doc.session_mut().playhead_seconds = 1.0;
    let err = doc
        .perform(EditorAction::SplitAtPlayhead, None)
        .expect("doc action")
        .unwrap_err();
    assert!(matches!(err, EditError::Split(SplitError::OutsideSegment)));
    assert_eq!(doc.project(), &before);
}

#[test]
fn waveform_secondary_press_splits_at_hit_time_without_a_platform_gap() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    doc.set_modifiers(SECONDARY);
    doc.hover_waveform(
        &lane_id,
        hit(WaveformHitTarget::RegionBody, Some("annotation-1"), 2.85),
    );

    let _ = ok(doc.press_waveform(&lane_id));

    let left = doc.project().segment("annotation-1").unwrap();
    let right = doc.project().segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.end_seconds, 2.85);
    assert_eq!(right.start_seconds, left.end_seconds);
    assert_eq!(selected(&doc).as_deref(), Some("annotation-1__studio_1"));
    assert!(doc.pointer().waveform_drag.is_none());
}

// ---- merge / insert -------------------------------------------------------------------

#[test]
fn row_action_merge_closes_menu_and_dirties_task() {
    let mut doc = stable_sample_doc();
    let original_count = segment_count(&doc);
    doc.session_mut().open_actions_segment_id = Some("annotation-1".to_string());

    let _ = ok(doc.merge_below("annotation-1"));

    assert_eq!(segment_count(&doc), original_count - 1);
    assert!(doc.session().open_actions_segment_id.is_none());
    assert!(doc.session().is_dirty);
}

#[test]
fn create_segment_before_fills_previous_lane_gap_and_selects_new_segment() {
    let mut doc = stable_sample_doc();
    let anchor = doc.project().segment("annotation-3").unwrap().clone();

    let _ = ok(doc.create_segment_before("annotation-3"));

    let created = doc.project().segment("annotation-3__studio_1").unwrap();
    assert_eq!(created.lane_id, anchor.lane_id);
    assert_eq!(created.speaker_id, anchor.speaker_id);
    assert!((created.start_seconds - 3.4).abs() < f64::EPSILON * 8.0);
    assert_eq!(created.end_seconds, 5.4);
    assert_eq!(selected(&doc).as_deref(), Some("annotation-3__studio_1"));
}

// ---- row clicks -----------------------------------------------------------------------

#[test]
fn transcript_row_click_selects_without_moving_playhead() {
    let mut doc = stable_sample_doc();
    doc.session_mut().playhead_seconds = 8.75;
    let before_viewport = doc.session().viewport;

    let outcome = doc.press_row("annotation-1", 1);

    assert_eq!(selected(&doc).as_deref(), Some("annotation-1"));
    assert_eq!(doc.session().playhead_seconds, 8.75);
    assert_eq!(doc.session().viewport, before_viewport);
    assert!(!doc.session().is_playing);
    assert!(!outcome.has_effect(&Effect::StartPlayback));
    assert!(!outcome.has_effect(&Effect::RestartPlayback));
}

#[test]
fn transcript_row_double_click_moves_playhead_to_segment_start() {
    let mut doc = stable_sample_doc();
    doc.session_mut().playhead_seconds = 8.75;
    let segment_start = doc.project().segment("annotation-1").unwrap().start_seconds;

    let _ = doc.press_row("annotation-1", 1);
    assert_eq!(doc.session().playhead_seconds, 8.75);

    let _ = doc.press_row("annotation-1", 2);

    assert_eq!(selected(&doc).as_deref(), Some("annotation-1"));
    assert_eq!(doc.session().playhead_seconds, segment_start);
}

// ---- waveform region press / boundary drags --------------------------------------------

#[test]
fn waveform_segment_click_selects_and_seeks_to_hit_time_without_dragging() {
    let mut doc = stable_sample_doc();
    let original = doc.project().segment("annotation-1").unwrap().clone();
    let lane_id = original.lane_id.clone();
    let hit_time = (original.start_seconds + original.end_seconds) / 2.0;
    doc.session_mut().playhead_seconds = 8.75;
    let _ = doc.select_segment_without_seek("annotation-2");
    let before_viewport = doc.session().viewport;

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionBody,
            Some(original.id.as_str()),
            hit_time,
        ),
    );
    let pressed = ok(doc.press_waveform(&lane_id));

    assert_eq!(selected(&doc).as_deref(), Some(original.id.as_str()));
    assert_eq!(doc.session().playhead_seconds, hit_time);
    assert_eq!(doc.session().viewport, before_viewport);
    assert!(doc.pointer().waveform_drag.is_none());
    assert!(!doc.session().is_playing);
    assert!(!has_play_range(&pressed.effects));

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionBody,
            Some(original.id.as_str()),
            hit_time + 0.4,
        ),
    );
    let during_hover = doc.project().segment(&original.id).unwrap();
    assert_eq!(during_hover.start_seconds, original.start_seconds);
    assert_eq!(during_hover.end_seconds, original.end_seconds);
    assert!(!doc.session().is_dirty);
    assert!(doc.pointer().waveform_drag.is_none());

    let _ = ok(doc.release_waveform(&lane_id));

    let released = doc.project().segment(&original.id).unwrap();
    assert_eq!(released.start_seconds, original.start_seconds);
    assert_eq!(released.end_seconds, original.end_seconds);
    assert_eq!(doc.session().playhead_seconds, hit_time);
    assert!(doc.pointer().waveform_drag.is_none());
}

#[test]
fn waveform_segment_click_restarts_active_playback_from_hit_time() {
    let mut doc = doc(vec![seg("a", "lane-1", 1.0, 3.0, "hello")]);
    doc.session_mut().playhead_seconds = 0.25;
    let _ = doc.toggle_playback();
    assert!(doc.session().is_playing);

    doc.hover_waveform("lane-1", hit(WaveformHitTarget::RegionBody, Some("a"), 1.5));
    let pressed = ok(doc.press_waveform("lane-1"));
    let released = ok(doc.release_waveform("lane-1"));

    assert!(pressed.has_effect(&Effect::RestartPlayback));
    assert!(!has_play_range(&pressed.effects));
    assert!(!has_play_range(&released.effects));
    assert!(doc.session().is_playing);
    assert_eq!(selected(&doc).as_deref(), Some("a"));
    assert_eq!(doc.session().playhead_seconds, 1.5);
    assert!(doc.pointer().waveform_drag.is_none());
}

#[test]
fn waveform_start_handle_drag_previews_then_commits_boundary_keeping_text_draft() {
    let mut doc = stable_sample_doc();
    let original = doc.project().segment("annotation-1").unwrap().clone();
    let lane_id = original.lane_id.clone();
    let moved_start = original.start_seconds + 0.25;
    let before_viewport = doc.session().viewport;
    let before_edit = "edge drag preserves editor";
    let edited_text = "edge! drag preserves editor";
    let _ = ok(doc.edit_text(
        &original.id,
        before_edit.to_string(),
        TextSelection::new(4, 4),
    ));
    let _ = ok(doc.edit_text(
        &original.id,
        edited_text.to_string(),
        TextSelection::new(5, 5),
    ));

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionStartHandle,
            Some(original.id.as_str()),
            original.start_seconds,
        ),
    );
    let _ = ok(doc.press_waveform(&lane_id));

    let drag = doc
        .pointer()
        .waveform_drag
        .as_ref()
        .expect("pressing a start handle activates a resize drag");
    assert_eq!(drag.target, WaveformHitTarget::RegionStartHandle);
    assert_eq!(drag.preview_start_seconds, original.start_seconds);
    assert_eq!(drag.preview_end_seconds, original.end_seconds);

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionBody,
            Some(original.id.as_str()),
            moved_start,
        ),
    );

    let drag = doc.pointer().waveform_drag.as_ref().unwrap();
    assert_eq!(drag.preview_start_seconds, moved_start);
    assert_eq!(drag.preview_end_seconds, original.end_seconds);
    assert!(drag.has_moved);
    let preview = doc.timeline_preview().expect("resize preview");
    assert_eq!(preview.segment_id, original.id);
    assert_eq!(preview.start_seconds, moved_start);
    assert_eq!(preview.end_seconds, original.end_seconds);
    assert_eq!(
        doc.project().segment(&original.id).unwrap().start_seconds,
        original.start_seconds,
        "dragging previews instead of mutating the project on every pointer move"
    );

    let _ = ok(doc.release_waveform(&lane_id));

    let segment = doc.project().segment(&original.id).unwrap();
    assert_eq!(segment.start_seconds, moved_start);
    assert_eq!(segment.end_seconds, original.end_seconds);
    assert_eq!(selected(&doc).as_deref(), Some(original.id.as_str()));
    assert_eq!(doc.session().playhead_seconds, original.start_seconds);
    assert_eq!(doc.session().viewport, before_viewport);
    assert!(doc.session().is_dirty);
    assert!(doc.pointer().waveform_drag.is_none());
    assert!(doc.timeline_preview().is_none());
    let draft = doc.draft(&original.id).unwrap();
    assert_eq!(draft.text(), edited_text);
    assert_eq!(draft.selection(), TextSelection::new(5, 5));
    assert_eq!(draft.time_value(TimeField::Start), format_time(moved_start));
    assert!(draft.can_undo());

    let _ = ok(doc.undo_text(&original.id));
    assert_eq!(draft_text(&doc, &original.id), before_edit);
}

#[test]
fn waveform_end_handle_drag_commits_segment_boundary() {
    let mut doc = stable_sample_doc();
    let original = doc.project().segment("annotation-1").unwrap().clone();
    let lane_id = original.lane_id.clone();
    let moved_end = original.end_seconds + 0.25;

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionEndHandle,
            Some(original.id.as_str()),
            original.end_seconds,
        ),
    );
    let _ = ok(doc.press_waveform(&lane_id));
    assert_eq!(
        doc.pointer().waveform_drag.as_ref().map(|drag| drag.target),
        Some(WaveformHitTarget::RegionEndHandle)
    );

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, moved_end));
    let _ = ok(doc.release_waveform(&lane_id));

    let segment = doc.project().segment(&original.id).unwrap();
    assert_eq!(segment.start_seconds, original.start_seconds);
    assert_eq!(segment.end_seconds, moved_end);
    assert!(doc.session().is_dirty);
    assert!(doc.can_undo());
    assert!(doc.pointer().waveform_drag.is_none());
}

#[test]
fn waveform_segment_handle_click_seeks_without_trimming_or_recentering_viewport() {
    let mut doc = stable_sample_doc();
    let original = doc.project().segment("annotation-1").unwrap().clone();
    set_viewport(&mut doc, 1.0, 6.0);
    doc.session_mut().playhead_seconds = 80.0;
    let before_viewport = doc.session().viewport;

    doc.hover_waveform(
        &original.lane_id,
        hit(
            WaveformHitTarget::RegionStartHandle,
            Some(original.id.as_str()),
            original.start_seconds,
        ),
    );
    let _ = ok(doc.press_waveform(&original.lane_id));
    let _ = ok(doc.release_waveform(&original.lane_id));

    let segment = doc.project().segment(&original.id).unwrap();
    assert_eq!(segment.start_seconds, original.start_seconds);
    assert_eq!(segment.end_seconds, original.end_seconds);
    assert_eq!(doc.session().playhead_seconds, original.start_seconds);
    assert_eq!(doc.session().viewport, before_viewport);
    assert!(!doc.session().is_dirty);
    assert!(!doc.can_undo());
    assert!(doc.pointer().waveform_drag.is_none());
}

#[test]
fn global_pointer_release_after_waveform_body_press_does_not_commit_move() {
    let mut doc = stable_sample_doc();
    let original = doc.project().segment("annotation-1").unwrap().clone();
    let lane_id = original.lane_id.clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionBody,
            Some(original.id.as_str()),
            grab_time,
        ),
    );
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionBody,
            Some(original.id.as_str()),
            grab_time + 0.25,
        ),
    );
    assert!(doc.pointer().waveform_drag.is_none());

    let _ = ok(doc.release_pointer());

    let segment = doc.project().segment(&original.id).unwrap();
    assert_eq!(segment.start_seconds, original.start_seconds);
    assert_eq!(segment.end_seconds, original.end_seconds);
    assert!(!doc.session().is_dirty);
    assert!(doc.pointer().waveform_drag.is_none());
}

#[test]
fn waveform_region_body_press_hover_release_does_not_create_boundary_undo_step() {
    let mut doc = stable_sample_doc();
    let original = doc.project().segment("annotation-1").unwrap().clone();
    let lane_id = original.lane_id.clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;
    assert!(!doc.can_undo());

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::RegionBody,
            Some(original.id.as_str()),
            grab_time,
        ),
    );
    let _ = ok(doc.press_waveform(&lane_id));
    for move_time in [grab_time + 0.2, grab_time + 0.4, grab_time + 0.6] {
        doc.hover_waveform(
            &lane_id,
            hit(
                WaveformHitTarget::RegionBody,
                Some(original.id.as_str()),
                move_time,
            ),
        );
    }
    let _ = ok(doc.release_waveform(&lane_id));

    let segment = doc.project().segment(&original.id).unwrap();
    assert_eq!(segment.start_seconds, original.start_seconds);
    assert_eq!(segment.end_seconds, original.end_seconds);
    assert!(!doc.can_undo());
    assert!(doc.pointer().waveform_drag.is_none());
}

// ---- cut-preview overlay drags --------------------------------------------------------

#[test]
fn waveform_body_drag_inside_active_cut_preview_locks_without_editing_project() {
    let mut doc = stable_sample_doc();
    let original_count = segment_count(&doc);
    set_viewport(&mut doc, 0.0, 10.0);
    doc.toggle_overlay();
    let initial = overlay_range(&doc).expect("T opens an overlay");
    let lane_id = overlay_lane(&doc).unwrap().to_string();

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::Waveform,
            None,
            initial.start_seconds + 0.4,
        ),
    );
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::Waveform,
            None,
            initial.start_seconds + 2.0,
        ),
    );
    let _ = ok(doc.release_waveform(&lane_id));

    assert_eq!(overlay_range(&doc), Some(initial));
    assert_eq!(segment_count(&doc), original_count);
    assert!(doc.create_region().is_none());
    assert!(doc.pointer().overlay_drag.is_none());
    assert!(!doc.session().is_dirty);
}

#[test]
fn secondary_waveform_body_drag_inside_active_overlay_moves_overlay_without_editing_project() {
    let mut doc = stable_sample_doc();
    let original_count = segment_count(&doc);
    set_viewport(&mut doc, 0.0, 10.0);
    doc.toggle_overlay();
    let initial = overlay_range(&doc).expect("T opens an overlay");
    let lane_id = overlay_lane(&doc).unwrap().to_string();
    doc.set_modifiers(SECONDARY);

    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::Waveform,
            None,
            initial.start_seconds + 0.4,
        ),
    );
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(
        &lane_id,
        hit(
            WaveformHitTarget::Waveform,
            None,
            initial.start_seconds + 2.0,
        ),
    );
    let _ = ok(doc.release_waveform(&lane_id));

    let overlay = overlay_range(&doc).unwrap();
    assert_eq!(segment_count(&doc), original_count);
    assert!(doc.create_region().is_none());
    assert!(!doc.session().is_dirty);
    assert!((overlay.start_seconds - (initial.start_seconds + 1.6)).abs() < 0.001);
    assert!(
        (overlay.duration_seconds() - initial.duration_seconds()).abs() < 0.001,
        "moving keeps the overlay length"
    );
}

#[test]
fn cut_preview_overlay_edge_drag_resizes_below_a_second() {
    let mut doc = stable_sample_doc();
    let lane_id = doc.project().lanes[0].id.clone();
    set_viewport(&mut doc, 0.0, 10.0);
    set_overlay(&mut doc, &lane_id, 2.0, 4.0);

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 2.0));
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 2.6));
    let _ = ok(doc.release_waveform(&lane_id));

    let overlay = overlay_range(&doc).unwrap();
    assert!((overlay.start_seconds - 2.6).abs() < 0.001);
    assert!((overlay.end_seconds - 4.0).abs() < 0.001);

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 4.0));
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 2.8));
    let _ = ok(doc.release_waveform(&lane_id));

    let overlay = overlay_range(&doc).unwrap();
    assert!((overlay.start_seconds - 2.6).abs() < 0.001);
    assert!((overlay.end_seconds - 2.8).abs() < 0.001);
    assert_eq!(overlay_lane(&doc), Some(lane_id.as_str()));
    assert!(!doc.session().is_dirty);
}

#[test]
fn cut_preview_edges_grab_within_twelve_css_px_on_either_side() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    // 1000 CSS px over 10 s: 12 px = 0.12 s.
    set_viewport(&mut doc, 0.0, 10.0);
    assert_eq!(doc.session().timeline_width_px(), 1000.0);
    set_overlay(&mut doc, "lane-1", 2.0, 4.0);

    let target = |doc: &EditorDoc, time: f64| doc.cut_overlay_target("lane-1", time);
    assert_eq!(target(&doc, 1.89), Some(OverlayDragTarget::StartHandle));
    assert_eq!(target(&doc, 2.11), Some(OverlayDragTarget::StartHandle));
    assert_eq!(target(&doc, 3.89), Some(OverlayDragTarget::EndHandle));
    assert_eq!(target(&doc, 4.11), Some(OverlayDragTarget::EndHandle));
    assert_eq!(target(&doc, 2.13), Some(OverlayDragTarget::Locked));
    assert_eq!(target(&doc, 1.87), None);
    assert_eq!(target(&doc, 4.13), None);

    // Zoomed in tenfold the window stays 12 px: 0.012 s.
    set_viewport(&mut doc, 1.5, 2.5);
    assert_eq!(target(&doc, 1.989), Some(OverlayDragTarget::StartHandle));
    assert_eq!(target(&doc, 1.98), None);
    assert_eq!(target(&doc, 2.02), Some(OverlayDragTarget::Locked));
}

#[test]
fn press_just_outside_a_cut_preview_edge_resizes_instead_of_creating_a_region() {
    let mut doc = doc(vec![seg("a", "lane-1", 6.0, 8.0, "text")]);
    set_viewport(&mut doc, 0.0, 10.0);
    set_overlay(&mut doc, "lane-1", 2.0, 4.0);
    let before = doc.project().segments.clone();

    doc.hover_waveform("lane-1", hit(WaveformHitTarget::Waveform, None, 1.92));
    let _ = ok(doc.press_waveform("lane-1"));
    assert!(doc.pointer().create_drag.is_none());
    doc.hover_waveform("lane-1", hit(WaveformHitTarget::Waveform, None, 1.4));
    let _ = ok(doc.release_waveform("lane-1"));

    let overlay = overlay_range(&doc).unwrap();
    assert!((overlay.start_seconds - 1.4).abs() < 0.001);
    assert!((overlay.end_seconds - 4.0).abs() < 0.001);
    assert_eq!(doc.project().segments, before);
    assert!(doc.create_region().is_none());
}

#[test]
fn secondary_inside_cut_preview_moves_it_even_over_an_edge() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    set_viewport(&mut doc, 0.0, 10.0);
    set_overlay(&mut doc, "lane-1", 2.0, 4.0);
    doc.set_modifiers(SECONDARY);

    let target = |time: f64| doc.cut_overlay_target("lane-1", time);
    assert_eq!(target(3.0), Some(OverlayDragTarget::Body));
    assert_eq!(target(2.05), Some(OverlayDragTarget::Body));
    assert_eq!(target(1.95), Some(OverlayDragTarget::StartHandle));
}

#[test]
fn alt_waveform_drag_creates_cut_preview_overlay_without_editing_project() {
    let mut doc = stable_sample_doc();
    let lane_id = doc.project().lanes[0].id.clone();
    let original_count = segment_count(&doc);
    set_viewport(&mut doc, 0.0, 10.0);
    set_overlay(&mut doc, &lane_id, 1.0, 2.0);
    doc.set_modifiers(ALT);

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 3.0));
    let _ = ok(doc.press_waveform(&lane_id));
    assert!(overlay_range(&doc).is_none());
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 5.25));
    let _ = ok(doc.release_waveform(&lane_id));

    let overlay = overlay_range(&doc).unwrap();
    assert_eq!(segment_count(&doc), original_count);
    assert!(doc.create_region().is_none());
    assert!(!doc.session().is_dirty);
    assert!((overlay.start_seconds - 3.0).abs() < 0.001);
    assert!((overlay.end_seconds - 5.25).abs() < 0.001);
}

#[test]
fn alt_drag_updates_the_cut_preview_on_every_pointer_move() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 8.0, "text")]);
    set_viewport(&mut doc, 0.0, 10.0);
    doc.set_modifiers(ALT);

    doc.hover_waveform("lane-1", hit(WaveformHitTarget::RegionBody, Some("a"), 3.0));
    let _ = ok(doc.press_waveform("lane-1"));
    for (time, expected) in [
        (3.06, (3.0, 3.06)),
        (4.5, (3.0, 4.5)),
        (5.25, (3.0, 5.25)),
        (1.5, (1.5, 3.0)),
        (6.0, (3.0, 6.0)),
    ] {
        doc.hover_waveform(
            "lane-1",
            hit(WaveformHitTarget::RegionBody, Some("a"), time),
        );
        let overlay = overlay_range(&doc).expect("the preview follows the pointer");
        assert!(
            (overlay.start_seconds - expected.0).abs() < 1e-9
                && (overlay.end_seconds - expected.1).abs() < 1e-9,
            "pointer at {time}: {overlay:?}"
        );
    }
    let _ = ok(doc.release_waveform("lane-1"));
    assert_eq!(overlay_lane(&doc), Some("lane-1"));
}

#[test]
fn alt_waveform_tiny_drag_keeps_cut_preview_unmaterialized() {
    let mut doc = stable_sample_doc();
    let lane_id = doc.project().lanes[0].id.clone();
    set_viewport(&mut doc, 0.0, 10.0);
    doc.session_mut().overlay = None;
    doc.set_modifiers(ALT);

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 3.0));
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 3.02));
    let _ = ok(doc.release_waveform(&lane_id));

    assert!(overlay_range(&doc).is_none());
    assert!(doc.pointer().overlay_drag.is_none());
}

#[test]
fn cut_preview_belongs_to_the_lane_it_was_drawn_on() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 6.0, 8.0, "one"),
        seg("b", "lane-2", 0.5, 8.0, "two"),
    ]);
    set_viewport(&mut doc, 0.0, 10.0);
    doc.set_modifiers(ALT);
    doc.hover_waveform("lane-2", hit(WaveformHitTarget::RegionBody, Some("b"), 2.0));
    let _ = ok(doc.press_waveform("lane-2"));
    doc.hover_waveform("lane-2", hit(WaveformHitTarget::RegionBody, Some("b"), 4.0));
    let _ = ok(doc.release_waveform("lane-2"));
    doc.set_modifiers(PointerModifiers::default());

    assert_eq!(overlay_lane(&doc), Some("lane-2"));
    assert_eq!(doc.session().overlay_on("lane-1"), None);
    assert_eq!(doc.cut_overlay_target("lane-1", 2.0), None);
    assert_eq!(
        doc.cut_overlay_target("lane-2", 2.0),
        Some(OverlayDragTarget::StartHandle)
    );

    // The same spot on the other lane is plain audio there: a press starts a region drag.
    doc.hover_waveform("lane-1", hit(WaveformHitTarget::Waveform, None, 2.0));
    let _ = ok(doc.press_waveform("lane-1"));
    assert!(doc.pointer().overlay_drag.is_none());
    assert!(doc.pointer().create_drag.is_some());
    doc.leave_waveform("lane-1");
    let _ = ok(doc.release_pointer());
    assert_eq!(overlay_lane(&doc), Some("lane-2"));
}

#[test]
fn t_opens_the_cut_preview_on_the_selected_rows_lane() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.5, 2.0, "one"),
        seg("b", "lane-2", 3.0, 8.0, "two"),
    ]);
    let _ = doc.select_segment("b");

    doc.toggle_overlay();

    assert_eq!(overlay_lane(&doc), Some("lane-2"));
    doc.toggle_overlay();
    assert!(doc.session().overlay.is_none());
}

#[test]
fn escape_cancels_the_cut_preview_before_toggling_playback() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.5, 2.0, "one")]);
    let _ = doc.select_segment("a");
    doc.toggle_overlay();
    assert!(doc.session().overlay.is_some());

    let _ = doc.escape();

    assert!(doc.session().overlay.is_none());
    assert!(!doc.session().is_playing);
}

// ---- create by drag -------------------------------------------------------------------

#[test]
fn empty_waveform_drag_creates_segment_and_focuses_its_text() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    set_viewport(&mut doc, 11.0, 14.0);
    doc.session_mut().playhead_seconds = 80.0;
    let before_viewport = doc.session().viewport;

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 12.0));
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 12.8));
    let outcome = ok(doc.release_waveform(&lane_id));

    assert_eq!(segment_count(&doc), original_count + 1);
    assert!(
        doc.create_region().is_none(),
        "no dialog after a drag create"
    );
    let created_id = selected(&doc).expect("created segment is selected");
    let created = doc
        .project()
        .segment(&created_id)
        .expect("created segment is in the project immediately");
    assert_eq!(created.lane_id, lane_id);
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    assert!(created.text.is_empty());
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: created_id.clone(),
        caret: 0,
    }));
    assert!(doc.pointer().create_drag.is_none());
    assert!(doc.session().is_dirty);
    assert_eq!(doc.session().playhead_seconds, 80.0);
    assert_eq!(doc.session().viewport, before_viewport);
}

#[test]
fn empty_waveform_drag_previews_created_segment_before_release() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    set_viewport(&mut doc, 11.0, 14.0);

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 12.0));
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 12.8));

    assert_eq!(
        segment_count(&doc),
        original_count,
        "the create preview does not commit a segment before release"
    );
    let preview = doc
        .timeline_preview()
        .expect("dragging over empty waveform shows a live create preview");
    assert_eq!(preview.segment_id, CREATE_PREVIEW_SEGMENT_ID);
    assert_eq!(preview.lane_id, lane_id);
    assert!((preview.start_seconds - 12.0).abs() < 0.000_001);
    assert!((preview.end_seconds - 12.8).abs() < 0.000_001);
    assert!(doc.pointer().create_drag.is_some());
}

#[test]
fn empty_waveform_click_seeks_on_release_without_creating_region() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    let target_time = 12.35;
    doc.session_mut().playhead_seconds = 80.0;

    doc.hover_waveform(
        &lane_id,
        hit(WaveformHitTarget::Waveform, None, target_time),
    );
    let _ = ok(doc.press_waveform(&lane_id));

    assert_eq!(doc.session().playhead_seconds, 80.0);
    assert!(doc.pointer().create_drag.is_some());

    let released = doc.release_waveform(&lane_id);

    assert!(released.is_ok(), "a click is not a too-short region");
    assert_eq!(doc.session().playhead_seconds, target_time);
    assert_eq!(segment_count(&doc), original_count);
    assert!(doc.create_region().is_none());
    assert!(doc.pointer().create_drag.is_none());
}

#[test]
fn empty_waveform_tiny_pointer_jitter_stays_click_not_create_drag() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    let target_time = 12.0;
    set_viewport(&mut doc, 11.0, 14.0);

    doc.hover_waveform(
        &lane_id,
        hit(WaveformHitTarget::Waveform, None, target_time),
    );
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(
        &lane_id,
        hit(WaveformHitTarget::Waveform, None, target_time + 0.01),
    );
    let _ = ok(doc.release_waveform(&lane_id));

    assert_eq!(doc.session().playhead_seconds, target_time);
    assert_eq!(segment_count(&doc), original_count);
    assert!(doc.create_region().is_none());
    assert!(doc.pointer().create_drag.is_none());
}

#[test]
fn empty_waveform_reverse_drag_normalizes_created_segment_range() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    set_viewport(&mut doc, 13.0, 15.0);

    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 14.0));
    let _ = ok(doc.press_waveform(&lane_id));
    doc.hover_waveform(&lane_id, hit(WaveformHitTarget::Waveform, None, 13.2));
    let _ = ok(doc.release_waveform(&lane_id));

    let created_id = selected(&doc).expect("created segment is selected");
    let created = doc.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 13.2);
    assert_eq!(created.end_seconds, 14.0);
    assert!(doc.create_region().is_none());
}

// ---- add-region dialog ----------------------------------------------------------------

#[test]
fn create_region_speaker_picker_uses_lane_labels() {
    let doc = doc(Vec::new());

    let options = track_options(doc.project());

    assert_eq!(
        options
            .iter()
            .map(|option| (option.lane_id.as_str(), option.label.as_str()))
            .collect::<Vec<_>>(),
        vec![("lane-1", "Speaker 1"), ("lane-2", "Speaker 2")]
    );
}

#[test]
fn create_region_transcription_enablement_rules() {
    let mut dialog = CreateRegionDialog::new("lane-1".to_string(), 12.0, 12.8);
    assert!(dialog.is_create_enabled());

    dialog.selected_lane_id = String::new();
    assert!(!dialog.is_create_enabled());

    dialog.selected_lane_id = "lane-1".to_string();
    dialog.start_error = Some(TimeEditError::InvalidFormat);
    assert!(!dialog.is_create_enabled());

    dialog.start_error = None;
    dialog.end_error = Some(TimeEditError::InvalidFormat);
    assert!(!dialog.is_create_enabled());

    dialog.end_error = None;
    dialog.start_seconds = 12.8;
    assert!(!dialog.is_create_enabled());
}

#[test]
fn submitting_create_region_creates_segment_with_text_and_closes() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let speaker_id = doc
        .project()
        .segment("annotation-1")
        .unwrap()
        .speaker_id
        .clone();
    let original_count = segment_count(&doc);
    doc.open_create_region(&lane_id, 12.0, 12.8);

    let _ = doc.set_create_region_text("hello new region".to_string());
    let _ = ok(doc.submit_create_region());

    assert_eq!(segment_count(&doc), original_count + 1);
    let created_id = selected(&doc).expect("created segment is selected");
    let created = doc.project().segment(&created_id).unwrap();
    assert_eq!(created.lane_id, lane_id);
    assert_eq!(created.speaker_id, speaker_id);
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    assert_eq!(created.text, "hello new region");
    assert_eq!(draft_text(&doc, &created_id), "hello new region");
    assert!(doc.create_region().is_none());
    assert!(doc.session().is_dirty);
}

#[test]
fn create_region_start_enter_updates_dialog_without_creating_a_segment() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    doc.open_create_region(&lane_id, 12.0, 12.8);

    doc.create_region_mut()
        .unwrap()
        .set_start_draft("00:12.20".to_string());
    let _ = ok(doc.commit_create_region_time(CreateRegionTimeField::Start));

    assert_eq!(segment_count(&doc), original_count);
    let dialog = doc.create_region().unwrap();
    assert!((dialog.start_seconds - 12.2).abs() < 1e-9);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.start_draft, "00:12.20");
    assert_eq!(dialog.end_draft, "00:12.80");
    assert_eq!(dialog.start_error, None);
}

#[test]
fn create_region_end_enter_reports_invalid_range_without_creating_a_segment() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    doc.open_create_region(&lane_id, 12.0, 12.8);

    doc.create_region_mut()
        .unwrap()
        .set_end_draft("00:11.90".to_string());
    let err = doc
        .commit_create_region_time(CreateRegionTimeField::End)
        .unwrap_err();

    assert!(matches!(
        err,
        EditError::Time(TimeEditError::EndNotAfterStart)
    ));
    assert_eq!(segment_count(&doc), original_count);
    let dialog = doc.create_region().unwrap();
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.end_draft, "00:11.90");
    assert_eq!(dialog.end_error, Some(TimeEditError::EndNotAfterStart));
}

#[test]
fn create_region_start_blur_resets_invalid_draft_and_keeps_error() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    let original_count = segment_count(&doc);
    doc.open_create_region(&lane_id, 12.0, 12.8);

    let dialog = doc.create_region_mut().unwrap();
    dialog.set_start_draft("00:12.90".to_string());
    let committed = dialog.blur_time_field(CreateRegionTimeField::Start);

    assert!(!committed);
    assert_eq!(segment_count(&doc), original_count);
    let dialog = doc.create_region().unwrap();
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.start_draft, "00:12.00");
    assert_eq!(dialog.start_error, Some(TimeEditError::StartNotBeforeEnd));
}

#[test]
fn create_region_escape_resets_active_time_draft_without_closing_dialog() {
    let mut doc = stable_sample_doc();
    let lane_id = lane_of(&doc, "annotation-1");
    doc.open_create_region(&lane_id, 12.0, 12.8);

    doc.create_region_mut()
        .unwrap()
        .set_start_draft("00:12.90".to_string());
    assert!(
        doc.commit_create_region_time(CreateRegionTimeField::Start)
            .is_err()
    );
    assert!(doc.reset_create_region_time());

    let dialog = doc
        .create_region()
        .expect("escape in a field keeps the dialog");
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.start_draft, "00:12.00");
    assert_eq!(dialog.start_error, None);
}

// ---- track cell -----------------------------------------------------------------------

#[test]
fn track_change_updates_segment_lane_and_speaker() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);

    let _ = ok(doc.change_segment_track("a", "lane-2"));

    let segment = doc.project().segment("a").unwrap();
    assert_eq!(segment.lane_id, "lane-2");
    assert_eq!(segment.speaker_id, "speaker-2");
    assert!(doc.session().is_dirty);
    assert!(doc.can_undo());
}

#[test]
fn set_segment_speaker_keeps_the_lane_when_speakers_are_free() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);

    let outcome = ok(doc.set_segment_speaker("a", "speaker-2"));

    let segment = doc.project().segment("a").unwrap();
    assert_eq!(segment.lane_id, "lane-1");
    assert_eq!(segment.speaker_id, "speaker-2");
    assert!(outcome.status.is_some());
    assert!(doc.session().is_dirty);
    ok(doc.undo());
    assert_eq!(doc.project().segment("a").unwrap().speaker_id, "speaker-1");
}

#[test]
fn set_segment_speaker_moves_the_row_to_the_speaker_lane_when_bound() {
    let project = project(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);
    let mut environment =
        exaudio_extension::document::DocumentEnvironment::local(project.id.clone());
    environment.policy.speakers_bound_to_lanes = true;
    let mut doc = EditorDoc::from_project(project, environment);

    let _ = ok(doc.set_segment_speaker("a", "speaker-2"));

    let segment = doc.project().segment("a").unwrap();
    assert_eq!(segment.lane_id, "lane-2");
    assert_eq!(segment.speaker_id, "speaker-2");
    assert!(matches!(
        doc.add_speaker_for_segment("a", "Speaker {n}"),
        Err(EditError::Unavailable(_))
    ));
    assert_eq!(doc.project().speakers.len(), 2);
}

#[test]
fn add_speaker_for_segment_creates_and_assigns_in_one_undo_step() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);

    let speaker_id = doc
        .add_speaker_for_segment("a", "Speaker {n}")
        .expect("free speakers");

    let speaker = doc.project().speaker(&speaker_id).expect("new speaker");
    assert_eq!(speaker.label, "Speaker 3");
    assert_eq!(doc.project().segment("a").unwrap().speaker_id, speaker_id);
    assert_eq!(doc.project().segment("a").unwrap().lane_id, "lane-1");
    assert!(doc.session().is_dirty);

    ok(doc.undo());
    assert!(doc.project().speaker(&speaker_id).is_none());
    assert_eq!(doc.project().segment("a").unwrap().speaker_id, "speaker-1");
}

#[test]
fn rename_speaker_trims_and_refuses_empty_names() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);

    assert!(matches!(
        doc.rename_speaker("speaker-1", "   "),
        Err(EditError::Unavailable(_))
    ));
    let unchanged = ok(doc.rename_speaker("speaker-1", " Speaker 1 "));
    assert_eq!(unchanged, Outcome::none());
    assert!(!doc.session().is_dirty);

    let outcome = ok(doc.rename_speaker("speaker-1", "  Alice "));

    assert_eq!(doc.project().speaker("speaker-1").unwrap().label, "Alice");
    assert!(outcome.status.is_some());
    assert!(doc.session().is_dirty);
    ok(doc.undo());
    assert_eq!(
        doc.project().speaker("speaker-1").unwrap().label,
        "Speaker 1"
    );
}
