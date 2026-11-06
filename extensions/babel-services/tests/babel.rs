mod common;

use std::collections::BTreeSet;

use common::{MockResponse, MockServer, contains};
use exaudio_babel_domain::editor_model::feedback_assessment::{
    FeedbackDraftSaveRequest, FeedbackInputResponse,
};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use exaudio_babel_services::babel::{
    self, BabelClient, BabelConfig, BabelRequest, Fixture, LoadedTask, OpenedTask, RouteContext,
    SaveAnnotationsRequest, SubmitReviewActionRequest, TaskRequest,
};
use serde_json::{Value, json};
use studio_editor::workflow::StudioWorkflow;

fn transcript(opened: OpenedTask) -> LoadedTask {
    match opened {
        OpenedTask::Transcript(task) => *task,
        OpenedTask::ErrorTask(task) => panic!("expected a transcript, got {task:?}"),
    }
}

const REVIEW_ACTION_FIXTURE: &str =
    include_str!("../../../crates/studio-babel/tests/fixtures/review-action-final.json");

fn feedback_save_request() -> FeedbackDraftSaveRequest {
    FeedbackDraftSaveRequest {
        form_step_response_id: "form-step-response-1".to_string(),
        input_responses: vec![FeedbackInputResponse {
            form_input_id: "comment".to_string(),
            value: "Looks good".to_string(),
        }],
    }
}

fn offline_config() -> BabelConfig {
    BabelConfig {
        ..BabelConfig::live("http://127.0.0.1:9")
    }
}

#[test]
fn outbound_annotations_carry_exact_asserted_warning_reasons() {
    let workflow = StudioWorkflow::from_babel_json(REVIEW_ACTION_FIXTURE).expect("fixture");
    let segment = &workflow.project().segments[0];
    let annotation_id = segment
        .source_annotation_id
        .clone()
        .unwrap_or_else(|| segment.id.clone());
    let find = |annotations: &[Value]| {
        annotations
            .iter()
            .find(|annotation| annotation["id"] == annotation_id.as_str())
            .cloned()
            .expect("matching annotation")
    };
    let baseline = SaveAnnotationsRequest::from_workflow(&workflow, &BTreeSet::new())
        .expect("baseline request");
    let exact_reason = "Keep  internal  spacing & punctuation!";
    let asserted = BTreeSet::from([format!("{}::{exact_reason}", segment.id)]);

    let save = SaveAnnotationsRequest::from_workflow(&workflow, &asserted).expect("save");
    let submit = SubmitReviewActionRequest::from_workflow(&workflow, &asserted).expect("submit");

    assert_eq!(
        save.review_action_id,
        "e823c3bb-2697-55fc-9272-c083a55874da"
    );
    assert_eq!(submit.review_action_id, save.review_action_id);
    assert!(!submit.is_stitched_queue);
    for annotations in [&save.annotations, &submit.annotations] {
        assert_eq!(
            find(annotations)["metadata"]["assertedWarnings"],
            json!([exact_reason])
        );
    }
    if let Value::Object(baseline_metadata) = find(&baseline.annotations)["metadata"].clone() {
        let saved = find(&save.annotations);
        for (key, value) in baseline_metadata {
            if key != "assertedWarnings" {
                assert_eq!(saved["metadata"][&key], value, "{key}");
            }
        }
    }

    let mut persisted = workflow.project().clone();
    persisted.segments[0]
        .metadata
        .insert("assertedWarnings".to_string(), json!([exact_reason]));
    let cleared = SaveAnnotationsRequest::from_workflow(
        &StudioWorkflow::from_project(persisted),
        &BTreeSet::new(),
    )
    .expect("cleared");
    assert_eq!(
        find(&cleared.annotations)["metadata"]["assertedWarnings"],
        json!([])
    );
}

