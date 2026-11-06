use exaudio_babel_domain::workflow::BabelWorkflowExt;
use exaudio_babel_lint::linter::{LintSeverity, ServerLintIssue};
use studio_babel::BabelTaskPackage;
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_editor::workflow::StudioWorkflow;

const SAMPLE_TASK_JSON: &str =
    include_str!("../../../crates/studio-editor/tests/fixtures/stable-workflow-task.json");
const REVIEW_ACTION_JSON: &str =
    include_str!("../../../crates/studio-babel/tests/fixtures/review-action-final.json");

fn workflow_with_segments(segments: Vec<Segment>) -> StudioWorkflow {
    StudioWorkflow::from_babel_project(Project {
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
    })
}

fn segment(id: &str, start_seconds: f64, end_seconds: f64, text: &str) -> Segment {
    Segment {
        id: id.to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds,
        end_seconds,
        text: text.to_string(),
        metadata: Default::default(),
    }
}

#[test]
fn workflow_imports_babel_json_and_exports_after_text_edit() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .apply_text_edit("annotation-1", "Исправленный текст.")
        .expect("text edit should apply");

    let exported = workflow
        .export_babel_json()
        .expect("edited project should export");
    let package: BabelTaskPackage =
        serde_json::from_str(&exported).expect("export should remain Babel JSON");

    let edited = package
        .annotations
        .iter()
        .find(|annotation| annotation.annotation_id == "annotation-1")
        .expect("edited annotation should exist");
    assert_eq!(edited.text, "Исправленный текст.");
}

#[test]
fn workflow_imports_babel_review_action_json() {
    let workflow =
        StudioWorkflow::from_babel_json(REVIEW_ACTION_JSON).expect("review action should import");

    let project = workflow.project();
    assert_eq!(project.id, "f5b736b9-60ff-5355-ae5a-e909aa644e72");
    assert_eq!(project.lanes.len(), 2);
    assert_eq!(project.segments.len(), 24);
    assert!(
        project
            .segment("6219876d-eba7-529f-b6c9-d949dbabfd12")
            .is_some()
    );
}

#[test]
fn workflow_exports_review_action_json_in_original_envelope_shape() {
    let mut workflow =
        StudioWorkflow::from_babel_json(REVIEW_ACTION_JSON).expect("review action should import");
    workflow
        .apply_text_edit(
            "6219876d-eba7-529f-b6c9-d949dbabfd12",
            "Edited through workflow.",
        )
        .expect("text edit should apply");

    let exported = workflow
        .export_babel_json()
        .expect("review action should export");
    let value: serde_json::Value =
        serde_json::from_str(&exported).expect("export should be valid JSON");

    assert!(value.is_array());
    assert_eq!(
        value.pointer("/0/result/data/json/actionId"),
        Some(&serde_json::json!("e823c3bb-2697-55fc-9272-c083a55874da"))
    );
    let annotations = value
        .pointer("/0/result/data/json/annotations")
        .and_then(serde_json::Value::as_array)
        .expect("annotations should remain under result.data.json");
    let edited = annotations
        .iter()
        .find(|annotation| {
            annotation.get("id") == Some(&serde_json::json!("6219876d-eba7-529f-b6c9-d949dbabfd12"))
        })
        .expect("edited annotation should export");
    assert_eq!(edited["content"], "Edited through workflow.");
}

#[test]
fn workflow_server_lint_metadata_tracks_current_snapshot_and_invalidation() {
    let mut workflow =
        StudioWorkflow::from_babel_json(REVIEW_ACTION_JSON).expect("review action should import");
    let issue = ServerLintIssue {
        annotation_id: "6219876d-eba7-529f-b6c9-d949dbabfd12".to_string(),
        reason: "Server-authoritative issue.".to_string(),
        severity: LintSeverity::Error,
    };

    workflow.set_babel_lint_snapshot_metadata(std::slice::from_ref(&issue));
    let exported: serde_json::Value = serde_json::from_str(
        &workflow
            .export_babel_json()
            .expect("review action with current lint should export"),
    )
    .expect("export should remain valid JSON");
    assert_eq!(
        exported.pointer("/0/result/data/json/lintErrors/0"),
        Some(&serde_json::json!({
            "annotationId": issue.annotation_id,
            "reason": issue.reason,
            "severity": "error",
        }))
    );

    workflow.clear_babel_lint_snapshot_metadata();
    let exported: serde_json::Value = serde_json::from_str(
        &workflow
            .export_babel_json()
            .expect("review action without stale lint should export"),
    )
    .expect("export should remain valid JSON");
    assert!(
        exported.pointer("/0/result/data/json/lintErrors").is_none(),
        "invalidated server lint must not be resurrected by the preserved Babel envelope"
    );
}

