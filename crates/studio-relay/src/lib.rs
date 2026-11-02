use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::Arc,
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{fs, io::AsyncWriteExt, process::Command, sync::Mutex};

pub type BoxedEnvelopeFuture = Pin<Box<dyn Future<Output = Result<String, RelayError>> + Send>>;
pub type EnvelopeExecutor = Arc<dyn Fn(Value) -> BoxedEnvelopeFuture + Send + Sync>;
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Clone)]
pub struct RelayConfig {
    pub store_path: PathBuf,
    pub owner_token: Option<String>,
    pub babel_service_command: Option<String>,
    pub upstream_url: Option<String>,
    pub task_cents: i64,
    pub now_millis: Clock,
    pub executor: Option<EnvelopeExecutor>,
}

impl RelayConfig {
    pub fn from_env() -> Self {
        Self {
            store_path: std::env::var_os("STUDIO_SUBACCOUNT_RELAY_STORE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(".studio-dev/subaccounts/relay-store.json")),
            owner_token: non_empty_env("STUDIO_SUBACCOUNT_OWNER_TOKEN"),
            babel_service_command: non_empty_env("STUDIO_BABEL_SERVICE_COMMAND"),
            upstream_url: non_empty_env("STUDIO_SUBACCOUNT_UPSTREAM_URL"),
            task_cents: std::env::var("STUDIO_SUBACCOUNT_TASK_CENTS")
                .ok()
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0),
            now_millis: Arc::new(now_millis),
            executor: None,
        }
    }
}

#[derive(Clone)]
pub struct RelayState {
    config: RelayConfig,
    store: Arc<Mutex<RelayStore>>,
}

impl RelayState {
    pub async fn new(config: RelayConfig) -> Result<Self, RelayError> {
        let store = RelayStore::load(&config.store_path).await?;
        Ok(Self {
            config,
            store: Arc::new(Mutex::new(store)),
        })
    }

    async fn save(&self, store: &mut RelayStore) -> Result<(), RelayError> {
        // Sweep on every save so persisted `active_claims` can never grow
        // across restarts, even if a claim is never completed.
        store.sweep_expired_claims((self.config.now_millis)());
        store.save(&self.config.store_path).await
    }

    async fn execute_envelope(&self, envelope: Value) -> Result<String, RelayError> {
        if let Some(executor) = self.config.executor.as_ref() {
            return executor(envelope).await;
        }
        if let Some(command_line) = self.config.babel_service_command.as_deref() {
            return execute_command(command_line, envelope.to_string()).await;
        }
        if let Some(upstream_url) = self.config.upstream_url.as_deref() {
            return execute_http_upstream(upstream_url, envelope).await;
        }
        Err(RelayError::bad_gateway(
            "relay upstream is not configured; set STUDIO_BABEL_SERVICE_COMMAND or STUDIO_SUBACCOUNT_UPSTREAM_URL",
        ))
    }
}

pub fn router(state: RelayState) -> Router {
    let state = Arc::new(state);
    Router::new()
        .route("/api/subaccounts/login", post(login))
        .route("/api/subaccounts/owner/dashboard", get(owner_dashboard))
        .route("/api/subaccounts/owner/subaccounts", post(owner_create))
        .route(
            "/api/subaccounts/owner/subaccounts/{subaccount_id}",
            delete(owner_revoke),
        )
        .route(
            "/api/subaccounts/owner/subaccounts/{subaccount_id}/projects",
            put(owner_assign_projects),
        )
        .route(
            "/api/subaccounts/owner/project-queues",
            put(owner_upsert_project_queue_configs),
        )
        .route("/api/subaccounts/babel-service", post(babel_service))
        .route(
            "/api/subaccounts/audio/presigned-urls",
            post(audio_presigned_urls),
        )
        .with_state(state)
}

#[derive(Debug)]
pub struct RelayError {
    status: StatusCode,
    message: String,
}

impl RelayError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl std::fmt::Display for RelayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for RelayError {}

impl IntoResponse for RelayError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<std::io::Error> for RelayError {
    fn from(error: std::io::Error) -> Self {
        RelayError::internal(error.to_string())
    }
}

