#[path = "../../../crates/studio-editor/tests/edit_support/mod.rs"]
mod edit_support;
use edit_support::*;
use exaudio_babel_domain::edit::BabelAnnotationDialogs;
use exaudio_babel_domain::edit::annotation::EditAnnotationError;
use exaudio_babel_domain::edit::create_region::{
    BACKGROUND_NOISE_ANNOTATION_PROJECT_ID, CreateRegionDialog, CreateRegionError,
    CreateRegionMode, secondary_category_options,
};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use serde_json::json;
use studio_core::Project;
use studio_editor::edit::{EditorDoc, Effect};
use studio_editor::workflow::StudioWorkflow;

fn babel_environment(project: &Project) -> exaudio_extension::document::DocumentEnvironment {
    let mut environment =
        exaudio_babel_domain::document_environment(project, "test", "exaudio.babel")
            .expect("Babel fixture mode must be supported");
    environment.providers = exaudio_babel_domain::domain_providers(project);
    exaudio_babel_lint::register(&mut environment.providers);
    exaudio_babel_tags::register(&mut environment.providers);
    environment
}

fn babel_doc(project: Project) -> EditorDoc {
    let environment = babel_environment(&project);
    EditorDoc::new(StudioWorkflow::from_babel_project(project), environment)
}
fn selected(doc: &EditorDoc) -> Option<String> {
    doc.session().selected_segment_id.clone()
}
fn segment_count(doc: &EditorDoc) -> usize {
    doc.project().segments.len()
}

/// Single-lane annotation project (`studio.babel.mode = annotation`).
fn annotation_mode_project() -> Project {
    let mut annotation_project = project(vec![seg("annotation-1", "lane-1", 0.0, 1.0, "")]);
    annotation_project
        .metadata
        .insert("studio.babel.mode".to_string(), "annotation".to_string());
    annotation_project
}

fn hierarchical_annotation_doc() -> EditorDoc {
    let mut annotation_project = annotation_mode_project();
    annotation_project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    babel_doc(annotation_project)
}

/// Stable sample project as a hierarchical background-noise task whose `annotation-2` is a
/// `background-noise / {secondary_label}` annotation with intensity 3.
fn annotation_edit_doc(secondary_label: &str) -> EditorDoc {
    let mut stable_project = stable_sample_doc().project().clone();
    stable_project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = stable_project
        .segment_mut("annotation-2")
        .expect("annotation-2 exists");
    segment.text = format!("background-noise / {secondary_label}");
    segment
        .metadata
        .insert("type".to_string(), json!("background_noise"));
    segment.metadata.insert("intensity".to_string(), json!(3));
    segment.metadata.insert(
        "content".to_string(),
        json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": secondary_label,
        }),
    );
    babel_doc(stable_project)
}

#[test]
fn create_region_annotation_enablement_follows_category_rules() {
    let mut dialog = CreateRegionDialog::new_annotation("lane-1".to_string(), 12.0, 12.8);
    assert_eq!(dialog.mode, CreateRegionMode::Annotation);
    assert!(!dialog.is_create_enabled());

    dialog.intensity = Some(3);
    dialog.primary_selection = "background-noise".to_string();
    assert!(!dialog.is_create_enabled());

    dialog.secondary_selection = "rustle".to_string();
    assert!(dialog.is_create_enabled());

    dialog.secondary_selection = "other".to_string();
    assert!(!dialog.is_create_enabled());

    dialog.custom_description = "chair scrape".to_string();
    assert!(dialog.is_create_enabled());
}

#[test]
fn create_region_annotation_requires_explicit_secondary_choice() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut dialog = CreateRegionDialog::new_annotation("lane-1".to_string(), 12.0, 12.8);
    dialog.intensity = Some(3);
    dialog.primary_selection = "background-speech".to_string();
    assert_eq!(
        secondary_category_options("background-speech"),
        vec!["background speech", "other"]
    );
    assert!(!dialog.is_create_enabled());

    let mut doc = hierarchical_annotation_doc();
    let original_count = segment_count(&doc);
    dialogs.open_create_region(&doc, "lane-1", 12.0, 12.8);
    let dialog = dialogs
        .create_region_mut()
        .expect("annotation dialog opens");
    dialog.set_primary_category("background-speech".to_string());
    dialog.set_intensity_label("3");

    let err = dialogs.submit_create_region(&mut doc).unwrap_err();

    assert_eq!(
        err.to_string(),
        CreateRegionError::IncompleteAnnotation.to_string()
    );
    assert_eq!(segment_count(&doc), original_count);
    assert!(dialogs.create_region().is_some());
    assert!(!doc.session().is_dirty);
}

