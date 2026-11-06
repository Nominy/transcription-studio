//! Response normalization (tRPC batch/JSONL shapes → plain JSON) and typed parsing.

use exaudio_babel_domain::editor_model::feedback_assessment::{
    FeedbackInput, FeedbackInputResponse,
};
use exaudio_babel_domain::i18n::{localized, with_detail};
use exaudio_babel_lint::linter::{LintSeverity, ServerLintIssue};
use serde_json::{Map, Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::BabelError;
use super::dashboard::normalize_dashboard_payload;
use super::request::ResponseShape;

/// Outcome of a successful annotation save.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveOutcome {
    /// Server `updatedAt` (or now, RFC 3339).
    pub last_saved: String,
    /// Server lint issues returned alongside the save, if any array was present.
    pub lint_issues: Option<Vec<ServerLintIssue>>,
}

/// Draft returned by `transcriptionFeedbackForm.getOrCreateDraft`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedbackDraftBootstrap {
    pub form_step_response_id: String,
    pub input_responses: Vec<FeedbackInputResponse>,
    pub updated_at: String,
}

pub(super) fn normalize_for_shape(shape: ResponseShape, response: &str) -> String {
    match shape {
        ResponseShape::Dashboard => normalize_dashboard_payload(response),
        ResponseShape::ReviewAction => normalize_review_action_payload(response),
        ResponseShape::Generic => normalize_payload(response),
    }
}

/// Unwraps `[{"result":{"data":{"json": X}}}]`-style responses to `X`.
pub(super) fn normalize_payload(response: &str) -> String {
    let trimmed = response.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_string();
    };
    trpc_data(&value).unwrap_or(value).to_string()
}

/// Review-action payloads keep their tRPC envelope (the importer reads it);
/// a bare review-action object is wrapped as `{"json": data}`.
fn normalize_review_action_payload(response: &str) -> String {
    let trimmed = response.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return trimmed.to_string();
    };
    if review_action_data_value(&value).is_some() {
        return value.to_string();
    }
    if is_review_action_data(&value) {
        return json!({ "json": value }).to_string();
    }
    if let Some(data) = trpc_data(&value)
        && is_review_action_data(&data)
    {
        return json!({ "json": data }).to_string();
    }
    value.to_string()
}

fn review_action_data_value(value: &Value) -> Option<&Value> {
    value.pointer(&review_action_data_pointer(value)?)
}

fn review_action_data_pointer(value: &Value) -> Option<String> {
    if let Some(pointer) = ["/result/data/json", "/data/json", "/json"]
        .into_iter()
        .find(|pointer| value.pointer(pointer).is_some())
    {
        return Some(pointer.to_string());
    }
    value
        .as_array()?
        .iter()
        .position(|entry| entry.pointer("/result/data/json").is_some())
        .map(|index| format!("/{index}/result/data/json"))
}

/// Review action id of a review-action payload that carries no annotations; Babel's editor
/// then loads them with `transcriptions.getAnnotationsByReviewActionId`.
pub(super) fn review_action_missing_annotations(payload: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(payload).ok()?;
    let data = review_action_data_value(&value)?;
    data.get("annotations")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
        .then(|| nonblank(data.get("actionId")))
        .flatten()
}

/// Puts a `getAnnotationsByReviewActionId` answer (`{annotations, lintErrors}`) into the
/// review-action payload, as Babel's editor does before showing the task.
pub(super) fn merge_review_action_annotations(
    payload: &str,
    response: &str,
) -> Result<String, BabelError> {
    let decoded = decode(response)?;
    let loaded = trpc_data(&decoded).unwrap_or(decoded);
    let Some(annotations) = loaded.get("annotations").filter(|value| value.is_array()) else {
        return Err(BabelError::new(response_error(
            &loaded,
            "The server's reply had no annotations.",
        )));
    };
    let mut value = decode(payload)?;
    let Some(Value::Object(data)) =
        review_action_data_pointer(&value).and_then(|pointer| value.pointer_mut(&pointer))
    else {
        return Err(BabelError::new("The server's reply had no task data."));
    };
    data.insert("annotations".to_string(), annotations.clone());
    if let Some(lint_errors) = loaded.get("lintErrors").filter(|value| value.is_array()) {
        data.insert("lintErrors".to_string(), lint_errors.clone());
    }
    Ok(value.to_string())
}

