//! Startup route parsing, project metadata entries and load plan (ported from the iced app's
//! startup_route.rs behavior tests).

use exaudio_babel_domain::route::{
    META_ACTIVE_QUEUE_ID, META_CHUNKED_QUEUE_ID, META_DISPLAY_FEEDBACK, META_FEEDBACK_INITIAL_OPEN,
    META_REVIEW_ACTION_ID, META_STITCHED_QUEUE_ID, META_TASK_LOOKUP_OPEN,
    META_TASK_LOOKUP_PROCESSED_ID, META_TASK_LOOKUP_REVIEW_ACTION_ID,
    META_TASK_LOOKUP_SHOW_STITCHED, normalize_key,
};
use exaudio_babel_domain::route::{RouteLoad, StartupRoute};

fn entry(key: &str, value: &str) -> (String, String) {
    (key.to_string(), value.to_string())
}

fn metadata_value(route: &StartupRoute, key: &str) -> Option<String> {
    route
        .metadata_entries()
        .into_iter()
        .find(|(entry_key, _)| entry_key == key)
        .map(|(_, value)| value)
}

#[test]
fn metadata_keys_match_babel_service_contract() {
    assert_eq!(META_REVIEW_ACTION_ID, "studio.babel.reviewActionId");
    assert_eq!(META_ACTIVE_QUEUE_ID, "studio.babel.activeQueueId");
    assert_eq!(META_CHUNKED_QUEUE_ID, "studio.babel.chunkedQueueId");
    assert_eq!(META_STITCHED_QUEUE_ID, "studio.babel.stitchedQueueId");
    assert_eq!(META_DISPLAY_FEEDBACK, "studio.babel.displayFeedback");
    assert_eq!(
        META_FEEDBACK_INITIAL_OPEN,
        "studio.babel.feedback.initialOpen"
    );
    assert_eq!(META_TASK_LOOKUP_OPEN, "studio.babel.taskLookup.open");
    assert_eq!(
        META_TASK_LOOKUP_PROCESSED_ID,
        "studio.babel.taskLookup.processedTranscriptionId"
    );
    assert_eq!(
        META_TASK_LOOKUP_REVIEW_ACTION_ID,
        "studio.babel.taskLookup.reviewActionId"
    );
    assert_eq!(
        META_TASK_LOOKUP_SHOW_STITCHED,
        "studio.babel.taskLookup.showStitchedOption"
    );
}

#[test]
fn startup_route_args_with_equals_values_load_review_action_before_default_claim() {
    let route = StartupRoute::parse_args([
        "studio-app",
        "--reviewActionId= review-action-url ",
        "--processedTranscriptionId= processed-url ",
        "--displayFeedback=true",
        "--chunkedQueueId= chunked-queue-url ",
        "--stitchedQueueId= stitched-queue-url ",
    ]);

    assert_eq!(
        route,
        StartupRoute {
            review_action_id: Some("review-action-url".to_string()),
            processed_transcription_id: Some("processed-url".to_string()),
            display_feedback: true,
            show_task_lookup: false,
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            stitched_queue_id: Some("stitched-queue-url".to_string()),
        }
    );
    assert_eq!(
        route.load(),
        Some(RouteLoad::ReviewAction {
            review_action_id: "review-action-url".to_string(),
        })
    );
    assert_eq!(
        metadata_value(&route, META_REVIEW_ACTION_ID).as_deref(),
        Some("review-action-url")
    );
    assert_eq!(
        metadata_value(&route, META_DISPLAY_FEEDBACK).as_deref(),
        Some("true")
    );
    assert_eq!(
        metadata_value(&route, META_FEEDBACK_INITIAL_OPEN).as_deref(),
        Some("true")
    );
}