#[test]
fn project_metadata_lookups_follow_fallback_order() {
    let mut project = StudioWorkflow::from_babel_json(REVIEW_ACTION_FIXTURE)
        .expect("fixture")
        .project()
        .clone();
    assert_eq!(
        babel::review_action_id(&project),
        "e823c3bb-2697-55fc-9272-c083a55874da",
        "falls back to segment metadata"
    );
    project.metadata.insert(
        babel::META_CHUNKED_QUEUE_ID.to_string(),
        "chunked".to_string(),
    );
    assert_eq!(babel::active_queue_id(&project), "chunked");
    project.metadata.insert(
        babel::META_ACTIVE_QUEUE_ID.to_string(),
        " active ".to_string(),
    );
    assert_eq!(babel::active_queue_id(&project), "active");
    assert_eq!(babel::stitched_queue_id(&project), "active");
    project.metadata.insert(
        babel::META_STITCHED_QUEUE_ID.to_string(),
        "stitched".to_string(),
    );
    assert_eq!(babel::stitched_queue_id(&project), "stitched");
    project
        .metadata
        .insert(babel::META_IS_STITCHED_QUEUE.to_string(), "YES".to_string());
    assert!(babel::is_stitched_queue(&project));
    project
        .metadata
        .insert(babel::META_PROJECT_ID.to_string(), "project-9".to_string());
    assert_eq!(babel::feedback_project_id(&project), "project-9");
}

