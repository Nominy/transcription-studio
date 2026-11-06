//! `RemoteBackend` against a local fake coordinator that enforces the real wire contract
//! (release header, owner tokens, 428 resend, queue polling, enhancement multipart).

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{FromRequest, Multipart, Path, Request, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use bytes::Bytes;
use exaudio_gold::remote::{
    GOLD_E2E_RELEASE, GOLD_ENHANCEMENT_MODEL, RemoteBackend, RemoteConfig, TimingTokenStore,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use studio_inference::{
    AlignRequest, BackendStatus, InferenceBackend, InferenceErrorKind, InferenceProgress,
    LaneAudio, PreservedRow, ProgressSink,
};

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    release: Option<String>,
    remote_key: Option<String>,
    bearer: Option<String>,
    request_id: Option<String>,
    /// (name, file name) of multipart fields, in order.
    fields: Vec<(String, Option<String>)>,
    payload: Option<Value>,
}

struct Fake {
    release: String,
    drafting: bool,
    capabilities_status: StatusCode,
    draft_override: Option<(StatusCode, Value)>,
    tamper_enhanced_sha: bool,
    seen: Vec<Seen>,
    timings: HashMap<(String, String), Value>,
    draft_texts: HashSet<(String, String)>,
    queue_polls: u32,
    enhance_polls: u32,
}

impl Fake {
    fn new() -> Self {
        Self {
            release: GOLD_E2E_RELEASE.to_string(),
            drafting: true,
            capabilities_status: StatusCode::OK,
            draft_override: None,
            tamper_enhanced_sha: false,
            seen: Vec::new(),
            timings: HashMap::new(),
            draft_texts: HashSet::new(),
            queue_polls: 0,
            enhance_polls: 0,
        }
    }
}

type Shared = Arc<Mutex<Fake>>;

fn header_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn seen(path: &str, headers: &HeaderMap) -> Seen {
    Seen {
        path: path.to_string(),
        release: header_value(headers, "x-babel-e2e-release"),
        remote_key: header_value(headers, "x-babel-remote-key"),
        bearer: header_value(headers, "authorization")
            .and_then(|value| value.strip_prefix("Bearer ").map(str::to_string)),
        request_id: header_value(headers, "x-babel-request-id"),
        fields: Vec::new(),
        payload: None,
    }
}

async fn read_multipart(mut multipart: Multipart, record: &mut Seen) -> Vec<(String, Bytes)> {
    let mut files = Vec::new();
    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
        let file_name = field.file_name().map(str::to_string);
        let bytes = field.bytes().await.unwrap();
        if name == "payload" {
            record.payload = Some(serde_json::from_slice(&bytes).unwrap());
        } else {
            files.push((name.clone(), bytes));
        }
        record.fields.push((name, file_name));
    }
    files
}

fn models(release: &str, llm: bool) -> Value {
    json!({
        "release": release,
        "execution": { "provider": "webgpu", "observedNeuralCpuFallback": false },
        "e2e": { "audio": "original", "llm": llm },
    })
}

fn timing_json(task_id: &str, lanes: &[String], release: &str) -> Value {
    json!({
        "taskId": task_id,
        "tracks": lanes.iter().map(|lane| json!({
            "lane": lane,
            "pcmSha256": "a".repeat(64),
            "sampleRate": 16000,
            "tokens": [{ "id": "t1", "text": "привет", "startSeconds": 0.1, "endSeconds": 0.5 }],
            "segments": [{ "id": "s1", "startSeconds": 0.1, "endSeconds": 0.5,
                           "startSample": 1600, "endSample": 8000, "sampleRate": 16000 }],
        })).collect::<Vec<_>>(),
        "summary": {},
        "models": models(release, false),
    })
}

async fn capabilities(State(fake): State<Shared>, headers: HeaderMap) -> Response {
    let mut fake = fake.lock().unwrap();
    fake.seen.push(seen("capabilities", &headers));
    if fake.capabilities_status != StatusCode::OK {
        return fake.capabilities_status.into_response();
    }
    Json(json!({
        "ready": true, "protocol": "babel-shared-e2e-webgpu-v1", "release": fake.release,
        "provider": "webgpu", "audio": "original", "neuralCpuFallback": false, "llm": false,
        "operations": {
            "e2e-transcribe": { "ready": true, "llm": false },
            "e2e-draft": { "ready": fake.drafting, "llm": true },
        },
    }))
    .into_response()
}