#[test]
fn annotation_range_opens_dialog_without_creating_segment() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let doc = babel_doc(annotation_mode_project());
    let original_count = segment_count(&doc);

    dialogs.open_create_region(&doc, "lane-1", 12.0, 12.8);

    let dialog = dialogs.create_region().expect("annotation dialog opens");
    assert_eq!(dialog.mode, CreateRegionMode::Annotation);
    assert!(dialog.created_segment_id.is_none());
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(segment_count(&doc), original_count);
    assert!(!doc.session().is_dirty);
}

#[test]
fn create_region_secondary_options_end_with_other() {
    assert_eq!(
        secondary_category_options("background-noise")
            .last()
            .copied(),
        Some("other")
    );
}

#[test]
fn create_region_secondary_changes_preserve_description_except_other() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let doc = hierarchical_annotation_doc();
    dialogs.open_create_region(&doc, "lane-1", 12.0, 12.8);
    {
        let dialog = dialogs.create_region_mut().unwrap();
        dialog.set_primary_category("background-noise".to_string());
        dialog.set_secondary_category("other".to_string());
        dialog.set_custom_description("custom wind".to_string());
    }

    dialogs
        .create_region_mut()
        .unwrap()
        .set_secondary_category("wind".to_string());

    let dialog = dialogs.create_region().unwrap();
    assert_eq!(dialog.secondary_selection, "wind");
    assert_eq!(dialog.custom_description, "custom wind");

    dialogs
        .create_region_mut()
        .unwrap()
        .set_secondary_category("other".to_string());

    let dialog = dialogs.create_region().unwrap();
    assert_eq!(dialog.secondary_selection, "other");
    assert_eq!(dialog.custom_description, "");
}

#[test]
fn create_region_intensity_change_accepts_numeric_zero_and_clears_on_empty() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let doc = hierarchical_annotation_doc();
    dialogs.open_create_region(&doc, "lane-1", 12.0, 12.8);

    dialogs
        .create_region_mut()
        .unwrap()
        .set_intensity_label("0");
    assert_eq!(dialogs.create_region().unwrap().intensity, Some(0));

    dialogs.create_region_mut().unwrap().set_intensity_label("");
    assert_eq!(dialogs.create_region().unwrap().intensity, None);
}

#[test]
fn create_region_annotation_category_rules_are_project_id_gated() {
    let mut fallback_project = annotation_mode_project();
    fallback_project.id = "ordinary-project".to_string();
    let mut fallback =
        CreateRegionDialog::new_for_project(&fallback_project, "lane-1".to_string(), 12.0, 12.8);

    assert_eq!(fallback.mode, CreateRegionMode::Annotation);
    assert!(!fallback.uses_hierarchical_labels);
    fallback.intensity = Some(2);
    assert!(fallback.is_create_enabled());

    let mut background_noise_project = fallback_project;
    background_noise_project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let mut hierarchical = CreateRegionDialog::new_for_project(
        &background_noise_project,
        "lane-1".to_string(),
        12.0,
        12.8,
    );

    assert!(hierarchical.uses_hierarchical_labels);
    hierarchical.intensity = Some(2);
    hierarchical.primary_selection = "background-noise".to_string();
    assert!(!hierarchical.is_create_enabled());
    hierarchical.secondary_selection = "rustle".to_string();
    assert!(hierarchical.is_create_enabled());
}

#[test]
fn submitting_annotation_create_region_creates_annotation_metadata() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut stable_project = stable_sample_doc().project().clone();
    stable_project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    stable_project.segments[0]
        .metadata
        .insert("type".to_string(), json!("background_noise"));
    let lane_id = stable_project.segments[0].lane_id.clone();
    let mut doc = babel_doc(stable_project);
    let original_count = segment_count(&doc);

    dialogs.open_create_region(&doc, &lane_id, 12.0, 12.8);
    let dialog = dialogs.create_region_mut().unwrap();
    assert_eq!(dialog.mode, CreateRegionMode::Annotation);
    dialog.set_primary_category("background-noise".to_string());
    dialog.set_secondary_category("rustle".to_string());
    dialog.set_intensity_label("3");

    let _ = ok(dialogs.submit_create_region(&mut doc));

    assert_eq!(segment_count(&doc), original_count + 1);
    let created_id = selected(&doc).expect("created annotation is selected");
    let created = doc.project().segment(&created_id).unwrap();
    assert_eq!(created.lane_id, lane_id);
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    assert_eq!(
        created.metadata.get("type"),
        Some(&json!("background_noise"))
    );
    assert_eq!(created.metadata.get("intensity"), Some(&json!(3)));
    assert_eq!(
        created.metadata.get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
    assert!(dialogs.create_region().is_none());
    assert!(doc.session().is_dirty);
}