#[test]
fn workflow_text_edit_exports_fresh_babel_updated_at() {
    let mut workflow =
        StudioWorkflow::from_babel_json(REVIEW_ACTION_JSON).expect("review action should import");
    let original_updated_at = workflow
        .project()
        .segment("6219876d-eba7-529f-b6c9-d949dbabfd12")
        .and_then(|segment| segment.metadata.get("updatedAt"))
        .and_then(serde_json::Value::as_str)
        .expect("fixture should carry imported updatedAt")
        .to_string();

    workflow
        .apply_text_edit(
            "6219876d-eba7-529f-b6c9-d949dbabfd12",
            "Edited through workflow.",
        )
        .expect("text edit should apply");

    let exported = workflow
        .export_babel_json()
        .expect("review action should export");
    let value: serde_json::Value =
        serde_json::from_str(&exported).expect("export should be valid JSON");
    let edited = value
        .pointer("/0/result/data/json/annotations")
        .and_then(serde_json::Value::as_array)
        .and_then(|annotations| {
            annotations.iter().find(|annotation| {
                annotation.get("id")
                    == Some(&serde_json::json!("6219876d-eba7-529f-b6c9-d949dbabfd12"))
            })
        })
        .expect("edited annotation should export");
    let updated_at = edited
        .get("updatedAt")
        .and_then(serde_json::Value::as_str)
        .expect("updatedAt should export");

    assert_ne!(
        updated_at, original_updated_at,
        "Babel text edits stamp updatedAt instead of preserving the imported value"
    );
    assert_eq!(updated_at.len(), "2026-06-21T12:34:56.000Z".len());
    assert!(updated_at.ends_with('Z'));
    assert_eq!(&updated_at[10..11], "T");
    assert_eq!(&updated_at[19..20], ".");
}

#[test]
fn workflow_marks_review_action_export_as_pass() {
    let mut workflow =
        StudioWorkflow::from_babel_json(REVIEW_ACTION_JSON).expect("review action should import");

    workflow.mark_review_passed();

    let exported = workflow
        .export_babel_json()
        .expect("review action should export");
    let value: serde_json::Value =
        serde_json::from_str(&exported).expect("export should be valid JSON");
    assert_eq!(
        value.pointer("/0/result/data/json/actionDecision"),
        Some(&serde_json::json!("pass"))
    );
}

#[test]
fn workflow_annotation_metadata_edit_exports_babel_timestamp_and_resolution() {
    let mut workflow =
        StudioWorkflow::from_babel_json(REVIEW_ACTION_JSON).expect("review action should import");

    workflow
        .update_annotation_metadata(
            "6219876d-eba7-529f-b6c9-d949dbabfd12",
            "background-noise / chair scrape",
            Some(4),
            serde_json::json!({
                "primaryLabel": "background-noise",
                "secondaryLabel": "chair scrape"
            }),
        )
        .expect("annotation metadata edit should apply");

    let exported = workflow
        .export_babel_json()
        .expect("review action should export");
    let value: serde_json::Value =
        serde_json::from_str(&exported).expect("export should be valid JSON");
    let edited = value
        .pointer("/0/result/data/json/annotations")
        .and_then(serde_json::Value::as_array)
        .and_then(|annotations| {
            annotations.iter().find(|annotation| {
                annotation.get("id")
                    == Some(&serde_json::json!("6219876d-eba7-529f-b6c9-d949dbabfd12"))
            })
        })
        .expect("edited annotation should export");
    let updated_at = edited
        .get("updatedAt")
        .and_then(serde_json::Value::as_str)
        .expect("updatedAt should export");

    assert_eq!(
        edited.pointer("/metadata/lowConfidenceResolved"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(updated_at.len(), "2026-06-21T12:34:56.000Z".len());
    assert!(updated_at.ends_with('Z'));
    assert_eq!(&updated_at[10..11], "T");
    assert_eq!(&updated_at[19..20], ".");
}

#[test]
fn workflow_applies_text_and_timing_edit_as_one_undo_step() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .apply_segment_edit("annotation-1", 1.5, 3.5, "combined edit")
        .expect("combined edit should apply");

    let edited = workflow.project().segment("annotation-1").unwrap();
    assert_eq!(edited.start_seconds, 1.5);
    assert_eq!(edited.end_seconds, 3.5);
    assert_eq!(edited.text, "combined edit");

    workflow.undo().expect("undo should restore combined edit");

    let restored = workflow.project().segment("annotation-1").unwrap();
    assert_eq!(restored.start_seconds, 1.2);
    assert_eq!(restored.end_seconds, 3.4);
    assert_eq!(restored.text, "Здравствуйте.");
}

#[test]
fn workflow_coalesces_immediate_timing_then_text_edit_for_one_undo_step() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .move_segment_boundary("annotation-1", 1.5, 3.5)
        .expect("timing edit should apply");
    workflow
        .apply_text_edit("annotation-1", "combined edit")
        .expect("text edit should apply");

    workflow.undo().expect("undo should restore combined edit");

    let restored = workflow.project().segment("annotation-1").unwrap();
    assert_eq!(restored.start_seconds, 1.2);
    assert_eq!(restored.end_seconds, 3.4);
    assert_eq!(restored.text, "Здравствуйте.");
}

