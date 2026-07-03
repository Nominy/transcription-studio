use std::{collections::BTreeMap, path::PathBuf};

use iced::Task;
use serde_json::json;
use studio_app::{StudioWorkflow, TranscriptRowKind, TranscriptWordDiffStatus};
use studio_core::{AudioLane, Project, Segment, Speaker};

use super::actions::update;
use super::renderer_capture::renderer_screenshot_task;
use super::{Message, StudioState};

const DIFF_WORKBENCH_SMOKE_ENV: &str = "STUDIO_SMOKE_DIFF_WORKBENCH_ON_START";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DiffWorkbenchSmokeMode {
    Transcription,
    Annotation,
    Both,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiffWorkbenchSmokeReport {
    pub(super) mode: String,
    pub(super) diff_checked: bool,
    pub(super) selected_compare_action_id: Option<String>,
    pub(super) reviewers_checked: Option<bool>,
    pub(super) editor_dirty: bool,
    pub(super) row_count: usize,
    pub(super) changed_marker: Option<String>,
    pub(super) changed_badge: Option<String>,
    pub(super) changed_tokens: Vec<String>,
    pub(super) modified_deleted_visible: bool,
    pub(super) annotation_ghost_visible: bool,
    pub(super) annotation_status: Option<String>,
    pub(super) avg_shift_value: Option<String>,
    pub(super) word_diff_value: Option<String>,
    pub(super) timestamp_tooltip_title: Option<String>,
    pub(super) timestamp_tolerance_labels: Vec<String>,
    pub(super) event_metric_value: Option<String>,
    pub(super) label_metric_value: Option<String>,
    pub(super) intensity_metric_value: Option<String>,
    pub(super) annotation_metric_tooltip_titles: Vec<String>,
}

impl DiffWorkbenchSmokeReport {
    pub(super) fn runtime_trace_line(&self) -> String {
        format!(
            "Transcription Studio diff smoke: mode={} checked={} selected={} reviewers={} dirty={} rows={} changed={} badge={} tokens={} modified_deleted={} annotation_ghost={} annotation_status={} avg_shift={} word_diff={} timestamp_tooltip={} bars={} annotation_metrics={}/{}/{} annotation_tooltips={}",
            self.mode,
            self.diff_checked,
            self.selected_compare_action_id.as_deref().unwrap_or("-"),
            self.reviewers_checked
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_string()),
            self.editor_dirty,
            self.row_count,
            self.changed_marker.as_deref().unwrap_or("-"),
            self.changed_badge.as_deref().unwrap_or("-"),
            joined_or_dash(&self.changed_tokens),
            self.modified_deleted_visible,
            self.annotation_ghost_visible,
            self.annotation_status.as_deref().unwrap_or("-"),
            self.avg_shift_value.as_deref().unwrap_or("-"),
            self.word_diff_value.as_deref().unwrap_or("-"),
            self.timestamp_tooltip_title.as_deref().unwrap_or("-"),
            joined_or_dash(&self.timestamp_tolerance_labels),
            self.event_metric_value.as_deref().unwrap_or("-"),
            self.label_metric_value.as_deref().unwrap_or("-"),
            self.intensity_metric_value.as_deref().unwrap_or("-"),
            joined_or_dash(&self.annotation_metric_tooltip_titles)
        )
    }
}

pub(super) fn diff_workbench_smoke_mode() -> Option<DiffWorkbenchSmokeMode> {
    let raw = std::env::var(DIFF_WORKBENCH_SMOKE_ENV).ok()?;
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "1" | "true" | "both" => Some(DiffWorkbenchSmokeMode::Both),
        "transcription" | "text" => Some(DiffWorkbenchSmokeMode::Transcription),
        "annotation" | "annotations" => Some(DiffWorkbenchSmokeMode::Annotation),
        "0" | "false" | "off" => None,
        _ => Some(DiffWorkbenchSmokeMode::Both),
    }
}

pub(super) fn run_diff_workbench_smoke(
    state: &mut StudioState,
    mode: DiffWorkbenchSmokeMode,
) -> Vec<DiffWorkbenchSmokeReport> {
    let modes = match mode {
        DiffWorkbenchSmokeMode::Transcription => vec![DiffWorkbenchSmokeMode::Transcription],
        DiffWorkbenchSmokeMode::Annotation => vec![DiffWorkbenchSmokeMode::Annotation],
        DiffWorkbenchSmokeMode::Both => vec![
            DiffWorkbenchSmokeMode::Transcription,
            DiffWorkbenchSmokeMode::Annotation,
        ],
    };

    modes
        .into_iter()
        .map(|mode| run_single_diff_workbench_smoke(state, mode))
        .collect()
}