#[test]
fn startup_route_query_arg_opens_task_lookup_with_processed_transcription() {
    let route = StartupRoute::parse_args([
        "studio-app",
        "babel://we-edit?processedTranscriptionId=processed-url&displayFeedback=true&taskLookup=true&stitchedQueueId=stitched-queue-url",
    ]);

    assert_eq!(
        route,
        StartupRoute {
            review_action_id: None,
            processed_transcription_id: Some("processed-url".to_string()),
            display_feedback: true,
            show_task_lookup: true,
            chunked_queue_id: None,
            stitched_queue_id: Some("stitched-queue-url".to_string()),
        }
    );
    assert_eq!(route.load(), Some(RouteLoad::OpenTaskLookup));
    assert_eq!(
        metadata_value(&route, META_TASK_LOOKUP_PROCESSED_ID).as_deref(),
        Some("processed-url")
    );
    assert_eq!(
        metadata_value(&route, META_STITCHED_QUEUE_ID).as_deref(),
        Some("stitched-queue-url")
    );
    assert_eq!(
        metadata_value(&route, META_TASK_LOOKUP_OPEN).as_deref(),
        Some("true")
    );
}

#[test]
fn processed_transcription_with_stitched_queue_loads_latest_review_action() {
    let route = StartupRoute {
        show_task_lookup: false,
        ..StartupRoute::parse_args([
            "babel://we-edit?processedTranscriptionId=processed-url&displayFeedback=true&taskLookup=true&stitchedQueueId=stitched-queue-url",
        ])
    };

    assert_eq!(
        route.load(),
        Some(RouteLoad::LatestReviewAction {
            processed_transcription_id: "processed-url".to_string(),
            queue_id: "stitched-queue-url".to_string(),
        })
    );
    assert_eq!(
        metadata_value(&route, META_DISPLAY_FEEDBACK).as_deref(),
        Some("true")
    );
}

#[test]
fn chunked_queue_only_route_claims_next_from_queue() {
    let route = StartupRoute {
        chunked_queue_id: Some("chunked-queue-url".to_string()),
        ..StartupRoute::default()
    };

    assert_eq!(
        route.load(),
        Some(RouteLoad::ClaimNextFromQueue {
            queue_id: "chunked-queue-url".to_string(),
        })
    );
}

#[test]
fn processed_transcription_without_stitched_queue_falls_back_to_chunked_claim() {
    let route = StartupRoute::parse_args([
        "--processed-transcription-id",
        "processed-url",
        "--chunked-queue-id",
        "chunked-queue-url",
    ]);
    assert_eq!(
        route.load(),
        Some(RouteLoad::ClaimNextFromQueue {
            queue_id: "chunked-queue-url".to_string(),
        })
    );

    let processed_only =
        StartupRoute::parse_args(["--processed-transcription-id", "processed-url"]);
    assert_eq!(processed_only.load(), None);
    assert!(!processed_only.is_empty());
}

#[test]
fn load_precedence_is_task_lookup_then_review_action_then_processed_then_claim() {
    let full = StartupRoute {
        review_action_id: Some("ra".to_string()),
        processed_transcription_id: Some("pt".to_string()),
        display_feedback: false,
        show_task_lookup: true,
        chunked_queue_id: Some("cq".to_string()),
        stitched_queue_id: Some("sq".to_string()),
    };
    assert_eq!(full.load(), Some(RouteLoad::OpenTaskLookup));

    let without_lookup = StartupRoute {
        show_task_lookup: false,
        ..full
    };
    assert_eq!(
        without_lookup.load(),
        Some(RouteLoad::ReviewAction {
            review_action_id: "ra".to_string(),
        })
    );

    let without_review = StartupRoute {
        review_action_id: None,
        ..without_lookup
    };
    assert_eq!(
        without_review.load(),
        Some(RouteLoad::LatestReviewAction {
            processed_transcription_id: "pt".to_string(),
            queue_id: "sq".to_string(),
        })
    );

    let without_processed = StartupRoute {
        processed_transcription_id: None,
        ..without_review
    };
    assert_eq!(
        without_processed.load(),
        Some(RouteLoad::ClaimNextFromQueue {
            queue_id: "cq".to_string(),
        })
    );
}