// ---- annotation type ------------------------------------------------------------------

#[test]
fn annotation_type_change_updates_segment_metadata_and_dirties() {
    let mut doc = babel_doc(stable_sample_doc().project().clone());

    doc.edit_workflow("Annotation type", |workflow| {
        workflow.set_segment_annotation_type("annotation-2", "non-transcription")
    })
    .unwrap();

    let segment = doc.project().segment("annotation-2").unwrap();
    assert_eq!(
        segment.metadata.get("type"),
        Some(&json!("non-transcription"))
    );
    assert!(doc.session().is_dirty);
}

// ---- edit-annotation dialog -----------------------------------------------------------

#[test]
fn edit_annotation_dialog_hydrates_metadata() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut doc = annotation_edit_doc("rustle");

    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-2"));

    let dialog = dialogs.edit_annotation().expect("edit annotation opens");
    assert_eq!(dialog.segment_id, "annotation-2");
    assert!(!dialog.is_new(), "a labelled segment is edited, not added");
    assert_eq!(dialog.primary_selection, "background-noise");
    assert_eq!(dialog.secondary_selection, "rustle");
    assert_eq!(dialog.custom_description, "");
    assert_eq!(dialog.intensity, Some(3));
    assert!(dialog.error.is_none());
    assert!(dialog.is_save_enabled());
    assert_eq!(selected(&doc).as_deref(), Some("annotation-2"));
}

#[test]
fn edit_annotation_dialog_hydrates_unknown_secondary_as_other_description() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut doc = annotation_edit_doc("chair scrape");

    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-2"));

    let dialog = dialogs.edit_annotation().unwrap();
    assert_eq!(dialog.primary_selection, "background-noise");
    assert_eq!(dialog.secondary_selection, "other");
    assert_eq!(dialog.custom_description, "chair scrape");
    assert!(dialog.is_save_enabled());
}

#[test]
fn edit_annotation_other_secondary_requires_description() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut doc = annotation_edit_doc("rustle");
    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-2"));
    dialogs
        .edit_annotation_mut()
        .unwrap()
        .set_secondary_selection("other".to_string());

    let err = dialogs.submit_edit_annotation(&mut doc).unwrap_err();

    assert_eq!(
        err.to_string(),
        EditAnnotationError::CategoryRequired.to_string()
    );
    let dialog = dialogs
        .edit_annotation()
        .expect("dialog stays open on error");
    assert_eq!(dialog.error, Some(EditAnnotationError::CategoryRequired));
    assert_eq!(
        doc.project()
            .segment("annotation-2")
            .unwrap()
            .metadata
            .get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
}

#[test]
fn edit_annotation_primary_change_requires_explicit_secondary_choice() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut doc = annotation_edit_doc("rustle");
    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-2"));

    dialogs
        .edit_annotation_mut()
        .unwrap()
        .set_primary_selection("background-speech".to_string());

    let dialog = dialogs.edit_annotation().unwrap();
    assert_eq!(dialog.primary_selection, "background-speech");
    assert_eq!(dialog.secondary_selection, "");
    assert!(!dialog.is_save_enabled());

    let err = dialogs.submit_edit_annotation(&mut doc).unwrap_err();

    assert_eq!(
        err.to_string(),
        EditAnnotationError::CategoryRequired.to_string()
    );
    assert_eq!(
        dialogs.edit_annotation().unwrap().error,
        Some(EditAnnotationError::CategoryRequired)
    );
    assert_eq!(
        doc.project()
            .segment("annotation-2")
            .unwrap()
            .metadata
            .get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
}

#[test]
fn saving_edit_annotation_dialog_updates_metadata_and_text() {
    let mut dialogs = BabelAnnotationDialogs::default();
    let mut doc = annotation_edit_doc("rustle");
    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-2"));
    {
        let dialog = dialogs.edit_annotation_mut().unwrap();
        dialog.set_secondary_selection("other".to_string());
        dialog.set_custom_description("chair scrape".to_string());
        dialog.set_intensity_label("4");
    }

    let outcome = ok(dialogs.submit_edit_annotation(&mut doc));

    assert!(dialogs.edit_annotation().is_none());
    assert!(doc.session().is_dirty);
    let segment = doc.project().segment("annotation-2").unwrap();
    assert_eq!(segment.text, "background-noise / chair scrape");
    assert_eq!(segment.metadata.get("intensity"), Some(&json!(4)));
    assert_eq!(
        segment.metadata.get("content"),
        Some(&json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "chair scrape"
        }))
    );
    assert_eq!(
        draft_text(&doc, "annotation-2"),
        "background-noise / chair scrape"
    );
    assert!(outcome.has_effect(&Effect::DraftChanged {
        segment_id: "annotation-2".to_string(),
    }));
}

