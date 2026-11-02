use anyhow::anyhow;
use serde_json::json;
use studio_services::review::{
    ReviewerNext, ReviewerSession, ReviewerWorkspace, parse_finalize_outcome,
    parse_generated_review, parse_reviewer_session,
};

fn session(session_comment: &str, card_one: &str, card_two: &str) -> ReviewerSession {
    parse_reviewer_session(
        &json!({
            "sessionId": "session-1",
            "reviewActionId": "review-action-42",
            "comments": {
                "sessionComment": session_comment,
                "cardComments": { "card-1": card_one, "card-2": card_two },
            },
            "cards": [
                { "id": "card-1", "changeIndex": 1, "categories": ["Word Accuracy"], "matchedTemplateId": "tpl-1" },
                { "id": "card-2", "changeIndex": 2, "categories": ["Punctuation"] },
            ],
            "suggestions": [{ "proposalId": "proposal-1" }],
        })
        .to_string(),
    )
    .unwrap()
}

fn loaded(session_comment: &str, card_one: &str, card_two: &str) -> ReviewerWorkspace {
    let mut workspace = ReviewerWorkspace::default();
    workspace.replace_session(session(session_comment, card_one, card_two));
    workspace
}

fn finalize_payload(mode: &str) -> String {
    json!({
        "sessionId": "session-1",
        "mode": mode,
        "categoryFeedback": [{ "category": "Word Accuracy", "score": 3, "note": "Final wording." }],
    })
    .to_string()
}

#[test]
fn loading_a_session_selects_the_first_card_and_proposal() {
    let workspace = loaded("session note", "note one", "note two");
    assert_eq!(workspace.session_id(), "session-1");
    assert_eq!(workspace.session_comment(), "session note");
    assert_eq!(workspace.selected_card_id(), "card-1");
    assert_eq!(workspace.card_comment(), "note one");
    assert_eq!(workspace.template_id(), "tpl-1");
    assert_eq!(workspace.proposal_id(), "proposal-1");
    assert!(!workspace.comments_dirty());
}

#[test]
fn card_selection_syncs_comment_and_template() {
    let mut workspace = loaded("", "note one", "note two");
    workspace.select_card("card-2");
    assert_eq!(workspace.card_comment(), "note two");
    assert_eq!(workspace.template_id(), "");
    workspace.select_card("card-1");
    assert_eq!(workspace.template_id(), "tpl-1");
}

#[test]
fn autosave_sends_one_save_with_session_and_selected_card_comments() {
    let mut workspace = ReviewerWorkspace::default();
    workspace.set_session_id("session-1");
    workspace.select_card("card-1");
    workspace.set_session_comment("session autosave note");
    workspace.set_card_comment("card autosave note");
    assert!(workspace.should_autosave());

    let save = workspace.begin_comment_save().unwrap();

    assert_eq!(save.session_id, "session-1");
    assert_eq!(save.body["sessionComment"], "session autosave note");
    assert_eq!(save.body["cardComments"]["card-1"], "card autosave note");
    assert!(workspace.comments_dirty());
    assert!(!workspace.should_autosave());
    assert!(workspace.begin_comment_save().is_err());
}

#[test]
fn comment_save_serializes_every_card_and_keeps_selection_on_ack() {
    let mut workspace = loaded("session old", "card one old", "card two old");
    workspace.set_card_comment("card one edited");
    workspace.select_card("card-2");
    workspace.set_card_comment("card two edited");
    workspace.select_card("card-1");

    let save = workspace.begin_comment_save().unwrap();
    assert_eq!(save.body["cardComments"]["card-1"], "card one edited");
    assert_eq!(save.body["cardComments"]["card-2"], "card two edited");

    let next = workspace.finish_comment_save(
        &save,
        Ok(session("session old", "card one edited", "card two edited")),
    );

    assert_eq!(next, ReviewerNext::Idle);
    assert!(!workspace.comments_dirty());
    assert_eq!(workspace.selected_card_id(), "card-1");
    assert_eq!(workspace.card_comment(), "card one edited");
    assert_eq!(
        workspace.session().unwrap().cards[1].comment,
        "card two edited"
    );
}

#[test]
fn ack_preserves_edits_made_while_the_save_was_in_flight() {
    let mut workspace = loaded("", "old", "");
    workspace.set_card_comment("sent snapshot");
    let save = workspace.begin_comment_save().unwrap();
    workspace.set_card_comment("newer local edit");

    workspace.finish_comment_save(&save, Ok(session("", "sent snapshot", "")));

    assert!(workspace.comments_dirty());
    assert_eq!(workspace.card_comment(), "newer local edit");
    assert_eq!(
        workspace.session().unwrap().cards[0].comment,
        "newer local edit"
    );
    assert!(workspace.should_autosave());
}

#[test]
fn failed_save_stays_dirty_and_waits_for_manual_retry() {
    let mut workspace = loaded("", "", "");
    workspace.set_session_comment("must survive");
    let save = workspace.begin_comment_save().unwrap();

    workspace.finish_comment_save(&save, Err(anyhow!("network down")));

    assert!(workspace.comments_dirty());
    assert!(workspace.comment_save_failed());
    assert!(!workspace.should_autosave());
    let retry = workspace.begin_comment_save().unwrap();
    assert!(!workspace.comment_save_failed());
    assert_eq!(retry.body["sessionComment"], "must survive");
}