#[test]
fn empty_route_has_no_metadata_and_no_load() {
    let route = StartupRoute::parse_args(["studio-app", "--unknown-flag=1", "positional"]);
    assert!(route.is_empty());
    assert_eq!(route, StartupRoute::default());
    assert!(route.metadata_entries().is_empty());
    assert_eq!(route.load(), None);
}

#[test]
fn display_feedback_alone_is_not_empty_but_loads_nothing() {
    let route = StartupRoute::parse_args(["--display-feedback"]);
    assert!(route.display_feedback);
    assert!(!route.is_empty());
    assert_eq!(route.load(), None);
}

#[test]
fn space_separated_values_and_bool_flags_are_parsed() {
    let route = StartupRoute::parse_args([
        "studio-app",
        "--review-action-id",
        "ra-1",
        "--display-feedback",
        "--chunked_queue_id",
        "cq-1",
        "--show-task-lookup-modal",
    ]);

    assert_eq!(
        route,
        StartupRoute {
            review_action_id: Some("ra-1".to_string()),
            processed_transcription_id: None,
            display_feedback: true,
            show_task_lookup: true,
            chunked_queue_id: Some("cq-1".to_string()),
            stitched_queue_id: None,
        }
    );
}

#[test]
fn every_task_lookup_flag_spelling_opens_task_lookup() {
    for flag in [
        "--task-lookup",
        "--taskLookup",
        "--show-task-lookup",
        "--show-task-lookup-modal",
        "--SHOW_TASK_LOOKUP",
    ] {
        let route = StartupRoute::parse_args([flag]);
        assert!(route.show_task_lookup, "{flag}");
        assert_eq!(route.load(), Some(RouteLoad::OpenTaskLookup), "{flag}");
    }
}

#[test]
fn value_after_pending_key_is_taken_verbatim_even_if_it_looks_like_a_flag() {
    let route = StartupRoute::parse_args(["--reviewActionId", "--chunkedQueueId=cq"]);
    assert_eq!(
        route.review_action_id.as_deref(),
        Some("--chunkedQueueId=cq")
    );
    assert_eq!(route.chunked_queue_id, None);
}

#[test]
fn trailing_key_without_value_is_ignored() {
    let route = StartupRoute::parse_args(["--reviewActionId"]);
    assert!(route.is_empty());
}

#[test]
fn keys_are_case_insensitive_and_ignore_dashes_and_underscores() {
    let route = StartupRoute::parse_args([
        "--Review_Action-Id=ra",
        "--PROCESSED-transcription_ID=pt",
        "--Chunked-Queue_Id=cq",
        "--stitched_queue_id=sq",
    ]);
    assert_eq!(route.review_action_id.as_deref(), Some("ra"));
    assert_eq!(route.processed_transcription_id.as_deref(), Some("pt"));
    assert_eq!(route.chunked_queue_id.as_deref(), Some("cq"));
    assert_eq!(route.stitched_queue_id.as_deref(), Some("sq"));

    assert_eq!(normalize_key("--Review_Action-Id"), "reviewactionid");
    assert_eq!(normalize_key("stitched-TX-queue_id"), "stitchedtxqueueid");
}

#[test]
fn stitched_tx_queue_id_alias_sets_stitched_queue() {
    let route = StartupRoute::parse_args(["--stitched-tx-queue-id=sq-alias"]);
    assert_eq!(route.stitched_queue_id.as_deref(), Some("sq-alias"));

    let query = StartupRoute::parse_query("stitchedTxQueueId=sq-query");
    assert_eq!(query.stitched_queue_id.as_deref(), Some("sq-query"));
}

#[test]
fn query_values_are_percent_and_plus_decoded_as_utf8() {
    let route = StartupRoute::parse_query(
        "reviewActionId=%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82+world&chunkedQueueId=a%2Bb%26c%3Dd",
    );
    assert_eq!(route.review_action_id.as_deref(), Some("привет world"));
    assert_eq!(route.chunked_queue_id.as_deref(), Some("a+b&c=d"));
}

