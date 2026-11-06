//! Worker dashboard: project list with queue ids and daily task progress.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::response::trpc_data;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dashboard {
    /// Babel worker id (needed for feedback drafts).
    pub worker_id: Option<String>,
    /// Active projects with at least one queue id, sorted by name then id.
    pub projects: Vec<DashboardProject>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashboardProject {
    pub id: String,
    pub name: String,
    pub chunked_queue_id: String,
    pub stitched_queue_id: String,
    /// Tasks the signed-in worker completed today on this project.
    pub tasks_completed_today: u64,
    /// Babel's daily cap for this project; `None` when Babel enforces none.
    pub daily_task_limit: Option<u64>,
    /// Babel project category (`error-categorization` for error-label projects).
    pub category: Option<String>,
}

impl DashboardProject {
    /// Error-label project: tasks open the error questionnaire and the lookup claims QA
    /// tasks by diff id.
    pub fn is_error_categorization(&self) -> bool {
        self.category.as_deref() == Some(super::error_task::ERROR_CATEGORIZATION)
    }
}

/// Totals over projects that report a daily limit:
/// `(completed today, summed limit, number of limited projects)`.
pub fn daily_task_progress(projects: &[DashboardProject]) -> (u64, u64, usize) {
    projects
        .iter()
        .filter_map(|project| project.daily_task_limit.map(|limit| (project, limit)))
        .fold(
            (0, 0, 0),
            |(completed, limit_total, count), (project, limit)| {
                (
                    completed + project.tasks_completed_today,
                    limit_total + limit,
                    count + 1,
                )
            },
        )
}

/// Parses a normalized dashboard payload (`{"worker", "projects", "projectConfigs"}`
/// or any nesting of project objects).
pub fn parse_dashboard(source: &str) -> Result<Dashboard, String> {
    let value: Value = serde_json::from_str(source)
        .map_err(|error| format!("failed to decode Babel projects response: {error}"))?;
    let queue_configs = queue_configs_by_project_id(&value);
    let mut projects = Vec::new();
    collect_projects(&value, &queue_configs, &mut projects);
    projects.sort_by(|left, right| left.name.cmp(&right.name).then(left.id.cmp(&right.id)));
    projects.dedup_by(|left, right| left.id == right.id);
    Ok(Dashboard {
        worker_id: worker_id(&value),
        projects,
    })
}

fn worker_id(value: &Value) -> Option<String> {
    value
        .get("worker")
        .and_then(|worker| worker.get("id"))
        .or_else(|| value.get("workerId"))
        .or_else(|| value.get("worker_id"))
        .or_else(|| {
            let worker = value.as_array()?.first()?;
            worker
                .pointer("/result/data/json/id")
                .or_else(|| worker.pointer("/data/json/id"))
                .or_else(|| worker.pointer("/result/data/id"))
                .or_else(|| worker.pointer("/data/id"))
                .or_else(|| worker.get("id"))
        })
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|worker_id| !worker_id.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, Default)]
struct QueueConfig {
    chunked_queue_id: String,
    stitched_queue_id: String,
}

const CHUNKED_KEYS: &[&str] = &[
    "chunkedQueueId",
    "chunked_queue_id",
    "chunkedReviewQueueId",
    "reviewQueueId",
    "queueId",
];
const STITCHED_KEYS: &[&str] = &[
    "stitchedTxQueueId",
    "stitched_tx_queue_id",
    "stitchedQueueId",
    "stitched_queue_id",
];

fn collect_projects(
    value: &Value,
    queue_configs: &BTreeMap<String, QueueConfig>,
    projects: &mut Vec<DashboardProject>,
) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_projects(item, queue_configs, projects);
            }
        }
        Value::Object(map) => {
            if let Some(project) = project_from_object(value, queue_configs) {
                projects.push(project);
            }
            for child in map.values() {
                collect_projects(child, queue_configs, projects);
            }
        }
        _ => {}
    }
}

