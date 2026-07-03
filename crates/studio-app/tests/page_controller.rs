use std::collections::BTreeMap;

use serde_json::json;
use studio_app::{
    PageControllerAutoClaimResult, PageControllerEffect, PageControllerInitialLoadInput,
    PageControllerRenderBranch, PageMode, ProcessedRecording, ProjectConfig, ReviewActionPageData,
    WorkerPermission, apply_auto_claim_success, apply_processed_transcription_url_load_success,
    apply_review_action_url_load_success, apply_task_lookup_claim_success,
    build_task_lookup_claim_effect, build_workbench_props, choose_page_controller_render_branch,
    choose_task_lookup_read_only_action, compute_initial_load_effect,
    create_initial_page_controller_state, derive_is_rtl, derive_page_mode,
    map_processed_recordings_for_workbench, resolve_stitched_permission,
};

fn project_config() -> ProjectConfig {
    ProjectConfig {
        chunked_queue_id: Some("chunked-queue".to_string()),
        stitched_tx_queue_id: Some("stitched-queue".to_string()),
    }
}

fn page_data() -> ReviewActionPageData {
    ReviewActionPageData {
        action_id: "action-1".to_string(),
        action_level: Some(2),
        action_worker_id: Some("worker-2".to_string()),
        action_decision: Some("completed".to_string()),
        is_worker_resuming_claim: false,
        transcription_chunk_processed_recordings: vec![
            ProcessedRecording {
                speaker: Some(2),
                chunked_processed_recording_id: "recording-1".to_string(),
                processed_recording_url: None,
                label: None,
                extra: BTreeMap::new(),
            },
            ProcessedRecording {
                speaker: None,
                chunked_processed_recording_id: "recording-2".to_string(),
                processed_recording_url: None,
                label: None,
                extra: BTreeMap::new(),
            },
        ],
        processed_recording_uri_map: BTreeMap::from([
            ("recording-1".to_string(), "s3://bucket/one.wav".to_string()),
            ("recording-2".to_string(), "s3://bucket/two.wav".to_string()),
        ]),
        annotations: Vec::new(),
        linter_errors: Vec::new(),
        belongs_to_another_worker: false,
    }
}

#[test]
fn page_controller_initial_load_priority_and_permissions_match_recovered_babel() {
    assert_eq!(
        derive_page_mode("4c0db8c4-699e-4014-9dbf-27f178c3da8e", None),
        PageMode::Annotation
    );
    assert_eq!(
        derive_page_mode("ordinary-project", None),
        PageMode::Transcription
    );
    assert!(!derive_is_rtl("ordinary-project", None));
    assert!(resolve_stitched_permission(
        Some(&[WorkerPermission {
            review_queue_id: "stitched-queue".to_string(),
            level: 1,
        }]),
        "project-1",
        &BTreeMap::from([("project-1".to_string(), project_config())]),
    ));

    let transition = compute_initial_load_effect(PageControllerInitialLoadInput {
        review_action_id: Some("review-action-1".to_string()),
        processed_transcription_id: Some("processed-1".to_string()),
        display_feedback: false,
        show_task_lookup_modal: false,
        permissions_loading: false,
        active_claim_loading: false,
        worker_id: Some("worker-1".to_string()),
        project_config: Some(project_config()),
        state: create_initial_page_controller_state(),
    });
    assert!(transition.state.load_guard);
    assert!(transition.state.active);
    assert!(transition.state.loading_action);
    assert_eq!(
        transition.effect,
        Some(PageControllerEffect::LoadReviewAction {
            review_action_id: "review-action-1".to_string(),
        })
    );

    let transition = compute_initial_load_effect(PageControllerInitialLoadInput {
        review_action_id: None,
        processed_transcription_id: Some("processed-1".to_string()),
        display_feedback: false,
        show_task_lookup_modal: false,
        permissions_loading: false,
        active_claim_loading: false,
        worker_id: Some("worker-1".to_string()),
        project_config: Some(project_config()),
        state: create_initial_page_controller_state(),
    });
    assert_eq!(
        transition.effect,
        Some(PageControllerEffect::LoadLatestByProcessedTranscription {
            processed_transcription_id: "processed-1".to_string(),
            queue_id: "stitched-queue".to_string(),
        })
    );

    let transition = compute_initial_load_effect(PageControllerInitialLoadInput {
        review_action_id: None,
        processed_transcription_id: None,
        display_feedback: false,
        show_task_lookup_modal: false,
        permissions_loading: false,
        active_claim_loading: false,
        worker_id: None,
        project_config: Some(project_config()),
        state: create_initial_page_controller_state(),
    });
    assert_eq!(
        transition.effect,
        Some(PageControllerEffect::ClaimNext {
            queue_id: "chunked-queue".to_string(),
        })
    );
}