#[test]
fn percent_decoding_accepts_lowercase_hex() {
    let route = StartupRoute::parse_query("reviewActionId=%d0%bf%2f");
    assert_eq!(route.review_action_id.as_deref(), Some("п/"));
}

#[test]
fn malformed_percent_escapes_are_kept_literally() {
    let route = StartupRoute::parse_query("reviewActionId=100%25%zz%4");
    assert_eq!(route.review_action_id.as_deref(), Some("100%%zz%4"));
}

#[test]
fn invalid_utf8_escapes_are_replaced_not_rejected() {
    let route = StartupRoute::parse_query("reviewActionId=ab%FFcd");
    assert_eq!(route.review_action_id.as_deref(), Some("ab\u{FFFD}cd"));
}

#[test]
fn arg_values_are_also_percent_decoded() {
    let route = StartupRoute::parse_args(["--reviewActionId=review%20action+42"]);
    assert_eq!(route.review_action_id.as_deref(), Some("review action 42"));
}

#[test]
fn values_are_trimmed_and_blank_values_unset_ids() {
    let route =
        StartupRoute::parse_query("reviewActionId=+%20+&chunkedQueueId=&stitchedQueueId=%20sq%20");
    assert_eq!(route.review_action_id, None);
    assert_eq!(route.chunked_queue_id, None);
    assert_eq!(route.stitched_queue_id.as_deref(), Some("sq"));

    let args = StartupRoute::parse_args(["--reviewActionId=   ", "--chunkedQueueId", "  "]);
    assert!(args.is_empty());
}

#[test]
fn later_values_override_earlier_ones() {
    let route = StartupRoute::parse_args([
        "--reviewActionId=first",
        "--reviewActionId=second",
        "--chunkedQueueId=cq",
        "--chunkedQueueId=",
    ]);
    assert_eq!(route.review_action_id.as_deref(), Some("second"));
    assert_eq!(route.chunked_queue_id, None);
}

#[test]
fn bool_values_accept_true_spellings_and_reject_others() {
    for value in ["true", "TRUE", "True", "1", "yes", "YES", "on", "On", ""] {
        let route = StartupRoute::parse_query(&format!("displayFeedback={value}"));
        assert!(route.display_feedback, "`{value}` should enable");
    }
    for value in ["false", "0", "no", "off", "maybe"] {
        let route = StartupRoute::parse_query(&format!("displayFeedback={value}"));
        assert!(!route.display_feedback, "`{value}` should not enable");
    }
}

#[test]
fn bare_query_key_is_treated_as_true() {
    let route = StartupRoute::parse_query("displayFeedback&taskLookup");
    assert!(route.display_feedback);
    assert!(route.show_task_lookup);
}

#[test]
fn bool_flag_with_explicit_false_value_disables_it() {
    let route = StartupRoute::parse_args(["--display-feedback=false", "--task-lookup=0"]);
    assert!(!route.display_feedback);
    assert!(!route.show_task_lookup);
}

#[test]
fn parse_query_accepts_optional_leading_question_mark_and_empty_pairs() {
    let with_mark = StartupRoute::parse_query("?reviewActionId=ra&&chunkedQueueId=cq&");
    let without_mark = StartupRoute::parse_query("reviewActionId=ra&chunkedQueueId=cq");
    assert_eq!(with_mark, without_mark);
    assert_eq!(with_mark.review_action_id.as_deref(), Some("ra"));
    assert_eq!(with_mark.chunked_queue_id.as_deref(), Some("cq"));
}

#[test]
fn query_args_merge_with_flag_args() {
    let route = StartupRoute::parse_args([
        "studio-app",
        "--display-feedback",
        "babel://we-edit?reviewActionId=ra",
        "--chunkedQueueId=cq",
    ]);
    assert!(route.display_feedback);
    assert_eq!(route.review_action_id.as_deref(), Some("ra"));
    assert_eq!(route.chunked_queue_id.as_deref(), Some("cq"));
}