impl From<serde_json::Error> for RelayError {
    fn from(error: serde_json::Error) -> Self {
        RelayError::bad_request(error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RelayStore {
    version: u8,
    subaccounts: Vec<SubaccountRecord>,
    sessions: Vec<SessionRecord>,
    #[serde(default)]
    project_queue_configs: BTreeMap<String, ProjectQueueConfig>,
}

impl Default for RelayStore {
    fn default() -> Self {
        Self {
            version: 1,
            subaccounts: Vec::new(),
            sessions: Vec::new(),
            project_queue_configs: BTreeMap::new(),
        }
    }
}

impl RelayStore {
    async fn load(path: &Path) -> Result<Self, RelayError> {
        match fs::read_to_string(path).await {
            Ok(source) => Ok(serde_json::from_str::<Self>(&source)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    async fn save(&self, path: &Path) -> Result<(), RelayError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let payload = serde_json::to_string_pretty(self)
            .map_err(|error| RelayError::internal(error.to_string()))?;
        fs::write(path, format!("{payload}\n")).await?;
        Ok(())
    }

    /// Drop expired claims and enforce the per-sub-account cap across every
    /// sub-account. Runs on every persisted save so the on-disk map stays
    /// bounded across restarts.
    fn sweep_expired_claims(&mut self, now: i64) {
        for subaccount in &mut self.subaccounts {
            sweep_expired_claims(subaccount, now);
            enforce_active_claims_cap(subaccount);
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectQueueConfig {
    #[serde(default)]
    chunked_queue_id: String,
    #[serde(default)]
    stitched_queue_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubaccountRecord {
    id: String,
    label: String,
    code: String,
    #[serde(default)]
    revoked: bool,
    #[serde(default)]
    allowed_project_ids: Vec<String>,
    #[serde(default)]
    analytics: AnalyticsRecord,
    // review-action id -> claimed-at millis. Bounded by TTL sweep
    // (ACTIVE_CLAIM_TTL_MILLIS) and hard cap (MAX_ACTIVE_CLAIMS).
    #[serde(default)]
    active_claims: BTreeMap<String, i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyticsRecord {
    completed_tasks: u64,
    worked_seconds: u64,
    earnings_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionRecord {
    token: String,
    subaccount_id: String,
    created_at: i64,
    #[serde(default)]
    last_seen_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    code: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnerCreateRequest {
    label: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnerAssignRequest {
    project_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectQueueConfigInput {
    project_id: String,
    #[serde(default)]
    chunked_queue_id: String,
    // Wire field is "stitchedTxQueueId" everywhere else in this contract (see
    // OwnerAssignRequest neighbors and studio-app's ProjectQueueConfig), which the
    // auto-derived camelCase rename would not produce from `stitched_queue_id`.
    #[serde(default, rename = "stitchedTxQueueId")]
    stitched_queue_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PushProjectQueueConfigsRequest {
    configs: Vec<ProjectQueueConfigInput>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BabelRelayRequest {
    request_kind: String,
    envelope: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AudioPresignRequest {
    review_action_id: String,
    chunked_processed_recording_ids: Vec<String>,
}

async fn login(
    State(state): State<Arc<RelayState>>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<Value>, RelayError> {
    let code = request.code.trim();
    if code.is_empty() {
        return Err(RelayError::bad_request("Sub-account code is required."));
    }
    let mut store = state.store.lock().await;
    let Some(subaccount) = store
        .subaccounts
        .iter()
        .find(|subaccount| !subaccount.revoked && constant_time_eq(&subaccount.code, code))
        .cloned()
    else {
        return Err(RelayError::unauthorized(
            "Sub-account code is invalid or revoked.",
        ));
    };
    let token = random_secret("sess");
    store
        .sessions
        .retain(|session| session.subaccount_id != subaccount.id);
    store.sessions.push(SessionRecord {
        token: token.clone(),
        subaccount_id: subaccount.id.clone(),
        created_at: (state.config.now_millis)(),
        last_seen_at: 0,
    });
    state.save(&mut store).await?;
    Ok(Json(json!({
        "session": {
            "token": token,
            "subaccountId": subaccount.id,
            "displayName": subaccount.label,
        }
    })))
}

async fn owner_dashboard(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
) -> Result<Json<Value>, RelayError> {
    require_owner(&headers, &state.config)?;
    let store = state.store.lock().await;
    Ok(Json(dashboard_payload(&store)))
}

async fn owner_create(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    Json(request): Json<OwnerCreateRequest>,
) -> Result<Json<Value>, RelayError> {
    require_owner(&headers, &state.config)?;
    let label = request.label.trim();
    if label.is_empty() {
        return Err(RelayError::bad_request("Sub-account label is required."));
    }
    let mut store = state.store.lock().await;
    store.subaccounts.push(SubaccountRecord {
        id: random_secret("sub"),
        label: label.to_string(),
        code: random_human_code(),
        revoked: false,
        allowed_project_ids: Vec::new(),
        analytics: AnalyticsRecord::default(),
        active_claims: BTreeMap::new(),
    });
    state.save(&mut store).await?;
    Ok(Json(dashboard_payload(&store)))
}

async fn owner_revoke(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    AxumPath(subaccount_id): AxumPath<String>,
) -> Result<Json<Value>, RelayError> {
    require_owner(&headers, &state.config)?;
    let mut store = state.store.lock().await;
    if let Some(subaccount) = store
        .subaccounts
        .iter_mut()
        .find(|subaccount| subaccount.id == subaccount_id)
    {
        subaccount.revoked = true;
        subaccount.code.clear();
    }
    store
        .sessions
        .retain(|session| session.subaccount_id != subaccount_id);
    state.save(&mut store).await?;
    Ok(Json(dashboard_payload(&store)))
}

async fn owner_assign_projects(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    AxumPath(subaccount_id): AxumPath<String>,
    Json(request): Json<OwnerAssignRequest>,
) -> Result<Json<Value>, RelayError> {
    require_owner(&headers, &state.config)?;
    let mut store = state.store.lock().await;
    let Some(subaccount) = store
        .subaccounts
        .iter_mut()
        .find(|subaccount| subaccount.id == subaccount_id && !subaccount.revoked)
    else {
        return Err(RelayError::not_found("Sub-account was not found."));
    };
    let mut seen = BTreeSet::new();
    subaccount.allowed_project_ids = request
        .project_ids
        .into_iter()
        .map(|project_id| project_id.trim().to_string())
        .filter(|project_id| !project_id.is_empty())
        .filter(|project_id| seen.insert(project_id.clone()))
        .collect();
    state.save(&mut store).await?;
    Ok(Json(dashboard_payload(&store)))
}

async fn owner_upsert_project_queue_configs(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    Json(request): Json<PushProjectQueueConfigsRequest>,
) -> Result<Json<Value>, RelayError> {
    require_owner(&headers, &state.config)?;
    let mut store = state.store.lock().await;
    let mut stored_project_ids = Vec::new();
    for entry in request.configs {
        let project_id = entry.project_id.trim().to_string();
        if project_id.is_empty() {
            continue;
        }
        let config = store
            .project_queue_configs
            .entry(project_id.clone())
            .or_default();
        let chunked = entry.chunked_queue_id.trim();
        if !chunked.is_empty() {
            config.chunked_queue_id = chunked.to_string();
        }
        let stitched = entry.stitched_queue_id.trim();
        if !stitched.is_empty() {
            config.stitched_queue_id = stitched.to_string();
        }
        stored_project_ids.push(project_id);
    }
    state.save(&mut store).await?;
    Ok(Json(json!({ "storedProjectIds": stored_project_ids })))
}

async fn babel_service(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    Json(request): Json<BabelRelayRequest>,
) -> Result<Json<Value>, RelayError> {
    let subaccount_id = {
        let store = state.store.lock().await;
        let (subaccount, _) = authenticate_subaccount(&headers, &store)?;
        if !queue_request_is_allowed(subaccount, &request.envelope, &store) {
            return Err(RelayError::forbidden(
                "Sub-account is not assigned to this project.",
            ));
        }
        subaccount.id.clone()
    };

    let mut payload = state.execute_envelope(request.envelope.clone()).await?;
    let mut store = state.store.lock().await;
    let now = (state.config.now_millis)();
    let mut should_save = false;
    if request.request_kind == "loadWorkerProjects" {
        let subaccount = store
            .subaccounts
            .iter()
            .find(|subaccount| subaccount.id == subaccount_id)
            .ok_or_else(|| RelayError::forbidden("Sub-account is revoked."))?;
        payload = filter_projects_payload(&payload, &subaccount.allowed_project_ids);
        let allowed_project_ids = subaccount.allowed_project_ids.clone();
        remember_project_queue_configs(&mut store, &payload);
        payload = attach_known_project_queue_configs(
            &payload,
            &store.project_queue_configs,
            &allowed_project_ids,
        );
        should_save = true;
    }
    if request.request_kind.starts_with("claim")
        && response_looks_successful(&payload)
        && let Some(subaccount) = store
            .subaccounts
            .iter_mut()
            .find(|subaccount| subaccount.id == subaccount_id)
    {
        remember_claim_start(subaccount, &payload, now);
        should_save = true;
    }
    if request.request_kind == "submitReviewAction"
        && response_looks_successful(&payload)
        && let Some(subaccount) = store
            .subaccounts
            .iter_mut()
            .find(|subaccount| subaccount.id == subaccount_id)
    {
        record_completed_task(subaccount, &request.envelope, now, state.config.task_cents);
        should_save = true;
    }
    if let Some(session) = store
        .sessions
        .iter_mut()
        .find(|session| session.subaccount_id == subaccount_id)
    {
        session.last_seen_at = now;
        should_save = true;
    }
    if should_save {
        state.save(&mut store).await?;
    }
    Ok(Json(json!({ "payload": payload })))
}

async fn audio_presigned_urls(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    Json(request): Json<AudioPresignRequest>,
) -> Result<Json<Value>, RelayError> {
    {
        let store = state.store.lock().await;
        authenticate_subaccount(&headers, &store)?;
    }
    let payload = state
        .execute_envelope(json!({
            "operation": "transcriptions.getPresignedChunkUrls",
            "payload": {
                "reviewActionId": request.review_action_id,
                "chunkedProcessedRecordingIds": request.chunked_processed_recording_ids,
            },
        }))
        .await?;
    Ok(Json(normalize_audio_presign_payload(payload)))
}

fn require_owner(headers: &HeaderMap, config: &RelayConfig) -> Result<(), RelayError> {
    let authorization = authorization_header(headers);
    if let Some(token) = config.owner_token.as_deref() {
        let expected = format!("Bearer {token}");
        if !constant_time_eq(&authorization, &expected) {
            return Err(RelayError::unauthorized("Owner authorization failed."));
        }
        return Ok(());
    }
    if authorization.trim().is_empty() {
        return Err(RelayError::unauthorized("Owner authorization is required."));
    }
    Ok(())
}

fn authenticate_subaccount<'a>(
    headers: &HeaderMap,
    store: &'a RelayStore,
) -> Result<(&'a SubaccountRecord, &'a SessionRecord), RelayError> {
    let token = authorization_header(headers)
        .strip_prefix("Bearer ")
        .unwrap_or("")
        .trim()
        .to_string();
    let Some(session) = store
        .sessions
        .iter()
        .find(|session| constant_time_eq(&session.token, &token))
    else {
        return Err(RelayError::unauthorized("Sub-account session is invalid."));
    };
    let Some(subaccount) = store
        .subaccounts
        .iter()
        .find(|subaccount| subaccount.id == session.subaccount_id)
    else {
        return Err(RelayError::forbidden("Sub-account is revoked."));
    };
    if subaccount.revoked {
        return Err(RelayError::forbidden("Sub-account is revoked."));
    }
    Ok((subaccount, session))
}

fn authorization_header(headers: &HeaderMap) -> String {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

async fn execute_command(command_line: &str, stdin: String) -> Result<String, RelayError> {
    let mut command = shell_command(command_line);
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| RelayError::bad_gateway(format!("failed to start upstream: {error}")))?;
    if let Some(mut child_stdin) = child.stdin.take() {
        child_stdin
            .write_all(stdin.as_bytes())
            .await
            .map_err(|error| {
                RelayError::bad_gateway(format!("failed to write upstream request: {error}"))
            })?;
    }
    let output = child.wait_with_output().await.map_err(|error| {
        RelayError::bad_gateway(format!("failed to read upstream response: {error}"))
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(RelayError::bad_gateway(if stderr.is_empty() {
            format!("upstream exited with {}", output.status)
        } else {
            stderr
        }));
    }
    String::from_utf8(output.stdout).map_err(|error| {
        RelayError::bad_gateway(format!("upstream response was not UTF-8: {error}"))
    })
}

async fn execute_http_upstream(upstream_url: &str, envelope: Value) -> Result<String, RelayError> {
    let response = reqwest::Client::new()
        .post(upstream_url)
        .json(&envelope)
        .send()
        .await
        .map_err(|error| {
            RelayError::bad_gateway(format!(
                "upstream HTTP request failed for {upstream_url}: {error}; verify this URL is reachable from the relay host"
            ))
        })?;
    let status = response.status();
    let body = response.text().await.map_err(|error| {
        RelayError::bad_gateway(format!(
            "upstream HTTP response failed for {upstream_url}: {error}"
        ))
    })?;
    if !status.is_success() {
        return Err(RelayError::bad_gateway(format!(
            "upstream HTTP {status} from {upstream_url}: {}",
            response_body_snippet(&body)
        )));
    }
    Ok(body)
}

fn response_body_snippet(body: &str) -> String {
    const LIMIT: usize = 300;
    let trimmed = body.trim();
    if trimmed.chars().count() > LIMIT {
        trimmed.chars().take(LIMIT).collect::<String>() + "..."
    } else {
        trimmed.to_string()
    }
}

#[cfg(windows)]
fn shell_command(command_line: &str) -> Command {
    let mut command = Command::new("cmd");
    command.arg("/C").arg(command_line);
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(not(windows))]
fn shell_command(command_line: &str) -> Command {
    let mut command = Command::new("sh");
    command.arg("-c").arg(command_line);
    command
}

fn queue_request_is_allowed(
    subaccount: &SubaccountRecord,
    envelope: &Value,
    store: &RelayStore,
) -> bool {
    if subaccount.allowed_project_ids.is_empty() {
        return true;
    }
    let Some(queue_id) = envelope
        .get("payload")
        .and_then(|payload| payload.get("queueId"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|queue_id| !queue_id.is_empty())
    else {
        return true;
    };
    project_id_for_queue_id(store, queue_id).is_some_and(|project_id| {
        subaccount
            .allowed_project_ids
            .iter()
            .any(|id| id == project_id)
    })
}

fn project_id_for_queue_id<'a>(store: &'a RelayStore, queue_id: &str) -> Option<&'a str> {
    let queue_id = queue_id.trim();
    store
        .project_queue_configs
        .iter()
        .find_map(|(project_id, config)| {
            (config.chunked_queue_id == queue_id || config.stitched_queue_id == queue_id)
                .then_some(project_id.as_str())
        })
}

fn filter_projects_payload(payload: &str, allowed_project_ids: &[String]) -> String {
    if allowed_project_ids.is_empty() {
        return payload.to_string();
    }
    let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
        return payload.to_string();
    };
    let allowed = allowed_project_ids.iter().cloned().collect::<BTreeSet<_>>();
    filter_projects_value(value, &allowed).to_string()
}

fn filter_projects_value(value: Value, allowed: &BTreeSet<String>) -> Value {
    match value {
        Value::Array(items) => {
            if items.iter().all(is_project_like) {
                Value::Array(
                    items
                        .into_iter()
                        .filter(|item| allowed.contains(project_id_for(item)))
                        .collect(),
                )
            } else {
                Value::Array(
                    items
                        .into_iter()
                        .filter_map(|item| match filter_projects_value(item, allowed) {
                            Value::Null => None,
                            value => Some(value),
                        })
                        .collect(),
                )
            }
        }
        Value::Object(map) => {
            if is_project_like(&Value::Object(map.clone()))
                && !allowed.contains(project_id_for_map(&map))
            {
                return Value::Null;
            }
            Value::Object(
                map.into_iter()
                    .filter_map(|(key, child)| match filter_projects_value(child, allowed) {
                        Value::Null => None,
                        value => Some((key, value)),
                    })
                    .collect(),
            )
        }
        value => value,
    }
}

fn remember_project_queue_configs(store: &mut RelayStore, payload: &str) {
    let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
        return;
    };
    for project in collect_project_likes(&value) {
        let project_id = project_id_for(project).to_string();
        if project_id.is_empty() {
            continue;
        }
        let config = store.project_queue_configs.entry(project_id).or_default();
        for key in ["chunkedQueueId", "chunked_queue_id"] {
            if let Some(queue_id) = project
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|queue_id| !queue_id.is_empty())
            {
                config.chunked_queue_id = queue_id.to_string();
            }
        }
        for key in ["stitchedTxQueueId", "stitchedQueueId", "stitched_queue_id"] {
            if let Some(queue_id) = project
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|queue_id| !queue_id.is_empty())
            {
                config.stitched_queue_id = queue_id.to_string();
            }
        }
    }
}

fn attach_known_project_queue_configs(
    payload: &str,
    configs: &BTreeMap<String, ProjectQueueConfig>,
    allowed_project_ids: &[String],
) -> String {
    let attachments: Vec<Value> = allowed_project_ids
        .iter()
        .filter_map(|project_id| {
            let config = configs.get(project_id)?;
            if config.chunked_queue_id.is_empty() && config.stitched_queue_id.is_empty() {
                return None;
            }
            Some(json!({
                "projectId": project_id,
                "chunkedQueueId": config.chunked_queue_id,
                "stitchedTxQueueId": config.stitched_queue_id,
            }))
        })
        .collect();
    if attachments.is_empty() {
        return payload.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(payload.trim()) else {
        return payload.to_string();
    };
    match &mut value {
        Value::Object(map) => {
            let mut merged: Vec<Value> = Vec::new();
            let mut seen_ids: BTreeSet<String> = BTreeSet::new();
            if let Some(Value::Array(existing)) = map.get("projectConfigs") {
                for entry in existing {
                    if let Some(id) = entry.get("projectId").and_then(Value::as_str) {
                        seen_ids.insert(id.to_string());
                    }
                    merged.push(entry.clone());
                }
            }
            for attachment in attachments {
                if let Some(id) = attachment.get("projectId").and_then(Value::as_str)
                    && seen_ids.insert(id.to_string())
                {
                    merged.push(attachment);
                }
            }
            map.insert("projectConfigs".to_string(), Value::Array(merged));
            value.to_string()
        }
        Value::Array(_) => json!({
            "projects": value,
            "projectConfigs": attachments,
        })
        .to_string(),
        _ => payload.to_string(),
    }
}

/// Claims older than this are treated as abandoned and swept. A
/// submitReviewAction may never arrive (or may carry an id shape that
/// resolves differently), so this bounds how long a stranded entry can live.
const ACTIVE_CLAIM_TTL_MILLIS: i64 = 24 * 60 * 60 * 1000;

/// Hard cap on tracked claims per sub-account; the oldest claims are evicted
/// first. Belt-and-braces bound on top of the TTL sweep.
const MAX_ACTIVE_CLAIMS: usize = 256;

/// Canonical claim-id resolution shared by BOTH the claim path (claim
/// response payload) and the submit path (submitReviewAction request
/// envelope), using the same key priority at both sites so a completed task
/// always removes the entry its claim inserted.
fn claim_review_action_id(value: &Value) -> Option<String> {
    find_string_by_keys(value, &["actionId", "reviewActionId", "id"])
}

/// Drop claims older than ACTIVE_CLAIM_TTL_MILLIS. Claims whose timestamp is
/// missing/legacy (0) are older than any real cutoff and are swept too.
fn sweep_expired_claims(subaccount: &mut SubaccountRecord, now: i64) {
    let cutoff = now.saturating_sub(ACTIVE_CLAIM_TTL_MILLIS);
    subaccount
        .active_claims
        .retain(|_, claimed_at| *claimed_at >= cutoff);
}

/// Belt-and-braces bound: never track more than MAX_ACTIVE_CLAIMS per
/// sub-account, evicting the oldest claims first.
fn enforce_active_claims_cap(subaccount: &mut SubaccountRecord) {
    while subaccount.active_claims.len() > MAX_ACTIVE_CLAIMS {
        let Some(oldest_id) = subaccount
            .active_claims
            .iter()
            .min_by_key(|(_, claimed_at)| **claimed_at)
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        subaccount.active_claims.remove(&oldest_id);
    }
}

fn remember_claim_start(subaccount: &mut SubaccountRecord, payload: &str, now: i64) {
    sweep_expired_claims(subaccount, now);
    let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
        return;
    };
    if let Some(review_action_id) = claim_review_action_id(&value) {
        subaccount.active_claims.insert(review_action_id, now);
        enforce_active_claims_cap(subaccount);
    }
}

fn record_completed_task(
    subaccount: &mut SubaccountRecord,
    envelope: &Value,
    now: i64,
    task_cents: i64,
) {
    subaccount.analytics.completed_tasks += 1;
    // Resolve with the same canonical function and key priority as the claim
    // path; fall back to the explicit payload.reviewActionId in case the
    // envelope nests a higher-priority key the claim response did not have.
    let started_at = claim_review_action_id(envelope)
        .and_then(|id| subaccount.active_claims.remove(&id))
        .or_else(|| {
            envelope
                .get("payload")
                .and_then(|payload| payload.get("reviewActionId"))
                .and_then(Value::as_str)
                .and_then(|id| subaccount.active_claims.remove(id))
        });
    if let Some(started_at) = started_at {
        let seconds = ((now - started_at).max(0) / 1000) as u64;
        subaccount.analytics.worked_seconds += seconds;
    }
    subaccount.analytics.earnings_cents += task_cents;
}

fn response_looks_successful(payload: &str) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
        return true;
    };
    find_bool_by_keys(&value, &["success", "ok"]).unwrap_or(true)
}

fn dashboard_payload(store: &RelayStore) -> Value {
    json!({
        "subaccounts": store.subaccounts.iter()
            .filter(|subaccount| !subaccount.revoked)
            .map(|subaccount| {
            json!({
                "id": subaccount.id,
                "label": subaccount.label,
                "code": if subaccount.revoked { Value::Null } else { Value::String(subaccount.code.clone()) },
                "revoked": subaccount.revoked,
                "allowedProjectIds": subaccount.allowed_project_ids,
                "analytics": {
                    "completedTasks": subaccount.analytics.completed_tasks,
                    "workedSeconds": subaccount.analytics.worked_seconds,
                    "earningsCents": subaccount.analytics.earnings_cents,
                }
            })
        }).collect::<Vec<_>>()
    })
}

fn collect_project_likes(value: &Value) -> Vec<&Value> {
    let mut projects = Vec::new();
    collect_project_likes_into(value, &mut projects);
    projects
}

fn collect_project_likes_into<'a>(value: &'a Value, projects: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_project_likes_into(item, projects);
            }
        }
        Value::Object(map) => {
            if is_project_like(value) {
                projects.push(value);
            }
            for child in map.values() {
                collect_project_likes_into(child, projects);
            }
        }
        _ => {}
    }
}

fn normalize_audio_presign_payload(payload: String) -> Value {
    let Ok(value) = serde_json::from_str::<Value>(&payload) else {
        return json!({ "payload": payload });
    };
    let urls = trpc_json_payload(&value)
        .or_else(|| value.get("urls"))
        .or_else(|| value.get("presignedUrls"))
        .or_else(|| value.get("data"))
        .unwrap_or(&value);
    if urls.is_object() {
        json!({ "urls": urls.clone() })
    } else {
        json!({ "payload": value })
    }
}

fn trpc_json_payload(value: &Value) -> Option<&Value> {
    if let Value::Array(items) = value {
        return items.first().and_then(trpc_json_payload);
    }
    value
        .pointer("/result/data/json")
        .or_else(|| value.pointer("/data/json"))
        .or_else(|| value.pointer("/result/data"))
        .or_else(|| value.pointer("/data"))
}

fn is_project_like(value: &Value) -> bool {
    !project_id_for(value).is_empty()
}

fn project_id_for(value: &Value) -> &str {
    value
        .get("projectId")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

fn project_id_for_map(map: &serde_json::Map<String, Value>) -> &str {
    map.get("projectId")
        .or_else(|| map.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
}

fn find_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .find_map(|item| find_string_by_keys(item, keys)),
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(value.to_string());
                }
            }
            map.values()
                .find_map(|child| find_string_by_keys(child, keys))
        }
        _ => None,
    }
}

fn find_bool_by_keys(value: &Value, keys: &[&str]) -> Option<bool> {
    match value {
        Value::Array(items) => items.iter().find_map(|item| find_bool_by_keys(item, keys)),
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key).and_then(Value::as_bool) {
                    return Some(value);
                }
            }
            map.values()
                .find_map(|child| find_bool_by_keys(child, keys))
        }
        _ => None,
    }
}

fn random_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 18];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}_{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn random_human_code() -> String {
    let mut bytes = [0u8; 6];

    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes).to_ascii_uppercase()
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0u8, |diff, (left, right)| diff | (left ^ right))
        == 0
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    };

    use axum::http::StatusCode;
    use reqwest::Client;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;
    #[test]
    fn restricted_subaccount_allows_known_queue_ids_for_assigned_project() {
        let subaccount = SubaccountRecord {
            id: "sub-1".to_string(),
            label: "Tester".to_string(),
            code: "CODE".to_string(),
            allowed_project_ids: vec!["project-allowed".to_string()],
            revoked: false,
            analytics: AnalyticsRecord::default(),
            active_claims: BTreeMap::new(),
        };
        let mut store = RelayStore::default();
        store.project_queue_configs.insert(
            "project-allowed".to_string(),
            ProjectQueueConfig {
                chunked_queue_id: "queue-allowed".to_string(),
                stitched_queue_id: "queue-allowed-stitched".to_string(),
            },
        );
        store.project_queue_configs.insert(
            "project-blocked".to_string(),
            ProjectQueueConfig {
                chunked_queue_id: "queue-blocked".to_string(),
                stitched_queue_id: "queue-blocked-stitched".to_string(),
            },
        );

        assert!(queue_request_is_allowed(
            &subaccount,
            &json!({ "payload": { "queueId": "queue-allowed" } }),
            &store
        ));
        assert!(!queue_request_is_allowed(
            &subaccount,
            &json!({ "payload": { "queueId": "queue-blocked" } }),
            &store
        ));
    }

    #[test]
    fn completed_submit_removes_claim_with_mismatched_id_shapes() {
        let mut subaccount = SubaccountRecord {
            id: "sub-1".to_string(),
            label: "Tester".to_string(),
            code: "CODE".to_string(),
            allowed_project_ids: Vec::new(),
            revoked: false,
            analytics: AnalyticsRecord::default(),
            active_claims: BTreeMap::new(),
        };
        let now = 1_783_425_600_000;
        // Claim response carries the id under "actionId" in tRPC nesting.
        remember_claim_start(
            &mut subaccount,
            &json!({
                "result": { "data": { "json": { "actionId": "review-action-9" } } }
            })
            .to_string(),
            now,
        );
        assert!(subaccount.active_claims.contains_key("review-action-9"));

        // Submit envelope carries the same id under "reviewActionId", nested
        // deeper than the historical direct payload.reviewActionId lookup saw.
        record_completed_task(
            &mut subaccount,
            &json!({
                "operation": "transcriptions.submitTranscriptReviewAction",
                "payload": { "submission": { "reviewActionId": "review-action-9" } }
            }),
            now + 60_000,
            125,
        );

        assert!(subaccount.active_claims.is_empty());
        assert_eq!(subaccount.analytics.completed_tasks, 1);
        assert_eq!(subaccount.analytics.worked_seconds, 60);
    }

    #[test]
    fn expired_claims_are_swept_on_insert_and_save() {
        let mut subaccount = SubaccountRecord {
            id: "sub-1".to_string(),
            label: "Tester".to_string(),
            code: "CODE".to_string(),
            allowed_project_ids: Vec::new(),
            revoked: false,
            analytics: AnalyticsRecord::default(),
            active_claims: BTreeMap::new(),
        };
        let now = 1_783_425_600_000_i64;
        subaccount
            .active_claims
            .insert("stale".to_string(), now - ACTIVE_CLAIM_TTL_MILLIS - 1);
        subaccount
            .active_claims
            .insert("boundary".to_string(), now - ACTIVE_CLAIM_TTL_MILLIS);
        subaccount
            .active_claims
            .insert("fresh".to_string(), now - 1_000);

        // The claim-insertion path sweeps expired entries.
        remember_claim_start(
            &mut subaccount,
            &json!({ "actionId": "new-claim" }).to_string(),
            now,
        );
        assert!(!subaccount.active_claims.contains_key("stale"));
        assert!(subaccount.active_claims.contains_key("boundary"));
        assert!(subaccount.active_claims.contains_key("fresh"));
        assert!(subaccount.active_claims.contains_key("new-claim"));

        // The store-level sweep (run on every save) drops the rest once they
        // age past the TTL.
        let mut store = RelayStore::default();
        store.subaccounts.push(subaccount);
        store.sweep_expired_claims(now + ACTIVE_CLAIM_TTL_MILLIS + 1);
        assert!(store.subaccounts[0].active_claims.is_empty());
    }

    #[test]
    fn active_claims_cap_evicts_oldest() {
        let mut subaccount = SubaccountRecord {
            id: "sub-1".to_string(),
            label: "Tester".to_string(),
            code: "CODE".to_string(),
            allowed_project_ids: Vec::new(),
            revoked: false,
            analytics: AnalyticsRecord::default(),
            active_claims: BTreeMap::new(),
        };
        let base = 1_783_425_600_000_i64;
        for index in 0..(MAX_ACTIVE_CLAIMS + 10) {
            remember_claim_start(
                &mut subaccount,
                &json!({ "actionId": format!("claim-{index}") }).to_string(),
                base + index as i64,
            );
        }
        assert_eq!(subaccount.active_claims.len(), MAX_ACTIVE_CLAIMS);
        assert!(!subaccount.active_claims.contains_key("claim-0"));
        assert!(!subaccount.active_claims.contains_key("claim-9"));
        assert!(subaccount.active_claims.contains_key("claim-10"));
        assert!(
            subaccount
                .active_claims
                .contains_key(&format!("claim-{}", MAX_ACTIVE_CLAIMS + 9))
        );
    }

    #[test]
    fn legacy_store_json_without_claim_timestamps_still_deserializes() {
        // Stores written before activeClaims existed must keep loading.
        let without_claims: SubaccountRecord = serde_json::from_value(json!({
            "id": "sub-1",
            "label": "Tester",
            "code": "CODE",
            "revoked": false,
            "allowedProjectIds": []
        }))
        .expect("subaccount without activeClaims");
        assert!(without_claims.active_claims.is_empty());

        // Existing stores carry plain id -> claimed-at-millis entries.
        let mut with_claims: SubaccountRecord = serde_json::from_value(json!({
            "id": "sub-2",
            "label": "Tester",
            "code": "CODE",
            "activeClaims": { "legacy-claim": 0, "tracked": 1_783_425_600_000_i64 }
        }))
        .expect("subaccount with legacy activeClaims");
        assert_eq!(with_claims.active_claims.len(), 2);

        // A legacy entry without a meaningful timestamp (0) is older than any
        // TTL cutoff and is swept instead of lingering forever.
        sweep_expired_claims(&mut with_claims, 1_783_425_600_000);
        assert!(!with_claims.active_claims.contains_key("legacy-claim"));
        assert!(with_claims.active_claims.contains_key("tracked"));
    }

    #[test]
    fn normalizes_trpc_audio_presign_payload_for_subaccount_clients() {
        let payload = json!([{
            "result": {
                "data": {
                    "json": {
                        "chunked-recording-1": "https://signed.example/audio.wav"
                    }
                }
            }
        }])
        .to_string();

        assert_eq!(
            normalize_audio_presign_payload(payload),
            json!({
                "urls": {
                    "chunked-recording-1": "https://signed.example/audio.wav"
                }
            })
        );
    }

    #[tokio::test]
    async fn owner_can_create_assign_login_relay_and_collect_analytics() {
        let temp = tempfile::tempdir().expect("temp dir");
        let now = Arc::new(AtomicI64::new(1_783_425_600_000));
        let executor_now = now.clone();
        let state = test_state(
            temp.path().join("store.json"),
            now.clone(),
            Arc::new(move |envelope| {
                let _now = executor_now.load(Ordering::SeqCst);
                Box::pin(async move {
                    let operation = envelope
                        .get("operation")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match operation {
                        "worker.get,worker.queryWorkerMetadata,worker.getProjects" => Ok(json!({
                            "projects": [
                                { "projectId": "project-1", "name": "Allowed", "chunkedQueueId": "queue-1" },
                                { "projectId": "project-2", "name": "Blocked", "chunkedQueueId": "queue-2" }
                            ]
                        })
                        .to_string()),
                        "transcriptions.claimNextReviewActionFromReviewQueue" => {
                            Ok(json!({ "success": true, "actionId": "review-action-1" }).to_string())
                        }
                        "transcriptions.submitTranscriptReviewAction" => {
                            Ok(json!({ "success": true, "updatedAt": "2026-07-07T12:05:00Z" }).to_string())
                        }
                        other => Err(RelayError::bad_gateway(format!("unexpected {other}"))),
                    }
                })
            }),
            125,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();

        let created: Value = client
            .post(format!("{base_url}/api/subaccounts/owner/subaccounts"))
            .bearer_auth("owner-token")
            .json(&json!({ "label": "Laptop" }))
            .send()
            .await
            .expect("create response")
            .json()
            .await
            .expect("create json");
        let subaccount = &created["subaccounts"][0];
        let subaccount_id = subaccount["id"].as_str().expect("subaccount id");
        let code = subaccount["code"].as_str().expect("subaccount code");

        client
            .put(format!(
                "{base_url}/api/subaccounts/owner/subaccounts/{subaccount_id}/projects"
            ))
            .bearer_auth("owner-token")
            .json(&json!({ "projectIds": ["project-1"] }))
            .send()
            .await
            .expect("assign response");

        let login: Value = client
            .post(format!("{base_url}/api/subaccounts/login"))
            .json(&json!({ "code": code }))
            .send()
            .await
            .expect("login response")
            .json()
            .await
            .expect("login json");
        let token = login["session"]["token"].as_str().expect("session token");

        let projects: Value = client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth(token)
            .json(&json!({
                "requestKind": "loadWorkerProjects",
                "envelope": { "operation": "worker.get,worker.queryWorkerMetadata,worker.getProjects" }
            }))
            .send()
            .await
            .expect("projects response")
            .json()
            .await
            .expect("projects json");
        let payload: Value = serde_json::from_str(projects["payload"].as_str().expect("payload"))
            .expect("payload json");
        assert_eq!(payload["projects"].as_array().expect("projects").len(), 1);
        assert_eq!(payload["projects"][0]["projectId"], "project-1");

        client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth(token)
            .json(&json!({
                "requestKind": "claimNextFromQueue",
                "envelope": {
                    "operation": "transcriptions.claimNextReviewActionFromReviewQueue",
                    "payload": { "queueId": "queue-1" }
                }
            }))
            .send()
            .await
            .expect("claim response");
        now.fetch_add(300_000, Ordering::SeqCst);
        client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth(token)
            .json(&json!({
                "requestKind": "submitReviewAction",
                "envelope": {
                    "operation": "transcriptions.submitTranscriptReviewAction",
                    "payload": { "reviewActionId": "review-action-1", "annotations": [] }
                }
            }))
            .send()
            .await
            .expect("submit response");

        let dashboard: Value = client
            .get(format!("{base_url}/api/subaccounts/owner/dashboard"))
            .bearer_auth("owner-token")
            .send()
            .await
            .expect("dashboard response")
            .json()
            .await
            .expect("dashboard json");
        assert_eq!(
            dashboard["subaccounts"][0]["analytics"]["completedTasks"],
            1
        );
        assert_eq!(
            dashboard["subaccounts"][0]["analytics"]["workedSeconds"],
            300
        );
        assert_eq!(
            dashboard["subaccounts"][0]["analytics"]["earningsCents"],
            125
        );
        handle.abort();
    }

    #[tokio::test]
    async fn assigned_project_queues_are_enforced_after_discovery() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(
            temp.path().join("store.json"),
            Arc::new(AtomicI64::new(1_783_425_600_000)),
            Arc::new(|envelope| {
                Box::pin(async move {
                    let operation = envelope
                        .get("operation")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if operation == "worker.get,worker.queryWorkerMetadata,worker.getProjects" {
                        Ok(json!({
                            "projects": [
                                { "projectId": "project-1", "name": "Allowed", "chunkedQueueId": "queue-1" }
                            ]
                        })
                        .to_string())
                    } else {
                        Ok(json!({ "success": true, "actionId": "review-action-2" }).to_string())
                    }
                })
            }),
            0,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();
        let created: Value = client
            .post(format!("{base_url}/api/subaccounts/owner/subaccounts"))
            .bearer_auth("owner-token")
            .json(&json!({ "label": "Phone" }))
            .send()
            .await
            .expect("create response")
            .json()
            .await
            .expect("create json");
        let subaccount_id = created["subaccounts"][0]["id"].as_str().expect("id");
        let code = created["subaccounts"][0]["code"].as_str().expect("code");
        client
            .put(format!(
                "{base_url}/api/subaccounts/owner/subaccounts/{subaccount_id}/projects"
            ))
            .bearer_auth("owner-token")
            .json(&json!({ "projectIds": ["project-1"] }))
            .send()
            .await
            .expect("assign response");
        let login: Value = client
            .post(format!("{base_url}/api/subaccounts/login"))
            .json(&json!({ "code": code }))
            .send()
            .await
            .expect("login response")
            .json()
            .await
            .expect("login json");
        let token = login["session"]["token"].as_str().expect("session token");
        client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth(token)
            .json(&json!({
                "requestKind": "loadWorkerProjects",
                "envelope": { "operation": "worker.get,worker.queryWorkerMetadata,worker.getProjects" }
            }))
            .send()
            .await
            .expect("projects response");
        let blocked = client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth(token)
            .json(&json!({
                "requestKind": "claimNextFromQueue",
                "envelope": {
                    "operation": "transcriptions.claimNextReviewActionFromReviewQueue",
                    "payload": { "queueId": "queue-2" }
                }
            }))
            .send()
            .await
            .expect("blocked response");
        assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
        handle.abort();
    }

    #[tokio::test]
    async fn revoke_archives_credentials_and_returns_only_active_subaccounts() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("store.json");
        let mut active_claims = BTreeMap::new();
        active_claims.insert("review-action-pending".to_string(), 1_783_425_600_000);
        RelayStore {
            version: 1,
            subaccounts: vec![
                SubaccountRecord {
                    id: "sub-revoked".to_string(),
                    label: "Revoked laptop".to_string(),
                    code: "REVOKE-CODE".to_string(),
                    revoked: false,
                    allowed_project_ids: vec!["project-original".to_string()],
                    analytics: AnalyticsRecord {
                        completed_tasks: 7,
                        worked_seconds: 3_600,
                        earnings_cents: 875,
                    },
                    active_claims,
                },
                SubaccountRecord {
                    id: "sub-active".to_string(),
                    label: "Active phone".to_string(),
                    code: "ACTIVE-CODE".to_string(),
                    revoked: false,
                    allowed_project_ids: vec!["project-active".to_string()],
                    analytics: AnalyticsRecord::default(),
                    active_claims: BTreeMap::new(),
                },
            ],
            sessions: vec![
                SessionRecord {
                    token: "revoked-session-1".to_string(),
                    subaccount_id: "sub-revoked".to_string(),
                    created_at: 1,
                    last_seen_at: 2,
                },
                SessionRecord {
                    token: "active-session".to_string(),
                    subaccount_id: "sub-active".to_string(),
                    created_at: 3,
                    last_seen_at: 4,
                },
                SessionRecord {
                    token: "revoked-session-2".to_string(),
                    subaccount_id: "sub-revoked".to_string(),
                    created_at: 5,
                    last_seen_at: 6,
                },
            ],
            project_queue_configs: BTreeMap::new(),
        }
        .save(&store_path)
        .await
        .expect("seed store");
        let state = test_state(
            store_path.clone(),
            Arc::new(AtomicI64::new(1_783_425_600_000)),
            Arc::new(|_| Box::pin(async { Ok(json!({ "success": true }).to_string()) })),
            0,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();

        let revoke_response = client
            .delete(format!(
                "{base_url}/api/subaccounts/owner/subaccounts/sub-revoked"
            ))
            .bearer_auth("owner-token")
            .send()
            .await
            .expect("revoke response");
        assert_eq!(revoke_response.status(), StatusCode::OK);
        let revoke_dashboard: Value = revoke_response.json().await.expect("revoke dashboard");
        assert_eq!(
            revoke_dashboard["subaccounts"],
            json!([{
                "id": "sub-active",
                "label": "Active phone",
                "code": "ACTIVE-CODE",
                "revoked": false,
                "allowedProjectIds": ["project-active"],
                "analytics": {
                    "completedTasks": 0,
                    "workedSeconds": 0,
                    "earningsCents": 0,
                }
            }])
        );
        assert!(!revoke_dashboard.to_string().contains("REVOKE-CODE"));

        let dashboard: Value = client
            .get(format!("{base_url}/api/subaccounts/owner/dashboard"))
            .bearer_auth("owner-token")
            .send()
            .await
            .expect("dashboard response")
            .json()
            .await
            .expect("dashboard json");
        assert_eq!(dashboard, revoke_dashboard);

        let revoked_login = client
            .post(format!("{base_url}/api/subaccounts/login"))
            .json(&json!({ "code": "REVOKE-CODE" }))
            .send()
            .await
            .expect("revoked login response");
        assert_eq!(revoked_login.status(), StatusCode::UNAUTHORIZED);

        let revoked_session = client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth("revoked-session-1")
            .json(&json!({
                "requestKind": "loadWorkerProjects",
                "envelope": { "operation": "worker.getProjects" }
            }))
            .send()
            .await
            .expect("revoked session response");
        assert_eq!(revoked_session.status(), StatusCode::UNAUTHORIZED);

        let assign_revoked = client
            .put(format!(
                "{base_url}/api/subaccounts/owner/subaccounts/sub-revoked/projects"
            ))
            .bearer_auth("owner-token")
            .json(&json!({ "projectIds": ["project-replacement"] }))
            .send()
            .await
            .expect("assign revoked response");
        assert_eq!(assign_revoked.status(), StatusCode::NOT_FOUND);

        let retry_response = client
            .delete(format!(
                "{base_url}/api/subaccounts/owner/subaccounts/sub-revoked"
            ))
            .bearer_auth("owner-token")
            .send()
            .await
            .expect("retry revoke response");
        assert_eq!(retry_response.status(), StatusCode::OK);
        let retry_dashboard: Value = retry_response.json().await.expect("retry dashboard");
        assert_eq!(retry_dashboard, revoke_dashboard);

        let active_login = client
            .post(format!("{base_url}/api/subaccounts/login"))
            .json(&json!({ "code": "ACTIVE-CODE" }))
            .send()
            .await
            .expect("active login response");
        assert_eq!(active_login.status(), StatusCode::OK);

        let persisted = RelayStore::load(&store_path)
            .await
            .expect("persisted store");
        let tombstone = persisted
            .subaccounts
            .iter()
            .find(|subaccount| subaccount.id == "sub-revoked")
            .expect("revoked tombstone");
        assert!(tombstone.revoked);
        assert!(tombstone.code.is_empty());
        assert_eq!(
            tombstone.allowed_project_ids,
            vec!["project-original".to_string()]
        );
        assert_eq!(tombstone.analytics.completed_tasks, 7);
        assert_eq!(tombstone.analytics.worked_seconds, 3_600);
        assert_eq!(tombstone.analytics.earnings_cents, 875);
        assert_eq!(
            tombstone.active_claims.get("review-action-pending"),
            Some(&1_783_425_600_000)
        );
        assert!(
            persisted
                .sessions
                .iter()
                .all(|session| session.subaccount_id != "sub-revoked")
        );
        assert!(
            persisted
                .sessions
                .iter()
                .any(|session| session.subaccount_id == "sub-active")
        );
        handle.abort();
    }

    #[tokio::test]
    async fn owner_project_queues_endpoint_requires_owner_auth() {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(
            temp.path().join("store.json"),
            Arc::new(AtomicI64::new(1_783_425_600_000)),
            Arc::new(|_| Box::pin(async { Ok(json!({ "success": true }).to_string()) })),
            0,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();

        let missing_auth = client
            .put(format!("{base_url}/api/subaccounts/owner/project-queues"))
            .json(&json!({ "configs": [] }))
            .send()
            .await
            .expect("missing auth response");
        assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);

        let wrong_auth = client
            .put(format!("{base_url}/api/subaccounts/owner/project-queues"))
            .bearer_auth("not-the-owner-token")
            .json(&json!({ "configs": [] }))
            .send()
            .await
            .expect("wrong auth response");
        assert_eq!(wrong_auth.status(), StatusCode::UNAUTHORIZED);

        handle.abort();
    }

    #[tokio::test]
    async fn owner_project_queues_endpoint_persists_durably() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("store.json");
        let state = test_state(
            store_path.clone(),
            Arc::new(AtomicI64::new(1_783_425_600_000)),
            Arc::new(|_| Box::pin(async { Ok(json!({ "success": true }).to_string()) })),
            0,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();

        let response = client
            .put(format!("{base_url}/api/subaccounts/owner/project-queues"))
            .bearer_auth("owner-token")
            .json(&json!({
                "configs": [
                    {
                        "projectId": "project-durable",
                        "chunkedQueueId": "chunked-durable",
                        "stitchedTxQueueId": "stitched-durable",
                    }
                ]
            }))
            .send()
            .await
            .expect("push response");
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.expect("push json");
        assert_eq!(body["storedProjectIds"], json!(["project-durable"]));

        handle.abort();

        let persisted = RelayStore::load(&store_path)
            .await
            .expect("persisted store");
        let config = persisted
            .project_queue_configs
            .get("project-durable")
            .expect("persisted config");
        assert_eq!(config.chunked_queue_id, "chunked-durable");
        assert_eq!(config.stitched_queue_id, "stitched-durable");
    }

    #[tokio::test]
    async fn owner_project_queues_endpoint_never_clobbers_known_field_with_blank() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store_path = temp.path().join("store.json");
        let state = test_state(
            store_path.clone(),
            Arc::new(AtomicI64::new(1_783_425_600_000)),
            Arc::new(|_| Box::pin(async { Ok(json!({ "success": true }).to_string()) })),
            0,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();

        client
            .put(format!("{base_url}/api/subaccounts/owner/project-queues"))
            .bearer_auth("owner-token")
            .json(&json!({
                "configs": [
                    { "projectId": "project-partial", "chunkedQueueId": "c1", "stitchedTxQueueId": "s1" }
                ]
            }))
            .send()
            .await
            .expect("first push response");

        client
            .put(format!("{base_url}/api/subaccounts/owner/project-queues"))
            .bearer_auth("owner-token")
            .json(&json!({
                "configs": [
                    { "projectId": "project-partial", "chunkedQueueId": "", "stitchedTxQueueId": "s2" }
                ]
            }))
            .send()
            .await
            .expect("second push response");

        handle.abort();

        let persisted = RelayStore::load(&store_path)
            .await
            .expect("persisted store");
        let config = persisted
            .project_queue_configs
            .get("project-partial")
            .expect("persisted config");
        assert_eq!(config.chunked_queue_id, "c1");
        assert_eq!(config.stitched_queue_id, "s2");
    }

    #[tokio::test]
    async fn subaccount_load_worker_projects_surfaces_owner_pushed_queue_configs_for_unlisted_project()
     {
        let temp = tempfile::tempdir().expect("temp dir");
        let state = test_state(
            temp.path().join("store.json"),
            Arc::new(AtomicI64::new(1_783_425_600_000)),
            Arc::new(|envelope| {
                Box::pin(async move {
                    let operation = envelope
                        .get("operation")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if operation == "worker.get,worker.queryWorkerMetadata,worker.getProjects" {
                        Ok(json!({
                            "projects": [
                                { "projectId": "project-dg", "name": "Gold DG" }
                            ]
                        })
                        .to_string())
                    } else {
                        Ok(json!({ "success": true }).to_string())
                    }
                })
            }),
            0,
        )
        .await;
        let (base_url, handle) = spawn_server(state).await;
        let client = Client::new();

        let created: Value = client
            .post(format!("{base_url}/api/subaccounts/owner/subaccounts"))
            .bearer_auth("owner-token")
            .json(&json!({ "label": "Reviewer" }))
            .send()
            .await
            .expect("create response")
            .json()
            .await
            .expect("create json");
        let subaccount_id = created["subaccounts"][0]["id"].as_str().expect("id");
        let code = created["subaccounts"][0]["code"].as_str().expect("code");

        client
            .put(format!(
                "{base_url}/api/subaccounts/owner/subaccounts/{subaccount_id}/projects"
            ))
            .bearer_auth("owner-token")
            .json(&json!({ "projectIds": ["project-dg"] }))
            .send()
            .await
            .expect("assign response");

        client
            .put(format!("{base_url}/api/subaccounts/owner/project-queues"))
            .bearer_auth("owner-token")
            .json(&json!({
                "configs": [
                    { "projectId": "project-dg", "chunkedQueueId": "dg-chunked", "stitchedTxQueueId": "dg-stitched" }
                ]
            }))
            .send()
            .await
            .expect("push queue config response");

        let login: Value = client
            .post(format!("{base_url}/api/subaccounts/login"))
            .json(&json!({ "code": code }))
            .send()
            .await
            .expect("login response")
            .json()
            .await
            .expect("login json");
        let token = login["session"]["token"].as_str().expect("session token");

        let projects: Value = client
            .post(format!("{base_url}/api/subaccounts/babel-service"))
            .bearer_auth(token)
            .json(&json!({
                "requestKind": "loadWorkerProjects",
                "envelope": { "operation": "worker.get,worker.queryWorkerMetadata,worker.getProjects" }
            }))
            .send()
            .await
            .expect("projects response")
            .json()
            .await
            .expect("projects json");
        let payload: Value = serde_json::from_str(projects["payload"].as_str().expect("payload"))
            .expect("payload json");
        let configs = payload["projectConfigs"]
            .as_array()
            .expect("project configs");
        let dg_config = configs
            .iter()
            .find(|config| config["projectId"] == "project-dg")
            .expect("dg config present");
        assert_eq!(dg_config["chunkedQueueId"], "dg-chunked");
        assert_eq!(dg_config["stitchedTxQueueId"], "dg-stitched");

        handle.abort();
    }

    #[tokio::test]
    async fn upstream_connection_error_identifies_relay_reachability() {
        let upstream_url = "http://127.0.0.1:9/babel-service";
        let error = execute_http_upstream(upstream_url, json!({ "operation": "health" }))
            .await
            .expect_err("closed upstream should fail");

        let message = error.to_string();
        assert!(message.contains(upstream_url));
        assert!(message.contains("reachable from the relay host"));
    }

    async fn test_state(
        store_path: PathBuf,
        now: Arc<AtomicI64>,
        executor: EnvelopeExecutor,
        task_cents: i64,
    ) -> RelayState {
        RelayState::new(RelayConfig {
            store_path,
            owner_token: Some("owner-token".to_string()),
            babel_service_command: None,
            upstream_url: None,
            task_cents,
            now_millis: Arc::new(move || now.load(Ordering::SeqCst)),
            executor: Some(executor),
        })
        .await
        .expect("relay state")
    }

    async fn spawn_server(state: RelayState) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local addr");
        let handle = tokio::spawn(async move {
            axum::serve(listener, router(state)).await.expect("serve");
        });
        (format!("http://{address}"), handle)
    }
}
