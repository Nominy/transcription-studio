use serde_json::Value;
use studio_core::{Project, Segment};

use super::{
    AnnotationDiffStatus, TranscriptAnnotationDiffView, TranscriptDiffCellKind, TranscriptDiffView,
    TranscriptWordDiff, TranscriptWordDiffStatus,
};

const META_ORIGINAL_TEXT: &str = "studio.babel.original.text";
const META_ORIGINAL_START_SECONDS: &str = "studio.babel.original.startSeconds";
const META_ORIGINAL_END_SECONDS: &str = "studio.babel.original.endSeconds";
const META_ORIGINAL_SPEAKER_ID: &str = "studio.babel.original.speakerId";
const META_ORIGINAL_LANE_ID: &str = "studio.babel.original.laneId";
const META_DIFF_WORD_DIFFS: &str = "studio.babel.diff.wordDiffs";
const META_COMPARE_SNAPSHOTS: &str = "studio.babel.diff.compareSnapshots";
const META_DIFF_RESULT: &str = "studio.babel.diff.result";
const META_DIFF_IS_COMPARE_NEWER: &str = "studio.babel.diff.isCompareNewer";
const META_ANNOTATION_GHOST_ROWS: &str = "studio.babel.diff.annotationGhostRows";
const DIFF_EPSILON_SECONDS: f64 = 0.000_001;

