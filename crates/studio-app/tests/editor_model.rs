use std::collections::BTreeMap;

use studio_app::{
    AdminFlagPayload, BufferedPlaybackDirection, CollapsiblePanelInput, CollapsiblePanelToggleIcon,
    ContextPanelTextDirection, DiffAnnotationDiffPairInput, DiffAnnotationForMatch,
    DiffAnnotationStatus, DiffComparePair, DiffReviewActionSummary, DiffToolbarMode,
    DiffTranscriptionDiffActionsInput, DiffTranscriptionSelectionInput, DropClaimControlView,
    EditorSession, FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraft, FeedbackDraftRuntimeState,
    FeedbackDraftSuccessInput, FeedbackDraftSuccessResponse, FeedbackInput, FeedbackInputResponse,
    FeedbackRubricEntry, FeedbackSaveRequestInput, LaneWaveform, LintKind, LintSeverity,
    LoopStripClientMetrics, LoopStripHitTarget, NO_WORK_DESCRIPTION, NoWorkModalEffect,
    NoWorkModalEvent, NoWorkModalInput, ORIGINAL_SCRIPT_STORAGE_KEY, OriginalScriptPanelInput,
    PROCESSED_TRANSCRIPTION_PLACEHOLDER, READ_ONLY_SELECT_DESCRIPTION, REVIEW_ACTION_PLACEHOLDER,
    ReadOnlyActionCommand, ReviewActionStripView, ReviewEventThrottleInput, ReviewState,
    SCRIPT_CONTEXT_STORAGE_KEY, STITCHED_SELECT_DESCRIPTION, SUBMISSION_ID_COPY_RESET_DELAY_MS,
    SaveTooltipVariant, ScriptContextPanelInput, StudioWorkflow, SubmissionCopyIcon,
    SubmitDialogDescriptionTone, SubmitTooltipVariant, TaskLookupEffect, TaskLookupEvent,
    TaskLookupInitialInput, TaskLookupSelectInput, TimeRange, TrackFilter,
    TranscriptActionMenuEntryView, TranscriptAnnotationDiffSource, TranscriptAnnotationKind,
    TranscriptDiffCellKind, TranscriptRowKind, TranscriptRowVisibleCells, TranscriptTextDirection,
    TranscriptWordDiffKind, TranscriptWordDiffStatus, TtsPanelEffect, WaveformPeakSource,
    build_annotation_diff_actions, build_annotation_diff_pair, build_annotation_status_map,
    build_collapsible_panel_model, build_confirm_edit_model, build_no_work_modal_model,
    build_original_script_panel_model, build_script_context_panel_model, build_select_action_model,
    build_toolbar_feedback_button, build_transcription_diff_actions, build_transcription_diff_pair,
    choose_annotation_compare_selection, choose_transcription_compare_selection,
    clear_feedback_draft, client_x_to_loop_time, compute_assessment_phase,
    compute_assessment_retry, compute_assessment_submit_target, compute_buffered_playback_range,
    compute_feedback_draft_reset, compute_feedback_draft_save_request,
    compute_final_assessment_submit_success, compute_practice_submit_success,
    compute_review_event_throttle, create_initial_task_lookup_state, get_rubric_fields_for_flags,
    has_other_feedback_flag, is_transcription_compare_newer, match_annotation_diff_rows,
    parse_feedback_responses, read_stored_collapsed_state, reduce_collapsible_panel_toggle,
    reduce_no_work_modal_event, reduce_task_lookup_event, resolve_annotation_diff_perspective,
    resolve_submission_id_copy_failure, resolve_submission_id_copy_success,
    serialize_feedback_draft, trim_task_lookup_field_value,
};
use studio_babel::import_json;
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_waveform::DecodedAudio;

const STABLE_WORKFLOW_TASK_JSON: &str = include_str!("fixtures/stable-workflow-task.json");

fn sample_project() -> Project {
    import_json(STABLE_WORKFLOW_TASK_JSON).expect("stable workflow fixture should import")
}

fn project_with_segments(segments: Vec<Segment>) -> Project {
    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
        speakers: vec![Speaker {
            id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
        }],
        lanes: vec![AudioLane {
            id: "track-1".to_string(),
            speaker_id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
            source_path: "audio/speaker-1.wav".to_string(),
        }],
        segments,
        metadata: Default::default(),
    }
}

fn two_lane_project(mut segments: Vec<Segment>) -> Project {
    for segment in &mut segments {
        if segment.lane_id == "track-2" {
            segment.speaker_id = "speaker-2".to_string();
        }
    }

    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
        speakers: vec![
            Speaker {
                id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
            },
            Speaker {
                id: "speaker-2".to_string(),
                label: "Speaker 2".to_string(),
            },
        ],
        lanes: vec![
            AudioLane {
                id: "track-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
                source_path: "audio/speaker-1.wav".to_string(),
            },
            AudioLane {
                id: "track-2".to_string(),
                speaker_id: "speaker-2".to_string(),
                label: "Speaker 2".to_string(),
                source_path: "audio/speaker-2.wav".to_string(),
            },
        ],
        segments,
        metadata: Default::default(),
    }
}

fn segment(id: &str, start_seconds: f64, end_seconds: f64, text: &str) -> Segment {
    Segment {
        id: id.to_string(),
        source_annotation_id: Some(id.to_string()),
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds,
        end_seconds,
        text: text.to_string(),
        metadata: Default::default(),
    }
}

fn action_menu_entry_summary(entry: &TranscriptActionMenuEntryView) -> (&str, &str, bool) {
    match entry {
        TranscriptActionMenuEntryView::Item(item) => ("item", item.label.as_str(), item.enabled),
        TranscriptActionMenuEntryView::Separator => ("separator", "", false),
    }
}

#[test]
fn editor_session_defaults_to_babel_workspace_state() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    assert_eq!(session.speed, 1.0);
    assert_eq!(session.zoom, 1.0);
    assert!(!session.is_playing);
    assert!(!session.diff_view);
    assert!(!session.loop_enabled);
    assert_eq!(session.track_filter, TrackFilter::All);
    assert_eq!(session.review_state, ReviewState::Draft);
    assert_eq!(session.selected_segment_id.as_deref(), Some("annotation-1"));
    assert_eq!(session.viewport.start_seconds, 0.0);
    assert!((session.viewport.duration_seconds() - 5.0).abs() < 0.001);
    assert_eq!(session.viewport.end_seconds, 5.0);
}

#[test]
fn editor_session_initializes_review_state_from_recovered_babel_action_decision() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    project
        .metadata
        .insert("babel.format".to_string(), "review-action".to_string());
    project.metadata.insert(
        "babel.review_action.actionDecision".to_string(),
        "in-progress".to_string(),
    );

    assert_eq!(
        EditorSession::for_project(&project).review_state,
        ReviewState::InProgress
    );

    project.metadata.insert(
        "babel.review_action.actionDecision".to_string(),
        "pass".to_string(),
    );
    assert_eq!(
        EditorSession::for_project(&project).review_state,
        ReviewState::Submitted
    );
}

#[test]
fn editor_session_support_id_uses_recovered_babel_review_action_id() {
    let project = StudioWorkflow::from_babel_json(include_str!(
        "../../studio-babel/tests/fixtures/review-action-final.json"
    ))
    .expect("review action fixture should import")
    .project()
    .clone();

    let session = EditorSession::for_project(&project);

    assert_eq!(session.support_id, "9edb22ab-0993-41f3-926e-90779da4c624");
}

#[test]
fn support_id_button_matches_recovered_babel_toolbar_copy_model() {
    let project = StudioWorkflow::from_babel_json(include_str!(
        "../../studio-babel/tests/fixtures/review-action-final.json"
    ))
    .expect("review action fixture should import")
    .project()
    .clone();
    let session = EditorSession::for_project(&project);

    let idle = session.support_id_button(false);
    assert_eq!(idle.tooltip_variant, "info");
    assert_eq!(
        idle.tooltip_content,
        "ID: 9edb22ab-0993-41f3-926e-90779da4c624"
    );
    assert_eq!(idle.button_label, "Copy Support ID");
    assert_eq!(idle.icon, "copy");
    assert_eq!(idle.clipboard_text, "9edb22ab-0993-41f3-926e-90779da4c624");

    let copied = session.support_id_button(true);
    assert_eq!(copied.button_label, "Copied!");
    assert_eq!(copied.icon, "copy");
    assert_eq!(copied.clipboard_text, idle.clipboard_text);
}

#[test]
fn toolbar_feedback_button_matches_recovered_babel_visibility_and_tooltip_model() {
    let visible = build_toolbar_feedback_button(true, true);
    assert!(visible.visible);
    assert_eq!(visible.tooltip_variant, "info");
    assert_eq!(visible.tooltip_content, "Add feedback for L1 transcriber");
    assert_eq!(visible.label, "Feedback");

    assert!(!build_toolbar_feedback_button(true, false).visible);
    assert!(!build_toolbar_feedback_button(false, true).visible);
}

#[test]
fn track_filter_options_include_all_tracks_and_each_lane() {
    let project = sample_project();
    let options = EditorSession::track_filter_options(&project);

    assert_eq!(options[0], TrackFilter::All);
    assert_eq!(
        options.iter().map(TrackFilter::label).collect::<Vec<_>>(),
        vec!["All Tracks", "Speaker 1", "Speaker 2"]
    );
}

#[test]
fn transport_strip_view_matches_babel_editor_action_row_state() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.mark_dirty();

    let strip = session.transport_strip(&project);

    assert_eq!(strip.track_filter, TrackFilter::All);
    assert_eq!(
        strip
            .track_filter_options
            .iter()
            .map(TrackFilter::label)
            .collect::<Vec<_>>(),
        vec!["All Tracks", "Speaker 1", "Speaker 2"]
    );
    assert_eq!(strip.play_label, "Play");
    assert_eq!(strip.save_label, "Save progress");
    assert!(!strip.save_disabled);
    assert!(strip.save_clickable);
    assert_eq!(strip.save_tooltip_variant, SaveTooltipVariant::Warning);
    assert_eq!(strip.save_tooltip_content, "You have unsaved changes");
    assert_eq!(strip.submit_label, "Submit Review");
    assert!(strip.problem_summary_label.is_some());

    session.toggle_playback();
    session.mark_saved();
    session.review_state = ReviewState::Submitted;
    let strip = session.transport_strip(&project);

    assert_eq!(strip.play_label, "Pause");
    assert_eq!(strip.save_label, "Saved");
    assert!(!strip.save_disabled);
    assert!(!strip.save_clickable);
    assert_eq!(strip.save_tooltip_variant, SaveTooltipVariant::Default);
    assert_eq!(strip.save_tooltip_content, "All changes saved");
    assert_eq!(
        strip.review_actions,
        ReviewActionStripView::Editable { drop_claim: None }
    );
}

#[test]
fn transport_strip_exposes_babel_undo_redo_disabled_state() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip_with_history(&project, false, false);
    assert!(strip.undo_disabled);
    assert!(strip.redo_disabled);

    let strip = session.transport_strip_with_history(&project, true, false);
    assert!(!strip.undo_disabled);
    assert!(strip.redo_disabled);

    let strip = session.transport_strip_with_history(&project, false, true);
    assert!(strip.undo_disabled);
    assert!(!strip.redo_disabled);
}

#[test]
fn save_state_matches_recovered_babel_completion_and_autosave_rules() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    assert_eq!(
        session.transport_strip(&project).save_tooltip_content,
        "All changes saved"
    );
    assert!(!session.should_run_auto_save_tick(false));

    session.mark_dirty();
    assert!(session.should_run_auto_save_tick(false));
    assert!(!session.should_run_auto_save_tick(true));

    session.begin_save();
    let strip = session.transport_strip(&project);
    assert_eq!(strip.save_label, "Saving...");
    assert!(strip.save_disabled);
    assert!(strip.save_clickable);
    assert!(!session.should_run_auto_save_tick(false));

    session.complete_save(false, "remote save timed out");
    let strip = session.transport_strip(&project);
    assert_eq!(strip.save_label, "Error - Retry");
    assert!(!strip.save_disabled);
    assert!(strip.save_clickable);
    assert_eq!(strip.save_tooltip_variant, SaveTooltipVariant::Warning);
    assert_eq!(
        strip.save_tooltip_content,
        "Save failed: remote save timed out"
    );
    assert!(session.is_dirty);
    assert!(session.last_saved_at.is_none());

    session.begin_save();
    assert!(!session.save_has_error);
    session.complete_save(true, "2026-06-22T12:01:00Z");
    let strip = session.transport_strip(&project);
    assert_eq!(strip.save_label, "Saved");
    assert!(!strip.save_disabled);
    assert!(!strip.save_clickable);
    assert_eq!(strip.save_tooltip_content, "All changes saved");
    assert!(!session.is_dirty);
    assert_eq!(
        session.last_saved_at.as_deref(),
        Some("2026-06-22T12:01:00Z")
    );
}

#[test]
fn save_completion_only_clears_the_revision_that_was_sent() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.mark_dirty();
    session.begin_save();
    let saved_revision = session.saving_revision().expect("save revision");
    session.mark_dirty();

    assert!(session.complete_save_for_revision(saved_revision, true, "2026-07-09T10:00:00Z"));
    assert!(session.is_dirty);
    assert!(!session.is_saving);
    assert!(session.should_run_auto_save_tick(false));
    assert_eq!(session.edit_revision(), saved_revision + 1);
    assert_eq!(
        session.last_saved_at.as_deref(),
        Some("2026-07-09T10:00:00Z")
    );
}

#[test]
fn mismatched_save_completion_cannot_finish_the_active_save() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.mark_dirty();
    session.begin_save();
    let active_revision = session.saving_revision().expect("save revision");

    assert!(!session.complete_save_for_revision(active_revision + 1, true, "stale callback"));
    assert!(session.is_dirty);
    assert!(session.is_saving);
    assert_eq!(session.saving_revision(), Some(active_revision));
    assert!(session.last_saved_at.is_none());
}

#[test]
fn save_retry_tooltip_reports_save_failure_reason() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.mark_dirty();
    session.begin_save();
    session.complete_save(false, "Babel rejected annotation payload");

    let strip = session.transport_strip(&project);
    assert_eq!(strip.save_label, "Error - Retry");
    assert!(strip.save_clickable);
    assert_eq!(strip.save_tooltip_variant, SaveTooltipVariant::Warning);
    assert_eq!(
        strip.save_tooltip_content,
        "Save failed: Babel rejected annotation payload"
    );
}

#[test]
fn review_event_throttle_matches_recovered_babel_twenty_second_gate() {
    let mut last_emitted_at_by_key = BTreeMap::new();

    let disabled = compute_review_event_throttle(ReviewEventThrottleInput {
        enabled: false,
        review_action_id: "review-1",
        event_type: "play",
        now_ms: 1_000,
        last_emitted_at_by_key: &last_emitted_at_by_key,
    });

    assert!(!disabled.should_emit);
    assert_eq!(disabled.key, "review-1:play");
    assert_eq!(disabled.next_last_emitted_at, None);

    last_emitted_at_by_key.insert("review-1:play".to_string(), 5_000);

    let disabled_with_previous = compute_review_event_throttle(ReviewEventThrottleInput {
        enabled: false,
        review_action_id: "review-1",
        event_type: "play",
        now_ms: 30_000,
        last_emitted_at_by_key: &last_emitted_at_by_key,
    });
    assert!(!disabled_with_previous.should_emit);
    assert_eq!(disabled_with_previous.next_last_emitted_at, Some(5_000));

    let suppressed = compute_review_event_throttle(ReviewEventThrottleInput {
        enabled: true,
        review_action_id: "review-1",
        event_type: "play",
        now_ms: 24_999,
        last_emitted_at_by_key: &last_emitted_at_by_key,
    });
    assert!(!suppressed.should_emit);
    assert_eq!(suppressed.next_last_emitted_at, Some(5_000));

    let emitted = compute_review_event_throttle(ReviewEventThrottleInput {
        enabled: true,
        review_action_id: "review-1",
        event_type: "play",
        now_ms: 25_000,
        last_emitted_at_by_key: &last_emitted_at_by_key,
    });
    assert!(emitted.should_emit);
    assert_eq!(emitted.next_last_emitted_at, Some(25_000));
}

#[test]
fn buffered_playback_range_matches_recovered_babel_timeout_padding() {
    let before = compute_buffered_playback_range(10.0, 500.0, BufferedPlaybackDirection::Before)
        .expect("finite before range should resolve");

    assert_eq!(before.start_seconds, 9.5);
    assert_eq!(before.end_seconds, 10.0);
    assert_eq!(before.timeout_ms, 700.0);

    let clamped_before =
        compute_buffered_playback_range(0.25, 500.0, BufferedPlaybackDirection::Before)
            .expect("before range should clamp at zero");
    assert_eq!(clamped_before.start_seconds, 0.0);
    assert_eq!(clamped_before.end_seconds, 0.25);
    assert_eq!(clamped_before.timeout_ms, 450.0);

    let after = compute_buffered_playback_range(10.0, -500.0, BufferedPlaybackDirection::After)
        .expect("after range should use absolute buffer length");
    assert_eq!(after.start_seconds, 10.0);
    assert_eq!(after.end_seconds, 10.5);
    assert_eq!(after.timeout_ms, 700.0);

    assert!(
        compute_buffered_playback_range(f64::NAN, 500.0, BufferedPlaybackDirection::After)
            .is_none()
    );
    assert!(
        compute_buffered_playback_range(10.0, f64::INFINITY, BufferedPlaybackDirection::After)
            .is_none()
    );
}

#[test]
fn transport_strip_switches_to_recovered_read_only_notice() {
    let mut project = sample_project();
    project
        .metadata
        .insert("studio.babel.readOnly".to_string(), "true".to_string());
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip(&project);

    assert_eq!(
        strip.review_actions,
        ReviewActionStripView::ReadOnly {
            notice: Some("No changes will persist during this read only session.".to_string()),
            actions: Vec::new(),
        }
    );
}

