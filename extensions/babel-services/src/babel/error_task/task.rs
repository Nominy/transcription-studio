//! An error-category task as Babel loads it: the review action plus its
//! `transcriptionError` comparison (`source: "real" | "synthetic"`).

use serde::Deserialize;
use serde_json::Value;

use super::diff_align::{AudioInterval, DiffSide, NormalizedDiff, utf16_len};
use crate::babel::BabelError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSource {
    /// A human vs machine transcript difference in real audio.
    Real,
    /// Text only, no audio.
    Synthetic,
}

/// A label already submitted on this diff (by L1/L2, or this worker's own).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ErrorLabel {
    pub level: Option<u32>,
    pub captures_whole_error: Option<bool>,
    pub meaning_changed: Option<bool>,
    pub human_transcript_wrong: Option<bool>,
    pub category_keys: Vec<String>,
    pub other_issues: Option<String>,
    pub notes: Option<String>,
}

/// The diff to label.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionError {
    pub id: String,
    /// Annotation (segment) id the diff belongs to.
    pub segment_id: String,
    pub language: String,
    pub source: ErrorSource,
    /// `substitution` | `deletion` | `insertion`.
    pub diff_type: String,
    pub diff: NormalizedDiff,
    pub prior_labels: Vec<ErrorLabel>,
    pub submitted_label: Option<ErrorLabel>,
    pub is_qa_review: bool,
}

impl TranscriptionError {
    /// Babel's diff-type label (`lv`); `None` for an unknown type.
    pub fn diff_type_label(&self) -> Option<&'static str> {
        match self.diff_type.as_str() {
            "substitution" => Some("Substitution"),
            "deletion" => Some("Deletion"),
            "insertion" => Some("Insertion"),
            _ => None,
        }
    }
}

/// The diff's segment and its target-speaker recording (real tasks).
#[derive(Debug, Clone, PartialEq)]
pub struct FocusSegment {
    pub annotation_id: String,
    /// Track identity: the annotation's `processedRecordingId`.
    pub processed_recording_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    /// Speaker number of the recording, when the payload lists it.
    pub speaker: Option<i64>,
    /// Local (staged) or remote audio path from `processedRecordingUriMap`.
    pub audio_path: Option<String>,
}

impl FocusSegment {
    pub fn interval(&self) -> AudioInterval {
        AudioInterval::new(self.start_seconds, self.end_seconds)
    }

    pub fn speaker_label(&self) -> String {
        self.speaker.map_or_else(
            || "Unknown Speaker".to_string(),
            |speaker| format!("Speaker {speaker}"),
        )
    }
}

/// A loaded error-category review action.
#[derive(Debug, Clone, PartialEq)]
pub struct ErrorTask {
    pub review_action_id: String,
    pub action_worker_id: Option<String>,
    pub action_decision: Option<String>,
    /// The queue the action was claimed from, when the payload says (`actionQueueId`).
    pub action_queue_id: Option<String>,
    pub error: TranscriptionError,
    /// `None` only for synthetic diffs.
    pub focus: Option<FocusSegment>,
}

