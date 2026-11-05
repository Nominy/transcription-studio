mod actions;
mod annotation_kind;
mod diff;
mod reviewer;
mod row_availability;
mod time;
mod visible_cells;
mod words;

use std::collections::BTreeSet;

use crate::babel_metadata::project_is_read_only;
use crate::linter::{
    LintKind, LintStatus, ProjectLintAnalysis, ServerLintSnapshot, UNRESOLVED_LOW_CONFIDENCE_RULE,
    combine_server_and_local_issues, server_lint_issues_for_segment, warning_key,
};
use crate::track_filter::TrackFilter;
use diff::{
    annotation_diff_state, annotation_removed_ghost_rows, diff_state, modified_segment_diff_rows,
};
use reviewer::ReviewerLookup;
use row_availability::compute_row_availability;
use studio_core::Project;
use words::build_transcript_word_views;

pub(crate) use actions::build_transcript_action_menu;
pub use actions::{TranscriptActionCommand, TranscriptActionMenuEntry, TranscriptTrackOptionView};
use actions::{build_transcript_track_options, selected_track_option};
pub use annotation_kind::{
    BABEL_NON_TRANSCRIPTION_TYPE, BABEL_TRANSCRIPTION_TYPE, TranscriptAnnotationKind,
};
pub use reviewer::TranscriptReviewerView;
pub use time::format_mm_ss;
pub use visible_cells::TranscriptRowVisibleCells;
pub(crate) use visible_cells::derive_transcription_row_visible_cells;

