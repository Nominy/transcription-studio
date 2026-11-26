use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(super) fn request(value: &Value) -> Result<Option<Value>, String> {
    match value {
        Value::Object(map) => {
            if let Some(records) = map
                .get("transcriptionChunkProcessedRecordings")
                .and_then(Value::as_array)
            {
                if records.is_empty() {
                    return Ok(None);
                }
                let action = map
                    .get("actionId")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or("Audio links require a review action ID. Reload the task.")?;
                let ids: Result<Vec<_>, _> = records
                    .iter()
                    .map(|r| {
                        r.get("chunkedProcessedRecordingId")
                            .and_then(Value::as_str)
                            .filter(|s| !s.trim().is_empty())
                            .ok_or("Audio fragment ID is missing. Reload the task.")
                    })
                    .collect();
                return Ok(Some(
                    json!({"reviewActionId": action, "chunkedProcessedRecordingIds": ids?}),
                ));
            }
            for child in map.values() {
                if let Some(request) = request(child)? {
                    return Ok(Some(request));
                }
            }
        }
        Value::Array(items) => {
            for child in items {
                if let Some(request) = request(child)? {
                    return Ok(Some(request));
                }
            }
        }
        _ => {}
    }
    Ok(None)
}

pub(super) fn parse(body: &str) -> Result<BTreeMap<String, String>, String> {
    let value: Value = serde_json::from_str(body).map_err(|_| "Invalid audio link response.")?;
    let root = value.as_array().and_then(|a| a.first()).unwrap_or(&value);
    let data = root
        .pointer("/result/data/json")
        .ok_or("Babel did not return audio link data.")?;
    let map = data
        .as_object()
        .ok_or("Babel audio link response is not a map.")?;
    map.iter()
        .map(|(id, value)| {
            let url = value
                .as_str()
                .filter(|s| s.starts_with("https://"))
                .ok_or("Babel returned an invalid audio link.")?;
            Ok((id.clone(), url.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_wrapped_task() {
        let v = json!([{"result":{"data":{"json":{"actionId":"review-1", "transcriptionChunkProcessedRecordings":[{"chunkedProcessedRecordingId":"chunk-1"}]}}}}]);
        assert_eq!(
            request(&v).unwrap().unwrap(),
            json!({"reviewActionId":"review-1","chunkedProcessedRecordingIds":["chunk-1"]})
        );
    }
    #[test]
    fn rejects_missing_action_or_chunk() {
        assert!(request(&json!({"transcriptionChunkProcessedRecordings":[{}]})).is_err());
        assert!(
            request(&json!({"actionId":"a","transcriptionChunkProcessedRecordings":[{}]})).is_err()
        );
    }
    #[test]
    fn parses_chunk_keyed_urls() {
        assert_eq!(
            parse(r#"[{"result":{"data":{"json":{"chunk-1":"https://example.test/audio"}}}}]"#)
                .unwrap()["chunk-1"],
            "https://example.test/audio"
        );
    }
    #[test]
    fn refuses_errors_and_unsafe_urls() {
        assert!(parse(r#"[{"error":{"message":"Denied"}}]"#).is_err());
        assert!(parse(r#"[{"result":{"data":{"json":{"c":"http://example.test/a"}}}}]"#).is_err());
    }
    #[test]
    fn non_chunk_payload_keeps_legacy_route() {
        assert!(request(&json!({"other":1})).unwrap().is_none());
    }
}