fn is_review_action_data(value: &Value) -> bool {
    value
        .get("actionId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        && value
            .get("transcriptionChunkProcessedRecordings")
            .and_then(Value::as_array)
            .is_some()
        && value.get("annotations").and_then(Value::as_array).is_some()
}

pub(super) fn trpc_data(value: &Value) -> Option<Value> {
    if let Value::Array(items) = value {
        return items.first().and_then(trpc_data);
    }
    value
        .pointer("/result/data/json")
        .or_else(|| value.pointer("/data/json"))
        .or_else(|| value.pointer("/result/data"))
        .or_else(|| value.pointer("/data"))
        .cloned()
}

fn decode(payload: &str) -> Result<Value, BabelError> {
    if payload.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(payload)
        .map_err(|error| BabelError::new(format!("The server sent an unreadable reply: {error}")))
}

fn response_success(value: &Value) -> bool {
    first_bool_by_key(value, &["success", "ok"]).unwrap_or(false)
}

fn response_error(value: &Value, fallback: &str) -> String {
    first_string_by_key(value, &["error", "message"])
        .filter(|message| !message.trim().is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "now".to_string())
}

fn updated_at(value: &Value) -> String {
    first_string_by_key(
        value,
        &["updatedAt", "updated_at", "lastSaved", "last_saved"],
    )
    .unwrap_or_else(now_rfc3339)
}

pub(super) fn parse_save(payload: &str) -> Result<SaveOutcome, BabelError> {
    let value = decode(payload)?;
    let lint_issues = server_lint_issues(&value).unwrap_or(None);
    if response_success(&value) {
        return Ok(SaveOutcome {
            last_saved: updated_at(&value),
            lint_issues,
        });
    }
    Err(BabelError {
        message: response_error(
            &value,
            localized(
                "Couldn't save your progress. Try again.",
                "Не удалось сохранить прогресс. Попробуйте снова.",
            ),
        ),
        lint_issues,
        signed_out: false,
    })
}

/// `feedbackIncomplete: true` or any returned lint issue fails the submit.
pub(super) fn parse_submit(payload: &str) -> Result<(), BabelError> {
    let value = decode(payload)?;
    if first_bool_by_key(&value, &["feedbackIncomplete"]) == Some(true) {
        return Err(BabelError::new(response_error(
            &value,
            localized(
                "Finish the feedback form before submitting.",
                "Перед отправкой заполните форму отзыва.",
            ),
        )));
    }
    let issues = server_lint_issues(&value)?;
    if let Some(nonempty) = issues.as_ref().filter(|issues| !issues.is_empty()) {
        return Err(BabelError {
            message: lint_failure_message(nonempty),
            lint_issues: issues,
            signed_out: false,
        });
    }
    if response_success(&value) {
        Ok(())
    } else {
        Err(BabelError {
            message: response_error(
                &value,
                localized(
                    "Couldn't submit. Try again.",
                    "Не удалось отправить. Попробуйте снова.",
                ),
            ),
            lint_issues: issues,
            signed_out: false,
        })
    }
}

/// `success`/`ok` true → Ok; otherwise the response error or `fallback`.
pub(super) fn parse_simple_mutation(payload: &str, fallback: &str) -> Result<(), BabelError> {
    let value = decode(payload)?;
    if response_success(&value) {
        Ok(())
    } else {
        Err(BabelError::new(response_error(&value, fallback)))
    }
}

/// `submitTranscriptionErrorLabel`: Babel's web treats any non-error reply as success, so
/// only an explicit `success: false` or an error message fails.
pub(super) fn parse_error_label_submit(payload: &str) -> Result<(), BabelError> {
    parse_mutation_value(
        payload,
        localized(
            "Couldn't submit the label. Try again.",
            "Не удалось отправить метку. Попробуйте снова.",
        ),
    )
    .map(|_| ())
}

/// Preserve practice/assessment result fields; a failed assessment (`passed: false`)
/// is still a successful request, distinct from a transport or explicit server error.
pub(super) fn parse_mutation_value(payload: &str, fallback: &str) -> Result<Value, BabelError> {
    let value = decode(payload)?;
    let explicit_failure = first_bool_by_key(&value, &["success", "ok"]) == Some(false);
    let error = value
        .get("error")
        .filter(|error| !error.is_null())
        .map(|error| {
            error
                .as_str()
                .map(str::to_string)
                .or_else(|| first_string_by_key(error, &["message"]))
                .unwrap_or_else(|| fallback.to_string())
        });
    match (explicit_failure, error) {
        (_, Some(message)) => Err(BabelError::new(message)),
        (true, None) => Err(BabelError::new(response_error(&value, fallback))),
        (false, None) => Ok(value),
    }
}

fn lint_failure_message(issues: &[ServerLintIssue]) -> String {
    with_detail(
        localized("Babel rejected the task", "Babel отклонил задачу"),
        issues
            .iter()
            .map(|issue| issue.reason.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

/// `lintErrors`/`linterErrors` arrays anywhere in the response, else a bare
/// (or tRPC-wrapped) issue array. `Ok(None)` when no lint array is present.
fn server_lint_issues(value: &Value) -> Result<Option<Vec<ServerLintIssue>>, BabelError> {
    let mut found_named_array = false;
    let mut issue_values = Vec::new();
    collect_named_lint_issue_values(value, &mut found_named_array, &mut issue_values);
    if found_named_array {
        return issue_values
            .into_iter()
            .map(parse_lint_issue)
            .collect::<Result<Vec<_>, _>>()
            .map(Some);
    }
    if let Some(items) = value
        .as_array()
        .filter(|items| is_standalone_lint_array(items))
    {
        return items
            .iter()
            .map(parse_lint_issue)
            .collect::<Result<_, _>>()
            .map(Some);
    }
    if let Some(Value::Array(items)) = trpc_data(value)
        && is_standalone_lint_array(&items)
    {
        return items
            .iter()
            .map(parse_lint_issue)
            .collect::<Result<_, _>>()
            .map(Some);
    }
    Ok(None)
}

fn collect_named_lint_issue_values<'a>(
    value: &'a Value,
    found_array: &mut bool,
    issues: &mut Vec<&'a Value>,
) {
    const KEYS: [&str; 2] = ["lintErrors", "linterErrors"];
    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(found) = map.get(key).and_then(Value::as_array) {
                    *found_array = true;
                    issues.extend(found);
                }
            }
            for (key, child) in map {
                if !KEYS.contains(&key.as_str()) {
                    collect_named_lint_issue_values(child, found_array, issues);
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_named_lint_issue_values(child, found_array, issues);
            }
        }
        _ => {}
    }
}

fn is_standalone_lint_array(items: &[Value]) -> bool {
    items.iter().all(|item| {
        item.as_object().is_some_and(|issue| {
            issue.contains_key("annotationId")
                && issue.contains_key("reason")
                && issue.contains_key("severity")
        })
    })
}

fn parse_lint_issue(value: &Value) -> Result<ServerLintIssue, BabelError> {
    let Some(issue) = value.as_object() else {
        return Err(BabelError::new("Babel lint issue was not an object."));
    };
    let annotation_id = required_lint_string(issue, "annotationId")?;
    let reason = required_lint_string(issue, "reason")?;
    let severity = match required_lint_string(issue, "severity")?
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => LintSeverity::Error,
        "warning" => LintSeverity::Warning,
        "pass" => LintSeverity::Pass,
        severity => {
            return Err(BabelError::new(format!(
                "Babel lint issue had unsupported severity `{severity}`."
            )));
        }
    };
    Ok(ServerLintIssue {
        annotation_id,
        reason,
        severity,
    })
}