#[test]
fn transport_strip_exposes_recovered_drop_claim_control_when_available() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.dropClaim.available".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip(&project);

    assert_eq!(
        strip.review_actions,
        ReviewActionStripView::Editable {
            drop_claim: Some(DropClaimControlView {
                label: "Drop Claim".to_string(),
                disabled: false,
                tooltip: "Drop action for another worker".to_string(),
            })
        }
    );

    project.metadata.insert(
        "studio.babel.dropClaim.isDropping".to_string(),
        "true".to_string(),
    );
    let strip = session.transport_strip(&project);

    assert_eq!(
        strip.review_actions,
        ReviewActionStripView::Editable {
            drop_claim: Some(DropClaimControlView {
                label: "Dropping...".to_string(),
                disabled: true,
                tooltip: "Drop action for another worker".to_string(),
            })
        }
    );
}

#[test]
fn transport_strip_matches_recovered_practice_read_only_actions() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.phase".to_string(),
        "practice".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.retryCount".to_string(),
        "1".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip(&project);

    match strip.review_actions {
        ReviewActionStripView::ReadOnly { notice, actions } => {
            assert_eq!(notice, None);
            assert_eq!(
                actions
                    .iter()
                    .map(|action| (action.label.as_str(), action.command, action.disabled))
                    .collect::<Vec<_>>(),
                vec![
                    (
                        "Retry Practice",
                        ReadOnlyActionCommand::RetryPractice,
                        false
                    ),
                    (
                        "Take Assessment",
                        ReadOnlyActionCommand::TakeAssessment,
                        false
                    ),
                ]
            );
        }
        ReviewActionStripView::Editable { .. } => {
            panic!("practice read-only mode should not be editable")
        }
    }
}

#[test]
fn transport_strip_matches_recovered_assessment_read_only_actions() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.phase".to_string(),
        "assessment".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip(&project);

    match strip.review_actions {
        ReviewActionStripView::ReadOnly { notice, actions } => {
            assert_eq!(
                notice.as_deref(),
                Some("No changes will persist during this assessment session.")
            );
            assert_eq!(
                actions
                    .iter()
                    .map(|action| (action.label.as_str(), action.command, action.disabled))
                    .collect::<Vec<_>>(),
                vec![("Retry", ReadOnlyActionCommand::RetryAssessment, false)]
            );
        }
        ReviewActionStripView::Editable { .. } => {
            panic!("assessment read-only mode should not be editable")
        }
    }
}

#[test]
fn transport_strip_matches_recovered_final_submitted_actions() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.phase".to_string(),
        "assessment".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.isFinalSubmitted".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip(&project);

    match strip.review_actions {
        ReviewActionStripView::ReadOnly { notice, actions } => {
            assert_eq!(
                notice.as_deref(),
                Some("Submitted successfully! Our team will review and get back to you soon.")
            );
            assert_eq!(
                actions
                    .iter()
                    .map(|action| (action.label.as_str(), action.command, action.disabled))
                    .collect::<Vec<_>>(),
                vec![(
                    "Back to Projects",
                    ReadOnlyActionCommand::BackToProjects,
                    false
                )]
            );
        }
        ReviewActionStripView::Editable { .. } => {
            panic!("submitted assessment mode should not be editable")
        }
    }
}

#[test]
fn transcript_rows_mark_recovered_read_only_state() {
    let mut project = sample_project();
    project
        .metadata
        .insert("studio.babel.readOnly".to_string(), "true".to_string());
    let session = EditorSession::for_project(&project);

    assert!(
        session
            .transcript_rows(&project)
            .iter()
            .all(|row| row.is_read_only)
    );

    project.metadata.remove("studio.babel.readOnly");
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );

    assert!(
        session
            .transcript_rows(&project)
            .iter()
            .all(|row| row.is_read_only)
    );
}

#[test]
fn transcript_rows_mark_recovered_babel_active_rows_from_track_region_map() {
    let mut track_two = segment("track-two-active", 1.0, 2.0, "track two");
    track_two.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("selected-track-one", 0.0, 0.8, "track one"),
        track_two,
    ]);
    let mut session = EditorSession::for_project(&project);
    session.selected_segment_id = Some("selected-track-one".to_string());
    session
        .active_region_by_track
        .insert("track-2".to_string(), "track-two-active".to_string());

    let rows = session.transcript_rows(&project);
    let selected_row = rows
        .iter()
        .find(|row| row.segment_id == "selected-track-one")
        .expect("selected row should be present");
    let active_row = rows
        .iter()
        .find(|row| row.segment_id == "track-two-active")
        .expect("active row should be present");

    assert!(selected_row.is_selected);
    assert!(!selected_row.is_active);
    assert!(!active_row.is_selected);
    assert!(active_row.is_active);
    assert_eq!(
        session.active_transcript_row_id(&project).as_deref(),
        Some("track-two-active")
    );

    session.track_filter = TrackFilter::lane("track-1".to_string(), "Speaker 1".to_string());

    assert_eq!(session.active_transcript_row_id(&project), None);
}

#[test]
fn transcript_empty_state_matches_recovered_babel_colspan_and_copy() {
    let transcription_project = project_with_segments(Vec::new());
    let session = EditorSession::for_project(&transcription_project);

    let empty = session.transcript_empty_state(&transcription_project);

    assert_eq!(empty.col_span, 7);
    assert_eq!(empty.title, "No transcriptions yet");
    assert_eq!(
        empty.description,
        "Click and drag on the waveform above to create a region, then add your transcription text."
    );

    let mut annotation_project = project_with_segments(Vec::new());
    annotation_project.metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    annotation_project.metadata.insert(
        "studio.babel.category.name".to_string(),
        "Background Noise".to_string(),
    );

    let annotation_empty = session.transcript_empty_state(&annotation_project);

    assert_eq!(annotation_empty.col_span, 6);
    assert_eq!(annotation_empty.title, "No annotations yet");
    assert_eq!(
        annotation_empty.description,
        "Click and drag on the waveform above to annotate regions with background noise."
    );

    annotation_project.metadata.insert(
        "studio.babel.category.usesHierarchicalLabels".to_string(),
        "true".to_string(),
    );

    assert_eq!(
        session.transcript_empty_state(&annotation_project).col_span,
        8
    );
}

#[test]
fn transport_strip_view_carries_babel_problem_summary_for_action_row() {
    let project = project_with_segments(vec![Segment {
        id: "annotation-1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 0.0,
        end_seconds: 1.0,
        text: String::new(),
        metadata: Default::default(),
    }]);
    let session = EditorSession::for_project(&project);

    let strip = session.transport_strip(&project);

    assert_eq!(strip.problem_summary_label.as_deref(), Some("1 error"));
}

#[test]
fn submit_control_matches_recovered_babel_clean_default_copy() {
    let project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    let session = EditorSession::for_project(&project);

    let submit = session.submit_control(&project, false, false, false);

    assert_eq!(submit.trigger_button, "Submit Review");
    assert_eq!(
        submit.trigger_tooltip_variant,
        SubmitTooltipVariant::Success
    );
    assert_eq!(submit.trigger_tooltip_content, "Submit your review");
    assert!(!submit.trigger_disabled);
    assert!(!submit.open);
    assert!(!submit.show_pending_overlay);
    assert_eq!(submit.pending_overlay_text, "Submitting your review...");
    assert_eq!(submit.dialog_title, "Confirm Submission");
    assert_eq!(
        submit.dialog_description,
        "Are you sure you want to submit?"
    );
    assert_eq!(
        submit.dialog_description_tone,
        SubmitDialogDescriptionTone::Default
    );
    assert_eq!(submit.cancel_label, "Cancel");
    assert!(!submit.cancel_disabled);
    assert_eq!(submit.confirm_label, "Submit");
    assert!(!submit.confirm_disabled);
}

#[test]
fn submit_control_blocks_recovered_babel_lint_errors_before_dialog() {
    let project = project_with_segments(vec![segment("empty", 1.0, 2.0, "")]);
    let session = EditorSession::for_project(&project);

    let submit = session.submit_control(&project, false, false, false);

    assert_eq!(
        submit.trigger_tooltip_variant,
        SubmitTooltipVariant::Destructive
    );
    assert_eq!(submit.trigger_tooltip_content, "Please fix linter errors");
    assert!(submit.trigger_disabled);
}

#[test]
fn submit_control_uses_recovered_babel_practice_copy_and_pending_state() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    project.metadata.insert(
        "studio.babel.diff.assessmentPhase".to_string(),
        "practice".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let submit = session.submit_control(&project, true, true, false);

    assert!(submit.open);
    assert_eq!(submit.trigger_button, "Submit Practice");
    assert_eq!(submit.dialog_title, "Submit Practice Attempt");
    assert!(submit.dialog_description.contains("Take Assessment"));
    assert_eq!(submit.confirm_label, "Submitting...");
    assert!(submit.confirm_disabled);
    assert!(submit.cancel_disabled);
    assert!(submit.show_pending_overlay);
}

#[test]
fn submit_control_uses_recovered_babel_error_copy_and_annotation_blockers() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    project.metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    project.metadata.insert(
        "studio.babel.submit.hasMissingIntensity".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let submit = session.submit_control(&project, true, false, true);

    assert_eq!(
        submit.trigger_tooltip_content,
        "Please set intensity for all annotations"
    );
    assert_eq!(
        submit.trigger_tooltip_variant,
        SubmitTooltipVariant::Destructive
    );
    assert!(submit.trigger_disabled);
    assert_eq!(
        submit.dialog_description,
        "Failed to submit annotations. Please try again."
    );
    assert_eq!(
        submit.dialog_description_tone,
        SubmitDialogDescriptionTone::Error
    );
}

#[test]
fn submit_control_matches_recovered_babel_blocking_priority_chain() {
    let mut annotation_project = project_with_segments(vec![segment("empty", 1.0, 2.0, "")]);
    annotation_project.metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    annotation_project.metadata.insert(
        "studio.babel.submit.hasFeedbackIncomplete".to_string(),
        "true".to_string(),
    );
    let annotation_session = EditorSession::for_project(&annotation_project);

    let annotation_submit =
        annotation_session.submit_control(&annotation_project, false, false, false);

    assert_eq!(
        annotation_submit.trigger_tooltip_variant,
        SubmitTooltipVariant::Success
    );
    assert_eq!(
        annotation_submit.trigger_tooltip_content,
        "Submit your review"
    );
    assert!(!annotation_submit.trigger_disabled);

    annotation_project.metadata.insert(
        "studio.babel.submit.hasMissingIntensity".to_string(),
        "true".to_string(),
    );
    let missing_intensity_submit =
        annotation_session.submit_control(&annotation_project, false, false, false);
    assert_eq!(
        missing_intensity_submit.trigger_tooltip_content,
        "Please set intensity for all annotations"
    );

    let warning_project = project_with_segments(vec![segment(
        "warning",
        1.0,
        60.0,
        "very long but intentionally valid",
    )]);
    let warning_session = EditorSession::for_project(&warning_project);
    let warning_submit = warning_session.submit_control(&warning_project, false, false, false);
    assert_eq!(
        warning_submit.trigger_tooltip_content,
        "Please acknowledge all warnings"
    );

    let mut low_confidence = segment("low-confidence", 1.0, 2.0, "alpha beta");
    low_confidence.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "word": "beta", "start": 1.2, "end": 1.4, "isYellow": true }
        ]),
    );
    let mut low_confidence_project = project_with_segments(vec![low_confidence]);
    low_confidence_project.metadata.insert(
        "studio.babel.submit.isFeedbackSaving".to_string(),
        "true".to_string(),
    );
    low_confidence_project.metadata.insert(
        "studio.babel.submit.hasFeedbackIncomplete".to_string(),
        "true".to_string(),
    );
    let low_confidence_session = EditorSession::for_project(&low_confidence_project);
    let low_confidence_submit =
        low_confidence_session.submit_control(&low_confidence_project, false, false, false);
    assert_eq!(
        low_confidence_submit.trigger_tooltip_content,
        "Please review all low-confidence words"
    );

    let mut feedback_project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    feedback_project.metadata.insert(
        "studio.babel.submit.isFeedbackSaving".to_string(),
        "true".to_string(),
    );
    feedback_project.metadata.insert(
        "studio.babel.submit.hasFeedbackIncomplete".to_string(),
        "true".to_string(),
    );
    let feedback_session = EditorSession::for_project(&feedback_project);
    let saving_submit = feedback_session.submit_control(&feedback_project, false, false, false);
    assert_eq!(
        saving_submit.trigger_tooltip_variant,
        SubmitTooltipVariant::Warning
    );
    assert_eq!(saving_submit.trigger_tooltip_content, "Saving feedback...");

    feedback_project
        .metadata
        .remove("studio.babel.submit.isFeedbackSaving");
    let incomplete_submit = feedback_session.submit_control(&feedback_project, false, false, false);
    assert_eq!(
        incomplete_submit.trigger_tooltip_content,
        "Please complete the feedback form"
    );
}

#[test]
fn admin_flag_control_matches_recovered_babel_copy_and_payload() {
    let model = EditorSession::admin_flag_control(true, "  poor acting  ", false);

    assert!(model.open);
    assert_eq!(model.trigger_label, "Flag Recording");
    assert_eq!(model.title, "Flag this recording?");
    assert!(model.description.contains("Admin rating marks"));
    assert_eq!(model.reason_label, "Reason (optional)");
    assert_eq!(
        model.reason_placeholder,
        "e.g. background noise, wrong emotion, mispronunciation..."
    );
    assert_eq!(model.reason_value, "  poor acting  ");
    assert!(!model.reason_disabled);
    assert_eq!(model.cancel_label, "Cancel");
    assert_eq!(model.confirm_label, "Confirm Flag");
    assert_eq!(
        EditorSession::admin_flag_payload(&model.reason_value),
        AdminFlagPayload {
            rating: 1,
            rating_reason: Some("poor acting".to_string()),
        }
    );
    assert_eq!(
        EditorSession::admin_flag_payload("   "),
        AdminFlagPayload {
            rating: 1,
            rating_reason: None,
        }
    );
}

#[test]
fn skip_chunk_control_matches_recovered_babel_copy_and_disabled_state() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    project
        .metadata
        .insert("studio.babel.skip.canSkip".to_string(), "false".to_string());

    let model = EditorSession::skip_chunk_control(&project, true, false);

    assert!(model.open);
    assert_eq!(model.trigger_label, "Skip Chunk");
    assert!(model.trigger_disabled);
    assert_eq!(
        model.trigger_tooltip.as_deref(),
        Some("Listen to this chunk for a bit longer before skipping.")
    );
    assert_eq!(model.title, "Skip this chunk?");
    assert!(model.description.contains("daily skip cap"));
    assert_eq!(model.cancel_label, "Cancel");
    assert_eq!(model.confirm_label, "Confirm Skip");

    let pending = EditorSession::skip_chunk_control(&project, true, true);
    assert_eq!(pending.confirm_label, "Skipping...");
    assert!(pending.confirm_disabled);
    assert!(pending.cancel_disabled);
}

#[test]
fn diff_toolbar_matches_babel_no_versions_state() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    let toolbar = session.diff_toolbar(&project);

    assert_eq!(toolbar.mode, DiffToolbarMode::Normal);
    assert!(!toolbar.show_reviewers_control);
    assert!(!toolbar.reviewers_checked);
    assert_eq!(toolbar.diff_toggle.label, "Diff View");
    assert!(!toolbar.diff_toggle.checked);
    assert!(toolbar.diff_toggle.disabled);
    assert_eq!(
        toolbar.no_versions_text.as_deref(),
        Some("No previous versions to compare")
    );
    assert!(!toolbar.compare_select.visible);
    assert!(!toolbar.unsaved_changes_badge.visible);
    assert!(!toolbar.metrics.visible);
    assert_eq!(
        toolbar
            .metrics
            .legend
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        vec!["Substituted", "Added", "Removed"]
    );
}

#[test]
fn diff_toolbar_gates_reviewer_control_like_recovered_babel() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.diff.hasReviewerToggle".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let toolbar = session.diff_toolbar(&project);

    assert!(toolbar.show_reviewers_control);
    assert!(toolbar.reviewers_checked);

    project.metadata.insert(
        "studio.babel.diff.assessmentPhase".to_string(),
        "practice".to_string(),
    );
    let toolbar = session.diff_toolbar(&project);

    assert_eq!(toolbar.mode, DiffToolbarMode::Assessment);
    assert_eq!(toolbar.assessment_label.as_deref(), Some("Practice Mode"));
    assert!(!toolbar.show_reviewers_control);
    assert!(!toolbar.diff_toggle.visible);
    assert!(!toolbar.compare_select.visible);
}