#[test]
fn query_route_matches_parse_query() {
    let query = "reviewActionId=ra&displayFeedback=1&stitchedQueueId=sq";
    assert_eq!(
        StartupRoute::parse_args([format!("app://open?{query}")]),
        StartupRoute::parse_query(query)
    );
}

#[test]
fn metadata_entries_cover_every_route_field() {
    let route = StartupRoute {
        review_action_id: Some("ra".to_string()),
        processed_transcription_id: Some("pt".to_string()),
        display_feedback: true,
        show_task_lookup: true,
        chunked_queue_id: Some("cq".to_string()),
        stitched_queue_id: Some("sq".to_string()),
    };
    let mut entries = route.metadata_entries();
    entries.sort();
    let mut expected = vec![
        entry(META_REVIEW_ACTION_ID, "ra"),
        entry(META_TASK_LOOKUP_REVIEW_ACTION_ID, "ra"),
        entry(META_TASK_LOOKUP_PROCESSED_ID, "pt"),
        entry(META_CHUNKED_QUEUE_ID, "cq"),
        entry(META_ACTIVE_QUEUE_ID, "cq"),
        entry(META_STITCHED_QUEUE_ID, "sq"),
        entry(META_DISPLAY_FEEDBACK, "true"),
        entry(META_FEEDBACK_INITIAL_OPEN, "true"),
        entry(META_TASK_LOOKUP_OPEN, "true"),
        entry(META_TASK_LOOKUP_SHOW_STITCHED, "true"),
    ];
    expected.sort();
    assert_eq!(entries, expected);
}

#[test]
fn chunked_queue_id_also_becomes_active_queue_id() {
    let route = StartupRoute::parse_args(["--chunkedQueueId=cq"]);
    assert_eq!(
        route.metadata_entries(),
        vec![
            entry(META_CHUNKED_QUEUE_ID, "cq"),
            entry(META_ACTIVE_QUEUE_ID, "cq"),
        ]
    );
}

#[test]
fn review_action_id_is_mirrored_into_task_lookup_metadata() {
    let route = StartupRoute::parse_args(["--reviewActionId=ra"]);
    assert_eq!(
        route.metadata_entries(),
        vec![
            entry(META_REVIEW_ACTION_ID, "ra"),
            entry(META_TASK_LOOKUP_REVIEW_ACTION_ID, "ra"),
        ]
    );
}

#[test]
fn task_lookup_show_stitched_option_follows_stitched_queue_presence() {
    let without_stitched = StartupRoute::parse_args(["--task-lookup"]);
    assert_eq!(
        without_stitched.metadata_entries(),
        vec![
            entry(META_TASK_LOOKUP_OPEN, "true"),
            entry(META_TASK_LOOKUP_SHOW_STITCHED, "false"),
        ]
    );

    let with_stitched = StartupRoute::parse_args(["--task-lookup", "--stitchedQueueId=sq"]);
    assert_eq!(
        metadata_value(&with_stitched, META_TASK_LOOKUP_SHOW_STITCHED).as_deref(),
        Some("true")
    );
}

#[test]
fn disabled_flags_write_no_metadata() {
    let route = StartupRoute::parse_args(["--reviewActionId=ra"]);
    let keys: Vec<String> = route
        .metadata_entries()
        .into_iter()
        .map(|(key, _)| key)
        .collect();
    assert!(!keys.iter().any(|key| key == META_DISPLAY_FEEDBACK));
    assert!(!keys.iter().any(|key| key == META_FEEDBACK_INITIAL_OPEN));
    assert!(!keys.iter().any(|key| key == META_TASK_LOOKUP_OPEN));
    assert!(!keys.iter().any(|key| key == META_TASK_LOOKUP_SHOW_STITCHED));
}