fn required_lint_string(issue: &Map<String, Value>, key: &str) -> Result<String, BabelError> {
    issue
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| BabelError::new(format!("Babel lint issue was missing `{key}`.")))
}

/// Feedback bootstrap payloads: tRPC data when present, else a top-level
/// `error`/`message` string is an error, else the value itself.
fn feedback_value(payload: &str) -> Result<Value, BabelError> {
    let value = decode(payload)?;
    if let Some(normalized) = trpc_data(&value) {
        return Ok(normalized);
    }
    if let Some(error) =
        first_string_by_key(&value, &["error", "message"]).filter(|error| !error.trim().is_empty())
    {
        return Err(BabelError::new(error));
    }
    Ok(value)
}

fn nonblank(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn parse_feedback_rubric_flags(payload: &str) -> Result<Vec<String>, BabelError> {
    let value = feedback_value(payload)?;
    let flags = value
        .as_array()
        .ok_or_else(|| BabelError::new("Babel feedback rubric response must be an array"))?;
    flags
        .iter()
        .enumerate()
        .map(|(index, flag)| {
            nonblank(Some(flag)).ok_or_else(|| {
                BabelError::new(format!(
                    "Babel feedback rubric flag at index {index} must be a string"
                ))
            })
        })
        .collect()
}

pub(super) fn parse_feedback_form_step_id(payload: &str) -> Result<String, BabelError> {
    let value = feedback_value(payload)?;
    nonblank(
        value
            .get("steps")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .and_then(|step| step.get("id")),
    )
    .ok_or_else(|| BabelError::new("Babel feedback form is missing its first step id"))
}

pub(super) fn parse_feedback_inputs(payload: &str) -> Result<Vec<FeedbackInput>, BabelError> {
    let value = feedback_value(payload)?;
    let values = value
        .as_array()
        .ok_or_else(|| BabelError::new("The server's feedback form reply was not a list."))?;
    if values.is_empty() {
        return Err(BabelError::new("The server's feedback form has no inputs."));
    }
    values
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let object = input.as_object().ok_or_else(|| {
                BabelError::new(format!(
                    "Babel feedback input at index {index} must be an object"
                ))
            })?;
            let id = nonblank(object.get("id")).ok_or_else(|| {
                BabelError::new(format!(
                    "Babel feedback input at index {index} is missing id"
                ))
            })?;
            let label = match object.get("label") {
                None | Some(Value::Null) => None,
                Some(Value::String(label)) => Some(label.clone()),
                Some(_) => {
                    return Err(BabelError::new(format!(
                        "Babel feedback input label at index {index} must be a string or null"
                    )));
                }
            };
            Ok(FeedbackInput { id, label })
        })
        .collect()
}