async fn lookup(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut fake = fake.lock().unwrap();
    let record = seen("lookup", &headers);
    let key = (
        record.bearer.clone().unwrap_or_default(),
        body["taskId"].as_str().unwrap().to_string(),
    );
    fake.seen.push(record);
    match fake.timings.get(&key) {
        Some(timing) => Json(timing.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn e2e_queue(State(fake): State<Shared>, Path(id): Path<String>) -> Json<Value> {
    let mut fake = fake.lock().unwrap();
    fake.queue_polls += 1;
    if fake.queue_polls == 1 {
        Json(json!({ "requestId": id, "status": "queued", "position": 2, "queuedCount": 3 }))
    } else {
        Json(
            json!({ "requestId": id, "status": "running", "position": 0, "queuedCount": 1,
                     "progress": { "phase": "decode", "completed": 1, "total": 4, "lane": "Alice" } }),
        )
    }
}

async fn wait_for(fake: &Shared, condition: impl Fn(&Fake) -> bool) {
    for _ in 0..400 {
        if condition(&fake.lock().unwrap()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("fake coordinator condition not reached");
}

async fn transcribe(
    State(fake): State<Shared>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    let mut record = seen("transcribe", &headers);
    let files = read_multipart(multipart, &mut record).await;
    assert!(files.iter().all(|(_, bytes)| bytes.starts_with(b"RIFF")));
    // Hold the request open until the client has polled the queue twice.
    if !record.payload.as_ref().unwrap()["taskId"]
        .as_str()
        .unwrap()
        .starts_with("error-task-clip:")
    {
        wait_for(&fake, |fake| fake.queue_polls >= 2).await;
    }
    let mut fake = fake.lock().unwrap();
    let payload = record.payload.clone().unwrap();
    let task_id = payload["taskId"].as_str().unwrap().to_string();
    let lanes: Vec<String> = payload["tracks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|track| track["lane"].as_str().unwrap().to_string())
        .collect();
    let token = record.bearer.clone().unwrap();
    let mut timing = timing_json(&task_id, &lanes, &fake.release);
    fake.timings
        .insert((token.clone(), task_id), timing.clone());
    fake.seen.push(record);
    timing["accessToken"] = json!(token);
    Json(timing).into_response()
}

async fn draft(State(fake): State<Shared>, headers: HeaderMap, request: Request) -> Response {
    let mut record = seen("draft", &headers);
    let multipart = header_value(&headers, "content-type")
        .is_some_and(|value| value.starts_with("multipart/form-data"));
    if multipart {
        let parts = Multipart::from_request(request, &()).await.unwrap();
        read_multipart(parts, &mut record).await;
    } else {
        let Json(body) = Json::<Value>::from_request(request, &()).await.unwrap();
        record.payload = Some(body);
    }
    let mut fake = fake.lock().unwrap();
    let payload = record.payload.clone().unwrap();
    let key = (
        record.bearer.clone().unwrap_or_default(),
        payload["taskId"].as_str().unwrap().to_string(),
    );
    fake.seen.push(record);
    if let Some((status, body)) = fake.draft_override.clone() {
        return (status, Json(body)).into_response();
    }
    if payload["text"] != "llm" {
        return (
            StatusCode::UPGRADE_REQUIRED,
            Json(json!({ "code": "llm-drafting-required" })),
        )
            .into_response();
    }
    if !fake.timings.contains_key(&key) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "detail": "shared E2E timing not found" })),
        )
            .into_response();
    }
    if !multipart && !fake.draft_texts.contains(&key) {
        return (StatusCode::PRECONDITION_REQUIRED,
                Json(json!({ "code": "original-audio-required", "detail": "Send this task's Original audio" })))
            .into_response();
    }
    fake.draft_texts.insert(key);
    let rows = match payload.pointer("/options/preserveRows").and_then(Value::as_array) {
        Some(rows) => rows
            .iter()
            .map(|row| json!({ "id": row["rowId"], "lane": row["speakerKey"], "startSeconds": row["startSeconds"],
                               "endSeconds": row["endSeconds"], "text": "сегмент" }))
            .collect(),
        None => vec![json!({ "id": "r1", "lane": "Alice", "startSeconds": 0.1, "endSeconds": 0.5, "text": "привет" })],
    };
    Json(json!({ "rows": rows, "summary": {}, "models": models(&fake.release, true) }))
        .into_response()
}

fn wav(sample_rate: u32, frames: u32) -> Bytes {
    let data = frames * 2;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    bytes.extend((0..frames).flat_map(|index| ((index % 200) as i16 * 50).to_le_bytes()));
    Bytes::from(bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn enhance(State(fake): State<Shared>, headers: HeaderMap, multipart: Multipart) -> Response {
    let mut record = seen("enhance", &headers);
    record.release = header_value(&headers, "x-babel-inference-release");
    let files = read_multipart(multipart, &mut record).await;
    wait_for(&fake, |fake| fake.enhance_polls >= 1).await;
    let payload = record.payload.clone().unwrap();
    let tamper = {
        let mut fake = fake.lock().unwrap();
        fake.seen.push(record);
        fake.tamper_enhanced_sha
    };
    let boundary = "fake-boundary-7d1";
    let mut body = Vec::new();
    let mut tracks = Vec::new();
    let mut parts = Vec::new();
    for (index, track) in payload["tracks"].as_array().unwrap().iter().enumerate() {
        let (field, source) = &files[index];
        assert_eq!(field, track["fieldName"].as_str().unwrap());
        assert_eq!(sha256_hex(source), track["sourceSha256"].as_str().unwrap());
        let frames = track["frameCount"].as_u64().unwrap() as u32;
        let output = wav(track["sampleRate"].as_u64().unwrap() as u32, frames);
        let total = output.len() as u64;
        tracks.push(json!({
            "trackId": track["trackId"], "speakerKey": track["speakerKey"], "trackLabel": track["trackLabel"],
            "mimeType": "audio/wav", "sampleRate": track["sampleRate"], "frameCount": track["frameCount"],
            "sourceSha256": track["sourceSha256"],
            "wavSha256": if tamper { "0".repeat(64) } else { sha256_hex(&output) },
            "totalBytes": total, "chunkCount": total.div_ceil(512 * 1024),
        }));
        parts.push((field.clone(), output));
    }
    let result = json!({ "ok": true, "provider": "swarm", "taskId": payload["taskId"],
                         "model": payload["model"]["id"], "modelSha256": payload["model"]["sha256"], "tracks": tracks });
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"result\"\r\n\r\n{result}\r\n"
        )
        .as_bytes(),
    );
    for (field, output) in parts {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{}.wav\"\r\nContent-Type: audio/wav\r\n\r\n", field.replace(':', "-")).as_bytes());
        body.extend_from_slice(&output);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (
        [(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )],
        body,
    )
        .into_response()
}