#[test]
fn workflow_coalesces_repeated_boundary_moves_for_one_undo_step() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .move_segment_boundary("annotation-1", 1.4, 3.6)
        .expect("first drag timing edit should apply");
    workflow
        .move_segment_boundary("annotation-1", 1.6, 3.8)
        .expect("second drag timing edit should coalesce");
    workflow
        .move_segment_boundary("annotation-1", 1.8, 4.0)
        .expect("third drag timing edit should coalesce");

    let edited = workflow.project().segment("annotation-1").unwrap();
    assert_eq!(edited.start_seconds, 1.8);
    assert_eq!(edited.end_seconds, 4.0);

    workflow
        .undo()
        .expect("one undo should restore the entire drag gesture");

    let restored = workflow.project().segment("annotation-1").unwrap();
    assert_eq!(restored.start_seconds, 1.2);
    assert_eq!(restored.end_seconds, 3.4);
    assert_eq!(restored.text, "Здравствуйте.");
}

#[test]
fn workflow_reports_undo_redo_availability_for_babel_action_row() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    assert!(!workflow.can_undo());
    assert!(!workflow.can_redo());

    workflow
        .apply_text_edit("annotation-1", "edited")
        .expect("text edit should apply");

    assert!(workflow.can_undo());
    assert!(!workflow.can_redo());

    workflow.undo().expect("undo should apply");

    assert!(!workflow.can_undo());
    assert!(workflow.can_redo());

    workflow.redo().expect("redo should apply");

    assert!(workflow.can_undo());
    assert!(!workflow.can_redo());
}

#[test]
fn workflow_changes_segment_track_and_derives_speaker_from_lane() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .change_segment_track("annotation-1", "track-speaker-2")
        .expect("track change should apply");

    let segment = workflow.project().segment("annotation-1").unwrap();
    assert_eq!(segment.speaker_id, "speaker-2");
    assert_eq!(segment.lane_id, "track-speaker-2");
}

#[test]
fn workflow_applies_timing_split_merge_delete_create_and_speaker_edits() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .move_segment_boundary("annotation-1", 1.0, 3.0)
        .expect("timing edit should apply");
    let split_id = workflow
        .split_segment("annotation-1", 2.0)
        .expect("split should apply");
    workflow
        .apply_text_edit(&split_id, "правая часть")
        .expect("right split text should edit");
    workflow
        .merge_with_next("annotation-1")
        .expect("merge should apply");
    workflow
        .delete_segment("annotation-2")
        .expect("delete should apply");
    let created_id = workflow
        .create_segment_after("annotation-1")
        .expect("create should apply");
    workflow
        .change_segment_speaker(&created_id, "speaker-2")
        .expect("speaker change should apply");

    let project = workflow.project();
    assert_eq!(project.segment("annotation-1").unwrap().start_seconds, 1.0);
    assert_eq!(project.segment("annotation-1").unwrap().end_seconds, 3.0);
    assert!(project.segment("annotation-2").is_none());
    assert_eq!(
        project.segment("annotation-1").unwrap().text,
        "Здравствуйте. правая часть"
    );
    let created = project
        .segment(&created_id)
        .expect("new segment should exist");
    assert_eq!(created.speaker_id, "speaker-2");
    assert_eq!(created.lane_id, "track-speaker-2");
}

#[test]
fn workflow_split_segment_clones_babel_content_on_both_halves() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    let split_id = workflow
        .split_segment("annotation-1", 2.0)
        .expect("split should apply");

    let left = workflow.project().segment("annotation-1").unwrap();
    let right = workflow.project().segment(&split_id).unwrap();
    assert_eq!(left.text, "Здравствуйте.");
    assert_eq!(right.text, "Здравствуйте.");
    assert!((left.end_seconds - 1.95).abs() < f64::EPSILON * 8.0);
    assert!((right.start_seconds - 2.05).abs() < f64::EPSILON * 8.0);
}

