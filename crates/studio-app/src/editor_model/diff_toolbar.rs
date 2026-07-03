use serde_json::Value;
use studio_core::Project;

use super::EditorSession;

const META_AVAILABLE_REVIEW_ACTIONS: &str = "studio.babel.diff.availableReviewActions";
const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";
const META_DIFF_METRICS: &str = "studio.babel.diff.metrics";
const META_ANNOTATION_TOOLBAR: &str = "studio.babel.diff.annotationToolbar";
const META_ANNOTATION_METRICS: &str = "studio.babel.diff.annotationMetrics";
const META_SHOW_REVIEWERS: &str = "studio.babel.diff.showReviewers";
const META_HAS_REVIEWER_TOGGLE: &str = "studio.babel.diff.hasReviewerToggle";
const META_ASSESSMENT_PHASE: &str = "studio.babel.diff.assessmentPhase";
const META_IS_LOADING: &str = "studio.babel.diff.isLoading";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffToolbarView {
    pub mode: DiffToolbarMode,
    pub assessment_label: Option<String>,
    pub show_reviewers_control: bool,
    pub reviewers_checked: bool,
    pub diff_toggle: DiffToolbarToggleView,
    pub unsaved_changes_badge: DiffToolbarBadgeView,
    pub compare_select: DiffCompareSelectView,
    pub no_versions_text: Option<String>,
    pub metrics: DiffToolbarMetricsView,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffToolbarMode {
    Normal,
    Assessment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffToolbarToggleView {
    pub visible: bool,
    pub id: String,
    pub label: String,
    pub checked: bool,
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffToolbarBadgeView {
    pub visible: bool,
    pub variant: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffCompareSelectView {
    pub visible: bool,
    pub label: String,
    pub value: Option<String>,
    pub disabled: bool,
    pub loading_text: String,
    pub placeholder: String,
    pub empty_text: String,
    pub options: Vec<DiffCompareOptionView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffCompareOptionView {
    pub value: String,
    pub label: String,
}

impl std::fmt::Display for DiffCompareOptionView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.label.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffToolbarMetricsView {
    pub visible: bool,
    pub attempt: Option<DiffMetricValueView>,
    pub avg_shift: Option<DiffMetricValueView>,
    pub word_diff: Option<DiffMetricValueView>,
    pub event: Option<DiffAnnotationMetricView>,
    pub label_accuracy: Option<DiffAnnotationMetricView>,
    pub intensity: Option<DiffAnnotationMetricView>,
    pub substitutions: Option<DiffMetricCountView>,
    pub additions: Option<DiffMetricCountView>,
    pub removals: Option<DiffMetricCountView>,
    pub legend: Vec<DiffToolbarLegendItemView>,
    pub timestamp_tooltip: Option<DiffTimestampMetricTooltipView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffMetricValueView {
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffMetricCountView {
    pub label: String,
    pub value: i64,
    pub class_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffAnnotationMetricView {
    pub label: String,
    pub value: String,
    pub tooltip: Option<DiffAnnotationMetricTooltipView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffToolbarLegendItemView {
    pub color: String,
    pub label: String,
    pub tooltip_title: Option<String>,
    pub tooltip_desc: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffTimestampMetricTooltipView {
    pub title: String,
    pub f1_score: DiffMetricValueView,
    pub secondary_metrics: Vec<DiffMetricValueView>,
    pub matched_segments_text: String,
    pub tolerance_bars: Vec<DiffToleranceBarView>,
    pub toggle_label: String,
    pub descriptions_visible: bool,
    pub descriptions: Vec<DiffMetricDescriptionView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffToleranceBarView {
    pub label: String,
    pub value: i64,
    pub value_label: String,
    pub color_class: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffMetricDescriptionView {
    pub label: String,
    pub desc: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffAnnotationMetricTooltipView {
    pub title: String,
    pub f1_score: DiffMetricValueView,
    pub secondary_metrics: Vec<DiffMetricValueView>,
    pub toggle_label: String,
    pub descriptions_visible: bool,
    pub descriptions: Vec<DiffMetricDescriptionView>,
}

impl EditorSession {
    pub fn diff_toolbar(&self, project: &Project) -> DiffToolbarView {
        build_diff_toolbar(project, self.diff_view, self.is_dirty)
    }
}

fn build_diff_toolbar(
    project: &Project,
    is_diff_mode: bool,
    has_unsaved_changes: bool,
) -> DiffToolbarView {
    let annotation_toolbar = metadata_bool(project, META_ANNOTATION_TOOLBAR);
    let actions = review_action_options(project, annotation_toolbar);
    let has_versions = !actions.is_empty();
    let is_loading = metadata_bool(project, META_IS_LOADING);
    let metrics = diff_metrics(project);
    let annotation_metrics = annotation_metrics(project);
    let phase = assessment_phase(project);
    let is_assessment = phase.is_some();
    let loading_text = if annotation_toolbar {
        "Loading\u{2026}"
    } else {
        "Loading..."
    };
    let placeholder = if annotation_toolbar {
        "Select version\u{2026}"
    } else {
        "Select version..."
    };
    let empty_text = if annotation_toolbar {
        "No other versions available"
    } else {
        "No previous versions available"
    };
    let no_versions_text = if annotation_toolbar {
        "No other versions to compare"
    } else {
        "No previous versions to compare"
    };

    DiffToolbarView {
        mode: if is_assessment {
            DiffToolbarMode::Assessment
        } else {
            DiffToolbarMode::Normal
        },
        assessment_label: phase.map(assessment_label),
        show_reviewers_control: !annotation_toolbar
            && !is_assessment
            && metadata_bool(project, META_HAS_REVIEWER_TOGGLE),
        reviewers_checked: metadata_bool(project, META_SHOW_REVIEWERS),
        diff_toggle: DiffToolbarToggleView {
            visible: !is_assessment,
            id: if annotation_toolbar {
                "annotation-diff-mode"
            } else {
                "diff-mode"
            }
            .to_string(),
            label: if annotation_toolbar {
                "Annotation Diff"
            } else {
                "Diff View"
            }
            .to_string(),
            checked: is_diff_mode,
            disabled: !is_diff_mode && !has_versions,
        },
        unsaved_changes_badge: DiffToolbarBadgeView {
            visible: !is_assessment && is_diff_mode && has_unsaved_changes,
            variant: "warning".to_string(),
            text: "Unsaved changes".to_string(),
        },
        compare_select: DiffCompareSelectView {
            visible: !is_assessment && is_diff_mode,
            label: "Compare:".to_string(),
            value: selected_compare_action_id(project),
            disabled: is_loading,
            loading_text: loading_text.to_string(),
            placeholder: placeholder.to_string(),
            empty_text: empty_text.to_string(),
            options: actions,
        },
        no_versions_text: (!is_assessment && !is_diff_mode && !has_versions && !is_loading)
            .then(|| no_versions_text.to_string()),
        metrics: if annotation_toolbar {
            build_annotation_metrics_view(is_diff_mode, annotation_metrics.as_ref())
        } else {
            build_metrics_view(is_diff_mode, metrics.as_ref())
        },
    }
}

fn assessment_phase(project: &Project) -> Option<&str> {
    project
        .metadata
        .get(META_ASSESSMENT_PHASE)
        .map(String::as_str)
        .filter(|phase| !phase.is_empty())
}

fn assessment_label(phase: &str) -> String {
    if phase == "practice" {
        "Practice Mode".to_string()
    } else {
        "Assessment".to_string()
    }
}

fn build_metrics_view(
    is_diff_mode: bool,
    metrics: Option<&TranscriptionDiffMetrics>,
) -> DiffToolbarMetricsView {
    let legend = transcription_diff_legend();
    let Some(metrics) = metrics.filter(|_| is_diff_mode) else {
        return DiffToolbarMetricsView {
            visible: false,
            attempt: None,
            avg_shift: None,
            word_diff: None,
            event: None,
            label_accuracy: None,
            intensity: None,
            substitutions: None,
            additions: None,
            removals: None,
            legend,
            timestamp_tooltip: None,
        };
    };

    DiffToolbarMetricsView {
        visible: true,
        attempt: None,
        avg_shift: Some(DiffMetricValueView {
            label: "Avg shift: ".to_string(),
            value: format!("{}ms", metrics.timestamp_metrics.segments.avg_shift_ms),
        }),
        word_diff: Some(DiffMetricValueView {
            label: "Word Diff: ".to_string(),
            value: format_percent(metrics.overall_word_error_rate, 2),
        }),
        event: None,
        label_accuracy: None,
        intensity: None,
        substitutions: Some(DiffMetricCountView {
            label: "S".to_string(),
            value: metrics.total_substitutions,
            class_name: "text-amber-600".to_string(),
        }),
        additions: Some(DiffMetricCountView {
            label: "A".to_string(),
            value: metrics.total_insertions,
            class_name: "text-green-600".to_string(),
        }),
        removals: Some(DiffMetricCountView {
            label: "R".to_string(),
            value: metrics.total_deletions,
            class_name: "text-red-600".to_string(),
        }),
        legend,
        timestamp_tooltip: Some(timestamp_tooltip(&metrics.timestamp_metrics)),
    }
}

fn build_annotation_metrics_view(
    is_diff_mode: bool,
    metrics: Option<&AnnotationDiffMetrics>,
) -> DiffToolbarMetricsView {
    let legend = annotation_diff_legend();
    let Some(metrics) = metrics.filter(|_| is_diff_mode) else {
        return DiffToolbarMetricsView {
            visible: false,
            attempt: None,
            avg_shift: None,
            word_diff: None,
            event: None,
            label_accuracy: None,
            intensity: None,
            substitutions: None,
            additions: None,
            removals: None,
            legend,
            timestamp_tooltip: None,
        };
    };

    DiffToolbarMetricsView {
        visible: true,
        attempt: None,
        avg_shift: None,
        word_diff: None,
        event: Some(annotation_metric_view(
            "Event",
            &metrics.event_detection,
            annotation_metric_tooltip_event(&metrics.event_detection),
        )),
        label_accuracy: Some(annotation_metric_view(
            "Label",
            &metrics.label_macro_average,
            annotation_metric_tooltip_label(&metrics.label_macro_average),
        )),
        intensity: Some(annotation_metric_view(
            "Intensity",
            &metrics.intensity_calibration,
            annotation_metric_tooltip_intensity(&metrics.intensity_calibration),
        )),
        substitutions: None,
        additions: None,
        removals: None,
        legend,
        timestamp_tooltip: None,
    }
}

fn annotation_metric_view(
    label: &str,
    metric: &AnnotationMetricTriplet,
    tooltip: DiffAnnotationMetricTooltipView,
) -> DiffAnnotationMetricView {
    DiffAnnotationMetricView {
        label: label.to_string(),
        value: format_percent(metric.f1, 1),
        tooltip: Some(tooltip),
    }
}

fn annotation_metric_tooltip(
    title: &str,
    metric: &AnnotationMetricTriplet,
    f1_desc: &str,
    precision_desc: &str,
    recall_desc: &str,
) -> DiffAnnotationMetricTooltipView {
    DiffAnnotationMetricTooltipView {
        title: title.to_string(),
        f1_score: DiffMetricValueView {
            label: "F1 Score".to_string(),
            value: format_percent(metric.f1, 1),
        },
        secondary_metrics: vec![
            DiffMetricValueView {
                label: "Precision".to_string(),
                value: format_percent(metric.precision, 1),
            },
            DiffMetricValueView {
                label: "Recall".to_string(),
                value: format_percent(metric.recall, 1),
            },
        ],
        toggle_label: "What does this mean?".to_string(),
        descriptions_visible: false,
        descriptions: vec![
            DiffMetricDescriptionView {
                label: "F1 Score".to_string(),
                desc: f1_desc.to_string(),
            },
            DiffMetricDescriptionView {
                label: "Precision".to_string(),
                desc: precision_desc.to_string(),
            },
            DiffMetricDescriptionView {
                label: "Recall".to_string(),
                desc: recall_desc.to_string(),
            },
        ],
    }
}

fn timestamp_tooltip(metrics: &TimestampMetrics) -> DiffTimestampMetricTooltipView {
    DiffTimestampMetricTooltipView {
        title: "Timestamp Accuracy".to_string(),
        f1_score: DiffMetricValueView {
            label: "F1 Score".to_string(),
            value: format_percent(metrics.timeline.f1, 1),
        },
        secondary_metrics: vec![
            DiffMetricValueView {
                label: "Recall".to_string(),
                value: format_percent(metrics.timeline.recall, 1),
            },
            DiffMetricValueView {
                label: "Precision".to_string(),
                value: format_percent(metrics.timeline.precision, 1),
            },
        ],
        matched_segments_text: format!(
            "{}/{} segments matched",
            metrics.segments.matched_segments, metrics.segments.total_segments
        ),
        tolerance_bars: vec![
            tolerance_bar("±50ms", metrics.segments.within_50_ms, "bg-emerald-500"),
            tolerance_bar("±100ms", metrics.segments.within_100_ms, "bg-sky-500"),
            tolerance_bar("±200ms", metrics.segments.within_200_ms, "bg-violet-500"),
        ],
        toggle_label: "What do these mean?".to_string(),
        descriptions_visible: false,
        descriptions: timestamp_metric_descriptions(),
    }
}

fn timestamp_metric_descriptions() -> Vec<DiffMetricDescriptionView> {
    vec![
        DiffMetricDescriptionView {
            label: "F1 Score".to_string(),
            desc: "Overall timestamp alignment score combining Precision and Recall. 100% means both annotations have identical speech boundaries.".to_string(),
        },
        DiffMetricDescriptionView {
            label: "Recall".to_string(),
            desc: "Low recall means the older annotation is missing speech the newer annotation captured, e.g. segments cut off early or started too late.".to_string(),
        },
        DiffMetricDescriptionView {
            label: "Precision".to_string(),
            desc: "Low precision means the older annotation marked time as speech that the newer annotation considers silence or background noise.".to_string(),
        },
        DiffMetricDescriptionView {
            label: "Segments matched & tolerance".to_string(),
            desc: "Segments paired one-to-one by word alignment (splits and merges excluded). The bars show what percentage of those matched segments have both start and end timestamps within ±50ms, ±100ms, and ±200ms.".to_string(),
        },
    ]
}

fn annotation_metric_tooltip_event(
    metric: &AnnotationMetricTriplet,
) -> DiffAnnotationMetricTooltipView {
    annotation_metric_tooltip(
        "Event Detection",
        metric,
        "Overall agreement on whether a sound event exists in each time window, ignoring label. 100% means the older and newer annotations agree on the exact presence and absence of all events.",
        "Low precision means the older annotation marks events the newer annotation considers silence.",
        "Low recall means the older annotation is missing events the newer annotation captured.",
    )
}

fn annotation_metric_tooltip_label(
    metric: &AnnotationMetricTriplet,
) -> DiffAnnotationMetricTooltipView {
    annotation_metric_tooltip(
        "Label Accuracy",
        metric,
        "Agreement on primary label identity, macro-averaged across all label classes. 100% means the older and newer annotations assign the same label to every event window.",
        "Low precision means the older annotation applies labels the newer annotation does not agree with.",
        "Low recall means the older annotation is missing label categories the newer annotation uses.",
    )
}

fn annotation_metric_tooltip_intensity(
    metric: &AnnotationMetricTriplet,
) -> DiffAnnotationMetricTooltipView {
    annotation_metric_tooltip(
        "Intensity Calibration",
        metric,
        "Agreement on intensity group (low 0-1 vs. high 2-4) per event window. 100% means the older and newer annotations consistently rate the strength of every sound event the same way.",
        "Low precision means the older annotation rates events as high intensity that the newer annotation considers low.",
        "Low recall means the older annotation rates events as low intensity that the newer annotation considers high.",
    )
}

fn tolerance_bar(label: &str, value: i64, color_class: &str) -> DiffToleranceBarView {
    DiffToleranceBarView {
        label: label.to_string(),
        value,
        value_label: format!("{value}%"),
        color_class: color_class.to_string(),
    }
}

fn transcription_diff_legend() -> Vec<DiffToolbarLegendItemView> {
    vec![
        DiffToolbarLegendItemView {
            color: "amber".to_string(),
            label: "Substituted".to_string(),
            tooltip_title: None,
            tooltip_desc: None,
        },
        DiffToolbarLegendItemView {
            color: "green".to_string(),
            label: "Added".to_string(),
            tooltip_title: None,
            tooltip_desc: None,
        },
        DiffToolbarLegendItemView {
            color: "red".to_string(),
            label: "Removed".to_string(),
            tooltip_title: None,
            tooltip_desc: None,
        },
    ]
}

fn annotation_diff_legend() -> Vec<DiffToolbarLegendItemView> {
    vec![
        DiffToolbarLegendItemView {
            color: "bg-green-500".to_string(),
            label: "Added".to_string(),
            tooltip_title: Some("Added".to_string()),
            tooltip_desc: Some(
                "You have this annotation but the comparison version doesn't.".to_string(),
            ),
        },
        DiffToolbarLegendItemView {
            color: "bg-amber-500".to_string(),
            label: "Substituted".to_string(),
            tooltip_title: Some("Substituted".to_string()),
            tooltip_desc: Some("Both versions have an overlapping annotation but differ in primary label, subcategory, or intensity. The old value is shown crossed out with the new value beside it.".to_string()),
        },
        DiffToolbarLegendItemView {
            color: "bg-red-500".to_string(),
            label: "Removed".to_string(),
            tooltip_title: Some("Removed".to_string()),
            tooltip_desc: Some(
                "The comparison version has this annotation but you don't.".to_string(),
            ),
        },
        DiffToolbarLegendItemView {
            color: "bg-neutral-300".to_string(),
            label: "Unchanged".to_string(),
            tooltip_title: Some("Unchanged".to_string()),
            tooltip_desc: Some(
                "Both versions have this annotation with the same label, subcategory, and intensity."
                    .to_string(),
            ),
        },
    ]
}

fn review_action_options(
    project: &Project,
    include_worker_name: bool,
) -> Vec<DiffCompareOptionView> {
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
            if include_worker_name {
                if let Some(worker_name) = action.get("workerName").and_then(Value::as_str) {
                    if !worker_name.is_empty() {
                        label.push_str(&format!(" \u{00b7} {worker_name}"));
                    }
                }
            }
            Some(DiffCompareOptionView { value: id, label })
        })
        .collect()
}

fn selected_compare_action_id(project: &Project) -> Option<String> {
    project
        .metadata
        .get(META_SELECTED_COMPARE_ACTION_ID)
        .filter(|value| !value.is_empty())
        .cloned()
}

fn diff_metrics(project: &Project) -> Option<TranscriptionDiffMetrics> {
    let value: Value = serde_json::from_str(project.metadata.get(META_DIFF_METRICS)?).ok()?;
    Some(TranscriptionDiffMetrics {
        timestamp_metrics: TimestampMetrics {
            timeline: TimestampTimelineMetrics {
                f1: value.pointer("/timestampMetrics/timeline/f1")?.as_f64()?,
                recall: value
                    .pointer("/timestampMetrics/timeline/recall")?
                    .as_f64()?,
                precision: value
                    .pointer("/timestampMetrics/timeline/precision")?
                    .as_f64()?,
            },
            segments: TimestampSegmentMetrics {
                matched_segments: value
                    .pointer("/timestampMetrics/segments/matchedSegments")?
                    .as_i64()?,
                total_segments: value
                    .pointer("/timestampMetrics/segments/totalSegments")?
                    .as_i64()?,
                within_50_ms: value
                    .pointer("/timestampMetrics/segments/within50ms")?
                    .as_i64()?,
                within_100_ms: value
                    .pointer("/timestampMetrics/segments/within100ms")?
                    .as_i64()?,
                within_200_ms: value
                    .pointer("/timestampMetrics/segments/within200ms")?
                    .as_i64()?,
                avg_shift_ms: value
                    .pointer("/timestampMetrics/segments/avgShiftMs")?
                    .as_i64()?,
            },
        },
        overall_word_error_rate: value.pointer("/overallWordErrorRate")?.as_f64()?,
        total_substitutions: value.pointer("/totalSubstitutions")?.as_i64()?,
        total_insertions: value.pointer("/totalInsertions")?.as_i64()?,
        total_deletions: value.pointer("/totalDeletions")?.as_i64()?,
    })
}

fn annotation_metrics(project: &Project) -> Option<AnnotationDiffMetrics> {
    let value: Value = serde_json::from_str(project.metadata.get(META_ANNOTATION_METRICS)?).ok()?;
    Some(AnnotationDiffMetrics {
        event_detection: annotation_metric_triplet(value.get("eventDetection")?)?,
        label_macro_average: annotation_metric_triplet(value.get("labelMacroAverage")?)?,
        intensity_calibration: annotation_metric_triplet(value.get("intensityCalibration")?)?,
    })
}

fn annotation_metric_triplet(value: &Value) -> Option<AnnotationMetricTriplet> {
    Some(AnnotationMetricTriplet {
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

fn format_percent(value: f64, fraction_digits: usize) -> String {
    format!("{:.*}%", fraction_digits, value * 100.0)
}

#[derive(Clone, Debug, PartialEq)]
struct TranscriptionDiffMetrics {
    timestamp_metrics: TimestampMetrics,
    overall_word_error_rate: f64,
    total_substitutions: i64,
    total_insertions: i64,
    total_deletions: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct AnnotationDiffMetrics {
    event_detection: AnnotationMetricTriplet,
    label_macro_average: AnnotationMetricTriplet,
    intensity_calibration: AnnotationMetricTriplet,
}

#[derive(Clone, Debug, PartialEq)]
struct AnnotationMetricTriplet {
    f1: f64,
    precision: f64,
    recall: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct TimestampMetrics {
    timeline: TimestampTimelineMetrics,
    segments: TimestampSegmentMetrics,
}

#[derive(Clone, Debug, PartialEq)]
struct TimestampTimelineMetrics {
    f1: f64,
    recall: f64,
    precision: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct TimestampSegmentMetrics {
    matched_segments: i64,
    total_segments: i64,
    within_50_ms: i64,
    within_100_ms: i64,
    within_200_ms: i64,
    avg_shift_ms: i64,
}