#[derive(Clone, Debug)]
pub(super) struct DiffState {
    pub marker: String,
    pub cell: TranscriptDiffView,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ModifiedSegmentDiffRow {
    pub annotation_id: String,
    pub lane_id: Option<String>,
    pub speaker_id: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub show_as_added: bool,
    pub cell: TranscriptDiffView,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AnnotationRemovedGhostRow {
    pub annotation_id: String,
    pub comparison_label: String,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
    pub text: String,
    pub diff: TranscriptAnnotationDiffView,
}

pub(super) fn diff_state(segment: &Segment, selected_compare_action_id: Option<&str>) -> DiffState {
    if segment.source_annotation_id.is_none() {
        return DiffState {
            marker: "New".to_string(),
            cell: build_new_cell(),
        };
    }

    let Some(snapshot) = selected_compare_action_id
        .and_then(|action_id| OriginalSnapshot::for_compare_action(segment, action_id))
        .or_else(|| OriginalSnapshot::for_segment(segment))
    else {
        return DiffState {
            marker: match segment.source_annotation_id.as_deref() {
                Some(source_annotation_id) if source_annotation_id == segment.id => {
                    "Babel source".to_string()
                }
                Some(source_annotation_id) => format!("Babel source {source_annotation_id}"),
                None => "New".to_string(),
            },
            cell: build_diff_cell(segment, None, false),
        };
    };

    let text_changed = snapshot.text.as_deref() != Some(segment.text.as_str());
    let timing_changed = snapshot.start_seconds.is_some_and(|start_seconds| {
        (start_seconds - segment.start_seconds).abs() > DIFF_EPSILON_SECONDS
    }) || snapshot.end_seconds.is_some_and(|end_seconds| {
        (end_seconds - segment.end_seconds).abs() > DIFF_EPSILON_SECONDS
    });
    let track_changed = snapshot
        .speaker_id
        .as_deref()
        .is_some_and(|speaker_id| speaker_id != segment.speaker_id)
        || snapshot
            .lane_id
            .as_deref()
            .is_some_and(|lane_id| lane_id != segment.lane_id);

    let mut changes = Vec::new();
    if text_changed {
        changes.push("text");
    }
    if timing_changed {
        changes.push("timing");
    }
    if track_changed {
        changes.push("track");
    }

    if changes.is_empty() {
        return DiffState {
            marker: "Babel unchanged".to_string(),
            cell: build_diff_cell(segment, Some(&snapshot), false),
        };
    }

    DiffState {
        marker: format!("Changed: {}", changes.join(", ")),
        cell: build_diff_cell(segment, Some(&snapshot), text_changed),
    }
}

pub(super) fn modified_segment_diff_rows(project: &Project) -> Vec<ModifiedSegmentDiffRow> {
    let Some(raw) = project.metadata.get(META_DIFF_RESULT) else {
        return Vec::new();
    };
    let Ok(diff_result) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let is_compare_newer = project
        .metadata
        .get(META_DIFF_IS_COMPARE_NEWER)
        .is_some_and(|value| value == "true" || value == "1");

    let Some(speaker_diffs) = diff_result.get("speakerDiffs").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for speaker_diff in speaker_diffs {
        let Some(mappings) = speaker_diff
            .get("segmentMappings")
            .and_then(Value::as_array)
        else {
            continue;
        };

        for mapping in mappings {
            let relationship = mapping
                .get("relationship")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let segments = if is_compare_newer {
                if relationship != "added" || !value_array_is_empty(mapping.get("segmentsA")) {
                    continue;
                }
                mapping.get("segmentsB").and_then(Value::as_array)
            } else {
                if relationship != "deleted" || !value_array_is_empty(mapping.get("segmentsB")) {
                    continue;
                }
                mapping.get("segmentsA").and_then(Value::as_array)
            };
            let Some(segments) = segments else {
                continue;
            };
            let word_diffs = mapping
                .get("wordDiffs")
                .and_then(word_diffs_from_value)
                .unwrap_or_default();

            for segment in segments {
                let Some(annotation_id) =
                    value_string(segment, &["annotationId", "annotation_id", "id"])
                else {
                    continue;
                };
                let start_seconds = value_seconds(
                    segment,
                    &["startSeconds", "startTimeInSeconds", "start_seconds"],
                );
                let end_seconds =
                    value_seconds(segment, &["endSeconds", "endTimeInSeconds", "end_seconds"]);
                let lane_id = value_string(segment, &["laneId", "trackId", "track_id"]);
                let speaker_id = value_string(segment, &["speakerId", "speakerKey", "speaker_key"]);
                rows.push(ModifiedSegmentDiffRow {
                    annotation_id,
                    lane_id,
                    speaker_id,
                    start_seconds,
                    end_seconds,
                    show_as_added: is_compare_newer,
                    cell: build_modified_segment_cell(word_diffs.clone(), is_compare_newer),
                });
            }
        }
    }

    rows.sort_by(|left, right| {
        left.start_seconds
            .unwrap_or(0.0)
            .total_cmp(&right.start_seconds.unwrap_or(0.0))
    });
    rows
}

pub(super) fn annotation_removed_ghost_rows(project: &Project) -> Vec<AnnotationRemovedGhostRow> {
    let Some(raw) = project.metadata.get(META_ANNOTATION_GHOST_ROWS) else {
        return Vec::new();
    };
    let Ok(Value::Array(rows)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };

    let mut ghost_rows = rows
        .iter()
        .filter_map(|row| {
            let annotation = row.get("annotation").unwrap_or(row);
            let annotation_id = value_string(annotation, &["id", "annotationId"])?;
            let comparison_label = row
                .get("comparisonLabel")
                .and_then(Value::as_str)
                .unwrap_or("Reference")
                .to_string();
            let start_seconds = value_seconds(
                annotation,
                &["startSeconds", "startTimeInSeconds", "start_seconds"],
            );
            let end_seconds = value_seconds(
                annotation,
                &["endSeconds", "endTimeInSeconds", "end_seconds"],
            );
            let intensity = value_u8(annotation, &["intensity"]);

            Some(AnnotationRemovedGhostRow {
                annotation_id,
                comparison_label,
                start_seconds,
                end_seconds,
                text: removed_annotation_text(annotation.get("content"), intensity),
                diff: TranscriptAnnotationDiffView {
                    status: AnnotationDiffStatus::UnmatchedReference,
                    other_label: None,
                    other_secondary_label: None,
                    other_intensity: None,
                    counterpart_start_seconds: start_seconds,
                    counterpart_end_seconds: end_seconds,
                },
            })
        })
        .collect::<Vec<_>>();

    ghost_rows.sort_by(|left, right| {
        left.start_seconds
            .unwrap_or(0.0)
            .total_cmp(&right.start_seconds.unwrap_or(0.0))
    });
    ghost_rows
}

pub(super) fn annotation_diff_state(
    segment: &Segment,
    selected_compare_action_id: Option<&str>,
) -> Option<TranscriptAnnotationDiffView> {
    if segment
        .metadata
        .get("type")
        .and_then(Value::as_str)
        .is_none_or(|annotation_type| annotation_type == "transcription")
    {
        return None;
    }

    let snapshot = selected_compare_action_id
        .and_then(|action_id| OriginalSnapshot::for_compare_action(segment, action_id))
        .or_else(|| OriginalSnapshot::for_segment(segment))?;
    let reference_content = snapshot.content.as_ref()?;
    let prediction_content = segment.metadata.get("content")?;
    let reference_primary = extract_primary_label(reference_content);
    let prediction_primary = extract_primary_label(prediction_content);
    let reference_secondary = extract_secondary_label(reference_content);
    let prediction_secondary = extract_secondary_label(prediction_content);
    let prediction_intensity = metadata_u8(segment, "intensity");

    let primary_matches = reference_primary == prediction_primary;
    let secondary_matches = reference_secondary == prediction_secondary;
    let intensity_matches = snapshot.intensity == prediction_intensity;
    let status = if !primary_matches {
        AnnotationDiffStatus::LabelMismatch
    } else if !secondary_matches {
        AnnotationDiffStatus::SubcategoryMismatch
    } else if !intensity_matches {
        AnnotationDiffStatus::IntensityMismatch
    } else {
        AnnotationDiffStatus::Matched
    };

    Some(TranscriptAnnotationDiffView {
        status,
        other_label: (!primary_matches)
            .then(|| reference_primary.clone())
            .flatten(),
        other_secondary_label: (primary_matches && !secondary_matches)
            .then(|| reference_secondary.clone())
            .flatten(),
        other_intensity: (!intensity_matches).then_some(snapshot.intensity).flatten(),
        counterpart_start_seconds: snapshot.start_seconds,
        counterpart_end_seconds: snapshot.end_seconds,
    })
}

#[derive(Clone, Debug)]
struct OriginalSnapshot {
    text: Option<String>,
    content: Option<Value>,
    intensity: Option<u8>,
    start_seconds: Option<f64>,
    end_seconds: Option<f64>,
    speaker_id: Option<String>,
    lane_id: Option<String>,
    word_diffs: Option<Vec<TranscriptWordDiff>>,
}

impl OriginalSnapshot {
    fn for_segment(segment: &Segment) -> Option<Self> {
        let snapshot = Self {
            text: metadata_string(segment, META_ORIGINAL_TEXT),
            content: segment
                .metadata
                .get("studio.babel.original.content")
                .cloned(),
            intensity: metadata_u8(segment, "studio.babel.original.intensity"),
            start_seconds: metadata_seconds(segment, META_ORIGINAL_START_SECONDS),
            end_seconds: metadata_seconds(segment, META_ORIGINAL_END_SECONDS),
            speaker_id: metadata_string(segment, META_ORIGINAL_SPEAKER_ID),
            lane_id: metadata_string(segment, META_ORIGINAL_LANE_ID),
            word_diffs: None,
        };

        snapshot.has_any_data().then_some(snapshot)
    }

    fn for_compare_action(segment: &Segment, action_id: &str) -> Option<Self> {
        let snapshots = segment.metadata.get(META_COMPARE_SNAPSHOTS)?;
        let snapshot = match snapshots {
            Value::Object(object) => object.get(action_id).or_else(|| {
                object
                    .get("items")
                    .and_then(|items| find_snapshot(items, action_id))
            })?,
            Value::Array(_) => find_snapshot(snapshots, action_id)?,
            _ => return None,
        };

        Self::from_value(snapshot)
    }

    fn from_value(value: &Value) -> Option<Self> {
        let snapshot = Self {
            text: value_string(value, &["text", "content"]),
            content: value.get("content").cloned(),
            intensity: value_u8(value, &["intensity"]),
            start_seconds: value_seconds(
                value,
                &["startSeconds", "startTimeInSeconds", "start_seconds"],
            ),
            end_seconds: value_seconds(value, &["endSeconds", "endTimeInSeconds", "end_seconds"]),
            speaker_id: value_string(value, &["speakerId", "speakerKey", "speaker_key"]),
            lane_id: value_string(value, &["laneId", "trackId", "track_id"]),
            word_diffs: value.get("wordDiffs").and_then(word_diffs_from_value),
        };

        snapshot.has_any_data().then_some(snapshot)
    }

    fn has_any_data(&self) -> bool {
        self.text.is_some()
            || self.content.is_some()
            || self.intensity.is_some()
            || self.start_seconds.is_some()
            || self.end_seconds.is_some()
            || self.speaker_id.is_some()
            || self.lane_id.is_some()
            || self.word_diffs.is_some()
    }
}

fn find_snapshot<'a>(value: &'a Value, action_id: &str) -> Option<&'a Value> {
    let Value::Array(items) = value else {
        return None;
    };