#[test]
fn diff_controller_matches_recovered_babel_compare_action_defaults_and_pairs() {
    let review_actions = vec![
        DiffReviewActionSummary {
            id: "l0".to_string(),
            level: 0,
            label: None,
            created_at: Some("old".to_string()),
            worker_id: Some("w0".to_string()),
        },
        DiffReviewActionSummary {
            id: "l1".to_string(),
            level: 1,
            label: None,
            created_at: Some("mid".to_string()),
            worker_id: Some("w1".to_string()),
        },
        DiffReviewActionSummary {
            id: "l3".to_string(),
            level: 3,
            label: None,
            created_at: Some("new".to_string()),
            worker_id: Some("w3".to_string()),
        },
    ];

    let assessment_actions = build_transcription_diff_actions(DiffTranscriptionDiffActionsInput {
        is_assessment_mode: true,
        assessment_answer_review_action_id: Some("answer"),
        review_actions: &review_actions,
        now_iso: "2026-06-21T00:00:00.000Z",
    });
    assert_eq!(
        assessment_actions,
        vec![DiffReviewActionSummary {
            id: "answer".to_string(),
            level: 0,
            label: Some("Answer".to_string()),
            created_at: Some("2026-06-21T00:00:00.000Z".to_string()),
            worker_id: None,
        }]
    );

    let transcription_actions =
        build_transcription_diff_actions(DiffTranscriptionDiffActionsInput {
            is_assessment_mode: false,
            assessment_answer_review_action_id: None,
            review_actions: &review_actions,
            now_iso: "unused",
        });
    assert_eq!(
        transcription_actions
            .iter()
            .map(|action| action.label.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("L0"), Some("L1"), Some("L3")]
    );

    let annotation_actions = build_annotation_diff_actions(&review_actions);
    assert_eq!(
        annotation_actions
            .iter()
            .map(|action| action.label.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("L0"), Some("L1"), Some("L3")]
    );

    assert_eq!(
        choose_transcription_compare_selection(DiffTranscriptionSelectionInput {
            is_opening: true,
            selected_compare_action_id: Some("stale"),
            available_actions: &annotation_actions,
            current_level: 2,
            is_assessment_mode: false,
        })
        .as_deref(),
        Some("stale")
    );
    assert_eq!(
        choose_transcription_compare_selection(DiffTranscriptionSelectionInput {
            is_opening: true,
            selected_compare_action_id: None,
            available_actions: &annotation_actions,
            current_level: 2,
            is_assessment_mode: false,
        })
        .as_deref(),
        Some("l1")
    );
    assert_eq!(
        choose_transcription_compare_selection(DiffTranscriptionSelectionInput {
            is_opening: false,
            selected_compare_action_id: Some("l1"),
            available_actions: &annotation_actions,
            current_level: 2,
            is_assessment_mode: false,
        }),
        None
    );
    assert_eq!(
        choose_transcription_compare_selection(DiffTranscriptionSelectionInput {
            is_opening: true,
            selected_compare_action_id: None,
            available_actions: &annotation_actions,
            current_level: 2,
            is_assessment_mode: true,
        })
        .as_deref(),
        Some("l0")
    );

    assert!(is_transcription_compare_newer(
        false,
        Some("l3"),
        &annotation_actions,
        2,
    ));
    assert!(is_transcription_compare_newer(true, Some("stale"), &[], 2,));
    assert_eq!(
        build_transcription_diff_pair("current", Some("l3"), true),
        Some(DiffComparePair {
            reference_review_action_id: "current".to_string(),
            current_review_action_id: "l3".to_string(),
        })
    );
    assert_eq!(
        build_transcription_diff_pair("current", Some("l1"), false),
        Some(DiffComparePair {
            reference_review_action_id: "l1".to_string(),
            current_review_action_id: "current".to_string(),
        })
    );

    assert_eq!(
        choose_annotation_compare_selection(true, &annotation_actions, 2).as_deref(),
        Some("l1")
    );
    assert_eq!(
        choose_annotation_compare_selection(false, &annotation_actions, 2),
        None
    );
    assert_eq!(
        build_annotation_diff_pair(DiffAnnotationDiffPairInput {
            current_review_action_id: "current",
            selected_compare_action_id: Some("l3"),
            selected_compare_action_level: Some(3),
            current_level: 2,
        }),
        Some(DiffComparePair {
            reference_review_action_id: "l3".to_string(),
            current_review_action_id: "current".to_string(),
        })
    );
    assert_eq!(
        resolve_annotation_diff_perspective(Some(3), 2),
        Some(TranscriptAnnotationDiffSource::Prediction)
    );
    assert_eq!(
        resolve_annotation_diff_perspective(Some(1), 2),
        Some(TranscriptAnnotationDiffSource::Reference)
    );
}

#[test]
fn annotation_diff_matching_matches_recovered_babel_overlap_status_and_ties() {
    let reference_annotations = vec![
        DiffAnnotationForMatch {
            id: "r1".to_string(),
            primary_label: Some("noise".to_string()),
            secondary_label: Some("wind".to_string()),
            intensity: Some(2),
            start_seconds: Some(0.0),
            end_seconds: Some(2.0),
        },
        DiffAnnotationForMatch {
            id: "r2".to_string(),
            primary_label: Some("noise".to_string()),
            secondary_label: Some("tap".to_string()),
            intensity: Some(1),
            start_seconds: Some(3.0),
            end_seconds: Some(4.0),
        },
    ];
    let prediction_annotations = vec![
        DiffAnnotationForMatch {
            id: "p1".to_string(),
            primary_label: Some("noise".to_string()),
            secondary_label: Some("wind".to_string()),
            intensity: Some(3),
            start_seconds: Some(0.2),
            end_seconds: Some(2.2),
        },
        DiffAnnotationForMatch {
            id: "p2".to_string(),
            primary_label: Some("other".to_string()),
            secondary_label: Some("tap".to_string()),
            intensity: Some(1),
            start_seconds: Some(8.0),
            end_seconds: Some(9.0),
        },
    ];

    let rows = match_annotation_diff_rows(&reference_annotations, &prediction_annotations);

    assert_eq!(
        rows.iter()
            .map(|row| (
                row.annotation.id.as_str(),
                row.source,
                row.status,
                row.matched_annotation_id.as_deref(),
                row.other_intensity,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "r1",
                TranscriptAnnotationDiffSource::Reference,
                DiffAnnotationStatus::IntensityMismatch,
                Some("p1"),
                Some(3),
            ),
            (
                "p1",
                TranscriptAnnotationDiffSource::Prediction,
                DiffAnnotationStatus::IntensityMismatch,
                Some("r1"),
                Some(2),
            ),
            (
                "r2",
                TranscriptAnnotationDiffSource::Reference,
                DiffAnnotationStatus::UnmatchedReference,
                None,
                None,
            ),
            (
                "p2",
                TranscriptAnnotationDiffSource::Prediction,
                DiffAnnotationStatus::UnmatchedPrediction,
                None,
                None,
            ),
        ]
    );

    let tie_rows = match_annotation_diff_rows(
        &[DiffAnnotationForMatch {
            id: "r-tie".to_string(),
            primary_label: None,
            secondary_label: None,
            intensity: None,
            start_seconds: Some(0.0),
            end_seconds: Some(10.0),
        }],
        &[
            DiffAnnotationForMatch {
                id: "p-first".to_string(),
                primary_label: None,
                secondary_label: None,
                intensity: None,
                start_seconds: Some(0.0),
                end_seconds: Some(5.0),
            },
            DiffAnnotationForMatch {
                id: "p-second".to_string(),
                primary_label: None,
                secondary_label: None,
                intensity: None,
                start_seconds: Some(5.0),
                end_seconds: Some(10.0),
            },
        ],
    );
    assert_eq!(
        tie_rows
            .iter()
            .map(|row| {
                (
                    row.annotation.id.as_str(),
                    row.matched_annotation_id.as_deref(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            ("r-tie", Some("p-first")),
            ("p-first", Some("r-tie")),
            ("p-second", None),
        ]
    );

    assert!(
        build_annotation_status_map(
            false,
            Some(TranscriptAnnotationDiffSource::Reference),
            &rows
        )
        .is_none()
    );
    let status_map =
        build_annotation_status_map(true, Some(TranscriptAnnotationDiffSource::Reference), &rows)
            .expect("diff mode with perspective should build status map");
    assert_eq!(
        status_map.get("r1").map(|entry| (
            entry.status,
            entry.other_intensity,
            entry.counterpart_start_seconds,
            entry.counterpart_end_seconds,
        )),
        Some((
            DiffAnnotationStatus::IntensityMismatch,
            Some(3),
            Some(0.2),
            Some(2.2),
        ))
    );
    assert_eq!(
        status_map.get("r2").map(|entry| entry.status),
        Some(DiffAnnotationStatus::UnmatchedPrediction)
    );
}

#[test]
fn diff_toolbar_matches_babel_active_metrics_state() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.diff.availableReviewActions".to_string(),
        serde_json::json!([
            { "id": "l0", "label": "L0" },
            { "id": "l1", "label": "L1" }
        ])
        .to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.metrics".to_string(),
        serde_json::json!({
            "timestampMetrics": {
                "timeline": { "f1": 0.9, "recall": 0.8, "precision": 0.7 },
                "segments": {
                    "matchedSegments": 3,
                    "totalSegments": 4,
                    "within50ms": 25,
                    "within100ms": 50,
                    "within200ms": 75,
                    "avgShiftMs": 42
                }
            },
            "overallWordErrorRate": 0.12345,
            "totalSubstitutions": 2,
            "totalInsertions": 3,
            "totalDeletions": 4
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();
    session.mark_dirty();

    let toolbar = session.diff_toolbar(&project);

    assert_eq!(toolbar.mode, DiffToolbarMode::Normal);
    assert!(toolbar.diff_toggle.checked);
    assert!(!toolbar.diff_toggle.disabled);
    assert!(toolbar.unsaved_changes_badge.visible);
    assert_eq!(toolbar.unsaved_changes_badge.text, "Unsaved changes");
    assert!(toolbar.compare_select.visible);
    assert_eq!(toolbar.compare_select.label, "Compare:");
    assert_eq!(toolbar.compare_select.value.as_deref(), Some("l0"));
    assert_eq!(
        toolbar
            .compare_select
            .options
            .iter()
            .map(|option| (option.value.as_str(), option.label.as_str()))
            .collect::<Vec<_>>(),
        vec![("l0", "L0"), ("l1", "L1")]
    );

    assert!(toolbar.metrics.visible);
    assert_eq!(toolbar.metrics.avg_shift.as_ref().unwrap().value, "42ms");
    assert_eq!(toolbar.metrics.word_diff.as_ref().unwrap().value, "12.35%");
    assert_eq!(toolbar.metrics.substitutions.as_ref().unwrap().value, 2);
    assert_eq!(toolbar.metrics.additions.as_ref().unwrap().value, 3);
    assert_eq!(toolbar.metrics.removals.as_ref().unwrap().value, 4);
    let tooltip = toolbar.metrics.timestamp_tooltip.as_ref().unwrap();
    assert_eq!(tooltip.title, "Timestamp Accuracy");
    assert_eq!(tooltip.f1_score.value, "90.0%");
    assert_eq!(tooltip.matched_segments_text, "3/4 segments matched");
    assert_eq!(
        tooltip
            .tolerance_bars
            .iter()
            .map(|bar| bar.label.as_str())
            .collect::<Vec<_>>(),
        vec!["±50ms", "±100ms", "±200ms"]
    );
    assert_eq!(tooltip.toggle_label, "What do these mean?");
    assert!(!tooltip.descriptions_visible);
    assert_eq!(
        tooltip
            .descriptions
            .iter()
            .map(|description| description.label.as_str())
            .collect::<Vec<_>>(),
        vec![
            "F1 Score",
            "Recall",
            "Precision",
            "Segments matched & tolerance"
        ]
    );
}

#[test]
fn diff_toolbar_matches_babel_annotation_metrics_state() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.diff.annotationToolbar".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.availableReviewActions".to_string(),
        serde_json::json!([
            { "id": "l0", "label": "L0", "workerName": "Ira" },
            { "id": "l1", "label": "L1" }
        ])
        .to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.annotationMetrics".to_string(),
        serde_json::json!({
            "eventDetection": { "f1": 0.91, "precision": 0.82, "recall": 0.73 },
            "labelMacroAverage": { "f1": 0.64, "precision": 0.55, "recall": 0.46 },
            "intensityCalibration": { "f1": 0.37, "precision": 0.28, "recall": 0.19 }
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let toolbar = session.diff_toolbar(&project);

    assert_eq!(toolbar.diff_toggle.id, "annotation-diff-mode");
    assert_eq!(toolbar.diff_toggle.label, "Annotation Diff");
    assert_eq!(toolbar.compare_select.placeholder, "Select version…");
    assert_eq!(toolbar.compare_select.loading_text, "Loading…");
    assert_eq!(
        toolbar.compare_select.empty_text,
        "No other versions available"
    );
    assert_eq!(
        toolbar
            .compare_select
            .options
            .iter()
            .map(|option| option.label.as_str())
            .collect::<Vec<_>>(),
        vec!["L0 · Ira", "L1"]
    );
    assert!(toolbar.metrics.visible);
    assert_eq!(toolbar.metrics.event.as_ref().unwrap().label, "Event");
    assert_eq!(toolbar.metrics.event.as_ref().unwrap().value, "91.0%");
    assert_eq!(
        toolbar.metrics.label_accuracy.as_ref().unwrap().label,
        "Label"
    );
    assert_eq!(
        toolbar.metrics.label_accuracy.as_ref().unwrap().value,
        "64.0%"
    );
    assert_eq!(
        toolbar.metrics.intensity.as_ref().unwrap().label,
        "Intensity"
    );
    assert_eq!(toolbar.metrics.intensity.as_ref().unwrap().value, "37.0%");
    let event_tooltip = toolbar
        .metrics
        .event
        .as_ref()
        .unwrap()
        .tooltip
        .as_ref()
        .unwrap();
    assert_eq!(event_tooltip.title, "Event Detection");
    assert_eq!(event_tooltip.toggle_label, "What does this mean?");
    assert_eq!(
        event_tooltip
            .descriptions
            .iter()
            .map(|description| description.label.as_str())
            .collect::<Vec<_>>(),
        vec!["F1 Score", "Precision", "Recall"]
    );
    assert_eq!(
        toolbar
            .metrics
            .legend
            .iter()
            .map(|item| (item.label.as_str(), item.tooltip_title.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            ("Added", Some("Added")),
            ("Substituted", Some("Substituted")),
            ("Removed", Some("Removed")),
            ("Unchanged", Some("Unchanged")),
        ]
    );
}

#[test]
fn transcript_rows_respect_selected_track_filter_and_format_times() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.track_filter = TrackFilter::lane("track-speaker-2", "Speaker 2");

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 9);
    assert_eq!(rows[0].display_id, "1");
    assert_eq!(rows[0].speaker_label, "Speaker 2");
    assert_eq!(rows[0].start_time, "00:03.50");
    assert_eq!(rows[0].end_time, "00:05.10");
    assert_eq!(rows[0].lint.severity, LintSeverity::Pass);
}

#[test]
fn transcript_rows_expose_babel_inline_track_selector_state() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 2.0, 3.0, "second"),
    ]);
    let session = EditorSession::for_project(&project);

    let row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "second")
        .unwrap();

    assert_eq!(row.selected_lane_id, "track-1");
    assert_eq!(
        row.track_options
            .iter()
            .map(|option| (option.lane_id.as_str(), option.label.as_str()))
            .collect::<Vec<_>>(),
        vec![("track-1", "Speaker 1"), ("track-2", "Speaker 2")]
    );
    assert_eq!(
        row.selected_track_option
            .as_ref()
            .map(|option| (option.lane_id.as_str(), option.label.as_str())),
        Some(("track-1", "Speaker 1"))
    );
}

#[test]
fn transcript_rows_expose_recovered_babel_reviewer_intervals_when_enabled() {
    let mut project = project_with_segments(vec![
        segment("first", 1.0, 2.0, "first"),
        segment("second", 5.0, 6.0, "second"),
    ]);
    project.metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.reviewerIntervals".to_string(),
        serde_json::json!([
            {
                "startOffsetInSeconds": 0.0,
                "endOffsetInSeconds": 3.0,
                "lastReviewer": {
                    "workerId": "worker-alpha",
                    "level": 1,
                    "discourseUsername": "alice"
                }
            },
            {
                "startOffsetInSeconds": 3.0,
                "endOffsetInSeconds": 10.0,
                "lastReviewer": {
                    "workerId": "worker-beta",
                    "level": 2
                }
            }
        ])
        .to_string(),
    );
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert!(rows.iter().all(|row| row.show_reviewer));
    assert_eq!(rows[0].reviewer.as_ref().unwrap().label, "L1: alice");
    assert_eq!(
        rows[0].reviewer.as_ref().unwrap().color,
        "hsl(49, 70%, 30%)"
    );
    assert_eq!(rows[1].reviewer.as_ref().unwrap().label, "L2: \u{2014}");
    assert_eq!(
        rows[1].reviewer.as_ref().unwrap().color,
        "hsl(287, 70%, 30%)"
    );
}

#[test]
fn transcript_rows_hide_reviewer_intervals_when_babel_toggle_is_off() {
    let mut project = project_with_segments(vec![segment("first", 1.0, 2.0, "first")]);
    project.metadata.insert(
        "studio.babel.diff.reviewerIntervals".to_string(),
        serde_json::json!([
            {
                "startOffsetInSeconds": 0.0,
                "endOffsetInSeconds": 3.0,
                "lastReviewer": {
                    "workerId": "worker-alpha",
                    "level": 1,
                    "discourseUsername": "alice"
                }
            }
        ])
        .to_string(),
    );
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert!(rows.iter().all(|row| !row.show_reviewer));
    assert!(rows.iter().all(|row| row.reviewer.is_none()));
}