pub(super) fn parse_feedback_draft(payload: &str) -> Result<FeedbackDraftBootstrap, BabelError> {
    let value = feedback_value(payload)?;
    let object = value
        .as_object()
        .ok_or_else(|| BabelError::new("Babel feedback draft response must be an object"))?;
    let form_step_response_id = nonblank(object.get("formStepResponseId")).ok_or_else(|| {
        BabelError::new("Babel feedback draft response is missing formStepResponseId")
    })?;
    let updated_at = nonblank(object.get("updatedAt"))
        .ok_or_else(|| BabelError::new("Babel feedback draft response is missing updatedAt"))?;
    let responses = match object.get("inputResponses") {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(values)) => values.as_slice(),
        Some(_) => {
            return Err(BabelError::new(
                "Babel feedback draft inputResponses must be an array",
            ));
        }
    };
    let input_responses = responses
        .iter()
        .enumerate()
        .map(|(index, response)| {
            let response = response.as_object().ok_or_else(|| {
                BabelError::new(format!(
                    "Babel feedback input response at index {index} must be an object"
                ))
            })?;
            let form_input_id = nonblank(response.get("formInputId")).ok_or_else(|| {
                BabelError::new(format!(
                    "Babel feedback input response at index {index} is missing formInputId"
                ))
            })?;
            let value = response
                .get("value")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    BabelError::new(format!(
                        "Babel feedback input response at index {index} is missing value"
                    ))
                })?;
            Ok(FeedbackInputResponse {
                form_input_id,
                value,
            })
        })
        .collect::<Result<Vec<_>, BabelError>>()?;
    Ok(FeedbackDraftBootstrap {
        form_step_response_id,
        input_responses,
        updated_at,
    })
}

/// Returns `last_saved`. `null`/empty success responses are accepted.
pub(super) fn parse_feedback_draft_save(payload: &str) -> Result<String, BabelError> {
    let value = decode(payload)?;
    if let Some(error) =
        first_string_by_key(&value, &["error", "message"]).filter(|error| !error.trim().is_empty())
    {
        return Err(BabelError::new(error));
    }
    if has_nonempty_value_by_key(&value, &["error", "message"])
        || has_false_bool_by_key(&value, &["success", "ok"])
    {
        return Err(BabelError::new(localized(
            "Couldn't save the feedback. Try again.",
            "Не удалось сохранить отзыв. Попробуйте снова.",
        )));
    }
    Ok(updated_at(&value))
}

