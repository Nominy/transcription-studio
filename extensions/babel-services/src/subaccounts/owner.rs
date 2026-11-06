//! Owner-side sub-account management on the relay host, authenticated with the owner's
//! main Babel account (its `auth.json`; dashboard "Sub-accounts" section).

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::{api_url, json_headers, normalize_relay_host_url, string_by_keys, success_body};

const OWNER_DASHBOARD_PATH: &str = "/api/subaccounts/owner/dashboard";
const OWNER_SUBACCOUNTS_PATH: &str = "/api/subaccounts/owner/subaccounts";
const OWNER_PROJECT_QUEUES_PATH: &str = "/api/subaccounts/owner/project-queues";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnerDashboard {
    /// Active (non-revoked) sub-accounts.
    pub subaccounts: Vec<ManagedSubaccount>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedSubaccount {
    pub id: String,
    pub label: String,
    /// Login code to hand to the worker.
    pub code: Option<String>,
    pub revoked: bool,
    pub allowed_project_ids: Vec<String>,
    pub completed_tasks: u64,
    pub worked_seconds: u64,
    pub earnings_cents: i64,
}

/// Queue ids of one dashboard project, pushed so the relay can claim on behalf of sub-accounts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectQueueConfig {
    pub project_id: String,
    pub chunked_queue_id: String,
    pub stitched_queue_id: String,
}

pub async fn load_dashboard(auth_store: &Path, host_url: &str) -> Result<OwnerDashboard> {
    let (host_url, headers) = owner_target(auth_store, host_url).await?;
    let response = studio_services::http::client()
        .get(api_url(&host_url, OWNER_DASHBOARD_PATH))
        .headers(headers)
        .send()
        .await;
    dashboard_response(response).await
}

/// Creates a sub-account with `label`; returns the refreshed dashboard.
pub async fn create_subaccount(
    auth_store: &Path,
    host_url: &str,
    label: &str,
) -> Result<OwnerDashboard> {
    let (host_url, headers) = owner_target(auth_store, host_url).await?;
    let response = studio_services::http::client()
        .post(api_url(&host_url, OWNER_SUBACCOUNTS_PATH))
        .headers(headers)
        .json(&json!({ "label": label.trim() }))
        .send()
        .await;
    dashboard_response(response).await
}

pub async fn revoke_subaccount(
    auth_store: &Path,
    host_url: &str,
    subaccount_id: &str,
) -> Result<OwnerDashboard> {
    let (host_url, headers) = owner_target(auth_store, host_url).await?;
    let response = studio_services::http::client()
        .delete(api_url(
            &host_url,
            &format!(
                "{OWNER_SUBACCOUNTS_PATH}/{}",
                encode_path_segment(subaccount_id)
            ),
        ))
        .headers(headers)
        .send()
        .await;
    dashboard_response(response).await
}

/// Replaces the projects a sub-account may work on.
pub async fn assign_projects(
    auth_store: &Path,
    host_url: &str,
    subaccount_id: &str,
    project_ids: &[String],
) -> Result<OwnerDashboard> {
    let (host_url, headers) = owner_target(auth_store, host_url).await?;
    let response = studio_services::http::client()
        .put(api_url(
            &host_url,
            &format!(
                "{OWNER_SUBACCOUNTS_PATH}/{}/projects",
                encode_path_segment(subaccount_id)
            ),
        ))
        .headers(headers)
        .json(&json!({ "projectIds": project_ids }))
        .send()
        .await;
    dashboard_response(response).await
}

/// Pushes dashboard project queue ids to the relay (`PUT .../owner/project-queues`).
pub async fn push_project_queue_configs(
    auth_store: &Path,
    host_url: &str,
    configs: &[ProjectQueueConfig],
) -> Result<()> {
    let (host_url, headers) = owner_target(auth_store, host_url).await?;
    let response = studio_services::http::client()
        .put(api_url(&host_url, OWNER_PROJECT_QUEUES_PATH))
        .headers(headers)
        .json(&json!({
            "configs": configs
                .iter()
                .map(|config| json!({
                    "projectId": config.project_id,
                    "chunkedQueueId": config.chunked_queue_id,
                    "stitchedTxQueueId": config.stitched_queue_id,
                }))
                .collect::<Vec<_>>()
        }))
        .send()
        .await
        .context("Sub-account owner request failed")?;
    success_body(response, "Sub-account owner").await.map(drop)
}