#[test]
fn page_controller_load_and_manual_lookup_branches_match_recovered_babel() {
    let mut state = create_initial_page_controller_state();
    state.loading_action = true;
    let loaded = apply_review_action_url_load_success(
        state.clone(),
        page_data(),
        project_config(),
        Some("worker-1".to_string()),
        true,
    );
    assert_eq!(loaded.active_queue_id, "chunked-queue");
    assert!(loaded.read_only);
    assert!(loaded.initial_feedback_open);
    assert!(!loaded.loading_action);

    let processed = apply_processed_transcription_url_load_success(
        state.clone(),
        page_data(),
        project_config(),
        false,
    );
    assert_eq!(processed.active_queue_id, "stitched-queue");
    assert!(processed.is_admin);
    assert!(processed.read_only);

    let failed = apply_auto_claim_success(
        state.clone(),
        PageControllerAutoClaimResult::Failure { error: None },
        project_config(),
    );
    assert_eq!(failed.load_error.as_deref(), Some("No tasks available"));
    assert!(!failed.loading_action);

    assert_eq!(
        choose_task_lookup_read_only_action(
            "review-action-2",
            "processed-2",
            Some(project_config())
        ),
        Some(PageControllerEffect::LoadReviewAction {
            review_action_id: "review-action-2".to_string(),
        })
    );
    assert_eq!(
        choose_task_lookup_read_only_action("", "processed-2", Some(project_config())),
        Some(PageControllerEffect::LoadLatestByProcessedTranscription {
            processed_transcription_id: "processed-2".to_string(),
            queue_id: "stitched-queue".to_string(),
        })
    );
    assert_eq!(
        build_task_lookup_claim_effect("processed-2", Some(project_config())),
        Some(PageControllerEffect::ClaimByProcessedTranscription {
            processed_transcription_id: "processed-2".to_string(),
            queue_id: "stitched-queue".to_string(),
        })
    );

    let claimed = apply_task_lookup_claim_success(state, page_data(), project_config());
    assert!(claimed.active);
    assert!(claimed.is_admin);
    assert_eq!(claimed.active_queue_id, "stitched-queue");
}

#[test]
fn page_controller_recording_mapping_render_branch_and_workbench_props_match_babel() {
    let data = page_data();
    let recordings = map_processed_recordings_for_workbench(
        Some(&data),
        &BTreeMap::from([(
            "s3://bucket/one.wav".to_string(),
            "https://signed/one.wav".to_string(),
        )]),
    );
    assert_eq!(
        recordings[0].processed_recording_url.as_deref(),
        Some("https://signed/one.wav")
    );
    assert_eq!(recordings[0].label.as_deref(), Some("Speaker 2"));
    assert_eq!(recordings[1].processed_recording_url.as_deref(), Some(""));
    assert_eq!(recordings[1].label.as_deref(), Some("Unknown Speaker"));

    assert_eq!(
        choose_page_controller_render_branch(
            true,
            false,
            false,
            Some(project_config()),
            false,
            false,
            None,
            "",
            &BTreeMap::new(),
            None,
        ),
        PageControllerRenderBranch::Loader
    );
    assert_eq!(
        choose_page_controller_render_branch(
            false,
            false,
            true,
            Some(project_config()),
            false,
            false,
            None,
            "",
            &BTreeMap::new(),
            None,
        ),
        PageControllerRenderBranch::LookupModal
    );
    assert_eq!(
        choose_page_controller_render_branch(
            false,
            false,
            false,
            Some(project_config()),
            false,
            false,
            Some(&data),
            "chunked-queue",
            &BTreeMap::from([("s3://bucket/one.wav".to_string(), "signed".to_string())]),
            None,
        ),
        PageControllerRenderBranch::Workbench
    );
    assert_eq!(
        choose_page_controller_render_branch(
            false,
            false,
            false,
            Some(project_config()),
            false,
            false,
            Some(&data),
            "chunked-queue",
            &BTreeMap::from([("s3://bucket/one.wav".to_string(), "signed".to_string())]),
            Some("boom"),
        ),
        PageControllerRenderBranch::Error
    );

    let props = build_workbench_props(
        &data,
        recordings,
        PageMode::Transcription,
        "project-1",
        Some("category-a".to_string()),
        "processed-1",
        "queue-1",
        true,
        false,
        false,
        true,
    );
    assert_eq!(props.review_action_id, "action-1");
    assert_eq!(props.review_action_level, Some(2));
    assert_eq!(props.annotations, Vec::<serde_json::Value>::new());
    assert_eq!(props.linter_errors, Vec::<serde_json::Value>::new());
    assert_eq!(props.transcription_chunk_processed_recordings.len(), 2);
    assert_eq!(props.mode, PageMode::Transcription);
    assert_eq!(props.project_category.as_deref(), Some("category-a"));
    assert!(props.is_admin);
    assert!(!props.read_only);
    assert!(props.initial_feedback_open);

    assert_eq!(data.annotations, Vec::<serde_json::Value>::new());
    assert_eq!(json!(props.project_id), json!("project-1"));
}
