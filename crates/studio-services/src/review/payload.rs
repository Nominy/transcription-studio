//! Request payloads for the Reviewer/Drafting backend built from the current project, plus the
//! project-metadata lookups they share (review action id, review level).

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};
use studio_core::{AudioLane, Project, Segment};
use time::OffsetDateTime;

use super::model::{GeneratedReview, ReviewerSession};

pub const DRAFT_OPENROUTER_API_KEY_ENV: &str = "STUDIO_DRAFT_OPENROUTER_API_KEY";
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// Project metadata key holding the reviewer session id selected for this task.
pub const META_REVIEWER_SELECTED_SESSION_ID: &str = "studio.native.reviewer.sessionId";

const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_EXTRA_TRACK_PREFIX: &str = "babel.extra.track.";
const META_REVIEW_ACTION_DECISION: &str = "babel.review_action.actionDecision";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";
const META_ORIGINAL_PREFIX: &str = "studio.babel.original.";
const META_ORIGINAL_TEXT: &str = "studio.babel.original.text";
const META_ORIGINAL_START_SECONDS: &str = "studio.babel.original.startSeconds";
const META_ORIGINAL_END_SECONDS: &str = "studio.babel.original.endSeconds";
const META_ORIGINAL_LANE_ID: &str = "studio.babel.original.laneId";
const META_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
const REVIEWER_ACTION_LEVEL: i64 = 2;
const ACTION_LEVEL_METADATA_KEYS: &[&str] = &[
    "action_level",
    "actionLevel",
    "review_action_level",
    "reviewActionLevel",
    "studio.babel.actionLevel",
    "studio.babel.reviewActionLevel",
];
const ACTION_LEVEL_JSON_KEYS: &[&str] = &[
    "action_level",
    "actionLevel",
    "review_action_level",
    "reviewActionLevel",
    "level",
];

/// Drafting inputs from the Gold Drafting tab. Empty `openrouter_api_key` falls back to
/// `STUDIO_DRAFT_OPENROUTER_API_KEY`, then `OPENROUTER_API_KEY`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DraftingConfig {
    pub openrouter_api_key: String,
    pub model: String,
    /// Omitted from payloads when empty or `default`.
    pub service_tier: String,
    /// Omitted from payloads when empty or `default`.
    pub reasoning_effort: String,
    pub draft_session_id: String,
}

