//! Review backend client behavior against a local fake (axum) server.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use exaudio_babel_services::review::{
    GRADE_CATEGORIES, ProposalDecision, ReviewBackend, ReviewCredentials, ReviewSnapshot,
    l1_reference_id, normalize_review_action,
};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
struct Hit {
    path: String,
    key: Option<String>,
    model: Option<String>,
    body: Value,
}

type Hits = Arc<Mutex<Vec<Hit>>>;

fn record(hits: &Hits, path: &str, headers: &HeaderMap, body: &str) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    };
    hits.lock().unwrap().push(Hit {
        path: path.to_string(),
        key: header("x-openrouter-key"),
        model: header("x-openrouter-model"),
        body: serde_json::from_str(body).unwrap_or(Value::Null),
    });
}

fn session_json(session_id: &str) -> Value {
    json!({
        "sessionId": session_id,
        "reviewActionId": "ra-l2",
        "backendVersion": { "service": "review-backend", "release": "r1" },
        "comments": { "sessionComment": "", "cardComments": {} },
        "cards": [{
            "id": "c1", "changeIndex": 1, "type": "TEXT CHANGE", "summary": "s",
            "evidence": "raw", "evidenceDetail": { "kind": "text-diff", "before": " a b ", "after": "a c", "inlineDiff": "a [-b] [+c]" },
            "matchSource": "manual", "matchedTemplateId": "t2", "initialMatchedTemplateId": "t1",
            "initialTemplateTitle": "First", "templateTitle": "Second"
        }],
        "suggestions": [{ "proposalId": "p1", "decision": "approved" }],
    })
}

fn grades_json(review_action_id: &str) -> Value {
    // Deliberately out of rubric order.
    let mut grades: Vec<Value> = GRADE_CATEGORIES
        .iter()
        .rev()
        .map(|category| {
            json!({
                "category": category, "score": 2, "scoreCap": 3, "note": "Повторяющиеся ошибки",
                "evidence": { "count": 1, "dominantKinds": ["word"], "samples": [
                    { "kind": "word", "severity": "material", "annotationId": "a1", "note": "n", "before": "x", "after": "y" }
                ]}
            })
        })
        .collect();
    grades[0]["score"] = json!(1);
    json!({
        "reviewActionId": review_action_id, "originalActionId": "ra-l1",
        "version": "guidelines-2026-10-decisions-v5", "model": "m", "grades": grades,
    })
}

async fn fake_backend() -> (String, Hits) {
    let hits: Hits = Arc::default();
    let app = Router::new()
        .route(
            "/api/review/sessions",
            post(
                |State(hits): State<Hits>, headers: HeaderMap, body: String| async move {
                    record(&hits, "/api/review/sessions", &headers, &body);
                    axum::Json(session_json("s1"))
                },
            ),
        )
        .route(
            "/api/review/grade",
            post(
                |State(hits): State<Hits>, headers: HeaderMap, body: String| async move {
                    record(&hits, "/api/review/grade", &headers, &body);
                    let id =
                        serde_json::from_str::<Value>(&body).unwrap()["reviewActionId"].clone();
                    axum::Json(grades_json(id.as_str().unwrap_or_default()))
                },
            ),
        )
        .route(
            "/api/review/sessions/{id}/comments",
            post(
                |State(hits): State<Hits>,
                 Path(id): Path<String>,
                 headers: HeaderMap,
                 body: String| async move {
                    record(
                        &hits,
                        &format!("/api/review/sessions/{id}/comments"),
                        &headers,
                        &body,
                    );
                    axum::Json(session_json(&id))
                },
            ),
        )
        .route(
            "/api/review/sessions/{id}/template-suggestions/{proposal}/decision",
            post(
                |State(hits): State<Hits>,
                 Path((id, proposal)): Path<(String, String)>,
                 headers: HeaderMap,
                 body: String| async move {
                    record(&hits, &format!("decision/{proposal}"), &headers, &body);
                    axum::Json(session_json(&id))
                },
            ),
        )
        .route(
            "/api/review/generate",
            post(
                |State(hits): State<Hits>, headers: HeaderMap, body: String| async move {
                    record(&hits, "/api/review/generate", &headers, &body);
                    // Redirects must never carry the key anywhere.
                    Response::builder()
                        .status(StatusCode::TEMPORARY_REDIRECT)
                        .header(header::LOCATION, "/stolen")
                        .body(axum::body::Body::empty())
                        .unwrap()
                        .into_response()
                },
            ),
        )
        .route(
            "/stolen",
            get(|State(hits): State<Hits>, headers: HeaderMap| async move {
                record(&hits, "/stolen", &headers, "");
                "{}"
            })
            .post(|State(hits): State<Hits>, headers: HeaderMap| async move {
                record(&hits, "/stolen", &headers, "");
                "{}"
            }),
        )
        .with_state(hits.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, hits)
}