#[test]
fn escape_dismisses_annotation_then_create_region_before_native_selection() {
    let mut doc = stable_sample_doc();
    set_overlay(&mut doc, "track-speaker-1", 3.5, 5.1);
    let _ = ok(doc.loop_cut_preview());
    let mut dialogs = BabelAnnotationDialogs::default();
    dialogs.open_create_region(&doc, "track-speaker-1", 12.0, 12.8);
    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-1"));
    assert!(dialogs.escape());
    assert!(dialogs.edit_annotation().is_none());
    assert!(dialogs.create_region().is_some());
    assert!(doc.session().loop_range().is_some());
    assert!(dialogs.escape());
    assert!(dialogs.create_region().is_none());
    assert!(doc.session().loop_range().is_some());
    assert!(!dialogs.escape());
    let _ = doc.escape();
    assert!(doc.session().loop_range().is_none());
}

#[test]
fn babel_document_split_preserves_domain_gap_and_given_time() {
    let mut doc = babel_doc(stable_sample_doc().project().clone());
    let _ = doc.select_segment("annotation-1");
    doc.session_mut().playhead_seconds = 2.75;
    let _ = ok(doc.split_segment_at("annotation-1", 2.0));
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().end_seconds,
        1.95
    );
    assert_eq!(
        doc.project()
            .segment("annotation-1__studio_1")
            .unwrap()
            .start_seconds,
        2.05
    );
}

#[test]
fn annotation_drag_dispatches_domain_dialog_without_creating_native_row() {
    let project = annotation_mode_project();
    let environment = babel_environment(&project);
    let mut doc = EditorDoc::new(StudioWorkflow::from_babel_project(project), environment);
    doc.session_mut().viewport = studio_editor::timeline_model::TimeRange {
        start_seconds: 11.0,
        end_seconds: 13.8,
    };
    let count = segment_count(&doc);
    doc.hover_waveform(
        "lane-1",
        hit(studio_waveform::WaveformHitTarget::Waveform, None, 12.0),
    );
    let _ = ok(doc.press_waveform("lane-1"));
    doc.hover_waveform(
        "lane-1",
        hit(studio_waveform::WaveformHitTarget::Waveform, None, 12.8),
    );
    let outcome = ok(doc.release_waveform("lane-1"));
    let arguments = outcome
        .effects
        .iter()
        .find_map(|effect| {
            if let Effect::ExecuteCommand { command, arguments } = effect {
                assert_eq!(command, "exaudio.babel.open-create-region");
                Some(arguments)
            } else {
                None
            }
        })
        .expect("annotation range must dispatch to its domain command");
    let mut dialogs = BabelAnnotationDialogs::default();
    let _ = ok(dialogs.open_create_region_command(&doc, arguments));
    let dialog = dialogs.create_region().unwrap();
    assert_eq!(dialog.mode, CreateRegionMode::Annotation);
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert!(dialog.created_segment_id.is_none());
    assert_eq!(segment_count(&doc), count);
    assert!(!doc.session().is_dirty);
}

#[test]
fn annotation_save_preserves_pending_text_and_has_one_project_undo_unit() {
    let mut doc = annotation_edit_doc("rustle");
    let original = doc.project().clone();
    let mut dialogs = BabelAnnotationDialogs::default();
    let _ = ok(dialogs.open_edit_annotation(&mut doc, "annotation-2"));
    let _ = ok(doc.edit_text(
        "annotation-2",
        "pending user draft".into(),
        Default::default(),
    ));
    dialogs
        .edit_annotation_mut()
        .unwrap()
        .set_intensity_label("4");
    let _ = ok(dialogs.submit_edit_annotation(&mut doc));
    assert_eq!(draft_text(&doc, "annotation-2"), "pending user draft");
    assert_eq!(
        doc.project()
            .segment("annotation-2")
            .unwrap()
            .metadata
            .get("intensity"),
        Some(&json!(4))
    );
    doc.with_workflow(|workflow| workflow.undo()).unwrap();
    assert_eq!(doc.project(), &original);
    assert!(!doc.workflow().can_undo());
}