const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptRowView {
    pub row_kind: TranscriptRowKind,
    pub sort_start_seconds: f64,
    pub segment_id: String,
    pub display_id: String,
    pub speaker_label: String,
    pub lane_id: String,
    pub selected_lane_id: String,
    pub selected_track_option: Option<TranscriptTrackOptionView>,
    pub track_options: Vec<TranscriptTrackOptionView>,
    pub annotation_kind: TranscriptAnnotationKind,
    pub annotation_type_label: String,
    pub start_time: String,
    pub end_time: String,
    pub visible_cells: TranscriptRowVisibleCells,
    pub show_reviewer: bool,
    pub reviewer: Option<TranscriptReviewerView>,
    pub text: String,
    pub is_selected: bool,
    pub lint: LintStatus,
    pub lint_issues: Vec<LintStatus>,
    pub diff_marker: Option<String>,
    pub diff_source_text: Option<String>,
    pub diff: Option<TranscriptDiffView>,
    pub annotation_diff: Option<TranscriptAnnotationDiffView>,
    pub word_views: Vec<TranscriptWordView>,
    pub has_asserted_warning: bool,
    pub has_unresolved_low_confidence: bool,
    pub is_read_only: bool,
    pub can_insert_above: bool,
    pub can_insert_below: bool,
    pub can_merge_above: bool,
    pub can_merge_below: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptRowKind {
    Annotation,
    ModifiedAdded,
    ModifiedDeleted,
    AnnotationRemovedGhost,
}

/// Annotation (non-transcription) diff against the compare snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptAnnotationDiffView {
    pub status: AnnotationDiffStatus,
    pub other_label: Option<String>,
    pub other_secondary_label: Option<String>,
    pub other_intensity: Option<u8>,
    pub counterpart_start_seconds: Option<f64>,
    pub counterpart_end_seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationDiffStatus {
    Matched,
    LabelMismatch,
    SubcategoryMismatch,
    IntensityMismatch,
    /// Present in the compare version only (removed-annotation ghost rows).
    UnmatchedReference,
}

/// Text-cell diff for diff mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptDiffView {
    pub kind: TranscriptDiffCellKind,
    /// Number of word-level changes (a removed+added pair counts once).
    pub change_count: usize,
    /// Original text when it differs from the current text.
    pub source_text: Option<String>,
    /// Word tokens; empty means "no content" (or, for `New`, show the segment text as-is).
    pub words: Vec<TranscriptWordDiff>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptDiffCellKind {
    Diff,
    New,
    ModifiedAdded,
    ModifiedDeleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptWordDiffStatus {
    Unchanged,
    Added,
    Removed,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptWordDiff {
    pub status: TranscriptWordDiffStatus,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptWordView {
    pub index: usize,
    pub text: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub is_low_confidence: bool,
}

pub(crate) fn build_transcript_rows(
    project: &Project,
    track_filter: &TrackFilter,
    selected_segment_id: Option<&str>,
    diff_view: bool,
    asserted_warning_keys: &BTreeSet<String>,
    server_lint_snapshot: Option<&ServerLintSnapshot>,
) -> Vec<TranscriptRowView> {
    let mut segments: Vec<_> = project.segments.iter().collect();
    segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    let row_availability = compute_row_availability(project);
    let lint_analysis = ProjectLintAnalysis::new(project);
    let selected_compare_action_id = selected_compare_action_id(project);
    let track_options = build_transcript_track_options(project);
    let reviewer_lookup = ReviewerLookup::for_project(project);
    let visible_cells = derive_transcription_row_visible_cells(project);
    let is_read_only = project_is_read_only(project);

    let mut rows: Vec<TranscriptRowView> = segments
        .into_iter()
        .filter(|segment| track_filter.matches_lane(&segment.lane_id))
        .map(|segment| {
            let annotation_kind = TranscriptAnnotationKind::from_segment(segment);
            let annotation_type_label = annotation_kind.label();
            let raw_lint_issues = combine_server_and_local_issues(
                server_lint_issues_for_segment(server_lint_snapshot, segment),
                lint_analysis.lint_segment_issues(segment),
            );
            let has_asserted_warning = raw_lint_issues.iter().any(|issue| {
                warning_key(segment, issue).is_some_and(|key| asserted_warning_keys.contains(&key))
            });
            let lint_issues = raw_lint_issues
                .into_iter()
                .filter(|issue| {
                    !warning_key(segment, issue)
                        .is_some_and(|key| asserted_warning_keys.contains(&key))
                })
                .collect::<Vec<_>>();
            let lint = if let Some(primary) = lint_issues.first() {
                primary.clone()
            } else if has_asserted_warning {
                LintStatus::pass("Warning asserted")
            } else {
                LintStatus::pass("OK")
            };
            let has_unresolved_low_confidence = lint_issues
                .iter()
                .any(|issue| issue.kind == LintKind::Gold(UNRESOLVED_LOW_CONFIDENCE_RULE));
            let availability = row_availability
                .get(&segment.id)
                .copied()
                .unwrap_or_default();
            let diff = diff_view.then(|| diff_state(segment, selected_compare_action_id));
            let diff_cell = diff.as_ref().map(|diff| diff.cell.clone());
            let annotation_diff = if diff_view {
                annotation_diff_state(segment, selected_compare_action_id)
            } else {
                None
            };
            TranscriptRowView {
                row_kind: TranscriptRowKind::Annotation,
                sort_start_seconds: segment.start_seconds,
                segment_id: segment.id.clone(),
                display_id: String::new(),
                speaker_label: speaker_label(project, &segment.speaker_id),
                lane_id: segment.lane_id.clone(),
                selected_lane_id: segment.lane_id.clone(),
                selected_track_option: selected_track_option(&track_options, &segment.lane_id),
                track_options: track_options.clone(),
                annotation_kind,
                annotation_type_label,
                start_time: format_mm_ss(segment.start_seconds),
                end_time: format_mm_ss(segment.end_seconds),
                visible_cells,
                show_reviewer: visible_cells.reviewer,
                reviewer: reviewer_lookup.reviewer_at(segment.start_seconds),
                text: segment.text.clone(),
                is_selected: selected_segment_id == Some(segment.id.as_str()),
                lint,
                lint_issues,
                diff_marker: diff.as_ref().map(|diff| diff.marker.clone()),
                diff_source_text: diff.as_ref().and_then(|diff| diff.cell.source_text.clone()),
                diff: diff_cell,
                annotation_diff,
                word_views: build_transcript_word_views(segment),
                has_asserted_warning,
                has_unresolved_low_confidence,
                is_read_only,
                can_insert_above: availability.can_insert_above,
                can_insert_below: availability.can_insert_below,
                can_merge_above: availability.can_merge_above,
                can_merge_below: availability.can_merge_below,
            }
        })
        .collect();

    if diff_view {
        rows.extend(
            modified_segment_diff_rows(project)
                .into_iter()
                .filter(|row| {
                    row.lane_id
                        .as_deref()
                        .is_none_or(|lane_id| track_filter.matches_lane(lane_id))
                })
                .map(|row| {
                    let row_kind = if row.show_as_added {
                        TranscriptRowKind::ModifiedAdded
                    } else {
                        TranscriptRowKind::ModifiedDeleted
                    };
                    let marker = if row.show_as_added {
                        "Added"
                    } else {
                        "Deleted"
                    };
                    let lane_id = row.lane_id.clone().unwrap_or_default();
                    TranscriptRowView {
                        row_kind,
                        sort_start_seconds: row.start_seconds.unwrap_or(0.0),
                        segment_id: format!("modified-{}", row.annotation_id),
                        display_id: String::new(),
                        speaker_label: marker.to_string(),
                        lane_id: lane_id.clone(),
                        selected_lane_id: lane_id.clone(),
                        selected_track_option: selected_track_option(&track_options, &lane_id),
                        track_options: track_options.clone(),
                        annotation_kind: TranscriptAnnotationKind::Transcription,
                        annotation_type_label: "Transcription".to_string(),
                        start_time: row
                            .start_seconds
                            .map(format_mm_ss)
                            .unwrap_or_else(|| "--".to_string()),
                        end_time: row
                            .end_seconds
                            .map(format_mm_ss)
                            .unwrap_or_else(|| "--".to_string()),
                        visible_cells,
                        show_reviewer: visible_cells.reviewer,
                        reviewer: row
                            .start_seconds
                            .and_then(|start_seconds| reviewer_lookup.reviewer_at(start_seconds)),
                        text: String::new(),
                        is_selected: false,
                        lint: LintStatus::pass("OK"),
                        lint_issues: Vec::new(),
                        diff_marker: Some(marker.to_string()),
                        diff_source_text: None,
                        diff: Some(row.cell),
                        annotation_diff: None,
                        word_views: Vec::new(),
                        has_asserted_warning: false,
                        has_unresolved_low_confidence: false,
                        is_read_only: true,
                        can_insert_above: false,
                        can_insert_below: false,
                        can_merge_above: false,
                        can_merge_below: false,
                    }
                }),
        );
        rows.extend(
            annotation_removed_ghost_rows(project)
                .into_iter()
                .map(|row| TranscriptRowView {
                    row_kind: TranscriptRowKind::AnnotationRemovedGhost,
                    sort_start_seconds: row.start_seconds.unwrap_or(0.0),
                    segment_id: format!("annotation-ghost-{}", row.annotation_id),
                    display_id: String::new(),
                    speaker_label: row.comparison_label,
                    lane_id: String::new(),
                    selected_lane_id: String::new(),
                    selected_track_option: None,
                    track_options: track_options.clone(),
                    annotation_kind: TranscriptAnnotationKind::NonTranscription(
                        "removed".to_string(),
                    ),
                    annotation_type_label: "Removed".to_string(),
                    start_time: row
                        .start_seconds
                        .map(format_mm_ss)
                        .unwrap_or_else(|| "--:--".to_string()),
                    end_time: row
                        .end_seconds
                        .map(format_mm_ss)
                        .unwrap_or_else(|| "--:--".to_string()),
                    visible_cells,
                    show_reviewer: visible_cells.reviewer,
                    reviewer: row
                        .start_seconds
                        .and_then(|start_seconds| reviewer_lookup.reviewer_at(start_seconds)),
                    text: row.text,
                    is_selected: false,
                    lint: LintStatus::pass("OK"),
                    lint_issues: Vec::new(),
                    diff_marker: Some("Removed".to_string()),
                    diff_source_text: None,
                    diff: None,
                    annotation_diff: Some(row.diff),
                    word_views: Vec::new(),
                    has_asserted_warning: false,
                    has_unresolved_low_confidence: false,
                    is_read_only: true,
                    can_insert_above: false,
                    can_insert_below: false,
                    can_merge_above: false,
                    can_merge_below: false,
                }),
        );
        rows.sort_by(|left, right| left.sort_start_seconds.total_cmp(&right.sort_start_seconds));
    }

    apply_visible_order_display_ids(&mut rows);

    rows
}

fn apply_visible_order_display_ids(rows: &mut [TranscriptRowView]) {
    for (index, row) in rows.iter_mut().enumerate() {
        row.display_id = (index + 1).to_string();
    }
}

fn selected_compare_action_id(project: &Project) -> Option<&str> {
    project
        .metadata
        .get(META_SELECTED_COMPARE_ACTION_ID)
        .map(String::as_str)
        .filter(|value| !value.is_empty())
}

pub(crate) fn speaker_label(project: &Project, speaker_id: &str) -> String {
    project
        .speakers
        .iter()
        .find(|speaker| speaker.id == speaker_id)
        .map(|speaker| speaker.label.clone())
        .unwrap_or_else(|| speaker_id.to_string())
}