pub(super) fn diff_workbench_renderer_screenshot_task(
    mode: DiffWorkbenchSmokeMode,
) -> Task<Message> {
    renderer_screenshot_task(
        PathBuf::from("target")
            .join("runtime-diff-smoke")
            .join(format!("{}-renderer.bmp", mode.screenshot_stem())),
    )
}

impl DiffWorkbenchSmokeMode {
    fn screenshot_stem(self) -> &'static str {
        match self {
            DiffWorkbenchSmokeMode::Transcription => "transcription",
            DiffWorkbenchSmokeMode::Annotation | DiffWorkbenchSmokeMode::Both => "annotation",
        }
    }
}

fn run_single_diff_workbench_smoke(
    state: &mut StudioState,
    mode: DiffWorkbenchSmokeMode,
) -> DiffWorkbenchSmokeReport {
    match mode {
        DiffWorkbenchSmokeMode::Transcription => {
            state.replace_workflow(
                StudioWorkflow::from_project(transcription_diff_smoke_project()),
                "Loaded transcription diff smoke project.".to_string(),
            );
            let _ = update(state, Message::ToggleDiffView);
            let _ = update(state, Message::ToggleDiffReviewers);
            let _ = update(state, Message::DiffCompareActionChanged("l1".to_string()));
            let report = capture_diff_workbench_report(state, "transcription");
            eprintln!("{}", report.runtime_trace_line());
            report
        }
        DiffWorkbenchSmokeMode::Annotation => {
            state.replace_workflow(
                StudioWorkflow::from_project(annotation_diff_smoke_project()),
                "Loaded annotation diff smoke project.".to_string(),
            );
            let _ = update(state, Message::ToggleDiffView);
            let _ = update(
                state,
                Message::DiffCompareActionChanged("ann-l1".to_string()),
            );
            let report = capture_diff_workbench_report(state, "annotation");
            eprintln!("{}", report.runtime_trace_line());
            report
        }
        DiffWorkbenchSmokeMode::Both => unreachable!("expanded before dispatch"),
    }
}