#[test]
fn finalize_waits_for_comment_ack_then_settles() {
    let mut workspace = loaded("", "", "");
    workspace.set_card_comment("final comment");

    assert_eq!(
        workspace.request_finalize("apply").unwrap(),
        ReviewerNext::SaveComments
    );
    assert_eq!(workspace.finalize_pending(), Some("apply"));
    let save = workspace.begin_comment_save().unwrap();
    let next = workspace.finish_comment_save(&save, Ok(session("", "final comment", "")));
    assert_eq!(
        next,
        ReviewerNext::Finalize {
            session_id: "session-1".to_string(),
            mode: "apply".to_string(),
        }
    );
    assert!(!workspace.comments_dirty());

    let outcome = parse_finalize_outcome(
        "session-1",
        &json!({ "mode": "apply" }),
        &finalize_payload("apply"),
    );
    let finalize = workspace
        .finish_finalize("session-1", "apply", outcome)
        .expect("current finalize")
        .unwrap();
    assert_eq!(finalize.feedback[0].note, "Final wording.");
    assert_eq!(workspace.finalize_pending(), None);
    assert!(workspace.finalize_result.is_some());
    assert!(workspace.generated_review.is_some());
}

#[test]
fn finalize_without_pending_comments_starts_immediately_and_once() {
    let mut workspace = loaded("", "", "");
    let next = workspace.request_finalize("skip").unwrap();
    assert!(matches!(next, ReviewerNext::Finalize { ref mode, .. } if mode == "skip"));
    assert_eq!(
        workspace.request_finalize("skip").unwrap(),
        ReviewerNext::Idle
    );

    let failed = workspace.finish_finalize("session-1", "skip", Err(anyhow!("HTTP 500")));
    assert!(failed.is_some_and(|result| result.is_err()));
    assert_eq!(workspace.finalize_pending(), None);
}

#[test]
fn comment_edits_after_the_ack_keep_finalize_waiting() {
    let mut workspace = loaded("", "", "");
    workspace.set_card_comment("first");
    workspace.request_finalize("apply").unwrap();
    let save = workspace.begin_comment_save().unwrap();
    workspace.set_card_comment("second");

    let next = workspace.finish_comment_save(&save, Ok(session("", "first", "")));

    assert_eq!(next, ReviewerNext::SaveComments);
    let save = workspace.begin_comment_save().unwrap();
    assert_eq!(save.body["cardComments"]["card-1"], "second");
}

#[test]
fn switching_sessions_drops_pending_work_and_ignores_late_results() {
    let mut workspace = loaded("", "", "");
    workspace.set_card_comment("draft");
    workspace.request_finalize("apply").unwrap();
    let save = workspace.begin_comment_save().unwrap();

    workspace.set_session_id("session-2");

    assert!(!workspace.comments_dirty());
    assert_eq!(workspace.finalize_pending(), None);
    assert_eq!(
        workspace.finish_comment_save(&save, Ok(session("", "draft", ""))),
        ReviewerNext::Idle
    );
    assert_eq!(workspace.session_id(), "session-2");
    let late = parse_finalize_outcome(
        "session-1",
        &json!({ "mode": "apply" }),
        &finalize_payload("apply"),
    );
    assert!(
        workspace
            .finish_finalize("session-1", "apply", late)
            .is_none()
    );
    assert!(workspace.require_session_id().is_ok());
}

#[test]
fn blank_targets_are_rejected() {
    let mut workspace = ReviewerWorkspace::default();
    assert!(workspace.require_session_id().is_err());
    assert!(workspace.begin_comment_save().is_err());
    assert!(workspace.request_finalize("apply").is_err());
    workspace.set_session_id("s");
    workspace.select_card("c");
    assert!(workspace.template_match_target().is_err());
    workspace.set_template_id("tpl");
    assert_eq!(
        workspace.template_match_target().unwrap(),
        ("s".to_string(), "c".to_string(), "tpl".to_string())
    );
    assert!(workspace.suggestion_target().is_err());
}

#[test]
fn selected_card_feedback_prefers_generated_cards_and_matches_categories() {
    let mut workspace = loaded("", "", "");
    workspace.generated_review = Some(
        parse_generated_review(
            r#"{"llm":{"feedback":[
                {"category":"Punctuation","score":2,"note":"Commas."},
                {"category":"Word Accuracy","score":3,"note":"Words."}
            ]}}"#,
        )
        .unwrap(),
    );

    let (card, feedback) = workspace.selected_card_feedback().unwrap();
    assert_eq!(card.id, "card-1");
    assert_eq!(feedback.unwrap().note, "Words.");

    workspace.select_card("feedback-0");
    let (card, feedback) = workspace.selected_card_feedback().unwrap();
    assert_eq!(card.kind, "CATEGORY FEEDBACK");
    assert_eq!(feedback.unwrap().note, "Commas.");
}
