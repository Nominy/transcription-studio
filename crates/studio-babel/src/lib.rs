use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use studio_core::{AudioLane, Project, Segment, Speaker};
use thiserror::Error;

const META_PROJECT_PRESET: &str = "project_preset";
const META_FORMAT: &str = "babel.format";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";
pub const META_REVIEW_ACTION_DECISION: &str = "babel.review_action.actionDecision";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_EXTRA_SPEAKER_PREFIX: &str = "babel.extra.speaker.";
const META_EXTRA_TRACK_PREFIX: &str = "babel.extra.track.";
const META_EXTRA_ANNOTATION_PREFIX: &str = "babel.extra.annotation.";
const META_ORIGINAL_PREFIX: &str = "studio.babel.original.";
const META_ORIGINAL_TEXT: &str = "studio.babel.original.text";
const META_ORIGINAL_START_SECONDS: &str = "studio.babel.original.startSeconds";
const META_ORIGINAL_END_SECONDS: &str = "studio.babel.original.endSeconds";
const META_ORIGINAL_SPEAKER_ID: &str = "studio.babel.original.speakerId";
const META_ORIGINAL_LANE_ID: &str = "studio.babel.original.laneId";
const META_NATIVE_PROJECT_PREFIX: &str = "studio.babel.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BabelTaskPackage {
    pub task_id: String,
    pub project_preset: String,
    pub speakers: Vec<BabelSpeaker>,
    pub audio_tracks: Vec<BabelAudioTrack>,
    pub annotations: Vec<BabelAnnotation>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BabelSpeaker {
    pub speaker_key: String,
    pub label: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BabelAudioTrack {
    pub track_id: String,
    pub speaker_key: String,
    pub path: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BabelAnnotation {
    pub annotation_id: String,
    pub speaker_key: String,
    pub track_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BabelImportError {
    #[error("duplicate speaker key `{0}`")]
    DuplicateSpeaker(String),
    #[error("duplicate audio track id `{0}`")]
    DuplicateTrack(String),
    #[error("duplicate annotation id `{0}`")]
    DuplicateAnnotation(String),
    #[error("annotation references unknown speaker `{0}`")]
    UnknownSpeaker(String),
    #[error("annotation references unknown audio track `{0}`")]
    UnknownTrack(String),
    #[error("audio track `{track_id}` references unknown speaker `{speaker_key}`")]
    UnknownTrackSpeaker {
        track_id: String,
        speaker_key: String,
    },
    #[error(
        "annotation `{annotation_id}` speaker `{speaker_key}` does not match track `{track_id}` speaker `{track_speaker_key}`"
    )]
    AnnotationTrackSpeakerMismatch {
        annotation_id: String,
        speaker_key: String,
        track_id: String,
        track_speaker_key: String,
    },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BabelExportError {
    #[error("project metadata is missing project_preset")]
    MissingProjectPreset,
    #[error("project metadata is missing review-action envelope")]
    MissingReviewActionEnvelope,
    #[error("project metadata contains invalid review-action envelope")]
    InvalidReviewActionEnvelope,
    #[error("review-action envelope does not contain result.data.json")]
    MissingReviewActionData,
}

#[derive(Clone, Debug, Error)]
pub enum BabelJsonImportError {
    #[error("failed to decode Babel JSON as task package or review action envelope")]
    UnsupportedShape,
    #[error("failed to import task package: {0}")]
    Package(#[from] BabelImportError),
    #[error("review action envelope does not contain result.data.json")]
    MissingReviewActionData,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewActionData {
    action_id: String,
    action_level: i64,
    #[serde(default)]
    action_worker_id: Option<String>,
    #[serde(default)]
    action_decision: Option<String>,
    #[serde(default)]
    is_worker_resuming_claim: Option<bool>,
    #[serde(default)]
    processed_recording_uri_map: BTreeMap<String, String>,
    transcription_chunk_processed_recordings: Vec<ReviewProcessedRecording>,
    annotations: Vec<ReviewAnnotation>,
    #[serde(default)]
    lint_errors: Vec<Value>,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewProcessedRecording {
    transcription_chunk_id: String,
    chunked_processed_recording_id: String,
    processed_recording_id: String,
    speaker: u32,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReviewAnnotation {
    id: String,
    review_action_id: String,
    #[serde(rename = "type")]
    annotation_type: String,
    content: Value,
    processed_recording_id: String,
    start_time_in_seconds: f64,
    end_time_in_seconds: f64,
    #[serde(default)]
    intensity: Option<Value>,
    #[serde(default)]
    metadata: Option<Value>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default, flatten)]
    extra: BTreeMap<String, Value>,
}

pub fn import_json(source: &str) -> Result<Project, BabelJsonImportError> {
    if let Ok(package) = serde_json::from_str::<BabelTaskPackage>(source) {
        return import_package(package).map_err(BabelJsonImportError::Package);
    }

    let raw_envelope: Value =
        serde_json::from_str(source).map_err(|_| BabelJsonImportError::UnsupportedShape)?;
    let review_action_value = review_action_data_value(&raw_envelope)
        .cloned()
        .ok_or(BabelJsonImportError::MissingReviewActionData)?;
    let review_action: ReviewActionData = serde_json::from_value(review_action_value)
        .map_err(|_| BabelJsonImportError::UnsupportedShape)?;
    let mut project = import_review_action(review_action)?;
    project.metadata.insert(
        META_REVIEW_ACTION_ENVELOPE.to_string(),
        serde_json::to_string(&raw_envelope).expect("review-action envelope should encode"),
    );
    Ok(project)
}

pub fn import_package(package: BabelTaskPackage) -> Result<Project, BabelImportError> {
    validate_unique_ids(&package)?;

    let speaker_labels_by_key: BTreeMap<_, _> = package
        .speakers
        .iter()
        .map(|speaker| (speaker.speaker_key.clone(), speaker.label.clone()))
        .collect();
    let track_speakers_by_id: BTreeMap<_, _> = package
        .audio_tracks
        .iter()
        .map(|track| (track.track_id.clone(), track.speaker_key.clone()))
        .collect();

    for track in &package.audio_tracks {
        if !speaker_labels_by_key.contains_key(&track.speaker_key) {
            return Err(BabelImportError::UnknownTrackSpeaker {
                track_id: track.track_id.clone(),
                speaker_key: track.speaker_key.clone(),
            });
        }
    }

    for annotation in &package.annotations {
        if !speaker_labels_by_key.contains_key(&annotation.speaker_key) {
            return Err(BabelImportError::UnknownSpeaker(
                annotation.speaker_key.clone(),
            ));
        }
        let track_speaker_key = track_speakers_by_id
            .get(&annotation.track_id)
            .ok_or_else(|| BabelImportError::UnknownTrack(annotation.track_id.clone()))?;
        if track_speaker_key != &annotation.speaker_key {
            return Err(BabelImportError::AnnotationTrackSpeakerMismatch {
                annotation_id: annotation.annotation_id.clone(),
                speaker_key: annotation.speaker_key.clone(),
                track_id: annotation.track_id.clone(),
                track_speaker_key: track_speaker_key.clone(),
            });
        }
    }

    let mut metadata = BTreeMap::new();
    metadata.insert(
        META_PROJECT_PRESET.to_string(),
        package.project_preset.clone(),
    );
    store_extra(&mut metadata, META_EXTRA_PACKAGE, &package.extra);
    promote_native_project_metadata(&mut metadata, &package.extra);
    for speaker in &package.speakers {
        store_extra(
            &mut metadata,
            &metadata_key(META_EXTRA_SPEAKER_PREFIX, &speaker.speaker_key),
            &speaker.extra,
        );
    }
    for track in &package.audio_tracks {
        store_extra(
            &mut metadata,
            &metadata_key(META_EXTRA_TRACK_PREFIX, &track.track_id),
            &track.extra,
        );
    }
    for annotation in &package.annotations {
        store_extra(
            &mut metadata,
            &metadata_key(META_EXTRA_ANNOTATION_PREFIX, &annotation.annotation_id),
            &annotation.extra,
        );
    }

    Ok(Project {
        name: format!("Babel task {}", package.task_id),
        id: package.task_id,
        speakers: package
            .speakers
            .into_iter()
            .map(|speaker| Speaker {
                id: speaker.speaker_key,
                label: speaker.label,
            })
            .collect(),
        lanes: package
            .audio_tracks
            .into_iter()
            .map(|track| AudioLane {
                id: track.track_id,
                label: speaker_labels_by_key
                    .get(&track.speaker_key)
                    .cloned()
                    .unwrap_or_else(|| track.path.clone()),
                speaker_id: track.speaker_key,
                source_path: track.path,
            })
            .collect(),
        segments: package
            .annotations
            .into_iter()
            .map(|annotation| {
                let mut metadata = segment_metadata_from_extra(&annotation.extra);
                store_original_annotation_snapshot(&mut metadata, &annotation);
                Segment {
                    id: annotation.annotation_id.clone(),
                    source_annotation_id: Some(annotation.annotation_id),
                    speaker_id: annotation.speaker_key,
                    lane_id: annotation.track_id,
                    start_seconds: annotation.start_seconds,
                    end_seconds: annotation.end_seconds,
                    text: annotation.text,
                    metadata,
                }
            })
            .collect(),
        metadata,
    })
}

fn import_review_action(data: ReviewActionData) -> Result<Project, BabelJsonImportError> {
    let action_decision = data.action_decision.clone();
    let mut recordings = data.transcription_chunk_processed_recordings;
    recordings.sort_by_key(|recording| recording.speaker);

    let task_id = recordings
        .first()
        .map(|recording| recording.transcription_chunk_id.clone())
        .unwrap_or_else(|| data.action_id.clone());
    let mut speaker_numbers: Vec<_> = recordings
        .iter()
        .map(|recording| recording.speaker)
        .collect();
    speaker_numbers.sort_unstable();
    speaker_numbers.dedup();

    let speakers = speaker_numbers
        .iter()
        .map(|speaker| BabelSpeaker {
            speaker_key: speaker_key(*speaker),
            label: format!("Speaker {speaker}"),
            extra: BTreeMap::new(),
        })
        .collect();

    let audio_tracks = recordings
        .iter()
        .map(|recording| {
            let mut extra = recording.extra.clone();
            extra.insert(
                "processed_recording_id".to_string(),
                json!(recording.processed_recording_id),
            );
            extra.insert(
                "chunked_processed_recording_id".to_string(),
                json!(recording.chunked_processed_recording_id),
            );
            extra.insert(
                "transcription_chunk_id".to_string(),
                json!(recording.transcription_chunk_id),
            );
            BabelAudioTrack {
                track_id: track_id(recording.speaker),
                speaker_key: speaker_key(recording.speaker),
                path: data
                    .processed_recording_uri_map
                    .get(&recording.chunked_processed_recording_id)
                    .cloned()
                    .unwrap_or_else(|| format!("audio/speaker_{}.wav", recording.speaker)),
                extra,
            }
        })
        .collect();

    let speakers_by_processed_recording: BTreeMap<_, _> = recordings
        .iter()
        .map(|recording| (recording.processed_recording_id.clone(), recording.speaker))
        .collect();

    let annotations = data
        .annotations
        .into_iter()
        .map(|annotation| {
            let speaker = speakers_by_processed_recording
                .get(&annotation.processed_recording_id)
                .copied()
                .unwrap_or(1);
            let mut extra = annotation.extra;
            extra.insert(
                "reviewActionId".to_string(),
                json!(annotation.review_action_id),
            );
            extra.insert("type".to_string(), json!(annotation.annotation_type));
            extra.insert(
                "processedRecordingId".to_string(),
                json!(annotation.processed_recording_id),
            );
            extra.insert("intensity".to_string(), json!(annotation.intensity));
            extra.insert("metadata".to_string(), json!(annotation.metadata));
            extra.insert("createdAt".to_string(), json!(annotation.created_at));
            extra.insert("updatedAt".to_string(), json!(annotation.updated_at));
            if !annotation.content.is_string() {
                extra.insert("content".to_string(), annotation.content.clone());
            }

            BabelAnnotation {
                annotation_id: annotation.id,
                speaker_key: speaker_key(speaker),
                track_id: track_id(speaker),
                start_seconds: annotation.start_time_in_seconds,
                end_seconds: annotation.end_time_in_seconds,
                text: review_annotation_text(&annotation.content),
                extra,
            }
        })
        .collect();

    let mut extra = data.extra;
    extra.insert("format".to_string(), json!("review-action"));
    extra.insert("action_id".to_string(), json!(data.action_id));
    extra.insert("action_level".to_string(), json!(data.action_level));
    extra.insert("action_worker_id".to_string(), json!(data.action_worker_id));
    extra.insert("action_decision".to_string(), json!(data.action_decision));
    extra.insert(
        "is_worker_resuming_claim".to_string(),
        json!(data.is_worker_resuming_claim),
    );
    extra.insert("lint_errors".to_string(), json!(data.lint_errors));

    let package = BabelTaskPackage {
        task_id,
        project_preset: "babel-review-action".to_string(),
        speakers,
        audio_tracks,
        annotations,
        extra,
    };
    let mut project = import_package(package)?;
    project
        .metadata
        .insert(META_FORMAT.to_string(), "review-action".to_string());
    if let Some(action_decision) = action_decision {
        project
            .metadata
            .insert(META_REVIEW_ACTION_DECISION.to_string(), action_decision);
    }
    Ok(project)
}

fn speaker_key(speaker: u32) -> String {
    format!("speaker-{speaker}")
}

fn track_id(speaker: u32) -> String {
    format!("track-speaker-{speaker}")
}

fn validate_unique_ids(package: &BabelTaskPackage) -> Result<(), BabelImportError> {
    let mut speaker_keys = BTreeSet::new();
    for speaker in &package.speakers {
        if !speaker_keys.insert(speaker.speaker_key.as_str()) {
            return Err(BabelImportError::DuplicateSpeaker(
                speaker.speaker_key.clone(),
            ));
        }
    }

    let mut track_ids = BTreeSet::new();
    for track in &package.audio_tracks {
        if !track_ids.insert(track.track_id.as_str()) {
            return Err(BabelImportError::DuplicateTrack(track.track_id.clone()));
        }
    }

    let mut annotation_ids = BTreeSet::new();
    for annotation in &package.annotations {
        if !annotation_ids.insert(annotation.annotation_id.as_str()) {
            return Err(BabelImportError::DuplicateAnnotation(
                annotation.annotation_id.clone(),
            ));
        }
    }

    Ok(())
}

pub fn export_package(project: &Project) -> Result<BabelTaskPackage, BabelExportError> {
    let project_preset = project
        .metadata
        .get(META_PROJECT_PRESET)
        .cloned()
        .ok_or(BabelExportError::MissingProjectPreset)?;

    Ok(BabelTaskPackage {
        task_id: project.id.clone(),
        project_preset,
        extra: load_extra(&project.metadata, META_EXTRA_PACKAGE),
        speakers: project
            .speakers
            .iter()
            .map(|speaker| BabelSpeaker {
                speaker_key: speaker.id.clone(),
                label: speaker.label.clone(),
                extra: load_extra(
                    &project.metadata,
                    &metadata_key(META_EXTRA_SPEAKER_PREFIX, &speaker.id),
                ),
            })
            .collect(),
        audio_tracks: project
            .lanes
            .iter()
            .map(|lane| BabelAudioTrack {
                track_id: lane.id.clone(),
                speaker_key: lane.speaker_id.clone(),
                path: lane.source_path.clone(),
                extra: load_extra(
                    &project.metadata,
                    &metadata_key(META_EXTRA_TRACK_PREFIX, &lane.id),
                ),
            })
            .collect(),
        annotations: project
            .segments
            .iter()
            .map(|segment| {
                let annotation_id = segment
                    .source_annotation_id
                    .clone()
                    .unwrap_or_else(|| segment.id.clone());
                let mut extra = load_extra(
                    &project.metadata,
                    &metadata_key(META_EXTRA_ANNOTATION_PREFIX, &annotation_id),
                );
                store_segment_top_level_fields_in_extra(&mut extra, &segment.metadata);
                store_segment_metadata_in_extra(&mut extra, &segment.metadata);
                BabelAnnotation {
                    extra,
                    annotation_id,
                    speaker_key: segment.speaker_id.clone(),
                    track_id: segment.lane_id.clone(),
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                    text: segment.text.clone(),
                }
            })
            .collect(),
    })
}

pub fn export_json(project: &Project) -> Result<Value, BabelExportError> {
    if project.metadata.get(META_FORMAT).map(String::as_str) == Some("review-action") {
        return export_review_action_json(project);
    }

    let package = export_package(project)?;
    Ok(serde_json::to_value(package).expect("Babel package should encode as JSON value"))
}

fn export_review_action_json(project: &Project) -> Result<Value, BabelExportError> {
    let encoded = project
        .metadata
        .get(META_REVIEW_ACTION_ENVELOPE)
        .ok_or(BabelExportError::MissingReviewActionEnvelope)?;
    let mut envelope: Value =
        serde_json::from_str(encoded).map_err(|_| BabelExportError::InvalidReviewActionEnvelope)?;
    let data_snapshot = review_action_data_value(&envelope)
        .cloned()
        .ok_or(BabelExportError::MissingReviewActionData)?;
    let annotations = build_review_action_annotations(project, &data_snapshot);
    let data = review_action_data_value_mut(&mut envelope)
        .ok_or(BabelExportError::MissingReviewActionData)?;
    let data_object = data
        .as_object_mut()
        .ok_or(BabelExportError::InvalidReviewActionEnvelope)?;
    data_object.insert("annotations".to_string(), Value::Array(annotations));
    if let Some(decision) = project.metadata.get(META_REVIEW_ACTION_DECISION) {
        data_object.insert("actionDecision".to_string(), json!(decision));
    }
    Ok(envelope)
}

fn review_action_data_value(envelope: &Value) -> Option<&Value> {
    envelope
        .pointer("/result/data/json")
        .or_else(|| envelope.pointer("/data/json"))
        .or_else(|| envelope.pointer("/json"))
        .or_else(|| {
            envelope
                .as_array()?
                .iter()
                .find_map(|entry| entry.pointer("/result/data/json"))
        })
}

fn review_action_data_value_mut(envelope: &mut Value) -> Option<&mut Value> {
    if envelope.pointer("/result/data/json").is_some() {
        return envelope.pointer_mut("/result/data/json");
    }
    if envelope.pointer("/data/json").is_some() {
        return envelope.pointer_mut("/data/json");
    }
    if envelope.pointer("/json").is_some() {
        return envelope.pointer_mut("/json");
    }
    envelope
        .as_array_mut()?
        .iter_mut()
        .find_map(|entry| entry.pointer_mut("/result/data/json"))
}

fn build_review_action_annotations(project: &Project, data: &Value) -> Vec<Value> {
    let existing_by_id: BTreeMap<String, Value> = data
        .get("annotations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|annotation| {
            annotation
                .get("id")
                .and_then(Value::as_str)
                .map(|id| (id.to_string(), annotation.clone()))
        })
        .collect();

    let mut segments: Vec<_> = project.segments.iter().collect();
    segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    segments
        .into_iter()
        .map(|segment| {
            let annotation_id = segment
                .source_annotation_id
                .as_deref()
                .unwrap_or(segment.id.as_str());
            let existing = existing_by_id
                .get(annotation_id)
                .or_else(|| existing_by_id.get(&segment.id));
            review_annotation_for_segment(project, data, segment, annotation_id, existing)
        })
        .collect()
}

fn review_annotation_for_segment(
    project: &Project,
    data: &Value,
    segment: &Segment,
    annotation_id: &str,
    existing: Option<&Value>,
) -> Value {
    let mut annotation = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    annotation.insert("id".to_string(), json!(annotation_id));
    annotation.insert(
        "reviewActionId".to_string(),
        first_value([
            segment.metadata.get("reviewActionId"),
            existing.and_then(|value| value.get("reviewActionId")),
            data.get("actionId"),
        ])
        .unwrap_or_else(|| json!("")),
    );
    annotation.insert(
        "type".to_string(),
        first_value([
            segment.metadata.get("type"),
            existing.and_then(|value| value.get("type")),
        ])
        .unwrap_or_else(|| json!("transcription")),
    );
    annotation.insert(
        "content".to_string(),
        segment
            .metadata
            .get("content")
            .cloned()
            .unwrap_or_else(|| json!(segment.text)),
    );
    annotation.insert(
        "processedRecordingId".to_string(),
        first_value([
            segment.metadata.get("processedRecordingId"),
            existing.and_then(|value| value.get("processedRecordingId")),
            processed_recording_id_for_lane(project, &segment.lane_id).as_ref(),
        ])
        .unwrap_or_else(|| json!("")),
    );
    annotation.insert(
        "startTimeInSeconds".to_string(),
        json!(segment.start_seconds),
    );
    annotation.insert("endTimeInSeconds".to_string(), json!(segment.end_seconds));
    annotation.insert(
        "intensity".to_string(),
        first_value([
            segment.metadata.get("intensity"),
            existing.and_then(|value| value.get("intensity")),
        ])
        .unwrap_or(Value::Null),
    );
    annotation.insert(
        "metadata".to_string(),
        review_annotation_metadata_value(segment, existing),
    );
    annotation.insert(
        "createdAt".to_string(),
        first_value([
            segment.metadata.get("createdAt"),
            existing.and_then(|value| value.get("createdAt")),
        ])
        .unwrap_or(Value::Null),
    );
    annotation.insert(
        "updatedAt".to_string(),
        first_value([
            segment.metadata.get("updatedAt"),
            existing.and_then(|value| value.get("updatedAt")),
        ])
        .unwrap_or(Value::Null),
    );

    Value::Object(annotation)
}

fn first_value<'a>(values: impl IntoIterator<Item = Option<&'a Value>>) -> Option<Value> {
    values.into_iter().flatten().next().cloned()
}

fn processed_recording_id_for_lane(project: &Project, lane_id: &str) -> Option<Value> {
    load_extra(
        &project.metadata,
        &metadata_key(META_EXTRA_TRACK_PREFIX, lane_id),
    )
    .get("processed_recording_id")
    .cloned()
}

fn review_annotation_metadata_value(segment: &Segment, existing: Option<&Value>) -> Value {
    let object: serde_json::Map<String, Value> = segment
        .metadata
        .iter()
        .filter(|(key, _)| is_exported_annotation_metadata_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if object.is_empty() {
        return existing
            .and_then(|value| value.get("metadata"))
            .cloned()
            .unwrap_or(Value::Null);
    }
    Value::Object(object)
}

fn metadata_key(prefix: &str, id: &str) -> String {
    format!("{prefix}{id}")
}

fn segment_metadata_from_extra(extra: &BTreeMap<String, Value>) -> BTreeMap<String, Value> {
    let mut metadata = extra
        .get("metadata")
        .and_then(Value::as_object)
        .map(|object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();

    copy_extra_to_metadata(extra, &mut metadata, "reviewActionId", "reviewActionId");
    copy_extra_to_metadata(extra, &mut metadata, "review_action_id", "reviewActionId");
    copy_extra_to_metadata(extra, &mut metadata, "type", "type");
    copy_extra_to_metadata(
        extra,
        &mut metadata,
        "processedRecordingId",
        "processedRecordingId",
    );
    copy_extra_to_metadata(
        extra,
        &mut metadata,
        "processed_recording_id",
        "processedRecordingId",
    );
    copy_extra_to_metadata(extra, &mut metadata, "intensity", "intensity");
    copy_extra_to_metadata(extra, &mut metadata, "content", "content");
    copy_extra_to_metadata(extra, &mut metadata, "createdAt", "createdAt");
    copy_extra_to_metadata(extra, &mut metadata, "created_at", "createdAt");
    copy_extra_to_metadata(extra, &mut metadata, "updatedAt", "updatedAt");
    copy_extra_to_metadata(extra, &mut metadata, "updated_at", "updatedAt");

    metadata
}

fn store_original_annotation_snapshot(
    metadata: &mut BTreeMap<String, Value>,
    annotation: &BabelAnnotation,
) {
    metadata.insert(META_ORIGINAL_TEXT.to_string(), json!(annotation.text));
    metadata.insert(
        META_ORIGINAL_START_SECONDS.to_string(),
        json!(annotation.start_seconds),
    );
    metadata.insert(
        META_ORIGINAL_END_SECONDS.to_string(),
        json!(annotation.end_seconds),
    );
    metadata.insert(
        META_ORIGINAL_SPEAKER_ID.to_string(),
        json!(annotation.speaker_key),
    );
    metadata.insert(
        META_ORIGINAL_LANE_ID.to_string(),
        json!(annotation.track_id),
    );
}

fn copy_extra_to_metadata(
    extra: &BTreeMap<String, Value>,
    metadata: &mut BTreeMap<String, Value>,
    extra_key: &str,
    metadata_key: &str,
) {
    if let Some(value) = extra.get(extra_key) {
        metadata.insert(metadata_key.to_string(), value.clone());
    }
}

fn store_segment_metadata_in_extra(
    extra: &mut BTreeMap<String, Value>,
    metadata: &BTreeMap<String, Value>,
) {
    let object: serde_json::Map<String, Value> = metadata
        .iter()
        .filter(|(key, _)| is_exported_annotation_metadata_key(key))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    if object.is_empty() {
        return;
    }
    extra.insert("metadata".to_string(), Value::Object(object));
}

fn store_segment_top_level_fields_in_extra(
    extra: &mut BTreeMap<String, Value>,
    metadata: &BTreeMap<String, Value>,
) {
    if let Some(annotation_type) = metadata.get("type") {
        extra.insert("type".to_string(), annotation_type.clone());
    }
    if let Some(content) = metadata.get("content") {
        extra.insert("content".to_string(), content.clone());
    }
    if let Some(intensity) = metadata.get("intensity") {
        extra.insert("intensity".to_string(), intensity.clone());
    }
}

fn is_exported_annotation_metadata_key(key: &str) -> bool {
    if key.starts_with(META_ORIGINAL_PREFIX) {
        return false;
    }

    !matches!(
        key,
        "reviewActionId"
            | "type"
            | "processedRecordingId"
            | "intensity"
            | "content"
            | "createdAt"
            | "updatedAt"
    )
}

fn review_annotation_text(content: &Value) -> String {
    if let Some(text) = content.as_str() {
        return text.to_string();
    }

    let Some(object) = content.as_object() else {
        return String::new();
    };
    let primary = object.get("primaryLabel").and_then(Value::as_str);
    let secondary = object.get("secondaryLabel").and_then(Value::as_str);

    match (primary, secondary) {
        (Some(primary), Some(secondary)) if !secondary.is_empty() => {
            format!("{primary} / {secondary}")
        }
        (Some(primary), _) => primary.to_string(),
        _ => String::new(),
    }
}

fn store_extra(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    extra: &BTreeMap<String, Value>,
) {
    if extra.is_empty() {
        return;
    }

    let encoded = serde_json::to_string(extra).expect("Babel extra metadata should encode");
    metadata.insert(key.to_string(), encoded);
}

fn promote_native_project_metadata(
    metadata: &mut BTreeMap<String, String>,
    extra: &BTreeMap<String, Value>,
) {
    for (key, value) in extra
        .iter()
        .filter(|(key, _)| key.starts_with(META_NATIVE_PROJECT_PREFIX))
    {
        let value = match value {
            Value::String(value) => value.clone(),
            _ => value.to_string(),
        };
        metadata.insert(key.clone(), value);
    }
}

fn load_extra(metadata: &BTreeMap<String, String>, key: &str) -> BTreeMap<String, Value> {
    metadata
        .get(key)
        .and_then(|encoded| serde_json::from_str(encoded).ok())
        .unwrap_or_default()
}
