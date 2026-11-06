mod annotation_kind;
mod diff;
mod reviewer;
mod visible_cells;
mod words;

use crate::babel_metadata::project_is_read_only;
use diff::{
    annotation_diff_state, annotation_removed_ghost_rows, diff_state, modified_segment_diff_rows,
};
use reviewer::ReviewerLookup;
use studio_core::Project;
use studio_editor::track_filter::TrackFilter;
use studio_editor::transcript_model::compute_row_availability;
use studio_editor::transcript_model::*;
pub(crate) use words::build_transcript_word_views;

const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";

pub fn build_transcript_rows(
    project: &Project,
    track_filter: &TrackFilter,
    selected_segment_id: Option<&str>,
    diff_view: bool,
) -> Vec<TranscriptRowView> {
    let mut segments: Vec<_> = project.segments.iter().collect();
    segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    let row_availability = compute_row_availability(project);
    let selected_compare_action_id = selected_compare_action_id(project);
    let track_options = build_transcript_track_options(project);
    let reviewer_lookup = ReviewerLookup::for_project(project);
    let visible_cells = visible_cells::derive_transcription_row_visible_cells(project);
    let is_read_only = project_is_read_only(project);

    let mut rows: Vec<TranscriptRowView> = segments
        .into_iter()
        .filter(|segment| track_filter.matches_lane(&segment.lane_id))
        .map(|segment| {
            let annotation_kind = annotation_kind::annotation_kind(segment);
            let annotation_type_label = annotation_kind.label();
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
                lint: None,
                lint_issues: Vec::new(),
                diff_marker: diff.as_ref().map(|diff| diff.marker.clone()),
                diff_source_text: diff.as_ref().and_then(|diff| diff.cell.source_text.clone()),
                diff: diff_cell,
                annotation_diff,
                word_views: build_transcript_word_views(segment),
                has_asserted_warning: false,
                has_unresolved_low_confidence:
                    exaudio_babel_lint::linter::has_unresolved_low_confidence_word(segment),
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
                        lint: None,
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
                    lint: None,
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

#[derive(Debug)]
pub struct BabelPresentationProvider {
    pub acknowledgements: std::sync::Arc<crate::workflow::BabelLowConfidenceCommandProvider>,
}

impl exaudio_extension::document::DecorationProvider for BabelPresentationProvider {
    fn decorate(
        &self,
        project: &Project,
        revision: u64,
    ) -> Vec<exaudio_extension::document::Decoration> {
        build_transcript_rows(project, &TrackFilter::All, None, true)
            .into_iter()
            .map(|mut row| {
                for word in &mut row.word_views {
                    word.is_acknowledged = self.acknowledgements.is_acknowledged(
                        &row.segment_id,
                        word.index,
                        revision,
                    );
                }
                exaudio_extension::document::Decoration {
                    source: "exaudio.babel-domain".into(),
                    kind: TRANSCRIPT_ROW_DECORATION.into(),
                    segment_id: row.segment_id.clone(),
                    range: None,
                    label: row.annotation_type_label.clone(),
                    data: serde_json::to_value(row).expect("finite transcript presentation"),
                }
            })
            .collect()
    }
}

impl exaudio_extension::document::SegmentKindProvider for BabelPresentationProvider {
    fn is_text(&self, segment: &studio_core::Segment) -> Option<bool> {
        Some(
            segment
                .metadata
                .get("type")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|kind| kind == "transcription"),
        )
    }
}