    items.iter().find(|item| {
        ["id", "actionId", "reviewActionId"]
            .iter()
            .any(|key| item.get(key).and_then(Value::as_str) == Some(action_id))
    })
}

fn metadata_string(segment: &Segment, key: &str) -> Option<String> {
    segment
        .metadata
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
}

fn metadata_seconds(segment: &Segment, key: &str) -> Option<f64> {
    segment
        .metadata
        .get(key)
        .and_then(serde_json::Value::as_f64)
}

fn metadata_u8(segment: &Segment, key: &str) -> Option<u8> {
    segment
        .metadata
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u8::try_from(value).ok())
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_str))
        .map(ToString::to_string)
}

fn value_seconds(value: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_f64))
}

fn value_u8(value: &Value, keys: &[&str]) -> Option<u8> {
    keys.iter()
        .find_map(|key| value.get(key).and_then(Value::as_u64))
        .and_then(|value| u8::try_from(value).ok())
}

fn value_array_is_empty(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_array)
        .is_none_or(|items| items.is_empty())
}

fn extract_primary_label(content: &Value) -> Option<String> {
    content
        .get("primaryLabel")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_secondary_label(content: &Value) -> Option<String> {
    content
        .get("secondaryLabel")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn removed_annotation_text(content: Option<&Value>, intensity: Option<u8>) -> String {
    let mut label = content
        .and_then(|content| {
            let primary = extract_primary_label(content);
            let secondary = extract_secondary_label(content);
            match (primary, secondary) {
                (Some(primary), Some(secondary)) => Some(format!("{primary} / {secondary}")),
                (Some(primary), None) => Some(primary),
                (None, Some(secondary)) => Some(secondary),
                (None, None) => content.as_str().map(ToString::to_string),
            }
        })
        .unwrap_or_else(|| "Removed annotation".to_string());

    if let Some(intensity) = intensity {
        label.push_str(&format!(" \u{00b7} intensity {intensity}"));
    }
    label
}

fn build_diff_cell(
    segment: &Segment,
    snapshot: Option<&OriginalSnapshot>,
    text_changed: bool,
) -> TranscriptDiffView {
    let source_text = text_changed
        .then(|| snapshot.and_then(|snapshot| snapshot.text.clone()))
        .flatten();
    let word_diffs = snapshot
        .and_then(|snapshot| snapshot.word_diffs.clone())
        .or_else(|| metadata_word_diffs(segment))
        .unwrap_or_else(|| match source_text.as_deref() {
            Some(original_text) => infer_word_diffs(original_text, &segment.text),
            None => unchanged_word_diffs(&segment.text),
        });

    TranscriptDiffView {
        kind: TranscriptDiffCellKind::Diff,
        change_count: count_word_diff_changes(&word_diffs),
        source_text,
        words: word_diffs,
    }
}

fn build_new_cell() -> TranscriptDiffView {
    TranscriptDiffView {
        kind: TranscriptDiffCellKind::New,
        change_count: 0,
        source_text: None,
        words: Vec::new(),
    }
}

fn build_modified_segment_cell(
    word_diffs: Vec<TranscriptWordDiff>,
    show_as_added: bool,
) -> TranscriptDiffView {
    TranscriptDiffView {
        kind: if show_as_added {
            TranscriptDiffCellKind::ModifiedAdded
        } else {
            TranscriptDiffCellKind::ModifiedDeleted
        },
        change_count: count_word_diff_changes(&word_diffs),
        source_text: None,
        words: word_diffs,
    }
}

fn metadata_word_diffs(segment: &Segment) -> Option<Vec<TranscriptWordDiff>> {
    word_diffs_from_value(segment.metadata.get(META_DIFF_WORD_DIFFS)?)
}

fn word_diffs_from_value(value: &Value) -> Option<Vec<TranscriptWordDiff>> {
    let Value::Array(items) = value else {
        return None;
    };

    Some(items.iter().map(word_diff_from_value).collect())
}

fn word_diff_from_value(value: &Value) -> TranscriptWordDiff {
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .map(word_diff_status)
        .unwrap_or(TranscriptWordDiffStatus::Unknown);
    let value = value
        .get("value")
        .map(format_word_value)
        .unwrap_or_default();

    TranscriptWordDiff { status, value }
}

fn word_diff_status(status: &str) -> TranscriptWordDiffStatus {
    match status {
        "unchanged" => TranscriptWordDiffStatus::Unchanged,
        "added" => TranscriptWordDiffStatus::Added,
        "removed" => TranscriptWordDiffStatus::Removed,
        _ => TranscriptWordDiffStatus::Unknown,
    }
}

fn format_word_value(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        value => value.to_string(),
    }
}

fn count_word_diff_changes(word_diffs: &[TranscriptWordDiff]) -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < word_diffs.len() {
        let diff = &word_diffs[index];
        if diff.status == TranscriptWordDiffStatus::Removed {
            let next = word_diffs.get(index + 1);
            if next.is_some_and(|next| next.status == TranscriptWordDiffStatus::Added) {
                count += 1;
                index += 2;
            } else {
                count += 1;
                index += 1;
            }
        } else {
            if diff.status == TranscriptWordDiffStatus::Added {
                count += 1;
            }
            index += 1;
        }
    }
    count
}

