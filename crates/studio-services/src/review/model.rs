//! Typed Reviewer/Drafting backend responses and their parsers.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};
use studio_core::Project;

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

#[derive(Debug, Clone, PartialEq)]
pub struct ReviewerTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub score: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerSuggestion {
    pub proposal_id: String,
    pub operation: String,
    pub category: String,
    pub target_template_id: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub report_texts: Vec<String>,
    pub reason: String,
    pub source_card_ids: Vec<String>,
    /// `pending` when absent.
    pub decision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerCard {
    pub id: String,
    pub change_index: i64,
    pub kind: String,
    pub summary: String,
    pub evidence: String,
    pub categories: Vec<String>,
    pub matched_template_id: Option<String>,
    pub template_title: Option<String>,
    pub match_source: Option<String>,
    pub opinion_text: Option<String>,
    pub rationale: Option<String>,
    /// Reviewer's comment for this card (from `comments.cardComments`).
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerSession {
    pub session_id: String,
    pub review_action_id: String,
    pub backend_service: Option<String>,
    pub backend_release: Option<String>,
    pub session_comment: String,
    pub cards: Vec<ReviewerCard>,
    pub suggestions: Vec<ReviewerSuggestion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerFeedback {
    /// `Uncategorized` when absent.
    pub category: String,
    pub score: String,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedReview {
    pub model: Option<String>,
    pub received_at: Option<String>,
    pub template_registry_version: Option<String>,
    pub matched_template_ids: Vec<String>,
    pub feedback: Vec<ReviewerFeedback>,
    /// Backend cards, else classification cards (`change-<n>`), else one card per feedback item
    /// (`feedback-<i>`, kind `CATEGORY FEEDBACK`).
    pub cards: Vec<ReviewerCard>,
    /// Echoed into analytics as `prepared`.
    pub prepared: Value,
    /// Echoed into analytics as `aiReview`.
    pub ai_review: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerFinalize {
    pub session_id: String,
    pub mode: String,
    pub applied_at: Option<String>,
    pub backend_service: Option<String>,
    pub backend_release: Option<String>,
    pub feedback: Vec<ReviewerFeedback>,
}

/// Finalize response: the finalize summary plus the generated-review view of the same payload.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalizeOutcome {
    pub finalize: ReviewerFinalize,
    pub generated: GeneratedReview,
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
pub struct DraftTranscription {
    /// Segment id taken from the request payload (`segment.rowId`, `segment.id`, `rowId`, `segmentId`).
    pub segment_id: Option<String>,
    pub text: String,
    pub model: Option<String>,
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

pub fn parse_reviewer_session(payload: &str) -> Result<ReviewerSession> {
    let value = parse_json(payload)?;
    let session_id = required_string(&value, "sessionId")?;
    let review_action_id = required_string(&value, "reviewActionId")?;
    let comments = value.get("comments").unwrap_or(&Value::Null);
    let card_comments = comments.get("cardComments").unwrap_or(&Value::Null);
    let backend_version = value.get("backendVersion").unwrap_or(&Value::Null);
    let cards = value
        .get("cards")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("review session response did not include cards"))?
        .iter()
        .map(|card| reviewer_card_from_value(card, card_comments))
        .collect();
    let suggestions = value
        .get("suggestions")
        .or_else(|| value.get("proposals"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|suggestion| ReviewerSuggestion {
                    proposal_id: optional_string(suggestion, "proposalId").unwrap_or_default(),
                    operation: optional_string(suggestion, "operation").unwrap_or_default(),
                    category: optional_string(suggestion, "category").unwrap_or_default(),
                    target_template_id: optional_string(suggestion, "targetTemplateId"),
                    title: optional_string(suggestion, "title").unwrap_or_default(),
                    description: optional_string(suggestion, "description"),
                    report_texts: string_array(suggestion.get("reportTexts")),
                    reason: optional_string(suggestion, "reason").unwrap_or_default(),
                    source_card_ids: string_array(suggestion.get("sourceCardIds")),
                    decision: optional_string(suggestion, "decision")
                        .unwrap_or_else(|| "pending".to_string()),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ReviewerSession {
        session_id,
        review_action_id,
        backend_service: backend_version
            .get("service")
            .and_then(Value::as_str)
            .map(str::to_string),
        backend_release: backend_version
            .get("release")
            .and_then(Value::as_str)
            .map(str::to_string),
        session_comment: comments
            .get("sessionComment")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        cards,
        suggestions,
    })
}

pub fn parse_generated_review(payload: &str) -> Result<GeneratedReview> {
    let value = parse_json(payload)?;
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
    let feedback = feedback_value
        .as_array()
        .map(|items| items.iter().map(reviewer_feedback_from_value).collect())
        .unwrap_or_else(Vec::new);
    let mut cards: Vec<ReviewerCard> = value
        .get("cards")
        .or_else(|| ai_review.get("cards"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|card| reviewer_card_from_value(card, &Value::Null))
                .collect()
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
                    .map(|(index, classification)| classification_card(index, classification))
                    .collect()
            })
            .unwrap_or_default();
    }
    if cards.is_empty() {
        cards = feedback
            .iter()
            .enumerate()
            .map(|(index, feedback)| ReviewerCard {
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
        bail!("review generate response did not include feedback, classifications, or cards");
    }
    Ok(GeneratedReview {
        model: optional_string(ai_review, "model"),
        received_at: optional_string(ai_review, "receivedAt"),
        template_registry_version: optional_string(ai_review, "templateRegistryVersion"),
        matched_template_ids: string_array(
            ai_review
                .get("matchedTemplateIds")
                .or_else(|| value.get("matchedTemplateIds")),
        ),
        prepared: value.get("prepared").cloned().unwrap_or(Value::Null),
        ai_review: if ai_review.is_object() {
            ai_review.clone()
        } else {
            json!({ "feedback": feedback_value })
        },
        feedback,
        cards,
    })
}

/// Parses a finalize response for the request (`session_id`, `{ "mode": ... }`).
pub fn parse_finalize_outcome(
    request_session_id: &str,
    request_body: &Value,
    payload: &str,
) -> Result<FinalizeOutcome> {
    let value = parse_json(payload)?;
    let generated = parse_generated_review(payload)?;
    let backend_version = value.get("backendVersion").unwrap_or(&Value::Null);
    let finalize = ReviewerFinalize {
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
            .or_else(|| value_to_compact_string(backend_version)),
        feedback: generated.feedback.clone(),
    };
    Ok(FinalizeOutcome {
        finalize,
        generated,
    })
}

pub fn parse_template_search(payload: &str) -> Result<Vec<ReviewerTemplate>> {
    let value = parse_json(payload)?;
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("template search response did not include results"))?;
    Ok(results
        .iter()
        .map(|result| ReviewerTemplate {
            id: optional_string(result, "id").unwrap_or_default(),
            title: optional_string(result, "title").unwrap_or_default(),
            description: optional_string(result, "description").unwrap_or_default(),
            category: optional_string(result, "category").unwrap_or_default(),
            score: optional_string(result, "score").unwrap_or_default(),
        })
        .collect())
}

/// Parses `{ draftRows: [...], summary: {...} }` (final payload of a draft generation).
pub fn parse_draft_result(payload: &str) -> Result<DraftResult> {
    let value = serde_json::from_str::<Value>(payload).context("malformed draft response")?;
    let rows = value
        .get("draftRows")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("draft response did not include draftRows"))?
        .iter()
        .map(|row| {
            let row_id = row
                .get("rowId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("draft row is missing rowId"))?
                .to_string();
            let rewritten_text = row
                .get("rewrittenText")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("draft row `{row_id}` is missing rewrittenText"))?
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

/// Parses a broker transcription response for the given request payload.
pub fn parse_transcription(request_payload: &Value, payload: &str) -> Result<DraftTranscription> {
    let value = parse_json(payload)?;
    Ok(DraftTranscription {
        segment_id: request_payload
            .pointer("/segment/rowId")
            .and_then(value_to_compact_string)
            .or_else(|| {
                request_payload
                    .pointer("/segment/id")
                    .and_then(value_to_compact_string)
            })
            .or_else(|| optional_string(request_payload, "rowId"))
            .or_else(|| optional_string(request_payload, "segmentId")),
        text: required_string(&value, "text")?,
        model: optional_string(&value, "model"),
    })
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
        .ok_or_else(|| anyhow!("redistribution response did not include results"))?;
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

fn classification_card(index: usize, classification: &Value) -> ReviewerCard {
    let change_index = classification
        .get("change")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(index as i64);
    let template_id = optional_string(classification, "templateId");
    ReviewerCard {
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
}

fn reviewer_feedback_from_value(value: &Value) -> ReviewerFeedback {
    ReviewerFeedback {
        category: optional_string(value, "category").unwrap_or_else(|| "Uncategorized".to_string()),
        score: optional_string(value, "score").unwrap_or_default(),
        note: optional_string(value, "note")
            .or_else(|| optional_string(value, "feedback"))
            .or_else(|| optional_string(value, "text"))
            .unwrap_or_default(),
    }
}

fn reviewer_card_from_value(card: &Value, card_comments: &Value) -> ReviewerCard {
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
    ReviewerCard {
        change_index,
        kind: optional_string(card, "type")
            .or_else(|| optional_string(card, "kind"))
            .unwrap_or_default(),
        summary: optional_string(card, "summary").unwrap_or_default(),
        evidence: optional_string(card, "evidence").unwrap_or_default(),
        categories: string_array(card.get("categories")),
        matched_template_id: optional_string(card, "matchedTemplateId")
            .or_else(|| optional_string(card, "templateId")),
        template_title: optional_string(card, "templateTitle"),
        match_source: optional_string(card, "matchSource"),
        opinion_text: optional_string(card, "opinionText"),
        rationale: optional_string(card, "rationale"),
        comment,
        id,
    }
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

fn parse_json(payload: &str) -> Result<Value> {
    serde_json::from_str::<Value>(payload).context("malformed JSON")
}

fn required_string(value: &Value, key: &str) -> Result<String> {
    optional_string(value, key).ok_or_else(|| anyhow!("missing `{key}`"))
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(value_to_compact_string)
}

/// Trimmed non-empty strings, numbers and bools as text.
pub(super) fn value_to_compact_string(value: &Value) -> Option<String> {
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
