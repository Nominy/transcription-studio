//! Diff-toolbar state: compare-version choices, toggles and diff metrics carried in task metadata.

use serde_json::Value;
use studio_core::Project;

use super::{AssessmentPhase, EditorSessionView};

const META_AVAILABLE_REVIEW_ACTIONS: &str = "studio.babel.diff.availableReviewActions";
const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";
const META_DIFF_METRICS: &str = "studio.babel.diff.metrics";
const META_ANNOTATION_TOOLBAR: &str = "studio.babel.diff.annotationToolbar";
const META_ANNOTATION_METRICS: &str = "studio.babel.diff.annotationMetrics";
const META_SHOW_REVIEWERS: &str = "studio.babel.diff.showReviewers";
const META_HAS_REVIEWER_TOGGLE: &str = "studio.babel.diff.hasReviewerToggle";
const META_ASSESSMENT_PHASE: &str = "studio.babel.diff.assessmentPhase";
const META_IS_LOADING: &str = "studio.babel.diff.isLoading";

#[derive(Clone, Debug, PartialEq)]
pub struct DiffToolbarState {
    /// Annotation (sound-event) task rather than a transcription task.
    pub annotation_mode: bool,
    /// Assessment tasks hide the diff toggle and compare controls.
    pub assessment_phase: Option<AssessmentPhase>,
    pub is_diff_mode: bool,
    pub has_unsaved_changes: bool,
    pub is_loading: bool,
    pub show_reviewers_control: bool,
    pub reviewers_checked: bool,
    pub compare_options: Vec<DiffCompareOption>,
    pub selected_compare_id: Option<String>,
    /// Present only while diff mode is on and the backend supplied metrics.
    pub metrics: Option<DiffMetrics>,
}

impl DiffToolbarState {
    pub fn diff_toggle_visible(&self) -> bool {
        self.assessment_phase.is_none()
    }

    /// The toggle cannot be turned on when there is nothing to compare against.
    pub fn diff_toggle_enabled(&self) -> bool {
        self.is_diff_mode || !self.compare_options.is_empty()
    }

    pub fn compare_select_visible(&self) -> bool {
        self.assessment_phase.is_none() && self.is_diff_mode
    }

    pub fn unsaved_changes_badge_visible(&self) -> bool {
        self.compare_select_visible() && self.has_unsaved_changes
    }