fn snapshot() -> ReviewSnapshot {
    let state = |id: &str, level: i64| {
        json!({
            "actionId": id, "actionLevel": level, "actionDecision": "", "lintErrors": [], "recordings": [],
            "capturedAt": "2026-01-01T00:00:00.000Z",
            "annotations": [{ "id": "a1", "content": "hello", "processedRecordingId": "pr1",
                "startTimeInSeconds": 0.0, "endTimeInSeconds": 1.0 }],
        })
    };
    ReviewSnapshot::new("ra-l2".to_string(), state("ra-l1", 1), state("ra-l2", 2)).unwrap()
}

fn credentials() -> ReviewCredentials {
    ReviewCredentials::new("sk-or-v1-test", "cloudflare/clef-omni").unwrap()
}

#[tokio::test]
async fn paid_routes_send_key_and_model_headers_and_unpaid_routes_do_not() {
    let (base, hits) = fake_backend().await;
    let backend = ReviewBackend::new(&base)
        .with_credentials(credentials())
        .unwrap();

    let session = backend.create_session(&snapshot()).await.unwrap();
    backend
        .update_comments(
            &session.session_id,
            json!({ "sessionComment": "x", "cardComments": {} }),
        )
        .await
        .unwrap();

    let hits = hits.lock().unwrap().clone();
    assert_eq!(hits[0].path, "/api/review/sessions");
    assert_eq!(hits[0].key.as_deref(), Some("sk-or-v1-test"));
    assert_eq!(hits[0].model.as_deref(), Some("cloudflare/clef-omni"));
    assert_eq!(hits[0].body["original"]["actionId"], "ra-l1");
    assert_eq!(hits[0].body["current"]["actionId"], "ra-l2");
    assert_eq!(hits[1].path, "/api/review/sessions/s1/comments");
    assert_eq!(hits[1].key, None);

    let card = &session.cards[0];
    assert!(card.display_summary().contains("a c"));
    assert_eq!(card.match_source.as_deref(), Some("manual"));
    assert_eq!(card.previous_template(), Some("First"));
    assert!(!session.suggestions[0].is_pending());
}

#[tokio::test]
async fn paid_route_without_a_key_fails_before_sending() {
    let (base, hits) = fake_backend().await;
    ReviewBackend::new(&base)
        .grade(&snapshot())
        .await
        .unwrap_err();
    assert!(hits.lock().unwrap().is_empty());
}

#[tokio::test]
async fn keyed_requests_do_not_follow_redirects() {
    let (base, hits) = fake_backend().await;
    let backend = ReviewBackend::new(&base)
        .with_credentials(credentials())
        .unwrap();
    let error = format!(
        "{:#}",
        backend.generate_review(&snapshot()).await.unwrap_err()
    );
    assert!(error.contains("307"), "{error}");
    assert!(!error.contains("sk-or-"), "{error}");
    let hits = hits.lock().unwrap();
    assert!(hits.iter().all(|hit| hit.path != "/stolen"));
}

#[tokio::test]
async fn grade_returns_five_categories_in_rubric_order() {
    let (base, hits) = fake_backend().await;
    let backend = ReviewBackend::new(&base)
        .with_credentials(credentials())
        .unwrap();
    let grade = backend.grade(&snapshot()).await.unwrap();
    let order: Vec<_> = grade.grades.iter().map(|grade| grade.category).collect();
    assert_eq!(order, GRADE_CATEGORIES);
    assert_eq!(
        grade.grades[4].score, 1,
        "Segmentation was the first item sent"
    );
    assert_eq!(grade.grades[0].samples[0].after.as_deref(), Some("y"));
    let hit = hits.lock().unwrap()[0].clone();
    assert_eq!(
        hit.body.as_object().unwrap().len(),
        3,
        "{{reviewActionId, original, current}}"
    );
}

#[tokio::test]
async fn suggestion_decisions_use_the_backend_values() {
    let (base, hits) = fake_backend().await;
    let backend = ReviewBackend::new(&base);
    backend
        .decide_template_suggestion("s1", "p1", ProposalDecision::Approved)
        .await
        .unwrap();
    backend
        .decide_template_suggestion("s1", "p1", ProposalDecision::Rejected)
        .await
        .unwrap();
    let hits = hits.lock().unwrap();
    assert_eq!(hits[0].body, json!({ "decision": "approved" }));
    assert_eq!(hits[1].body, json!({ "decision": "rejected" }));
    assert_eq!(hits[0].key, None);
}

