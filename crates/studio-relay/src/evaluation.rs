use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EvaluationBinding {
    caller: String,
    project_id: String,
    task_ids: BTreeSet<String>,
}

fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or_default()
}

fn authorize_binding(
    binding: &EvaluationBinding,
    caller: &str,
    payload: &Value,
    op: &str,
) -> Result<(), RelayError> {
    if binding.caller != caller {
        return Err(RelayError::new(
            StatusCode::CONFLICT,
            "This evaluation session belongs to another client.",
        ));
    }
    if matches!(op, "evalTasks.saveProgress" | "evalTasks.submitSession") {
        let tasks = payload
            .get("tasks")
            .and_then(Value::as_array)
            .ok_or_else(|| RelayError::bad_request("Evaluation tasks are required."))?;
        let mut seen = BTreeSet::new();
        for task in tasks {
            let id = field(task, "taskId");
            if !binding.task_ids.contains(id) || !seen.insert(id) {
                return Err(RelayError::forbidden(
                    "Unknown or duplicate evaluation task.",
                ));
            }
        }
        if op == "evalTasks.submitSession" && seen.len() != binding.task_ids.len() {
            return Err(RelayError::bad_request(
                "Submit requires every task in the session.",
            ));
        }
    }
    Ok(())
}

pub(super) async fn handle(
    State(state): State<Arc<RelayState>>,
    headers: HeaderMap,
    Json(envelope): Json<Value>,
) -> Result<Json<Value>, RelayError> {
    let op = field(&envelope, "operation");
    if !matches!(
        op,
        "evalTasks.serveSessionTasks"
            | "evalTasks.saveProgress"
            | "evalTasks.submitSession"
            | "evalTasks.holdSession"
    ) {
        return Err(RelayError::forbidden("Unsupported evaluation operation."));
    }
    if envelope.get("batchInput").is_some() {
        return Err(RelayError::bad_request(
            "Evaluation batches are not supported.",
        ));
    }
    let payload = envelope
        .get("payload")
        .ok_or_else(|| RelayError::bad_request("Missing evaluation payload."))?;
    let _guard = state.evaluation_lock.lock().await;
    let caller;
    let project = field(payload, "projectId");
    let session = field(payload, "sessionId");
    {
        let store = state.store.lock().await;
        // An explicitly configured owner secret is required for owner privilege.
        if state.config.owner_token.is_some() && require_owner(&headers, &state.config).is_ok() {
            caller = "owner".to_string();
        } else {
            let (account, _) = authenticate_subaccount(&headers, &store)?;
            caller = format!("sub:{}", account.id);
            let assigned_project = if op == "evalTasks.serveSessionTasks" {
                project
            } else {
                store
                    .evaluation_sessions
                    .get(session)
                    .map(|b| b.project_id.as_str())
                    .unwrap_or_default()
            };
            if !account.allowed_project_ids.is_empty()
                && !account
                    .allowed_project_ids
                    .iter()
                    .any(|id| id == assigned_project)
            {
                return Err(RelayError::forbidden(
                    "Sub-account is not assigned to this project.",
                ));
            }
        }
        if op == "evalTasks.serveSessionTasks" {
            if project.is_empty() {
                return Err(RelayError::bad_request("Project ID is required."));
            }
        } else {
            let binding = store.evaluation_sessions.get(session).ok_or_else(|| {
                RelayError::new(
                    StatusCode::CONFLICT,
                    "Load the evaluation session before continuing.",
                )
            })?;
            authorize_binding(binding, &caller, payload, op)?;
        }
    }
    let response = state.execute_envelope(envelope.clone()).await?;
    let parsed: Value = serde_json::from_str(&response)
        .map_err(|_| RelayError::bad_gateway("Invalid evaluation upstream response."))?;
    let item = parsed.as_array().and_then(|v| v.first()).unwrap_or(&parsed);
    // Return upstream errors without changing ownership.
    if let Some(data) = item.pointer("/result/data/json") {
        let mut store = state.store.lock().await;
        if op == "evalTasks.serveSessionTasks" && field(data, "kind") == "session" {
            let id = field(data, "sessionId");
            let tasks = data
                .get("tasks")
                .and_then(Value::as_array)
                .ok_or_else(|| RelayError::bad_gateway("Evaluation session has no tasks."))?;
            let task_ids: BTreeSet<String> = tasks
                .iter()
                .map(|t| field(t, "taskId").to_owned())
                .collect();
            if id.is_empty()
                || task_ids.is_empty()
                || task_ids.contains("")
                || task_ids.len() != tasks.len()
            {
                return Err(RelayError::bad_gateway(
                    "Invalid evaluation session identifiers.",
                ));
            }
            if let Some(existing) = store.evaluation_sessions.get(id) {
                if existing.caller != caller || existing.project_id != project {
                    return Err(RelayError::new(
                        StatusCode::CONFLICT,
                        "Babel resumed a session already being worked on by another client. Finish that session first.",
                    ));
                }
            }
            store.evaluation_sessions.insert(
                id.to_owned(),
                EvaluationBinding {
                    caller,
                    project_id: project.to_owned(),
                    task_ids,
                },
            );
            state.save(&mut store).await?;
        } else if op == "evalTasks.submitSession" && field(data, "kind") == "completed" {
            store.evaluation_sessions.remove(session);
            state.save(&mut store).await?;
        }
    }
    Ok(Json(json!({"payload": response})))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding() -> EvaluationBinding {
        EvaluationBinding {
            caller: "sub:one".into(),
            project_id: "project".into(),
            task_ids: BTreeSet::from(["a".into(), "b".into()]),
        }
    }
    #[test]
    fn rejects_other_clients_and_unknown_or_duplicate_tasks() {
        let b = binding();
        assert!(authorize_binding(&b, "owner", &json!({}), "evalTasks.holdSession").is_err());
        for tasks in [
            json!([{"taskId":"unknown"}]),
            json!([{"taskId":"a"},{"taskId":"a"}]),
        ] {
            assert!(
                authorize_binding(
                    &b,
                    "sub:one",
                    &json!({"tasks":tasks}),
                    "evalTasks.saveProgress"
                )
                .is_err()
            );
        }
    }
    #[test]
    fn progress_can_be_partial_but_submit_must_cover_session() {
        let b = binding();
        let partial = json!({"tasks":[{"taskId":"a"}]});
        assert!(authorize_binding(&b, "sub:one", &partial, "evalTasks.saveProgress").is_ok());
        assert!(authorize_binding(&b, "sub:one", &partial, "evalTasks.submitSession").is_err());
        assert!(
            authorize_binding(
                &b,
                "sub:one",
                &json!({"tasks":[{"taskId":"a"},{"taskId":"b"}]}),
                "evalTasks.submitSession"
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn session_lifecycle_preserves_owner_and_checks_assignment_before_upstream() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let state = Arc::new(
            RelayState::new(RelayConfig {
                store_path: temp.path().join("store.json"),
                owner_token: Some("test-owner".into()),
                babel_service_command: None,
                upstream_url: None,
                task_cents: 0,
                now_millis: Arc::new(|| 100),
                executor: Some(Arc::new(move |request| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async move {
                        let data = match field(&request, "operation") {
                            "evalTasks.serveSessionTasks" => {
                                json!({"kind":"session", "sessionId":"s", "tasks":[{"taskId":"a"}]})
                            }
                            "evalTasks.submitSession" => json!({"kind":"completed"}),
                            _ => json!({"kind":"saved"}),
                        };
                        Ok(json!([{"result":{"data":{"json":data}}}]).to_string())
                    })
                })),
            })
            .await
            .unwrap(),
        );
        {
            let mut store = state.store.lock().await;
            store.subaccounts.push(SubaccountRecord {
                id: "one".into(),
                label: "test".into(),
                code: "test-code".into(),
                revoked: false,
                allowed_project_ids: vec!["project".into()],
                analytics: AnalyticsRecord::default(),
                active_claims: BTreeMap::new(),
            });
            store.sessions.push(SessionRecord {
                token: "test-sub".into(),
                subaccount_id: "one".into(),
                created_at: 100,
                last_seen_at: 100,
            });
        }
        let mut owner = HeaderMap::new();
        owner.insert("authorization", "Bearer test-owner".parse().unwrap());
        let mut sub = HeaderMap::new();
        sub.insert("authorization", "Bearer test-sub".parse().unwrap());
        let serve =
            json!({"operation":"evalTasks.serveSessionTasks","payload":{"projectId":"project"}});
        let forbidden =
            json!({"operation":"evalTasks.serveSessionTasks","payload":{"projectId":"other"}});
        assert!(
            handle(State(state.clone()), sub.clone(), Json(forbidden))
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        handle(State(state.clone()), owner.clone(), Json(serve.clone()))
            .await
            .unwrap();
        let error = handle(State(state.clone()), sub.clone(), Json(serve))
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(
            state.store.lock().await.evaluation_sessions["s"].caller,
            "owner"
        );
        let submit = json!({"operation":"evalTasks.submitSession","payload":{"sessionId":"s","tasks":[{"taskId":"a"}]}});
        assert!(
            handle(State(state.clone()), sub, Json(submit.clone()))
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        handle(State(state.clone()), owner, Json(submit))
            .await
            .unwrap();
        assert!(state.store.lock().await.evaluation_sessions.is_empty());
        let persisted = RelayStore::load(&state.config.store_path).await.unwrap();
        assert!(persisted.evaluation_sessions.is_empty());
        assert_eq!(persisted.subaccounts.len(), 1);
    }

    #[tokio::test]
    async fn resources_enforce_project_access_and_read_only_operations() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let temp = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let state = Arc::new(
            RelayState::new(RelayConfig {
                store_path: temp.path().join("store.json"),
                owner_token: Some("owner".into()),
                babel_service_command: None,
                upstream_url: None,
                task_cents: 0,
                now_millis: Arc::new(|| 100),
                executor: Some(Arc::new(move |_| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    Box::pin(async { Ok("[]".into()) })
                })),
            })
            .await
            .unwrap(),
        );
        {
            let mut store = state.store.lock().await;
            store.subaccounts.push(SubaccountRecord {
                id: "one".into(),
                label: "test".into(),
                code: "test".into(),
                revoked: false,
                allowed_project_ids: vec!["allowed".into()],
                analytics: AnalyticsRecord::default(),
                active_claims: BTreeMap::new(),
            });
            store.sessions.push(SessionRecord {
                token: "session".into(),
                subaccount_id: "one".into(),
                created_at: 100,
                last_seen_at: 100,
            });
        }
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer session".parse().unwrap());
        let request =
            json!({"operation":"project.getProjectInstructions","payload":{"projectId":"allowed"}});
        assert!(
            super::super::project_resources(
                State(state.clone()),
                HeaderMap::new(),
                Json(request.clone())
            )
            .await
            .is_err()
        );
        for invalid in [
            json!({"operation":"project.getProjectInstructions","payload":{"projectId":"other"}}),
            json!({"operation":"project.acknowledgeProjectInstructions","payload":{"projectId":"allowed"}}),
            json!({"operation":"project.getProjectInstructions","payload":{"projectId":"allowed"},"batchInput":{}}),
        ] {
            assert!(
                super::super::project_resources(
                    State(state.clone()),
                    headers.clone(),
                    Json(invalid)
                )
                .await
                .is_err()
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        super::super::project_resources(
            State(state.clone()),
            headers.clone(),
            Json(request.clone()),
        )
        .await
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            super::super::babel_service(
                State(state),
                headers,
                Json(BabelRelayRequest {
                    request_kind: "resource".into(),
                    envelope: request
                })
            )
            .await
            .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
