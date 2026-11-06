use exaudio_babel_services::review::{
    GeneratedReview, ReviewerCard, ReviewerFeedback, ReviewerSession, build_draft_generate_payload,
    build_draft_redistribute_payload, build_reviewer_analytics_payload, build_reviewer_payload,
    draft_generate_payloads_are_equivalent, ensure_draft_session_id, review_action_id,
    reviewer_capability_available,
};
use exaudio_gold::polish::DraftingConfig;
use serde_json::{Value, json};

use super::{review_action_workflow, sample_project};

fn config() -> DraftingConfig {
    DraftingConfig::from_inputs(" sk-test ", "openai/gpt-4o-mini", "flex", "low", "draft-1")
}

#[test]
fn reviewer_payload_uses_preserved_original_and_current_transcript() {
    let mut project = review_action_workflow().project().clone();
    let first = project.segments.first_mut().unwrap();
    let annotation_id = first
        .source_annotation_id
        .clone()
        .unwrap_or_else(|| first.id.clone());
    let original_text = first.metadata["studio.babel.original.text"]
        .as_str()
        .unwrap()
        .to_string();
    first.text = "Corrected native transcript row".to_string();

    let payload = build_reviewer_payload(&project).unwrap();

    assert_eq!(
        payload["reviewActionId"],
        payload["current"]["annotations"][0]["reviewActionId"]
    );
    let annotation = |side: &str| {
        payload[side]["annotations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|annotation| annotation["id"] == annotation_id)
            .unwrap()
            .clone()
    };
    assert_eq!(annotation("original")["content"], json!(original_text));
    assert_eq!(
        annotation("current")["content"],
        json!("Corrected native transcript row")
    );
    assert_eq!(payload["babelDiff"], Value::Null);
    assert!(
        payload["current"]["recordings"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
    assert!(
        annotation("current")["metadata"]
            .as_object()
            .is_none_or(|metadata| !metadata
                .keys()
                .any(|key| key.starts_with("studio.babel.original.")))
    );
}

#[test]
fn reviewer_payload_falls_back_to_package_review_action_id() {
    let mut project = review_action_workflow().project().clone();
    project.metadata.remove("studio.babel.reviewActionId");
    for segment in &mut project.segments {
        segment.metadata.remove("reviewActionId");
        segment.metadata.remove("review_action_id");
    }
    project.metadata.insert(
        "babel.extra.package".to_string(),
        r#"{"action_level":2,"reviewActionId":"review-action-42"}"#.to_string(),
    );

    let payload = build_reviewer_payload(&project).unwrap();

    assert_eq!(payload["reviewActionId"], json!("review-action-42"));
    assert_eq!(payload["current"]["actionId"], json!("review-action-42"));
}

#[test]
fn reviewer_payload_requires_a_review_action_id() {
    assert!(review_action_id(&sample_project()).is_none());
    assert!(build_reviewer_payload(&sample_project()).is_err());
}

#[test]
fn l2_aliases_enable_reviewer_and_build_a_stable_l1_baseline() {
    let base = review_action_workflow().project().clone();
    for (key, value) in [
        ("actionLevel", "2"),
        ("action_level", "L2"),
        ("studio.babel.actionLevel", "l2"),
        ("babel.extra.package", r#"{"actionLevel":"L2"}"#),
        ("babel.extra.package", r#"{"action_level":2}"#),
    ] {
        let mut project = base.clone();
        for alias in [
            "actionLevel",
            "action_level",
            "studio.babel.actionLevel",
            "babel.extra.package",
        ] {
            project.metadata.remove(alias);
        }
        project.metadata.insert(key.to_string(), value.to_string());
        project.metadata.insert(
            "babel.review_action.actionDecision".to_string(),
            "fail".to_string(),
        );

        assert!(reviewer_capability_available(&project), "{key}={value}");
        let payload = build_reviewer_payload(&project).unwrap();
        assert_eq!(payload["current"]["actionLevel"], json!(2));
        assert_eq!(payload["original"]["actionLevel"], json!(1));
        assert_eq!(payload["original"]["actionDecision"], "pass");
        assert_eq!(payload["current"]["actionDecision"], "fail");
        assert_eq!(payload["reviewActionId"], payload["current"]["actionId"]);
    }
}

#[test]
fn reviewer_is_unavailable_outside_l2() {
    let mut project = sample_project();
    assert!(!reviewer_capability_available(&project));
    project
        .metadata
        .insert("actionLevel".to_string(), "1".to_string());
    assert!(!reviewer_capability_available(&project));
}

#[test]
fn analytics_payload_snapshots_generated_feedback_and_comments() {
    let project = review_action_workflow().project().clone();
    let generated = GeneratedReview {
        model: Some("openai/gpt-4o-mini".to_string()),
        received_at: None,
        template_registry_version: None,
        matched_template_ids: Vec::new(),
        feedback: vec![ReviewerFeedback {
            category: "Word Accuracy".to_string(),
            score: "1".to_string(),
            note: "Use the wrong-word template text.".to_string(),
        }],
        cards: Vec::new(),
        prepared: json!({ "promptVersion": "review-v1" }),
        ai_review: json!({ "feedback": [{ "note": "Use the wrong-word template text." }] }),
    };
    let session = ReviewerSession {
        session_id: "session-1".to_string(),
        review_action_id: "review-action-1".to_string(),
        backend_service: None,
        backend_release: None,
        session_comment: "overall".to_string(),
        cards: vec![
            ReviewerCard {
                id: "card-1".to_string(),
                change_index: 1,
                comment: "card note".to_string(),
                ..ReviewerCard::default()
            },
            ReviewerCard {
                id: "card-2".to_string(),
                comment: "  ".to_string(),
                ..ReviewerCard::default()
            },
        ],
        suggestions: Vec::new(),
    };

    let payload =
        build_reviewer_analytics_payload(&project, Some(&session), Some(&generated)).unwrap();

    assert_eq!(payload["prepared"]["promptVersion"], "review-v1");
    assert_eq!(
        payload["aiReview"]["feedback"][0]["note"],
        "Use the wrong-word template text."
    );
    assert_eq!(
        payload["inputBoxes"]["categories"]["Word Accuracy"],
        json!({ "score": "1", "note": "Use the wrong-word template text." })
    );
    assert_eq!(
        payload["inputBoxes"]["notes"],
        json!([
            { "scope": "session", "comment": "overall" },
            { "scope": "card", "cardId": "card-1", "comment": "card note" },
        ])
    );
    assert_eq!(payload["metadata"]["client"], "transcription-studio-native");
    assert_eq!(payload["metadata"]["source"], "native-capability-panel");
    assert_eq!(payload["metadata"]["sessionId"], "session-1");
    assert_eq!(payload["metadata"]["reviewActionId"], "review-action-1");

    let bare = build_reviewer_analytics_payload(&project, None, None).unwrap();
    assert_eq!(bare["inputBoxes"], json!({ "categories": {}, "notes": [] }));
    assert_eq!(bare["aiReview"], Value::Null);
    assert!(bare.get("prepared").is_none());
}

#[test]
fn drafting_payloads_follow_the_gold_extension_contract() {
    let workflow = review_action_workflow();
    let project = workflow.project();
    let selected = project.segments[0].id.as_str();

    let generate = build_draft_generate_payload(project, &config()).unwrap();
    assert_eq!(generate["projectPreset"], "ru-gold-2sp-v1");
    assert_eq!(generate["openRouterApiKey"], "sk-test");
    assert_eq!(generate["model"], "openai/gpt-4o-mini");
    assert_eq!(generate["serviceTier"], "flex");
    assert_eq!(generate["reasoningEffort"], "low");
    assert_eq!(generate["draftSessionId"], "draft-1");
    assert_eq!(generate["jobId"], json!(review_action_id(project).unwrap()));
    assert!(
        generate["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["rowId"] == selected)
    );

    let redistribute =
        build_draft_redistribute_payload(project, Some(selected), &config()).unwrap();
    assert_eq!(
        redistribute["groups"][0]["speakerKey"],
        project.segments[0].speaker_id
    );
    assert!(
        redistribute["groups"][0]["draftAllocations"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn drafting_payloads_omit_default_options_and_sort_rows() {
    let mut project = sample_project();
    project.segments.reverse();
    let config = DraftingConfig::from_inputs("sk-test", "", "default", "default", "");

    let generate = build_draft_generate_payload(&project, &config).unwrap();

    for key in ["model", "serviceTier", "reasoningEffort", "draftSessionId"] {
        assert!(generate.get(key).is_none(), "{key}");
    }
    assert_eq!(generate["jobId"], "project-1");
    let rows = generate["rows"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row["rowId"].clone())
            .collect::<Vec<_>>(),
        vec![json!("s1"), json!("s2"), json!("s3")]
    );
    assert_eq!(rows[2]["index"], 2);
}

#[test]
fn redistribution_groups_only_the_selected_speaker() {
    let payload =
        build_draft_redistribute_payload(&sample_project(), Some("s3"), &config()).unwrap();
    let group = &payload["groups"][0];
    assert_eq!(group["groupId"], "project-1:spk-1");
    assert_eq!(group["fullText"], "First row. Third row.");
    assert_eq!(
        group["draftAllocations"],
        json!([
            { "segmentId": "s1", "text": "First row." },
            { "segmentId": "s3", "text": "Third row." },
        ])
    );
    assert_eq!(group["segments"][1]["index"], 2);
}

#[test]
fn generated_session_id_does_not_defeat_pending_equivalence() {
    let mut pending = json!({ "jobId": "job-1", "rows": [{ "rowId": "row-1" }] });
    ensure_draft_session_id(&mut pending);
    assert!(
        pending["draftSessionId"]
            .as_str()
            .unwrap()
            .starts_with("job-1:")
    );
    let requested = json!({ "jobId": "job-1", "rows": [{ "rowId": "row-1" }] });
    assert!(draft_generate_payloads_are_equivalent(&pending, &requested));
}

#[test]
fn changed_inputs_or_explicit_sessions_stay_distinct() {
    let pending = json!({
        "jobId": "job-1",
        "rows": [{ "rowId": "row-1" }],
        "model": "model-a",
        "draftSessionId": "session-a",
    });
    let changed_model =
        json!({ "jobId": "job-1", "rows": [{ "rowId": "row-1" }], "model": "model-b" });
    let changed_session = json!({
        "jobId": "job-1",
        "rows": [{ "rowId": "row-1" }],
        "model": "model-a",
        "draftSessionId": "session-b",
    });
    assert!(!draft_generate_payloads_are_equivalent(
        &pending,
        &changed_model
    ));
    assert!(!draft_generate_payloads_are_equivalent(
        &pending,
        &changed_session
    ));

    let mut explicit = changed_session.clone();
    ensure_draft_session_id(&mut explicit);
    assert_eq!(explicit["draftSessionId"], "session-b");
}
