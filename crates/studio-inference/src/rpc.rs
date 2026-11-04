//! Framing for calls into Gold's host inference page (`globalThis.babelInference`, host API v1,
//! see Gold `src/backend/host-api.ts`).
//!
//! A call is one CDP `Runtime.evaluate` (`awaitPromise`, `returnByValue`) of an async wrapper
//! around `babelInference.<method>(...args, emit)`. Host methods resolve to an envelope
//! `{ ok: true, result } | { ok: false, error: { code, message } }`; the wrapper turns anything
//! else (a missing method, an unexpected throw) into the same error envelope so one decoder
//! handles every outcome. Progress and streamed payloads arrive through the CDP binding
//! [`EMIT_BINDING`] as `{ requestId, event }` JSON strings.

use serde::Deserialize;
use serde_json::Value;

/// Name of the CDP binding the page calls for every progress/stream event.
pub const EMIT_BINDING: &str = "__studioEmit";

/// Host API version this crate speaks (`babelInference.hostApiVersion`).
pub const HOST_API_VERSION: u64 = 1;

/// Builds the `Runtime.evaluate` expression for one host-page call:
/// `babelInference[method](...args, emit)`, with `emit` tagging events with `request_id`.
pub fn call_expression(request_id: &str, method: &str, args: &[Value]) -> String {
    let request_id = Value::from(request_id);
    let method = Value::from(method);
    let mut arguments = String::new();
    for arg in args {
        arguments.push_str(&arg.to_string());
        arguments.push_str(", ");
    }
    format!(
        r#"(async () => {{
  const requestId = {request_id};
  const method = {method};
  const host = globalThis.babelInference;
  if (!host || typeof host[method] !== 'function') {{
    return {{ ok: false, error: {{ code: null, message: `Gold inference page does not expose ${{method}}().` }} }};
  }}
  const emit = event => {{ globalThis.{EMIT_BINDING}(JSON.stringify({{ requestId, event }})); }};
  try {{
    return await host[method]({arguments}emit);
  }} catch (error) {{
    return {{ ok: false, error: {{ code: null, message: String(error?.message ?? error) }} }};
  }}
}})()"#
    )
}

/// An error reported by the page (or raised while evaluating the call).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PageError {
    /// Gold's `HostErrorCode` (`needs-models`, `no-webgpu`, `cancelled`, ...); `None` when the
    /// failure did not come from a host method's envelope.
    #[serde(default)]
    pub code: Option<String>,
    pub message: String,
}

/// Why a `Runtime.evaluate` response did not produce a page result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    #[error("{}", .0.message)]
    Page(PageError),
    #[error("malformed DevTools response: {0}")]
    Malformed(String),
}

/// Decodes the CDP `Runtime.evaluate` response of a [`call_expression`] into the envelope's
/// `result` (`null` when absent).
pub fn parse_call_response(raw: &str) -> Result<Value, FrameError> {
    let envelope = parse_evaluate_value(raw)?;
    match envelope.get("ok").and_then(Value::as_bool) {
        Some(true) => Ok(envelope.get("result").cloned().unwrap_or(Value::Null)),
        Some(false) => {
            let error = envelope.get("error").ok_or_else(|| {
                FrameError::Malformed("error envelope without `error`".to_string())
            })?;
            let error = PageError::deserialize(error)
                .map_err(|error| FrameError::Malformed(format!("page error: {error}")))?;
            Err(FrameError::Page(error))
        }
        None => Err(FrameError::Malformed(format!(
            "page result is not an envelope: {}",
            truncate(&envelope.to_string(), 200)
        ))),
    }
}

/// Decodes a plain `Runtime.evaluate` into its by-value result; `undefined` is `null`.
pub fn parse_evaluate_value(raw: &str) -> Result<Value, FrameError> {
    let mut response: Value =
        serde_json::from_str(raw).map_err(|error| FrameError::Malformed(error.to_string()))?;
    if let Some(details) = response.get("exceptionDetails") {
        return Err(FrameError::Page(exception_error(details)));
    }
    let result = response
        .get_mut("result")
        .ok_or_else(|| FrameError::Malformed("evaluation returned no result".to_string()))?;
    let mut value = result
        .get_mut("value")
        .map(Value::take)
        .unwrap_or(Value::Null);
    integralize(&mut value);
    Ok(value)
}

