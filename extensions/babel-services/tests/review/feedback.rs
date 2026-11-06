use anyhow::anyhow;
use exaudio_babel_domain::editor_model::feedback_assessment::{
    FeedbackInput, FeedbackInputResponse, FeedbackRubricEntry,
};
use exaudio_babel_services::babel::FeedbackDraftBootstrap;
use exaudio_babel_services::review::{
    FeedbackBootstrap, FeedbackBootstrapIds, FeedbackForm, ReviewerFeedback, parse_grade_result,
    reviewer_feedback_rating,
};
use serde_json::json;
use studio_core::Project;

use super::sample_project;

fn feedback(category: &str, score: &str, note: &str) -> ReviewerFeedback {
    ReviewerFeedback {
        category: category.to_string(),
        score: score.to_string(),
        note: note.to_string(),
    }
}

fn input(id: &str, label: &str) -> serde_json::Value {
    json!({ "id": id, "label": label })
}

fn project_with_form(flags: &str, inputs: serde_json::Value) -> Project {
    let mut project = sample_project();
    for (key, value) in [
        ("studio.babel.feedback.showButton", "true".to_string()),
        ("studio.babel.feedback.rubricFlags", flags.to_string()),
        (
            "studio.babel.feedback.formStepResponseId",
            "feedback-step-1".to_string(),
        ),
        ("studio.babel.feedback.inputs", inputs.to_string()),
    ] {
        project.metadata.insert(key.to_string(), value);
    }
    project
}

fn word_accuracy_form() -> FeedbackForm {
    FeedbackForm::from_project(&project_with_form(
        "tx-word-accuracy,tx-other-feedback",
        json!([
            input("word-rating", "Word Accuracy"),
            input("word-comment", "Word Accuracy Comment"),
            input("other", "Other Feedback"),
        ]),
    ))
    .unwrap()
}

fn generated_form(flags: &str, fields: &[(&str, &str)]) -> FeedbackForm {
    let inputs = fields
        .iter()
        .flat_map(|(key, label)| {
            [
                input(&format!("{key}-rating"), label),
                input(&format!("{key}-comment"), &format!("{label} Comment")),
            ]
        })
        .collect::<Vec<_>>();
    FeedbackForm::from_project(&project_with_form(flags, json!(inputs))).unwrap()
}

fn response_value<'a>(
    responses: &'a [FeedbackInputResponse],
    form_input_id: &str,
) -> Option<&'a str> {
    responses
        .iter()
        .find(|response| response.form_input_id == form_input_id)
        .map(|response| response.value.as_str())
}

#[test]
fn project_metadata_drives_form_presence_and_rubric() {
    assert!(FeedbackForm::from_project(&sample_project()).is_none());
    let form = word_accuracy_form();
    assert!(form.has_other_feedback);
    assert_eq!(form.rubric_fields.len(), 1);
    assert_eq!(form.rubric_fields[0].key, "wordAccuracy");
    assert_eq!(
        form.draft().entries.get("wordAccuracy"),
        Some(&FeedbackRubricEntry {
            rating: 1,
            comment: String::new(),
        })
    );
    assert!(!form.is_ready());
    assert_eq!(
        form.form_step_response_id.as_deref(),
        Some("feedback-step-1")
    );
}

#[test]
fn initial_feedback_metadata_hydrates_entries() {
    let mut project = project_with_form("tx-word-accuracy,tx-other-feedback", json!([]));
    project.metadata.insert(
        "studio.babel.feedback.initialFeedback".to_string(),
        json!({
            "wordAccuracy": { "rating": "3 - great", "comment": "Accurate." },
            "otherFeedback": "Thanks"
        })
        .to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.readOnly".to_string(),
        "true".to_string(),
    );

    let mut form = FeedbackForm::from_project(&project).unwrap();

    assert!(form.read_only);
    assert_eq!(form.draft().entries["wordAccuracy"].rating, 3);
    assert_eq!(form.draft().entries["wordAccuracy"].comment, "Accurate.");
    assert_eq!(form.draft().other_feedback, "Thanks");
    assert!(form.is_ready());
    assert!(!form.set_comment("wordAccuracy", "edit"));
    assert!(form.request_save().is_none());
}

