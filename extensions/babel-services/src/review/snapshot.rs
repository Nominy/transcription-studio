//! Immutable original/current transcript pair sent to Magic Review and the Grader.
//!
//! `current` is the open L2 task as edited in Studio; `original` is the task's stable L1
//! review action fetched from Babel (`getReviewActionsForChunk` → level-1 action →
//! `getReviewActionDataById`), as the Review Helper does.

use anyhow::{Result, anyhow, bail};
use exaudio_babel_domain::i18n::{localized, with_detail};
use serde_json::{Value, json};
use studio_core::Project;

use crate::babel::{BabelClient, BabelRequest};

fn original_missing_message() -> &'static str {
    localized(
        "Couldn't find the original transcript for this task.",
        "Не удалось найти исходную расшифровку для этой задачи.",
    )
}

/// One captured review pair. [`ReviewSnapshot::transcript_key`] identifies the transcript
/// content so results can be refused after the transcript changed.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewSnapshot {
    pub review_action_id: String,
    pub original: Value,
    pub current: Value,
    transcript_key: String,
}

impl ReviewSnapshot {
    /// Validates the pair: current is the L2+ action itself, original a distinct level-1 action.
    pub fn new(review_action_id: String, original: Value, current: Value) -> Result<Self> {
        fn action_id(state: &Value) -> &str {
            state.get("actionId").and_then(Value::as_str).unwrap_or("")
        }
        let level = |state: &Value| {
            state
                .get("actionLevel")
                .and_then(Value::as_i64)
                .unwrap_or(0)
        };
        if action_id(&current) != review_action_id || level(&current) <= 1 {
            bail!(
                "{}",
                localized(
                    "Magic Review is only available for review tasks.",
                    "Magic Review доступен только для задач ревью.",
                )
            );
        }
        if action_id(&original).is_empty()
            || action_id(&original) == review_action_id
            || level(&original) != 1
        {
            bail!("{}", original_missing_message());
        }
        let transcript_key = transcript_key(&original, &current);
        Ok(Self {
            review_action_id,
            original,
            current,
            transcript_key,
        })
    }

    /// Identity of the reviewed transcripts (ignores capture times).
    pub fn transcript_key(&self) -> &str {
        &self.transcript_key
    }

    /// `/api/review/generate` and `/api/review/sessions` body.
    pub fn prepare_body(&self) -> Value {
        json!({
            "reviewActionId": self.review_action_id,
            "original": self.original,
            "current": self.current,
            "babelDiff": Value::Null,
        })
    }

    /// `/api/review/grade` body.
    pub fn grade_body(&self) -> Value {
        json!({
            "reviewActionId": self.review_action_id,
            "original": self.original,
            "current": self.current,
        })
    }
}

/// Transcript identity of the project's current state against `original` (compare with
/// [`ReviewSnapshot::transcript_key`] before applying results).
pub fn current_transcript_key(project: &Project, original: &Value) -> Result<String> {
    let (_, current) = super::payload::current_review_action(project)?;
    Ok(transcript_key(original, &current))
}

fn transcript_key(original: &Value, current: &Value) -> String {
    json!({
        "original": original.get("actionId"),
        "originalAnnotations": original.get("annotations"),
        "current": current.get("actionId"),
        "annotations": current.get("annotations"),
        "recordings": current.get("recordings"),
    })
    .to_string()
}

/// Fetches the stable L1 original of `review_action_id` from Babel as a normalized state.
pub async fn load_l1_original(babel: &BabelClient, review_action_id: &str) -> Result<Value> {
    let list = babel
        .review_actions_for_chunk(review_action_id)
        .await
        .map_err(|error| anyhow!("{error}"))?;
    let reference_id = l1_reference_id(&list, review_action_id)?;
    let raw = babel
        .raw(&BabelRequest::LoadReviewAction {
            review_action_id: reference_id.clone(),
        })
        .await
        .map_err(|error| {
            anyhow!(
                "{}",
                with_detail(
                    localized(
                        "Couldn't load the original transcript.",
                        "Не удалось загрузить исходную расшифровку.",
                    ),
                    error,
                )
            )
        })?;
    let original = normalize_review_action(&raw)?;
    if original.get("actionId").and_then(Value::as_str) != Some(reference_id.as_str()) {
        bail!(
            "{}",
            localized(
                "Couldn't read the original transcript. Reload the task.",
                "Не удалось прочитать исходную расшифровку. Перезагрузите задачу.",
            )
        );
    }
    Ok(original)
}