/// Owners (no active sub-account session) with a valid relay URL push queue configs whenever
/// dashboard projects load or the relay URL is saved. Returns the normalized host to push to,
/// or `None` when no push is due.
pub fn project_queue_sync_host(
    subaccount_session_active: bool,
    host_url: &str,
    configs: &[ProjectQueueConfig],
) -> Option<String> {
    if subaccount_session_active || configs.is_empty() {
        return None;
    }
    normalize_relay_host_url(host_url).ok()
}

/// Accepts `{subaccounts|subAccounts|items}` (optionally under `data`); drops revoked rows.
pub fn parse_dashboard(body: &str) -> Result<OwnerDashboard> {
    let value = serde_json::from_str::<Value>(body)
        .context("failed to decode Sub-account owner response")?;
    let source = value.get("data").unwrap_or(&value);
    let Some(items) = source
        .get("subaccounts")
        .or_else(|| source.get("subAccounts"))
        .or_else(|| source.get("items"))
        .and_then(Value::as_array)
    else {
        return Ok(OwnerDashboard::default());
    };
    Ok(OwnerDashboard {
        subaccounts: items
            .iter()
            .filter_map(parse_managed_subaccount)
            .filter(|subaccount| !subaccount.revoked)
            .collect(),
    })
}

fn parse_managed_subaccount(value: &Value) -> Option<ManagedSubaccount> {
    let id = string_by_keys(
        value,
        &["id", "subaccountId", "subAccountId", "subaccount_id"],
    )?;
    let analytics = value.get("analytics").unwrap_or(value);
    Some(ManagedSubaccount {
        label: string_by_keys(value, &["label", "name", "displayName", "display_name"])
            .unwrap_or_else(|| id.clone()),
        code: string_by_keys(value, &["code", "specialCode", "inviteCode", "loginCode"]),
        revoked: ["revoked", "isRevoked", "disabled"]
            .iter()
            .find_map(|key| value.get(*key).and_then(Value::as_bool))
            .unwrap_or(false),
        allowed_project_ids: [
            "allowedProjectIds",
            "projectIds",
            "assignedProjectIds",
            "allowed_projects",
        ]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_array))
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default(),
        completed_tasks: u64_by_keys(
            analytics,
            &["completedTasks", "taskCount", "tasksCompleted", "tasks"],
        ),
        worked_seconds: u64_by_keys(
            analytics,
            &[
                "workedSeconds",
                "timeSeconds",
                "workSeconds",
                "durationSeconds",
            ],
        ),
        earnings_cents: ["earningsCents", "moneyCents", "earnedCents", "payoutCents"]
            .iter()
            .find_map(|key| analytics.get(*key).and_then(Value::as_i64))
            .unwrap_or(0),
        id,
    })
}

async fn owner_target(
    auth_store: &Path,
    host_url: &str,
) -> Result<(String, reqwest::header::HeaderMap)> {
    let host_url = normalize_relay_host_url(host_url)?;
    let authorization = crate::auth::mint_babel_authorization(
        auth_store,
        &crate::auth::babel_base_url(),
        &crate::auth::clerk_url(),
        OffsetDateTime::now_utc().unix_timestamp(),
    )
    .await
    .context("Main Babel account auth is required for Sub-account controls")?;
    Ok((host_url, json_headers(&authorization)?))
}

async fn dashboard_response(
    response: reqwest::Result<reqwest::Response>,
) -> Result<OwnerDashboard> {
    let response = response.context("Sub-account owner request failed")?;
    parse_dashboard(&success_body(response, "Sub-account owner").await?)
}

fn u64_by_keys(value: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| {
            let value = value.get(*key)?;
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
        })
        .unwrap_or(0)
}

fn encode_path_segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}