fn project_from_object(
    value: &Value,
    queue_configs: &BTreeMap<String, QueueConfig>,
) -> Option<DashboardProject> {
    if is_queue_config_only(value) || !is_active(value) {
        return None;
    }
    let id = direct_string(value, &["id", "projectId", "project_id", "slug", "name"])?;
    let name = direct_string(
        value,
        &["displayName", "display_name", "title", "label", "name"],
    )
    .unwrap_or_else(|| id.clone());
    let discovered = queue_configs.get(&id);
    let chunked_queue_id = direct_string(value, CHUNKED_KEYS)
        .or_else(|| discovered.map(|config| config.chunked_queue_id.clone()))
        .unwrap_or_default();
    let stitched_queue_id = direct_string(value, STITCHED_KEYS)
        .or_else(|| discovered.map(|config| config.stitched_queue_id.clone()))
        .unwrap_or_default();
    if chunked_queue_id.is_empty() && stitched_queue_id.is_empty() {
        return None;
    }
    Some(DashboardProject {
        id,
        name,
        chunked_queue_id,
        stitched_queue_id,
        tasks_completed_today: nested_u64(value, &["workerStats", "tasksCompletedToday"])
            .unwrap_or(0),
        daily_task_limit: nested_u64(value, &["taskLimit", "limit"]),
        category: direct_string(value, &["category"]),
    })
}

fn nested_u64(value: &Value, path: &[&str]) -> Option<u64> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))?
        .as_u64()
}

fn is_active(value: &Value) -> bool {
    [
        &["status"][..],
        &["permissionStatus", "permission_status"][..],
    ]
    .iter()
    .all(|keys| {
        direct_string(value, keys).is_none_or(|status| status.eq_ignore_ascii_case("active"))
    })
}

fn queue_configs_by_project_id(value: &Value) -> BTreeMap<String, QueueConfig> {
    fn collect(value: &Value, configs: &mut BTreeMap<String, QueueConfig>) {
        match value {
            Value::Array(items) => items.iter().for_each(|item| collect(item, configs)),
            Value::Object(map) => {
                if let Some((project_id, config)) = queue_config_from_object(value) {
                    configs.insert(project_id, config);
                }
                map.values().for_each(|child| collect(child, configs));
            }
            _ => {}
        }
    }
    let mut configs = BTreeMap::new();
    collect(value, &mut configs);
    configs
}

fn queue_config_from_object(value: &Value) -> Option<(String, QueueConfig)> {
    let project_id = direct_string(value, &["projectId", "project_id", "id"])?;
    let chunked_queue_id = direct_string(value, CHUNKED_KEYS).unwrap_or_default();
    let stitched_queue_id = direct_string(value, STITCHED_KEYS).unwrap_or_default();
    if chunked_queue_id.is_empty() && stitched_queue_id.is_empty() {
        return None;
    }
    Some((
        project_id,
        QueueConfig {
            chunked_queue_id,
            stitched_queue_id,
        },
    ))
}

fn is_queue_config_only(value: &Value) -> bool {
    queue_config_from_object(value).is_some()
        && direct_string(
            value,
            &[
                "displayName",
                "display_name",
                "title",
                "label",
                "name",
                "slug",
                "status",
                "permissionStatus",
                "permission_status",
            ],
        )
        .is_none()
}

fn direct_string(value: &Value, keys: &[&str]) -> Option<String> {
    let map = value.as_object()?;
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|found| !found.is_empty())
            .map(str::to_string)
    })
}

/// Live/command dashboard batch (`worker.get`, `worker.getProjects`) →
/// `{"worker": ..., "projects": [...]}`.
pub(super) fn normalize_dashboard_payload(response: &str) -> String {
    let trimmed = response.trim_start_matches('\u{feff}').trim();
    let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
        return response.trim().to_string();
    };
    if let Value::Array(items) = &value
        && let [worker, projects] = items.as_slice()
        && let Some(projects) = trpc_data(projects).filter(Value::is_array)
    {
        let worker = trpc_data(worker).filter(Value::is_object);
        return json!({ "worker": worker, "projects": projects }).to_string();
    }
    let normalized = trpc_data(&value).unwrap_or(value);
    if normalized.is_array() {
        json!({ "projects": normalized }).to_string()
    } else {
        normalized.to_string()
    }
}

/// Babel's project page resolves a project's review queues with
/// `transcriptions.getProjectQueues({ projectId })`. One batched query covers the whole
/// dashboard; projects that are not transcription projects answer `NOT_FOUND` in-band.
pub(super) const PROJECT_QUEUES_OPERATION: &str = "transcriptions.getProjectQueues";

/// Project ids of a normalized dashboard payload, in payload order.
pub(super) fn dashboard_project_ids(normalized_payload: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(normalized_payload) else {
        return Vec::new();
    };
    value
        .get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|project| direct_string(project, &["projectId", "project_id", "id"]))
        .collect()
}