#[test]
fn read_only_form_without_feedback_stays_empty() {
    let mut project = project_with_form("tx-word-accuracy", json!([]));
    project.metadata.insert(
        "studio.babel.feedback.readOnly".to_string(),
        "1".to_string(),
    );
    let form = FeedbackForm::from_project(&project).unwrap();
    assert!(form.draft().entries.is_empty());
}

#[test]
fn edits_respect_limits_and_save_the_rubric_payload() {
    let mut form = word_accuracy_form();
    assert!(form.set_rating("wordAccuracy", 9));
    assert!(form.set_comment("wordAccuracy", "Line one\nline two"));
    assert!(!form.set_comment("wordAccuracy", "Line one\nline two"));
    assert!(!form.set_comment("wordAccuracy", &"x".repeat(501)));
    assert!(!form.set_comment("unknown", "x"));
    assert!(form.set_other_feedback("General note"));

    let request = form.request_save().unwrap();

    assert!(form.is_saving());
    assert_eq!(request.form_step_response_id, "feedback-step-1");
    assert_eq!(
        response_value(&request.input_responses, "word-rating"),
        Some("3")
    );
    assert_eq!(
        response_value(&request.input_responses, "word-comment"),
        Some("Line one\nline two")
    );
    assert_eq!(
        response_value(&request.input_responses, "other"),
        Some("General note")
    );
    assert!(form.is_ready());
}

#[test]
fn rapid_edits_save_the_latest_snapshot_after_the_in_flight_one() {
    let mut form = word_accuracy_form();
    form.set_comment("wordAccuracy", "First edit");
    let first = form.request_save().unwrap();
    form.set_comment("wordAccuracy", "Newest edit");
    form.set_other_feedback("Newest general note");
    assert!(form.request_save().is_none());

    let newest = form
        .finish_save(&first, Err(anyhow!("stale request timed out")))
        .expect("the newest snapshot should be sent next");

    assert!(form.is_saving());
    assert!(!form.save_has_error());
    assert_eq!(
        response_value(&newest.input_responses, "word-comment"),
        Some("Newest edit")
    );
    assert_eq!(
        response_value(&newest.input_responses, "other"),
        Some("Newest general note")
    );
    assert!(form.finish_save(&first, Ok("late".to_string())).is_none());

    assert!(
        form.finish_save(&newest, Ok("2026-07-18T10:02:00Z".to_string()))
            .is_none()
    );
    assert!(!form.is_saving());
    assert_eq!(form.last_saved(), Some("2026-07-18T10:02:00Z"));
}

#[test]
fn terminal_save_error_blocks_submit_until_a_new_edit_saves() {
    let mut workflow = studio_editor::workflow::StudioWorkflow::from_project(sample_project());
    let mut form = word_accuracy_form();
    form.set_comment("wordAccuracy", "Failed edit");
    let failed = form.request_save().unwrap();

    form.finish_save(&failed, Err(anyhow!("network unavailable")));

    assert!(!form.is_saving());
    assert!(form.save_has_error());
    assert_eq!(form.last_autosave_error(), Some("network unavailable"));
    form.sync_submit_state(&mut workflow);
    let blocked_metadata = workflow.project().metadata.clone();

    form.set_comment("wordAccuracy", "Retried edit");
    let retry = form.request_save().unwrap();
    assert!(!form.save_has_error());
    form.finish_save(&retry, Ok("2026-07-18T10:03:00Z".to_string()));
    assert!(form.is_ready() && !form.is_saving() && !form.save_has_error());
    form.sync_submit_state(&mut workflow);
    assert_ne!(workflow.project().metadata, blocked_metadata);
}

#[test]
fn loading_and_failed_bootstrap_block_submit() {
    let mut form = FeedbackForm::loading(false);
    assert!(form.is_loading());
    assert!(form.submit_block_reason().is_some());
    form.mark_bootstrap_failed("Feedback form load failed: offline");
    assert!(!form.is_loading());
    assert_eq!(
        form.bootstrap_error(),
        Some("Feedback form load failed: offline")
    );
    assert!(form.submit_block_reason().is_some());
    assert!(word_accuracy_form().submit_block_reason().is_none());
}

