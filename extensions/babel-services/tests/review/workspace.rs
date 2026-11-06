use anyhow::anyhow;
use exaudio_babel_services::review::{
    ReviewerNext, ReviewerSession, ReviewerTemplate, ReviewerWorkspace, parse_finalize_outcome,
    parse_reviewer_session,
};
use serde_json::json;

fn session_with_id(
    session_id: &str,
    session_comment: &str,
    card_one: &str,
    card_two: &str,
) -> ReviewerSession {
    parse_reviewer_session(
        &json!({
            "sessionId": session_id,
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

fn session(session_comment: &str, card_one: &str, card_two: &str) -> ReviewerSession {
    session_with_id("session-1", session_comment, card_one, card_two)
}

fn loaded(session_comment: &str, card_one: &str, card_two: &str) -> ReviewerWorkspace {
    let mut workspace = ReviewerWorkspace::default();
    workspace.replace_session(session(session_comment, card_one, card_two));
    workspace
}

fn finalize_payload(feedback: serde_json::Value) -> String {
    json!({ "sessionId": "session-1", "mode": "apply", "categoryFeedback": feedback }).to_string()
}

fn template(id: &str) -> ReviewerTemplate {
    ReviewerTemplate {
        id: id.to_string(),
        title: id.to_string(),
        description: String::new(),
        category: "Word Accuracy".to_string(),
        score: String::new(),
    }
}

#[test]
fn every_card_keeps_its_own_comment_draft_and_saves_together() {
    let mut workspace = loaded("session old", "card one old", "card two old");
    workspace.set_card_comment("card-1", "card one edited");
    workspace.set_card_comment("card-2", "card two edited");
    workspace.set_card_comment("unknown-card", "ignored");
    assert!(workspace.should_autosave());

    let save = workspace.begin_comment_save().unwrap();
    assert_eq!(
        save.body,
        json!({
            "sessionComment": "session old",
            "cardComments": { "card-1": "card one edited", "card-2": "card two edited" },
        })
    );
    assert!(!workspace.should_autosave());
    assert!(workspace.begin_comment_save().is_err());

    let next = workspace.finish_comment_save(
        &save,
        Ok(session("session old", "card one edited", "card two edited")),
    );
    assert_eq!(next, ReviewerNext::Idle);
    assert!(!workspace.comments_dirty());
    assert_eq!(workspace.card_comment("card-2"), "card two edited");
}

#[test]
fn comment_saves_list_commented_and_cleared_cards_only() {
    let mut workspace = loaded("", "saved note", "");
    workspace.set_card_comment("card-1", "");
    workspace.set_session_comment("overall");

    let save = workspace.begin_comment_save().unwrap();

    // card-1 is cleared on the backend; untouched empty card-2 is not reported as commented.
    assert_eq!(
        save.body,
        json!({ "sessionComment": "overall", "cardComments": { "card-1": "" } })
    );
}

#[test]
fn ack_preserves_edits_made_while_the_save_was_in_flight() {
    let mut workspace = loaded("", "old", "");
    workspace.set_card_comment("card-1", "sent snapshot");
    let save = workspace.begin_comment_save().unwrap();
    workspace.set_card_comment("card-1", "newer local edit");

    workspace.finish_comment_save(&save, Ok(session("", "sent snapshot", "")));

    assert!(workspace.comments_dirty());
    assert_eq!(workspace.card_comment("card-1"), "newer local edit");
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
    assert_eq!(retry.body["sessionComment"], "must survive");
}

#[test]
fn finalize_flushes_comments_then_requires_feedback() {
    let mut workspace = loaded("", "", "");
    workspace.set_card_comment("card-1", "final comment");

    assert_eq!(
        workspace.request_finalize("apply").unwrap(),
        ReviewerNext::SaveComments
    );
    let save = workspace.begin_comment_save().unwrap();
    let next = workspace.finish_comment_save(&save, Ok(session("", "final comment", "")));
    assert_eq!(
        next,
        ReviewerNext::Finalize {
            session_id: "session-1".to_string(),
            mode: "apply".to_string(),
        }
    );

    let empty = parse_finalize_outcome(
        "session-1",
        &json!({ "mode": "apply" }),
        &finalize_payload(json!([{ "category": "Word Accuracy", "score": 1, "note": "" }])),
    )
    .map(|mut outcome| {
        outcome.finalize.feedback.clear();
        outcome
    });
    let result = workspace
        .finish_finalize("session-1", "apply", empty)
        .unwrap();
    assert!(result.is_err(), "empty feedback must not count as applied");

    workspace.request_finalize("apply").unwrap();
    let outcome = parse_finalize_outcome(
        "session-1",
        &json!({ "mode": "apply" }),
        &finalize_payload(
            json!([{ "category": "Word Accuracy", "score": 3, "note": "Final wording." }]),
        ),
    );
    let finalize = workspace
        .finish_finalize("session-1", "apply", outcome)
        .expect("current finalize")
        .unwrap();
    assert_eq!(finalize.feedback[0].note, "Final wording.");
    assert_eq!(workspace.finalize_pending(), None);
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
fn a_new_session_drops_pending_work_and_ignores_late_results() {
    let mut workspace = loaded("", "", "");
    workspace.set_card_comment("card-1", "draft");
    workspace.toggle_expanded("card-1");
    workspace.set_template_search_open("card-1", true);
    workspace.request_finalize("apply").unwrap();
    let save = workspace.begin_comment_save().unwrap();

    workspace.replace_session(session_with_id("session-2", "", "fresh", ""));

    assert!(!workspace.comments_dirty());
    assert_eq!(workspace.finalize_pending(), None);
    assert!(!workspace.is_expanded("card-1"));
    assert!(workspace.template_search("card-1").is_none());
    assert_eq!(workspace.card_comment("card-1"), "fresh");
    assert_eq!(
        workspace.finish_comment_save(&save, Ok(session("", "draft", ""))),
        ReviewerNext::Idle
    );
    let late = parse_finalize_outcome(
        "session-1",
        &json!({ "mode": "apply" }),
        &finalize_payload(json!([{ "category": "Word Accuracy", "score": 1, "note": "x" }])),
    );
    assert!(
        workspace
            .finish_finalize("session-1", "apply", late)
            .is_none()
    );
}

#[test]
fn cards_expand_independently_and_all_at_once() {
    let mut workspace = loaded("", "", "");
    workspace.toggle_expanded("card-2");
    assert!(workspace.is_expanded("card-2"));
    assert!(!workspace.is_expanded("card-1"));
    workspace.set_all_expanded(true);
    assert!(workspace.is_expanded("card-1") && workspace.is_expanded("card-2"));
    workspace.set_all_expanded(false);
    assert!(!workspace.is_expanded("card-2"));
}

#[test]
fn template_search_keeps_only_the_latest_query_per_card() {
    let mut workspace = loaded("", "", "");
    assert!(workspace.begin_template_search("card-1", "word").is_none());
    workspace.set_template_search_open("card-1", true);
    workspace.set_template_search_open("card-2", true);

    let old = workspace.begin_template_search("card-1", "wor").unwrap();
    let new = workspace.begin_template_search("card-1", "word").unwrap();
    let other = workspace.begin_template_search("card-2", "comma").unwrap();
    assert!(workspace.template_search("card-1").unwrap().loading);

    assert!(workspace.finish_template_search(&new, Ok(vec![template("tpl-new")])));
    assert!(!workspace.finish_template_search(&old, Ok(vec![template("tpl-old")])));
    assert_eq!(
        workspace.template_search("card-1").unwrap().results[0].id,
        "tpl-new"
    );
    assert!(workspace.finish_template_search(&other, Err(anyhow!("HTTP 502"))));
    let card_two = workspace.template_search("card-2").unwrap();
    assert!(card_two.results.is_empty());
    assert!(card_two.error.as_deref().unwrap().contains("502"));

    assert!(workspace.begin_template_search("card-1", "  ").is_none());
    assert!(
        workspace
            .template_search("card-1")
            .unwrap()
            .results
            .is_empty()
    );

    let pending = workspace.begin_template_search("card-1", "late").unwrap();
    workspace.set_template_search_open("card-1", false);
    assert!(!workspace.finish_template_search(&pending, Ok(vec![template("tpl")])));
}

#[test]
fn blank_session_blocks_requests() {
    let mut workspace = ReviewerWorkspace::default();
    assert!(workspace.begin_comment_save().is_err());
    assert!(workspace.request_finalize("apply").is_err());
    workspace.set_session_comment("ignored without a session");
    assert!(!workspace.comments_dirty());
}