fn capture_diff_workbench_report(state: &StudioState, mode: &str) -> DiffWorkbenchSmokeReport {
    let project = state
        .project()
        .expect("diff smoke project should remain loaded");
    let editor = state
        .editor
        .as_ref()
        .expect("diff smoke editor should remain loaded");
    let toolbar = editor.diff_toolbar(project);
    let rows = editor.transcript_rows(project);
    let changed_row = rows.iter().find(|row| row.segment_id == "diff-changed");
    let changed_diff = changed_row.and_then(|row| row.diff.as_ref());
    let annotation_row = rows.iter().find(|row| row.segment_id == "diff-annotation");
    let timestamp_tooltip = toolbar.metrics.timestamp_tooltip.as_ref();
    let event_tooltip = toolbar
        .metrics
        .event
        .as_ref()
        .and_then(|metric| metric.tooltip.as_ref());
    let label_tooltip = toolbar
        .metrics
        .label_accuracy
        .as_ref()
        .and_then(|metric| metric.tooltip.as_ref());
    let intensity_tooltip = toolbar
        .metrics
        .intensity
        .as_ref()
        .and_then(|metric| metric.tooltip.as_ref());

    DiffWorkbenchSmokeReport {
        mode: mode.to_string(),
        diff_checked: toolbar.diff_toggle.checked,
        selected_compare_action_id: toolbar.compare_select.value,
        reviewers_checked: toolbar
            .show_reviewers_control
            .then_some(toolbar.reviewers_checked),
        editor_dirty: editor.is_dirty,
        row_count: rows.len(),
        changed_marker: changed_row.and_then(|row| row.diff_marker.clone()),
        changed_badge: changed_diff.map(|diff| diff.badge_label.clone()),
        changed_tokens: changed_diff
            .map(|diff| {
                diff.word_diff
                    .tokens
                    .iter()
                    .map(|token| {
                        format!(
                            "{}:{}",
                            word_diff_status_label(token.status),
                            token.value.trim()
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        modified_deleted_visible: rows
            .iter()
            .any(|row| row.row_kind == TranscriptRowKind::ModifiedDeleted),
        annotation_ghost_visible: rows
            .iter()
            .any(|row| row.row_kind == TranscriptRowKind::AnnotationRemovedGhost),
        annotation_status: annotation_row
            .and_then(|row| row.annotation_diff.as_ref())
            .map(|diff| diff.status.clone()),
        avg_shift_value: toolbar
            .metrics
            .avg_shift
            .as_ref()
            .map(|metric| metric.value.clone()),
        word_diff_value: toolbar
            .metrics
            .word_diff
            .as_ref()
            .map(|metric| metric.value.clone()),
        timestamp_tooltip_title: timestamp_tooltip.map(|tooltip| tooltip.title.clone()),
        timestamp_tolerance_labels: timestamp_tooltip
            .map(|tooltip| {
                tooltip
                    .tolerance_bars
                    .iter()
                    .map(|bar| normalize_tolerance_label(&bar.label))
                    .collect()
            })
            .unwrap_or_default(),
        event_metric_value: toolbar
            .metrics
            .event
            .as_ref()
            .map(|metric| metric.value.clone()),
        label_metric_value: toolbar
            .metrics
            .label_accuracy
            .as_ref()
            .map(|metric| metric.value.clone()),
        intensity_metric_value: toolbar
            .metrics
            .intensity
            .as_ref()
            .map(|metric| metric.value.clone()),
        annotation_metric_tooltip_titles: [event_tooltip, label_tooltip, intensity_tooltip]
            .into_iter()
            .flatten()
            .map(|tooltip| tooltip.title.clone())
            .collect(),
    }
}

fn transcription_diff_smoke_project() -> Project {
    let mut metadata = diff_common_metadata();
    metadata.insert("project_preset".to_string(), "diff-smoke".to_string());
    metadata.insert(
        "studio.babel.diff.availableReviewActions".to_string(),
        json!([
            { "id": "l0", "label": "L0", "level": 0, "workerName": "Reference" },
            { "id": "l1", "label": "L1", "level": 1, "workerName": "Current" }
        ])
        .to_string(),
    );
    metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    metadata.insert(
        "studio.babel.diff.hasReviewerToggle".to_string(),
        "true".to_string(),
    );
    metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    metadata.insert(
        "studio.babel.diff.metrics".to_string(),
        json!({
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

    Project {
        id: "diff-transcription-smoke".to_string(),
        name: "Diff Transcription Smoke".to_string(),
        speakers: diff_speakers(),
        lanes: diff_lanes(),
        segments: vec![
            changed_transcription_segment(),
            current_segment(
                "current-after",
                "track-1",
                "speaker-1",
                5.0,
                5.8,
                "current after",
            ),
            new_transcription_segment(),
            annotation_diff_segment("l1"),
        ],
        metadata,
    }
}

fn annotation_diff_smoke_project() -> Project {
    let mut metadata = diff_common_metadata();
    metadata.insert(
        "project_preset".to_string(),
        "annotation-diff-smoke".to_string(),
    );
    metadata.insert(
        "studio.babel.diff.annotationToolbar".to_string(),
        "true".to_string(),
    );
    metadata.insert(
        "studio.babel.diff.availableReviewActions".to_string(),
        json!([
            { "id": "ann-l0", "label": "L0", "level": 0, "workerName": "Ira" },
            { "id": "ann-l1", "label": "L1", "level": 1, "workerName": "Noor" }
        ])
        .to_string(),
    );
    metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "ann-l0".to_string(),
    );
    metadata.insert(
        "studio.babel.diff.annotationMetrics".to_string(),
        json!({
            "eventDetection": { "f1": 0.91, "precision": 0.82, "recall": 0.73 },
            "labelMacroAverage": { "f1": 0.64, "precision": 0.55, "recall": 0.46 },
            "intensityCalibration": { "f1": 0.37, "precision": 0.28, "recall": 0.19 }
        })
        .to_string(),
    );

    Project {
        id: "diff-annotation-smoke".to_string(),
        name: "Diff Annotation Smoke".to_string(),
        speakers: diff_speakers(),
        lanes: diff_lanes(),
        segments: vec![
            annotation_diff_segment("ann-l1"),
            current_segment(
                "annotation-current",
                "track-2",
                "speaker-2",
                4.0,
                4.8,
                "background-noise / knock",
            ),
        ],
        metadata,
    }
}

fn diff_common_metadata() -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "studio.babel.diff.result".to_string(),
        json!({
            "speakerDiffs": [{
                "segmentMappings": [{
                    "relationship": "deleted",
                    "segmentsA": [{
                        "annotationId": "deleted-reference",
                        "startTimeInSeconds": 2.4,
                        "endTimeInSeconds": 3.2,
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
    metadata.insert(
        "studio.babel.diff.annotationGhostRows".to_string(),
        json!([{
            "comparisonLabel": "Reference",
            "annotation": {
                "id": "removed-annotation",
                "content": {
                    "primaryLabel": "background-noise",
                    "secondaryLabel": "rustle"
                },
                "intensity": 3,
                "startTimeInSeconds": 3.3,
                "endTimeInSeconds": 3.8
            }
        }])
        .to_string(),
    );
    metadata
}

fn changed_transcription_segment() -> Segment {
    let mut segment = current_segment(
        "diff-changed",
        "track-1",
        "speaker-1",
        1.0,
        2.0,
        "edited text",
    );
    segment.metadata.insert(
        "studio.babel.diff.compareSnapshots".to_string(),
        json!({
            "l0": {
                "text": "first compare text",
                "startSeconds": 0.8,
                "endSeconds": 1.8,
                "speakerId": "speaker-2",
                "laneId": "track-2",
                "wordDiffs": [
                    { "status": "removed", "value": "first" },
                    { "status": "added", "value": "edited" },
                    { "status": "unchanged", "value": "text" }
                ]
            },
            "l1": {
                "text": "selected compare text",
                "startSeconds": 0.9,
                "endSeconds": 1.9,
                "speakerId": "speaker-2",
                "laneId": "track-2",
                "wordDiffs": [
                    { "status": "removed", "value": "selected" },
                    { "status": "added", "value": "edited" },
                    { "status": "unchanged", "value": "text" }
                ]
            }
        }),
    );
    segment
}

fn new_transcription_segment() -> Segment {
    Segment {
        source_annotation_id: None,
        ..current_segment(
            "diff-new",
            "track-2",
            "speaker-2",
            6.0,
            6.8,
            "fresh current line",
        )
    }
}

fn annotation_diff_segment(compare_action_id: &str) -> Segment {
    let mut metadata = BTreeMap::new();
    let mut snapshots = serde_json::Map::new();
    snapshots.insert(
        compare_action_id.to_string(),
        json!({
            "content": {
                "primaryLabel": "background-noise",
                "secondaryLabel": "mic bump"
            },
            "intensity": 2,
            "startTimeInSeconds": 7.0,
            "endTimeInSeconds": 7.8
        }),
    );
    metadata.insert("type".to_string(), json!("background_noise"));
    metadata.insert("intensity".to_string(), json!(4));
    metadata.insert(
        "content".to_string(),
        json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    metadata.insert(
        "studio.babel.diff.compareSnapshots".to_string(),
        serde_json::Value::Object(snapshots),
    );

    Segment {
        id: "diff-annotation".to_string(),
        source_annotation_id: Some("diff-annotation".to_string()),
        speaker_id: "speaker-2".to_string(),
        lane_id: "track-2".to_string(),
        start_seconds: 7.0,
        end_seconds: 7.8,
        text: "background-noise / rustle".to_string(),
        metadata,
    }
}

fn current_segment(
    id: &str,
    lane_id: &str,
    speaker_id: &str,
    start_seconds: f64,
    end_seconds: f64,
    text: &str,
) -> Segment {
    Segment {
        id: id.to_string(),
        source_annotation_id: Some(id.to_string()),
        speaker_id: speaker_id.to_string(),
        lane_id: lane_id.to_string(),
        start_seconds,
        end_seconds,
        text: text.to_string(),
        metadata: BTreeMap::new(),
    }
}

fn diff_speakers() -> Vec<Speaker> {
    vec![
        Speaker {
            id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
        },
        Speaker {
            id: "speaker-2".to_string(),
            label: "Speaker 2".to_string(),
        },
    ]
}

fn diff_lanes() -> Vec<AudioLane> {
    vec![
        AudioLane {
            id: "track-1".to_string(),
            speaker_id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
            source_path: "audio/speaker_1.wav".to_string(),
        },
        AudioLane {
            id: "track-2".to_string(),
            speaker_id: "speaker-2".to_string(),
            label: "Speaker 2".to_string(),
            source_path: "audio/speaker_2.wav".to_string(),
        },
    ]
}

fn word_diff_status_label(status: TranscriptWordDiffStatus) -> &'static str {
    match status {
        TranscriptWordDiffStatus::Unchanged => "unchanged",
        TranscriptWordDiffStatus::Added => "added",
        TranscriptWordDiffStatus::Removed => "removed",
        TranscriptWordDiffStatus::Unknown => "unknown",
    }
}

fn normalize_tolerance_label(label: &str) -> String {
    label.replace('\u{00b1}', "+/-")
}

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(",")
    }
}