impl DraftingConfig {
    /// Trims every field.
    pub fn from_inputs(
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

    fn resolved_api_key(&self) -> Result<String> {
        let key = if self.openrouter_api_key.is_empty() {
            std::env::var(DRAFT_OPENROUTER_API_KEY_ENV)
                .or_else(|_| std::env::var(OPENROUTER_API_KEY_ENV))
                .unwrap_or_default()
                .trim()
                .to_string()
        } else {
            self.openrouter_api_key.clone()
        };
        if key.is_empty() {
            bail!(
                "drafting requires `{DRAFT_OPENROUTER_API_KEY_ENV}` or `{OPENROUTER_API_KEY_ENV}`"
            );
        }
        Ok(key)
    }

    fn add_optional_fields(&self, payload: &mut Value) {
        let Some(object) = payload.as_object_mut() else {
            return;
        };
        if !self.model.is_empty() {
            object.insert("model".to_string(), json!(self.model));
        }
        if !self.service_tier.is_empty() && self.service_tier != "default" {
            object.insert("serviceTier".to_string(), json!(self.service_tier));
        }
        if !self.reasoning_effort.is_empty() && self.reasoning_effort != "default" {
            object.insert("reasoningEffort".to_string(), json!(self.reasoning_effort));
        }
        if !self.draft_session_id.is_empty() {
            object.insert("draftSessionId".to_string(), json!(self.draft_session_id));
        }
    }
}

/// Review level from project metadata (`action_level`/`actionLevel`/... aliases, `L2`/`l2`
/// literals, or nested JSON incl. `babel.extra.package`).
pub fn project_review_action_level(project: &Project) -> Option<i64> {
    ACTION_LEVEL_METADATA_KEYS
        .iter()
        .chain(std::iter::once(&META_EXTRA_PACKAGE))
        .find_map(|key| {
            project
                .metadata
                .get(*key)
                .and_then(|value| review_action_level_from_text(value))
        })
}

/// The Reviewer tab is only available for L2 review tasks (otherwise the panel opens on
/// Gold Drafting).
pub fn reviewer_capability_available(project: &Project) -> bool {
    project_review_action_level(project) == Some(REVIEWER_ACTION_LEVEL)
}

/// Babel review action id: project metadata, then segment metadata, then the package extra,
/// then the raw review-action envelope.
pub fn review_action_id(project: &Project) -> Option<String> {
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
        .or_else(|| {
            let envelope = project
                .metadata
                .get(META_REVIEW_ACTION_ENVELOPE)
                .and_then(|encoded| serde_json::from_str::<Value>(encoded).ok())?;
            find_object_with_annotations(&envelope)
                .and_then(|object| string_from_map(object, &["actionId", "reviewActionId"]))
        })
}

/// `{ reviewActionId, original, current, babelDiff: null }` for review generate/create session.
/// For L2 tasks `original` is the L1 baseline (`actionLevel` 1, decision `pass`, original texts
/// and timings preserved in segment metadata).
pub fn build_reviewer_payload(project: &Project) -> Result<Value> {
    let review_action_id = review_action_id(project)
        .ok_or_else(|| anyhow!("current project has no Babel review action id"))?;
    let package_extra = package_extra(project);
    let captured_at = current_utc_timestamp();
    Ok(json!({
        "reviewActionId": review_action_id,
        "original": normalized_review_action(project, &review_action_id, &captured_at, true, &package_extra),
        "current": normalized_review_action(project, &review_action_id, &captured_at, false, &package_extra),
        "babelDiff": Value::Null,
    }))
}

/// Analytics submission payload (also sent best-effort on L2 submit): reviewer payload +
/// `inputBoxes` (generated feedback categories, non-empty session/card comments),
/// `prepared`/`aiReview` from the generated review, and session ids in `metadata`.
pub fn build_reviewer_analytics_payload(
    project: &Project,
    session: Option<&ReviewerSession>,
    generated_review: Option<&GeneratedReview>,
) -> Result<Value> {
    let Value::Object(mut payload) = build_reviewer_payload(project)? else {
        unreachable!("reviewer payload is an object");
    };
    let mut metadata = Map::new();
    metadata.insert("client".to_string(), json!("transcription-studio-native"));
    metadata.insert("source".to_string(), json!("native-capability-panel"));
    metadata.insert("capturedAt".to_string(), json!(current_utc_timestamp()));
    if let Some(session) = session {
        metadata.insert("sessionId".to_string(), json!(session.session_id));
        metadata.insert(
            "reviewActionId".to_string(),
            json!(session.review_action_id),
        );
    }
    payload.insert(
        "inputBoxes".to_string(),
        reviewer_input_boxes(session, generated_review),
    );
    payload.insert(
        "aiReview".to_string(),
        generated_review.map_or(Value::Null, |review| review.ai_review.clone()),
    );
    if let Some(generated_review) = generated_review {
        payload.insert("prepared".to_string(), generated_review.prepared.clone());
    }
    payload.insert("metadata".to_string(), Value::Object(metadata));
    Ok(Value::Object(payload))
}

/// `/api/draft/generate[/stream]` payload with every project row (sorted by start).
pub fn build_draft_generate_payload(project: &Project, config: &DraftingConfig) -> Result<Value> {
    let api_key = config.resolved_api_key()?;
    let rows = sorted_segments(project)
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
        .collect::<Vec<_>>();
    let mut payload = json!({
        "projectPreset": "ru-gold-2sp-v1",
        "jobId": review_action_id(project).unwrap_or_else(|| project.id.clone()),
        "rows": rows,
        "openRouterApiKey": api_key,
    });
    config.add_optional_fields(&mut payload);
    Ok(payload)
}

/// Adds `draftSessionId = <jobId>:<utc timestamp>` when absent so a dropped stream can be
/// reconciled (see [`super::ReviewBackend::generate_drafts`]).
pub fn ensure_draft_session_id(payload: &mut Value) {
    if super::client::draft_payload_has_session_id(payload) {
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

/// Whether a requested generation duplicates a pending one. A generated `draftSessionId` on the
/// pending payload is ignored unless the request names its own session.
pub fn draft_generate_payloads_are_equivalent(pending: &Value, requested: &Value) -> bool {
    if super::client::draft_payload_has_session_id(requested) {
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

/// `/api/broker/transcribe-segment` payload for the selected segment (or the first one).
pub fn build_draft_transcribe_segment_payload(
    project: &Project,
    selected_segment_id: Option<&str>,
    config: &DraftingConfig,
) -> Result<Value> {
    let api_key = config.resolved_api_key()?;
    let segment = selected_segment(project, selected_segment_id)?;
    let mut payload = json!({
        "openRouterApiKey": api_key,
        "segment": {
            "rowId": segment.id,
            "speakerKey": segment.speaker_id,
            "startSeconds": segment.start_seconds,
            "endSeconds": segment.end_seconds,
        }
    });
    config.add_optional_fields(&mut payload);
    Ok(payload)
}

/// `/api/broker/redistribute-text` payload: one group with every segment of the selected
/// segment's speaker and their current texts as draft allocations.
pub fn build_draft_redistribute_payload(
    project: &Project,
    selected_segment_id: Option<&str>,
    config: &DraftingConfig,
) -> Result<Value> {
    let api_key = config.resolved_api_key()?;
    let selected = selected_segment(project, selected_segment_id)?;
    let same_speaker = sorted_segments(project)
        .into_iter()
        .enumerate()
        .filter(|(_, segment)| segment.speaker_id == selected.speaker_id)
        .collect::<Vec<_>>();
    let rows = same_speaker
        .iter()
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
    let draft_allocations = same_speaker
        .iter()
        .map(|(_, segment)| json!({ "segmentId": segment.id, "text": segment.text }))
        .collect::<Vec<_>>();
    let full_text = same_speaker
        .iter()
        .map(|(_, segment)| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let mut payload = json!({
        "openRouterApiKey": api_key,
        "groups": [{
            "groupId": format!("{}:{}", project.id, selected.speaker_id),
            "speakerKey": selected.speaker_id,
            "fullText": full_text,
            "segments": rows,
            "draftAllocations": draft_allocations,
        }]
    });
    config.add_optional_fields(&mut payload);
    Ok(payload)
}

/// Selected reviewer session id persisted in project metadata (empty when unset).
pub fn reviewer_session_id_from_project(project: &Project) -> String {
    project
        .metadata
        .get(META_REVIEWER_SELECTED_SESSION_ID)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn reviewer_input_boxes(
    session: Option<&ReviewerSession>,
    generated_review: Option<&GeneratedReview>,
) -> Value {
    let categories = generated_review
        .into_iter()
        .flat_map(|review| &review.feedback)
        .map(|feedback| {
            (
                feedback.category.clone(),
                json!({ "score": feedback.score, "note": feedback.note }),
            )
        })
        .collect::<Map<_, _>>();
    let mut notes = Vec::new();
    if let Some(session) = session {
        if !session.session_comment.trim().is_empty() {
            notes.push(json!({ "scope": "session", "comment": session.session_comment }));
        }
        notes.extend(
            session
                .cards
                .iter()
                .filter(|card| !card.comment.trim().is_empty())
                .map(|card| json!({ "scope": "card", "cardId": card.id, "comment": card.comment })),
        );
    }
    json!({ "categories": categories, "notes": notes })
}

fn normalized_review_action(
    project: &Project,
    review_action_id: &str,
    captured_at: &str,
    original: bool,
    package_extra: &Map<String, Value>,
) -> Value {
    let current_level = project_review_action_level(project).unwrap_or(0);
    let l1_baseline = original && current_level == REVIEWER_ACTION_LEVEL;
    let action_level = if l1_baseline { 1 } else { current_level };
    let action_decision = if l1_baseline {
        "pass".to_string()
    } else {
        project
            .metadata
            .get(META_REVIEW_ACTION_DECISION)
            .cloned()
            .or_else(|| string_from_map(package_extra, &["action_decision", "actionDecision"]))
            .unwrap_or_default()
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
    let lane_id = if original {
        metadata_string(segment, META_ORIGINAL_LANE_ID).unwrap_or_else(|| segment.lane_id.clone())
    } else {
        segment.lane_id.clone()
    };
    let (content, start, end) = if original {
        (
            metadata_string(segment, META_ORIGINAL_TEXT).unwrap_or_else(|| segment.text.clone()),
            metadata_f64(segment, META_ORIGINAL_START_SECONDS).unwrap_or(segment.start_seconds),
            metadata_f64(segment, META_ORIGINAL_END_SECONDS).unwrap_or(segment.end_seconds),
        )
    } else {
        (
            metadata_string(segment, "content").unwrap_or_else(|| segment.text.clone()),
            segment.start_seconds,
            segment.end_seconds,
        )
    };
    let metadata = segment
        .metadata
        .iter()
        .filter(|(key, _)| !key.starts_with(META_ORIGINAL_PREFIX))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    json!({
        "id": segment.source_annotation_id.as_deref().unwrap_or(segment.id.as_str()),
        "reviewActionId": metadata_string(segment, "reviewActionId").unwrap_or_else(|| review_action_id.to_string()),
        "type": metadata_string(segment, "type").unwrap_or_else(|| "transcription".to_string()),
        "content": content,
        "processedRecordingId": metadata_string(segment, "processedRecordingId")
            .or_else(|| string_from_map(&track_extra(project, &lane_id), &["processed_recording_id", "processedRecordingId"]))
            .unwrap_or_default(),
        "startTimeInSeconds": start,
        "endTimeInSeconds": end,
        "metadata": if metadata.is_empty() { Value::Null } else { Value::Object(metadata) },
    })
}

fn review_recordings(project: &Project) -> Vec<Value> {
    let mut lanes = project.lanes.iter().collect::<Vec<_>>();
    lanes.sort_by(|left, right| left.id.cmp(&right.id));
    lanes
        .into_iter()
        .enumerate()
        .map(|(index, lane)| {
            let extra = track_extra(project, &lane.id);
            let (start, end) = lane_bounds(project, &lane.id).unwrap_or((0.0, 0.0));
            let processed_recording_id =
                string_from_map(&extra, &["processed_recording_id", "processedRecordingId"]);
            json!({
                "id": string_from_map(&extra, &["chunked_processed_recording_id", "chunkedProcessedRecordingId", "id"])
                    .or_else(|| processed_recording_id.clone())
                    .unwrap_or_else(|| lane.id.clone()),
                "transcriptionChunkId": string_from_map(&extra, &["transcription_chunk_id", "transcriptionChunkId"])
                    .unwrap_or_else(|| project.id.clone()),
                "processedRecordingId": processed_recording_id.unwrap_or_else(|| lane.id.clone()),
                "speaker": speaker_number(lane).unwrap_or(index as i64 + 1),
                "startTimeInSeconds": start,
                "endTimeInSeconds": end,
            })
        })
        .collect()
}

fn selected_segment<'a>(
    project: &'a Project,
    selected_segment_id: Option<&str>,
) -> Result<&'a Segment> {
    selected_segment_id
        .and_then(|id| project.segment(id))
        .or_else(|| sorted_segments(project).into_iter().next())
        .ok_or_else(|| anyhow!("selected segment is not available"))
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

fn lane_bounds(project: &Project, lane_id: &str) -> Option<(f64, f64)> {
    project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id)
        .map(|segment| (segment.start_seconds, segment.end_seconds))
        .reduce(|(start, end), (next_start, next_end)| (start.min(next_start), end.max(next_end)))
}

fn speaker_number(lane: &AudioLane) -> Option<i64> {
    lane.speaker_id
        .chars()
        .filter(char::is_ascii_digit)
        .collect::<String>()
        .parse::<i64>()
        .ok()
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
        .and_then(|value| match value {
            Value::Object(object) => Some(object),
            _ => None,
        })
        .unwrap_or_default()
}

fn metadata_string(segment: &Segment, key: &str) -> Option<String> {
    segment.metadata.get(key).and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        _ => None,
    })
}

fn metadata_f64(segment: &Segment, key: &str) -> Option<f64> {
    segment.metadata.get(key).and_then(Value::as_f64)
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

fn string_from_map(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key).and_then(|value| match value {
            Value::String(value) if !value.trim().is_empty() => Some(value.clone()),
            Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn review_action_level_from_text(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    parse_level_literal(value).or_else(|| {
        serde_json::from_str::<Value>(value)
            .ok()
            .and_then(|json| review_action_level_from_json(&json))
    })
}

fn review_action_level_from_json(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(value) => parse_level_literal(value),
        Value::Object(map) => ACTION_LEVEL_JSON_KEYS
            .iter()
            .find_map(|key| map.get(*key).and_then(review_action_level_from_json)),
        _ => None,
    }
}

fn parse_level_literal(value: &str) -> Option<i64> {
    let value = value.trim();
    value
        .strip_prefix(['L', 'l'])
        .unwrap_or(value)
        .parse::<i64>()
        .ok()
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