/// Id of the level-1 action in the chunk's review list.
pub fn l1_reference_id(payload: &str, review_action_id: &str) -> Result<String> {
    let value: Value = serde_json::from_str(payload.trim())
        .map_err(|_| anyhow!("The server sent an unexpected reply."))?;
    let list = find_array_with_levels(&value)
        .ok_or_else(|| anyhow!("The server sent an unexpected reply."))?;
    list.iter()
        .find_map(|item| {
            let level = item.get("level").and_then(|level| {
                level
                    .as_i64()
                    .or_else(|| level.as_str()?.trim().parse().ok())
            });
            let id = item.get("id").and_then(Value::as_str).map(str::trim)?;
            (level == Some(1) && !id.is_empty()).then(|| id.to_string())
        })
        .filter(|id| id != review_action_id)
        .ok_or_else(|| anyhow!("{}", original_missing_message()))
}

fn find_array_with_levels(value: &Value) -> Option<&Vec<Value>> {
    match value {
        Value::Array(items) if items.iter().any(|item| item.get("level").is_some()) => Some(items),
        Value::Array(items) => items.iter().find_map(find_array_with_levels),
        Value::Object(object) => object.values().find_map(find_array_with_levels),
        _ => None,
    }
}

/// Normalizes a `getReviewActionDataById` response like the Review Helper's parser.
pub fn normalize_review_action(payload: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(payload.trim())
        .map_err(|_| anyhow!("The server sent an unreadable reply."))?;
    let raw = find_review_action(&value)
        .ok_or_else(|| anyhow!("The server's reply had no annotations."))?;
    let number = |value: Option<&Value>| {
        value
            .and_then(|value| {
                value
                    .as_f64()
                    .or_else(|| value.as_str()?.trim().parse().ok())
            })
            .filter(|number| number.is_finite())
            .unwrap_or(0.0)
    };
    let text = |value: Option<&Value>| match value {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    let string = |value: Option<&Value>| value.and_then(Value::as_str).unwrap_or("").to_string();
    let mut annotations: Vec<Value> = raw
        .get("annotations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, item)| {
            json!({
                "id": item.get("id").and_then(Value::as_str).map_or_else(|| format!("idx-{index}"), str::to_string),
                "reviewActionId": string(item.get("reviewActionId")),
                "type": string(item.get("type")),
                "content": text(item.get("content")),
                "processedRecordingId": string(item.get("processedRecordingId")),
                "startTimeInSeconds": number(item.get("startTimeInSeconds")),
                "endTimeInSeconds": number(item.get("endTimeInSeconds")),
                "metadata": item.get("metadata").filter(|value| value.is_object()).cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    annotations.sort_by(|a, b| {
        number(a.get("startTimeInSeconds")).total_cmp(&number(b.get("startTimeInSeconds")))
    });
    let mut recordings: Vec<Value> = raw
        .get("transcriptionChunkProcessedRecordings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|item| {
            json!({
                "id": string(item.get("id")),
                "transcriptionChunkId": string(item.get("transcriptionChunkId")),
                "processedRecordingId": string(item.get("processedRecordingId")),
                "speaker": number(item.get("speaker")),
                "startTimeInSeconds": number(item.get("startTimeInSeconds")),
                "endTimeInSeconds": number(item.get("endTimeInSeconds")),
            })
        })
        .collect();
    recordings.sort_by(|a, b| {
        number(a.get("startTimeInSeconds")).total_cmp(&number(b.get("startTimeInSeconds")))
    });
    let action_id = raw
        .get("actionId")
        .or_else(|| raw.get("reviewActionId"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(json!({
        "actionId": action_id,
        "actionLevel": number(raw.get("actionLevel")) as i64,
        "actionDecision": string(raw.get("actionDecision")),
        "annotations": annotations,
        "recordings": recordings,
        "lintErrors": raw.get("lintErrors").filter(|value| value.is_array()).cloned().unwrap_or_else(|| json!([])),
        "capturedAt": super::payload::current_utc_timestamp(),
    }))
}

fn find_review_action(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    match value {
        Value::Object(object)
            if object.get("annotations").is_some_and(Value::is_array)
                && (object.get("actionId").is_some_and(Value::is_string)
                    || object.get("reviewActionId").is_some_and(Value::is_string)) =>
        {
            Some(object)
        }
        Value::Object(object) => object.values().find_map(find_review_action),
        Value::Array(items) => items.iter().find_map(find_review_action),
        _ => None,
    }
}

/// Captures the pair for `project`: current from the editor, original from Babel.
pub async fn capture_review_snapshot(
    babel: &BabelClient,
    review_action_id: String,
    current: Value,
) -> Result<ReviewSnapshot> {
    let original = load_l1_original(babel, &review_action_id).await?;
    ReviewSnapshot::new(review_action_id, original, current)
}

/// `(reviewActionId, current state)` of the open task; fails unless it is an L2 review.
pub fn current_review_state(project: &Project) -> Result<(String, Value)> {
    super::payload::current_review_action(project)
}