#[test]
fn transcript_rows_expose_recovered_babel_hidden_timestamp_visible_cells() {
    let mut project = project_with_segments(vec![segment("first", 1.0, 2.0, "first")]);
    project.metadata.insert(
        "studio.babel.table.hideTimestamps".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let row = session.transcript_rows(&project).remove(0);

    assert_eq!(
        row.visible_cells,
        TranscriptRowVisibleCells {
            id: false,
            track: false,
            start: false,
            end: false,
            reviewer: true,
            text: true,
            linter: true,
            actions: false,
        }
    );
    assert_eq!(
        session.transcript_visible_cells(&project),
        row.visible_cells
    );
}

#[test]
fn timeline_lanes_include_deterministic_waveform_blocks_and_colors() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    let lanes = session.timeline_lanes(&project);

    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0].label, "Speaker 1");
    assert_eq!(lanes[0].color_token, "blue");
    assert_eq!(lanes[0].blocks.len(), 1);
    assert_eq!(lanes[1].label, "Speaker 2");
    assert_eq!(lanes[1].color_token, "purple");
    assert_eq!(lanes[1].blocks.len(), 1);
    assert_eq!(lanes.iter().map(|lane| lane.blocks.len()).sum::<usize>(), 2);
    assert!(lanes.iter().all(|lane| lane.scrollbar.is_visible));
    assert!(lanes.iter().all(|lane| lane.scrollbar.left_percent == 0.0));
    assert!(
        lanes
            .iter()
            .all(|lane| (lane.scrollbar.width_percent - 1.062).abs() < 0.01)
    );
    let speaker_one = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    assert!(speaker_one.blocks[0].left_percent > 0.0);
    assert!(speaker_one.blocks[0].width_percent > 0.0);
    assert_eq!(speaker_one.blocks[0].peaks.len(), 256);
    assert_eq!(
        speaker_one.blocks[0].peak_source,
        WaveformPeakSource::Synthetic
    );
    assert_eq!(speaker_one.blocks[0].peaks, speaker_one.blocks[0].peaks);
}

#[test]
fn timeline_scrollbar_drag_pans_babel_scroll_window_without_moving_playhead() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.set_zoom(&project, 2.0);
    let playhead = session.playhead_seconds;
    let before = session.viewport;

    session.drag_timeline_scrollbar_thumb(&project, 0.45, 0.0);

    assert_eq!(session.playhead_seconds, playhead);
    assert!((session.viewport.duration_seconds() - before.duration_seconds()).abs() < 0.001);
    assert!(session.viewport.start_seconds > before.start_seconds);
    assert_eq!(
        session.timeline_lanes(&project)[0].scrollbar.left_percent,
        45.0
    );
}

#[test]
fn timeline_lanes_expose_babel_track_control_state() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    let lanes = session.timeline_lanes(&project);
    let speaker_one = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    assert!(!speaker_one.is_collapsed);
    assert!(!speaker_one.is_muted);
    assert_eq!(speaker_one.vertical_zoom, 20.0);
    assert_eq!(speaker_one.track_height, 165.0);

    session.toggle_lane_collapse("track-speaker-1");
    session.toggle_lane_mute("track-speaker-2");
    session.set_lane_vertical_zoom("track-speaker-2", 7.5);

    let lanes = session.timeline_lanes(&project);
    let speaker_one = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    let speaker_two = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap();
    assert!(speaker_one.is_collapsed);
    assert_eq!(speaker_one.track_height, 52.0);
    assert!(speaker_two.is_muted);
    assert_eq!(speaker_two.vertical_zoom, 7.5);
}

#[test]
fn timeline_lanes_expose_recovered_track_chrome_and_viewport_state() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    let lane = session
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    assert_eq!(lane.chrome.collapse_tooltip, "Hide track");
    assert_eq!(lane.chrome.collapse_aria_label, "Hide track");
    assert_eq!(lane.chrome.solo_aria_label, "Solo track");
    assert_eq!(lane.viewport_state.container_display, "block");
    assert!(!lane.viewport_state.pointer_events_disabled);
    assert_eq!(lane.viewport_state.container_height, 165.0);
    assert_eq!(lane.viewport_state.waveform_height, 165.0);
    assert_eq!(lane.viewport_state.spectrogram_height, None);
    assert!(lane.viewport_state.render_solo_controls);
    assert!(lane.viewport_state.render_vertical_zoom_slider);

    session.toggle_lane_collapse("track-speaker-1");
    session.toggle_lane_mute("track-speaker-1");

    let lane = session
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    assert_eq!(lane.chrome.collapse_tooltip, "Show track");
    assert_eq!(lane.chrome.collapse_aria_label, "Show track");
    assert_eq!(lane.chrome.solo_aria_label, "Unsolo track");
    assert_eq!(lane.viewport_state.container_display, "hidden");
    assert!(lane.viewport_state.pointer_events_disabled);
    assert_eq!(lane.viewport_state.container_height, 0.0);
    assert_eq!(lane.viewport_state.waveform_height, 165.0);
    assert_eq!(lane.viewport_state.spectrogram_height, None);
    assert!(!lane.viewport_state.render_solo_controls);
    assert!(!lane.viewport_state.render_vertical_zoom_slider);
}

#[test]
fn timeline_lanes_expose_babel_one_second_overlay_when_enabled() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.seek_to_time(&project, 12.25);
    session.viewport = studio_app::TimeRange {
        start_seconds: 12.0,
        end_seconds: 17.0,
    };

    session.toggle_overlay(&project);

    let lane = session.timeline_lanes(&project).remove(0);
    let overlay = lane
        .overlay
        .as_ref()
        .expect("enabled overlay should be visible on timeline lanes");
    assert_eq!(overlay.start_seconds, 12.25);
    assert_eq!(overlay.end_seconds, 13.25);
    assert!(
        (overlay.width_percent - (1.0 / session.viewport.duration_seconds() * 100.0) as f32).abs()
            < 0.001
    );

    let bounds = studio_app::TimeRange::for_project(&project);
    session.move_overlay_to_time(&project, bounds.end_seconds - 0.2);

    let overlay = session
        .overlay_range
        .expect("overlay state should still be stored outside the visible viewport");
    assert_eq!(overlay.end_seconds, bounds.end_seconds);
    assert!((overlay.end_seconds - overlay.start_seconds - 1.0).abs() < 0.001);
    assert!(session.timeline_lanes(&project).remove(0).overlay.is_none());

    session.toggle_overlay(&project);

    assert!(session.timeline_lanes(&project)[0].overlay.is_none());
}

#[test]
fn lane_vertical_zoom_is_clamped_to_recovered_waveform_scale_unlock_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.set_lane_vertical_zoom("track-speaker-1", 0.1);
    assert_eq!(
        session
            .timeline_lanes(&project)
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .vertical_zoom,
        0.3
    );

    session.set_lane_vertical_zoom("track-speaker-1", 50.0);
    assert_eq!(
        session
            .timeline_lanes(&project)
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .vertical_zoom,
        50.0
    );

    session.set_lane_vertical_zoom("track-speaker-1", 1_500.0);
    assert_eq!(
        session
            .timeline_lanes(&project)
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .vertical_zoom,
        1_000.0
    );
}

#[test]
fn timeline_lanes_use_loaded_audio_peaks_for_matching_lane_and_fallback_for_missing_lane() {
    let mut other_lane = segment("other-lane", 0.0, 1.0, "other");
    other_lane.lane_id = "track-2".to_string();
    other_lane.speaker_id = "speaker-2".to_string();
    let project = two_lane_project(vec![segment("audio-backed", 0.0, 1.0, "audio"), other_lane]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(4, vec![vec![0.5; 4]]).expect("valid audio");

    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let lanes = session.timeline_lanes(&project);
    let audio_block = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .unwrap()
        .blocks[0]
        .clone();
    let fallback_block = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-2")
        .unwrap()
        .blocks[0]
        .clone();

    assert_eq!(audio_block.peak_source, WaveformPeakSource::Audio);
    assert!(
        audio_block
            .peaks
            .iter()
            .all(|peak| (*peak - 0.5).abs() < f32::EPSILON)
    );
    assert_eq!(fallback_block.peak_source, WaveformPeakSource::Synthetic);
}

#[test]
fn timeline_lanes_expose_wavesurfer_render_model_with_segment_blocks() {
    let project = two_lane_project(vec![segment("audio-backed", 0.0, 1.0, "audio")]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(4, vec![vec![0.25, -0.5, 0.75, -1.0]]).expect("valid audio");

    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let lanes = session.timeline_lanes(&project);
    let model = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .unwrap()
        .waveform_model
        .as_ref()
        .expect("audio-backed lane should expose render model");

    assert_eq!(model.visible_start_seconds, 0.0);
    assert_eq!(model.visible_end_seconds, 1.0);
    assert!(model.regions.is_empty());
    assert_eq!(model.channels[0].peaks.len(), 2_048);
    let lane = lanes.iter().find(|lane| lane.lane_id == "track-1").unwrap();
    assert_eq!(lane.blocks.len(), 1);
    assert_eq!(lane.blocks[0].segment_id, "audio-backed");
    assert_eq!(lane.blocks[0].peak_source, WaveformPeakSource::Audio);
}

#[test]
fn lane_waveform_reuses_timeline_render_cache_across_playhead_only_changes() {
    let project = two_lane_project(vec![segment("audio-backed", 0.0, 1.0, "audio")]);
    let audio = DecodedAudio::new(4, vec![vec![0.25, -0.5, 0.75, -1.0]]).expect("valid audio");
    let waveform = LaneWaveform::from_decoded(audio);
    let segment_refs = project.segments.iter().collect::<Vec<_>>();
    let viewport = studio_app::TimeRange {
        start_seconds: 0.0,
        end_seconds: 1.0,
    };

    let first = waveform
        .render_model(&segment_refs, viewport, 0.25)
        .expect("first render should build");
    let after_first = waveform.render_cache_stats();
    let second = waveform
        .render_model(&segment_refs, viewport, 0.75)
        .expect("second render should reuse cached peaks");
    let after_second = waveform.render_cache_stats();

    assert_eq!(after_first.misses, 1);
    assert_eq!(after_first.hits, 0);
    assert_eq!(after_second.misses, 1);
    assert_eq!(after_second.hits, 1);
    assert!(std::sync::Arc::ptr_eq(&first.channels, &second.channels));
    assert!(std::sync::Arc::ptr_eq(
        &first.channels[0].peaks,
        &second.channels[0].peaks
    ));
    assert!(std::sync::Arc::ptr_eq(&first.regions, &second.regions));
    assert_ne!(
        first.cursor.progress_percent,
        second.cursor.progress_percent
    );

    let shifted_viewport = studio_app::TimeRange {
        start_seconds: 0.25,
        end_seconds: 1.25,
    };
    let _ = waveform.render_model(&segment_refs, shifted_viewport, 0.75);

    let after_shift = waveform.render_cache_stats();
    assert_eq!(after_shift.misses, 1);
    assert_eq!(after_shift.hits, 2);
}

#[test]
fn lane_waveform_reuses_segment_peak_cache_across_timeline_rebuilds() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 2.0, 3.0, "second"),
    ]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");

    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let first_lanes = session.timeline_lanes(&project);
    let first_audio_blocks = first_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("audio lane should render")
        .blocks
        .iter()
        .filter(|block| block.peak_source == WaveformPeakSource::Audio)
        .count();
    let stats_after_first = session
        .lane_waveforms
        .get("track-1")
        .expect("audio waveform should remain attached")
        .segment_peak_cache_stats();

    assert_eq!(first_audio_blocks, 2);
    assert_eq!(stats_after_first.misses, 2);
    assert_eq!(stats_after_first.hits, 0);

    let second_lanes = session.timeline_lanes(&project);
    let second_audio_blocks = second_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("audio lane should render")
        .blocks
        .iter()
        .filter(|block| block.peak_source == WaveformPeakSource::Audio)
        .count();
    let stats_after_second = session
        .lane_waveforms
        .get("track-1")
        .expect("audio waveform should remain attached")
        .segment_peak_cache_stats();

    assert_eq!(second_audio_blocks, first_audio_blocks);
    assert_eq!(stats_after_second.misses, stats_after_first.misses);
    assert_eq!(
        stats_after_second.hits, stats_after_first.hits,
        "unchanged timeline block rebuilds should reuse static waveform blocks before consulting the segment peak cache"
    );
}

#[test]
fn lane_waveform_reuses_overview_render_cache_across_playhead_only_changes() {
    let audio = DecodedAudio::new(4, vec![vec![0.25, -0.5, 0.75, -1.0]]).expect("valid audio");
    let waveform = LaneWaveform::from_decoded(audio);

    let first = waveform
        .overview_render_model(0.25)
        .expect("first overview should build");
    let second = waveform
        .overview_render_model(0.75)
        .expect("second overview should reuse cached peaks");
    let stats = waveform.render_cache_stats();

    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
    assert!(std::sync::Arc::ptr_eq(&first.channels, &second.channels));
    assert!(std::sync::Arc::ptr_eq(
        &first.channels[0].peaks,
        &second.channels[0].peaks
    ));
    assert_ne!(
        first.cursor.progress_percent,
        second.cursor.progress_percent
    );
}

#[test]
fn lane_waveform_can_use_precomputed_peaks_without_decoded_audio() {
    let project = two_lane_project(vec![segment("peak-backed", 0.0, 1.0, "peaks")]);
    let mut session = EditorSession::for_project(&project);
    let waveform = LaneWaveform::from_precomputed_peaks(vec![vec![0.0, 0.5, -1.0, 0.25]], 1.0)
        .expect("valid peak source");

    session.set_lane_waveform("track-1", waveform);

    let lanes = session.timeline_lanes(&project);
    let lane = lanes.iter().find(|lane| lane.lane_id == "track-1").unwrap();
    let model = lane.waveform_model.as_ref().unwrap();

    assert_eq!(lane.blocks[0].peak_source, WaveformPeakSource::Audio);
    assert!(model.regions.is_empty());
    assert_eq!(lane.blocks[0].segment_id, "peak-backed");
    assert_eq!(model.channels[0].peaks.len(), 2_048);
}

#[test]
fn overview_waveform_uses_loaded_audio_and_reports_viewport_window() {
    let project = project_with_segments(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 3.0, 4.0, "second"),
    ]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(1, vec![vec![0.1, 0.4, 0.8, 0.2]]).expect("valid audio");
    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));
    session.playhead_seconds = 2.0;

    session.set_zoom(&project, 2.0);
    let overview = session.overview_waveform(&project);
    let model = overview
        .waveform_model
        .as_ref()
        .expect("audio-backed overview should expose a Wavesurfer render model");

    assert_eq!(overview.peak_source, WaveformPeakSource::Audio);
    assert_eq!(model.visible_start_seconds, 0.0);
    assert_eq!(model.visible_end_seconds, 4.0);
    assert_eq!(model.channels[0].peaks.len(), 1_024);
    assert!((overview.viewport_left_percent - 18.75).abs() < 0.001);
    assert!((overview.viewport_width_percent - 62.5).abs() < 0.001);
    assert!((overview.playhead_percent - 50.0).abs() < 0.001);
}

#[test]
fn overview_waveform_exposes_all_visible_lane_audio_models() {
    let mut second_lane_segment = segment("second-lane", 1.0, 2.0, "second");
    second_lane_segment.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("first-lane", 0.0, 1.0, "first"),
        second_lane_segment,
    ]);
    let mut session = EditorSession::for_project(&project);

    session.set_lane_waveform(
        "track-1",
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.1, 0.2, 0.3, 0.4]]).expect("valid audio"),
        ),
    );
    session.set_lane_waveform(
        "track-2",
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.4, 0.3, 0.2, 0.1]]).expect("valid audio"),
        ),
    );

    let overview = session.overview_waveform(&project);

    assert_eq!(overview.peak_source, WaveformPeakSource::Audio);
    assert_eq!(
        overview
            .lane_waveform_models
            .iter()
            .map(|lane| lane.lane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["track-1", "track-2"]
    );
    assert_eq!(
        overview
            .lane_waveform_models
            .iter()
            .map(|lane| lane.color_token.as_str())
            .collect::<Vec<_>>(),
        vec!["blue", "purple"],
        "overview lane waveforms should use the same color tokens as the timeline lanes"
    );

    session.track_filter = TrackFilter::lane("track-2", "Speaker 2");
    let filtered_overview = session.overview_waveform(&project);

    assert_eq!(
        filtered_overview
            .lane_waveform_models
            .iter()
            .map(|lane| lane.lane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["track-2"]
    );
    assert_eq!(
        filtered_overview.lane_waveform_models[0].color_token, "purple",
        "lane colors must stay stable when the track filter hides other lanes"
    );
}

#[test]
fn overview_segment_blocks_use_timeline_lane_color_order() {
    let mut second_lane_segment = segment("second-lane", 1.0, 2.0, "second");
    second_lane_segment.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("first-lane", 0.0, 1.0, "first"),
        second_lane_segment,
    ]);
    let mut session = EditorSession::for_project(&project);

    let overview = session.overview_waveform(&project);
    assert_eq!(
        overview
            .blocks
            .iter()
            .map(|block| block.color_token.as_str())
            .collect::<Vec<_>>(),
        vec!["blue", "purple"]
    );
    assert_eq!(
        overview
            .blocks
            .iter()
            .map(|block| block.lane_row)
            .collect::<Vec<_>>(),
        vec![0, 1],
        "each lane's segment blocks should land in that lane's own overview row"
    );
    assert_eq!(overview.block_lane_rows, 2);

    session.track_filter = TrackFilter::lane("track-2", "Speaker 2");
    let filtered_overview = session.overview_waveform(&project);
    assert_eq!(
        filtered_overview
            .blocks
            .iter()
            .map(|block| block.color_token.as_str())
            .collect::<Vec<_>>(),
        vec!["purple"]
    );
    assert_eq!(
        filtered_overview
            .blocks
            .iter()
            .map(|block| block.lane_row)
            .collect::<Vec<_>>(),
        vec![0],
        "filtering to one lane should collapse the overview to a single block row"
    );
    assert_eq!(filtered_overview.block_lane_rows, 1);
}

#[test]
fn overview_viewport_start_fraction_pans_window_without_moving_playhead() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-17");
    let playhead = session.playhead_seconds;

    session.set_zoom(&project, 4.0);
    let visible_duration = session.viewport.duration_seconds();
    session.set_overview_viewport_start_fraction(&project, 0.5);

    let bounds = studio_app::TimeRange::for_project(&project);
    let expected_start = bounds.start_seconds + bounds.duration_seconds() * 0.5;
    assert_eq!(session.playhead_seconds, playhead);
    assert!((session.viewport.duration_seconds() - visible_duration).abs() < 0.001);
    assert!((session.viewport.start_seconds - expected_start).abs() < 0.001);

    session.set_overview_viewport_start_fraction(&project, 1.0);

    assert!((session.viewport.end_seconds - bounds.end_seconds).abs() < 0.001);
}

