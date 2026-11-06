//! Shared drafting protocol values and parsers; no platform metadata interpretation.

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use serde_json::Value;
use studio_core::Project;

fn parse_json(payload: &str) -> Result<Value> {
    serde_json::from_str(payload).context("malformed JSON")
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

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(value_to_compact_string).collect())
        .unwrap_or_default()
}
/// `/health` summary for the capability panel header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendHealth {
    pub service: String,
    pub backend_version: Option<String>,
    /// `credits remaining: N` or `credits unavailable: <error>`.
    pub openrouter_status: Option<String>,
    pub test_mode: Option<bool>,
    /// `provider: x`, `model: y`, `tier: z`, `reasoning: r` (whichever are reported).
    pub provider_diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRow {
    pub row_id: String,
    pub rewritten_text: String,
    /// `rewritten` rows are applicable; others (`unchanged`, `failed`, ...) are informational.
    pub status: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftSummary {
    pub total_rows: usize,
    pub rewritten_rows: usize,
    pub unchanged_rows: usize,
    pub failed_rows: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftResult {
    pub rows: Vec<DraftRow>,
    pub summary: Option<DraftSummary>,
    /// The stream dropped and the result came from the non-stream reconcile request.
    pub reconciled_after_stream_loss: bool,
}

/// Per-row preview state for the drafting diff list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRowPreview {
    pub row: DraftRow,
    pub original_text: String,
    pub applied: bool,
    pub skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedistributionAllocation {
    pub segment_id: String,
    pub text: String,
}

/// Moves `sentence_count` sentences between adjacent 1-based allocation indexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedistributionMove {
    pub from_index: usize,
    pub to_index: usize,
    pub sentence_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedistributionPreview {
    pub group_id: String,
    pub model: Option<String>,
    pub accept_draft: bool,
    pub notes: Option<String>,
    pub moves: Vec<RedistributionMove>,
    /// Draft allocations sent in the request for this group.
    pub allocations: Vec<RedistributionAllocation>,
    /// Backend error or move validation error; blocks apply.
    pub error: Option<String>,
    pub applied: bool,
}

pub fn parse_backend_health(payload: &str) -> Result<BackendHealth> {
    let value = parse_json(payload)?;
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
    let provider_diagnostics = [
        ("provider", &["provider", "openRouterProvider"][..]),
        ("model", &["model", "draftingModel", "openRouterModel"][..]),
        ("tier", &["serviceTier", "tier"][..]),
        ("reasoning", &["reasoningEffort", "reasoning"][..]),
    ]
    .into_iter()
    .filter_map(|(label, keys)| {
        find_string_by_keys(&value, keys).map(|value| format!("{label}: {value}"))
    })
    .collect();
    Ok(BackendHealth {
        service: value
            .get("service")
            .and_then(Value::as_str)
            .unwrap_or("review-backend")
            .to_string(),
        backend_version,
        openrouter_status,
        test_mode: value.get("testMode").and_then(Value::as_bool),
        provider_diagnostics,
    })
}

/// Parses `{ draftRows: [...], summary: {...} }` (final payload of a draft generation).
pub fn parse_draft_result(payload: &str) -> Result<DraftResult> {
    let value = serde_json::from_str::<Value>(payload)
        .context("The draft couldn't be read. (malformed JSON)")?;
    let rows = value
        .get("draftRows")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("The draft couldn't be read. (no draftRows)"))?
        .iter()
        .map(|row| {
            let row_id = row
                .get("rowId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("The draft couldn't be read. (row without rowId)"))?
                .to_string();
            let rewritten_text = row
                .get("rewrittenText")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    anyhow!("The draft couldn't be read. (row {row_id} has no rewrittenText)")
                })?
                .to_string();
            Ok(DraftRow {
                rewritten_text,
                status: row
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                warnings: string_array(row.get("warnings")),
                row_id,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let summary = value.get("summary").map(|summary| DraftSummary {
        total_rows: summary_usize(summary, "totalRows"),
        rewritten_rows: summary_usize(summary, "rewrittenRows"),
        unchanged_rows: summary_usize(summary, "unchangedRows"),
        failed_rows: summary_usize(summary, "failedRows"),
    });
    Ok(DraftResult {
        rows,
        summary,
        reconciled_after_stream_loss: false,
    })
}

/// Diff previews for drafted rows, with the project's current text as `original_text`.
pub fn draft_row_previews(project: &Project, result: &DraftResult) -> Vec<DraftRowPreview> {
    result
        .rows
        .iter()
        .map(|row| DraftRowPreview {
            original_text: project
                .segment(&row.row_id)
                .map(|segment| segment.text.clone())
                .unwrap_or_default(),
            row: row.clone(),
            applied: false,
            skipped: false,
        })
        .collect()
}

/// Parses a redistribution response, pairing each result with its request group allocations and
/// validating model moves.
pub fn parse_redistribution(
    request_payload: &Value,
    payload: &str,
) -> Result<Vec<RedistributionPreview>> {
    let value = parse_json(payload)?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("The result couldn't be read. (no results)"))?;
    let groups = request_payload
        .get("groups")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(results
        .iter()
        .map(|result| {
            let group_id = optional_string(result, "groupId").unwrap_or_default();
            let allocations = groups
                .iter()
                .find(|group| {
                    group.get("groupId").and_then(Value::as_str) == Some(group_id.as_str())
                })
                .and_then(|group| group.get("draftAllocations"))
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .map(|allocation| RedistributionAllocation {
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
                        .filter_map(|(index, item)| match redistribution_move_from_value(item) {
                            Ok(move_item) => Some(move_item),
                            Err(error) => {
                                move_errors.push(format!(
                                    "Invalid redistribution move {}: {error}.",
                                    index + 1
                                ));
                                None
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
                super::apply::apply_redistribution_moves(&allocations, &moves)
                    .err()
                    .map(|error| error.to_string())
            } else {
                None
            };
            RedistributionPreview {
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

fn redistribution_move_from_value(value: &Value) -> Result<RedistributionMove> {
    Ok(RedistributionMove {
        from_index: required_move_usize(value, "fromIndex")?,
        to_index: required_move_usize(value, "toIndex")?,
        sentence_count: required_move_usize(value, "sentenceCount")?,
    })
}

fn required_move_usize(value: &Value, key: &str) -> Result<usize> {
    value
        .get(key)
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse::<u64>().ok())
        })
        .map(|value| value as usize)
        .filter(|value| *value > 0)
        .ok_or_else(|| anyhow!("missing `{key}`"))
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

fn find_number_by_key(value: &Value, key: &str) -> Option<f64> {
    match value {
        Value::Object(object) => object.get(key).and_then(Value::as_f64).or_else(|| {
            object
                .values()
                .find_map(|child| find_number_by_key(child, key))
        }),
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_number_by_key(child, key)),
        _ => None,
    }
}

fn find_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(object) => keys
            .iter()
            .find_map(|key| object.get(*key).and_then(value_to_compact_string))
            .or_else(|| {
                object
                    .values()
                    .find_map(|child| find_string_by_keys(child, keys))
            }),
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
/// One audio lane attached to a drafting request as multipart parts
/// `audioTrackMeta:<track_id>` (JSON) + `audioTrack:<track_id>` (file).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrack {
    pub track_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub metadata: Value,
    /// Shared bytes: cloning a request never copies the encoded audio.
    pub bytes: Bytes,
}
