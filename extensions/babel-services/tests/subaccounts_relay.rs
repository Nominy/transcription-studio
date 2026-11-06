//! Sub-account session/settings persistence, relay response parsing, owner dashboard parsing,
//! and relay launcher behavior.

use std::{io::Cursor, path::PathBuf, time::Duration};

use exaudio_babel_services::relay::{
    RelayLaunchConfig, RelayProcess, RelayStatus, read_relay_ready,
    relay_executable_path_for_app_exe, relay_ready_url, wait_for_relay_ready,
};
use exaudio_babel_services::subaccounts::owner::{
    ProjectQueueConfig, parse_dashboard, project_queue_sync_host,
};
use exaudio_babel_services::subaccounts::{
    SubaccountSession, normalize_relay_host_url, parse_login_response, parse_presigned_urls,
    parse_relay_payload, parse_relay_settings, parse_session, read_relay_host_url, read_session,
    save_relay_host_url, save_session,
};

#[test]
fn relay_host_urls_get_a_scheme_and_lose_trailing_slashes() {
    for (input, expected) in [
        ("relay.example.test/", "https://relay.example.test"),
        ("127.0.0.1:8787/", "http://127.0.0.1:8787"),
        ("localhost:8787", "http://localhost:8787"),
        ("[::1]:8787", "http://[::1]:8787"),
        (
            " https://relay.example.test/base/ ",
            "https://relay.example.test/base",
        ),
    ] {
        assert_eq!(
            normalize_relay_host_url(input).unwrap(),
            expected,
            "{input}"
        );
    }
    for invalid in ["", "   ", "ftp://relay.example.test", "not a relay URL"] {
        assert!(normalize_relay_host_url(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn session_store_accepts_aliases_and_round_trips() {
    let session = parse_session(
        r#"{"hostUrl":"http://127.0.0.1:3000/","accessToken":"secret","subaccountId":"sub-1","displayName":"Desk"}"#,
    )
    .unwrap();
    assert_eq!(
        session,
        SubaccountSession {
            host_url: "http://127.0.0.1:3000".to_string(),
            token: "secret".to_string(),
            subaccount_id: "sub-1".to_string(),
            display_name: "Desk".to_string(),
        }
    );
    assert!(parse_session(r#"{"hostUrl":"https://relay.example"}"#).is_err());
    assert!(parse_session(r#"{"token":"t"}"#).is_err());

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested").join("subaccount-session.json");
    assert_eq!(read_session(&path).unwrap(), None);
    save_session(&path, &session).unwrap();
    assert_eq!(read_session(&path).unwrap(), Some(session));
    std::fs::write(&path, "  ").unwrap();
    assert_eq!(read_session(&path).unwrap(), None);
}

#[test]
fn relay_settings_round_trip_the_normalized_host() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("relay-settings.json");
    assert_eq!(read_relay_host_url(&path).unwrap(), None);

    assert_eq!(
        save_relay_host_url(&path, "127.0.0.1:8787/").unwrap(),
        "http://127.0.0.1:8787"
    );
    assert_eq!(
        read_relay_host_url(&path).unwrap().as_deref(),
        Some("http://127.0.0.1:8787")
    );
    assert!(save_relay_host_url(&path, "ftp://x").is_err());
    assert_eq!(
        parse_relay_settings(r#"{"hostUrl":"http://localhost:8787/"}"#).unwrap(),
        "http://localhost:8787"
    );
}

#[test]
fn login_response_reads_wrapped_or_flat_sessions() {
    let wrapped = parse_login_response(
        "https://relay.example",
        r#"{"session":{"sessionToken":"tok","subAccountId":"sub-1","label":"Laptop"}}"#,
    )
    .unwrap();
    assert_eq!(wrapped.host_url, "https://relay.example");
    assert_eq!(wrapped.token, "tok");
    assert_eq!(wrapped.subaccount_id, "sub-1");
    assert_eq!(wrapped.display_name, "Laptop");

    let flat = parse_login_response(
        "https://relay.example",
        r#"{"token":"t2","data":{"id":"x"}}"#,
    )
    .unwrap();
    assert_eq!(flat.token, "t2");
    assert_eq!(flat.subaccount_id, "x");
    assert!(parse_login_response("https://relay.example", r#"{"ok":true}"#).is_err());
}

#[test]
fn relay_payloads_unwrap_envelopes() {
    assert_eq!(parse_relay_payload(r#"{"payload":"[1,2]"}"#), "[1,2]");
    assert_eq!(parse_relay_payload(r#"{"response":{"a":1}}"#), r#"{"a":1}"#);
    assert_eq!(parse_relay_payload(r#"{"data":[true]}"#), "[true]");
    assert_eq!(parse_relay_payload("\u{feff} plain text "), "plain text");
    assert_eq!(parse_relay_payload(r#"{"other":1}"#), r#"{"other":1}"#);
}

#[test]
fn presigned_urls_parse_from_trpc_wrapped_and_encoded_shapes() {
    for body in [
        r#"[{"result":{"data":{"json":{"s3://bucket/audio.wav":"https://signed.example/audio.wav"}}}}]"#,
        r#"{"urls":{"s3://bucket/audio.wav":"https://signed.example/audio.wav","s3://bucket/x.wav":" "}}"#,
        r#"{"payload":"{\"presignedUrls\":{\"s3://bucket/audio.wav\":\"https://signed.example/audio.wav\"}}"}"#,
    ] {
        let urls = parse_presigned_urls(body).unwrap();
        assert_eq!(urls.len(), 1, "{body}");
        assert_eq!(
            urls.get("s3://bucket/audio.wav").map(String::as_str),
            Some("https://signed.example/audio.wav")
        );
    }
    assert!(parse_presigned_urls("[1]").is_err());
}

#[test]
fn owner_dashboard_reads_analytics_and_drops_revoked_rows() {
    let dashboard = parse_dashboard(
        r#"{"subaccounts":[
            {"id":"sub-1","label":"Laptop","code":"CODE1","allowedProjectIds":["p1","p2"],"analytics":{"completedTasks":3,"workedSeconds":4500,"earningsCents":1234}},
            {"id":"sub-revoked","code":"OLD","revoked":true},
            {"id":"sub-is-revoked","isRevoked":true},
            {"id":"sub-disabled","disabled":true},
            {"label":"no id"}
        ]}"#,
    )
    .unwrap();
    assert_eq!(dashboard.subaccounts.len(), 1);
    let subaccount = &dashboard.subaccounts[0];
    assert_eq!(subaccount.code.as_deref(), Some("CODE1"));
    assert_eq!(subaccount.allowed_project_ids, vec!["p1", "p2"]);
    assert_eq!(
        (
            subaccount.completed_tasks,
            subaccount.worked_seconds,
            subaccount.earnings_cents
        ),
        (3, 4500, 1234)
    );

    let wrapped =
        parse_dashboard(r#"{"data":{"items":[{"subaccountId":"s2","tasks":2}]}}"#).unwrap();
    assert_eq!(wrapped.subaccounts[0].label, "s2");
    assert_eq!(wrapped.subaccounts[0].completed_tasks, 2);
    assert!(parse_dashboard("{}").unwrap().subaccounts.is_empty());
}

#[test]
fn project_queues_sync_only_for_owners_with_a_valid_relay() {
    let configs = [ProjectQueueConfig {
        project_id: "08a8456a-c472-5b36-863f-637384cf3a77".to_string(),
        chunked_queue_id: "dg-chunked-queue".to_string(),
        stitched_queue_id: "dg-stitched-queue".to_string(),
    }];
    assert_eq!(
        project_queue_sync_host(false, "https://relay.example.test/", &configs).as_deref(),
        Some("https://relay.example.test")
    );
    assert_eq!(
        project_queue_sync_host(true, "https://relay.example.test", &configs),
        None
    );
    assert_eq!(project_queue_sync_host(false, "", &configs), None);
    assert_eq!(
        project_queue_sync_host(false, "not a relay URL", &configs),
        None
    );
    assert_eq!(
        project_queue_sync_host(false, "https://relay.example.test", &[]),
        None
    );
}

#[test]
fn relay_readiness_is_read_from_stderr() {
    assert_eq!(
        relay_ready_url("studio-relay listening on http://127.0.0.1:8787").as_deref(),
        Some("http://127.0.0.1:8787")
    );
    assert_eq!(relay_ready_url("other log"), None);
    assert_eq!(
        read_relay_ready(Cursor::new(
            "warning before bind\nstudio-relay listening on http://127.0.0.1:3001\n"
        ))
        .unwrap(),
        "http://127.0.0.1:3001"
    );
    let error = read_relay_ready(Cursor::new("bind failed\n")).unwrap_err();
    assert!(error.to_string().contains("bind failed"));
}

#[tokio::test]
async fn relay_ready_future_resolves_or_times_out() {
    let ready = wait_for_relay_ready(
        Cursor::new("studio-relay listening on http://127.0.0.1:9000\n"),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(ready, "http://127.0.0.1:9000");

    struct Blocking;
    impl std::io::Read for Blocking {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(Duration::from_secs(2));
            Ok(0)
        }
    }
    assert!(
        wait_for_relay_ready(Blocking, Duration::from_millis(50))
            .await
            .is_err()
    );
}

#[test]
fn relay_executable_resolves_beside_the_app() {
    let app_exe = PathBuf::from("install").join("studio-app.exe");
    let relay = relay_executable_path_for_app_exe(&app_exe);
    assert_eq!(relay.parent(), app_exe.parent());
    assert_eq!(
        relay.file_name().and_then(|name| name.to_str()),
        Some(if cfg!(windows) {
            "studio-relay.exe"
        } else {
            "studio-relay"
        })
    );
}

#[test]
fn launch_args_prefer_the_babel_command_over_http_upstream() {
    let config = RelayLaunchConfig {
        executable_path: PathBuf::from("studio-relay.exe"),
        bind: "127.0.0.1:8787".to_string(),
        store_path: PathBuf::from("relay-store.json"),
        babel_service_command: Some("node bridge.js".to_string()),
        upstream_url: Some("https://relay-upstream.example/envelope".to_string()),
        task_cents: Some("42".to_string()),
    };
    assert_eq!(
        config.args(),
        [
            "--bind",
            "127.0.0.1:8787",
            "--store",
            "relay-store.json",
            "--babel-service-command",
            "node bridge.js",
            "--task-cents",
            "42",
        ]
    );
    assert_eq!(config.local_url(), "http://127.0.0.1:8787");

    let upstream = RelayLaunchConfig {
        babel_service_command: None,
        task_cents: None,
        ..config
    };
    assert_eq!(
        &upstream.args()[4..],
        ["--upstream-url", "https://relay-upstream.example/envelope"]
    );
}

#[test]
fn relay_start_failure_and_outcomes_drive_status() {
    let dir = tempfile::tempdir().unwrap();
    let mut relay = RelayProcess::default();
    assert!(relay.can_start() && !relay.can_stop());

    let missing = RelayLaunchConfig {
        executable_path: dir.path().join("missing-studio-relay.exe"),
        bind: "127.0.0.1:8787".to_string(),
        store_path: dir.path().join("store").join("relay-store.json"),
        babel_service_command: None,
        upstream_url: None,
        task_cents: None,
    };
    assert!(relay.start(&missing).is_err());
    assert_eq!(relay.status(), RelayStatus::Failed);
    assert!(relay.last_error().is_some());
    assert!(relay.can_start());

    relay.complete_start(&Ok("http://127.0.0.1:8787".to_string()));
    assert_eq!(relay.status(), RelayStatus::Running);
    assert_eq!(relay.url(), "http://127.0.0.1:8787");
    relay.stop().unwrap();
    assert_eq!(relay.status(), RelayStatus::Stopped);
}
