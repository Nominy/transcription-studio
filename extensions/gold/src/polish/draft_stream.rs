//! Incremental Server-Sent Events for `/api/draft/generate/stream` (Gold `backend-client.ts`):
//! `started` → total rows, `row` → one streamed row result, `done` → the final
//! `{draftRows, summary, generationMeta}`, `error` → failure. The parser is fed raw body chunks
//! as they arrive (split anywhere, CRLF or LF, comments, multi-line `data:`).

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use super::model::{DraftRow, DraftSummary, parse_draft_result};

/// One dispatched SSE event (`event:` name, joined `data:` lines).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

/// Line-oriented SSE decoder over arbitrary byte chunks.
#[derive(Debug, Default)]
pub struct SseParser {
    pending: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

impl SseParser {
    /// Feeds bytes; returns the events completed by them.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(newline) = self.pending.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=newline).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line);
            self.line(&line, &mut events);
        }
        events
    }

    /// Flushes a final event not followed by a blank line (end of body).
    pub fn finish(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            let line = String::from_utf8_lossy(&rest)
                .trim_end_matches('\r')
                .to_string();
            self.line(&line, &mut events);
        }
        self.dispatch(&mut events);
        events
    }

    fn line(&mut self, line: &str, events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            self.dispatch(events);
            return;
        }
        if line.starts_with(':') {
            return;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data.push(value.to_string()),
            _ => {}
        }
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) {
        let event = self.event.take();
        if self.data.is_empty() {
            return;
        }
        events.push(SseEvent {
            event: event.unwrap_or_else(|| "message".to_string()),
            data: std::mem::take(&mut self.data).join("\n"),
        });
    }
}

/// Progress of a streamed draft generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftStreamEvent {
    Started {
        total_rows: usize,
    },
    /// One finished row; replaces an earlier streamed row with the same id.
    Row {
        row: DraftRow,
        completed_rows: usize,
        total_rows: usize,
        summary: Option<DraftSummary>,
    },
    /// The stream dropped; the result is being reconciled for the same draft session.
    Reconnecting,
}

/// What a decoded SSE event means for the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftStreamStep {
    Progress(DraftStreamEvent),
    /// The `done` payload (parse with [`parse_draft_result`]).
    Done(String),
    Ignored,
}

/// Interprets one SSE event; an `error` event fails with its message.
pub fn draft_stream_step(event: &SseEvent) -> Result<DraftStreamStep> {
    match event.event.as_str() {
        "started" => {
            let value: Value = serde_json::from_str(&event.data)
                .context("The draft couldn't be read. (malformed started event)")?;
            Ok(DraftStreamStep::Progress(DraftStreamEvent::Started {
                total_rows: count(&value, "totalRows"),
            }))
        }
        "row" => {
            let value: Value = serde_json::from_str(&event.data)
                .context("The draft couldn't be read. (malformed row event)")?;
            let row = value
                .get("row")
                .ok_or_else(|| anyhow!("The draft couldn't be read. (row event without row)"))?;
            let parsed = parse_draft_result(
                &json!({ "draftRows": [row], "summary": value.get("summary") }).to_string(),
            )?;
            let summary = value
                .get("summary")
                .filter(|summary| summary.is_object())
                .and(parsed.summary);
            let row =
                parsed.rows.into_iter().next().ok_or_else(|| {
                    anyhow!("The draft couldn't be read. (row event without row)")
                })?;
            Ok(DraftStreamStep::Progress(DraftStreamEvent::Row {
                row,
                completed_rows: count(&value, "completedRows"),
                total_rows: count(&value, "totalRows"),
                summary,
            }))
        }
        "done" => Ok(DraftStreamStep::Done(event.data.clone())),
        "error" => bail!("{}", stream_error_message(&event.data)),
        _ => Ok(DraftStreamStep::Ignored),
    }
}

fn count(value: &Value, key: &str) -> usize {
    value
        .get(key)
        .and_then(Value::as_u64)
        .map_or(0, |count| count as usize)
}

/// `{error: "..."}` / `{message: "..."}` / `{error: {message}}` or the raw data; a generic
/// message when there is none.
fn stream_error_message(data: &str) -> String {
    let message = serde_json::from_str::<Value>(data).ok().and_then(|value| {
        [
            value.get("error").and_then(Value::as_str),
            value.pointer("/error/message").and_then(Value::as_str),
            value.get("message").and_then(Value::as_str),
        ]
        .into_iter()
        .flatten()
        .next()
        .map(str::to_string)
    });
    let message = message.unwrap_or_else(|| data.trim().to_string());
    if message.trim().is_empty() {
        "Drafting failed.".to_string()
    } else {
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_handles_split_chunks_crlf_comments_and_multiline_data() {
        let mut parser = SseParser::default();
        let body =
            ": keepalive\r\nevent: started\r\ndata: {\"totalRows\"\r\ndata: :2}\r\n\r\nevent: ro";
        let mut events = parser.push(body.as_bytes());
        events.extend(
            parser.push(b"w\ndata: {\"row\":{\"rowId\":\"a\",\"rewrittenText\":\"\xd0\x9f"),
        );
        events.extend(parser.push(
            b"\xd1\x80\",\"status\":\"rewritten\"},\"completedRows\":1,\"totalRows\":2}\n\n",
        ));
        events.extend(parser.push(b"event: done\ndata: {\"draftRows\":[]}"));
        events.extend(parser.finish());

        let steps: Vec<DraftStreamStep> = events
            .iter()
            .map(|event| draft_stream_step(event).unwrap())
            .collect();
        assert_eq!(
            steps[0],
            DraftStreamStep::Progress(DraftStreamEvent::Started { total_rows: 2 })
        );
        match &steps[1] {
            DraftStreamStep::Progress(DraftStreamEvent::Row {
                row,
                completed_rows: 1,
                total_rows: 2,
                summary: None,
            }) => assert_eq!(row.rewritten_text, "Пр"),
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(
            steps[2],
            DraftStreamStep::Done("{\"draftRows\":[]}".to_string())
        );
    }

    #[test]
    fn error_event_fails_with_the_server_message_alone() {
        let error = |data: &str| {
            draft_stream_step(&SseEvent {
                event: "error".into(),
                data: data.into(),
            })
            .unwrap_err()
            .to_string()
        };
        // Shown inside "Refinement failed: … Apply the draft or retry refinement."
        assert_eq!(
            error("{\"error\":\"OpenRouter key rejected\"}"),
            "OpenRouter key rejected"
        );
        assert!(!error("{\"error\":\"\"}").trim().is_empty());
    }
}
