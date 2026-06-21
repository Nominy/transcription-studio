use studio_app::StudioWorkflow;
use studio_babel::BabelTaskPackage;
use studio_core::{AudioLane, Project, Segment, Speaker};

const SAMPLE_TASK_JSON: &str = include_str!("../assets/sample-task.json");

fn workflow_with_segments(segments: Vec<Segment>) -> StudioWorkflow {
    StudioWorkflow::from_project(Project {
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
fn workflow_create_segment_after_uses_shorter_positive_gap_before_next_same_lane_segment() {
    let mut workflow = workflow_with_segments(vec![
        segment("anchor", 1.0, 2.0, "left"),
        segment("next", 2.4, 3.0, "next"),
    ]);

    let created_id = workflow
        .create_segment_after("anchor")
        .expect("create should fit in the available gap");

    let created = workflow.project().segment(&created_id).unwrap();
    assert_eq!(created.start_seconds, 2.0);
    assert_eq!(created.end_seconds, 2.4);
}

#[test]
fn workflow_create_segment_after_rejects_when_next_same_lane_segment_leaves_no_gap() {
    let mut workflow = workflow_with_segments(vec![
        segment("anchor", 1.0, 2.0, "left"),
        segment("next", 2.0, 3.0, "next"),
    ]);

    let result = workflow.create_segment_after("anchor");

    assert!(result.is_err());
    assert_eq!(workflow.project().segments.len(), 2);
}

#[test]
fn workflow_saves_and_reloads_local_project() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let mut workflow =
        StudioWorkflow::from_babel_json(SAMPLE_TASK_JSON).expect("sample task should import");
    workflow
        .apply_text_edit("annotation-3", "Сохраненный текст.")
        .expect("text edit should apply");

    workflow
        .save_local_project(temp.path())
        .expect("local project should save");
    let loaded = StudioWorkflow::load_local_project(temp.path()).expect("project should reload");

    assert_eq!(
        loaded.project().segment("annotation-3").unwrap().text,
        "Сохраненный текст."
    );
}