#[test]
fn route_context_overlays_current_project_route_metadata() {
    let mut project = StudioWorkflow::from_babel_json(REVIEW_ACTION_FIXTURE)
        .expect("fixture")
        .project()
        .clone();
    project
        .metadata
        .insert(babel::META_DISPLAY_FEEDBACK.to_string(), "true".to_string());
    project
        .metadata
        .insert(babel::META_CHUNKED_QUEUE_ID.to_string(), "old".to_string());
    let context = RouteContext {
        project_id: "project-1".to_string(),
        chunked_queue_id: "chunked-1".to_string(),
        ..RouteContext::default()
    };
    let merged = context.merged_over(Some(&project));
    let get = |key: &str| {
        merged
            .iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(get(babel::META_DISPLAY_FEEDBACK), Some("true"));
    assert_eq!(get(babel::META_CHUNKED_QUEUE_ID), Some("chunked-1"));
    assert_eq!(get(babel::META_ACTIVE_QUEUE_ID), Some("chunked-1"));
    assert_eq!(get(babel::META_PROJECT_ID), Some("project-1"));
    assert_eq!(get(babel::META_STITCHED_QUEUE_ID), None);
}

#[test]
fn request_envelopes_match_babel_contract() {
    let cases = [
        (
            BabelRequest::LoadFeedbackRubricFlags {
                project_id: "project-1".to_string(),
            },
            json!({"operation": "transcriptionFeedbackForm.getRubricFlagNames", "payload": {"projectId": "project-1"}}),
        ),
        (
            BabelRequest::LoadFeedbackForm {
                project_id: "project-1".to_string(),
            },
            json!({"operation": "transcriptionFeedbackForm.getForm", "payload": {"projectId": "project-1"}}),
        ),
        (
            BabelRequest::LoadFeedbackInputs {
                step_id: "step-1".to_string(),
            },
            json!({"operation": "forms.getFormInputsByStepId", "payload": {"stepId": "step-1"}}),
        ),
        (
            BabelRequest::LoadFeedbackDraft {
                form_step_id: "step-1".to_string(),
                review_action_id: "review-1".to_string(),
                worker_id: "worker-1".to_string(),
            },
            json!({"operation": "transcriptionFeedbackForm.getOrCreateDraft", "payload": {
                "formStepId": "step-1", "reviewActionId": "review-1", "workerId": "worker-1"
            }}),
        ),
        (
            BabelRequest::SaveFeedbackDraft(feedback_save_request()),
            json!({"operation": "transcriptionFeedbackForm.saveDraft", "payload": {
                "formStepResponseId": "form-step-response-1",
                "inputResponses": [{ "formInputId": "comment", "value": "Looks good" }]
            }}),
        ),
        (
            BabelRequest::AdminRateReviewAction {
                review_action_id: "review-1".to_string(),
                rating: 5,
                rating_reason: None,
            },
            json!({"operation": "transcriptions.adminRateTtsTranscription", "payload": {
                "reviewActionId": "review-1", "rating": 5, "ratingReason": null
            }}),
        ),
        (
            BabelRequest::DropWorkerAction {
                queue_id: "queue-1".to_string(),
                review_action_id: "review-1".to_string(),
            },
            json!({"operation": "transcriptions.dropWorkerAction", "payload": {"queueId": "queue-1", "reviewActionId": "review-1"}}),
        ),
        (
            BabelRequest::SkipReviewAction {
                review_action_id: "review-1".to_string(),
            },
            json!({"operation": "transcriptions.skipTranscriptReviewAction", "payload": {"reviewActionId": "review-1"}}),
        ),
        (
            BabelRequest::ClaimByProcessedTranscription {
                processed_transcription_id: "pt-1".to_string(),
                queue_id: "queue-1".to_string(),
            },
            json!({"operation": "transcriptions.claimReviewActionByProcessedTranscriptionId", "payload": {
                "processedTranscriptionId": "pt-1", "queueId": "queue-1"
            }}),
        ),
        (
            BabelRequest::LoadAnnotationsByReviewAction {
                review_action_id: "review-1".to_string(),
            },
            json!({"operation": "transcriptions.getAnnotationsByReviewActionId", "payload": {"reviewActionId": "review-1"}}),
        ),
    ];
    for (request, expected) in cases {
        assert_eq!(request.envelope(), expected);
    }
}

#[tokio::test]
async fn command_bridge_receives_envelope_on_stdin_and_response_is_normalized() {
    let temp = tempfile::tempdir().expect("temp dir");
    let request_log = temp.path().join("request.json");
    let script_path = temp.path().join(if cfg!(windows) {
        "service adapter.cmd"
    } else {
        "service adapter.sh"
    });
    write_adapter_script(&script_path);
    let client = BabelClient::new(BabelConfig {
        command: Some(format!(
            "\"{}\" \"{}\"",
            script_path.display(),
            request_log.display()
        )),
        ..offline_config()
    });

    let last_saved = client
        .save_feedback_draft(&feedback_save_request())
        .await
        .expect("command response");

    assert_eq!(last_saved, "2026-06-23T12:00:00Z");
    let envelope: Value =
        serde_json::from_str(&std::fs::read_to_string(request_log).expect("stdin log"))
            .expect("envelope json");
    assert_eq!(
        envelope,
        BabelRequest::SaveFeedbackDraft(feedback_save_request()).envelope()
    );
    assert!(client.config().relay_session.is_none());
}

#[tokio::test]
async fn live_request_mints_bearer_token_from_native_auth_store() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store_path = temp.path().join("babel-auth.json");
    std::fs::write(
        &store_path,
        r#"{"version":1,"cookies":[{"name":"__client","value":"client-cookie","domain":"127.0.0.1","path":"/","expiresAt":4102444800}]}"#,
    )
    .expect("store");
    let clerk = MockServer::json(vec![
        r#"{"response":{"object":"client","last_active_session_id":"sess_native","sessions":[{"id":"sess_native","status":"active"}]}}"#,
        r#"{"object":"token","jwt":"jwt-from-clerk"}"#,
    ]);
    let babel = MockServer::json(vec![
        r#"[{"result":{"data":{"json":{"success":true,"updatedAt":"2026-06-29T12:00:00Z"}}}}]"#,
    ]);
    let client = BabelClient::new(BabelConfig {
        auth_store: Some(store_path),
        clerk_url: clerk.base_url.clone(),
        ..BabelConfig::live(babel.base_url.clone())
    });

    let last_saved = client
        .save_feedback_draft(&feedback_save_request())
        .await
        .expect("live save");

    assert_eq!(last_saved, "2026-06-29T12:00:00Z");
    let clerk_requests = clerk.join();
    assert!(contains(&clerk_requests[0], "GET /v1/client?"));
    assert!(contains(
        &clerk_requests[0],
        "Cookie: __client=client-cookie"
    ));
    assert!(contains(
        &clerk_requests[1],
        "POST /v1/client/sessions/sess_native/tokens?"
    ));
    let babel_requests = babel.join();
    assert!(contains(
        &babel_requests[0],
        "POST /api/trpc/transcriptionFeedbackForm.saveDraft?batch=1"
    ));
    assert!(contains(
        &babel_requests[0],
        "Authorization: Bearer jwt-from-clerk"
    ));
    assert!(!contains(&babel_requests[0], "Cookie: __session="));
    assert!(contains(&babel_requests[0], "formStepResponseId"));
}