    /// "No previous versions to compare" hint.
    pub fn no_versions_hint_visible(&self) -> bool {
        self.assessment_phase.is_none()
            && !self.is_diff_mode
            && self.compare_options.is_empty()
            && !self.is_loading
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffCompareOption {
    pub id: String,
    /// Version label; annotation tasks append ` · <worker name>`.
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiffMetrics {
    Transcription(TranscriptionDiffMetrics),
    Annotation(AnnotationDiffMetrics),
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptionDiffMetrics {
    pub timestamp: TimestampMetrics,
    /// 0.0..=1.0
    pub overall_word_error_rate: f64,
    pub total_substitutions: i64,
    pub total_insertions: i64,
    pub total_deletions: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimestampMetrics {
    pub timeline: MetricTriplet,
    pub matched_segments: i64,
    pub total_segments: i64,
    /// Percent of matched segments with both edges within ±50/100/200 ms.
    pub within_50_ms: i64,
    pub within_100_ms: i64,
    pub within_200_ms: i64,
    pub avg_shift_ms: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationDiffMetrics {
    pub event_detection: MetricTriplet,
    pub label_macro_average: MetricTriplet,
    pub intensity_calibration: MetricTriplet,
}

/// Fractions in 0.0..=1.0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricTriplet {
    pub f1: f64,
    pub precision: f64,
    pub recall: f64,
}

impl EditorSessionView<'_> {
    pub fn diff_toolbar(&self, project: &Project) -> DiffToolbarState {
        let annotation_mode = metadata_bool(project, META_ANNOTATION_TOOLBAR);
        let assessment_phase = project
            .metadata
            .get(META_ASSESSMENT_PHASE)
            .and_then(|phase| AssessmentPhase::parse(phase));
        let metrics = if !self.diff_view {
            None
        } else if annotation_mode {
            annotation_metrics(project).map(DiffMetrics::Annotation)
        } else {
            transcription_metrics(project).map(DiffMetrics::Transcription)
        };

        DiffToolbarState {
            annotation_mode,
            assessment_phase,
            is_diff_mode: self.diff_view,
            has_unsaved_changes: self.is_dirty,
            is_loading: metadata_bool(project, META_IS_LOADING),
            show_reviewers_control: !annotation_mode
                && assessment_phase.is_none()
                && metadata_bool(project, META_HAS_REVIEWER_TOGGLE),
            reviewers_checked: metadata_bool(project, META_SHOW_REVIEWERS),
            compare_options: compare_options(project, annotation_mode),
            selected_compare_id: project
                .metadata
                .get(META_SELECTED_COMPARE_ACTION_ID)
                .filter(|value| !value.is_empty())
                .cloned(),
            metrics,
        }
    }
}

fn compare_options(project: &Project, include_worker_name: bool) -> Vec<DiffCompareOption> {
    let Some(raw) = project.metadata.get(META_AVAILABLE_REVIEW_ACTIONS) else {
        return Vec::new();
    };
    let Ok(Value::Array(actions)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };

    actions
        .into_iter()
        .filter_map(|action| {
            let id = action.get("id")?.as_str()?.to_string();
            let mut label = action.get("label")?.as_str()?.to_string();
            if include_worker_name
                && let Some(worker_name) = action.get("workerName").and_then(Value::as_str)
                && !worker_name.is_empty()
            {
                label.push_str(&format!(" \u{00b7} {worker_name}"));
            }
            Some(DiffCompareOption { id, label })
        })
        .collect()
}

fn transcription_metrics(project: &Project) -> Option<TranscriptionDiffMetrics> {
    let value: Value = serde_json::from_str(project.metadata.get(META_DIFF_METRICS)?).ok()?;
    let f64_at = |pointer: &str| value.pointer(pointer).and_then(Value::as_f64);
    let i64_at = |pointer: &str| value.pointer(pointer).and_then(Value::as_i64);
    Some(TranscriptionDiffMetrics {
        timestamp: TimestampMetrics {
            timeline: MetricTriplet {
                f1: f64_at("/timestampMetrics/timeline/f1")?,
                precision: f64_at("/timestampMetrics/timeline/precision")?,
                recall: f64_at("/timestampMetrics/timeline/recall")?,
            },
            matched_segments: i64_at("/timestampMetrics/segments/matchedSegments")?,
            total_segments: i64_at("/timestampMetrics/segments/totalSegments")?,
            within_50_ms: i64_at("/timestampMetrics/segments/within50ms")?,
            within_100_ms: i64_at("/timestampMetrics/segments/within100ms")?,
            within_200_ms: i64_at("/timestampMetrics/segments/within200ms")?,
            avg_shift_ms: i64_at("/timestampMetrics/segments/avgShiftMs")?,
        },
        overall_word_error_rate: f64_at("/overallWordErrorRate")?,
        total_substitutions: i64_at("/totalSubstitutions")?,
        total_insertions: i64_at("/totalInsertions")?,
        total_deletions: i64_at("/totalDeletions")?,
    })
}

fn annotation_metrics(project: &Project) -> Option<AnnotationDiffMetrics> {
    let value: Value = serde_json::from_str(project.metadata.get(META_ANNOTATION_METRICS)?).ok()?;
    Some(AnnotationDiffMetrics {
        event_detection: metric_triplet(value.get("eventDetection")?)?,
        label_macro_average: metric_triplet(value.get("labelMacroAverage")?)?,
        intensity_calibration: metric_triplet(value.get("intensityCalibration")?)?,
    })
}

fn metric_triplet(value: &Value) -> Option<MetricTriplet> {
    Some(MetricTriplet {
        f1: value.get("f1")?.as_f64()?,
        precision: value.get("precision")?.as_f64()?,
        recall: value.get("recall")?.as_f64()?,
    })
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project
        .metadata
        .get(key)
        .is_some_and(|value| value == "true" || value == "1")
}