#[test]
fn workflow_merges_next_segment_on_same_lane() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .merge_with_next("annotation-1")
        .expect("merge should skip interleaved speakers");

    let project = workflow.project();
    assert!(project.segment("annotation-3").is_none());
    let merged = project
        .segment("annotation-1")
        .expect("left segment should remain");
    assert_eq!(merged.end_seconds, 7.2);
    assert_eq!(merged.text, "Здравствуйте. Уточните номер.");
    assert!(project.segment("annotation-2").is_some());
}

#[test]
fn workflow_merges_previous_segment_on_same_lane() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");

    workflow
        .merge_with_previous("annotation-3")
        .expect("merge should skip interleaved speakers");

    let project = workflow.project();
    assert!(project.segment("annotation-1").is_none());
    let merged = project
        .segment("annotation-3")
        .expect("active segment should remain like recovered Babel merge-above");
    assert_eq!(merged.start_seconds, 1.2);
    assert_eq!(merged.end_seconds, 7.2);
    assert_eq!(merged.text, "Здравствуйте. Уточните номер.");
    assert!(project.segment("annotation-2").is_some());
}

#[test]
fn workflow_merges_nearest_later_segment_on_same_lane_when_segments_are_unsorted() {
    let mut workflow = workflow_with_segments(vec![
        segment("anchor", 1.0, 2.0, "left"),
        segment("far", 5.0, 6.0, "far"),
        segment("near", 3.0, 4.0, "near"),
    ]);

    workflow
        .merge_with_next("anchor")
        .expect("merge should use nearest later segment by time");

    let project = workflow.project();
    assert!(project.segment("near").is_none());
    assert!(project.segment("far").is_some());
    let merged = project.segment("anchor").unwrap();
    assert_eq!(merged.end_seconds, 4.0);
    assert_eq!(merged.text, "left near");
}

#[test]
fn workflow_export_preserves_imported_annotation_id_after_merging_created_left_segment() {
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");
    let created_id = workflow
        .create_segment_after("annotation-1")
        .expect("create should apply");
    workflow
        .merge_with_next(&created_id)
        .expect("merge should apply");

    let exported = workflow.export_babel_json().expect("export should apply");
    let package: BabelTaskPackage =
        serde_json::from_str(&exported).expect("export should remain Babel JSON");

    assert!(
        package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == "annotation-3")
    );
    assert!(
        !package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == created_id)
    );
}

#[test]
fn workflow_create_segment_after_uses_full_babel_gap_before_next_same_lane_segment() {
    let mut workflow = workflow_with_segments(vec![
        segment("anchor", 1.0, 2.0, "left"),
        segment("next", 4.0, 5.0, "next"),
    ]);

    let created_id = workflow
        .create_segment_after("anchor")
        .expect("create should fill the available gap");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 2.0);
    assert_eq!(created.end_seconds, 4.0);
}

#[test]
fn workflow_create_segment_before_uses_full_babel_gap_after_previous_same_lane_segment() {
    let mut workflow = workflow_with_segments(vec![
        segment("previous", 1.0, 2.0, "previous"),
        segment("anchor", 4.0, 5.0, "anchor"),
    ]);

    let created_id = workflow
        .create_segment_before("anchor")
        .expect("create should fill the available gap");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 2.0);
    assert_eq!(created.end_seconds, 4.0);
}

#[test]
fn workflow_create_segment_before_uses_babel_minimum_span_when_previous_touches_anchor() {
    let mut workflow = workflow_with_segments(vec![
        segment("previous", 1.0, 2.0, "previous"),
        segment("anchor", 2.0, 3.0, "anchor"),
    ]);

    let created_id = workflow
        .create_segment_before("anchor")
        .expect("create should match Babel's minimum overlapping span");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 2.0);
    assert_eq!(created.end_seconds, 2.01);
}

#[test]
fn workflow_create_segment_after_uses_babel_minimum_span_when_next_touches_anchor() {
    let mut workflow = workflow_with_segments(vec![
        segment("anchor", 1.0, 2.0, "left"),
        segment("next", 2.0, 3.0, "next"),
    ]);

    let created_id = workflow
        .create_segment_after("anchor")
        .expect("create should match Babel's minimum overlapping span");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 2.0);
    assert_eq!(created.end_seconds, 2.01);
}