#[test]
fn linter_reports_empty_text_invalid_timing_missing_lane_and_overlap() {
    let mut project = project_with_segments(vec![
        segment("one", 1.0, 3.0, ""),
        segment("two", 2.5, 4.0, "overlap"),
        segment("three", 5.0, 4.0, "bad timing"),
        segment("four", 6.0, 7.0, "missing lane"),
    ]);
    project.segments[3].lane_id = "missing".to_string();
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Error);
    assert_eq!(rows[0].lint.message, "Segment cannot have empty text.");
    assert_eq!(
        rows[0]
            .lint_issues
            .iter()
            .map(|issue| issue.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            LintKind::EmptyText,
            LintKind::Overlap,
            LintKind::DistanceBetweenSegments,
        ]
    );
    assert_eq!(rows[1].lint.severity, LintSeverity::Error);
    assert_eq!(rows[1].lint.message, "Segments cannot overlap.");
    assert_eq!(rows[2].lint.severity, LintSeverity::Error);
    assert!(rows[2].lint.message.contains("start before end"));
    assert_eq!(rows[3].lint.severity, LintSeverity::Error);
    assert!(rows[3].lint.message.contains("unknown track"));
}

#[test]
fn linter_counts_recovered_babel_pairwise_overlap_and_distance_issues() {
    let project = project_with_segments(vec![
        segment("left", 0.0, 5.0, "left"),
        segment("middle", 4.0, 7.0, "middle"),
        segment("right", 6.0, 9.0, "right"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    let issue_kinds = rows
        .iter()
        .map(|row| {
            (
                row.segment_id.as_str(),
                row.lint_issues
                    .iter()
                    .map(|issue| issue.kind.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        issue_kinds,
        vec![
            (
                "left",
                vec![LintKind::Overlap, LintKind::DistanceBetweenSegments],
            ),
            (
                "middle",
                vec![
                    LintKind::Overlap,
                    LintKind::Overlap,
                    LintKind::DistanceBetweenSegments,
                ],
            ),
            ("right", vec![LintKind::Overlap]),
        ]
    );
    assert_eq!(session.blocking_lint_count(&project), 6);
}

#[test]
fn linter_uses_recovered_babel_adjacent_pair_overlap_windows() {
    let project = project_with_segments(vec![
        segment("long", 0.0, 10.0, "long"),
        segment("inside", 2.0, 3.0, "inside"),
        segment("later-contained", 4.0, 5.0, "later contained"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(
        rows[0]
            .lint_issues
            .iter()
            .filter(|issue| issue.kind == LintKind::Overlap)
            .count(),
        1
    );
    assert_eq!(
        rows[1]
            .lint_issues
            .iter()
            .filter(|issue| issue.kind == LintKind::Overlap)
            .count(),
        1
    );
    assert_eq!(rows[2].lint.severity, LintSeverity::Pass);
}

#[test]
fn linter_reports_babel_text_hygiene_errors() {
    let project = project_with_segments(vec![
        segment("double-spaces", 1.0, 2.0, "alpha  beta"),
        segment("leading-space", 4.0, 5.0, " alpha"),
        segment("trailing-space", 7.0, 8.0, "alpha "),
        segment("newline", 10.0, 11.0, "alpha\nbeta"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Error);
    assert_eq!(
        rows[0].lint.message,
        "Double spaces are not allowed in text."
    );
    assert_eq!(rows[1].lint.severity, LintSeverity::Error);
    assert_eq!(
        rows[1].lint.message,
        "Extra spaces at the end or beginning of segments are not allowed."
    );
    assert_eq!(rows[2].lint.severity, LintSeverity::Error);
    assert_eq!(
        rows[2].lint.message,
        "Extra spaces at the end or beginning of segments are not allowed."
    );
    assert_eq!(rows[3].lint.severity, LintSeverity::Error);
    assert_eq!(
        rows[3].lint.message,
        "Segment contains newline character(s)."
    );
}

#[test]
fn linter_reports_recovered_babel_duplicate_warnings() {
    let repeated_50 = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwx";
    let exact_duplicate = "alpha beta gamma delta epsilon zeta";
    let adjacent_duplicate = "alpha beta gamma delta";
    let project = project_with_segments(vec![
        segment("intra-duplicate", 1.0, 2.0, repeated_50),
        segment("exact-original", 4.0, 5.0, exact_duplicate),
        segment("exact-later", 7.0, 8.0, exact_duplicate),
        segment("adjacent-original", 10.0, 11.0, adjacent_duplicate),
        segment("adjacent-later", 13.0, 14.0, adjacent_duplicate),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Warning);
    assert_eq!(
        rows[0].lint.message,
        "Repeated 50-char substring: \"abcdefghijklmnopqrstuvwxyzabcdefghijklmn...\""
    );
    assert_eq!(rows[1].lint.severity, LintSeverity::Pass);
    assert_eq!(rows[2].lint.severity, LintSeverity::Warning);
    assert_eq!(
        rows[2].lint.message,
        "Exact duplicate of another segment in this transcript."
    );
    assert_eq!(rows[3].lint.severity, LintSeverity::Pass);
    assert_eq!(rows[4].lint.severity, LintSeverity::Warning);
    assert_eq!(
        rows[4].lint.message,
        "Possible duplicate of the adjacent segment."
    );
}

#[test]
fn linter_reports_babel_gap_and_double_dash_errors() {
    let project = project_with_segments(vec![
        segment("too-close", 1.0, 2.0, "first"),
        segment("after-too-close", 2.5, 3.0, "second"),
        segment("triple-dash", 5.0, 6.0, "alpha---beta"),
        segment("spaced-double-dash", 8.0, 9.0, "alpha --beta"),
        segment("double-dash-punctuation", 11.0, 12.0, "alpha--, beta"),
        segment("double-dash-missing-space", 14.0, 15.0, "alpha--beta"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(
        rows[0].lint.message,
        "Segments must be at least 1 second apart."
    );
    assert_eq!(rows[1].lint.severity, LintSeverity::Pass);
    assert_eq!(
        rows[2].lint.message,
        "Triple or more dashes (---) are typically avoided."
    );
    assert_eq!(
        rows[3].lint.message,
        "Double dash should hug the preceding word (no space before --)."
    );
    assert_eq!(
        rows[4].lint.message,
        "Punctuation immediately after double dash is typically avoided."
    );
    assert_eq!(
        rows[5].lint.message,
        "Double dash mid-sentence should be followed by a space or closing quote."
    );
    assert!(
        rows.iter()
            .filter(|row| row.lint.severity == LintSeverity::Error)
            .count()
            >= 5
    );
}

#[test]
fn linter_reports_babel_emphasis_unicode_and_punctuation_errors() {
    let project = project_with_segments(vec![
        segment("multiple-asterisks", 1.0, 2.0, "alpha ** beta"),
        segment("unpaired-asterisks", 4.0, 5.0, "alpha *beta"),
        segment("empty-emphasis", 7.0, 8.0, "alpha * * beta"),
        segment("punct-emphasis", 10.0, 11.0, "alpha *beta,* gamma"),
        segment("consecutive-punct", 13.0, 14.0, "alpha,, beta"),
        segment("emoji", 16.0, 17.0, "alpha 😀 beta"),
        segment("em-dash", 19.0, 20.0, "alpha — beta"),
        segment("unsupported-unicode", 22.0, 23.0, "alpha “beta…”"),
        segment("parentheses", 25.0, 26.0, "alpha (beta)"),
        segment("unpaired-quotes", 28.0, 29.0, "alpha \"beta"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(
        rows[0].lint.message,
        "Multiple asterisks in a row (**, ***, etc.) are not allowed."
    );
    assert_eq!(
        rows[1].lint.message,
        "Asterisks must come in pairs (*word*)."
    );
    assert_eq!(
        rows[2].lint.message,
        "Empty emphasis (*  *) is not allowed - add text between asterisks."
    );
    assert_eq!(
        rows[3].lint.message,
        "Punctuation should be outside emphasis asterisk."
    );
    assert_eq!(
        rows[4].lint.message,
        "Consecutive terminal punctuation or commas detected."
    );
    assert_eq!(
        rows[5].lint.message,
        "Emoji are not allowed in transcriptions. Found: 😀"
    );
    assert_eq!(
        rows[6].lint.message,
        "Em dash (—) is not allowed. Use a standard dash with spaces on either side ( - ) instead."
    );
    assert_eq!(
        rows[7].lint.message,
        "Unsupported Unicode character(s) detected. Found: “ … ”"
    );
    assert_eq!(
        rows[8].lint.message,
        "Parentheses are not used for transcription, with rare exceptions. Use [] or <> for tags, or {} for MIS/PRO/comments."
    );
    assert_eq!(
        rows[9].lint.message,
        "Quotation marks must come in pairs — this segment has an odd number of quotation marks."
    );
}

#[test]
fn linter_preserves_multiple_babel_issues_per_row_and_counts_each_blocker() {
    let project = project_with_segments(vec![segment("many-issues", 1.0, 2.0, " alpha  ** 😀 ")]);
    let mut session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert_eq!(
        rows[0].lint.message,
        "Multiple asterisks in a row (**, ***, etc.) are not allowed."
    );
    assert_eq!(
        rows[0]
            .lint_issues
            .iter()
            .map(|issue| issue.message.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Multiple asterisks in a row (**, ***, etc.) are not allowed.",
            "Double spaces are not allowed in text.",
            "Extra spaces at the end or beginning of segments are not allowed.",
            "Emoji are not allowed in transcriptions. Found: 😀",
        ]
    );
    assert_eq!(session.blocking_lint_count(&project), 4);
    assert_eq!(
        session
            .transport_strip(&project)
            .problem_summary_label
            .as_deref(),
        Some("4 errors")
    );
    assert_eq!(
        session.submit_review(&project).unwrap_err(),
        "Please fix linter errors (4 blocking lint issue(s) remain)"
    );
}

#[test]
fn non_transcription_babel_annotations_skip_transcription_lint_and_actions() {
    let mut annotation = segment("non-transcription", 1.0, 2.0, "");
    annotation
        .metadata
        .insert("type".to_string(), serde_json::json!("non-transcription"));
    let project = project_with_segments(vec![annotation]);
    let session = EditorSession::for_project(&project);

    let row = session.transcript_rows(&project).remove(0);

    assert_eq!(
        row.annotation_kind,
        TranscriptAnnotationKind::NonTranscription("non-transcription".to_string())
    );
    assert_eq!(row.annotation_type_label, "non-transcription");
    assert_eq!(row.lint.severity, LintSeverity::Pass);
    assert!(row.lint_issues.is_empty());

    let menu = session.transcript_action_menu(&project, &row);
    assert_eq!(
        menu.items
            .iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            ("item", "Edit", true),
            ("separator", "", false),
            ("item", "Merge With Above", false),
            ("item", "Merge With Below", false),
            ("separator", "", false),
            ("item", "Delete", true),
        ]
    );
}

#[test]
fn action_command_model_is_limited_to_recovered_babel_row_menu_commands() {
    let source = include_str!("../src/transcript_model/actions.rs");

    assert!(!source.contains("Apply,"));
    assert!(!source.contains("Split,"));
    assert!(!source.contains("SetTranscription"));
    assert!(!source.contains("SetNonTranscription"));
}

#[test]
fn submit_review_is_blocked_by_lint_errors_and_marks_clean_project_submitted() {
    let project = project_with_segments(vec![segment("clean", 1.0, 2.0, "clean")]);
    let mut session = EditorSession::for_project(&project);
    session.mark_dirty();
    session.mark_saved();

    session
        .submit_review(&project)
        .expect("clean project should submit");
    assert_eq!(session.review_state, ReviewState::Submitted);

    let bad_project = project_with_segments(vec![segment("bad", 1.0, 2.0, "")]);
    let mut bad_session = EditorSession::for_project(&bad_project);
    let error = bad_session
        .submit_review(&bad_project)
        .expect_err("lint errors should block submit");
    assert!(error.contains("1 blocking"));
    assert_eq!(bad_session.review_state, ReviewState::Draft);
}

#[test]
fn submit_review_checks_hidden_lanes_even_when_filter_is_clean() {
    let mut hidden_bad = segment("annotation-2", 2.0, 3.0, "");
    hidden_bad.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("annotation-1", 0.0, 1.0, "visible clean"),
        hidden_bad,
    ]);
    let mut session = EditorSession::for_project(&project);
    session.track_filter = TrackFilter::lane("track-1", "Speaker 1");

    assert_eq!(session.transcript_rows(&project).len(), 1);
    assert_eq!(session.blocking_lint_count(&project), 1);
    assert!(session.submit_review(&project).is_err());
}

#[test]
fn submit_review_blocks_unacknowledged_warnings_until_asserted() {
    let project = project_with_segments(vec![segment(
        "warning",
        1.0,
        60.0,
        "very long but intentionally valid",
    )]);
    let mut session = EditorSession::for_project(&project);

    let error = session
        .submit_review(&project)
        .expect_err("Babel blocks submit while warnings are unacknowledged");
    assert!(error.contains("Please acknowledge all warnings"));
    assert_eq!(session.review_state, ReviewState::Draft);

    session.select_segment(&project, "warning");
    assert!(session.toggle_warning_assertion(&project));
    session
        .submit_review(&project)
        .expect("asserted warnings should no longer block submit");
    assert_eq!(session.review_state, ReviewState::Submitted);
}

#[test]
fn submit_review_blocks_unresolved_low_confidence_until_resolved_or_edited() {
    let mut unresolved = segment("low-confidence", 1.0, 2.0, "alpha beta");
    unresolved.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "word": "beta", "start": 1.2, "end": 1.4, "isYellow": true }
        ]),
    );
    let unresolved_project = project_with_segments(vec![unresolved]);
    let mut session = EditorSession::for_project(&unresolved_project);

    let row = session.transcript_rows(&unresolved_project).remove(0);
    assert_eq!(row.lint.severity, LintSeverity::Warning);
    assert!(row.lint.message.contains("low-confidence"));
    assert!(!session.toggle_warning_assertion(&unresolved_project));
    let error = session
        .submit_review(&unresolved_project)
        .expect_err("low-confidence words require explicit review");
    assert!(error.contains("Please review all low-confidence words"));

    let mut resolved = unresolved_project.clone();
    resolved
        .segment_mut("low-confidence")
        .unwrap()
        .metadata
        .insert("lowConfidenceResolved".to_string(), serde_json::json!(true));
    let mut resolved_session = EditorSession::for_project(&resolved);
    resolved_session
        .submit_review(&resolved)
        .expect("resolved low-confidence metadata should unblock submit");

    let mut edited = unresolved_project.clone();
    edited.segment_mut("low-confidence").unwrap().text = "alpha".to_string();
    let mut edited_session = EditorSession::for_project(&edited);
    edited_session
        .submit_review(&edited)
        .expect("editing out the highlighted word should unblock submit");
}

#[test]
fn transcript_rows_expose_recovered_babel_low_confidence_word_views_for_text_cell() {
    let mut segment = segment("low-confidence", 1.0, 2.0, "alpha beta gamma");
    segment.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "index": 3, "finalWord": "alpha", "start": 1.05, "end": 1.2, "isYellow": false },
            { "index": 4, "finalWord": "beta", "start": 1.25, "end": 1.45, "isYellow": true },
            { "index": 5, "finalWord": "missing", "start": 1.5, "end": 1.6, "isYellow": true },
            { "index": 6, "finalWord": "gamma", "start": 1.6, "end": 1.8, "isYellow": false }
        ]),
    );
    let project = project_with_segments(vec![segment]);
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].word_views.len(), 1);
    assert_eq!(rows[0].word_views[0].index, 1);
    assert_eq!(rows[0].word_views[0].text, "beta");
    assert_eq!(rows[0].word_views[0].start_seconds, Some(1.25));
    assert_eq!(rows[0].word_views[0].end_seconds, Some(1.45));
    assert!(rows[0].word_views[0].is_low_confidence);
}

#[test]
fn seek_to_time_moves_playhead_and_selects_segment_under_timestamp() {
    let project = project_with_segments(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 1.0, 2.0, "second"),
    ]);
    let mut session = EditorSession::for_project(&project);

    session.seek_to_time(&project, 1.25);

    assert_eq!(session.playhead_seconds, 1.25);
    assert_eq!(session.selected_segment_id.as_deref(), Some("second"));
}

#[test]
fn linter_reports_babel_ru_annotation_tag_errors_and_warnings() {
    let project = project_with_segments(vec![
        segment("valid-pro", 1.0, 2.0, "один {СКАЗ: два} три"),
        segment("malformed-pro", 3.0, 4.0, "один {СКАЗ : два} три"),
        segment("unbracketed-pro", 5.0, 6.0, "один СКАЗ: два"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Pass);
    assert_eq!(rows[1].lint.severity, LintSeverity::Error);
    assert!(
        rows[1]
            .lint
            .message
            .contains("Malformed or disallowed curly-bracket tag")
    );
    assert_eq!(rows[2].lint.severity, LintSeverity::Warning);
    assert!(
        rows[2]
            .lint
            .message
            .contains("Annotation tag outside brackets")
    );
}

#[test]
fn linter_reports_babel_square_bracket_validation_errors() {
    let project = project_with_segments(vec![
        segment("valid-unintelligible", 1.0, 2.0, "один [неразборчиво] два"),
        segment("valid-chuckle", 4.0, 5.0, "один [смешок] два"),
        segment("invalid-tag", 7.0, 8.0, "один [unknown] два"),
        segment("unclosed-tag", 10.0, 11.0, "один [неразборчиво два"),
        segment("empty-tag", 13.0, 14.0, "один [] два"),
        segment("spaced-tag", 16.0, 17.0, "один [ неразборчиво ] два"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Pass);
    assert_eq!(rows[1].lint.severity, LintSeverity::Pass);
    assert_eq!(rows[2].lint.message, "Invalid bracket tag found.");
    assert_eq!(rows[3].lint.message, "Brackets must be properly closed.");
    assert_eq!(rows[4].lint.message, "Empty brackets [] are not allowed.");
    assert_eq!(
        rows[5].lint.message,
        "Spaces are not allowed within bracket tags."
    );
}

#[test]
fn linter_reports_babel_curly_validation_and_tag_only_punctuation_errors() {
    let project = project_with_segments(vec![
        segment("valid-curly", 1.0, 2.0, "один {СКАЗ: два} три"),
        segment("empty-curly", 4.0, 5.0, "один {} два"),
        segment("unclosed-curly", 7.0, 8.0, "один {СКАЗ: два"),
        segment("spaced-curly", 10.0, 11.0, "один { СКАЗ: два } три"),
        segment("tag-only-square-punct", 13.0, 14.0, "[неразборчиво]."),
        segment("tag-only-curly-punct", 16.0, 17.0, "{СКАЗ: слово}!"),
        segment("tag-with-speech-punct", 19.0, 20.0, "слово [неразборчиво]."),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Pass);
    assert_eq!(
        rows[1].lint.message,
        "Empty curly brackets {} are not allowed."
    );
    assert_eq!(
        rows[2].lint.message,
        "Curly brackets must be properly closed."
    );
    assert_eq!(
        rows[3].lint.message,
        "Spaces are not allowed within bracket tags."
    );
    assert_eq!(
        rows[4].lint.message,
        "A segment that contains only a tag must not include any punctuation."
    );
    assert_eq!(
        rows[5].lint.message,
        "A segment that contains only a tag must not include any punctuation."
    );
    assert_eq!(rows[6].lint.severity, LintSeverity::Pass);
}

#[test]
fn zoom_reframes_the_visible_timeline_around_the_playhead() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-17");

    let before = session
        .timeline_lanes(&project)
        .iter()
        .flat_map(|lane| lane.blocks.iter())
        .find(|block| block.segment_id == "annotation-17")
        .unwrap()
        .left_percent;
    let before_duration = session.viewport.duration_seconds();
    session.set_zoom(&project, 2.0);
    let after = session
        .timeline_lanes(&project)
        .iter()
        .flat_map(|lane| lane.blocks.iter())
        .find(|block| block.segment_id == "annotation-17")
        .unwrap()
        .left_percent;

    assert_eq!(session.zoom, 2.0);
    assert!((session.viewport.duration_seconds() - 2.5).abs() < 0.001);
    assert!(session.viewport.duration_seconds() < before_duration);
    assert!(session.viewport.start_seconds > 100.0);
    assert!(session.viewport.end_seconds < 390.0);
    assert!((before - after).abs() < 0.001);
}

#[test]
fn zoom_clamps_to_babel_toolbar_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.set_zoom(&project, 0.01);
    assert_eq!(session.zoom, 0.05);

    session.set_zoom(&project, 20.0);
    assert_eq!(session.zoom, 10.0);
}

#[test]
fn loop_toggle_matches_recovered_babel_stored_range_state() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-2");

    session.toggle_loop(&project);

    assert!(!session.loop_enabled);
    assert!(session.stored_loop_range.is_none());
    assert!(!session.loop_strip(&project).has_range);

    session.set_loop_range_from_drag(&project, 3.5, 5.1);

    assert!(session.loop_enabled);
    assert_eq!(session.stored_loop_range, Some(session.loop_range));
    assert!(session.loop_strip(&project).has_range);

    session.toggle_loop(&project);

    assert!(!session.loop_enabled);
    assert_eq!(session.stored_loop_range, Some(session.loop_range));
    assert!(session.loop_strip(&project).has_range);
    assert_eq!(session.loop_label(), "Loop 00:03.50 - 00:05.10 Off");

    session.toggle_loop(&project);

    assert!(session.loop_enabled);
    assert_eq!(session.loop_range.start_seconds, 3.5);
    assert_eq!(session.loop_range.end_seconds, 5.1);
    assert!(session.loop_label().contains("00:03.50"));
}

#[test]
fn loop_strip_reports_visible_range_geometry_and_hit_targets() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-2");
    session.set_zoom(&project, 4.0);
    session.set_loop_range_from_drag(&project, 3.5, 5.1);

    let strip = session.loop_strip(&project);

    assert!(strip.has_range);
    assert!(strip.is_enabled);
    assert_eq!(strip.start_seconds, 3.5);
    assert_eq!(strip.end_seconds, 5.1);
    assert!(strip.left_percent >= 0.0);
    assert!(strip.width_percent > 0.0);
    assert!(strip.playhead_percent >= strip.left_percent);

    let start_fraction = strip.left_percent / 100.0;
    let body_fraction = (strip.left_percent + strip.width_percent / 2.0) / 100.0;
    let end_fraction = (strip.left_percent + strip.width_percent) / 100.0;

    assert_eq!(
        session.loop_strip_hit(&project, start_fraction).target,
        LoopStripHitTarget::StartHandle
    );
    assert_eq!(
        session.loop_strip_hit(&project, body_fraction).target,
        LoopStripHitTarget::RangeBody
    );
    assert_eq!(
        session.loop_strip_hit(&project, end_fraction).target,
        LoopStripHitTarget::EndHandle
    );
    assert_eq!(
        session.loop_strip_hit(&project, 0.01).target,
        LoopStripHitTarget::Empty
    );
}

#[test]
fn loop_client_x_conversion_matches_recovered_babel_scroll_metrics() {
    let metrics = LoopStripClientMetrics {
        container_left: 100.0,
        scroll_left: 50.0,
        duration: 20.0,
        zoom_value: 10.0,
    };

    assert_eq!(client_x_to_loop_time(None, 150.0), 0.0);
    assert_eq!(client_x_to_loop_time(Some(metrics), 150.0), 10.0);
    assert_eq!(client_x_to_loop_time(Some(metrics), 0.0), 0.0);
    assert_eq!(client_x_to_loop_time(Some(metrics), 500.0), 20.0);

    let viewport = TimeRange {
        start_seconds: 5.0,
        end_seconds: 10.0,
    };
    let derived = LoopStripClientMetrics::for_viewport(viewport, 20.0, 1_000.0);
    assert_eq!(derived.zoom_value, 200.0);
    assert_eq!(derived.scroll_left, 1_000.0);
    assert_eq!(client_x_to_loop_time(Some(derived), 500.0), 7.5);
}

#[test]
fn loop_handle_drag_matches_recovered_babel_hundredth_second_edge_math() {
    let project = project_with_segments(vec![segment("one", 0.0, 10.0, "one")]);
    let mut session = EditorSession::for_project(&project);

    session.loop_range = TimeRange {
        start_seconds: 3.5,
        end_seconds: 5.1,
    };
    session.stored_loop_range = Some(session.loop_range);
    session.set_loop_start(&project, 5.5);

    assert!((session.loop_range.start_seconds - 5.09).abs() < 0.000_001);
    assert_eq!(session.loop_range.end_seconds, 5.1);

    session.loop_range = TimeRange {
        start_seconds: 0.0,
        end_seconds: 0.005,
    };
    session.stored_loop_range = Some(session.loop_range);
    session.set_loop_start(&project, 0.0);

    assert!((session.loop_range.start_seconds - -0.005).abs() < 0.000_001);
    assert_eq!(session.loop_range.end_seconds, 0.005);

    session.loop_range = TimeRange {
        start_seconds: 9.995,
        end_seconds: 10.0,
    };
    session.stored_loop_range = Some(session.loop_range);
    session.set_loop_end(&project, 10.0);

    assert_eq!(session.loop_range.start_seconds, 9.995);
    assert!((session.loop_range.end_seconds - 10.005).abs() < 0.000_001);
}

#[test]
fn enabled_loop_range_does_not_follow_subsequent_segment_selection_like_recovered_babel() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-2");
    session.set_loop_range_from_drag(&project, 3.5, 5.1);

    session.select_segment(&project, "annotation-3");

    assert!(session.loop_enabled);
    assert_eq!(session.selected_segment_id.as_deref(), Some("annotation-3"));
    assert_eq!(session.loop_range.start_seconds, 3.5);
    assert_eq!(session.loop_range.end_seconds, 5.1);
    assert_eq!(session.stored_loop_range, Some(session.loop_range));
}

#[test]
fn loop_clear_removes_recovered_babel_stored_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.set_loop_range_from_drag(&project, 3.5, 5.1);

    assert!(session.loop_enabled);
    assert!(session.stored_loop_range.is_some());

    session.clear_loop(&project);

    assert!(session.loop_enabled);
    assert!(session.stored_loop_range.is_none());
    assert!(!session.loop_strip(&project).has_range);
    assert_eq!(session.loop_label(), "Loop Off");
}

#[test]
fn transport_advances_playhead_with_speed_and_respects_loop_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-2");
    session.set_speed(2.0);

    session.advance_transport(&project, 1.25);

    assert_eq!(session.playhead_seconds, 3.5);

    session.toggle_playback();
    session.advance_transport(&project, 1.25);

    assert_eq!(session.playhead_seconds, 6.0);
    assert!(session.viewport.start_seconds <= session.playhead_seconds);
    assert!(session.viewport.end_seconds >= session.playhead_seconds);

    session.set_loop_range_from_drag(&project, 3.5, 5.1);
    session.seek_to_time(&project, 4.5);
    session.advance_transport(&project, 0.4);

    assert!(session.loop_enabled);
    assert!((session.playhead_seconds - 3.7).abs() < f64::EPSILON * 8.0);
    assert!(session.is_playing);
}

#[test]
fn playback_start_preserves_viewport_through_its_midpoint() {
    let project = project_with_segments(vec![segment("long", 0.0, 60.0, "long")]);
    let mut session = EditorSession::for_project(&project);
    let viewport = TimeRange {
        start_seconds: 10.0,
        end_seconds: 15.0,
    };

    session.viewport = viewport;
    session.playhead_seconds = 11.0;
    session.prepare_viewport_for_playback(&project);
    assert_eq!(session.viewport, viewport);

    session.playhead_seconds = 12.5;
    session.prepare_viewport_for_playback(&project);
    assert_eq!(session.viewport, viewport);
}

#[test]
fn playback_start_centers_playhead_when_outside_or_past_viewport_midpoint() {
    let project = project_with_segments(vec![segment("long", 0.0, 60.0, "long")]);
    let mut session = EditorSession::for_project(&project);

    for (playhead_seconds, expected_start_seconds) in [(4.0, 1.5), (13.0, 10.5), (30.0, 27.5)] {
        session.viewport = TimeRange {
            start_seconds: 10.0,
            end_seconds: 15.0,
        };
        session.playhead_seconds = playhead_seconds;

        session.prepare_viewport_for_playback(&project);

        assert!((session.viewport.start_seconds - expected_start_seconds).abs() < 0.000_001);
        assert!((session.viewport.duration_seconds() - 5.0).abs() < 0.000_001);
    }
}

#[test]
fn transport_follow_waits_until_playhead_crosses_viewport_midpoint() {
    let project = project_with_segments(vec![segment("long", 0.0, 60.0, "long")]);
    let mut session = EditorSession::for_project(&project);
    let viewport = TimeRange {
        start_seconds: 10.0,
        end_seconds: 15.0,
    };
    session.viewport = viewport;
    session.playhead_seconds = 11.0;
    session.is_playing = true;

    session.advance_transport(&project, 1.0);
    assert_eq!(session.playhead_seconds, 12.0);
    assert_eq!(session.viewport, viewport);

    session.advance_transport(&project, 1.0);
    assert_eq!(session.playhead_seconds, 13.0);
    assert!((session.viewport.start_seconds - 10.5).abs() < 0.000_001);
    assert!((session.viewport.end_seconds - 15.5).abs() < 0.000_001);
}

#[test]
fn transport_selects_segment_under_playhead_without_rewinding_playhead() {
    let project = project_with_segments(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 1.0, 2.0, "second"),
        segment("third", 3.0, 4.0, "third"),
    ]);
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "first");
    session.toggle_playback();

    session.advance_transport(&project, 1.25);

    assert_eq!(session.selected_segment_id.as_deref(), Some("second"));
    assert_eq!(session.playhead_seconds, 1.25);
}