#[test]
fn bootstrap_hydrates_saved_responses_or_defaults() {
    let inputs = vec![
        FeedbackInput {
            id: "word-rating".to_string(),
            label: Some("Word Accuracy".to_string()),
        },
        FeedbackInput {
            id: "word-comment".to_string(),
            label: Some("Word Accuracy Comment".to_string()),
        },
        FeedbackInput {
            id: "other".to_string(),
            label: Some("Other Feedback".to_string()),
        },
    ];
    let bootstrap = |responses: Vec<FeedbackInputResponse>| FeedbackBootstrap {
        rubric_flag_names: vec![
            "tx-word-accuracy".to_string(),
            "tx-other-feedback".to_string(),
        ],
        inputs: inputs.clone(),
        draft: FeedbackDraftBootstrap {
            form_step_response_id: "runtime-response".to_string(),
            input_responses: responses,
            updated_at: "2026-07-18T10:00:00Z".to_string(),
        },
    };
    let response = |id: &str, value: &str| FeedbackInputResponse {
        form_input_id: id.to_string(),
        value: value.to_string(),
    };

    let form = FeedbackForm::from_bootstrap(
        bootstrap(vec![
            response("word-rating", "2"),
            response("word-comment", "Initially saved"),
            response("other", "General"),
        ]),
        true,
    )
    .unwrap();
    assert!(form.is_tts_project);
    assert_eq!(
        form.draft().entries["wordAccuracy"].comment,
        "Initially saved"
    );
    assert_eq!(form.draft().entries["wordAccuracy"].rating, 2);
    assert_eq!(form.draft().other_feedback, "General");
    assert_eq!(form.last_saved(), Some("2026-07-18T10:00:00Z"));
    assert_eq!(
        form.form_step_response_id.as_deref(),
        Some("runtime-response")
    );

    let fresh = FeedbackForm::from_bootstrap(bootstrap(Vec::new()), false).unwrap();
    assert_eq!(fresh.last_saved(), None);
    assert_eq!(fresh.draft().entries["wordAccuracy"].rating, 1);

    let mut empty = bootstrap(Vec::new());
    empty.inputs.clear();
    assert!(FeedbackForm::from_bootstrap(empty, false).is_err());
}

#[test]
fn bootstrap_ids_fall_back_to_project_metadata_and_require_all_ids() {
    let mut project = sample_project();
    project.id = "feedback-project-1".to_string();
    project.metadata.insert(
        "babel.extra.package".to_string(),
        r#"{"action_level":2,"action_id":"review-action-42"}"#.to_string(),
    );

    let ids = FeedbackBootstrapIds::for_project(&project, " worker-7 ").unwrap();
    assert_eq!(ids.project_id, "feedback-project-1");
    assert_eq!(ids.review_action_id, "review-action-42");
    assert_eq!(ids.worker_id, "worker-7");

    assert!(FeedbackBootstrapIds::for_project(&project, "").is_err());
}

#[test]
fn generated_feedback_applies_every_matching_category() {
    let mut form = generated_form(
        "tx-word-accuracy,tx-timestamp-accuracy,tx-punctuation-formatting",
        &[
            ("wordAccuracy", "Word Accuracy"),
            ("timestampAccuracy", "Timestamp Accuracy"),
            ("punctuationFormatting", "Punctuation & Formatting"),
        ],
    );

    let applied = form.apply_reviewer_feedback(&[
        feedback("Word Accuracy", "3", "The corrected words are accurate."),
        feedback("timestampAccuracy", "2", "One timestamp needs review."),
        feedback("Punctuation", "1", "Punctuation   and casing\nneed work."),
    ]);

    assert_eq!(applied, 3);
    for (key, rating, comment) in [
        ("wordAccuracy", 3, "The corrected words are accurate."),
        ("timestampAccuracy", 2, "One timestamp needs review."),
        (
            "punctuationFormatting",
            1,
            "Punctuation and casing need work.",
        ),
    ] {
        assert_eq!(
            form.draft().entries.get(key),
            Some(&FeedbackRubricEntry {
                rating,
                comment: comment.to_string(),
            })
        );
    }
    let request = form.request_save().unwrap();
    assert_eq!(
        response_value(&request.input_responses, "timestampAccuracy-rating"),
        Some("2")
    );
}

