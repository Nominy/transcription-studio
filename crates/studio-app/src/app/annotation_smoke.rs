use std::collections::BTreeMap;

use iced::keyboard::Modifiers;
use serde_json::json;
use studio_app::StudioWorkflow;
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_waveform::{WaveformHit, WaveformHitTarget};

use super::create_region::BACKGROUND_NOISE_ANNOTATION_PROJECT_ID;
use super::{CreateRegionDialog, StudioState, TimeField};

const ANNOTATION_MUTATION_SMOKE_ENV: &str = "STUDIO_SMOKE_ANNOTATION_MUTATIONS_ON_START";

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AnnotationMutationSmokeReport {
    pub(super) edit_text: String,
    pub(super) edit_intensity: Option<u8>,
    pub(super) timed_range: (f64, f64),
    pub(super) track_lane_id: String,
    pub(super) split_created_id: Option<String>,
    pub(super) merge_removed: bool,
    pub(super) delete_removed: bool,
    pub(super) created_id: Option<String>,
    pub(super) saved: bool,
    pub(super) segment_count: usize,
}

impl AnnotationMutationSmokeReport {
    pub(super) fn runtime_trace_line(&self) -> String {
        format!(
            "Transcription Studio annotation smoke: edit_text={} edit_intensity={} time={:.3}-{:.3}s track={} split={} merge_removed={} delete_removed={} created={} saved={} segments={}",
            self.edit_text,
            self.edit_intensity
                .map(|value| value.to_string())
                .unwrap_or_else(|| "none".to_string()),
            self.timed_range.0,
            self.timed_range.1,
            self.track_lane_id,
            self.split_created_id.as_deref().unwrap_or("-"),
            self.merge_removed,
            self.delete_removed,
            self.created_id.as_deref().unwrap_or("-"),
            self.saved,
            self.segment_count
        )
    }
}

pub(super) fn annotation_mutation_smoke_enabled() -> bool {
    std::env::var(ANNOTATION_MUTATION_SMOKE_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn run_annotation_mutation_smoke(
    state: &mut StudioState,
) -> AnnotationMutationSmokeReport {
    state.replace_workflow(
        StudioWorkflow::from_project(annotation_mutation_smoke_project()),
        "Loaded annotation mutation smoke project.".to_string(),
    );

    state.request_edit_annotation("ann-edit");
    state.change_edit_annotation_secondary_category("other".to_string());
    state.change_edit_annotation_custom_description("chair scrape".to_string());
    state.change_edit_annotation_intensity("4".to_string());
    state.submit_edit_annotation();

    state.update_segment_time_draft("ann-time-track", TimeField::Start, "00:01.30".to_string());
    state.commit_segment_time_edit("ann-time-track", TimeField::Start, false);
    state.update_segment_time_draft("ann-time-track", TimeField::End, "00:02.30".to_string());
    state.commit_segment_time_edit("ann-time-track", TimeField::End, false);
    state.select_segment_track("ann-time-track", "track-speaker-2");

    state.keyboard_modifiers = Modifiers::COMMAND;
    state.hover_waveform_hit(
        "track-speaker-1".to_string(),
        WaveformHit {
            target: WaveformHitTarget::RegionBody,
            region_id: Some("ann-split".to_string()),
            time_seconds: 3.0,
        },
    );
    state.press_waveform_hit("track-speaker-1");
    state.keyboard_modifiers = Modifiers::empty();

    state.merge_segment_below("ann-merge-a");
    state.delete_segment("ann-delete");

    let project = state
        .project()
        .expect("annotation smoke project should remain loaded")
        .clone();
    state.create_region_dialog = Some(CreateRegionDialog::new_for_project(
        &project,
        "track-speaker-2".to_string(),
        6.4,
        7.0,
    ));
    state.change_create_region_primary_category("background-noise".to_string());
    state.change_create_region_secondary_category("other".to_string());
    state.change_create_region_custom_description("annotation smoke create".to_string());
    state.change_create_region_intensity("2".to_string());
    state.submit_create_region();

    let created_id = state
        .editor
        .as_ref()
        .and_then(|editor| editor.selected_segment_id.clone());
    state.save_progress();

    let project = state
        .project()
        .expect("annotation smoke project should remain loaded");
    let edit_segment = project
        .segment("ann-edit")
        .expect("edited annotation should exist");
    let timed_segment = project
        .segment("ann-time-track")
        .expect("timed annotation should exist");
    let split_created_id = project
        .segment("ann-split__studio_1")
        .map(|segment| segment.id.clone());
    let report = AnnotationMutationSmokeReport {
        edit_text: edit_segment.text.clone(),
        edit_intensity: edit_segment
            .metadata
            .get("intensity")
            .and_then(|value| value.as_u64())
            .and_then(|value| u8::try_from(value).ok()),
        timed_range: (timed_segment.start_seconds, timed_segment.end_seconds),
        track_lane_id: timed_segment.lane_id.clone(),
        split_created_id,
        merge_removed: project.segment("ann-merge-b").is_none(),
        delete_removed: project.segment("ann-delete").is_none(),
        created_id,
        saved: state.editor.as_ref().is_some_and(|editor| {
            !editor.is_dirty && editor.last_saved_at.is_some() && !editor.save_has_error
        }),
        segment_count: project.segments.len(),
    };
    eprintln!("{}", report.runtime_trace_line());
    report
}

fn annotation_mutation_smoke_project() -> Project {
    let mut metadata = BTreeMap::new();
    metadata.insert("project_preset".to_string(), "annotation-smoke".to_string());
    metadata.insert("studio.babel.mode".to_string(), "annotation".to_string());
    Project {
        id: BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string(),
        name: "Annotation Mutation Smoke".to_string(),
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
                id: "track-speaker-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
                source_path: "audio/speaker_1.wav".to_string(),
            },
            AudioLane {
                id: "track-speaker-2".to_string(),
                speaker_id: "speaker-2".to_string(),
                label: "Speaker 2".to_string(),
                source_path: "audio/speaker_2.wav".to_string(),
            },
        ],
        segments: vec![
            annotation_segment("ann-edit", "track-speaker-1", "speaker-1", 0.0, 1.0),
            annotation_segment("ann-time-track", "track-speaker-1", "speaker-1", 1.2, 2.2),
            annotation_segment("ann-split", "track-speaker-1", "speaker-1", 2.4, 4.0),
            annotation_segment("ann-merge-a", "track-speaker-1", "speaker-1", 4.2, 4.8),
            annotation_segment("ann-merge-b", "track-speaker-1", "speaker-1", 4.8, 5.4),
            annotation_segment("ann-delete", "track-speaker-2", "speaker-2", 5.6, 6.2),
        ],
        metadata,
    }
}

fn annotation_segment(
    id: &str,
    lane_id: &str,
    speaker_id: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> Segment {
    let mut metadata = BTreeMap::new();
    metadata.insert("type".to_string(), json!("background_noise"));
    metadata.insert("intensity".to_string(), json!(3));
    metadata.insert(
        "content".to_string(),
        json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    Segment {
        id: id.to_string(),
        source_annotation_id: Some(id.to_string()),
        speaker_id: speaker_id.to_string(),
        lane_id: lane_id.to_string(),
        start_seconds,
        end_seconds,
        text: "background-noise / rustle".to_string(),
        metadata,
    }
}
