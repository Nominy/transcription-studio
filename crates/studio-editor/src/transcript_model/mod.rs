mod actions;
mod row_availability;
mod time;
use crate::track_filter::TrackFilter;
pub(crate) use actions::build_transcript_action_menu;
pub use actions::{
    TranscriptActionCommand, TranscriptActionMenuEntry, TranscriptTrackOptionView,
    build_transcript_track_options, selected_track_option,
};
use exaudio_extension::document::{Decoration, Diagnostic, DiagnosticCollection, Severity};
pub use row_availability::compute_row_availability;
use std::collections::BTreeSet;
use studio_core::Project;
pub use time::format_mm_ss;
/// Neutral, versioned display payload. Metadata interpretation belongs to its provider.
pub const TRANSCRIPT_ROW_DECORATION: &str = "exaudio.transcript.row.v1";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TranscriptAnnotationKind {
    Transcription,
    NonTranscription(String),
}
impl TranscriptAnnotationKind {
    pub fn is_transcription(&self) -> bool {
        matches!(self, Self::Transcription)
    }
    pub fn label(&self) -> String {
        match self {
            Self::Transcription => "transcription".into(),
            Self::NonTranscription(label) => label.clone(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptReviewerView {
    pub label: String,
    pub color_hue: Option<u16>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptRowVisibleCells {
    pub id: bool,
    pub track: bool,
    pub start: bool,
    pub end: bool,
    pub reviewer: bool,
    pub text: bool,
    pub linter: bool,
    pub actions: bool,
}
impl Default for TranscriptRowVisibleCells {
    fn default() -> Self {
        Self {
            id: true,
            track: true,
            start: true,
            end: true,
            reviewer: false,
            text: true,
            linter: true,
            actions: true,
        }
    }
}
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
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
    pub lint: Option<Diagnostic>,
    pub lint_issues: Vec<Diagnostic>,
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptRowKind {
    Annotation,
    ModifiedAdded,
    ModifiedDeleted,
    AnnotationRemovedGhost,
}

/// Annotation (non-transcription) diff against the compare snapshot.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct TranscriptAnnotationDiffView {
    pub status: AnnotationDiffStatus,
    pub other_label: Option<String>,
    pub other_secondary_label: Option<String>,
    pub other_intensity: Option<u8>,
    pub counterpart_start_seconds: Option<f64>,
    pub counterpart_end_seconds: Option<f64>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationDiffStatus {
    Matched,
    LabelMismatch,
    SubcategoryMismatch,
    IntensityMismatch,
    /// Present in the compare version only (removed-annotation ghost rows).
    UnmatchedReference,
}

/// Text-cell diff for diff mode.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TranscriptDiffView {
    pub kind: TranscriptDiffCellKind,
    /// Number of word-level changes (a removed+added pair counts once).
    pub change_count: usize,
    /// Original text when it differs from the current text.
    pub source_text: Option<String>,
    /// Word tokens; empty means "no content" (or, for `New`, show the segment text as-is).
    pub words: Vec<TranscriptWordDiff>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptDiffCellKind {
    Diff,
    New,
    ModifiedAdded,
    ModifiedDeleted,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptWordDiffStatus {
    Unchanged,
    Added,
    Removed,
    Unknown,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct TranscriptWordDiff {
    pub status: TranscriptWordDiffStatus,
    pub value: String,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct TranscriptWordView {
    pub index: usize,
    pub text: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub is_low_confidence: bool,
    pub is_acknowledged: bool,
    pub command: Option<String>,
    pub command_arguments: serde_json::Value,
}

/// Extension-produced presentation data for one document revision.
pub(crate) struct RowContributions<'a> {
    pub diagnostics: &'a [DiagnosticCollection],
    pub decorations: Vec<Decoration>,
}

pub(crate) fn build_transcript_rows(
    project: &Project,
    track_filter: &TrackFilter,
    selected_segment_id: Option<&str>,
    diff_view: bool,
    asserted_warning_keys: &BTreeSet<String>,
    contributions: RowContributions<'_>,
    read_only: bool,
) -> Vec<TranscriptRowView> {
    let RowContributions {
        diagnostics,
        decorations,
    } = contributions;
    let availability = compute_row_availability(project);
    let track_options = build_transcript_track_options(project);
    let mut rows = std::collections::BTreeMap::new();
    for segment in &project.segments {
        let available = availability.get(&segment.id).copied().unwrap_or_default();
        rows.insert(
            segment.id.clone(),
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
                annotation_kind: TranscriptAnnotationKind::Transcription,
                annotation_type_label: "transcription".into(),
                start_time: format_mm_ss(segment.start_seconds),
                end_time: format_mm_ss(segment.end_seconds),
                visible_cells: TranscriptRowVisibleCells::default(),
                show_reviewer: false,
                reviewer: None,
                text: segment.text.clone(),
                is_selected: false,
                lint: None,
                lint_issues: Vec::new(),
                diff_marker: None,
                diff_source_text: None,
                diff: None,
                annotation_diff: None,
                word_views: Vec::new(),
                has_asserted_warning: false,
                has_unresolved_low_confidence: false,
                is_read_only: read_only,
                can_insert_above: available.can_insert_above,
                can_insert_below: available.can_insert_below,
                can_merge_above: available.can_merge_above,
                can_merge_below: available.can_merge_below,
            },
        );
    }
    for decoration in decorations {
        if decoration.kind != TRANSCRIPT_ROW_DECORATION {
            continue;
        }
        let Ok(mut row) = serde_json::from_value::<TranscriptRowView>(decoration.data) else {
            continue;
        };
        if row.segment_id != decoration.segment_id {
            continue;
        }
        if row.row_kind != TranscriptRowKind::Annotation && !diff_view {
            continue;
        }
        row.is_read_only |= read_only;
        if !diff_view {
            row.diff_marker = None;
            row.diff_source_text = None;
            row.diff = None;
            row.annotation_diff = None;
        }
        rows.insert(row.segment_id.clone(), row);
    }
    let mut rows: Vec<_> = rows
        .into_values()
        .filter(|row| row.lane_id.is_empty() || track_filter.matches_lane(&row.lane_id))
        .collect();
    rows.sort_by(|a, b| a.sort_start_seconds.total_cmp(&b.sort_start_seconds));
    for (index, row) in rows.iter_mut().enumerate() {
        row.display_id = (index + 1).to_string();
        row.is_selected = selected_segment_id == Some(row.segment_id.as_str());
        row.has_asserted_warning = false;
        row.lint_issues = diagnostics
            .iter()
            .flat_map(|collection| &collection.diagnostics)
            .filter(|issue| issue.segment_id == row.segment_id)
            .filter(|issue| {
                let asserted = issue.severity == Severity::Warning
                    && asserted_warning_keys.contains(&diagnostic_key(issue));
                row.has_asserted_warning |= asserted;
                !asserted
            })
            .cloned()
            .collect();
        row.lint = row.lint_issues.first().cloned();
        if row.is_read_only {
            row.can_insert_above = false;
            row.can_insert_below = false;
            row.can_merge_above = false;
            row.can_merge_below = false;
        }
    }
    rows
}

pub fn diagnostic_key(issue: &Diagnostic) -> String {
    format!(
        "{}::{}::{}::{}",
        issue.segment_id, issue.source, issue.rule, issue.message
    )
}
pub(crate) fn speaker_label(project: &Project, speaker_id: &str) -> String {
    project
        .speakers
        .iter()
        .find(|speaker| speaker.id == speaker_id)
        .map(|speaker| speaker.label.clone())
        .unwrap_or_else(|| speaker_id.to_string())
}
