//! Gold Draft polish streaming against a local fake backend: rows reach the caller while the
//! stream is still open, a dropped connection is reconciled once for the same draft session,
//! and HTTP / `error` event failures are not retried.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use bytes::Bytes;
use exaudio_babel_services::review::ReviewBackend;
use exaudio_gold::polish::DraftStreamEvent;
use futures_util::stream;
use serde_json::{Value, json};
use tokio::sync::Notify;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// started, row, wait for the client to see the row, then done.
    Incremental,
    /// started, row, then the connection breaks.
    Drop,
    /// HTTP 500.
    Rejected,
    /// An `error` event.
    ErrorEvent,
}

#[derive(Clone)]
struct Fake {
    mode: Mode,
    row_seen: Arc<Notify>,
    hits: Arc<Mutex<Vec<(String, Value)>>>,
}

const STARTED: &str = "event: started\ndata: {\"totalRows\":2}\n\n";
const ROW: &str = "event: row\ndata: {\"row\":{\"rowId\":\"s1\",\"rewrittenText\":\"Первая.\",\"status\":\"rewritten\"},\"completedRows\":1,\"totalRows\":2}\n\n";
const DONE: &str = "event: done\ndata: {\"draftRows\":[{\"rowId\":\"s1\",\"rewrittenText\":\"Первая.\",\"status\":\"rewritten\"},{\"rowId\":\"s2\",\"rewrittenText\":\"Вторая\",\"status\":\"unchanged\"}],\"summary\":{\"totalRows\":2,\"rewrittenRows\":1,\"unchangedRows\":1,\"failedRows\":0}}\n\n";

fn done_json() -> Value {
    json!({
        "draftRows": [{ "rowId": "s1", "rewrittenText": "Первая.", "status": "rewritten" }],
        "summary": { "totalRows": 1, "rewrittenRows": 1, "unchangedRows": 0, "failedRows": 0 }
    })
}

async fn stream_route(State(fake): State<Fake>, body: String) -> Response {
    let payload: Value = serde_json::from_str(&body).unwrap();
    fake.hits.lock().unwrap().push(("stream".into(), payload));
    match fake.mode {
        Mode::Rejected => (StatusCode::INTERNAL_SERVER_ERROR, "boom").into_response(),
        Mode::ErrorEvent => Response::new(Body::from(format!(
            "{STARTED}event: error\ndata: {{\"error\":\"OpenRouter key rejected\"}}\n\n"
        ))),
        Mode::Incremental | Mode::Drop => {
            let steps: Vec<u8> = vec![0, 1, 2];
            let body = stream::unfold((steps.into_iter(), fake), |(mut steps, fake)| async move {
                let item: Result<Bytes, std::io::Error> = match steps.next()? {
                    0 => Ok(Bytes::from_static(STARTED.as_bytes())),
                    1 => Ok(Bytes::from_static(ROW.as_bytes())),
                    _ if fake.mode == Mode::Drop => Err(std::io::Error::other("connection reset")),
                    _ => {
                        // Only an incremental client can see the row before `done` exists.
                        fake.row_seen.notified().await;
                        Ok(Bytes::from_static(DONE.as_bytes()))
                    }
                };
                Some((item, (steps, fake)))
            });
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(body))
                .unwrap()
        }
    }
}

async fn reconcile_route(State(fake): State<Fake>, body: String) -> Response {
    let payload: Value = serde_json::from_str(&body).unwrap();
    fake.hits
        .lock()
        .unwrap()
        .push(("reconcile".into(), payload));
    axum::Json(done_json()).into_response()
}

async fn serve(mode: Mode) -> (ReviewBackend, Fake) {
    let fake = Fake {
        mode,
        row_seen: Arc::new(Notify::new()),
        hits: Arc::default(),
    };
    let app = Router::new()
        .route("/api/draft/generate/stream", post(stream_route))
        .route("/api/draft/generate", post(reconcile_route))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (ReviewBackend::new(base), fake)
}

fn payload() -> Value {
    json!({ "projectPreset": "ru-gold-2sp-v1", "jobId": "job-1", "rows": [] })
}

#[tokio::test]
async fn rows_arrive_while_the_stream_is_still_open() {
    let (backend, fake) = serve(Mode::Incremental).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let (sink, notify) = (events.clone(), fake.row_seen.clone());
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        backend.generate_drafts(payload(), Vec::new(), move |event| {
            if matches!(event, DraftStreamEvent::Row { .. }) {
                notify.notify_one();
            }
            sink.lock().unwrap().push(event);
        }),
    )
    .await
    .expect("a buffering client never sees the row before done")
    .unwrap();

    assert_eq!(result.rows.len(), 2);
    assert!(!result.reconciled_after_stream_loss);
    let events = events.lock().unwrap();
    assert_eq!(events[0], DraftStreamEvent::Started { total_rows: 2 });
    assert!(matches!(
        &events[1],
        DraftStreamEvent::Row { row, completed_rows: 1, total_rows: 2, .. } if row.row_id == "s1"
    ));
}

#[tokio::test]
async fn a_dropped_stream_is_reconciled_once_for_the_same_session() {
    let (backend, fake) = serve(Mode::Drop).await;
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let result = backend
        .generate_drafts(payload(), Vec::new(), move |event| {
            sink.lock().unwrap().push(event)
        })
        .await
        .unwrap();

    assert!(result.reconciled_after_stream_loss);
    assert!(
        events
            .lock()
            .unwrap()
            .contains(&DraftStreamEvent::Reconnecting)
    );
    let hits = fake.hits.lock().unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].0, "reconcile");
    let session = hits[0].1["draftSessionId"].as_str().unwrap();
    assert!(!session.is_empty());
    assert_eq!(hits[1].1["draftSessionId"].as_str(), Some(session));
}

#[tokio::test]
async fn http_and_error_event_failures_are_not_retried() {
    for mode in [Mode::Rejected, Mode::ErrorEvent] {
        let (backend, fake) = serve(mode).await;
        let error = backend
            .generate_drafts(payload(), Vec::new(), |_| {})
            .await
            .unwrap_err();
        let message = format!("{error:#}");
        assert!(
            message.contains("HTTP 500") || message.contains("OpenRouter key rejected"),
            "{message}"
        );
        assert_eq!(fake.hits.lock().unwrap().len(), 1);
    }
}