fn has_nonempty_value_by_key(value: &Value, keys: &[&str]) -> bool {
    match value {
        Value::Object(map) => {
            keys.iter().any(|key| {
                map.get(*key).is_some_and(|value| match value {
                    Value::Null => false,
                    Value::String(value) => !value.trim().is_empty(),
                    Value::Array(values) => !values.is_empty(),
                    Value::Object(values) => !values.is_empty(),
                    Value::Bool(_) | Value::Number(_) => true,
                })
            }) || map
                .values()
                .any(|child| has_nonempty_value_by_key(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .any(|child| has_nonempty_value_by_key(child, keys)),
        _ => false,
    }
}

fn has_false_bool_by_key(value: &Value, keys: &[&str]) -> bool {
    match value {
        Value::Object(map) => {
            keys.iter()
                .any(|key| map.get(*key).and_then(Value::as_bool) == Some(false))
                || map.values().any(|child| has_false_bool_by_key(child, keys))
        }
        Value::Array(items) => items.iter().any(|child| has_false_bool_by_key(child, keys)),
        _ => false,
    }
}

fn first_bool_by_key(value: &Value, keys: &[&str]) -> Option<bool> {
    match value {
        Value::Object(map) => keys
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_bool))
            .or_else(|| {
                map.values()
                    .find_map(|child| first_bool_by_key(child, keys))
            }),
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_bool_by_key(child, keys)),
        _ => None,
    }
}