fn unchanged_word_diffs(text: &str) -> Vec<TranscriptWordDiff> {
    split_words(text)
        .into_iter()
        .map(|value| TranscriptWordDiff {
            status: TranscriptWordDiffStatus::Unchanged,
            value,
        })
        .collect()
}

fn infer_word_diffs(original_text: &str, current_text: &str) -> Vec<TranscriptWordDiff> {
    let original = split_words(original_text);
    let current = split_words(current_text);
    let mut lcs = vec![vec![0usize; current.len() + 1]; original.len() + 1];

    for original_index in (0..original.len()).rev() {
        for current_index in (0..current.len()).rev() {
            lcs[original_index][current_index] = if original[original_index]
                == current[current_index]
            {
                1 + lcs[original_index + 1][current_index + 1]
            } else {
                lcs[original_index + 1][current_index].max(lcs[original_index][current_index + 1])
            };
        }
    }

    let mut result = Vec::new();
    let mut original_index = 0;
    let mut current_index = 0;

    while original_index < original.len() && current_index < current.len() {
        if original[original_index] == current[current_index] {
            result.push(TranscriptWordDiff {
                status: TranscriptWordDiffStatus::Unchanged,
                value: current[current_index].clone(),
            });
            original_index += 1;
            current_index += 1;
        } else if lcs[original_index + 1][current_index] >= lcs[original_index][current_index + 1] {
            result.push(TranscriptWordDiff {
                status: TranscriptWordDiffStatus::Removed,
                value: original[original_index].clone(),
            });
            original_index += 1;
        } else {
            result.push(TranscriptWordDiff {
                status: TranscriptWordDiffStatus::Added,
                value: current[current_index].clone(),
            });
            current_index += 1;
        }
    }

    while original_index < original.len() {
        result.push(TranscriptWordDiff {
            status: TranscriptWordDiffStatus::Removed,
            value: original[original_index].clone(),
        });
        original_index += 1;
    }

    while current_index < current.len() {
        result.push(TranscriptWordDiff {
            status: TranscriptWordDiffStatus::Added,
            value: current[current_index].clone(),
        });
        current_index += 1;
    }

    result
}

fn split_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(ToString::to_string).collect()
}
