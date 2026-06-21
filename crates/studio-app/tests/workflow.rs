use studio_app::StudioWorkflow;
use studio_babel::BabelTaskPackage;

const SAMPLE_TASK_JSON: &str = include_str!("../assets/sample-task.json");

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