#[tokio::test]
async fn live_dashboard_resolves_project_queues_in_one_batched_query() {
    // Babel's project list carries no queue ids; they come from `getProjectQueues`.
    let babel = MockServer::json(vec![
        r#"[{"result":{"data":{"json":{"id":"worker-1"}}}},{"result":{"data":{"json":[{"projectId":"project-1","name":"Project 1"},{"projectId":"project-2","name":"Not transcription"}]}}}]"#,
        r#"[{"result":{"data":{"json":{"chunkedQueueId":"queue-1","stitchedTxQueueId":"stitched-1"}}}},{"error":{"json":{"message":"Project not found","data":{"code":"NOT_FOUND"}}}}]"#,
    ]);
    let client = BabelClient::new(BabelConfig {
        cookie: Some("__session=session-cookie".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    });

    let dashboard = client.load_dashboard().await.expect("dashboard");

    assert_eq!(dashboard.worker_id.as_deref(), Some("worker-1"));
    assert_eq!(dashboard.projects.len(), 1);
    assert_eq!(dashboard.projects[0].id, "project-1");
    assert_eq!(dashboard.projects[0].chunked_queue_id, "queue-1");
    assert_eq!(dashboard.projects[0].stitched_queue_id, "stitched-1");
    let requests = babel.join();
    assert_eq!(requests.len(), 2);
    assert!(contains(
        &requests[0],
        "GET /api/trpc/worker.get,worker.getProjects?batch=1"
    ));
    assert!(contains(&requests[0], "Cookie: __session=session-cookie"));
    assert!(contains(
        &requests[1],
        "GET /api/trpc/transcriptions.getProjectQueues,transcriptions.getProjectQueues?batch=1"
    ));
    assert!(contains(&requests[1], "Cookie: __session=session-cookie"));
}

#[tokio::test]
async fn missing_native_session_and_http_401_are_signed_out_but_other_failures_are_not() {
    // No Clerk session in the native store: nothing is sent, the user must sign in again.
    let temp = tempfile::tempdir().expect("temp dir");
    let store_path = temp.path().join("babel-auth.json");
    std::fs::write(&store_path, r#"{"version":1,"cookies":[]}"#).expect("store");
    let signed_out = BabelClient::new(BabelConfig {
        auth_store: Some(store_path),
        clerk_url: "http://127.0.0.1:9".to_string(),
        ..BabelConfig::live("http://127.0.0.1:9")
    })
    .load_dashboard()
    .await
    .expect_err("no session");
    assert!(signed_out.signed_out, "{}", signed_out.message);

    // A saved login whose Clerk check cannot reach the network is not a sign-in problem.
    let offline_store = temp.path().join("offline-auth.json");
    std::fs::write(
        &offline_store,
        r#"{"version":1,"cookies":[{"name":"__client","value":"client-cookie","domain":"127.0.0.1","path":"/","expiresAt":4102444800}]}"#,
    )
    .expect("store");
    let offline = BabelClient::new(BabelConfig {
        auth_store: Some(offline_store),
        clerk_url: "http://127.0.0.1:9".to_string(),
        ..BabelConfig::live("http://127.0.0.1:9")
    })
    .load_dashboard()
    .await
    .expect_err("clerk unreachable");
    assert!(!offline.signed_out, "{}", offline.message);

    // Babel rejects the session.
    let babel = MockServer::start(vec![MockResponse::with_status(
        "401 Unauthorized",
        r#"[{"error":{"json":{"message":"UNAUTHORIZED"}}}]"#,
    )]);
    let rejected = BabelClient::new(BabelConfig {
        cookie: Some("__session=expired".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    })
    .load_dashboard()
    .await
    .expect_err("401");
    babel.join();
    assert!(rejected.signed_out, "{}", rejected.message);

    // A server failure is not a sign-in problem.
    let babel = MockServer::start(vec![MockResponse::with_status(
        "500 Internal Server Error",
        "{}",
    )]);
    let failed = BabelClient::new(BabelConfig {
        cookie: Some("__session=ok".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    })
    .load_dashboard()
    .await
    .expect_err("500");
    babel.join();
    assert!(!failed.signed_out, "{}", failed.message);
}

#[tokio::test]
async fn live_submit_returns_server_lint_issues() {
    let babel = MockServer::json(vec![
        r#"[{"result":{"data":{"json":{"success":false,"linterErrors":[{"annotationId":"a-1","reason":"Invalid bracket tag found.","severity":"error"}]}}}}]"#,
    ]);
    let client = BabelClient::new(BabelConfig {
        authorization: Some("Bearer fixed".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    });

    let error = client
        .submit_review_action(SubmitReviewActionRequest {
            review_action_id: "review-1".to_string(),
            annotations: vec![json!({"id": "a-1"})],
            is_stitched_queue: true,
        })
        .await
        .unwrap_err();
    assert!(error.message.contains("Invalid bracket tag found."));
    assert_eq!(
        error
            .lint_issues
            .as_deref()
            .map(|issues| issues[0].annotation_id.as_str()),
        Some("a-1")
    );

    let requests = babel.join();
    assert!(contains(
        &requests[0],
        "POST /api/trpc/transcriptions.submitTranscriptReviewAction?batch=1"
    ));
    assert!(contains(&requests[0], "authorization: bearer fixed"));
    assert!(contains(&requests[0], "\"isstitchedqueue\":true"));
}

/// The review-action fixture with local audio, as a claim response, plus its annotations.
/// With `without_annotations` the claim carries an empty `annotations` array.
fn local_audio_claim_payload(without_annotations: bool) -> (Value, Value) {
    let mut payload: Value = serde_json::from_str(REVIEW_ACTION_FIXTURE).expect("fixture");
    let data = payload
        .pointer_mut("/0/result/data/json")
        .expect("review action data");
    for value in data["processedRecordingUriMap"]
        .as_object_mut()
        .expect("uri map")
        .values_mut()
    {
        *value = json!("audio/local.wav");
    }
    let annotations = data["annotations"].clone();
    if without_annotations {
        data["annotations"] = json!([]);
    }
    (payload, annotations)
}

#[tokio::test]
async fn claim_without_annotations_loads_them_by_review_action_id() {
    let (claim, annotations) = local_audio_claim_payload(true);
    let annotation_id = annotations[0]["id"]
        .as_str()
        .expect("annotation id")
        .to_string();
    let review_action_id = claim
        .pointer("/0/result/data/json/actionId")
        .and_then(Value::as_str)
        .expect("action id")
        .to_string();
    let loaded = json!([{ "result": { "data": { "json": {
        "annotations": annotations,
        "lintErrors": [{
            "annotationId": annotation_id,
            "reason": "Fallback lint reason.",
            "severity": "warning",
        }],
    } } } }]);
    let babel = MockServer::start(vec![claim.to_string().into(), loaded.to_string().into()]);
    let client = BabelClient::new(BabelConfig {
        authorization: Some("Bearer fixed".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    });

    let task = transcript(
        client
            .open_task(
                &TaskRequest::ClaimNext {
                    queue_id: "queue-1".to_string(),
                },
                Vec::new(),
            )
            .await
            .expect("open claimed task"),
    );

    let project = task.workflow.project();
    assert_eq!(project.segments.len(), 24);
    let lint = exaudio_babel_domain::babel_metadata::server_lint_snapshot_from_project(project, 0)
        .expect("imported server lint");
    assert_eq!(lint.issues.len(), 1);
    assert_eq!(lint.issues[0].reason, "Fallback lint reason.");
    let requests = babel.join();
    assert!(contains(
        &requests[0],
        "POST /api/trpc/transcriptions.claimNextReviewActionFromReviewQueue?batch=1"
    ));
    assert!(contains(
        &requests[1],
        "GET /api/trpc/transcriptions.getAnnotationsByReviewActionId?batch=1&input="
    ));
    assert!(contains(&requests[1], &review_action_id));
}

#[tokio::test]
async fn claim_with_annotations_makes_no_annotations_request() {
    let (claim, _) = local_audio_claim_payload(false);
    let babel = MockServer::start(vec![claim.to_string().into()]);
    let client = BabelClient::new(BabelConfig {
        authorization: Some("Bearer fixed".to_string()),
        ..BabelConfig::live(babel.base_url.clone())
    });

    let task = transcript(
        client
            .open_task(
                &TaskRequest::ClaimNext {
                    queue_id: "queue-1".to_string(),
                },
                Vec::new(),
            )
            .await
            .expect("open claimed task"),
    );

    assert_eq!(task.workflow.project().segments.len(), 24);
    assert_eq!(babel.join().len(), 1);
}

fn pcm16_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[tokio::test]
async fn fixture_task_stages_presigned_audio_and_merges_route_metadata() {
    let temp = tempfile::tempdir().expect("temp dir");
    let wav = pcm16_wav(&[0, 1000, -1000, 0], 8_000);
    let audio = MockServer::start(vec![wav.clone().into(), wav.into()]);
    // Babel's editor presigns tracks by `chunkedProcessedRecordingId` for the review action.
    let signed = json!({
        "7a54b1cc-d57a-52b2-a2a6-d00ab89ab96b": format!("{}/a.wav", audio.base_url),
        "cc369471-bbd7-543f-881e-c4738c76215d": format!("{}/b.wav", audio.base_url),
    });
    let presign = json!([{ "result": { "data": { "json": signed } } }]).to_string();
    let babel = MockServer::start(vec![presign.into()]);
    let client = BabelClient::new(BabelConfig {
        fixture: Some(Fixture::Json(REVIEW_ACTION_FIXTURE.to_string())),
        authorization: Some("Bearer fixed".to_string()),
        auth_store: Some(temp.path().join("babel-auth.json")),
        ..BabelConfig::live(babel.base_url.clone())
    });
    let route = RouteContext {
        project_id: "project-1".to_string(),
        chunked_queue_id: "queue-1".to_string(),
        ..RouteContext::default()
    };

    let task = transcript(
        client
            .open_task(
                &TaskRequest::ClaimNext {
                    queue_id: "queue-1".to_string(),
                },
                route.merged_over(None),
            )
            .await
            .expect("open fixture task"),
    );

    assert_eq!(task.remote_audio_count, 2);
    assert!(
        task.audio_base_dir
            .starts_with(temp.path().join("audio-cache"))
    );
    assert_eq!(
        std::fs::read_dir(&task.audio_base_dir)
            .expect("cache dir")
            .count(),
        2
    );
    let project = task.workflow.project();
    assert_eq!(project.segments.len(), 24);
    assert_eq!(
        project
            .metadata
            .get(babel::META_ACTIVE_QUEUE_ID)
            .map(String::as_str),
        Some("queue-1")
    );
    assert_eq!(
        project
            .metadata
            .get(babel::META_PROJECT_ID)
            .map(String::as_str),
        Some("project-1")
    );
    assert!(
        project
            .metadata
            .contains_key("studio.babel.lintTagLanguage")
    );
    let presign_requests = babel.join();
    assert!(contains(
        &presign_requests[0],
        "GET /api/trpc/transcriptions.getPresignedChunkUrls?batch=1&input="
    ));
    let input = url::form_urlencoded::parse(
        presign_requests[0]
            .split_whitespace()
            .nth(1)
            .and_then(|target| target.split_once('?'))
            .map(|(_, query)| query)
            .unwrap_or_default()
            .as_bytes(),
    )
    .find(|(key, _)| key == "input")
    .map(|(_, value)| value.into_owned())
    .expect("input query");
    let input: Value = serde_json::from_str(&input).expect("input json");
    assert_eq!(
        input["0"]["json"]["chunkedprocessedrecordingids"],
        json!([
            "7a54b1cc-d57a-52b2-a2a6-d00ab89ab96b",
            "cc369471-bbd7-543f-881e-c4738c76215d"
        ])
    );
    assert!(input["0"]["json"]["reviewactionid"].is_string());
    assert!(contains(
        &presign_requests[0],
        "authorization: bearer fixed"
    ));
    assert_eq!(audio.join().len(), 2);
}

#[tokio::test]
async fn fixture_without_remote_audio_opens_offline() {
    let mut payload: Value = serde_json::from_str(REVIEW_ACTION_FIXTURE).expect("fixture");
    let uri_map = payload
        .pointer_mut("/0/result/data/json/processedRecordingUriMap")
        .and_then(Value::as_object_mut)
        .expect("uri map");
    for value in uri_map.values_mut() {
        *value = json!("audio/local.wav");
    }
    let client = BabelClient::new(BabelConfig {
        fixture: Some(Fixture::Json(payload.to_string())),
        ..offline_config()
    });

    let task = transcript(
        client
            .open_task(
                &TaskRequest::ReviewAction {
                    review_action_id: "r".to_string(),
                },
                Vec::new(),
            )
            .await
            .expect("offline task"),
    );

    assert_eq!(task.remote_audio_count, 0);
    assert_eq!(task.audio_base_dir, std::path::PathBuf::from("."));
    assert!(client.config().relay_session.is_none());
}

#[tokio::test]
async fn subaccount_session_routes_envelope_to_relay() {
    let temp = tempfile::tempdir().expect("temp dir");
    let relay = MockServer::json(vec![
        r#"{"payload":{"success":true,"updatedAt":"2026-07-07T12:00:00Z"}}"#,
    ]);
    let store_path = temp.path().join("subaccount-session.json");
    std::fs::write(
        &store_path,
        format!(
            r#"{{"version":1,"hostUrl":"{}","token":"relay-token","subaccountId":"sub-1"}}"#,
            relay.base_url
        ),
    )
    .expect("session store");
    let session = exaudio_babel_services::subaccounts::read_session(&store_path)
        .expect("read session")
        .expect("saved session");
    let client = BabelClient::relay(session);
    // Changing or removing the persisted global session cannot redirect this document.
    std::fs::remove_file(&store_path).expect("remove global login");

    let outcome = client
        .save_annotations(SaveAnnotationsRequest {
            review_action_id: "review-action-1".to_string(),
            annotations: vec![json!({ "id": "annotation-1" })],
        })
        .await
        .expect("relay save");

    assert_eq!(outcome.last_saved, "2026-07-07T12:00:00Z");
    assert!(client.config().relay_session.is_some());
    let requests = relay.join();
    assert!(contains(
        &requests[0],
        "POST /api/subaccounts/babel-service"
    ));
    assert!(contains(&requests[0], "Authorization: Bearer relay-token"));
    assert!(contains(
        &requests[0],
        "transcriptions.saveAnnotationsByReviewActionId"
    ));
    assert!(contains(
        &requests[0],
        "\"requestkind\":\"saveannotations\""
    ));
}

#[test]
fn dashboard_parsing_keeps_active_projects_with_queues_and_daily_progress() {
    let dashboard = babel::parse_dashboard(
        &json!([
            {
                "projectId": "eaa2b709-7e0a-5683-bca7-61099efa0b71",
                "name": "RU-tx-gold",
                "permissionStatus": "active",
                "status": "active",
                "chunkedQueueId": "11111111-1111-1111-1111-111111111111",
                "stitchedTxQueueId": "22222222-2222-2222-2222-222222222222",
                "taskLimit": { "limit": 25 },
                "workerStats": { "tasksCompletedToday": 7 }
            },
            {
                "projectId": "bc39f561-f8b5-537e-98d1-58467d23ee57",
                "name": "Russian-Podcast",
                "permissionStatus": "active",
                "status": "completed",
                "chunkedQueueId": "33333333-3333-3333-3333-333333333333"
            },
            {
                "projectId": "d1",
                "name": "RU-Emotion-VA",
                "status": "active",
                "chunkedQueueId": "44444444-4444-4444-4444-444444444444",
                "workerStats": { "tasksCompletedToday": 2 }
            }
        ])
        .to_string(),
    )
    .expect("dashboard");

    let names: Vec<_> = dashboard
        .projects
        .iter()
        .map(|project| project.name.as_str())
        .collect();
    assert_eq!(names, ["RU-Emotion-VA", "RU-tx-gold"]);
    let gold = &dashboard.projects[1];
    assert_eq!(gold.daily_task_limit, Some(25));
    assert_eq!(gold.tasks_completed_today, 7);
    assert_eq!(dashboard.projects[0].daily_task_limit, None);
    assert_eq!(dashboard.projects[0].tasks_completed_today, 2);
    assert_eq!(babel::daily_task_progress(&dashboard.projects), (7, 25, 1));
}

#[test]
fn dashboard_parsing_merges_discovered_queue_configs() {
    let dashboard = babel::parse_dashboard(
        &json!({
            "worker": { "id": "worker-7" },
            "projects": [{
                "projectId": "08a8456a-c472-5b36-863f-637384cf3a77",
                "name": "RU-tx-gold-dg",
                "permissionStatus": "active",
                "status": "active"
            }],
            "projectConfigs": [{
                "projectId": "08a8456a-c472-5b36-863f-637384cf3a77",
                "chunkedQueueId": "dg-chunked-queue",
                "stitchedTxQueueId": "dg-stitched-queue"
            }]
        })
        .to_string(),
    )
    .expect("dashboard");
    assert_eq!(dashboard.worker_id.as_deref(), Some("worker-7"));
    assert_eq!(dashboard.projects.len(), 1);
    assert_eq!(dashboard.projects[0].chunked_queue_id, "dg-chunked-queue");
    assert_eq!(dashboard.projects[0].stitched_queue_id, "dg-stitched-queue");
}

fn write_adapter_script(path: &std::path::Path) {
    #[cfg(windows)]
    std::fs::write(
        path,
        concat!(
            "@echo off\r\n",
            "more > \"%~1\"\r\n",
            "echo {\"result\":{\"data\":{\"success\":true,\"updatedAt\":\"2026-06-23T12:00:00Z\"}}}\r\n",
        ),
    )
    .expect("write adapter");

    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(
            path,
            concat!(
                "#!/bin/sh\n",
                "cat > \"$1\"\n",
                "printf '%s\\n' '{\"result\":{\"data\":{\"success\":true,\"updatedAt\":\"2026-06-23T12:00:00Z\"}}}'\n",
            ),
        )
        .expect("write adapter");
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod");
    }
}

#[tokio::test]
async fn direct_account_ignores_another_accounts_relay_session() {
    let temp = tempfile::tempdir().expect("temp dir");
    let relay_account =
        exaudio_babel_services::account::AccountStore::new(temp.path().join("relay"));
    exaudio_babel_services::subaccounts::save_session(
        &relay_account.session_path(),
        &exaudio_babel_services::subaccounts::SubaccountSession {
            host_url: "http://127.0.0.1:9".into(),
            token: "unrelated-relay-token".into(),
            subaccount_id: "other-account".into(),
            display_name: "Other account".into(),
        },
    )
    .expect("persist unrelated login");
    let direct = MockServer::json(vec![
        r#"{"success":true,"updatedAt":"2026-07-07T12:00:00Z"}"#,
    ]);
    let client = BabelClient::direct(BabelConfig {
        authorization: Some("Bearer direct-account".into()),
        base_url: direct.base_url.clone(),
        ..BabelConfig::for_account(&temp.path().join("direct"))
    });
    assert!(client.config().relay_session.is_none());
    client
        .save_annotations(SaveAnnotationsRequest {
            review_action_id: "direct-document".into(),
            annotations: vec![],
        })
        .await
        .expect("save with direct origin");
    let requests = direct.join();
    assert!(contains(
        &requests[0],
        "Authorization: Bearer direct-account"
    ));
    assert!(!contains(&requests[0], "unrelated-relay-token"));
}

#[tokio::test]
async fn onboarding_mutations_use_captured_operations_and_preserve_assessment_results() {
    let server = MockServer::json(vec![
        r#"{"passed":false,"granted":false,"attemptNumber":2,"wordErrorRate":0.4}"#,
        "null",
        r#"{"attemptNumber":3}"#,
    ]);
    let client = BabelClient::direct(BabelConfig {
        authorization: Some("Bearer onboarding-account".into()),
        ..BabelConfig::live(server.base_url.clone())
    });
    let request = SaveAnnotationsRequest {
        review_action_id: "onboarding-action".into(),
        annotations: vec![json!({"id":"row-1"})],
    };
    let outcome = client
        .submit_assessment(request.clone())
        .await
        .expect("assessment response");
    assert_eq!(outcome["passed"], false);
    assert_eq!(outcome["attemptNumber"], 2);
    client
        .transition_to_assessment("queue-1")
        .await
        .expect("transition");
    let practice = client
        .submit_practice_attempt(request)
        .await
        .expect("practice response");
    assert_eq!(practice["attemptNumber"], 3);
    let requests = server.join();
    assert!(contains(&requests[0], "transcriptions.submitAssessment"));
    assert!(contains(
        &requests[1],
        "transcriptions.transitionToAssessment"
    ));
    assert!(contains(&requests[1], "\"queueid\":\"queue-1\""));
    assert!(contains(
        &requests[2],
        "transcriptions.submitPracticeAttempt"
    ));
    assert!(
        requests
            .iter()
            .all(|request| !contains(request, "isStitchedQueue"))
    );
}