async fn enhance_queue(
    State(fake): State<Shared>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    assert!(header_value(&headers, "authorization").is_some());
    fake.lock().unwrap().enhance_polls += 1;
    Json(json!({ "requestId": id, "status": "running",
                 "progress": { "phase": "enhancing", "trackId": "track-a", "trackIndex": 0, "trackCount": 2,
                               "completedChunks": 1, "totalChunks": 2 } }))
    .into_response()
}

struct Harness {
    fake: Shared,
    base_url: String,
    _dir: tempfile::TempDir,
    tokens_path: std::path::PathBuf,
}

impl Harness {
    async fn start(fake: Fake) -> Self {
        let fake = Arc::new(Mutex::new(fake));
        let app = Router::new()
            .route("/v1/e2e/capabilities", get(capabilities))
            .route("/v1/e2e/timing/lookup", post(lookup))
            .route("/v1/e2e/queue/{id}", get(e2e_queue))
            .route("/v1/e2e/transcribe", post(transcribe))
            .route("/v1/e2e/draft", post(draft))
            .route("/v1/enhance", post(enhance))
            .route("/v1/queue/{id}", get(enhance_queue))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!(
            "http://127.0.0.1:{}/",
            listener.local_addr().unwrap().port()
        );
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let dir = tempfile::tempdir().unwrap();
        let tokens_path = dir.path().join("tokens.json");
        Self {
            fake,
            base_url,
            _dir: dir,
            tokens_path,
        }
    }

