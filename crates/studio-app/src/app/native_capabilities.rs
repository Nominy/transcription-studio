use serde_json::{Map, Value, json};
use studio_core::{AudioLane, Project, Segment, SegmentUpdate};
use studio_waveform::DecodedAudio;
use time::OffsetDateTime;

use super::review_backend_client::{BackendMultipartAudioTrack, ReviewBackendRequest};
use super::state::{
    DraftAudioGuard, NativeCapabilityModalTab, REVIEW_BACKEND_REQUEST_LIMIT,
    ReviewerPendingFinalize, StudioState, project_review_action_level,
};

pub(super) const DRAFT_OPENROUTER_API_KEY_ENV: &str = "STUDIO_DRAFT_OPENROUTER_API_KEY";
pub(super) const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_EXTRA_TRACK_PREFIX: &str = "babel.extra.track.";
const META_REVIEW_ACTION_DECISION: &str = "babel.review_action.actionDecision";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";
const META_ORIGINAL_TEXT: &str = "studio.babel.original.text";
const META_ORIGINAL_START_SECONDS: &str = "studio.babel.original.startSeconds";
const META_ORIGINAL_END_SECONDS: &str = "studio.babel.original.endSeconds";
const META_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
const META_ORIGINAL_LANE_ID: &str = "studio.babel.original.laneId";
pub(super) const META_REVIEWER_SELECTED_SESSION_ID: &str = "studio.native.reviewer.sessionId";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftGenerateAudioMode {
    Guard,
    UseAvailableAudio,
    TextOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftingRequestConfig {
    pub(super) openrouter_api_key: String,
    pub(super) model: String,
    pub(super) service_tier: String,
    pub(super) reasoning_effort: String,
    pub(super) draft_session_id: String,
}

#[derive(Debug)]
struct DraftRowResult {
    row_id: String,
    rewritten_text: String,
    status: String,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NativeCapabilityHealth {
    pub(super) service: String,
    pub(super) backend_version: Option<String>,
    pub(super) openrouter_status: Option<String>,
    pub(super) test_mode: Option<bool>,
    pub(super) provider_diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ReviewerTemplatePreview {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) description: String,
    pub(super) category: String,
    pub(super) score: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewerSuggestionPreview {
    pub(super) proposal_id: String,
    pub(super) operation: String,
    pub(super) category: String,
    pub(super) target_template_id: Option<String>,
    pub(super) title: String,
    pub(super) description: Option<String>,
    pub(super) report_texts: Vec<String>,
    pub(super) reason: String,
    pub(super) source_card_ids: Vec<String>,
    pub(super) decision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewerCardPreview {
    pub(super) id: String,
    pub(super) change_index: i64,
    pub(super) kind: String,
    pub(super) summary: String,
    pub(super) evidence: String,
    pub(super) categories: Vec<String>,
    pub(super) matched_template_id: Option<String>,
    pub(super) template_title: Option<String>,
    pub(super) match_source: Option<String>,
    pub(super) opinion_text: Option<String>,
    pub(super) rationale: Option<String>,
    pub(super) comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewerSessionPreview {
    pub(super) session_id: String,
    pub(super) review_action_id: String,
    pub(super) backend_service: Option<String>,
    pub(super) backend_release: Option<String>,
    pub(super) session_comment: String,
    pub(super) cards: Vec<ReviewerCardPreview>,
    pub(super) suggestions: Vec<ReviewerSuggestionPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewerFeedbackPreview {
    pub(super) category: String,
    pub(super) score: String,
    pub(super) note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ReviewerGeneratedReviewPreview {
    pub(super) model: Option<String>,
    pub(super) received_at: Option<String>,
    pub(super) template_registry_version: Option<String>,
    pub(super) matched_template_ids: Vec<String>,
    pub(super) feedback: Vec<ReviewerFeedbackPreview>,
    pub(super) cards: Vec<ReviewerCardPreview>,
    pub(super) prepared: Value,
    pub(super) ai_review: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReviewerFinalizePreview {
    pub(super) session_id: String,
    pub(super) mode: String,
    pub(super) applied_at: Option<String>,
    pub(super) backend_service: Option<String>,
    pub(super) backend_release: Option<String>,
    pub(super) feedback: Vec<ReviewerFeedbackPreview>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftSummaryPreview {
    pub(super) total_rows: usize,
    pub(super) rewritten_rows: usize,
    pub(super) unchanged_rows: usize,
    pub(super) failed_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftRowPreview {
    pub(super) row_id: String,
    pub(super) original_text: String,
    pub(super) rewritten_text: String,
    pub(super) status: String,
    pub(super) warnings: Vec<String>,
    pub(super) applied: bool,
    pub(super) skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftRedistributionAllocationPreview {
    pub(super) segment_id: String,
    pub(super) text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftRedistributionMovePreview {
    pub(super) from_index: usize,
    pub(super) to_index: usize,
    pub(super) sentence_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftRedistributionPreview {
    pub(super) group_id: String,
    pub(super) model: Option<String>,
    pub(super) accept_draft: bool,
    pub(super) notes: Option<String>,
    pub(super) moves: Vec<DraftRedistributionMovePreview>,
    pub(super) allocations: Vec<DraftRedistributionAllocationPreview>,
    pub(super) error: Option<String>,
    pub(super) applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DraftTranscriptionPreview {
    pub(super) segment_id: Option<String>,
    pub(super) original_text: String,
    pub(super) text: String,
    pub(super) model: Option<String>,
}

impl DraftingRequestConfig {
    pub(super) fn from_inputs(
        openrouter_api_key: &str,
        model: &str,
        service_tier: &str,
        reasoning_effort: &str,
        draft_session_id: &str,
    ) -> Self {
        Self {
            openrouter_api_key: openrouter_api_key.trim().to_string(),
            model: model.trim().to_string(),
            service_tier: service_tier.trim().to_string(),
            reasoning_effort: reasoning_effort.trim().to_string(),
            draft_session_id: draft_session_id.trim().to_string(),
        }
    }

    pub(super) fn with_env_fallbacks(mut self) -> Self {
        if self.openrouter_api_key.is_empty() {
            self.openrouter_api_key = std::env::var(DRAFT_OPENROUTER_API_KEY_ENV)
                .or_else(|_| std::env::var(OPENROUTER_API_KEY_ENV))
                .unwrap_or_default()
                .trim()
                .to_string();
        }
        self
    }
}

pub(super) fn build_reviewer_payload(project: &Project) -> Result<Value, String> {
    let review_action_id = review_action_id(project)
        .ok_or_else(|| "current project has no Babel review action id".to_string())?;
    let package_extra = package_extra(project);
    let captured_at = current_utc_timestamp();

    Ok(json!({
        "reviewActionId": review_action_id,
        "original": normalized_review_action(project, &review_action_id, &captured_at, true, &package_extra),
        "current": normalized_review_action(project, &review_action_id, &captured_at, false, &package_extra),
        "babelDiff": Value::Null,
    }))
}

pub(super) fn build_reviewer_analytics_payload(project: &Project) -> Result<Value, String> {
    let payload = build_reviewer_payload(project)?;
    Ok(json!({
        "reviewActionId": payload.get("reviewActionId").cloned().unwrap_or(Value::Null),
        "original": payload.get("original").cloned().unwrap_or(Value::Null),
        "current": payload.get("current").cloned().unwrap_or(Value::Null),
        "babelDiff": payload.get("babelDiff").cloned().unwrap_or(Value::Null),
        "inputBoxes": {
            "categories": {},
            "notes": []
        },
        "aiReview": Value::Null,
        "metadata": {
            "client": "transcription-studio-native",
            "source": "native-capability-panel",
            "capturedAt": current_utc_timestamp()
        }
    }))
}

fn build_reviewer_analytics_payload_with_session(
    project: &Project,
    session: Option<&ReviewerSessionPreview>,
    generated_review: Option<&ReviewerGeneratedReviewPreview>,
) -> Result<Value, String> {
    let mut payload = build_reviewer_analytics_payload(project)?;
    payload["inputBoxes"] = reviewer_input_boxes_snapshot(session, generated_review);
    if let Some(generated_review) = generated_review {
        payload["prepared"] = generated_review.prepared.clone();
        payload["aiReview"] = generated_review.ai_review.clone();
    }
    if let Some(session) = session
        && let Some(metadata) = payload.get_mut("metadata").and_then(Value::as_object_mut)
    {
        metadata.insert("sessionId".to_string(), json!(session.session_id));
        metadata.insert(
            "reviewActionId".to_string(),
            json!(session.review_action_id),
        );
    }
    Ok(payload)
}

fn reviewer_input_boxes_snapshot(
    session: Option<&ReviewerSessionPreview>,
    generated_review: Option<&ReviewerGeneratedReviewPreview>,
) -> Value {
    let mut categories = Map::new();
    if let Some(generated_review) = generated_review {
        for feedback in &generated_review.feedback {
            categories.insert(
                feedback.category.clone(),
                json!({
                    "score": feedback.score,
                    "note": feedback.note,
                }),
            );
        }
    }

    let mut notes = Vec::new();
    if let Some(session) = session {
        if !session.session_comment.trim().is_empty() {
            notes.push(json!({
                "scope": "session",
                "comment": session.session_comment,
            }));
        }
        for card in &session.cards {
            if !card.comment.trim().is_empty() {
                notes.push(json!({
                    "scope": "card",
                    "cardId": card.id,
                    "comment": card.comment,
                }));
            }
        }
    }

    json!({
        "categories": Value::Object(categories),
        "notes": notes,
    })
}

pub(super) fn build_draft_generate_payload(
    project: &Project,
    config: &DraftingRequestConfig,
) -> Result<Value, String> {
    let config = require_drafting_api_key(config)?;
    let mut payload = json!({
        "projectPreset": "ru-gold-2sp-v1",
        "jobId": draft_job_id(project),
        "rows": drafting_rows(project),
        "openRouterApiKey": config.openrouter_api_key,
    });
    add_optional_drafting_fields(&mut payload, &config);
    Ok(payload)
}

pub(super) fn build_draft_transcribe_segment_payload(
    project: &Project,
    selected_segment_id: Option<&str>,
    config: &DraftingRequestConfig,
) -> Result<Value, String> {
    let config = require_drafting_api_key(config)?;
    let segment = selected_segment(project, selected_segment_id)?;
    let mut payload = json!({
        "openRouterApiKey": config.openrouter_api_key,
        "segment": {
            "rowId": segment.id,
            "speakerKey": segment.speaker_id,
            "startSeconds": segment.start_seconds,
            "endSeconds": segment.end_seconds,
        }
    });
    add_optional_drafting_fields(&mut payload, &config);
    Ok(payload)
}

pub(super) fn build_draft_redistribute_payload(
    project: &Project,
    selected_segment_id: Option<&str>,
    config: &DraftingRequestConfig,
) -> Result<Value, String> {
    let config = require_drafting_api_key(config)?;
    let selected = selected_segment(project, selected_segment_id)?;
    let rows = sorted_segments(project)
        .into_iter()
        .enumerate()
        .filter(|(_, segment)| segment.speaker_id == selected.speaker_id)
        .map(|(index, segment)| {
            json!({
                "id": segment.id,
                "index": index,
                "speakerKey": segment.speaker_id,
                "startSeconds": segment.start_seconds,
                "endSeconds": segment.end_seconds,
                "text": segment.text,
            })
        })
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return Err("redistribution requires at least one same-speaker segment".to_string());
    }
    let draft_allocations = rows
        .iter()
        .filter_map(|row| {
            Some(json!({
                "segmentId": row.get("id")?.as_str()?,
                "text": row.get("text")?.as_str()?,
            }))
        })
        .collect::<Vec<_>>();
    let full_text = rows
        .iter()
        .filter_map(|row| row.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");

    let mut payload = json!({
        "openRouterApiKey": config.openrouter_api_key,
        "groups": [{
            "groupId": format!("{}:{}", project.id, selected.speaker_id),
            "speakerKey": selected.speaker_id,
            "fullText": full_text,
            "segments": rows,
            "draftAllocations": draft_allocations,
        }]
    });
    add_optional_drafting_fields(&mut payload, &config);
    Ok(payload)
}

fn require_drafting_api_key(
    config: &DraftingRequestConfig,
) -> Result<DraftingRequestConfig, String> {
    let config = config.clone().with_env_fallbacks();
    if config.openrouter_api_key.trim().is_empty() {
        return Err(format!(
            "drafting requires `{DRAFT_OPENROUTER_API_KEY_ENV}` or `{OPENROUTER_API_KEY_ENV}`"
        ));
    }
    Ok(config)
}

fn add_optional_drafting_fields(payload: &mut Value, config: &DraftingRequestConfig) {
    let Some(object) = payload.as_object_mut() else {
        return;
    };
    if !config.model.is_empty() {
        object.insert("model".to_string(), json!(config.model));
    }
    if !config.service_tier.is_empty() && config.service_tier != "default" {
        object.insert("serviceTier".to_string(), json!(config.service_tier));
    }
    if !config.reasoning_effort.is_empty() && config.reasoning_effort != "default" {
        object.insert(
            "reasoningEffort".to_string(),
            json!(config.reasoning_effort),
        );
    }
    if !config.draft_session_id.is_empty() {
        object.insert("draftSessionId".to_string(), json!(config.draft_session_id));
    }
}

fn normalized_review_action(
    project: &Project,
    review_action_id: &str,
    captured_at: &str,
    original: bool,
    package_extra: &Map<String, Value>,
) -> Value {
    let current_action_level = project_review_action_level(project).unwrap_or(0);
    let action_level = if original && current_action_level == 2 {
        1
    } else {
        current_action_level
    };
    let action_decision = if original && current_action_level == 2 {
        "pass".to_string()
    } else {
        action_decision(project, package_extra)
    };
    let annotations = sorted_segments(project)
        .into_iter()
        .map(|segment| review_annotation(project, segment, review_action_id, original))
        .collect::<Vec<_>>();
    json!({
        "actionId": review_action_id,
        "actionLevel": action_level,
        "actionDecision": action_decision,
        "annotations": annotations,
        "recordings": review_recordings(project),
        "lintErrors": package_extra.get("lint_errors").and_then(Value::as_array).cloned().unwrap_or_default(),
        "capturedAt": captured_at,
    })
}

fn review_annotation(
    project: &Project,
    segment: &Segment,
    review_action_id: &str,
    original: bool,
) -> Value {
    let source_id = segment
        .source_annotation_id
        .as_deref()
        .unwrap_or(segment.id.as_str());
    let lane_id = if original {
        metadata_string(segment, META_ORIGINAL_LANE_ID).unwrap_or_else(|| segment.lane_id.clone())
    } else {
        segment.lane_id.clone()
    };
    json!({
        "id": source_id,
        "reviewActionId": metadata_string(segment, "reviewActionId").unwrap_or_else(|| review_action_id.to_string()),
        "type": metadata_string(segment, "type").unwrap_or_else(|| "transcription".to_string()),
        "content": annotation_text(segment, original),
        "processedRecordingId": metadata_string(segment, "processedRecordingId")
            .or_else(|| processed_recording_id_for_lane(project, &lane_id))
            .unwrap_or_default(),
        "startTimeInSeconds": annotation_start(segment, original),
        "endTimeInSeconds": annotation_end(segment, original),
        "metadata": annotation_metadata(segment),
    })
}

fn review_recordings(project: &Project) -> Vec<Value> {
    sorted_lanes(project)
        .into_iter()
        .enumerate()
        .map(|(index, lane)| {
            let extra = track_extra(project, &lane.id);
            let bounds = lane_bounds(project, &lane.id);
            json!({
                "id": string_from_map(&extra, &["chunked_processed_recording_id", "chunkedProcessedRecordingId", "id"])
                    .or_else(|| string_from_map(&extra, &["processed_recording_id", "processedRecordingId"]))
                    .unwrap_or_else(|| lane.id.clone()),
                "transcriptionChunkId": string_from_map(&extra, &["transcription_chunk_id", "transcriptionChunkId"])
                    .unwrap_or_else(|| project.id.clone()),
                "processedRecordingId": string_from_map(&extra, &["processed_recording_id", "processedRecordingId"])
                    .unwrap_or_else(|| lane.id.clone()),
                "speaker": speaker_number(lane).unwrap_or(index as i64 + 1),
                "startTimeInSeconds": bounds.map(|bounds| bounds.0).unwrap_or(0.0),
                "endTimeInSeconds": bounds.map(|bounds| bounds.1).unwrap_or(0.0),
            })
        })
        .collect()
}

fn drafting_rows(project: &Project) -> Vec<Value> {
    sorted_segments(project)
        .into_iter()
        .enumerate()
        .map(|(index, segment)| {
            json!({
                "rowId": segment.id,
                "speakerKey": segment.speaker_id,
                "startSeconds": segment.start_seconds,
                "endSeconds": segment.end_seconds,
                "text": segment.text,
                "index": index,
            })
        })
        .collect()
}

fn selected_segment<'a>(
    project: &'a Project,
    selected_segment_id: Option<&str>,
) -> Result<&'a Segment, String> {
    selected_segment_id
        .and_then(|id| project.segment(id))
        .or_else(|| sorted_segments(project).into_iter().next())
        .ok_or_else(|| "selected segment is not available".to_string())
}

fn sorted_segments(project: &Project) -> Vec<&Segment> {
    let mut segments = project.segments.iter().collect::<Vec<_>>();
    segments.sort_by(|left, right| {
        left.start_seconds
            .total_cmp(&right.start_seconds)
            .then_with(|| left.id.cmp(&right.id))
    });
    segments
}

fn sorted_lanes(project: &Project) -> Vec<&AudioLane> {
    let mut lanes = project.lanes.iter().collect::<Vec<_>>();
    lanes.sort_by(|left, right| left.id.cmp(&right.id));
    lanes
}

fn annotation_text(segment: &Segment, original: bool) -> String {
    if original {
        return metadata_string(segment, META_ORIGINAL_TEXT)
            .unwrap_or_else(|| segment.text.clone());
    }
    metadata_string(segment, "content").unwrap_or_else(|| segment.text.clone())
}

fn annotation_start(segment: &Segment, original: bool) -> f64 {
    if original {
        return metadata_f64(segment, META_ORIGINAL_START_SECONDS).unwrap_or(segment.start_seconds);
    }
    segment.start_seconds
}

fn annotation_end(segment: &Segment, original: bool) -> f64 {
    if original {
        return metadata_f64(segment, META_ORIGINAL_END_SECONDS).unwrap_or(segment.end_seconds);
    }
    segment.end_seconds
}

fn annotation_metadata(segment: &Segment) -> Value {
    let object = segment
        .metadata
        .iter()
        .filter(|(key, _)| !key.starts_with("studio.babel.original."))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    if object.is_empty() {
        Value::Null
    } else {
        Value::Object(object)
    }
}

fn lane_bounds(project: &Project, lane_id: &str) -> Option<(f64, f64)> {
    let mut matching = project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id);
    let first = matching.next()?;
    let mut start = first.start_seconds;
    let mut end = first.end_seconds;
    for segment in matching {
        start = start.min(segment.start_seconds);
        end = end.max(segment.end_seconds);
    }
    Some((start, end))
}

fn processed_recording_id_for_lane(project: &Project, lane_id: &str) -> Option<String> {
    string_from_map(
        &track_extra(project, lane_id),
        &["processed_recording_id", "processedRecordingId"],
    )
}

fn track_extra(project: &Project, lane_id: &str) -> Map<String, Value> {
    metadata_json_object(project, &format!("{META_EXTRA_TRACK_PREFIX}{lane_id}"))
}

fn package_extra(project: &Project) -> Map<String, Value> {
    metadata_json_object(project, META_EXTRA_PACKAGE)
}

fn metadata_json_object(project: &Project, key: &str) -> Map<String, Value> {
    project
        .metadata
        .get(key)
        .and_then(|encoded| serde_json::from_str::<Value>(encoded).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn metadata_string(segment: &Segment, key: &str) -> Option<String> {
    segment.metadata.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    })
}
fn review_action_id(project: &Project) -> Option<String> {
    project
        .metadata
        .get(META_REVIEW_ACTION_ID)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            project
                .segments
                .iter()
                .find_map(|segment| metadata_string(segment, "reviewActionId"))
        })
        .or_else(|| {
            string_from_map(
                &package_extra(project),
                &[
                    "reviewActionId",
                    "review_action_id",
                    "action_id",
                    "actionId",
                ],
            )
        })
        .or_else(|| review_action_id_from_envelope(project))
}

fn metadata_f64(segment: &Segment, key: &str) -> Option<f64> {
    segment.metadata.get(key).and_then(Value::as_f64)
}

fn review_action_id_from_envelope(project: &Project) -> Option<String> {
    let value = project
        .metadata
        .get(META_REVIEW_ACTION_ENVELOPE)
        .and_then(|encoded| serde_json::from_str::<Value>(encoded).ok())?;
    find_object_with_annotations(&value)
        .and_then(|object| string_from_map(object, &["actionId", "reviewActionId"]))
}

fn find_object_with_annotations(value: &Value) -> Option<&Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if object.get("annotations").is_some_and(Value::is_array) {
                return Some(object);
            }
            object.values().find_map(find_object_with_annotations)
        }
        Value::Array(items) => items.iter().find_map(find_object_with_annotations),
        _ => None,
    }
}

fn action_decision(project: &Project, package_extra: &Map<String, Value>) -> String {
    project
        .metadata
        .get(META_REVIEW_ACTION_DECISION)
        .cloned()
        .or_else(|| string_from_map(package_extra, &["action_decision", "actionDecision"]))
        .unwrap_or_default()
}

fn draft_job_id(project: &Project) -> String {
    review_action_id(project).unwrap_or_else(|| project.id.clone())
}

fn ensure_draft_stream_session_id(payload: &mut Value) {
    let has_session_id = payload
        .get("draftSessionId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty());
    if has_session_id {
        return;
    }
    let job_id = payload
        .get("jobId")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("native-draft")
        .to_string();
    if let Some(object) = payload.as_object_mut() {
        object.insert(
            "draftSessionId".to_string(),
            json!(format!("{job_id}:{}", current_utc_timestamp())),
        );
    }
}

fn draft_payload_has_session_id(payload: &Value) -> bool {
    payload
        .get("draftSessionId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
}

fn is_reconnectable_draft_stream_error(error: &str) -> bool {
    let message = error.to_ascii_lowercase();
    message.contains("could not reach backend")
        || message.contains("failed to fetch")
        || message.contains("networkerror")
        || message.contains("load failed")
        || message.contains("response read failed")
}

fn speaker_number(lane: &AudioLane) -> Option<i64> {
    lane.speaker_id
        .chars()
        .filter(|char| char.is_ascii_digit())
        .collect::<String>()
        .parse::<i64>()
        .ok()
}

fn string_from_map(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
            Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn current_utc_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond() / 1_000_000
    )
}

impl StudioState {
    pub(super) fn toggle_native_capability_panel(&mut self) {
        let opening = !self.native_capability_panel_open;
        self.native_capability_panel_open = opening;
        if opening {
            self.native_capability_modal_tab = if self.reviewer_capability_available() {
                NativeCapabilityModalTab::Reviewer
            } else {
                NativeCapabilityModalTab::GoldDrafting
            };
        }
    }

    pub(super) fn queue_draft_backend_health(&mut self) {
        self.queue_review_backend_request(
            "Checking Reviewer/Drafting backend...",
            ReviewBackendRequest::BackendHealth,
        );
    }

    pub(super) fn queue_reviewer_generate(&mut self) {
        self.queue_current_reviewer_payload(
            "Reviewer generate",
            ReviewBackendRequest::ReviewGenerate,
        );
    }

    pub(super) fn queue_reviewer_create_session(&mut self) {
        self.queue_current_reviewer_payload(
            "Reviewer session",
            ReviewBackendRequest::ReviewCreateSession,
        );
    }

    pub(super) fn queue_reviewer_load_session(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer session") {
            return;
        }
        let session_id = match required_reviewer_field("session ID", &self.reviewer_session_id) {
            Ok(value) => value,
            Err(status) => {
                self.status = status;
                return;
            }
        };
        self.queue_review_backend_request(
            "Loading review session...",
            ReviewBackendRequest::ReviewGetSession { session_id },
        );
    }

    pub(super) fn queue_reviewer_save_comments(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer comments") {
            return;
        }
        if self.has_pending_reviewer_comment_request() {
            self.status = "Reviewer comments are already being saved.".to_string();
            return;
        }
        let session_id = match required_reviewer_field("session ID", &self.reviewer_session_id) {
            Ok(value) => value,
            Err(status) => {
                self.status = status;
                return;
            }
        };
        let body = self.reviewer_comments_body();
        self.reviewer_comment_save_failed = false;
        self.queue_review_backend_request(
            "Saving review comments...",
            ReviewBackendRequest::ReviewUpdateComments { session_id, body },
        );
    }

    pub(super) fn queue_reviewer_template_search(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer templates") {
            return;
        }
        let query = self.reviewer_template_query.trim().to_string();
        self.queue_review_backend_request(
            "Searching review templates...",
            ReviewBackendRequest::ReviewSearchTemplates {
                query,
                limit: Some(20),
            },
        );
    }

    pub(super) fn update_reviewer_session_comment(&mut self, value: String) {
        if self.reviewer_session_comment == value {
            return;
        }
        self.reviewer_session_comment = value.clone();
        if let Some(session) = self.reviewer_session_preview.as_mut() {
            session.session_comment = value;
        }
        self.reviewer_comments_dirty = true;
    }

    pub(super) fn set_reviewer_cards_expanded(&mut self, expanded: bool) {
        self.reviewer_cards_expanded = expanded;
    }

    pub(super) fn toggle_reviewer_suggestions_expanded(&mut self) {
        self.reviewer_suggestions_expanded = !self.reviewer_suggestions_expanded;
    }

    pub(super) fn update_reviewer_session_id(&mut self, value: String) {
        let session_id = value.trim().to_string();
        self.cancel_reviewer_session_persistence_if_changed(&session_id);
        self.reviewer_session_id = session_id;
        self.persist_reviewer_selected_session_id();
    }

    pub(super) fn update_reviewer_card_comment(&mut self, value: String) {
        if self.reviewer_card_comment == value {
            return;
        }
        self.reviewer_card_comment = value.clone();
        if let Some(card) = self.reviewer_session_preview.as_mut().and_then(|session| {
            session
                .cards
                .iter_mut()
                .find(|card| card.id == self.reviewer_card_id)
        }) {
            card.comment = value;
        }
        self.reviewer_comments_dirty = true;
    }

    pub(super) fn select_reviewer_card(&mut self, card_id: String) {
        let card_id = card_id.trim().to_string();
        self.reviewer_card_id = card_id.clone();
        let Some(card) = self
            .reviewer_session_preview
            .as_ref()
            .and_then(|session| session.cards.iter().find(|card| card.id == card_id))
        else {
            return;
        };
        self.reviewer_card_comment = card.comment.clone();
        self.reviewer_template_id = card.matched_template_id.clone().unwrap_or_default();
    }

    pub(super) fn auto_save_reviewer_comments(&mut self) {
        if !self.reviewer_capability_available()
            || !self.reviewer_comments_dirty
            || self.reviewer_comment_save_failed
            || self.reviewer_session_id.trim().is_empty()
            || self.has_pending_reviewer_comment_request()
        {
            return;
        }
        self.queue_reviewer_save_comments();
    }

    pub(super) fn should_run_reviewer_comment_save_tick(&self) -> bool {
        self.reviewer_capability_available()
            && self.reviewer_comments_dirty
            && !self.reviewer_comment_save_failed
            && !self.reviewer_session_id.trim().is_empty()
            && !self.has_pending_reviewer_comment_request()
    }

    pub(super) fn queue_reviewer_set_template_match(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer template match") {
            return;
        }
        let (session_id, card_id, template_id) = match (
            required_reviewer_field("session ID", &self.reviewer_session_id),
            required_reviewer_field("card ID", &self.reviewer_card_id),
            required_reviewer_field("template ID", &self.reviewer_template_id),
        ) {
            (Ok(session_id), Ok(card_id), Ok(template_id)) => (session_id, card_id, template_id),
            (Err(status), _, _) | (_, Err(status), _) | (_, _, Err(status)) => {
                self.status = status;
                return;
            }
        };
        self.queue_review_backend_request(
            "Setting review template match...",
            ReviewBackendRequest::ReviewSetTemplateMatch {
                session_id,
                card_id,
                body: json!({ "templateId": template_id }),
            },
        );
    }

    pub(super) fn queue_reviewer_clear_template_match(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer template match") {
            return;
        }
        let (session_id, card_id) = match (
            required_reviewer_field("session ID", &self.reviewer_session_id),
            required_reviewer_field("card ID", &self.reviewer_card_id),
        ) {
            (Ok(session_id), Ok(card_id)) => (session_id, card_id),
            (Err(status), _) | (_, Err(status)) => {
                self.status = status;
                return;
            }
        };
        self.queue_review_backend_request(
            "Clearing review template match...",
            ReviewBackendRequest::ReviewClearTemplateMatch {
                session_id,
                card_id,
            },
        );
    }

    pub(super) fn queue_reviewer_template_suggestions(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer suggestions") {
            return;
        }
        let session_id = match required_reviewer_field("session ID", &self.reviewer_session_id) {
            Ok(value) => value,
            Err(status) => {
                self.status = status;
                return;
            }
        };
        self.queue_review_backend_request(
            "Generating template suggestions...",
            ReviewBackendRequest::ReviewGenerateTemplateSuggestions { session_id },
        );
    }

    pub(super) fn queue_reviewer_decide_template_suggestion(&mut self, decision: &'static str) {
        if self.block_reviewer_capability_unavailable("Reviewer suggestions") {
            return;
        }
        let (session_id, proposal_id) = match (
            required_reviewer_field("session ID", &self.reviewer_session_id),
            required_reviewer_field("proposal ID", &self.reviewer_proposal_id),
        ) {
            (Ok(session_id), Ok(proposal_id)) => (session_id, proposal_id),
            (Err(status), _) | (_, Err(status)) => {
                self.status = status;
                return;
            }
        };
        self.queue_review_backend_request(
            "Submitting template suggestion decision...",
            ReviewBackendRequest::ReviewDecideTemplateSuggestion {
                session_id,
                proposal_id,
                body: json!({ "decision": decision }),
            },
        );
    }

    pub(super) fn queue_reviewer_finalize(&mut self, mode: &'static str) {
        if self.block_reviewer_capability_unavailable("Reviewer finalize") {
            return;
        }
        let session_id = match required_reviewer_field("session ID", &self.reviewer_session_id) {
            Ok(value) => value,
            Err(status) => {
                self.status = status;
                return;
            }
        };
        if self.reviewer_comments_dirty || self.has_pending_reviewer_comment_request() {
            self.reviewer_pending_finalize = Some(ReviewerPendingFinalize {
                session_id,
                mode: mode.to_string(),
                workspace_generation: self.workspace_generation,
            });
            if !self.has_pending_reviewer_comment_request() {
                self.queue_reviewer_save_comments();
            }
            if self.has_pending_reviewer_comment_request() {
                self.status =
                    format!("Saving all reviewer comments before finalizing as {mode}...");
            }
            return;
        }
        self.enqueue_reviewer_finalize(mode);
    }

    pub(super) fn queue_reviewer_submit_analytics(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer analytics") {
            return;
        }
        if !self.apply_pending_drafts("Reviewer analytics") {
            return;
        }
        let Some(project) = self.project().cloned() else {
            self.status = "Reviewer analytics unavailable: no project loaded.".to_string();
            return;
        };
        match build_reviewer_analytics_payload_with_session(
            &project,
            self.reviewer_session_preview.as_ref(),
            self.reviewer_generated_review_preview.as_ref(),
        ) {
            Ok(payload) => self.queue_review_backend_request(
                "Submitting Reviewer analytics...",
                ReviewBackendRequest::ReviewSubmitAnalytics(payload),
            ),
            Err(error) => self.fail_native_capability_request("Reviewer analytics", error),
        }
    }
    pub(super) fn queue_reviewer_submit_analytics_best_effort(&mut self) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        match build_reviewer_analytics_payload_with_session(
            &project,
            self.reviewer_session_preview.as_ref(),
            self.reviewer_generated_review_preview.as_ref(),
        ) {
            Ok(payload) => {
                let _ = self.try_push_review_backend_request(
                    ReviewBackendRequest::ReviewSubmitAnalytics(payload),
                );
            }
            Err(error) => self.native_capability_last_error = Some(error),
        }
    }

    pub(super) fn queue_reviewer_history(&mut self) {
        if self.block_reviewer_capability_unavailable("Reviewer history") {
            return;
        }
        let query = self
            .reviewer_history_query
            .split('&')
            .filter_map(|part| {
                let trimmed = part.trim();
                if trimmed.is_empty() {
                    return None;
                }
                let (key, value) = trimmed.split_once('=').unwrap_or(("q", trimmed));
                Some((key.trim().to_string(), value.trim().to_string()))
            })
            .collect::<Vec<_>>();
        self.queue_review_backend_request(
            "Loading review history...",
            ReviewBackendRequest::ReviewHistory { query },
        );
    }

    pub(super) fn queue_draft_generate_rows(&mut self) {
        self.queue_draft_generate_rows_with_audio_mode(DraftGenerateAudioMode::Guard);
    }

    pub(super) fn retry_draft_audio_capture(&mut self) {
        self.queue_draft_generate_rows_with_audio_mode(DraftGenerateAudioMode::Guard);
    }

    pub(super) fn queue_draft_generate_rows_with_available_audio(&mut self) {
        self.queue_draft_generate_rows_with_audio_mode(DraftGenerateAudioMode::UseAvailableAudio);
    }

    pub(super) fn queue_draft_generate_rows_text_only(&mut self) {
        self.queue_draft_generate_rows_with_audio_mode(DraftGenerateAudioMode::TextOnly);
    }

    fn queue_draft_generate_rows_with_audio_mode(&mut self, audio_mode: DraftGenerateAudioMode) {
        let Some(project) = self.project().cloned() else {
            self.status = "Drafting unavailable: no project loaded.".to_string();
            return;
        };
        let config = self.drafting_request_config();
        match build_draft_generate_payload(&project, &config) {
            Ok(mut payload) => {
                if audio_mode != DraftGenerateAudioMode::TextOnly
                    && self.has_pending_audio_draft_generation(&payload)
                {
                    self.status = "Gold Drafting row generation is already pending.".to_string();
                    self.native_capability_last_error = None;
                    return;
                }
                ensure_draft_stream_session_id(&mut payload);
                let audio_tracks = if audio_mode == DraftGenerateAudioMode::TextOnly {
                    Vec::new()
                } else {
                    self.broker_audio_tracks_for_project(&project)
                };
                if audio_mode == DraftGenerateAudioMode::Guard
                    && self.show_draft_audio_guard_if_needed(&audio_tracks)
                {
                    return;
                }
                if audio_mode == DraftGenerateAudioMode::UseAvailableAudio
                    && audio_tracks.is_empty()
                {
                    self.status =
                        "Drafting audio retry found no native speaker-lane audio.".to_string();
                    self.show_draft_audio_guard_if_needed(&audio_tracks);
                    return;
                }
                let request = if audio_tracks.is_empty() {
                    ReviewBackendRequest::DraftGenerateStream(payload)
                } else {
                    ReviewBackendRequest::DraftGenerateStreamWithAudio {
                        payload,
                        audio_tracks,
                    }
                };
                self.draft_audio_guard = None;
                self.queue_review_backend_request("Streaming Gold Drafting rows...", request);
            }
            Err(error) => self.fail_native_capability_request("Drafting", error),
        }
    }

    pub(super) fn queue_draft_transcribe_selected_segment(&mut self) {
        let Some(project) = self.project().cloned() else {
            self.status = "Drafting unavailable: no project loaded.".to_string();
            return;
        };
        let selected_segment_id = self
            .editor
            .as_ref()
            .and_then(|editor| editor.selected_segment_id.as_deref());
        let config = self.drafting_request_config();
        match build_draft_transcribe_segment_payload(&project, selected_segment_id, &config) {
            Ok(payload) => {
                if self.has_pending_audio_segment_transcription(&payload) {
                    self.status = "Selected-segment transcription is already pending.".to_string();
                    self.native_capability_last_error = None;
                    return;
                }
                let audio_tracks = self.broker_audio_tracks_for_project(&project);
                if audio_tracks.is_empty() {
                    self.status =
                        "Drafting transcription blocked: load native lane audio first.".to_string();
                    return;
                }
                self.queue_review_backend_request(
                    "Transcribing selected segment...",
                    ReviewBackendRequest::DraftBrokerTranscribeSegmentWithAudio {
                        payload,
                        audio_tracks,
                    },
                );
            }
            Err(error) => self.fail_native_capability_request("Drafting", error),
        }
    }

    pub(super) fn queue_draft_redistribute_text(&mut self) {
        let Some(project) = self.project().cloned() else {
            self.status = "Drafting unavailable: no project loaded.".to_string();
            return;
        };
        let selected_segment_id = self
            .editor
            .as_ref()
            .and_then(|editor| editor.selected_segment_id.as_deref());
        let config = self.drafting_request_config();
        match build_draft_redistribute_payload(&project, selected_segment_id, &config) {
            Ok(payload) => self.queue_review_backend_request(
                "Reviewing text redistribution...",
                ReviewBackendRequest::DraftBrokerRedistributeText(payload),
            ),
            Err(error) => self.fail_native_capability_request("Drafting", error),
        }
    }

    pub(super) fn apply_last_draft_rows(&mut self) {
        if self.block_read_only_edit("Apply drafting") {
            return;
        }
        let Some(response) = self.native_capability_last_response.as_ref() else {
            self.status = "Apply drafting blocked: no draft response available.".to_string();
            return;
        };
        let rows = match parse_draft_rows_response(response) {
            Ok(rows) => rows,
            Err(error) => {
                self.status = format!("Apply drafting blocked: {error}");
                return;
            }
        };
        let rows = rows
            .into_iter()
            .filter(|row| row.status == "rewritten")
            .collect::<Vec<_>>();
        if rows.is_empty() {
            self.status = "Apply drafting found no rewritten rows.".to_string();
            return;
        }

        let (updates, missing) = match self.segment_updates_for_rows(rows) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let applied = updates.len();
        if applied == 0 {
            self.status = "Apply drafting found no rows present in this project.".to_string();
            return;
        }
        if !self.apply_segment_updates("Apply drafting", updates) {
            return;
        }

        self.status = if missing.is_empty() {
            format!("Applied {applied} drafted rows.")
        } else {
            format!(
                "Applied {applied} drafted rows; missing {} rows.",
                missing.len()
            )
        };
    }

    pub(super) fn apply_last_transcribed_segment_text(&mut self) {
        if self.block_read_only_edit("Apply drafting segment") {
            return;
        }
        let Some(response) = self.native_capability_last_response.as_ref() else {
            self.status =
                "Apply drafting segment blocked: no transcription response available.".to_string();
            return;
        };
        let value = match serde_json::from_str::<Value>(response) {
            Ok(value) => value,
            Err(error) => {
                self.status =
                    format!("Apply drafting segment blocked: malformed response: {error}");
                return;
            }
        };
        let Some(text) = value
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            self.status =
                "Apply drafting segment blocked: response did not include text.".to_string();
            return;
        };
        let preview_segment_id = self
            .draft_transcription_preview
            .as_ref()
            .filter(|preview| preview.text.as_str() == text.as_str())
            .and_then(|preview| preview.segment_id.clone());
        let Some(segment_id) = preview_segment_id.or_else(|| {
            self.editor
                .as_ref()
                .and_then(|editor| editor.selected_segment_id.clone())
        }) else {
            self.status = "Apply drafting segment blocked: no selected segment.".to_string();
            return;
        };
        let applied = match &mut self.workflow {
            Ok(workflow) => {
                let Some(segment) = workflow.project().segment(&segment_id).cloned() else {
                    self.status = format!(
                        "Apply drafting segment blocked: selected segment `{segment_id}` was not found."
                    );
                    return;
                };
                match workflow.apply_segment_edit(
                    &segment_id,
                    segment.start_seconds,
                    segment.end_seconds,
                    text,
                ) {
                    Ok(()) => true,
                    Err(error) => {
                        self.status =
                            format!("Apply drafting segment failed for `{segment_id}`: {error}");
                        return;
                    }
                }
            }
            Err(error) => {
                self.status = format!("Apply drafting segment unavailable: {error}");
                return;
            }
        };
        if applied {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            self.invalidate_transcript_layout_cache();
            self.status = format!("Applied drafted text to `{segment_id}`.");
        }
    }

    pub(super) fn skip_draft_row(&mut self, row_id: &str) {
        let Some(preview) = self
            .draft_row_previews
            .iter_mut()
            .find(|preview| preview.row_id == row_id)
        else {
            self.status = format!("Skip drafting row blocked: `{row_id}` was not in the preview.");
            return;
        };
        preview.skipped = true;
        preview.applied = false;
        self.status = format!("Skipped drafted row `{row_id}`.");
    }

    pub(super) fn apply_draft_row(&mut self, row_id: &str) {
        if self.block_read_only_edit("Apply drafting row") {
            return;
        }
        let Some(preview) = self
            .draft_row_previews
            .iter()
            .find(|preview| preview.row_id == row_id)
            .cloned()
        else {
            self.status = format!("Apply drafting row blocked: `{row_id}` was not in the preview.");
            return;
        };
        if preview.skipped {
            self.status = format!("Apply drafting row blocked: `{row_id}` was skipped.");
            return;
        }
        if preview.status != "rewritten" {
            self.status = format!(
                "Apply drafting row blocked: `{row_id}` has status `{}`.",
                preview.status
            );
            return;
        }
        let (updates, missing) = match self.segment_updates_for_rows(vec![DraftRowResult {
            row_id: preview.row_id.clone(),
            rewritten_text: preview.rewritten_text.clone(),
            status: preview.status.clone(),
            warnings: preview.warnings.clone(),
        }]) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if !missing.is_empty() || updates.is_empty() {
            self.status = format!("Apply drafting row blocked: `{row_id}` was not found.");
            return;
        }
        if self.apply_segment_updates("Apply drafting row", updates) {
            if let Some(preview) = self
                .draft_row_previews
                .iter_mut()
                .find(|preview| preview.row_id == row_id)
            {
                preview.applied = true;
                preview.skipped = false;
            }
            self.status = format!("Applied drafted row `{row_id}`.");
        }
    }

    pub(super) fn apply_redistribution_group(&mut self, group_id: &str) {
        if self.block_read_only_edit("Apply redistribution") {
            return;
        }
        let Some(preview) = self
            .draft_redistribution_previews
            .iter()
            .find(|preview| preview.group_id == group_id)
            .cloned()
        else {
            self.status =
                format!("Apply redistribution blocked: group `{group_id}` was not in the preview.");
            return;
        };
        if let Some(error) = preview.error.as_ref() {
            self.status = format!("Apply redistribution blocked: {error}");
            return;
        }
        let allocations = if preview.accept_draft {
            Ok(preview.allocations.clone())
        } else if preview.moves.is_empty() {
            Err(format!(
                "Apply redistribution blocked: group `{group_id}` was not accepted and has no model moves."
            ))
        } else {
            apply_redistribution_moves_to_allocations(&preview.allocations, &preview.moves)
        };
        let allocations = match allocations {
            Ok(allocations) => allocations,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let rows = allocations
            .iter()
            .map(|allocation| DraftRowResult {
                row_id: allocation.segment_id.clone(),
                rewritten_text: allocation.text.clone(),
                status: "rewritten".to_string(),
                warnings: Vec::new(),
            })
            .collect::<Vec<_>>();
        let (updates, missing) = match self.segment_updates_for_rows(rows) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if updates.is_empty() {
            self.status =
                format!("Apply redistribution blocked: group `{group_id}` has no project rows.");
            return;
        }
        let applied = updates.len();
        if self.apply_segment_updates("Apply redistribution", updates) {
            if let Some(preview) = self
                .draft_redistribution_previews
                .iter_mut()
                .find(|preview| preview.group_id == group_id)
            {
                preview.applied = true;
            }
            self.status = if missing.is_empty() && preview.accept_draft {
                format!("Applied redistribution group `{group_id}` to {applied} rows.")
            } else if missing.is_empty() {
                format!(
                    "Applied redistribution group `{group_id}` to {applied} rows using {} model move(s).",
                    preview.moves.len()
                )
            } else {
                format!(
                    "Applied redistribution group `{group_id}` to {applied} rows; missing {} rows.",
                    missing.len()
                )
            };
        }
    }

    pub(super) fn handle_native_capability_success(
        &mut self,
        request: &ReviewBackendRequest,
        payload: &str,
    ) {
        match request {
            ReviewBackendRequest::BackendHealth => match parse_backend_health(payload) {
                Ok(health) => {
                    self.native_capability_health = Some(health);
                    self.status = "Reviewer/Drafting backend is reachable.".to_string();
                }
                Err(error) => self.status = format!("Backend health parse failed: {error}"),
            },
            ReviewBackendRequest::ReviewCreateSession(_)
            | ReviewBackendRequest::ReviewGetSession { .. }
            | ReviewBackendRequest::ReviewSetTemplateMatch { .. }
            | ReviewBackendRequest::ReviewClearTemplateMatch { .. }
            | ReviewBackendRequest::ReviewGenerateTemplateSuggestions { .. }
            | ReviewBackendRequest::ReviewDecideTemplateSuggestion { .. } => {
                match parse_reviewer_session_preview(payload) {
                    Ok(session) => self.replace_reviewer_session_preview(session),
                    Err(error) => self.status = format!("Reviewer session parse failed: {error}"),
                }
            }
            ReviewBackendRequest::ReviewUpdateComments { session_id, body } => {
                if session_id != &self.reviewer_session_id {
                    return;
                }
                match parse_reviewer_session_preview(payload) {
                    Ok(session) => {
                        self.complete_reviewer_comment_save(session_id, body, session);
                    }
                    Err(error) => {
                        self.reviewer_comments_dirty = true;
                        self.reviewer_comment_save_failed = true;
                        self.status = format!(
                            "Reviewer comments response could not be acknowledged: {error}"
                        );
                    }
                }
            }
            ReviewBackendRequest::ReviewGenerate(_) => {
                match parse_generated_review_preview(payload) {
                    Ok(preview) => {
                        let feedback = preview.feedback.clone();
                        let feedback_count = feedback.len();
                        self.reviewer_generated_review_preview = Some(preview);
                        let applied = self.apply_reviewer_feedback_items(&feedback);
                        if applied > 0 {
                            self.status = format!(
                                "Generated review automatically applied {applied} of {feedback_count} feedback item(s); saving draft..."
                            );
                        }
                    }
                    Err(error) => self.status = format!("Reviewer generate parse failed: {error}"),
                }
            }
            ReviewBackendRequest::ReviewFinalizeSession { session_id, body } => {
                if !self.reviewer_finalize_request_is_current(session_id, body) {
                    return;
                }
                match parse_reviewer_finalize_preview(session_id, body, payload) {
                    Ok(preview) => {
                        let feedback_count = preview.feedback.len();
                        let applied = self.apply_reviewer_feedback_items(&preview.feedback);
                        if let Ok(generated) = parse_generated_review_preview(payload) {
                            self.reviewer_generated_review_preview = Some(generated);
                        }
                        let mode = preview.mode.clone();
                        self.reviewer_finalize_preview = Some(preview);
                        self.reviewer_pending_finalize = None;
                        self.native_capability_panel_open = false;
                        self.status = format!(
                            "Finalized review session as {mode}; applied {applied} of {feedback_count} feedback items."
                        );
                    }
                    Err(error) => {
                        self.reviewer_pending_finalize = None;
                        self.status = format!("Reviewer finalize parse failed: {error}");
                    }
                }
            }
            ReviewBackendRequest::ReviewSearchTemplates { .. } => {
                match parse_template_search_results(payload) {
                    Ok(results) => {
                        self.reviewer_template_results = results;
                        self.status = format!(
                            "Loaded {} review template results.",
                            self.reviewer_template_results.len()
                        );
                    }
                    Err(error) => self.status = format!("Template search parse failed: {error}"),
                }
            }
            ReviewBackendRequest::DraftGenerate(_)
            | ReviewBackendRequest::DraftGenerateWithAudio { .. }
            | ReviewBackendRequest::DraftGenerateStream(_) => match self.project() {
                Some(project) => match parse_draft_preview(project, payload) {
                    Ok((rows, summary)) => {
                        let count = rows.len();
                        self.draft_row_previews = rows;
                        self.draft_summary_preview = summary;
                        self.status = format!("Loaded {count} drafted row previews.");
                    }
                    Err(error) => self.status = format!("Draft preview parse failed: {error}"),
                },
                None => self.status = "Draft preview unavailable: no project loaded.".to_string(),
            },
            ReviewBackendRequest::DraftBrokerTranscribeSegment(request_payload)
            | ReviewBackendRequest::DraftBrokerTranscribeSegmentWithAudio {
                payload: request_payload,
                ..
            } => {
                let preview = match self.project() {
                    Some(project) => {
                        parse_transcription_preview_for_project(project, request_payload, payload)
                    }
                    None => parse_transcription_preview(payload),
                };
                match preview {
                    Ok(preview) => {
                        self.draft_transcription_preview = Some(preview);
                        self.status = "Loaded drafted segment transcription preview.".to_string();
                    }
                    Err(error) => {
                        self.status = format!("Draft transcription preview parse failed: {error}")
                    }
                }
            }
            ReviewBackendRequest::DraftBrokerRedistributeText(request_payload) => {
                match parse_redistribution_previews(request_payload, payload) {
                    Ok(previews) => {
                        let count = previews.len();
                        self.draft_redistribution_previews = previews;
                        self.status = format!("Loaded {count} redistribution previews.");
                    }
                    Err(error) => {
                        self.status = format!("Redistribution preview parse failed: {error}")
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn reconcile_draft_stream_failure(
        &mut self,
        request: &ReviewBackendRequest,
        error: &str,
    ) -> bool {
        if !is_reconnectable_draft_stream_error(error) {
            return false;
        }
        let reconcile_request = match request {
            ReviewBackendRequest::DraftGenerateStream(payload)
                if draft_payload_has_session_id(payload) =>
            {
                ReviewBackendRequest::DraftGenerate(payload.clone())
            }
            ReviewBackendRequest::DraftGenerateStreamWithAudio {
                payload,
                audio_tracks,
            } if draft_payload_has_session_id(payload) => {
                ReviewBackendRequest::DraftGenerateWithAudio {
                    payload: payload.clone(),
                    audio_tracks: audio_tracks.clone(),
                }
            }
            _ => return false,
        };
        self.queue_review_backend_request(
            "Stream connection lost. Reconciling final draft response...",
            reconcile_request,
        );
        true
    }

    fn queue_current_reviewer_payload(
        &mut self,
        action_name: &str,
        wrap: impl FnOnce(Value) -> ReviewBackendRequest,
    ) {
        if self.block_reviewer_capability_unavailable(action_name) {
            return;
        }
        if !self.apply_pending_drafts(action_name) {
            return;
        }
        let Some(project) = self.project().cloned() else {
            self.status = format!("{action_name} unavailable: no project loaded.");
            return;
        };
        match build_reviewer_payload(&project) {
            Ok(payload) => {
                self.queue_review_backend_request(format!("{action_name} queued..."), wrap(payload))
            }
            Err(error) => self.fail_native_capability_request(action_name, error),
        }
    }

    fn block_reviewer_capability_unavailable(&mut self, action_name: &str) -> bool {
        if self.reviewer_capability_available() {
            return false;
        }
        self.native_capability_modal_tab = NativeCapabilityModalTab::GoldDrafting;
        let message = "Reviewer is only available for L2 review tasks.";
        self.status = format!("{action_name} unavailable: {message}");
        self.native_capability_last_error = Some(message.to_string());
        true
    }

    fn queue_review_backend_request(
        &mut self,
        status: impl Into<String>,
        request: ReviewBackendRequest,
    ) {
        let status = status.into();
        if self.review_backend_request_is_pending(&request) {
            self.status = format!("{status} Request is already pending.");
            self.native_capability_last_error = None;
            return;
        }
        if self.pending_review_backend_request_count() >= REVIEW_BACKEND_REQUEST_LIMIT {
            let error = format!(
                "review backend already has {REVIEW_BACKEND_REQUEST_LIMIT} pending requests"
            );
            self.status = format!("{status} Blocked: {error}.");
            self.native_capability_last_error = Some(error);
            return;
        }
        self.status = status;
        self.native_capability_last_error = None;
        self.review_backend_requests.push(request);
    }

    fn try_push_review_backend_request(&mut self, request: ReviewBackendRequest) -> bool {
        if self.review_backend_request_is_pending(&request)
            || self.pending_review_backend_request_count() >= REVIEW_BACKEND_REQUEST_LIMIT
        {
            return false;
        }
        self.review_backend_requests.push(request);
        true
    }

    fn review_backend_request_is_pending(&self, request: &ReviewBackendRequest) -> bool {
        self.review_backend_requests
            .iter()
            .chain(self.dispatched_review_backend_requests.iter())
            .any(|pending| pending == request)
    }

    fn pending_review_backend_request_count(&self) -> usize {
        self.review_backend_requests.len()
            + self
                .dispatched_review_backend_requests
                .iter()
                .filter(|request| !self.review_backend_requests.contains(request))
                .count()
    }

    fn has_pending_audio_draft_generation(&self, payload: &Value) -> bool {
        self.review_backend_requests
            .iter()
            .chain(self.dispatched_review_backend_requests.iter())
            .any(|request| {
                matches!(
                    request,
                    ReviewBackendRequest::DraftGenerateWithAudio {
                        payload: pending,
                        ..
                    }
                        | ReviewBackendRequest::DraftGenerateStreamWithAudio {
                            payload: pending,
                            ..
                        }
                        if draft_generate_payloads_are_equivalent(pending, payload)
                )
            })
    }

    fn has_pending_audio_segment_transcription(&self, payload: &Value) -> bool {
        self.review_backend_requests
            .iter()
            .chain(self.dispatched_review_backend_requests.iter())
            .any(|request| {
                matches!(
                    request,
                    ReviewBackendRequest::DraftBrokerTranscribeSegmentWithAudio {
                        payload: pending,
                        ..
                    } if pending == payload
                )
            })
    }

    fn show_draft_audio_guard_if_needed(
        &mut self,
        audio_tracks: &[BackendMultipartAudioTrack],
    ) -> bool {
        let Some(status) = self.draft_audio_cue_status() else {
            return false;
        };
        if status.total_lanes == 0 || status.loaded_tracks >= status.total_lanes {
            return false;
        }
        let can_use_partial_audio = status.loaded_tracks > 0;
        let problem = if status.loaded_tracks == 0 {
            format!(
                "No speaker-lane audio is loaded. {} generic/native track(s) cannot cover {} speaker lane(s).",
                audio_tracks.len(),
                status.total_lanes
            )
        } else {
            format!(
                "Only {} of {} speaker lane(s) are loaded for drafting audio.",
                status.loaded_tracks, status.total_lanes
            )
        };
        self.draft_audio_guard = Some(DraftAudioGuard {
            expected_speaker_lanes: status.total_lanes,
            captured_speaker_lanes: status.loaded_tracks,
            captured_tracks: audio_tracks.len(),
            can_use_partial_audio,
            message: format!(
                "{problem} Audio cues may miss laughter or other audible events if the draft starts now."
            ),
        });
        self.status = "Audio capture needs review before generating.".to_string();
        true
    }

    fn fail_native_capability_request(&mut self, action_name: &str, error: String) {
        self.status = format!("{action_name} unavailable: {error}");
        self.native_capability_last_error = Some(error);
    }

    pub(super) fn handle_native_capability_failure(
        &mut self,
        request: &ReviewBackendRequest,
        error: &str,
    ) -> bool {
        match request {
            ReviewBackendRequest::ReviewUpdateComments { session_id, .. } => {
                if session_id != &self.reviewer_session_id {
                    return true;
                }
                self.reviewer_comments_dirty = true;
                self.reviewer_comment_save_failed = true;
                self.status = format!(
                    "Reviewer comments save failed: {error}. Drafts remain unsaved; use Save comments to retry."
                );
                true
            }
            ReviewBackendRequest::ReviewFinalizeSession { session_id, body } => {
                if !self.reviewer_finalize_request_is_current(session_id, body) {
                    return true;
                }
                self.reviewer_pending_finalize = None;
                self.status = format!("Reviewer finalize failed: {error}");
                true
            }
            _ => false,
        }
    }

    fn cancel_reviewer_session_persistence_if_changed(&mut self, next_session_id: &str) {
        if self.reviewer_session_id == next_session_id {
            return;
        }
        self.reviewer_comments_dirty = false;
        self.reviewer_comment_save_failed = false;
        self.reviewer_pending_finalize = None;
        self.drain_review_backend_requests_matching(|request| {
            matches!(
                request,
                ReviewBackendRequest::ReviewUpdateComments { session_id, .. }
                    | ReviewBackendRequest::ReviewFinalizeSession { session_id, .. }
                    if session_id != next_session_id
            )
        });
    }

    fn reviewer_finalize_request_is_current(
        &self,
        request_session_id: &str,
        request_body: &Value,
    ) -> bool {
        let Some(request_mode) = request_body.get("mode").and_then(Value::as_str) else {
            return false;
        };
        self.reviewer_session_id == request_session_id
            && self
                .reviewer_pending_finalize
                .as_ref()
                .is_some_and(|pending| {
                    pending.session_id == request_session_id
                        && pending.mode == request_mode
                        && pending.workspace_generation == self.workspace_generation
                })
    }

    fn reviewer_comments_body(&self) -> Value {
        let mut card_comments = Map::new();
        if let Some(session) = self.reviewer_session_preview.as_ref() {
            for card in &session.cards {
                card_comments.insert(card.id.clone(), json!(card.comment));
            }
        }
        if !self.reviewer_card_id.trim().is_empty()
            && !card_comments.contains_key(self.reviewer_card_id.trim())
        {
            card_comments.insert(
                self.reviewer_card_id.trim().to_string(),
                json!(self.reviewer_card_comment),
            );
        }
        json!({
            "sessionComment": self.reviewer_session_comment,
            "cardComments": card_comments,
        })
    }

    fn merge_current_reviewer_comments_into(&self, session: &mut ReviewerSessionPreview) {
        if session.session_id != self.reviewer_session_id {
            return;
        }
        session.session_comment = self.reviewer_session_comment.clone();
        let comments = self
            .reviewer_comments_body()
            .get("cardComments")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for card in &mut session.cards {
            if let Some(comment) = comments.get(&card.id).and_then(Value::as_str) {
                card.comment = comment.to_string();
            }
        }
    }

    fn complete_reviewer_comment_save(
        &mut self,
        request_session_id: &str,
        request_body: &Value,
        mut session: ReviewerSessionPreview,
    ) {
        if self.reviewer_session_id != request_session_id
            || session.session_id != request_session_id
        {
            return;
        }
        let acknowledged_current = request_body == &self.reviewer_comments_body();
        self.reviewer_comment_save_failed = false;
        if acknowledged_current {
            self.reviewer_comments_dirty = false;
        } else {
            self.merge_current_reviewer_comments_into(&mut session);
            self.reviewer_comments_dirty = true;
        }
        self.replace_reviewer_session_preview(session);

        let Some(pending) = self.reviewer_pending_finalize.as_ref() else {
            return;
        };
        if pending.session_id != request_session_id
            || pending.workspace_generation != self.workspace_generation
        {
            return;
        }
        if self.reviewer_comments_dirty {
            self.queue_reviewer_save_comments();
        } else {
            let mode = pending.mode.clone();
            self.enqueue_reviewer_finalize(&mode);
        }
    }

    fn enqueue_reviewer_finalize(&mut self, mode: &str) {
        let Ok(session_id) = required_reviewer_field("session ID", &self.reviewer_session_id)
        else {
            return;
        };
        if self
            .review_backend_requests
            .iter()
            .chain(self.dispatched_review_backend_requests.iter())
            .any(|request| matches!(request, ReviewBackendRequest::ReviewFinalizeSession { .. }))
        {
            return;
        }
        self.reviewer_pending_finalize = Some(ReviewerPendingFinalize {
            session_id: session_id.clone(),
            mode: mode.to_string(),
            workspace_generation: self.workspace_generation,
        });
        self.queue_review_backend_request(
            "Finalizing review session...",
            ReviewBackendRequest::ReviewFinalizeSession {
                session_id,
                body: json!({ "mode": mode }),
            },
        );
    }

    fn has_pending_reviewer_comment_request(&self) -> bool {
        self.review_backend_requests
            .iter()
            .chain(self.dispatched_review_backend_requests.iter())
            .any(|request| {
                matches!(
                    request,
                    ReviewBackendRequest::ReviewUpdateComments { session_id, .. }
                        if session_id == &self.reviewer_session_id
                )
            })
    }

    fn drafting_request_config(&self) -> DraftingRequestConfig {
        DraftingRequestConfig::from_inputs(
            &self.draft_openrouter_api_key,
            &self.draft_model,
            &self.draft_service_tier,
            &self.draft_reasoning_effort,
            &self.draft_session_id,
        )
    }

    fn replace_reviewer_session_preview(&mut self, mut session: ReviewerSessionPreview) {
        self.cancel_reviewer_session_persistence_if_changed(&session.session_id);
        let selected_card_id = self.reviewer_card_id.clone();
        let preserve_local_comments = session.session_id == self.reviewer_session_id
            && (self.reviewer_comments_dirty || self.has_pending_reviewer_comment_request());
        if preserve_local_comments {
            self.merge_current_reviewer_comments_into(&mut session);
        }

        self.reviewer_session_id = session.session_id.clone();
        self.reviewer_session_comment = session.session_comment.clone();
        self.persist_reviewer_selected_session_id();
        let selected_card = session
            .cards
            .iter()
            .find(|card| card.id == selected_card_id)
            .or_else(|| session.cards.first());
        if let Some(card) = selected_card {
            self.reviewer_card_id = card.id.clone();
            self.reviewer_card_comment = card.comment.clone();
            self.reviewer_template_id = card.matched_template_id.clone().unwrap_or_default();
        } else {
            self.reviewer_card_id.clear();
            self.reviewer_card_comment.clear();
            self.reviewer_template_id.clear();
        }
        self.reviewer_proposal_id = session
            .suggestions
            .first()
            .map(|suggestion| suggestion.proposal_id.clone())
            .unwrap_or_default();
        let card_count = session.cards.len();
        self.reviewer_session_preview = Some(session);
        self.status = format!("Loaded review session with {card_count} cards.");
    }

    pub(super) fn hydrate_reviewer_selection_from_project_metadata(&mut self) {
        self.reviewer_session_id = self
            .project()
            .and_then(|project| project.metadata.get(META_REVIEWER_SELECTED_SESSION_ID))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_default();
    }

    fn persist_reviewer_selected_session_id(&mut self) {
        let value = self.reviewer_session_id.trim().to_string();
        let updated = if let Ok(workflow) = &mut self.workflow {
            workflow
                .merge_project_metadata([(META_REVIEWER_SELECTED_SESSION_ID.to_string(), value)]);
            true
        } else {
            false
        };
        if updated {
            self.invalidate_render_caches_for_project_metadata();
        }
    }

    fn segment_updates_for_rows(
        &self,
        rows: Vec<DraftRowResult>,
    ) -> Result<(Vec<SegmentUpdate>, Vec<String>), String> {
        let Some(project) = self.project() else {
            return Err("Apply drafting unavailable: no project loaded.".to_string());
        };
        let mut updates = Vec::new();
        let mut missing = Vec::new();
        for row in rows {
            let Some(segment) = project.segment(&row.row_id) else {
                missing.push(row.row_id);
                continue;
            };
            updates.push(SegmentUpdate {
                segment_id: row.row_id,
                start_seconds: segment.start_seconds,
                end_seconds: segment.end_seconds,
                text: row.rewritten_text,
            });
        }
        Ok((updates, missing))
    }

    fn apply_segment_updates(&mut self, action_name: &str, updates: Vec<SegmentUpdate>) -> bool {
        let target_segment_ids = updates
            .iter()
            .map(|update| update.segment_id.clone())
            .collect::<Vec<_>>();
        if !self.apply_pending_drafts(action_name) {
            return false;
        }

        let applied = match &mut self.workflow {
            Ok(workflow) => match workflow.apply_segment_edits(updates) {
                Ok(()) => true,
                Err(error) => {
                    self.status = format!("{action_name} failed: {error}");
                    false
                }
            },
            Err(error) => {
                self.status = format!("{action_name} unavailable: {error}");
                false
            }
        };
        if applied {
            for segment_id in target_segment_ids {
                self.refresh_segment_draft(&segment_id);
            }
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            self.invalidate_transcript_layout_cache();
        }
        applied
    }

    fn broker_audio_tracks_for_project(
        &self,
        project: &Project,
    ) -> Vec<BackendMultipartAudioTrack> {
        let Some(editor) = self.editor.as_ref() else {
            return Vec::new();
        };
        project
            .lanes
            .iter()
            .filter_map(|lane| {
                let audio = editor
                    .lane_waveforms
                    .get(&lane.id)
                    .and_then(|waveform| waveform.decoded_audio())?;
                let bytes = encode_decoded_audio_as_pcm16_wav(audio).ok()?;
                Some(BackendMultipartAudioTrack {
                    track_id: lane.id.clone(),
                    file_name: format!("{}.wav", safe_audio_file_stem(&lane.id)),
                    mime_type: "audio/wav".to_string(),
                    metadata: json!({
                        "speakerKey": lane.speaker_id,
                        "trackLabel": lane.label,
                    }),
                    bytes: bytes.into(),
                })
            })
            .collect()
    }
}

fn draft_generate_payloads_are_equivalent(pending: &Value, requested: &Value) -> bool {
    if requested
        .get("draftSessionId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        return pending == requested;
    }
    let (Some(pending), Some(requested)) = (pending.as_object(), requested.as_object()) else {
        return pending == requested;
    };
    pending
        .iter()
        .filter(|(key, _)| key.as_str() != "draftSessionId")
        .all(|(key, value)| requested.get(key) == Some(value))
        && requested
            .iter()
            .all(|(key, value)| pending.get(key) == Some(value))
}

fn encode_decoded_audio_as_pcm16_wav(audio: &DecodedAudio) -> Result<Vec<u8>, String> {
    let channels = audio.number_of_channels();
    if channels == 0 || channels > u16::MAX as usize {
        return Err(format!("unsupported channel count: {channels}"));
    }
    let frame_count = audio.frame_count();
    let sample_rate = audio.sample_rate();
    let bytes_per_sample = 2usize;
    let data_len = frame_count
        .checked_mul(channels)
        .and_then(|samples| samples.checked_mul(bytes_per_sample))
        .ok_or_else(|| "audio is too large to encode".to_string())?;
    let riff_size = 36usize
        .checked_add(data_len)
        .ok_or_else(|| "audio is too large to encode".to_string())?;
    if riff_size > u32::MAX as usize || data_len > u32::MAX as usize {
        return Err("audio is too large to encode as WAV".to_string());
    }

    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(riff_size as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&(channels as u16).to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate
        .checked_mul(channels as u32)
        .and_then(|value| value.checked_mul(bytes_per_sample as u32))
        .ok_or_else(|| "audio sample rate is too large".to_string())?;
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = (channels * bytes_per_sample) as u16;
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());

    let channel_data = (0..channels)
        .map(|channel| {
            audio
                .channel_data(channel)
                .ok_or_else(|| format!("missing decoded audio channel: {channel}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    for frame_index in 0..frame_count {
        for channel in &channel_data {
            let sample = channel
                .get(frame_index)
                .copied()
                .unwrap_or_default()
                .clamp(-1.0, 1.0);
            let scaled = if sample < 0.0 {
                sample * 32768.0
            } else {
                sample * 32767.0
            };
            bytes.extend_from_slice(&(scaled.round() as i16).to_le_bytes());
        }
    }

    Ok(bytes)
}

fn safe_audio_file_stem(value: &str) -> String {
    let stem = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    if stem.is_empty() {
        "audio".to_string()
    } else {
        stem
    }
}

fn required_reviewer_field(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("Reviewer request blocked: missing {label}."));
    }
    Ok(trimmed.to_string())
}

fn parse_draft_rows_response(response: &str) -> Result<Vec<DraftRowResult>, String> {
    let value = serde_json::from_str::<Value>(response)
        .map_err(|error| format!("malformed draft response: {error}"))?;
    let rows = value
        .get("draftRows")
        .and_then(Value::as_array)
        .ok_or_else(|| "draft response did not include draftRows".to_string())?;
    rows.iter()
        .map(|row| {
            let row_id = row
                .get("rowId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| "draft row is missing rowId".to_string())?
                .to_string();
            let rewritten_text = row
                .get("rewrittenText")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("draft row `{row_id}` is missing rewrittenText"))?
                .to_string();
            let status = row
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let warnings = string_array(row.get("warnings").unwrap_or(&Value::Null));
            Ok(DraftRowResult {
                row_id,
                rewritten_text,
                status,
                warnings,
            })
        })
        .collect()
}

fn parse_backend_health(payload: &str) -> Result<NativeCapabilityHealth, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let service = value
        .get("service")
        .and_then(Value::as_str)
        .unwrap_or("review-backend")
        .to_string();
    let backend_version = value
        .get("backendVersion")
        .and_then(value_to_compact_string)
        .or_else(|| {
            value
                .get("backendVersion")
                .and_then(|version| version.get("release"))
                .and_then(value_to_compact_string)
        });
    let openrouter_status = find_number_by_key(&value, "remaining_credits")
        .map(|remaining| format!("credits remaining: {}", compact_number(remaining)))
        .or_else(|| {
            value
                .get("openRouterCredits")
                .and_then(|credits| credits.get("error"))
                .and_then(Value::as_str)
                .map(|error| format!("credits unavailable: {error}"))
        });
    Ok(NativeCapabilityHealth {
        service,
        backend_version,
        openrouter_status,
        test_mode: value.get("testMode").and_then(Value::as_bool),
        provider_diagnostics: health_provider_diagnostics(&value),
    })
}

fn health_provider_diagnostics(value: &Value) -> Vec<String> {
    [
        ("provider", &["provider", "openRouterProvider"][..]),
        ("model", &["model", "draftingModel", "openRouterModel"][..]),
        ("tier", &["serviceTier", "tier"][..]),
        ("reasoning", &["reasoningEffort", "reasoning"][..]),
    ]
    .into_iter()
    .filter_map(|(label, keys)| {
        find_string_by_keys(value, keys).map(|value| format!("{label}: {value}"))
    })
    .collect()
}

fn parse_generated_review_preview(payload: &str) -> Result<ReviewerGeneratedReviewPreview, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let ai_review = value
        .get("llm")
        .filter(|value| value.is_object())
        .or_else(|| value.get("aiReview").filter(|value| value.is_object()))
        .unwrap_or(&value);
    let feedback_value = ai_review
        .get("feedback")
        .or_else(|| value.get("categoryFeedback"))
        .or_else(|| value.get("feedback"))
        .unwrap_or(&Value::Null);
    let feedback = ai_review
        .get("feedback")
        .or_else(|| value.get("categoryFeedback"))
        .or_else(|| value.get("feedback"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(reviewer_feedback_preview_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut cards = value
        .get("cards")
        .or_else(|| ai_review.get("cards"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|card| reviewer_card_preview_from_value(card, &Value::Null))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if cards.is_empty() {
        cards = ai_review
            .get("classifications")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .map(|(index, classification)| {
                        let change_index = classification
                            .get("change")
                            .and_then(|value| {
                                value.as_i64().or_else(|| value.as_str()?.parse().ok())
                            })
                            .unwrap_or(index as i64);
                        let template_id = optional_string(classification, "templateId");
                        ReviewerCardPreview {
                            id: format!("change-{change_index}"),
                            change_index,
                            kind: "TEMPLATE MATCH".to_string(),
                            summary: template_id
                                .as_ref()
                                .map(|id| format!("Matched template {id}"))
                                .unwrap_or_else(|| "Generated reviewer classification".to_string()),
                            evidence: classification.to_string(),
                            categories: Vec::new(),
                            matched_template_id: template_id,
                            template_title: None,
                            match_source: Some("model".to_string()),
                            opinion_text: None,
                            rationale: None,
                            comment: String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    }
    if cards.is_empty() {
        cards = feedback
            .iter()
            .enumerate()
            .map(|(index, feedback)| ReviewerCardPreview {
                id: format!("feedback-{index}"),
                change_index: index as i64,
                kind: "CATEGORY FEEDBACK".to_string(),
                summary: feedback.category.clone(),
                evidence: feedback.note.clone(),
                categories: vec![feedback.category.clone()],
                matched_template_id: None,
                template_title: None,
                match_source: None,
                opinion_text: Some(feedback.note.clone()),
                rationale: None,
                comment: String::new(),
            })
            .collect();
    }
    if cards.is_empty() && feedback.is_empty() {
        return Err(
            "review generate response did not include feedback, classifications, or cards"
                .to_string(),
        );
    }
    Ok(ReviewerGeneratedReviewPreview {
        model: optional_string(ai_review, "model"),
        received_at: optional_string(ai_review, "receivedAt"),
        template_registry_version: optional_string(ai_review, "templateRegistryVersion"),
        matched_template_ids: string_array(
            ai_review
                .get("matchedTemplateIds")
                .or_else(|| value.get("matchedTemplateIds"))
                .unwrap_or(&Value::Null),
        ),
        feedback,
        cards,
        prepared: value.get("prepared").cloned().unwrap_or(Value::Null),
        ai_review: if ai_review.is_object() {
            ai_review.clone()
        } else {
            json!({ "feedback": feedback_value.clone() })
        },
    })
}

fn parse_reviewer_finalize_preview(
    request_session_id: &str,
    request_body: &Value,
    payload: &str,
) -> Result<ReviewerFinalizePreview, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let generated = parse_generated_review_preview(payload)?;
    let backend_version = value.get("backendVersion").unwrap_or(&Value::Null);
    Ok(ReviewerFinalizePreview {
        session_id: optional_string(&value, "sessionId")
            .unwrap_or_else(|| request_session_id.to_string()),
        mode: optional_string(&value, "mode")
            .or_else(|| optional_string(request_body, "mode"))
            .unwrap_or_else(|| "apply".to_string()),
        applied_at: optional_string(&value, "appliedAt"),
        backend_service: backend_version
            .get("service")
            .and_then(value_to_compact_string),
        backend_release: backend_version
            .get("release")
            .and_then(value_to_compact_string)
            .or_else(|| {
                value
                    .get("backendVersion")
                    .and_then(value_to_compact_string)
            }),
        feedback: generated.feedback,
    })
}

fn reviewer_feedback_preview_from_value(value: &Value) -> ReviewerFeedbackPreview {
    ReviewerFeedbackPreview {
        category: optional_string(value, "category").unwrap_or_else(|| "Uncategorized".to_string()),
        score: value
            .get("score")
            .and_then(value_to_compact_string)
            .unwrap_or_default(),
        note: optional_string(value, "note")
            .or_else(|| optional_string(value, "feedback"))
            .or_else(|| optional_string(value, "text"))
            .unwrap_or_default(),
    }
}

fn reviewer_card_preview_from_value(card: &Value, card_comments: &Value) -> ReviewerCardPreview {
    let change_index = card
        .get("changeIndex")
        .and_then(Value::as_i64)
        .or_else(|| card.get("change").and_then(Value::as_i64))
        .unwrap_or_default();
    let id = card
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| change_index.to_string());
    let comment = card_comments
        .get(&id)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    ReviewerCardPreview {
        id,
        change_index,
        kind: optional_string(card, "type")
            .or_else(|| optional_string(card, "kind"))
            .unwrap_or_default(),
        summary: optional_string(card, "summary").unwrap_or_default(),
        evidence: optional_string(card, "evidence").unwrap_or_default(),
        categories: string_array(card.get("categories").unwrap_or(&Value::Null)),
        matched_template_id: optional_string(card, "matchedTemplateId")
            .or_else(|| optional_string(card, "templateId")),
        template_title: optional_string(card, "templateTitle"),
        match_source: optional_string(card, "matchSource"),
        opinion_text: optional_string(card, "opinionText"),
        rationale: optional_string(card, "rationale"),
        comment,
    }
}

fn parse_reviewer_session_preview(payload: &str) -> Result<ReviewerSessionPreview, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let session_id = required_string(&value, "sessionId")?;
    let review_action_id = required_string(&value, "reviewActionId")?;
    let comments = value.get("comments").unwrap_or(&Value::Null);
    let session_comment = comments
        .get("sessionComment")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let card_comments = comments.get("cardComments").unwrap_or(&Value::Null);
    let backend_version = value.get("backendVersion").unwrap_or(&Value::Null);
    let backend_service = backend_version
        .get("service")
        .and_then(Value::as_str)
        .map(str::to_string);
    let backend_release = backend_version
        .get("release")
        .and_then(Value::as_str)
        .map(str::to_string);
    let cards = value
        .get("cards")
        .and_then(Value::as_array)
        .ok_or_else(|| "review session response did not include cards".to_string())?
        .iter()
        .map(|card| reviewer_card_preview_from_value(card, card_comments))
        .collect::<Vec<_>>();
    let suggestions_value = value
        .get("suggestions")
        .or_else(|| value.get("proposals"))
        .unwrap_or(&Value::Null);
    let suggestions = suggestions_value
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|suggestion| ReviewerSuggestionPreview {
                    proposal_id: optional_string(suggestion, "proposalId").unwrap_or_default(),
                    operation: optional_string(suggestion, "operation").unwrap_or_default(),
                    category: optional_string(suggestion, "category").unwrap_or_default(),
                    target_template_id: optional_string(suggestion, "targetTemplateId"),
                    title: optional_string(suggestion, "title").unwrap_or_default(),
                    description: optional_string(suggestion, "description"),
                    report_texts: string_array(
                        suggestion.get("reportTexts").unwrap_or(&Value::Null),
                    ),
                    reason: optional_string(suggestion, "reason").unwrap_or_default(),
                    source_card_ids: string_array(
                        suggestion.get("sourceCardIds").unwrap_or(&Value::Null),
                    ),
                    decision: optional_string(suggestion, "decision")
                        .unwrap_or_else(|| "pending".to_string()),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ReviewerSessionPreview {
        session_id,
        review_action_id,
        backend_service,
        backend_release,
        session_comment,
        cards,
        suggestions,
    })
}

fn parse_template_search_results(payload: &str) -> Result<Vec<ReviewerTemplatePreview>, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "template search response did not include results".to_string())?;
    Ok(results
        .iter()
        .map(|result| ReviewerTemplatePreview {
            id: optional_string(result, "id").unwrap_or_default(),
            title: optional_string(result, "title").unwrap_or_default(),
            description: optional_string(result, "description").unwrap_or_default(),
            category: optional_string(result, "category").unwrap_or_default(),
            score: result
                .get("score")
                .and_then(value_to_compact_string)
                .unwrap_or_default(),
        })
        .collect())
}

fn parse_draft_preview(
    project: &Project,
    payload: &str,
) -> Result<(Vec<DraftRowPreview>, Option<DraftSummaryPreview>), String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let rows = parse_draft_rows_response(payload)?
        .into_iter()
        .map(|row| DraftRowPreview {
            original_text: project
                .segment(&row.row_id)
                .map(|segment| segment.text.clone())
                .unwrap_or_default(),
            row_id: row.row_id,
            rewritten_text: row.rewritten_text,
            status: row.status,
            warnings: row.warnings,
            applied: false,
            skipped: false,
        })
        .collect::<Vec<_>>();
    let summary = value.get("summary").map(|summary| DraftSummaryPreview {
        total_rows: summary_usize(summary, "totalRows"),
        rewritten_rows: summary_usize(summary, "rewrittenRows"),
        unchanged_rows: summary_usize(summary, "unchangedRows"),
        failed_rows: summary_usize(summary, "failedRows"),
    });
    Ok((rows, summary))
}

fn parse_transcription_preview(payload: &str) -> Result<DraftTranscriptionPreview, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let text = required_string(&value, "text")?;
    Ok(DraftTranscriptionPreview {
        segment_id: None,
        original_text: String::new(),
        text,
        model: optional_string(&value, "model"),
    })
}

fn parse_transcription_preview_for_project(
    project: &Project,
    request_payload: &Value,
    payload: &str,
) -> Result<DraftTranscriptionPreview, String> {
    let mut preview = parse_transcription_preview(payload)?;
    if let Some(segment_id) = transcription_request_segment_id(request_payload) {
        preview.segment_id = Some(segment_id.clone());
        if let Some(segment) = project.segment(&segment_id) {
            preview.original_text = segment.text.clone();
        }
    }
    Ok(preview)
}

fn transcription_request_segment_id(payload: &Value) -> Option<String> {
    payload
        .pointer("/segment/rowId")
        .and_then(value_to_compact_string)
        .or_else(|| {
            payload
                .pointer("/segment/id")
                .and_then(value_to_compact_string)
        })
        .or_else(|| payload.get("rowId").and_then(value_to_compact_string))
        .or_else(|| payload.get("segmentId").and_then(value_to_compact_string))
}

fn parse_redistribution_previews(
    request_payload: &Value,
    payload: &str,
) -> Result<Vec<DraftRedistributionPreview>, String> {
    let value = serde_json::from_str::<Value>(payload)
        .map_err(|error| format!("malformed JSON: {error}"))?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| "redistribution response did not include results".to_string())?;
    let groups = request_payload
        .get("groups")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(results
        .iter()
        .map(|result| {
            let group_id = optional_string(result, "groupId").unwrap_or_default();
            let group = groups.iter().find(|group| {
                group.get("groupId").and_then(Value::as_str) == Some(group_id.as_str())
            });
            let allocations = group
                .and_then(|group| group.get("draftAllocations"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|allocation| DraftRedistributionAllocationPreview {
                            segment_id: optional_string(allocation, "segmentId")
                                .unwrap_or_default(),
                            text: optional_string(allocation, "text").unwrap_or_default(),
                        })
                        .filter(|allocation| !allocation.segment_id.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let review = result.get("review").unwrap_or(&Value::Null);
            let mut move_errors = Vec::new();
            let moves = review
                .get("moves")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .enumerate()
                        .filter_map(|(index, item)| {
                            match redistribution_move_preview_from_value(item) {
                                Ok(move_item) => Some(move_item),
                                Err(error) => {
                                    move_errors.push(format!(
                                        "Invalid redistribution move {}: {error}.",
                                        index + 1
                                    ));
                                    None
                                }
                            }
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let accept_draft = review
                .get("acceptDraft")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let validation_error = if !move_errors.is_empty() {
                Some(move_errors.join(" "))
            } else if !accept_draft && !moves.is_empty() {
                apply_redistribution_moves_to_allocations(&allocations, &moves)
                    .err()
                    .map(|error| {
                        error
                            .strip_prefix("Apply redistribution blocked: ")
                            .unwrap_or(&error)
                            .to_string()
                    })
            } else {
                None
            };
            DraftRedistributionPreview {
                group_id,
                model: optional_string(result, "model")
                    .or_else(|| optional_string(&value, "model")),
                accept_draft,
                notes: optional_string(review, "notes"),
                moves,
                allocations,
                error: optional_string(result, "error").or(validation_error),
                applied: false,
            }
        })
        .collect())
}

pub(super) fn apply_redistribution_moves_to_allocations(
    allocations: &[DraftRedistributionAllocationPreview],
    moves: &[DraftRedistributionMovePreview],
) -> Result<Vec<DraftRedistributionAllocationPreview>, String> {
    let mut next = allocations.to_vec();
    for move_item in moves {
        if move_item.from_index == 0
            || move_item.to_index == 0
            || move_item.sentence_count == 0
            || move_item.from_index.abs_diff(move_item.to_index) != 1
        {
            return Err(format!(
                "Apply redistribution blocked: invalid model move {}->{} x{}.",
                move_item.from_index, move_item.to_index, move_item.sentence_count
            ));
        }
        let from = move_item.from_index - 1;
        let to = move_item.to_index - 1;
        if from >= next.len() || to >= next.len() {
            return Err(format!(
                "Apply redistribution blocked: model move {}->{} is outside {} allocations.",
                move_item.from_index,
                move_item.to_index,
                next.len()
            ));
        }

        let mut source_sentences = split_sentences_preserving_text(&next[from].text);
        let mut target_sentences = split_sentences_preserving_text(&next[to].text);
        if source_sentences.len() < move_item.sentence_count {
            return Err(format!(
                "Apply redistribution blocked: allocation {} has only {} sentence(s), cannot move {}.",
                move_item.from_index,
                source_sentences.len(),
                move_item.sentence_count
            ));
        }
        let moved = if to > from {
            source_sentences.split_off(source_sentences.len() - move_item.sentence_count)
        } else {
            source_sentences
                .drain(0..move_item.sentence_count)
                .collect::<Vec<_>>()
        };
        if to > from {
            let mut combined = moved;
            combined.extend(target_sentences);
            target_sentences = combined;
        } else {
            target_sentences.extend(moved);
        }
        next[from].text = join_sentence_parts(&source_sentences);
        next[to].text = join_sentence_parts(&target_sentences);
    }
    Ok(next)
}

fn split_sentences_preserving_text(value: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if matches!(ch, '.' | '?' | '!' | '…') {
            let end = index + ch.len_utf8();
            let sentence = value[start..end].trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_string());
            }
            start = end;
        }
    }
    let tail = value[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    if sentences.is_empty() && !value.trim().is_empty() {
        sentences.push(value.trim().to_string());
    }
    sentences
}

fn join_sentence_parts(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn redistribution_move_preview_from_value(
    value: &Value,
) -> Result<DraftRedistributionMovePreview, String> {
    Ok(DraftRedistributionMovePreview {
        from_index: required_move_usize(value, "fromIndex")?,
        to_index: required_move_usize(value, "toIndex")?,
        sentence_count: required_move_usize(value, "sentenceCount")?,
    })
}

fn required_move_usize(value: &Value, key: &str) -> Result<usize, String> {
    value
        .get(key)
        .and_then(value_usize)
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("missing `{key}`"))
}

fn required_string(value: &Value, key: &str) -> Result<String, String> {
    optional_string(value, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("missing `{key}`"))
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(value_to_compact_string)
}

fn value_to_compact_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.trim().is_empty() => Some(value.trim().to_string()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

fn string_array(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(value_to_compact_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn summary_usize(value: &Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
        .unwrap_or_default() as usize
}

fn value_usize(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse::<u64>().ok())
        .map(|value| value as usize)
}

fn find_number_by_key(value: &Value, key: &str) -> Option<f64> {
    match value {
        Value::Object(object) => {
            if let Some(number) = object.get(key).and_then(Value::as_f64) {
                return Some(number);
            }
            object
                .values()
                .find_map(|child| find_number_by_key(child, key))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_number_by_key(child, key)),
        _ => None,
    }
}

fn find_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => {
            for key in keys {
                if let Some(value) = object.get(*key).and_then(value_to_compact_string) {
                    return Some(value);
                }
            }
            object
                .values()
                .find_map(|child| find_string_by_keys(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_by_keys(child, keys)),
        _ => None,
    }
}

fn compact_number(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_draft_session_id_does_not_defeat_pending_equivalence() {
        let pending = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
            "draftSessionId": "job-1:2026-07-23T10:00:00Z",
        });
        let requested = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
        });

        assert!(draft_generate_payloads_are_equivalent(&pending, &requested));
    }

    #[test]
    fn explicit_or_changed_draft_inputs_remain_distinct() {
        let pending = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
            "model": "model-a",
            "draftSessionId": "session-a",
        });
        let changed_model = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
            "model": "model-b",
        });
        let changed_session = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
            "model": "model-a",
            "draftSessionId": "session-b",
        });

        assert!(!draft_generate_payloads_are_equivalent(
            &pending,
            &changed_model
        ));
        assert!(!draft_generate_payloads_are_equivalent(
            &pending,
            &changed_session
        ));
    }
    #[test]
    fn queued_audio_generation_is_detected_before_encoding_again() {
        let mut state = StudioState::default();
        let pending = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
            "draftSessionId": "job-1:2026-07-23T10:00:00Z",
        });
        state
            .review_backend_requests
            .push(ReviewBackendRequest::DraftGenerateStreamWithAudio {
                payload: pending,
                audio_tracks: Vec::new(),
            });
        let requested = json!({
            "jobId": "job-1",
            "rows": [{"rowId": "row-1"}],
        });

        assert!(state.has_pending_audio_draft_generation(&requested));
    }
}