#[test]
fn workflow_create_segment_before_uses_babel_zero_boundary_fallback() {
    let mut workflow = workflow_with_segments(vec![
        segment("previous", 0.0, 0.0, "previous"),
        segment("anchor", 1.0, 2.0, "anchor"),
    ]);

    let created_id = workflow
        .create_segment_before("anchor")
        .expect("create should use Babel's falsy zero boundary fallback");

    let created = workflow.project().segment(&created_id).unwrap();
    assert!((created.start_seconds - 0.2).abs() < f64::EPSILON * 8.0);
    assert_eq!(created.end_seconds, 1.0);
}

#[test]
fn workflow_create_segment_at_time_uses_babel_blank_region_duration() {
    let mut workflow = workflow_with_segments(vec![segment("anchor", 1.0, 2.0, "anchor")]);

    let created_id = workflow
        .create_segment_at_time_in_lane("track-1", 12.25)
        .expect("create at time should apply");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.lane_id, "track-1");
    assert_eq!(created.speaker_id, "speaker-1");
    assert_eq!(created.start_seconds, 12.25);
    assert_eq!(created.end_seconds, 13.05);
    assert!(created.text.is_empty());
}

#[test]
fn workflow_create_blank_segment_starts_at_zero_like_babel_dialog_default() {
    let mut workflow = workflow_with_segments(vec![segment("anchor", 1.0, 2.0, "anchor")]);

    let created_id = workflow
        .create_blank_segment_in_lane("track-1")
        .expect("blank create should apply");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 0.0);
    assert_eq!(created.end_seconds, 0.8);
    assert!(created.text.is_empty());
}

#[test]
fn workflow_saves_and_reloads_local_project() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");
    workflow
        .apply_text_edit("annotation-3", "Сохраненный текст.")
        .expect("text edit should apply");

    let source = temp.path().join("source");
    for lane in &workflow.project().lanes {
        let path = source.join(&lane.source_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            include_bytes!(
                "../../../crates/studio-waveform/tests/fixtures/babel_line/tone-8k-mono.wav"
            ),
        )
        .unwrap();
    }
    workflow
        .save_local_project(temp.path(), &source)
        .expect("local project should save");
    let loaded = StudioWorkflow::load_local_project(temp.path()).expect("project should reload");

    assert_eq!(
        loaded.project().segment("annotation-3").unwrap().text,
        "Сохраненный текст."
    );
}

#[test]
fn low_confidence_command_accumulates_words_and_returns_one_revision_bound_edit() {
    use exaudio_babel_domain::workflow::BabelLowConfidenceCommandProvider;
    use exaudio_extension::document::{CommandProvider, CommandRequest, DocumentEdit};
    let mut row = segment("review", 0.0, 2.0, "first second");
    row.metadata.insert(
        "babelLowConfidenceWords".into(),
        serde_json::json!([
            {"index": 0, "finalWord": "first", "start": 0.0, "end": 1.0, "isYellow": true},
            {"index": 1, "finalWord": "second", "start": 1.0, "end": 2.0, "isYellow": true}
        ]),
    );
    let workflow = workflow_with_segments(vec![row]);
    let provider = BabelLowConfidenceCommandProvider::default();
    let run = |index, revision| {
        provider.execute(
            "babel.acknowledge-low-confidence-word",
            &CommandRequest {
                project: workflow.project(),
                revision,
                segment_id: Some("review"),
                text: None,
                selection: None,
                caret_offset: None,
                arguments: &serde_json::json!({"word_index": index}),
            },
        )
    };
    assert!(run(9, 7).is_err());
    let partial = run(0, 7).unwrap();
    assert!(partial.transaction.is_none());
    assert!(partial.status.is_some());
    let complete = run(1, 7).unwrap().transaction.unwrap();
    assert_eq!(complete.expected_revision, 7);
    assert_eq!(complete.edits.len(), 1);
    let DocumentEdit::ReplaceSegment { segment } = &complete.edits[0] else {
        panic!("review must replace the annotation atomically")
    };
    assert_eq!(
        segment.metadata.get("lowConfidenceResolved"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(segment.text, "first second");
    assert!(
        !workflow
            .project()
            .segment("review")
            .unwrap()
            .metadata
            .contains_key("lowConfidenceResolved")
    );

    assert!(run(0, 8).unwrap().transaction.is_none());
    assert!(
        run(1, 9).unwrap().transaction.is_none(),
        "a changed document invalidates partial acknowledgements"
    );
    assert!(run(0, 9).unwrap().transaction.is_some());
}