pub(super) fn first_string_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => keys
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_string)
            .or_else(|| {
                map.values()
                    .find_map(|child| first_string_by_key(child, keys))
            }),
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_string_by_key(child, keys)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(annotation_id: &str, reason: &str, severity: LintSeverity) -> ServerLintIssue {
        ServerLintIssue {
            annotation_id: annotation_id.to_string(),
            reason: reason.to_string(),
            severity,
        }
    }

    #[test]
    fn normalizes_trpc_batch_json_results() {
        assert_eq!(
            normalize_payload(
                r#"[{"result":{"data":{"json":{"success":true,"updatedAt":"2026-06-28T10:00:00Z"}}}}]"#
            ),
            r#"{"success":true,"updatedAt":"2026-06-28T10:00:00Z"}"#
        );
    }

    #[test]
    fn normalized_mutation_payloads_parse_as_success() {
        let normalized = normalize_payload(
            r#"[{"result":{"data":{"json":{"success":true,"updatedAt":"2026-06-28T10:00:00Z"}}}}]"#,
        );
        assert_eq!(
            parse_save(&normalized).expect("save").last_saved,
            "2026-06-28T10:00:00Z"
        );
        parse_submit(&normalized).expect("submit");
        parse_simple_mutation(&normalized, "drop failed").expect("drop");
        assert_eq!(
            parse_feedback_draft_save(&normalized).expect("feedback save"),
            "2026-06-28T10:00:00Z"
        );
    }

    #[test]
    fn mutation_failures_use_server_error_or_fallback() {
        assert_eq!(
            parse_simple_mutation(r#"{"success":false,"message":"Queue closed"}"#, "fallback")
                .unwrap_err()
                .message,
            "Queue closed"
        );
        assert_eq!(
            parse_simple_mutation(r#"{"success":false}"#, "fallback")
                .unwrap_err()
                .message,
            "fallback"
        );
    }

    #[test]
    fn feedback_draft_save_accepts_null_and_rejects_false_or_errors() {
        let normalized = normalize_payload(r#"[{"result":{"data":{"json":null}}}]"#);
        assert_eq!(normalized, "null");
        parse_feedback_draft_save(&normalized).expect("null is success");

        for payload in [r#"{"success":false}"#, r#"{"ok":false}"#] {
            parse_feedback_draft_save(payload).unwrap_err();
        }
        let normalized =
            normalize_payload(r#"[{"error":{"json":{"message":"Feedback draft was rejected"}}}]"#);
        assert_eq!(
            parse_feedback_draft_save(&normalized).unwrap_err().message,
            "Feedback draft was rejected"
        );
    }

    #[test]
    fn feedback_bootstrap_parsers_decode_batch_success_and_reject_malformed() {
        assert_eq!(
            parse_feedback_rubric_flags(
                r#"[{"result":{"data":{"json":["tx-word-accuracy","tx-other-feedback"]}}}]"#
            )
            .expect("flags"),
            vec![
                "tx-word-accuracy".to_string(),
                "tx-other-feedback".to_string()
            ]
        );
        assert_eq!(
            parse_feedback_form_step_id(
                r#"[{"result":{"data":{"json":{"steps":[{"id":"step-1"}]}}}}]"#
            )
            .expect("form"),
            "step-1"
        );
        assert!(parse_feedback_form_step_id(r#"{"steps":[]}"#).is_err());
        assert_eq!(
            parse_feedback_inputs(
                r#"[{"id":"comment","label":null},{"id":"score","label":"Score"}]"#
            )
            .expect("inputs"),
            vec![
                FeedbackInput {
                    id: "comment".to_string(),
                    label: None
                },
                FeedbackInput {
                    id: "score".to_string(),
                    label: Some("Score".to_string())
                },
            ]
        );
        assert!(parse_feedback_inputs("[]").is_err());
        assert!(
            parse_feedback_draft(r#"{"formStepResponseId":"response-1","inputResponses":[]}"#)
                .is_err(),
            "missing updatedAt"
        );
        assert_eq!(
            parse_feedback_draft(
                r#"{"formStepResponseId":"r-1","updatedAt":"t","inputResponses":[{"formInputId":"comment","value":"ok"}]}"#
            )
            .expect("draft"),
            FeedbackDraftBootstrap {
                form_step_response_id: "r-1".to_string(),
                input_responses: vec![FeedbackInputResponse {
                    form_input_id: "comment".to_string(),
                    value: "ok".to_string(),
                }],
                updated_at: "t".to_string(),
            }
        );
    }

    #[test]
    fn submit_with_lint_issues_fails_and_returns_them() {
        let error = parse_submit(
            r#"{"success":false,"linterErrors":[{"annotationId":"a","reason":"Bad tag","severity":"error"}]}"#,
        )
        .unwrap_err();
        assert!(error.message.contains("Bad tag"));
        assert_eq!(
            error.lint_issues,
            Some(vec![issue("a", "Bad tag", LintSeverity::Error)])
        );
        let dual = parse_submit(
            r#"{"success":false,"lintErrors":[{"annotationId":"a","reason":"Warn","severity":"warning"}],"linterErrors":[{"annotationId":"b","reason":"Bad tag","severity":"error"}]}"#,
        )
        .unwrap_err();
        assert_eq!(
            dual.lint_issues,
            Some(vec![
                issue("a", "Warn", LintSeverity::Warning),
                issue("b", "Bad tag", LintSeverity::Error),
            ])
        );
        parse_submit(r#"{"success":true,"linterErrors":[]}"#).expect("empty lint array passes");
    }

    #[test]
    fn submit_with_incomplete_feedback_fails_without_lint() {
        let error = parse_submit(
            r#"{"success":false,"linterErrors":[],"feedbackIncomplete":true,"message":"Rubric missing"}"#,
        )
        .unwrap_err();
        assert!(error.message.contains("Rubric missing"));
        assert_eq!(error.lint_issues, None);
        parse_submit(r#"{"success":false,"linterErrors":[],"feedbackIncomplete":true}"#)
            .unwrap_err();
        parse_submit(r#"{"success":true,"linterErrors":[],"feedbackIncomplete":false}"#)
            .expect("complete feedback passes");
    }

    #[test]
    fn review_action_normalization_wraps_bare_claim_payload_for_importer() {
        let payload = normalize_for_shape(
            ResponseShape::ReviewAction,
            r#"{"actionId":"review-action-1","actionLevel":1,"success":true,"processedRecordingUriMap":{},"transcriptionChunkProcessedRecordings":[],"annotations":[],"lintErrors":[]}"#,
        );
        let value = serde_json::from_str::<Value>(&payload).expect("wrapped");
        assert_eq!(
            value.pointer("/json/actionId").and_then(Value::as_str),
            Some("review-action-1")
        );
    }
}