/// DevTools serializes JS numbers beyond 32 bits as doubles (`2561023582.0`); turn every
/// integral double back into an integer so byte counts and sample indices decode as `u64`.
/// Integers still decode as `f64` wherever a float is expected.
fn integralize(value: &mut Value) {
    match value {
        Value::Number(number) => {
            if let Some(float) = number.as_f64().filter(|_| number.is_f64())
                && float.fract() == 0.0
                && float.abs() < 9_007_199_254_740_992.0
            {
                *value = Value::from(float as i64);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(integralize),
        Value::Object(map) => map.values_mut().for_each(integralize),
        _ => {}
    }
}

fn exception_error(details: &Value) -> PageError {
    let description = details
        .get("exception")
        .and_then(|exception| exception.get("description"))
        .and_then(Value::as_str)
        .or_else(|| details.get("text").and_then(Value::as_str))
        .unwrap_or("script evaluation failed");
    // V8 descriptions are `Name: message\n    at ...`; the first line carries the message.
    PageError {
        code: None,
        message: description
            .lines()
            .next()
            .unwrap_or(description)
            .to_string(),
    }
}

fn truncate(text: &str, max: usize) -> &str {
    match text.char_indices().nth(max) {
        Some((index, _)) => &text[..index],
        None => text,
    }
}

/// One event the page emitted for request `request_id`.
#[derive(Debug, Clone, PartialEq)]
pub struct EmittedEvent {
    pub request_id: String,
    pub event: Value,
}

/// Decodes the parameters of a CDP `Runtime.bindingCalled` notification. Returns `None` for
/// other bindings and for payloads that are not `{ requestId, event }`.
pub fn parse_binding_called(params_json: &str) -> Option<EmittedEvent> {
    #[derive(Deserialize)]
    struct BindingCalled {
        name: String,
        payload: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Payload {
        request_id: String,
        #[serde(default)]
        event: Value,
    }
    let called: BindingCalled = serde_json::from_str(params_json).ok()?;
    if called.name != EMIT_BINDING {
        return None;
    }
    let payload: Payload = serde_json::from_str(&called.payload).ok()?;
    Some(EmittedEvent {
        request_id: payload.request_id,
        event: payload.event,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn evaluate_response(value: Value) -> String {
        json!({ "result": { "type": "object", "value": value } }).to_string()
    }

    #[test]
    fn ok_envelope_yields_result() {
        let raw = evaluate_response(json!({ "ok": true, "result": { "tracks": [1, 2] } }));
        assert_eq!(
            parse_call_response(&raw).unwrap(),
            json!({ "tracks": [1, 2] })
        );
        let raw = evaluate_response(json!({ "ok": true }));
        assert_eq!(parse_call_response(&raw).unwrap(), Value::Null);
    }

    #[test]
    fn error_envelope_keeps_code() {
        let raw = evaluate_response(json!({
            "ok": false, "error": { "code": "needs-models", "message": "Models are not installed." }
        }));
        assert_eq!(
            parse_call_response(&raw),
            Err(FrameError::Page(PageError {
                code: Some("needs-models".to_string()),
                message: "Models are not installed.".to_string(),
            }))
        );
    }

    #[test]
    fn wrapper_errors_have_no_code() {
        let raw = evaluate_response(json!({
            "ok": false, "error": { "code": null, "message": "x is undefined" }
        }));
        let Err(FrameError::Page(error)) = parse_call_response(&raw) else {
            panic!("expected page error");
        };
        assert_eq!(error.code, None);
        assert_eq!(error.message, "x is undefined");
    }

    #[test]
    fn exception_details_become_page_errors() {
        let raw = json!({
            "result": { "type": "object", "subtype": "error" },
            "exceptionDetails": {
                "text": "Uncaught",
                "exception": { "description": "ReferenceError: __studioEmit is not defined\n    at <anonymous>:1:1" }
            }
        })
        .to_string();
        assert_eq!(
            parse_call_response(&raw),
            Err(FrameError::Page(PageError {
                code: None,
                message: "ReferenceError: __studioEmit is not defined".to_string(),
            }))
        );
    }

    #[test]
    fn non_envelopes_are_malformed() {
        let undefined = json!({ "result": { "type": "undefined" } }).to_string();
        assert!(matches!(
            parse_call_response(&undefined),
            Err(FrameError::Malformed(_))
        ));
        let bare = evaluate_response(json!({ "tracks": [] }));
        assert!(matches!(
            parse_call_response(&bare),
            Err(FrameError::Malformed(_))
        ));
        let no_error = evaluate_response(json!({ "ok": false }));
        assert!(matches!(
            parse_call_response(&no_error),
            Err(FrameError::Malformed(_))
        ));
        assert!(matches!(
            parse_call_response("not json"),
            Err(FrameError::Malformed(_))
        ));
    }

    #[test]
    fn integral_doubles_decode_as_integers() {
        let raw = r#"{"result":{"type":"object","value":{"ok":true,"result":{"totalBytes":2561023582.0,"start":1.5,"n":[3.0,-4.0]}}}}"#;
        let result = parse_call_response(raw).unwrap();
        assert_eq!(result["totalBytes"].as_u64(), Some(2_561_023_582));
        assert_eq!(result["start"].as_f64(), Some(1.5));
        assert_eq!(result["n"], json!([3, -4]));
        let seconds: f64 = serde_json::from_value(result["n"][0].clone()).unwrap();
        assert_eq!(seconds, 3.0);
    }

    #[test]
    fn plain_evaluate_undefined_is_null() {
        let raw = json!({ "result": { "type": "undefined" } }).to_string();
        assert_eq!(parse_evaluate_value(&raw).unwrap(), Value::Null);
    }

    #[test]
    fn binding_events_are_routed_by_request() {
        let params = json!({
            "name": EMIT_BINDING,
            "payload": json!({ "requestId": "r7", "event": { "progress": { "phase": "decode" } } }).to_string(),
            "executionContextId": 3
        })
        .to_string();
        assert_eq!(
            parse_binding_called(&params),
            Some(EmittedEvent {
                request_id: "r7".to_string(),
                event: json!({ "progress": { "phase": "decode" } }),
            })
        );
    }

    #[test]
    fn foreign_bindings_and_garbage_are_ignored() {
        let other =
            json!({ "name": "other", "payload": "{}", "executionContextId": 1 }).to_string();
        assert_eq!(parse_binding_called(&other), None);
        let garbage =
            json!({ "name": EMIT_BINDING, "payload": "nope", "executionContextId": 1 }).to_string();
        assert_eq!(parse_binding_called(&garbage), None);
    }

    #[test]
    fn expression_embeds_arguments_before_emit() {
        let expression = call_expression(
            "id\"1",
            "timing",
            &[
                json!({ "taskId": "t'1</script>" }),
                json!({ "a": { "url": "u" } }),
            ],
        );
        assert!(expression.contains(r#"const requestId = "id\"1";"#));
        assert!(
            expression
                .contains(r#"host[method]({"taskId":"t'1</script>"}, {"a":{"url":"u"}}, emit)"#)
        );
        assert!(call_expression("r", "status", &[]).contains("host[method](emit)"));
    }
}