#[test]
fn review_settings_are_validated_and_keys_only_go_to_allowed_origins() {
    assert!(ReviewCredentials::new("", "").is_err());
    assert!(ReviewCredentials::new("sk-12345", "").is_err());
    assert!(ReviewCredentials::new("sk-or-has space", "").is_err());
    assert!(ReviewCredentials::new(&format!("sk-or-{}", "x".repeat(507)), "").is_err());
    assert!(ReviewCredentials::new("sk-or-ok", "not a model").is_err());
    assert!(ReviewCredentials::new("sk-or-ok", "typesafe/jev-1.14").is_ok());
    assert!(
        ReviewCredentials::new("sk-or-ok", "")
            .unwrap()
            .model()
            .is_none()
    );
    assert!(!format!("{:?}", credentials()).contains("sk-or"));

    for allowed in [
        "https://reviewgen.ovh",
        "http://localhost:8787",
        "http://127.0.0.1:9",
    ] {
        assert!(
            ReviewBackend::new(allowed)
                .with_credentials(credentials())
                .is_ok(),
            "{allowed}"
        );
    }
    for refused in [
        "https://evil.example",
        "http://reviewgen.ovh",
        "https://reviewgen.ovh/proxy",
        "https://user@reviewgen.ovh",
    ] {
        assert!(
            ReviewBackend::new(refused)
                .with_credentials(credentials())
                .is_err(),
            "{refused}"
        );
    }
}

#[test]
fn snapshot_requires_a_distinct_l1_original() {
    let state =
        |id: &str, level: i64| json!({ "actionId": id, "actionLevel": level, "annotations": [] });
    assert!(ReviewSnapshot::new("ra".into(), state("ra", 1), state("ra", 2)).is_err());
    assert!(ReviewSnapshot::new("ra".into(), state("l1", 2), state("ra", 2)).is_err());
    assert!(ReviewSnapshot::new("ra".into(), state("l1", 1), state("ra", 1)).is_err());
    let snapshot = ReviewSnapshot::new("ra".into(), state("l1", 1), state("ra", 2)).unwrap();
    assert_eq!(snapshot.prepare_body()["babelDiff"], Value::Null);
    let mut changed = state("ra", 2);
    changed["capturedAt"] = json!("later");
    let same = ReviewSnapshot::new("ra".into(), state("l1", 1), changed).unwrap();
    assert_eq!(
        snapshot.transcript_key(),
        same.transcript_key(),
        "capture time is not content"
    );
}

#[test]
fn l1_original_is_found_and_normalized_like_the_review_helper() {
    let list = json!([
        { "id": "ra-l2", "level": 2 },
        { "id": "ra-l1", "level": 1 },
    ])
    .to_string();
    assert_eq!(l1_reference_id(&list, "ra-l2").unwrap(), "ra-l1");
    assert!(l1_reference_id(&json!([{ "id": "ra-l2", "level": 2 }]).to_string(), "ra-l2").is_err());

    let raw = json!({ "json": {
        "actionId": "ra-l1", "actionLevel": 1, "actionDecision": "pass",
        "annotations": [
            { "id": "b", "reviewActionId": "ra-l1", "type": "t", "content": 11, "processedRecordingId": "pr", "startTimeInSeconds": 5, "endTimeInSeconds": 6 },
            { "id": "a", "reviewActionId": "ra-l1", "type": "t", "content": "hi", "processedRecordingId": "pr", "startTimeInSeconds": "1.5", "endTimeInSeconds": 2 },
        ],
        "transcriptionChunkProcessedRecordings": [{ "id": "r", "transcriptionChunkId": "c", "processedRecordingId": "pr", "speaker": 1 }],
    }})
    .to_string();
    let normalized = normalize_review_action(&raw).unwrap();
    assert_eq!(normalized["actionId"], "ra-l1");
    assert_eq!(normalized["actionLevel"], 1);
    assert_eq!(normalized["annotations"][0]["id"], "a");
    assert_eq!(normalized["annotations"][0]["startTimeInSeconds"], 1.5);
    assert_eq!(normalized["annotations"][1]["content"], "11");
    assert_eq!(normalized["recordings"][0]["processedRecordingId"], "pr");
}