/// Batched `getProjectQueues` envelope for `project_ids` (`None` when there are none).
pub(super) fn project_queues_envelope(project_ids: &[String]) -> Option<Value> {
    if project_ids.is_empty() {
        return None;
    }
    let operation = vec![PROJECT_QUEUES_OPERATION; project_ids.len()].join(",");
    let batch: serde_json::Map<String, Value> = project_ids
        .iter()
        .enumerate()
        .map(|(ix, id)| (ix.to_string(), json!({ "json": { "projectId": id } })))
        .collect();
    Some(json!({ "operation": operation, "batchInput": batch }))
}

/// `{projectId, chunkedQueueId, stitchedTxQueueId}` configs from a batched
/// `getProjectQueues` response (items answer `project_ids` in order; errors are skipped).
pub(super) fn project_configs_from_queues(project_ids: &[String], response: &str) -> Vec<Value> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(response.trim()) else {
        return Vec::new();
    };
    project_ids
        .iter()
        .zip(items.iter())
        .filter_map(|(project_id, item)| {
            let data = trpc_data(item)?;
            let chunked = direct_string(&data, &["chunkedQueueId"]).unwrap_or_default();
            let stitched = direct_string(&data, &["stitchedTxQueueId"]).unwrap_or_default();
            (!chunked.is_empty() || !stitched.is_empty()).then(|| {
                json!({
                    "projectId": project_id,
                    "chunkedQueueId": chunked,
                    "stitchedTxQueueId": stitched,
                })
            })
        })
        .collect()
}

/// Adds discovered configs as `projectConfigs` to a normalized dashboard payload.
pub(super) fn attach_project_configs(payload: &str, configs: Vec<Value>) -> String {
    if configs.is_empty() {
        return payload.to_string();
    }
    let Ok(mut value) = serde_json::from_str::<Value>(payload) else {
        return payload.to_string();
    };
    match &mut value {
        Value::Object(map) => {
            map.insert("projectConfigs".to_string(), Value::Array(configs));
            value.to_string()
        }
        Value::Array(_) => json!({ "projects": value, "projectConfigs": configs }).to_string(),
        _ => payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_batch_normalization_retains_worker_and_projects() {
        let normalized = normalize_dashboard_payload(
            r#"[
                {"result":{"data":{"json":{"id":"worker-1"}}}},
                {"result":{"data":{"json":[{"projectId":"project-1","name":"Project One"}]}}}
            ]"#,
        );
        assert_eq!(
            serde_json::from_str::<Value>(&normalized).expect("normalized"),
            json!({
                "worker": { "id": "worker-1" },
                "projects": [{ "projectId": "project-1", "name": "Project One" }],
            })
        );
    }

    #[test]
    fn project_queues_batch_maps_answers_to_projects_and_skips_errors() {
        let ids = vec!["p-1".to_string(), "p-2".to_string(), "p-3".to_string()];
        let envelope = project_queues_envelope(&ids).expect("envelope");
        assert_eq!(
            envelope["operation"],
            "transcriptions.getProjectQueues,transcriptions.getProjectQueues,transcriptions.getProjectQueues"
        );
        assert_eq!(
            envelope["batchInput"]["2"],
            json!({ "json": { "projectId": "p-3" } })
        );

        let configs = project_configs_from_queues(
            &ids,
            r#"[
                {"result":{"data":{"json":{"chunkedQueueId":"q-1","stitchedTxQueueId":"s-1"}}}},
                {"error":{"json":{"message":"Project not found","data":{"code":"NOT_FOUND"}}}},
                {"result":{"data":{"json":{"chunkedQueueId":"q-3","stitchedTxQueueId":null}}}}
            ]"#,
        );
        assert_eq!(
            configs,
            vec![
                json!({"projectId":"p-1","chunkedQueueId":"q-1","stitchedTxQueueId":"s-1"}),
                json!({"projectId":"p-3","chunkedQueueId":"q-3","stitchedTxQueueId":""}),
            ]
        );

        let payload = attach_project_configs(
            r#"{"projects":[{"projectId":"p-1","name":"One"},{"projectId":"p-2","name":"Two"}]}"#,
            configs,
        );
        let dashboard = parse_dashboard(&payload).expect("dashboard");
        assert_eq!(dashboard.projects.len(), 1);
        assert_eq!(dashboard.projects[0].chunked_queue_id, "q-1");
        assert_eq!(dashboard.projects[0].stitched_queue_id, "s-1");
    }
}