#[test]
fn diff_view_adds_visible_row_markers() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    assert!(session.transcript_rows(&project)[0].diff_marker.is_none());
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    assert_eq!(rows[0].diff_marker.as_deref(), Some("Babel unchanged"));
    assert!(rows[0].diff_source_text.is_none());
}

#[test]
fn diff_view_reports_changed_fields_and_original_text_from_babel_snapshot() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "edited text");
    changed.speaker_id = "speaker-2".to_string();
    changed.lane_id = "track-2".to_string();
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("original text"),
    );
    changed.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(1.0),
    );
    changed.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(2.0),
    );
    changed.metadata.insert(
        "studio.babel.original.speakerId".to_string(),
        serde_json::json!("speaker-1"),
    );
    changed.metadata.insert(
        "studio.babel.original.laneId".to_string(),
        serde_json::json!("track-1"),
    );

    let mut unchanged = segment("annotation-2", 3.0, 4.0, "same text");
    unchanged.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("same text"),
    );
    unchanged.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(3.0),
    );
    unchanged.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(4.0),
    );
    unchanged.metadata.insert(
        "studio.babel.original.speakerId".to_string(),
        serde_json::json!("speaker-1"),
    );
    unchanged.metadata.insert(
        "studio.babel.original.laneId".to_string(),
        serde_json::json!("track-1"),
    );

    let added = Segment {
        id: "annotation-2__studio_1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 5.0,
        end_seconds: 6.0,
        text: "new native row".to_string(),
        metadata: Default::default(),
    };
    let project = two_lane_project(vec![changed, unchanged, added]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(
        rows[0].diff_marker.as_deref(),
        Some("Changed: text, timing, track")
    );
    assert_eq!(rows[0].diff_source_text.as_deref(), Some("original text"));
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Babel unchanged"));
    assert!(rows[1].diff_source_text.is_none());
    assert_eq!(rows[2].diff_marker.as_deref(), Some("New"));
    assert!(rows[2].diff_source_text.is_none());
}

#[test]
fn diff_view_uses_selected_compare_action_snapshot_for_row_diff_source() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "edited text");
    changed.speaker_id = "speaker-2".to_string();
    changed.lane_id = "track-2".to_string();
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("imported original"),
    );
    changed.metadata.insert(
        "studio.babel.diff.compareSnapshots".to_string(),
        serde_json::json!({
            "l0": {
                "text": "first review text",
                "startSeconds": 1.25,
                "endSeconds": 2.25,
                "speakerId": "speaker-1",
                "laneId": "track-1",
                "wordDiffs": [
                    { "status": "removed", "value": "first" },
                    { "status": "added", "value": "edited" },
                    { "status": "unchanged", "value": "text" }
                ]
            },
            "l1": {
                "text": "selected compare text",
                "startSeconds": 1.0,
                "endSeconds": 2.0,
                "speakerId": "speaker-1",
                "laneId": "track-1",
                "wordDiffs": [
                    { "status": "removed", "value": "selected" },
                    { "status": "added", "value": "edited" },
                    { "status": "unchanged", "value": "text" }
                ]
            }
        }),
    );
    let mut project = two_lane_project(vec![changed]);
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l1".to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let diff = rows[0].diff.as_ref().unwrap();

    assert_eq!(
        rows[0].diff_source_text.as_deref(),
        Some("selected compare text")
    );
    assert_eq!(
        rows[0].diff_marker.as_deref(),
        Some("Changed: text, timing, track")
    );
    assert_eq!(diff.badge_label, "1 change");
    assert_eq!(
        diff.word_diff
            .tokens
            .iter()
            .map(|token| (token.status, token.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (TranscriptWordDiffStatus::Removed, "selected "),
            (TranscriptWordDiffStatus::Added, "edited "),
            (TranscriptWordDiffStatus::Unchanged, "text "),
        ]
    );
}

#[test]
fn diff_view_exposes_babel_annotation_status_from_compare_snapshot() {
    let mut annotation = segment("annotation-1", 1.0, 2.0, "background-noise / rustle");
    annotation
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    annotation.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    annotation
        .metadata
        .insert("intensity".to_string(), serde_json::json!(4));
    annotation.metadata.insert(
        "studio.babel.diff.compareSnapshots".to_string(),
        serde_json::json!({
            "l0": {
                "content": {
                    "primaryLabel": "background-noise",
                    "secondaryLabel": "mic bump"
                },
                "intensity": 2,
                "startTimeInSeconds": 1.0,
                "endTimeInSeconds": 2.0
            }
        }),
    );
    let mut project = project_with_segments(vec![annotation]);
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let annotation_diff = rows[0].annotation_diff.as_ref().unwrap();

    assert_eq!(annotation_diff.status, "subcategory-mismatch");
    assert_eq!(
        annotation_diff.other_secondary_label.as_deref(),
        Some("mic bump")
    );
    assert_eq!(annotation_diff.other_intensity, Some(2));
    assert_eq!(annotation_diff.counterpart_start_seconds, Some(1.0));
    assert_eq!(annotation_diff.counterpart_end_seconds, Some(2.0));
}