#[test]
fn category_aliases_target_the_loaded_tts_rubric() {
    let mut form = generated_form(
        "tts-word-character-accuracy,tts-punctuation-formatting-accuracy,tts-non-speech-tag-accuracy,tts-style-tag-accuracy",
        &[
            ("wordCharacterAccuracy", "Word & Character Set Accuracy"),
            (
                "punctuationFormattingAccuracy",
                "Punctuation & Formatting Accuracy",
            ),
            ("nonSpeechTagAccuracy", "Non-Speech Tag Accuracy"),
            ("styleTagAccuracy", "Style Tag Accuracy"),
        ],
    );

    let applied = form.apply_reviewer_feedback(&[
        feedback("words", "2 - acceptable", "TTS words are mostly correct."),
        feedback("punctuation", "3", "TTS punctuation is correct."),
        feedback("non-speech tags", "1", "A sound tag is missing."),
        feedback("style tags", "2", "One delivery tag needs review."),
    ]);

    assert_eq!(applied, 4);
    for (key, rating) in [
        ("wordCharacterAccuracy", 2),
        ("punctuationFormattingAccuracy", 3),
        ("nonSpeechTagAccuracy", 1),
        ("styleTagAccuracy", 2),
    ] {
        assert_eq!(form.draft().entries[key].rating, rating, "{key}");
    }
}

#[test]
fn unknown_ambiguous_or_empty_feedback_is_not_applied() {
    let mut form = generated_form(
        "tx-punctuation-formatting,tts-punctuation-formatting-accuracy",
        &[
            ("punctuationFormatting", "Punctuation & Formatting"),
            (
                "punctuationFormattingAccuracy",
                "Punctuation & Formatting Accuracy",
            ),
        ],
    );
    let before = form.draft().clone();

    let applied = form.apply_reviewer_feedback(&[
        feedback(
            "punctuation",
            "3",
            "Ambiguous when both rubrics are loaded.",
        ),
        feedback("speaker identity", "2", "Not in the rubric."),
        feedback("Punctuation & Formatting", "2", "   "),
    ]);

    assert_eq!(applied, 0);
    assert_eq!(form.draft(), &before);
}

#[test]
fn grade_scores_change_ratings_and_leave_comments_untouched() {
    let mut form = word_accuracy_form();
    form.set_comment("wordAccuracy", "My own note.");
    let grade = parse_grade_result(
        &json!({
            "reviewActionId": "ra-2",
            "grades": [
                { "category": "Word Accuracy", "score": 3, "note": "Много ошибок", "scoreCap": 3 },
                { "category": "Timestamp Accuracy", "score": 1, "note": "" },
                { "category": "Punctuation & Formatting", "score": 1, "note": "" },
                { "category": "Tags & Emphasis", "score": 2, "note": "" },
                { "category": "Segmentation", "score": 1, "note": "" },
            ],
        })
        .to_string(),
    )
    .unwrap();

    let applied = form.apply_grade_scores(&grade.grades);

    assert!(applied >= 1);
    assert_eq!(form.draft().entries["wordAccuracy"].rating, 3);
    assert_eq!(form.draft().entries["wordAccuracy"].comment, "My own note.");
    let request = form.request_save().unwrap();
    assert_eq!(
        response_value(&request.input_responses, "word-rating"),
        Some("3")
    );
}

#[test]
fn scores_map_to_ratings_only_within_range() {
    assert_eq!(reviewer_feedback_rating("2 - acceptable"), Some(2));
    assert_eq!(reviewer_feedback_rating(" +3"), Some(3));
    assert_eq!(reviewer_feedback_rating("4"), None);
    assert_eq!(reviewer_feedback_rating("great"), None);
    assert_eq!(reviewer_feedback_rating(""), None);
}