impl ErrorTask {
    /// Babel's read-only rule for a viewed (not claimed) action: editable only while it is
    /// this worker's in-progress action.
    pub fn viewed_read_only(&self, worker_id: Option<&str>) -> bool {
        let own = worker_id.is_some_and(|worker| self.action_worker_id.as_deref() == Some(worker));
        !(own && self.action_decision.as_deref() == Some("in-progress"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawError {
    id: String,
    segment_id: String,
    language: String,
    source: String,
    diff_type: String,
    human_normalized_text: String,
    human_normalized_span_start: f64,
    human_normalized_span_end: f64,
    machine_normalized_text: String,
    machine_normalized_span_start: f64,
    machine_normalized_span_end: f64,
    #[serde(default)]
    prior_labels: Vec<ErrorLabel>,
    #[serde(default)]
    submitted_label: Option<ErrorLabel>,
    #[serde(default)]
    is_qa_review: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawAnnotation {
    id: String,
    #[serde(default)]
    processed_recording_id: Option<String>,
    #[serde(default)]
    start_time_in_seconds: Option<f64>,
    #[serde(default)]
    end_time_in_seconds: Option<f64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRecording {
    processed_recording_id: String,
    chunked_processed_recording_id: String,
    #[serde(default)]
    speaker: Option<i64>,
}

/// The review-action object inside a (possibly tRPC-wrapped) task payload.
pub(crate) fn review_action_data(value: &Value) -> Option<&Value> {
    if let Some(items) = value.as_array() {
        return items.iter().find_map(review_action_data);
    }
    ["/result/data/json", "/data/json", "/json"]
        .into_iter()
        .find_map(|pointer| value.pointer(pointer))
        .or_else(|| value.is_object().then_some(value))
}

/// Whether a task payload is an error-category task (`transcriptionError` present).
pub fn payload_is_error_task(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .as_ref()
        .and_then(review_action_data)
        .and_then(|data| data.get("transcriptionError"))
        .is_some_and(|error| !error.is_null())
}

fn span(value: f64, side: &str, text: &str) -> Result<usize, BabelError> {
    if value.fract() != 0.0 || value < 0.0 || value > utf16_len(text) as f64 {
        return Err(BabelError::new(format!(
            "Babel returned an invalid {side} diff span. Reload the task."
        )));
    }
    Ok(value as usize)
}

fn side(text: String, start: f64, end: f64, name: &str) -> Result<DiffSide, BabelError> {
    let start = span(start, name, &text)?;
    let end = span(end, name, &text)?;
    if end < start {
        return Err(BabelError::new(format!(
            "Babel returned an invalid {name} diff span. Reload the task."
        )));
    }
    Ok(DiffSide::new(text, start, end))
}

/// Parses an error-category task from a review-action payload. The focus segment is the
/// annotation whose id equals `segmentId`; its `processedRecordingId` selects the audio
/// (never the visual row order).
pub fn parse_error_task(payload: &str) -> Result<ErrorTask, BabelError> {
    let value: Value = serde_json::from_str(payload)
        .map_err(|error| BabelError::new(format!("The server sent an unreadable task: {error}")))?;
    let data = review_action_data(&value)
        .ok_or_else(|| BabelError::new("The server's reply had no task data."))?;
    if data.get("success").and_then(Value::as_bool) == Some(false) {
        let message = data
            .get("error")
            .and_then(Value::as_str)
            .filter(|message| !message.trim().is_empty())
            .unwrap_or("No tasks available");
        return Err(BabelError::new(message));
    }
    let string = |key: &str| {
        data.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let review_action_id =
        string("actionId").ok_or_else(|| BabelError::new("Babel task has no review action id."))?;
    let raw: RawError = serde_json::from_value(
        data.get("transcriptionError")
            .cloned()
            .ok_or_else(|| BabelError::new("This task has no transcription error to label."))?,
    )
    .map_err(|error| BabelError::new(format!("Babel returned an unreadable diff: {error}")))?;
    let source = match raw.source.as_str() {
        "real" => ErrorSource::Real,
        "synthetic" => ErrorSource::Synthetic,
        other => {
            return Err(BabelError::new(format!(
                "Unsupported transcription error source \"{other}\"."
            )));
        }
    };
    let diff = NormalizedDiff {
        human: side(
            raw.human_normalized_text,
            raw.human_normalized_span_start,
            raw.human_normalized_span_end,
            "human",
        )?,
        machine: side(
            raw.machine_normalized_text,
            raw.machine_normalized_span_start,
            raw.machine_normalized_span_end,
            "machine",
        )?,
    };
    let error = TranscriptionError {
        id: raw.id,
        segment_id: raw.segment_id,
        language: raw.language,
        source,
        diff_type: raw.diff_type,
        diff,
        prior_labels: raw.prior_labels,
        submitted_label: raw.submitted_label,
        is_qa_review: raw.is_qa_review,
    };
    let focus = match source {
        ErrorSource::Synthetic => None,
        ErrorSource::Real => Some(focus_segment(data, &error.segment_id)?),
    };
    Ok(ErrorTask {
        review_action_id,
        action_worker_id: string("actionWorkerId"),
        action_decision: string("actionDecision"),
        action_queue_id: string("actionQueueId"),
        error,
        focus,
    })
}

fn focus_segment(data: &Value, segment_id: &str) -> Result<FocusSegment, BabelError> {
    let unmatched = || BabelError::new("Couldn’t match this error to a segment. Reload the task.");
    let annotations: Vec<RawAnnotation> = data
        .get("annotations")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| BabelError::new(format!("Babel returned unreadable segments: {error}")))?
        .unwrap_or_default();
    let annotation = annotations
        .into_iter()
        .find(|annotation| annotation.id == segment_id)
        .ok_or_else(unmatched)?;
    let (Some(processed_recording_id), Some(start_seconds), Some(end_seconds)) = (
        annotation.processed_recording_id,
        annotation.start_time_in_seconds,
        annotation.end_time_in_seconds,
    ) else {
        return Err(unmatched());
    };
    if !start_seconds.is_finite() || !end_seconds.is_finite() || end_seconds <= start_seconds {
        return Err(BabelError::new(
            "Couldn’t read this diff or its audio. Reload the task.",
        ));
    }
    let recordings: Vec<RawRecording> = data
        .get("transcriptionChunkProcessedRecordings")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| BabelError::new(format!("Babel returned unreadable recordings: {error}")))?
        .unwrap_or_default();
    let recording = recordings
        .iter()
        .find(|recording| recording.processed_recording_id == processed_recording_id);
    let audio_path = recording.and_then(|recording| {
        data.get("processedRecordingUriMap")
            .and_then(|map| map.get(&recording.chunked_processed_recording_id))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    Ok(FocusSegment {
        annotation_id: annotation.id,
        processed_recording_id,
        start_seconds,
        end_seconds,
        speaker: recording.and_then(|recording| recording.speaker),
        audio_path,
    })
}