#[test]
fn diff_view_exposes_babel_text_cell_badges_separate_from_linter() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "edited text");
    changed.speaker_id = "speaker-2".to_string();
    changed.lane_id = "track-2".to_string();
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("original text"),
    );
    changed.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(1.0),
    );
    changed.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(2.0),
    );
    changed.metadata.insert(
        "studio.babel.original.speakerId".to_string(),
        serde_json::json!("speaker-1"),
    );
    changed.metadata.insert(
        "studio.babel.original.laneId".to_string(),
        serde_json::json!("track-1"),
    );

    let added = Segment {
        id: "annotation-2__studio_1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 5.0,
        end_seconds: 6.0,
        text: "new native row".to_string(),
        metadata: Default::default(),
    };
    let project = two_lane_project(vec![changed, added]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.message, "OK");
    assert_eq!(rows[0].diff.as_ref().unwrap().badge_label, "1 change");
    assert_eq!(
        rows[0].diff.as_ref().unwrap().source_text.as_deref(),
        Some("original text")
    );
    assert_eq!(rows[1].lint.message, "OK");
    assert_eq!(rows[1].diff.as_ref().unwrap().badge_label, "New");
    assert!(rows[1].diff.as_ref().unwrap().source_text.is_none());
}

#[test]
fn action_menu_omits_native_restore_command_for_babel_snapshot_rows() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "edited text");
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("original text"),
    );
    changed.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(1.0),
    );
    changed.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(2.0),
    );
    let project = project_with_segments(vec![changed]);
    let session = EditorSession::for_project(&project);
    let row = session.transcript_rows(&project).remove(0);

    let menu = session.transcript_action_menu(&project, &row);

    assert!(
        !menu
            .items
            .iter()
            .any(|entry| action_menu_entry_summary(entry).1 == "Revert to original")
    );
}

#[test]
fn diff_view_renders_babel_word_diff_tokens_in_text_cell() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "alpha gamma new");
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("alpha beta old"),
    );
    changed.metadata.insert(
        "studio.babel.diff.wordDiffs".to_string(),
        serde_json::json!([
            { "status": "unchanged", "value": "alpha" },
            { "status": "removed", "value": "beta" },
            { "status": "added", "value": "gamma" },
            { "status": "removed", "value": "old" },
            { "status": "added", "value": "new" }
        ]),
    );
    let project = project_with_segments(vec![changed]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let diff = rows[0].diff.as_ref().unwrap();

    assert_eq!(diff.kind, TranscriptDiffCellKind::Diff);
    assert_eq!(diff.badge_label, "2 changes");
    assert_eq!(diff.word_diff.kind, TranscriptWordDiffKind::Tokens);
    assert_eq!(diff.word_diff.class_name, "text-xs leading-relaxed");
    assert_eq!(diff.word_diff.text, None);
    assert_eq!(
        diff.word_diff
            .tokens
            .iter()
            .map(|token| (
                token.status,
                token.value.as_str(),
                token.class_name.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                TranscriptWordDiffStatus::Unchanged,
                "alpha ",
                "text-foreground"
            ),
            (
                TranscriptWordDiffStatus::Removed,
                "beta ",
                "text-red-700 bg-red-100 line-through rounded px-0.5"
            ),
            (
                TranscriptWordDiffStatus::Added,
                "gamma ",
                "text-green-700 bg-green-100 rounded px-0.5"
            ),
            (
                TranscriptWordDiffStatus::Removed,
                "old ",
                "text-red-700 bg-red-100 line-through rounded px-0.5"
            ),
            (
                TranscriptWordDiffStatus::Added,
                "new ",
                "text-green-700 bg-green-100 rounded px-0.5"
            ),
        ]
    );
}

#[test]
fn diff_view_renders_babel_empty_diff_and_new_cell_models() {
    let mut unchanged_empty = segment("annotation-1", 1.5, 2.5, "");
    unchanged_empty.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!(""),
    );
    unchanged_empty.metadata.insert(
        "studio.babel.diff.wordDiffs".to_string(),
        serde_json::json!([]),
    );

    let added_empty = Segment {
        id: "annotation-2__studio_1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 5.0,
        end_seconds: 6.0,
        text: "".to_string(),
        metadata: Default::default(),
    };
    let project = project_with_segments(vec![unchanged_empty, added_empty]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let unchanged_diff = rows[0].diff.as_ref().unwrap();
    let added_diff = rows[1].diff.as_ref().unwrap();

    assert_eq!(unchanged_diff.kind, TranscriptDiffCellKind::Diff);
    assert_eq!(unchanged_diff.badge_label, "No changes");
    assert_eq!(unchanged_diff.word_diff.kind, TranscriptWordDiffKind::Empty);
    assert_eq!(unchanged_diff.word_diff.text.as_deref(), Some("No content"));
    assert_eq!(
        unchanged_diff.word_diff.class_name,
        "text-muted-foreground italic text-xs leading-relaxed"
    );

    assert_eq!(added_diff.kind, TranscriptDiffCellKind::New);
    assert_eq!(added_diff.badge_label, "New");
    assert_eq!(added_diff.word_diff.kind, TranscriptWordDiffKind::Plain);
    assert_eq!(added_diff.word_diff.text.as_deref(), Some("(empty)"));
    assert_eq!(added_diff.word_diff.direction, TranscriptTextDirection::Ltr);
    assert_eq!(
        added_diff.word_diff.class_name,
        "text-xs text-muted-foreground"
    );
}

#[test]
fn diff_view_interleaves_recovered_babel_deleted_segment_rows() {
    let project = project_with_segments(vec![
        segment("current-a", 1.0, 1.5, "current before"),
        segment("current-b", 4.0, 4.5, "current after"),
    ]);
    let mut project = project;
    project.metadata.insert(
        "studio.babel.diff.result".to_string(),
        serde_json::json!({
            "speakerDiffs": [{
                "segmentMappings": [{
                    "relationship": "deleted",
                    "segmentsA": [{
                        "annotationId": "deleted-reference",
                        "startTimeInSeconds": 2.0,
                        "endTimeInSeconds": 3.0,
                        "trackId": "track-1",
                        "speakerId": "speaker-1"
                    }],
                    "segmentsB": [],
                    "wordDiffs": [
                        { "status": "removed", "value": "old" },
                        { "status": "removed", "value": "missing" }
                    ]
                }]
            }]
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].row_kind, TranscriptRowKind::ModifiedDeleted);
    assert_eq!(rows[1].segment_id, "modified-deleted-reference");
    assert_eq!(rows[1].display_id, "2");
    assert_eq!(rows[1].speaker_label, "Deleted");
    assert_eq!(rows[1].start_time, "00:02.00");
    assert_eq!(rows[1].end_time, "00:03.00");
    assert!(rows[1].is_read_only);
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Deleted"));
    let diff = rows[1].diff.as_ref().unwrap();
    assert_eq!(diff.kind, TranscriptDiffCellKind::ModifiedDeleted);
    assert_eq!(diff.badge_label, "Deleted");
    assert_eq!(
        diff.word_diff
            .tokens
            .iter()
            .map(|token| (token.status, token.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (TranscriptWordDiffStatus::Removed, "old "),
            (TranscriptWordDiffStatus::Removed, "missing "),
        ]
    );
}

#[test]
fn diff_view_interleaves_recovered_babel_compare_newer_added_segment_rows() {
    let mut project = project_with_segments(vec![
        segment("current-a", 1.0, 1.5, "current before"),
        segment("current-b", 4.0, 4.5, "current after"),
    ]);
    project.metadata.insert(
        "studio.babel.diff.isCompareNewer".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.result".to_string(),
        serde_json::json!({
            "speakerDiffs": [{
                "segmentMappings": [{
                    "relationship": "added",
                    "segmentsA": [],
                    "segmentsB": [{
                        "annotationId": "newer-added",
                        "startTimeInSeconds": 2.0,
                        "endTimeInSeconds": 3.0,
                        "trackId": "track-1",
                        "speakerId": "speaker-1"
                    }],
                    "wordDiffs": [
                        { "status": "added", "value": "newer" },
                        { "status": "added", "value": "line" }
                    ]
                }]
            }]
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].row_kind, TranscriptRowKind::ModifiedAdded);
    assert_eq!(rows[1].segment_id, "modified-newer-added");
    assert_eq!(rows[1].display_id, "2");
    assert_eq!(rows[1].speaker_label, "Added");
    assert!(rows[1].is_read_only);
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Added"));
    let diff = rows[1].diff.as_ref().unwrap();
    assert_eq!(diff.kind, TranscriptDiffCellKind::ModifiedAdded);
    assert_eq!(diff.badge_label, "Added");
    assert_eq!(
        diff.word_diff
            .tokens
            .iter()
            .map(|token| (token.status, token.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (TranscriptWordDiffStatus::Added, "newer "),
            (TranscriptWordDiffStatus::Added, "line "),
        ]
    );
}

#[test]
fn diff_view_interleaves_recovered_babel_removed_annotation_ghost_rows() {
    let mut project = project_with_segments(vec![
        segment("current-a", 1.0, 1.5, "current before"),
        segment("current-b", 4.0, 4.5, "current after"),
    ]);
    project.metadata.insert(
        "studio.babel.diff.annotationGhostRows".to_string(),
        serde_json::json!([{
            "comparisonLabel": "Reference",
            "annotation": {
                "id": "removed-annotation",
                "content": {
                    "primaryLabel": "background-noise",
                    "secondaryLabel": "rustle"
                },
                "intensity": 3,
                "startTimeInSeconds": 2.0,
                "endTimeInSeconds": 3.0
            }
        }])
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].row_kind, TranscriptRowKind::AnnotationRemovedGhost);
    assert_eq!(rows[1].segment_id, "annotation-ghost-removed-annotation");
    assert_eq!(rows[1].display_id, "2");
    assert_eq!(rows[1].speaker_label, "Reference");
    assert_eq!(
        rows[1].annotation_kind,
        TranscriptAnnotationKind::NonTranscription("removed".to_string())
    );
    assert_eq!(rows[1].annotation_type_label, "Removed");
    assert_eq!(rows[1].start_time, "00:02.00");
    assert_eq!(rows[1].end_time, "00:03.00");
    assert_eq!(rows[1].text, "background-noise / rustle · intensity 3");
    assert!(rows[1].is_read_only);
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Removed"));
    let annotation_diff = rows[1].annotation_diff.as_ref().unwrap();
    assert_eq!(annotation_diff.status, "unmatched-reference");
}

#[test]
fn warning_assertions_hide_active_warnings_from_problem_navigation() {
    let project = project_with_segments(vec![
        segment("warning", 1.0, 60.0, "very long but intentionally valid"),
        segment("clean", 61.0, 62.0, "clean"),
        segment("error", 63.0, 64.0, ""),
    ]);
    let mut session = EditorSession::for_project(&project);

    assert_eq!(session.problem_row_count(&project), 2);
    session.select_segment(&project, "warning");
    assert!(session.toggle_warning_assertion(&project));

    let warning_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "warning")
        .unwrap();
    assert_eq!(warning_row.lint.severity, LintSeverity::Pass);
    assert_eq!(warning_row.lint.message, "Warning asserted");
    assert!(warning_row.has_asserted_warning);
    assert_eq!(session.problem_row_count(&project), 1);

    session.navigate_problem_rows(&project, 1);
    assert_eq!(session.selected_segment_id.as_deref(), Some("error"));
}

#[test]
fn row_views_expose_directional_merge_availability_per_lane() {
    let mut other_lane = segment("other-lane", 2.0, 3.0, "interleaved");
    other_lane.lane_id = "track-2".to_string();
    other_lane.speaker_id = "speaker-2".to_string();
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        other_lane,
        segment("second", 4.0, 5.0, "second"),
        segment("third", 7.0, 8.0, "third"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    let first = rows.iter().find(|row| row.segment_id == "first").unwrap();
    let second = rows.iter().find(|row| row.segment_id == "second").unwrap();
    let third = rows.iter().find(|row| row.segment_id == "third").unwrap();

    assert!(!first.can_merge_above);
    assert!(first.can_merge_below);
    assert!(second.can_merge_above);
    assert!(second.can_merge_below);
    assert!(third.can_merge_above);
    assert!(!third.can_merge_below);
}

#[test]
fn transcript_action_menu_is_compact_babel_kebab_model() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 2.0, 3.0, "second"),
    ]);
    let session = EditorSession::for_project(&project);
    let second_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "second")
        .unwrap();

    let menu = session.transcript_action_menu(&project, &second_row);

    assert_eq!(menu.segment_id, "second");
    assert_eq!(
        menu.items
            .iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            ("item", "Add Segment Above", true),
            ("item", "Add Segment Below", true),
            ("separator", "", false),
            ("item", "Merge With Above", true),
            ("item", "Merge With Below", false),
            ("separator", "", false),
            ("item", "Delete", true),
        ]
    );
}

#[test]
fn transcript_action_menu_exposes_edit_for_non_transcription_babel_annotations() {
    let mut annotation = segment("noise", 0.0, 1.0, "background-noise / rustle");
    annotation
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    let project = project_with_segments(vec![annotation]);
    let session = EditorSession::for_project(&project);
    let row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "noise")
        .unwrap();

    let menu = session.transcript_action_menu(&project, &row);

    assert_eq!(
        menu.items
            .iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            ("item", "Edit", true),
            ("separator", "", false),
            ("item", "Merge With Above", false),
            ("item", "Merge With Below", false),
            ("separator", "", false),
            ("item", "Delete", true),
        ]
    );
}

#[test]
fn transcript_action_menu_keeps_track_selection_out_of_recovered_babel_overflow_menu() {
    let source = include_str!("../src/transcript_model/actions.rs");
    let menu_struct = source
        .split("pub struct TranscriptActionMenuItemView")
        .next()
        .expect("action menu struct should be before menu item struct");

    assert!(!menu_struct.contains("selected_lane_id"));
    assert!(!menu_struct.contains("selected_track_option"));
    assert!(!menu_struct.contains("track_options"));
    assert!(!menu_struct.contains("TranscriptActionMenuLayout"));
}

#[test]
fn transcript_action_menu_keeps_recovered_babel_insert_actions_even_when_rows_touch() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 1.0, 2.0, "second"),
    ]);
    let session = EditorSession::for_project(&project);
    let first_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "first")
        .unwrap();
    let second_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "second")
        .unwrap();

    let first_menu = session.transcript_action_menu(&project, &first_row);
    let menu = session.transcript_action_menu(&project, &second_row);

    assert_eq!(
        first_menu
            .items
            .iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            ("item", "Add Segment Above", true),
            ("item", "Add Segment Below", true),
            ("separator", "", false),
            ("item", "Merge With Above", false),
            ("item", "Merge With Below", true),
            ("separator", "", false),
            ("item", "Delete", true),
        ]
    );
    assert_eq!(
        menu.items
            .iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            ("item", "Add Segment Above", true),
            ("item", "Add Segment Below", true),
            ("separator", "", false),
            ("item", "Merge With Above", true),
            ("item", "Merge With Below", false),
            ("separator", "", false),
            ("item", "Delete", true),
        ]
    );
}

#[test]
fn display_ids_are_visible_row_order_numbers_not_babel_annotation_ids() {
    let project = project_with_segments(vec![
        segment("annotation-20", 0.0, 1.0, "twenty"),
        segment("annotation-2", 2.0, 3.0, "two"),
        Segment {
            id: "annotation-20__studio_1".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 4.0,
            end_seconds: 5.0,
            text: "derived".to_string(),
            metadata: Default::default(),
        },
    ]);

    let rows = EditorSession::for_project(&project).transcript_rows(&project);

    assert_eq!(
        rows.iter()
            .map(|row| row.segment_id.as_str())
            .collect::<Vec<_>>(),
        vec!["annotation-20", "annotation-2", "annotation-20__studio_1"]
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.display_id.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2", "3"]
    );
}

