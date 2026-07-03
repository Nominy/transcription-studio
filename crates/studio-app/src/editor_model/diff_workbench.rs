use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffReviewActionSummary {
    pub id: String,
    pub level: i32,
    pub label: Option<String>,
    pub created_at: Option<String>,
    pub worker_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffTranscriptionDiffActionsInput<'a> {
    pub is_assessment_mode: bool,
    pub assessment_answer_review_action_id: Option<&'a str>,
    pub review_actions: &'a [DiffReviewActionSummary],
    pub now_iso: &'a str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffTranscriptionSelectionInput<'a> {
    pub is_opening: bool,
    pub selected_compare_action_id: Option<&'a str>,
    pub available_actions: &'a [DiffReviewActionSummary],
    pub current_level: i32,
    pub is_assessment_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiffAnnotationDiffPairInput<'a> {
    pub current_review_action_id: &'a str,
    pub selected_compare_action_id: Option<&'a str>,
    pub selected_compare_action_level: Option<i32>,
    pub current_level: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffComparePair {
    pub reference_review_action_id: String,
    pub current_review_action_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptAnnotationDiffSource {
    Reference,
    Prediction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffAnnotationStatus {
    Matched,
    LabelMismatch,
    SubcategoryMismatch,
    IntensityMismatch,
    UnmatchedReference,
    UnmatchedPrediction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffAnnotationForMatch {
    pub id: String,
    pub primary_label: Option<String>,
    pub secondary_label: Option<String>,
    pub intensity: Option<i32>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffAnnotationRow {
    pub annotation: DiffAnnotationForMatch,
    pub source: TranscriptAnnotationDiffSource,
    pub status: DiffAnnotationStatus,
    pub matched_annotation_id: Option<String>,
    pub other_label: Option<String>,
    pub other_secondary_label: Option<String>,
    pub other_intensity: Option<i32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiffAnnotationStatusEntry {
    pub status: DiffAnnotationStatus,
    pub other_label: Option<String>,
    pub other_secondary_label: Option<String>,
    pub other_intensity: Option<i32>,
    pub counterpart_start_seconds: Option<f64>,
    pub counterpart_end_seconds: Option<f64>,
}

pub fn build_transcription_diff_actions(
    input: DiffTranscriptionDiffActionsInput<'_>,
) -> Vec<DiffReviewActionSummary> {
    if input.is_assessment_mode {
        if let Some(answer_review_action_id) = input.assessment_answer_review_action_id {
            return vec![DiffReviewActionSummary {
                id: answer_review_action_id.to_string(),
                level: 0,
                label: Some("Answer".to_string()),
                created_at: Some(input.now_iso.to_string()),
                worker_id: None,
            }];
        }
    }

    label_review_actions(input.review_actions)
}

pub fn choose_transcription_compare_selection(
    input: DiffTranscriptionSelectionInput<'_>,
) -> Option<String> {
    if !input.is_opening {
        return None;
    }

    if let Some(selected_compare_action_id) = input.selected_compare_action_id {
        return Some(selected_compare_action_id.to_string());
    }

    let first_action = input.available_actions.first()?;
    if input.is_assessment_mode {
        return Some(first_action.id.clone());
    }

    Some(
        input
            .available_actions
            .iter()
            .find(|action| action.level == input.current_level - 1)
            .unwrap_or(first_action)
            .id
            .clone(),
    )
}

pub fn is_transcription_compare_newer(
    is_assessment_mode: bool,
    selected_compare_action_id: Option<&str>,
    available_actions: &[DiffReviewActionSummary],
    current_level: i32,
) -> bool {
    if is_assessment_mode {
        return true;
    }

    selected_compare_action_id.is_some_and(|selected_compare_action_id| {
        available_actions
            .iter()
            .find(|action| action.id == selected_compare_action_id)
            .is_some_and(|action| action.level > current_level)
    })
}

pub fn build_transcription_diff_pair(
    current_review_action_id: &str,
    selected_compare_action_id: Option<&str>,
    is_compare_newer: bool,
) -> Option<DiffComparePair> {
    let selected_compare_action_id = selected_compare_action_id?;
    Some(if is_compare_newer {
        DiffComparePair {
            reference_review_action_id: current_review_action_id.to_string(),
            current_review_action_id: selected_compare_action_id.to_string(),
        }
    } else {
        DiffComparePair {
            reference_review_action_id: selected_compare_action_id.to_string(),
            current_review_action_id: current_review_action_id.to_string(),
        }
    })
}

pub fn build_annotation_diff_actions(
    review_actions: &[DiffReviewActionSummary],
) -> Vec<DiffReviewActionSummary> {
    label_review_actions(review_actions)
}

pub fn choose_annotation_compare_selection(
    is_opening: bool,
    available_actions: &[DiffReviewActionSummary],
    current_level: i32,
) -> Option<String> {
    if !is_opening {
        return None;
    }

    let first_action = available_actions.first()?;
    Some(
        available_actions
            .iter()
            .find(|action| action.level == current_level - 1)
            .unwrap_or(first_action)
            .id
            .clone(),
    )
}

pub fn build_annotation_diff_pair(
    input: DiffAnnotationDiffPairInput<'_>,
) -> Option<DiffComparePair> {
    let selected_compare_action_id = input.selected_compare_action_id?;
    let selected_compare_action_level = input.selected_compare_action_level?;
    let selected_is_higher_level = selected_compare_action_level > input.current_level;
    Some(DiffComparePair {
        reference_review_action_id: if selected_is_higher_level {
            selected_compare_action_id
        } else {
            input.current_review_action_id
        }
        .to_string(),
        current_review_action_id: if selected_is_higher_level {
            input.current_review_action_id
        } else {
            selected_compare_action_id
        }
        .to_string(),
    })
}

pub fn resolve_annotation_diff_perspective(
    selected_compare_action_level: Option<i32>,
    current_level: i32,
) -> Option<TranscriptAnnotationDiffSource> {
    let selected_compare_action_level = selected_compare_action_level?;
    Some(if selected_compare_action_level > current_level {
        TranscriptAnnotationDiffSource::Prediction
    } else {
        TranscriptAnnotationDiffSource::Reference
    })
}

pub fn match_annotation_diff_rows(
    reference_annotations: &[DiffAnnotationForMatch],
    prediction_annotations: &[DiffAnnotationForMatch],
) -> Vec<DiffAnnotationRow> {
    let mut rows = Vec::new();
    let mut matched_prediction_ids = BTreeSet::new();
    let reference_rows = valid_annotations_sorted_by_start(reference_annotations);
    let prediction_rows = valid_annotations_sorted_by_start(prediction_annotations);

    for reference in reference_rows {
        let mut best_prediction = None;
        let mut best_score = 0.0;

        for prediction in &prediction_rows {
            if matched_prediction_ids.contains(&prediction.id) {
                continue;
            }

            let score = annotation_overlap_score(&reference, prediction);
            if score > best_score {
                best_score = score;
                best_prediction = Some(prediction);
            }
        }

        if let Some(prediction) = best_prediction.filter(|_| best_score >= 0.3) {
            matched_prediction_ids.insert(prediction.id.clone());
            let primary_matches = reference.primary_label == prediction.primary_label;
            let secondary_matches = reference.secondary_label == prediction.secondary_label;
            let intensity_matches = reference.intensity == prediction.intensity;
            let status = if primary_matches {
                if secondary_matches {
                    if intensity_matches {
                        DiffAnnotationStatus::Matched
                    } else {
                        DiffAnnotationStatus::IntensityMismatch
                    }
                } else {
                    DiffAnnotationStatus::SubcategoryMismatch
                }
            } else {
                DiffAnnotationStatus::LabelMismatch
            };

            rows.push(DiffAnnotationRow {
                annotation: reference.clone(),
                source: TranscriptAnnotationDiffSource::Reference,
                status,
                matched_annotation_id: Some(prediction.id.clone()),
                other_label: (!primary_matches)
                    .then(|| prediction.primary_label.clone())
                    .flatten(),
                other_secondary_label: (primary_matches && !secondary_matches)
                    .then(|| prediction.secondary_label.clone())
                    .flatten(),
                other_intensity: (!intensity_matches)
                    .then_some(prediction.intensity)
                    .flatten(),
            });
            rows.push(DiffAnnotationRow {
                annotation: prediction.clone(),
                source: TranscriptAnnotationDiffSource::Prediction,
                status,
                matched_annotation_id: Some(reference.id.clone()),
                other_label: (!primary_matches)
                    .then(|| reference.primary_label.clone())
                    .flatten(),
                other_secondary_label: (primary_matches && !secondary_matches)
                    .then(|| reference.secondary_label.clone())
                    .flatten(),
                other_intensity: (!intensity_matches)
                    .then_some(reference.intensity)
                    .flatten(),
            });
        } else {
            rows.push(DiffAnnotationRow {
                annotation: reference,
                source: TranscriptAnnotationDiffSource::Reference,
                status: DiffAnnotationStatus::UnmatchedReference,
                matched_annotation_id: None,
                other_label: None,
                other_secondary_label: None,
                other_intensity: None,
            });
        }
    }

    for prediction in prediction_rows {
        if !matched_prediction_ids.contains(&prediction.id) {
            rows.push(DiffAnnotationRow {
                annotation: prediction,
                source: TranscriptAnnotationDiffSource::Prediction,
                status: DiffAnnotationStatus::UnmatchedPrediction,
                matched_annotation_id: None,
                other_label: None,
                other_secondary_label: None,
                other_intensity: None,
            });
        }
    }

    rows
}

pub fn build_annotation_status_map(
    is_diff_mode: bool,
    perspective: Option<TranscriptAnnotationDiffSource>,
    diff_rows: &[DiffAnnotationRow],
) -> Option<BTreeMap<String, DiffAnnotationStatusEntry>> {
    if !is_diff_mode {
        return None;
    }
    let perspective = perspective?;

    let spans_by_annotation_id = diff_rows
        .iter()
        .map(|row| {
            (
                row.annotation.id.clone(),
                (row.annotation.start_seconds, row.annotation.end_seconds),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut status_map = BTreeMap::new();

    for row in diff_rows.iter().filter(|row| row.source == perspective) {
        let counterpart = row
            .matched_annotation_id
            .as_ref()
            .and_then(|id| spans_by_annotation_id.get(id));
        status_map.insert(
            row.annotation.id.clone(),
            DiffAnnotationStatusEntry {
                status: match row.status {
                    DiffAnnotationStatus::UnmatchedReference
                    | DiffAnnotationStatus::UnmatchedPrediction => {
                        DiffAnnotationStatus::UnmatchedPrediction
                    }
                    status => status,
                },
                other_label: row.other_label.clone(),
                other_secondary_label: row.other_secondary_label.clone(),
                other_intensity: row.other_intensity,
                counterpart_start_seconds: counterpart.and_then(|(start, _)| *start),
                counterpart_end_seconds: counterpart.and_then(|(_, end)| *end),
            },
        );
    }

    Some(status_map)
}

fn valid_annotations_sorted_by_start(
    annotations: &[DiffAnnotationForMatch],
) -> Vec<DiffAnnotationForMatch> {
    let mut rows = annotations
        .iter()
        .filter(|annotation| annotation.start_seconds.is_some() && annotation.end_seconds.is_some())
        .cloned()
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.start_seconds
            .unwrap_or(0.0)
            .total_cmp(&right.start_seconds.unwrap_or(0.0))
    });
    rows
}

fn annotation_overlap_score(
    reference: &DiffAnnotationForMatch,
    prediction: &DiffAnnotationForMatch,
) -> f64 {
    let reference_start = reference.start_seconds.unwrap_or(0.0);
    let reference_end = reference.end_seconds.unwrap_or(reference_start);
    let prediction_start = prediction.start_seconds.unwrap_or(0.0);
    let prediction_end = prediction.end_seconds.unwrap_or(prediction_start);
    let intersection =
        (reference_end.min(prediction_end) - reference_start.max(prediction_start)).max(0.0);
    let union =
        reference_end - reference_start + (prediction_end - prediction_start) - intersection;
    if union > 0.0 {
        intersection / union
    } else {
        0.0
    }
}

fn label_review_actions(
    review_actions: &[DiffReviewActionSummary],
) -> Vec<DiffReviewActionSummary> {
    review_actions
        .iter()
        .map(|action| {
            let mut action = action.clone();
            action.label = Some(format!("L{}", action.level));
            action
        })
        .collect()
}
