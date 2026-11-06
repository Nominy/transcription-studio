use exaudio_babel_services::review::{
    parse_finalize_outcome, parse_generated_review, parse_reviewer_session, parse_template_search,
};
use exaudio_gold::polish::{
    draft_row_previews, parse_backend_health, parse_draft_result, parse_redistribution,
};
use serde_json::json;

use super::sample_project;

#[test]
fn reviewer_session_parses_cards_comments_suggestions_and_backend() {
    let session = parse_reviewer_session(
        &json!({
            "sessionId": "session-1",
            "reviewActionId": "review-action-1",
            "backendVersion": { "service": "babel-review-backend", "release": "0.3.9" },
            "cards": [{
                "id": "card-1",
                "changeIndex": 4,
                "type": "TEXT CHANGE",
                "summary": "Text changed",
                "evidence": "[-old+new]",
                "categories": ["Word Accuracy"],
                "matchedTemplateId": "tpl-1",
                "templateTitle": "Wrong word",
                "matchSource": "model",
                "opinionText": "Use the template.",
                "rationale": "The diff is a word substitution."
            }],
            "comments": {
                "sessionComment": "session note",
                "cardComments": { "card-1": "card note" }
            },
            "suggestions": [{
                "proposalId": "proposal-1",
                "operation": "create_template",
                "category": "Word Accuracy",
                "title": "New template",
                "description": "Reusable review note",
                "reportTexts": ["Use this wording"],
                "reason": "Repeated pattern",
                "sourceCardIds": ["card-1"]
            }]
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(session.session_id, "session-1");
    assert_eq!(
        session.backend_service.as_deref(),
        Some("babel-review-backend")
    );
    assert_eq!(session.backend_release.as_deref(), Some("0.3.9"));
    assert_eq!(session.session_comment, "session note");
    let card = &session.cards[0];
    assert_eq!(card.change_index, 4);
    assert_eq!(card.kind, "TEXT CHANGE");
    assert_eq!(card.template_title.as_deref(), Some("Wrong word"));
    assert_eq!(card.matched_template_id.as_deref(), Some("tpl-1"));
    assert_eq!(card.comment, "card note");
    let suggestion = &session.suggestions[0];
    assert_eq!(suggestion.proposal_id, "proposal-1");
    assert_eq!(suggestion.report_texts, vec!["Use this wording"]);
    assert_eq!(suggestion.source_card_ids, vec!["card-1"]);
    assert_eq!(suggestion.decision, "pending");
}

#[test]
fn reviewer_session_requires_ids_and_cards() {
    assert!(parse_reviewer_session(r#"{"reviewActionId":"r","cards":[]}"#).is_err());
    assert!(parse_reviewer_session(r#"{"sessionId":"s","reviewActionId":"r"}"#).is_err());
    let session = parse_reviewer_session(
        r#"{"sessionId":"s","reviewActionId":"r","cards":[],"proposals":[{"proposalId":"p"}]}"#,
    )
    .unwrap();
    assert_eq!(session.suggestions[0].proposal_id, "p");
}

#[test]
fn template_search_scores_render_as_text() {
    let results = parse_template_search(
        &json!({
            "query": "wrong word",
            "results": [{
                "id": "tpl-1",
                "title": "Wrong word",
                "description": "Use for word substitutions",
                "category": "Word Accuracy",
                "score": 0.91
            }]
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, "tpl-1");
    assert_eq!(results[0].score, "0.91");
    assert!(parse_template_search("{}").is_err());
}

#[test]
fn generated_review_uses_llm_feedback_and_classification_cards() {
    let review = parse_generated_review(
        &json!({
            "prepared": { "promptVersion": "review-v1" },
            "llm": {
                "feedback": [{ "category": "Word Accuracy", "score": 1, "note": "Use the wrong-word template text." }],
                "model": "openai/gpt-4o-mini",
                "receivedAt": "2026-06-29T10:00:00Z",
                "matchedTemplateIds": ["tpl-1"],
                "classifications": [{ "change": 4, "templateId": "tpl-1" }],
                "templateRegistryVersion": "registry-v1"
            }
        })
        .to_string(),
    )
    .unwrap();

    assert_eq!(review.model.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(
        review.template_registry_version.as_deref(),
        Some("registry-v1")
    );
    assert_eq!(review.matched_template_ids, vec!["tpl-1"]);
    assert_eq!(review.feedback[0].category, "Word Accuracy");
    assert_eq!(review.feedback[0].score, "1");
    assert_eq!(review.cards[0].id, "change-4");
    assert_eq!(review.cards[0].kind, "TEMPLATE MATCH");
    assert_eq!(
        review.cards[0].matched_template_id.as_deref(),
        Some("tpl-1")
    );
    assert_eq!(review.prepared["promptVersion"], "review-v1");
    assert_eq!(review.ai_review["model"], "openai/gpt-4o-mini");
}

#[test]
fn generated_review_without_cards_builds_category_feedback_cards() {
    let review = parse_generated_review(
        r#"{"categoryFeedback":[{"category":"Timestamp Accuracy","score":"2","feedback":"Timing note."}]}"#,
    )
    .unwrap();
    assert_eq!(review.cards[0].id, "feedback-0");
    assert_eq!(review.cards[0].kind, "CATEGORY FEEDBACK");
    assert_eq!(review.cards[0].categories, vec!["Timestamp Accuracy"]);
    assert_eq!(review.cards[0].evidence, "Timing note.");
    assert_eq!(review.ai_review["categoryFeedback"][0]["score"], "2");

    assert!(parse_generated_review(r#"{"prepared":{}}"#).is_err());
}

#[test]
fn finalize_outcome_uses_response_or_request_identity() {
    let outcome = parse_finalize_outcome(
        "session-1",
        &json!({ "mode": "skip" }),
        &json!({
            "sessionId": "session-1",
            "backendVersion": { "service": "babel-review-backend", "release": "0.3.9" },
            "categoryFeedback": [{ "category": "Timestamp Accuracy", "score": 1, "note": "Timing issue note." }],
            "appliedAt": "2026-06-29T10:00:00Z",
            "mode": "skip",
            "aiReview": null
        })
        .to_string(),
    )
    .unwrap();
    let finalize = &outcome.finalize;
    assert_eq!(finalize.session_id, "session-1");
    assert_eq!(finalize.mode, "skip");
    assert_eq!(finalize.applied_at.as_deref(), Some("2026-06-29T10:00:00Z"));
    assert_eq!(finalize.backend_release.as_deref(), Some("0.3.9"));
    assert_eq!(finalize.feedback[0].score, "1");
    assert_eq!(finalize.feedback[0].note, "Timing issue note.");
    assert_eq!(outcome.generated.cards[0].kind, "CATEGORY FEEDBACK");

    let fallback = parse_finalize_outcome(
        "session-2",
        &json!({ "mode": "apply" }),
        r#"{"feedback":[{"category":"Word Accuracy","note":"ok"}],"backendVersion":"1.2.3"}"#,
    )
    .unwrap()
    .finalize;
    assert_eq!(fallback.session_id, "session-2");
    assert_eq!(fallback.mode, "apply");
    assert_eq!(fallback.backend_release.as_deref(), Some("1.2.3"));
}

#[test]
fn backend_health_reports_credits_and_provider_diagnostics() {
    let health = parse_backend_health(
        &json!({
            "service": "review-backend",
            "backendVersion": { "release": "0.4.0" },
            "testMode": false,
            "openRouterCredits": { "data": { "remaining_credits": 12.0 } },
            "drafting": { "draftingModel": "openai/gpt-4o-mini", "serviceTier": "flex" }
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(health.backend_version.as_deref(), Some("0.4.0"));
    assert_eq!(
        health.openrouter_status.as_deref(),
        Some("credits remaining: 12")
    );
    assert_eq!(health.test_mode, Some(false));
    assert_eq!(
        health.provider_diagnostics,
        vec!["model: openai/gpt-4o-mini", "tier: flex"]
    );

    let failing = parse_backend_health(r#"{"openRouterCredits":{"error":"no key"}}"#).unwrap();
    assert_eq!(failing.service, "review-backend");
    assert_eq!(
        failing.openrouter_status.as_deref(),
        Some("credits unavailable: no key")
    );
}

#[test]
fn draft_result_parses_rows_summary_and_previews() {
    let result = parse_draft_result(
        &json!({
            "draftRows": [
                { "rowId": "s1", "rewrittenText": "Drafted.", "status": "rewritten", "warnings": ["w"] },
                { "rowId": "missing", "rewrittenText": "x", "status": "unchanged" }
            ],
            "summary": { "totalRows": 2, "rewrittenRows": "1", "unchangedRows": 1, "failedRows": 0 }
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(result.rows[0].warnings, vec!["w"]);
    let summary = result.summary.clone().unwrap();
    assert_eq!(
        (
            summary.total_rows,
            summary.rewritten_rows,
            summary.unchanged_rows
        ),
        (2, 1, 1)
    );
    assert!(!result.reconciled_after_stream_loss);

    let previews = draft_row_previews(&sample_project(), &result);
    assert_eq!(previews[0].original_text, "First row.");
    assert_eq!(previews[1].original_text, "");
    assert!(!previews[0].applied && !previews[0].skipped);

    assert!(parse_draft_result(r#"{"draftRows":[{"rewrittenText":"x"}]}"#).is_err());
    assert!(parse_draft_result(r#"{"draftRows":[{"rowId":"a"}]}"#).is_err());
    assert!(parse_draft_result("{}").is_err());
}

#[test]
fn redistribution_pairs_results_with_request_allocations() {
    let request = json!({
        "groups": [{
            "groupId": "g1",
            "draftAllocations": [
                { "segmentId": "s1", "text": "One. Two." },
                { "segmentId": "s3", "text": "Three." }
            ]
        }]
    });
    let previews = parse_redistribution(
        &request,
        &json!({
            "model": "top-model",
            "results": [
                { "groupId": "g1", "review": { "acceptDraft": false, "notes": "move one", "moves": [{ "fromIndex": 1, "toIndex": 2, "sentenceCount": "1" }] } },
                { "groupId": "g2", "error": "upstream failed" }
            ]
        })
        .to_string(),
    )
    .unwrap();
    assert_eq!(previews[0].model.as_deref(), Some("top-model"));
    assert_eq!(previews[0].notes.as_deref(), Some("move one"));
    assert_eq!(previews[0].allocations.len(), 2);
    assert_eq!(previews[0].moves[0].sentence_count, 1);
    assert_eq!(previews[0].error, None);
    assert!(previews[1].allocations.is_empty());
    assert_eq!(previews[1].error.as_deref(), Some("upstream failed"));
}

#[test]
fn malformed_or_impossible_moves_surface_validation_errors() {
    let request = json!({
        "groups": [{ "groupId": "g1", "draftAllocations": [{ "segmentId": "s1", "text": "Only one." }, { "segmentId": "s3", "text": "" }] }]
    });
    let previews = parse_redistribution(
        &request,
        r#"{"results":[
            {"groupId":"g1","review":{"moves":[{"fromIndex":1,"toIndex":2}]}},
            {"groupId":"g1","review":{"moves":[{"fromIndex":1,"toIndex":2,"sentenceCount":2}]}}
        ]}"#,
    )
    .unwrap();
    assert!(previews[0].error.is_some());
    assert!(previews[0].moves.is_empty());
    assert!(previews[1].error.is_some());
    assert_eq!(previews[1].moves.len(), 1);
}