#[test]
fn tts_context_panel_model_matches_recovered_babel_contract() {
    let context = BTreeMap::from([
        ("speech".to_string(), Some("slow".to_string())),
        ("physical".to_string(), Some("  ".to_string())),
        ("character".to_string(), Some("Alice".to_string())),
        ("vocal".to_string(), None),
        ("preamble".to_string(), Some(" keep leading ".to_string())),
    ]);

    let panel = build_script_context_panel_model(ScriptContextPanelInput {
        emotion: Some("calm".to_string()),
        submission_id: Some("submission-123".to_string()),
        context: Some(context),
        hidden: true,
        is_rtl: true,
        copied_submission_id: true,
    })
    .expect("context, emotion, or submission id should show script context panel");

    assert_eq!(panel.panel.title, "Script Context");
    assert_eq!(panel.panel.storage_key, SCRIPT_CONTEXT_STORAGE_KEY);
    assert_eq!(panel.panel.dir, ContextPanelTextDirection::Rtl);
    assert!(panel.panel.hidden);
    assert!(!panel.panel.content_visible);
    assert_eq!(panel.panel.toggle_tooltip, "Show Script Context");
    assert_eq!(panel.panel.toggle_aria_label, "Show Script Context");
    assert!(!panel.panel.toggle_aria_expanded);
    assert_eq!(panel.panel.toggle_icon, CollapsiblePanelToggleIcon::Show);
    assert!(panel.panel.toggle_is_active);
    assert_eq!(panel.emotion.as_deref(), Some("calm"));
    assert_eq!(
        panel
            .fields
            .iter()
            .map(|field| (
                field.key.as_str(),
                field.label.as_str(),
                field.value.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("character", "Character", "Alice"),
            ("speech", "Speech", "slow"),
            ("preamble", "Preamble", " keep leading "),
        ]
    );

    let submission = panel.submission.expect("submission id model should exist");
    assert_eq!(submission.label, "Submission ID");
    assert_eq!(submission.code_dir, ContextPanelTextDirection::Ltr);
    assert_eq!(submission.submission_id, "submission-123");
    assert_eq!(submission.button_title, "Copy submission ID");
    assert_eq!(submission.button_aria_label, "Copy submission ID");
    assert_eq!(submission.icon, SubmissionCopyIcon::Check);
    assert_eq!(submission.clipboard_text, "submission-123");
}

#[test]
fn tts_context_panel_toggle_and_copy_effects_match_recovered_babel() {
    assert!(read_stored_collapsed_state(Some("true")));
    assert!(!read_stored_collapsed_state(Some("false")));
    assert!(!read_stored_collapsed_state(Some("TRUE")));
    assert!(!read_stored_collapsed_state(None));

    let panel = build_collapsible_panel_model(CollapsiblePanelInput {
        title: "Original Script".to_string(),
        storage_key: ORIGINAL_SCRIPT_STORAGE_KEY.to_string(),
        hidden: false,
        is_rtl: false,
    });
    assert_eq!(panel.dir, ContextPanelTextDirection::Ltr);
    assert!(panel.content_visible);
    assert_eq!(panel.toggle_tooltip, "Hide Original Script");
    assert_eq!(panel.toggle_aria_label, "Hide Original Script");
    assert!(panel.toggle_aria_expanded);
    assert_eq!(panel.toggle_icon, CollapsiblePanelToggleIcon::Hide);
    assert!(!panel.toggle_is_active);

    let toggle = reduce_collapsible_panel_toggle(SCRIPT_CONTEXT_STORAGE_KEY, false);
    assert!(toggle.hidden);
    assert_eq!(
        toggle.effects,
        vec![TtsPanelEffect::LocalStorageSet {
            key: SCRIPT_CONTEXT_STORAGE_KEY.to_string(),
            value: "true".to_string(),
        }]
    );

    let success = resolve_submission_id_copy_success();
    assert!(success.copied);
    assert_eq!(
        success.effects,
        vec![TtsPanelEffect::ResetCopiedAfterMs {
            ms: SUBMISSION_ID_COPY_RESET_DELAY_MS,
        }]
    );

    let failure = resolve_submission_id_copy_failure("permission denied");
    assert!(!failure.copied);
    assert_eq!(
        failure.effects,
        vec![TtsPanelEffect::ConsoleError {
            prefix: "Failed to copy submission ID:".to_string(),
            error: "permission denied".to_string(),
        }]
    );
}

#[test]
fn original_script_panel_model_matches_recovered_babel_contract() {
    let panel = build_original_script_panel_model(OriginalScriptPanelInput {
        script: "Original line\nSecond line".to_string(),
        hidden: false,
        is_rtl: false,
    });

    assert_eq!(panel.panel.title, "Original Script");
    assert_eq!(panel.panel.storage_key, ORIGINAL_SCRIPT_STORAGE_KEY);
    assert_eq!(panel.panel.dir, ContextPanelTextDirection::Ltr);
    assert!(!panel.panel.hidden);
    assert!(panel.panel.content_visible);
    assert_eq!(panel.panel.toggle_tooltip, "Hide Original Script");
    assert_eq!(panel.panel.toggle_aria_label, "Hide Original Script");
    assert!(panel.panel.toggle_aria_expanded);
    assert_eq!(panel.panel.toggle_icon, CollapsiblePanelToggleIcon::Hide);
    assert!(!panel.panel.toggle_is_active);
    assert_eq!(panel.script, "Original line\nSecond line");
}

#[test]
fn script_context_panel_stays_absent_until_babel_inputs_exist() {
    assert!(
        build_script_context_panel_model(ScriptContextPanelInput {
            emotion: None,
            submission_id: None,
            context: None,
            hidden: false,
            is_rtl: false,
            copied_submission_id: false,
        })
        .is_none()
    );

    assert!(
        build_script_context_panel_model(ScriptContextPanelInput {
            emotion: Some(String::new()),
            submission_id: Some(String::new()),
            context: None,
            hidden: false,
            is_rtl: false,
            copied_submission_id: false,
        })
        .is_none()
    );
}

#[test]
fn no_work_modal_model_and_events_match_recovered_babel_contract() {
    let model = build_no_work_modal_model(NoWorkModalInput {
        open: true,
        mode: "review".to_string(),
    });

    assert!(model.open);
    assert!(!model.can_user_close);
    assert_eq!(
        model.title,
        "Sorry, we weren't able to find a review job for you!"
    );
    assert_eq!(model.description, NO_WORK_DESCRIPTION);
    assert_eq!(model.button_label, "Back to Projects");
    assert_eq!(
        reduce_no_work_modal_event(NoWorkModalEvent::OpenChange { open: true }),
        Vec::<NoWorkModalEffect>::new()
    );
    assert_eq!(
        reduce_no_work_modal_event(NoWorkModalEvent::OpenChange { open: false }),
        vec![NoWorkModalEffect::Close]
    );
    assert_eq!(
        reduce_no_work_modal_event(NoWorkModalEvent::ClickBackToProjects),
        vec![NoWorkModalEffect::Close, NoWorkModalEffect::BackToProjects]
    );
}

#[test]
fn task_lookup_select_action_model_matches_recovered_babel_contract() {
    let stitched = build_select_action_model(TaskLookupSelectInput {
        processed_transcription_id: "processed-1".to_string(),
        review_action_id: String::new(),
        show_stitched_option: Some(true),
    });

    assert_eq!(stitched.branch, "select-action");
    assert_eq!(stitched.title, "Select Action");
    assert_eq!(stitched.description, STITCHED_SELECT_DESCRIPTION);
    assert!(stitched.show_processed_transcription_input);
    assert_eq!(
        stitched.processed_transcription_placeholder,
        PROCESSED_TRANSCRIPTION_PLACEHOLDER
    );
    assert_eq!(
        stitched.review_action_placeholder,
        REVIEW_ACTION_PLACEHOLDER
    );
    assert_eq!(stitched.processed_transcription_id, "processed-1");
    assert_eq!(stitched.review_action_id, "");
    assert!(!stitched.claim_stitched_disabled);
    assert!(!stitched.read_only_disabled);

    let read_only = build_select_action_model(TaskLookupSelectInput {
        processed_transcription_id: String::new(),
        review_action_id: String::new(),
        show_stitched_option: None,
    });

    assert_eq!(read_only.description, READ_ONLY_SELECT_DESCRIPTION);
    assert!(!read_only.show_processed_transcription_input);
    assert!(read_only.claim_stitched_disabled);
    assert!(read_only.read_only_disabled);
}

#[test]
fn task_lookup_reducer_trims_inputs_and_matches_confirm_edit_priority() {
    assert_eq!(trim_task_lookup_field_value("  abc \n"), "abc");

    let state = create_initial_task_lookup_state(TaskLookupInitialInput {
        processed_transcription_id: String::new(),
        review_action_id: String::new(),
        show_stitched_option: Some(true),
    });

    let changed = reduce_task_lookup_event(
        state,
        TaskLookupEvent::ChangeProcessedTranscriptionId {
            value: "  processed-7  ".to_string(),
        },
    );
    assert_eq!(changed.state.processed_transcription_id, "processed-7");
    assert_eq!(
        changed.effects,
        vec![TaskLookupEffect::SetProcessedTranscriptionId {
            value: "processed-7".to_string(),
        }]
    );

    let confirming = reduce_task_lookup_event(changed.state, TaskLookupEvent::ClickClaimStitched);
    assert!(confirming.state.confirming_edit);
    assert!(confirming.effects.is_empty());

    let canceled = reduce_task_lookup_event(confirming.state, TaskLookupEvent::CancelConfirmEdit);
    assert!(!canceled.state.confirming_edit);
    assert!(canceled.effects.is_empty());

    let confirming = reduce_task_lookup_event(canceled.state, TaskLookupEvent::ClickClaimStitched);
    let confirmed = reduce_task_lookup_event(confirming.state, TaskLookupEvent::ConfirmEdit);
    assert!(!confirmed.state.confirming_edit);
    assert_eq!(confirmed.effects, vec![TaskLookupEffect::ClaimById]);

    let blocked = reduce_task_lookup_event(
        create_initial_task_lookup_state(TaskLookupInitialInput {
            processed_transcription_id: "processed-7".to_string(),
            review_action_id: String::new(),
            show_stitched_option: Some(false),
        }),
        TaskLookupEvent::ClickClaimStitched,
    );
    assert!(!blocked.state.confirming_edit);
    assert!(blocked.effects.is_empty());
}

#[test]
fn task_lookup_read_only_prefers_review_action_id_then_processed_transcription_id() {
    let by_review_action = reduce_task_lookup_event(
        create_initial_task_lookup_state(TaskLookupInitialInput {
            processed_transcription_id: "processed-7".to_string(),
            review_action_id: "review-action-4".to_string(),
            show_stitched_option: Some(true),
        }),
        TaskLookupEvent::ClickReadOnly,
    );
    assert_eq!(
        by_review_action.effects,
        vec![TaskLookupEffect::ViewReadOnlyByReviewActionId]
    );

    let by_processed = reduce_task_lookup_event(
        create_initial_task_lookup_state(TaskLookupInitialInput {
            processed_transcription_id: "processed-7".to_string(),
            review_action_id: String::new(),
            show_stitched_option: Some(true),
        }),
        TaskLookupEvent::ClickReadOnly,
    );
    assert_eq!(
        by_processed.effects,
        vec![TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId]
    );

    let no_target = reduce_task_lookup_event(
        create_initial_task_lookup_state(TaskLookupInitialInput {
            processed_transcription_id: String::new(),
            review_action_id: String::new(),
            show_stitched_option: Some(true),
        }),
        TaskLookupEvent::ClickReadOnly,
    );
    assert!(no_target.effects.is_empty());

    let confirm = build_confirm_edit_model();
    assert_eq!(confirm.branch, "confirm-edit");
    assert_eq!(confirm.title, "Confirm Edit");
    assert_eq!(confirm.cancel_label, "Cancel");
    assert_eq!(confirm.confirm_label, "Confirm Edit");
    assert!(confirm.body.contains("submit again"));
    assert!(confirm.body.contains("read only flow"));
}

#[test]
fn feedback_rubric_flags_match_recovered_babel_contract() {
    assert_eq!(FEEDBACK_MAX_COMMENT_LENGTH, 500);
    assert_eq!(
        get_rubric_fields_for_flags(None)
            .iter()
            .map(|field| field.label)
            .collect::<Vec<_>>(),
        vec![
            "Word Accuracy",
            "Timestamp Accuracy",
            "Punctuation & Formatting",
            "Tags & Emphasis",
            "Segmentation",
        ]
    );
    assert_eq!(
        get_rubric_fields_for_flags(Some(&["tts-style-tag-accuracy", "tts-other-feedback",]))
            .iter()
            .map(|field| (field.key, field.label, field.required))
            .collect::<Vec<_>>(),
        vec![("styleTagAccuracy", "Style Tag Accuracy", true)]
    );
    assert!(has_other_feedback_flag(None));
    assert!(has_other_feedback_flag(Some(&["tts-other-feedback"])));
    assert!(!has_other_feedback_flag(Some(&["tts-style-tag-accuracy"])));
}

#[test]
fn feedback_response_parsing_normalizes_ratings_comments_and_other_feedback() {
    let inputs = vec![
        FeedbackInput {
            id: "word-rating".to_string(),
            label: Some("Word Accuracy".to_string()),
        },
        FeedbackInput {
            id: "word-comment".to_string(),
            label: Some("Word Accuracy Comment".to_string()),
        },
        FeedbackInput {
            id: "timestamp-rating".to_string(),
            label: Some("Timestamp Accuracy".to_string()),
        },
        FeedbackInput {
            id: "other".to_string(),
            label: Some("Other Feedback".to_string()),
        },
    ];
    let responses = vec![
        FeedbackInputResponse {
            form_input_id: "word-rating".to_string(),
            value: "3".to_string(),
        },
        FeedbackInputResponse {
            form_input_id: "word-comment".to_string(),
            value: "Great wording".to_string(),
        },
        FeedbackInputResponse {
            form_input_id: "timestamp-rating".to_string(),
            value: "9".to_string(),
        },
        FeedbackInputResponse {
            form_input_id: "other".to_string(),
            value: "Global note".to_string(),
        },
    ];

    let draft = parse_feedback_responses(&responses, Some(&inputs))
        .expect("responses plus inputs should parse feedback");

    assert_eq!(
        draft.entries.get("wordAccuracy"),
        Some(&FeedbackRubricEntry {
            rating: 3,
            comment: "Great wording".to_string(),
        })
    );
    assert_eq!(
        draft.entries.get("timestampAccuracy"),
        Some(&FeedbackRubricEntry {
            rating: 1,
            comment: String::new(),
        })
    );
    assert_eq!(draft.other_feedback, "Global note");
    assert_eq!(parse_feedback_responses(&responses, None), None);
    assert_eq!(parse_feedback_responses(&[], Some(&inputs)), None);
}

#[test]
fn feedback_save_reset_and_success_mapping_match_recovered_babel_contract() {
    let mut draft = FeedbackDraft::default();
    draft.entries.insert(
        "wordAccuracy".to_string(),
        FeedbackRubricEntry {
            rating: 2,
            comment: "Fix one word".to_string(),
        },
    );
    draft.entries.insert(
        "timestampAccuracy".to_string(),
        FeedbackRubricEntry {
            rating: 3,
            comment: "Good timing".to_string(),
        },
    );
    draft.other_feedback = "Other note".to_string();
    let inputs = vec![
        FeedbackInput {
            id: "word-rating".to_string(),
            label: Some("Word Accuracy".to_string()),
        },
        FeedbackInput {
            id: "word-comment".to_string(),
            label: Some("Word Accuracy Comment".to_string()),
        },
        FeedbackInput {
            id: "timestamp-rating".to_string(),
            label: Some("Timestamp Accuracy".to_string()),
        },
        FeedbackInput {
            id: "other".to_string(),
            label: Some("Other Feedback".to_string()),
        },
        FeedbackInput {
            id: "tts-other".to_string(),
            label: Some("TTS Other Feedback".to_string()),
        },
    ];

    assert_eq!(
        serialize_feedback_draft(&draft, Some(&inputs), None, None, None),
        vec![
            FeedbackInputResponse {
                form_input_id: "word-rating".to_string(),
                value: "2".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "word-comment".to_string(),
                value: "Fix one word".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "timestamp-rating".to_string(),
                value: "3".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "other".to_string(),
                value: "Other note".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "tts-other".to_string(),
                value: "Other note".to_string(),
            },
        ]
    );
    assert_eq!(
        compute_feedback_draft_save_request(FeedbackSaveRequestInput {
            enabled: false,
            form_step_response_id: Some("response-1".to_string()),
            draft_feedback: draft.clone(),
            inputs: Some(inputs.clone()),
            rubric_fields: None,
            rubric_flag_names: None,
            has_other_feedback: None,
        }),
        None
    );
    let save = compute_feedback_draft_save_request(FeedbackSaveRequestInput {
        enabled: true,
        form_step_response_id: Some("response-1".to_string()),
        draft_feedback: draft.clone(),
        inputs: Some(inputs.clone()),
        rubric_fields: None,
        rubric_flag_names: None,
        has_other_feedback: Some(false),
    })
    .expect("enabled save with response id should serialize");
    assert_eq!(save.form_step_response_id, "response-1");
    assert_eq!(save.input_responses.len(), 3);

    let success = studio_app::apply_feedback_draft_success(FeedbackDraftSuccessInput {
        response: FeedbackDraftSuccessResponse {
            form_step_response_id: "response-2".to_string(),
            updated_at: "2026-06-22T10:20:30Z".to_string(),
            input_responses: Some(save.input_responses.clone()),
        },
        cached_inputs: Some(inputs.clone()),
        query_inputs: None,
    });
    assert_eq!(success.form_step_response_id, "response-2");
    assert_eq!(success.last_saved.as_deref(), Some("2026-06-22T10:20:30Z"));
    assert!(success.draft_feedback.is_some());

    let runtime = FeedbackDraftRuntimeState {
        draft_feedback: Some(draft.clone()),
        last_saved: Some("yesterday".to_string()),
        form_step_response_id: Some("response-1".to_string()),
        pending_draft: Some(draft),
        has_timer: true,
        cached_inputs: Some(inputs),
    };
    let reset = compute_feedback_draft_reset(runtime.clone());
    assert!(reset.draft_feedback.is_none());
    assert!(reset.last_saved.is_none());
    assert!(reset.form_step_response_id.is_none());
    assert!(reset.pending_draft.is_none());
    assert!(!reset.has_timer);
    assert!(reset.cached_inputs.is_some());

    let cleared = clear_feedback_draft(runtime);
    assert!(cleared.draft_feedback.is_none());
    assert!(cleared.last_saved.is_none());
    assert_eq!(cleared.form_step_response_id.as_deref(), Some("response-1"));
    assert!(cleared.pending_draft.is_none());
    assert!(!cleared.has_timer);
}

#[test]
fn assessment_phase_submit_success_and_retry_match_recovered_babel_contract() {
    assert_eq!(compute_assessment_phase(false, Some(true)), None);
    assert_eq!(
        compute_assessment_phase(true, Some(true)),
        Some(studio_app::AssessmentPhase::Practice)
    );
    assert_eq!(
        compute_assessment_phase(true, Some(false)),
        Some(studio_app::AssessmentPhase::Assessment)
    );
    assert_eq!(compute_assessment_phase(true, None), None);
    assert_eq!(
        compute_assessment_submit_target(Some(studio_app::AssessmentPhase::Assessment)),
        studio_app::AssessmentSubmitTarget::FinalAssessment
    );
    assert_eq!(
        compute_assessment_submit_target(Some(studio_app::AssessmentPhase::Practice)),
        studio_app::AssessmentSubmitTarget::PracticeAttempt
    );
    assert_eq!(
        compute_assessment_submit_target(None),
        studio_app::AssessmentSubmitTarget::PracticeAttempt
    );

    let practice = compute_practice_submit_success();
    assert!(practice.is_assessment_read_only);
    assert!(practice.enable_diff_mode);
    assert!(practice.refetch_diff);

    let final_success = compute_final_assessment_submit_success();
    assert!(final_success.is_assessment_read_only);
    assert!(final_success.is_final_submitted);
    assert!(final_success.invalidate_projects);

    let retry = compute_assessment_retry();
    assert!(!retry.is_assessment_read_only);
    assert!(retry.disable_diff_mode);
}
