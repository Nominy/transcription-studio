use bytes::Bytes;
use exaudio_babel_services::review::{
    BackendHttpMethod, ReviewBackend, ReviewBackendRequest, parse_sse_done_payload,
    review_backend_url,
};
use exaudio_gold::polish::AudioTrack;
use serde_json::{Value, json};

#[test]
fn reviewer_session_and_template_routes_encode_ids_and_queries() {
    let parts = ReviewBackendRequest::ReviewSetTemplateMatch {
        session_id: "session/one".to_string(),
        card_id: "card A".to_string(),
        body: json!({ "templateId": "tpl-1" }),
    }
    .to_parts();
    assert_eq!(parts.method, BackendHttpMethod::Post);
    assert_eq!(
        parts.path,
        "/api/review/sessions/session%2Fone/cards/card%20A/template-match"
    );
    assert_eq!(parts.body, Some(json!({ "templateId": "tpl-1" })));

    let search = ReviewBackendRequest::ReviewSearchTemplates {
        query: "tone issue".to_string(),
        limit: Some(7),
    }
    .to_parts();
    assert_eq!(search.method, BackendHttpMethod::Get);
    assert_eq!(
        search.path,
        "/api/review/templates/search?q=tone+issue&limit=7"
    );
    assert_eq!(search.body, None);
}

#[test]
fn every_reviewer_route_matches_the_backend_contract() {
    let s1 = || "s1".to_string();
    let cases = [
        (
            ReviewBackendRequest::BackendHealth,
            BackendHttpMethod::Get,
            "/health",
            false,
        ),
        (
            ReviewBackendRequest::ReviewGenerate(json!({})),
            BackendHttpMethod::Post,
            "/api/review/generate",
            true,
        ),
        (
            ReviewBackendRequest::ReviewCreateSession(json!({})),
            BackendHttpMethod::Post,
            "/api/review/sessions",
            true,
        ),
        (
            ReviewBackendRequest::ReviewGrade(json!({})),
            BackendHttpMethod::Post,
            "/api/review/grade",
            true,
        ),
        (
            ReviewBackendRequest::ReviewUpdateComments {
                session_id: s1(),
                body: json!({ "sessionComment": "ok", "cardComments": {} }),
            },
            BackendHttpMethod::Post,
            "/api/review/sessions/s1/comments",
            true,
        ),
        (
            ReviewBackendRequest::ReviewClearTemplateMatch {
                session_id: s1(),
                card_id: "card".to_string(),
            },
            BackendHttpMethod::Post,
            "/api/review/sessions/s1/cards/card/template-clear",
            false,
        ),
        (
            ReviewBackendRequest::ReviewGenerateTemplateSuggestions { session_id: s1() },
            BackendHttpMethod::Post,
            "/api/review/sessions/s1/template-suggestions",
            false,
        ),
        (
            ReviewBackendRequest::ReviewDecideTemplateSuggestion {
                session_id: s1(),
                proposal_id: "p1".to_string(),
                body: json!({ "decision": "approved" }),
            },
            BackendHttpMethod::Post,
            "/api/review/sessions/s1/template-suggestions/p1/decision",
            true,
        ),
        (
            ReviewBackendRequest::ReviewFinalizeSession {
                session_id: s1(),
                body: json!({ "mode": "apply" }),
            },
            BackendHttpMethod::Post,
            "/api/review/sessions/s1/finalize",
            true,
        ),
        (
            ReviewBackendRequest::ReviewSubmitAnalytics(json!({})),
            BackendHttpMethod::Post,
            "/api/trpc/transcriptions.submitTranscriptReviewAction",
            true,
        ),
    ];
    for (request, method, path, has_body) in cases {
        let parts = request.to_parts();
        assert_eq!(parts.method, method, "{path}");
        assert_eq!(parts.path, path);
        assert_eq!(parts.body.is_some(), has_body, "{path}");
    }
}

#[test]
fn drafting_routes_post_json_or_multipart_payloads() {
    let track = AudioTrack {
        track_id: "lane-1".to_string(),
        file_name: "lane_1.wav".to_string(),
        mime_type: "audio/wav".to_string(),
        metadata: json!({ "speakerKey": "spk-1" }),
        bytes: Bytes::from_static(b"RIFF"),
    };
    let cases = [
        (
            ReviewBackendRequest::DraftGenerate {
                payload: json!({ "jobId": "job" }),
                audio_tracks: Vec::new(),
            },
            "/api/draft/generate",
            false,
        ),
        (
            ReviewBackendRequest::DraftGenerateStream {
                payload: json!({ "jobId": "job" }),
                audio_tracks: vec![track],
            },
            "/api/draft/generate/stream",
            true,
        ),
        (
            ReviewBackendRequest::DraftRedistributeText(json!({ "groups": [] })),
            "/api/broker/redistribute-text",
            false,
        ),
    ];
    for (request, path, stream) in cases {
        let parts = request.to_parts();
        assert_eq!(parts.method, BackendHttpMethod::Post);
        assert_eq!(parts.path, path);
        assert!(parts.body.is_some());
        assert_eq!(request.is_stream(), stream, "{path}");
    }
}

#[test]
fn cloning_audio_requests_shares_the_encoded_payload() {
    let request = ReviewBackendRequest::DraftGenerateStream {
        payload: json!({ "jobId": "job-1" }),
        audio_tracks: vec![AudioTrack {
            track_id: "lane-1".to_string(),
            file_name: "lane-1.wav".to_string(),
            mime_type: "audio/wav".to_string(),
            metadata: Value::Null,
            bytes: Bytes::from(vec![1_u8; 4096]),
        }],
    };
    let original = request.to_parts();
    let cloned = request.clone().to_parts();
    assert_eq!(
        original.audio_tracks[0].bytes.as_ptr(),
        cloned.audio_tracks[0].bytes.as_ptr()
    );
}

#[test]
fn sse_stream_resolves_to_the_done_event_payload() {
    let stream = concat!(
        "event: started\n",
        "data: {\"totalRows\":2}\n",
        "\n",
        "event: row\n",
        "data: {\"row\":{\"rowId\":\"r1\"},\"completedRows\":1,\"totalRows\":2}\n",
        "\n",
        "event: done\n",
        "data: {\"draftRows\":[{\"rowId\":\"r1\",\"rewrittenText\":\"Gold\",\"status\":\"rewritten\",\"warnings\":[]}],\"summary\":{\"totalRows\":1}}\n",
        "\n"
    );
    assert_eq!(
        parse_sse_done_payload(stream).unwrap(),
        "{\"draftRows\":[{\"rowId\":\"r1\",\"rewrittenText\":\"Gold\",\"status\":\"rewritten\",\"warnings\":[]}],\"summary\":{\"totalRows\":1}}"
    );
}

#[test]
fn sse_error_event_or_missing_done_fails() {
    let error = parse_sse_done_payload("event: error\ndata: {\"message\":\"boom\"}\n\n")
        .unwrap_err()
        .to_string();
    assert!(error.contains("boom"));
    assert!(parse_sse_done_payload("event: row\ndata: {}\n\n").is_err());
}

#[test]
fn backend_urls_join_without_double_slashes() {
    assert_eq!(
        review_backend_url("https://reviewgen.ovh/", "/api/review/generate"),
        "https://reviewgen.ovh/api/review/generate"
    );
    assert_eq!(
        ReviewBackend::new(" https://review.example/ ").base_url(),
        "https://review.example"
    );
}
