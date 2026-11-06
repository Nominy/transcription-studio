//! Request payloads for the Reviewer/Drafting backend built from the current project, plus the
//! project-metadata lookups they share (review action id, review level).

use anyhow::{Result, anyhow, bail};
use serde_json::{Map, Value, json};
use studio_core::{AudioLane, Project, Segment};
use time::OffsetDateTime;

use super::model::{GeneratedReview, ReviewerSession};
use exaudio_babel_domain::i18n::localized;
use exaudio_gold::polish::DraftingConfig;

fn not_a_babel_task() -> &'static str {
    localized(
        "Magic Review is only available for Babel tasks.",
        "Magic Review доступен только для задач Babel.",
    )
}

fn review_tasks_only() -> &'static str {
    localized(
        "Magic Review is only available for review tasks.",
        "Magic Review доступен только для задач ревью.",
    )
}

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
    let review_action_id =
        review_action_id(project).ok_or_else(|| anyhow!("{}", not_a_babel_task()))?;
    let package_extra = package_extra(project);
    let captured_at = current_utc_timestamp();
    Ok(json!({
        "reviewActionId": review_action_id,
        "original": normalized_review_action(project, &review_action_id, &captured_at, true, &package_extra),
        "current": normalized_review_action(project, &review_action_id, &captured_at, false, &package_extra),
        "babelDiff": Value::Null,
    }))
}

/// `(reviewActionId, current normalized state)` of an L2+ review task.
pub(super) fn current_review_action(project: &Project) -> Result<(String, Value)> {
    let review_action_id =
        review_action_id(project).ok_or_else(|| anyhow!("{}", not_a_babel_task()))?;
    if project_review_action_level(project).unwrap_or(0) <= 1 {
        bail!("{}", review_tasks_only());
    }
    let current = normalized_review_action(
        project,
        &review_action_id,
        &current_utc_timestamp(),
        false,
        &package_extra(project),
    );
    Ok((review_action_id, current))
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
        .ok_or_else(|| {
            anyhow!(
                "{}",
                localized("Select a segment first.", "Сначала выберите сегмент.")
            )
        })
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

pub(super) fn current_utc_timestamp() -> String {
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