    fn backend(&self, access_key: Option<&str>) -> RemoteBackend {
        let mut config = RemoteConfig::new(self.base_url.clone(), access_key.map(str::to_string));
        config.queue_poll_interval = Duration::from_millis(10);
        config.enhance_poll_interval = Duration::from_millis(10);
        RemoteBackend::new(config, Arc::new(TimingTokenStore::new(&self.tokens_path)))
    }

    fn seen(&self, path: &str) -> Vec<Seen> {
        self.fake
            .lock()
            .unwrap()
            .seen
            .iter()
            .filter(|seen| seen.path == path)
            .cloned()
            .collect()
    }
}

fn lanes() -> Vec<LaneAudio> {
    vec![
        LaneAudio {
            lane_id: "track-a".into(),
            speaker_key: "Alice".into(),
            label: "Speaker 1".into(),
            wav: wav(16000, 1600),
        },
        LaneAudio {
            lane_id: "track-b".into(),
            speaker_key: "Bob".into(),
            label: "Speaker 2".into(),
            wav: wav(16000, 1600),
        },
    ]
}

fn recorder() -> (ProgressSink, Arc<Mutex<Vec<InferenceProgress>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink_events = events.clone();
    (
        ProgressSink::new(move |event| sink_events.lock().unwrap().push(event)),
        events,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn timing_uploads_originals_reports_queue_and_reuses_the_persisted_owner_token() {
    let harness = Harness::start(Fake::new()).await;
    let backend = harness.backend(Some("secret-key"));
    let (progress, events) = recorder();

    let timing = backend.timing("task-1", &lanes(), progress).await.unwrap();

    assert_eq!(timing.task_id, "task-1");
    assert_eq!(
        timing
            .tracks
            .iter()
            .map(|track| track.lane.as_str())
            .collect::<Vec<_>>(),
        ["Alice", "Bob"]
    );
    let upload = &harness.seen("transcribe")[0];
    assert_eq!(upload.release.as_deref(), Some(GOLD_E2E_RELEASE));
    assert_eq!(upload.remote_key.as_deref(), Some("secret-key"));
    assert!(upload.request_id.is_some());
    assert_eq!(
        upload.fields,
        [
            ("payload".to_string(), None),
            ("audio:1".to_string(), Some("audio-1.wav".to_string())),
            ("audio:2".to_string(), Some("audio-2.wav".to_string()))
        ]
    );
    assert_eq!(
        upload.payload.as_ref().unwrap()["tracks"],
        json!([{ "lane": "Alice", "fieldName": "audio:1" }, { "lane": "Bob", "fieldName": "audio:2" }])
    );
    let events = events.lock().unwrap().clone();
    assert!(events.contains(&InferenceProgress::Queued {
        position: 2,
        count: 3
    }));
    assert!(events.contains(&InferenceProgress::Stage {
        stage: "decode".into(),
        completed: 1,
        total: Some(4),
        detail: Some("Alice".into()),
        clip: None
    }));

    // A fresh backend (app restart) reuses the persisted token: the coordinator's cached timing
    // is found by lookup and nothing is uploaded again.
    let token = upload.bearer.clone().unwrap();
    assert_eq!(token.len(), 43);
    let again = harness
        .backend(None)
        .timing("task-1", &lanes(), ProgressSink::none())
        .await
        .unwrap();
    assert_eq!(again, timing);
    assert_eq!(harness.seen("transcribe").len(), 1);
    assert_eq!(
        harness.seen("lookup").last().unwrap().bearer.as_deref(),
        Some(token.as_str())
    );
    // A different task gets a different owner.
    harness
        .backend(None)
        .timing("task-2", &lanes(), ProgressSink::none())
        .await
        .unwrap();
    assert_ne!(
        harness.seen("transcribe")[1].bearer.as_deref(),
        Some(token.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_resends_original_audio_once_when_the_server_requires_it() {
    let harness = Harness::start(Fake::new()).await;
    let backend = harness.backend(None);
    let timing = backend
        .timing("task-1", &lanes(), ProgressSink::none())
        .await
        .unwrap();

    let draft = backend
        .draft("task-1", &lanes(), &timing, None, ProgressSink::none())
        .await
        .unwrap();

    assert_eq!(draft.rows[0].text, "привет");
    let requests = harness.seen("draft");
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].payload,
        Some(json!({ "taskId": "task-1", "text": "llm" }))
    );
    assert!(requests[0].fields.is_empty());
    assert_eq!(
        requests[1].payload.as_ref().unwrap()["tracks"][1],
        json!({ "lane": "Bob", "fieldName": "audio:2" })
    );
    assert_eq!(requests[1].fields.len(), 3);
    assert_eq!(requests[0].bearer, requests[1].bearer);

    // The server now holds the LLM text: a single-row draft goes out as JSON only.
    let row = PreservedRow {
        row_id: "row-7".into(),
        speaker_key: "Bob".into(),
        start_seconds: 1.0,
        end_seconds: 2.0,
        text: String::new(),
        index: 0,
    };
    let segment = backend
        .draft(
            "task-1",
            &lanes(),
            &timing,
            Some(vec![row]),
            ProgressSink::none(),
        )
        .await
        .unwrap();
    assert_eq!(segment.rows[0].id, "row-7");
    let last = harness.seen("draft").pop().unwrap();
    assert!(last.fields.is_empty());
    assert_eq!(
        last.payload.unwrap()["options"]["preserveRows"][0],
        json!({ "rowId": "row-7", "speakerKey": "Bob", "startSeconds": 1.0, "endSeconds": 2.0, "text": "", "index": 0 })
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn draft_capacity_and_release_errors_are_typed() {
    let mut fake = Fake::new();
    fake.drafting = false;
    let harness = Harness::start(fake).await;
    let backend = harness.backend(None);
    let timing = backend
        .timing("task-1", &lanes(), ProgressSink::none())
        .await
        .unwrap();

    let error = backend
        .draft("task-1", &lanes(), &timing, None, ProgressSink::none())
        .await
        .unwrap_err();
    assert_eq!(error.kind, InferenceErrorKind::NoCapacity);
    assert!(
        harness.seen("draft").is_empty(),
        "no draft request without draft capability"
    );

    harness.fake.lock().unwrap().drafting = true;
    harness.fake.lock().unwrap().draft_override = Some((
        StatusCode::SERVICE_UNAVAILABLE,
        json!({ "code": "no-llm-drafting-capacity", "detail": "busy" }),
    ));
    let error = backend
        .draft("task-1", &lanes(), &timing, None, ProgressSink::none())
        .await
        .unwrap_err();
    assert_eq!(error.kind, InferenceErrorKind::NoCapacity);

    harness.fake.lock().unwrap().draft_override = Some((
        StatusCode::UPGRADE_REQUIRED,
        json!({ "code": "llm-drafting-required", "detail": "update" }),
    ));
    let error = backend
        .draft("task-1", &lanes(), &timing, None, ProgressSink::none())
        .await
        .unwrap_err();
    assert_eq!(error.kind, InferenceErrorKind::ReleaseMismatch);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_on_another_release_never_receives_audio() {
    let mut fake = Fake::new();
    fake.release = "some-other-release-r9".into();
    let harness = Harness::start(fake).await;
    let backend = harness.backend(None);

    let error = backend
        .timing("task-1", &lanes(), ProgressSink::none())
        .await
        .unwrap_err();
    assert_eq!(error.kind, InferenceErrorKind::ReleaseMismatch);
    assert!(matches!(
        backend.status().await,
        BackendStatus::Unavailable {
            kind: InferenceErrorKind::ReleaseMismatch,
            ..
        }
    ));

    harness.fake.lock().unwrap().capabilities_status = StatusCode::UPGRADE_REQUIRED;
    let error = backend
        .timing("task-1", &lanes(), ProgressSink::none())
        .await
        .unwrap_err();
    assert_eq!(error.kind, InferenceErrorKind::ReleaseMismatch);

    assert!(harness.seen("transcribe").is_empty() && harness.seen("lookup").is_empty());
    assert!(
        harness
            .seen("capabilities")
            .iter()
            .all(|seen| seen.release.as_deref() == Some(GOLD_E2E_RELEASE))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn status_reports_per_operation_capability() {
    let mut fake = Fake::new();
    fake.drafting = false;
    let harness = Harness::start(fake).await;
    let BackendStatus::Ready {
        release,
        operations,
    } = harness.backend(None).status().await
    else {
        panic!("expected ready");
    };
    assert_eq!(release, GOLD_E2E_RELEASE);
    assert!(operations.timing && !operations.draft);
}

#[tokio::test(flavor = "multi_thread")]
async fn enhancement_returns_verified_lanes_and_rejects_tampered_audio() {
    let harness = Harness::start(Fake::new()).await;
    let backend = harness.backend(None);
    let (progress, events) = recorder();

    let enhanced = backend.enhance("task-1", &lanes(), progress).await.unwrap();

    assert_eq!(
        enhanced
            .iter()
            .map(|lane| lane.lane_id.as_str())
            .collect::<Vec<_>>(),
        ["track-a", "track-b"]
    );
    assert!(
        enhanced
            .iter()
            .all(|lane| lane.model_id == GOLD_ENHANCEMENT_MODEL.id && lane.wav.len() == 44 + 3200)
    );
    let upload = &harness.seen("enhance")[0];
    assert_eq!(
        upload.payload.as_ref().unwrap()["model"]["id"],
        GOLD_ENHANCEMENT_MODEL.id
    );
    assert_eq!(
        upload
            .fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["payload", "audio:1", "audio:2"]
    );
    assert!(events.lock().unwrap().contains(&InferenceProgress::Stage {
        stage: "enhancing".into(),
        completed: 1,
        total: Some(2),
        detail: Some("track-a".into()),
        clip: None
    }));

    harness.fake.lock().unwrap().tamper_enhanced_sha = true;
    let error = backend
        .enhance("task-1", &lanes(), ProgressSink::none())
        .await
        .unwrap_err();
    assert!(error.message.contains("SHA-256"), "{error}");
}

#[tokio::test(flavor = "multi_thread")]
async fn align_uploads_one_clip_with_a_fresh_owner() {
    let harness = Harness::start(Fake::new()).await;
    let backend = harness.backend(None);
    let request = |id: &str| AlignRequest {
        task_id: format!("error-task-clip:key:{id}"),
        request_id: id.into(),
        lane: "track-a".into(),
        language: "ru".into(),
        wav: wav(16000, 1600),
    };

    let first = backend
        .align(request("req-1"), ProgressSink::none())
        .await
        .unwrap();
    backend
        .align(request("req-2"), ProgressSink::none())
        .await
        .unwrap();

    assert_eq!(first.model, GOLD_E2E_RELEASE);
    assert_eq!(first.tokens[0].text, "привет");
    let uploads = harness.seen("transcribe");
    assert_eq!(
        uploads[0].fields[1],
        (
            "audio:1".to_string(),
            Some("target-speaker-original.wav".to_string())
        )
    );
    assert_eq!(uploads[0].request_id.as_deref(), Some("req-1"));
    assert_ne!(uploads[0].bearer, uploads[1].bearer);
}

/// Read-only GET against the live public coordinator; run explicitly (`--run-ignored all`).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "network: live coordinator capabilities"]
async fn live_coordinator_negotiates_the_pinned_release() {
    let tokens = tempfile::tempdir().unwrap();
    let backend = |release: &str| {
        let mut config = RemoteConfig::new(exaudio_gold::prefs::PUBLIC_INFERENCE_BASE_URL, None);
        config.release = release.to_string();
        RemoteBackend::new(
            config,
            Arc::new(TimingTokenStore::new(tokens.path().join("tokens.json"))),
        )
    };
    // The pinned release is served (ready) or merely lacks a free GPU right now (503).
    let status = backend(GOLD_E2E_RELEASE).status().await;
    assert!(
        matches!(&status, BackendStatus::Ready { release, operations } if release == GOLD_E2E_RELEASE && operations.timing)
            || matches!(
                &status,
                BackendStatus::Unavailable {
                    kind: InferenceErrorKind::NoCapacity,
                    ..
                }
            ),
        "{status:?}"
    );
    // An unserved release is refused with 426 before any upload.
    let status = backend("not-a-served-release-r0").status().await;
    assert!(
        matches!(
            status,
            BackendStatus::Unavailable {
                kind: InferenceErrorKind::ReleaseMismatch,
                ..
            }
        ),
        "{status:?}"
    );
}
