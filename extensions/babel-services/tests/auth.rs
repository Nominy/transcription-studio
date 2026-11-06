mod common;

use common::{MockResponse, MockServer, contains};
use exaudio_babel_services::account::{
    AccountKind, AccountRecord, AccountStatus, AccountStore, direct_instance_id, discover_records,
    move_providers_root, relay_instance_id,
};
use exaudio_babel_services::auth::{
    self, AuthCookie, AuthStore,
    cli::{AuthCli, AuthCliCommand, parse_args},
    flow::{AuthFlow, AuthStartup, AuthStep},
};
use exaudio_babel_services::subaccounts::{self, SubaccountSession};

const NOW: i64 = 1_782_754_800;

fn cookie(name: &str, value: &str, domain: &str, expires_at: Option<i64>) -> AuthCookie {
    AuthCookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: domain.to_string(),
        path: "/".to_string(),
        secure: false,
        http_only: false,
        same_site: String::new(),
        expires_at,
    }
}

#[test]
fn store_parses_clerk_cookies_and_pending_login() {
    let store = auth::store_from_str(
        r#"{
            "version": 1,
            "cookies": [{
                "name": "__client", "value": "client-cookie",
                "domain": ".Clerk.babel.audio", "path": "/", "expiresAt": 1814289611
            }],
            "pendingSignIn": { "id": "si_native", "phoneNumberId": "idn_phone" }
        }"#,
    )
    .expect("store");

    assert_eq!(store.pending_sign_in_id.as_deref(), Some("si_native"));
    assert_eq!(store.pending_phone_number_id.as_deref(), Some("idn_phone"));
    assert_eq!(
        store.cookies,
        vec![cookie(
            "__client",
            "client-cookie",
            "clerk.babel.audio",
            Some(1_814_289_611_000)
        )]
    );
}

#[test]
fn store_round_trips_through_save() {
    let temp = tempfile::tempdir().expect("temp dir");
    let path = temp.path().join("nested").join("babel-auth.json");
    let store = AuthStore {
        cookies: vec![AuthCookie {
            secure: true,
            http_only: true,
            same_site: "Lax".to_string(),
            ..cookie(
                "__client",
                "c",
                "clerk.babel.audio",
                Some(4_102_444_800_000),
            )
        }],
        pending_sign_in_id: Some("si_1".to_string()),
        pending_phone_number_id: Some("phone_1".to_string()),
    };
    auth::save_store(&path, &store).expect("save");
    assert_eq!(auth::read_store(&path).expect("read"), store);
}

#[test]
fn accounts_in_different_directories_do_not_share_cookies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let first = AccountStore::new(temp.path().join(direct_instance_id("+7 999 000 00 01")));
    let second = AccountStore::new(temp.path().join(direct_instance_id("+7 999 000 00 02")));
    assert_ne!(first.dir(), second.dir());
    let store = AuthStore {
        cookies: vec![cookie(
            "__client",
            "first-cookie",
            "clerk.babel.audio",
            None,
        )],
        ..AuthStore::default()
    };
    auth::save_store(&first.auth_path(), &store).expect("first store");

    assert_eq!(
        first.status("https://clerk.babel.audio", NOW),
        AccountStatus::Ready {
            detail: "Signed in".into()
        }
    );
    assert_eq!(
        second.status("https://clerk.babel.audio", NOW),
        AccountStatus::NeedsSignIn
    );
    assert!(second.client().expect("read").is_none());
    let client = first.client().expect("read").expect("direct client");
    assert_eq!(
        client.config().auth_store.as_deref(),
        Some(first.auth_path().as_path())
    );
    assert_eq!(client.config().data_dir.as_deref(), Some(first.dir()));

    first.clear_credentials().expect("sign out");
    assert_eq!(
        first.status("https://clerk.babel.audio", NOW),
        AccountStatus::NeedsSignIn
    );
}

#[test]
fn expired_or_pending_cookies_need_sign_in() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store = AccountStore::new(temp.path().join("acct"));
    auth::save_store(
        &store.auth_path(),
        &AuthStore {
            cookies: vec![cookie(
                "__client",
                "old",
                "clerk.babel.audio",
                Some(NOW - 1),
            )],
            ..AuthStore::default()
        },
    )
    .expect("expired store");
    assert_eq!(
        store.status("https://clerk.babel.audio", NOW),
        AccountStatus::NeedsSignIn
    );
    auth::save_store(
        &store.auth_path(),
        &AuthStore {
            cookies: vec![cookie("__client", "fresh", "clerk.babel.audio", None)],
            pending_sign_in_id: Some("si_pending".into()),
            ..AuthStore::default()
        },
    )
    .expect("pending store");
    assert_eq!(
        store.status("https://clerk.babel.audio", NOW),
        AccountStatus::NeedsSignIn
    );
}

#[test]
fn relay_session_makes_a_relay_client_and_wins_over_cookies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let session = SubaccountSession {
        host_url: "https://relay.example.com".into(),
        token: "relay-token".into(),
        subaccount_id: "sub-1".into(),
        display_name: "Travel laptop".into(),
    };
    let store = AccountStore::new(
        temp.path()
            .join(relay_instance_id(&session.host_url, &session.subaccount_id)),
    );
    subaccounts::save_session(&store.session_path(), &session).expect("session");
    assert_eq!(
        store.status("https://clerk.babel.audio", NOW),
        AccountStatus::Ready {
            detail: "https://relay.example.com".into()
        }
    );
    let client = store.client().expect("read").expect("relay client");
    assert_eq!(client.config().relay_session.as_ref(), Some(&session));
    assert_eq!(client.config().data_dir.as_deref(), Some(store.dir()));
    store.clear_credentials().expect("sign out");
    assert!(store.client().expect("read").is_none());
}

#[test]
fn records_are_rediscovered_and_the_root_moves_as_a_whole() {
    let temp = tempfile::tempdir().expect("temp dir");
    let root = temp.path().join("providers");
    let provider_dir = root.join("exaudio.babel.account");
    let record = AccountRecord {
        id: direct_instance_id("tester@example.com"),
        label: "tester@example.com".into(),
        kind: AccountKind::Direct,
        identifier: "tester@example.com".into(),
    };
    let store = AccountStore::new(provider_dir.join(&record.id));
    store.save_record(&record).expect("record");
    auth::save_store(
        &store.auth_path(),
        &AuthStore {
            cookies: vec![cookie("__client", "c", "clerk.babel.audio", None)],
            ..AuthStore::default()
        },
    )
    .expect("cookies");
    // A directory whose name does not match its record is ignored.
    AccountStore::new(provider_dir.join("stray"))
        .save_record(&record)
        .expect("stray record");
    assert_eq!(discover_records(&provider_dir), vec![record.clone()]);

    let target = temp.path().join("portable").join("providers");
    move_providers_root(&root, &target).expect("move");
    assert!(!root.exists(), "the source root is moved, not copied");
    let moved = AccountStore::new(target.join("exaudio.babel.account").join(&record.id));
    assert_eq!(moved.read_record().expect("read").as_ref(), Some(&record));
    assert_eq!(
        moved.status("https://clerk.babel.audio", NOW),
        AccountStatus::Ready {
            detail: "Signed in".into()
        }
    );
    assert!(move_providers_root(&target, &target.join("nested")).is_err());
}

#[test]
fn cookie_header_filters_by_domain_path_and_expiry() {
    let store = AuthStore {
        cookies: vec![
            cookie("__client", "client-cookie", "clerk.babel.audio", None),
            cookie("__session", "session-cookie", "dashboard.babel.audio", None),
            cookie("expired", "old", "clerk.babel.audio", Some(10_000)),
        ],
        ..AuthStore::default()
    };
    assert_eq!(
        auth::cookie_header_for_url(&store, "https://clerk.babel.audio/v1/client", 20)
            .expect("header"),
        "__client=client-cookie"
    );
    assert_eq!(
        auth::cookie_header_for_url(
            &store,
            "https://dashboard.babel.audio/api/trpc/worker.get",
            20
        )
        .expect("header"),
        "__session=session-cookie"
    );
}

#[test]
fn babel_backend_cookie_header_matches_session_family() {
    assert_eq!(
        auth::babel_backend_cookie_header("jwt-value", NOW).expect("header"),
        "__session=jwt-value; __session_v1aVht_F=jwt-value; __client_uat=1782754800; __client_uat_v1aVht_F=1782754800"
    );
}

#[test]
fn active_session_id_reads_clerk_client_shapes() {
    assert_eq!(
        auth::active_session_id(&serde_json::json!({
            "response": { "object": "client", "sessions": [{ "id": "sess_native", "status": "active" }] }
        }))
        .as_deref(),
        Some("sess_native")
    );
    assert_eq!(
        auth::active_session_id(&serde_json::json!({
            "response": { "object": "sign_in_attempt", "created_session_id": "sess_created" }
        }))
        .as_deref(),
        Some("sess_created")
    );
}

#[tokio::test]
async fn start_login_stores_pending_sign_in_and_refreshed_cookies() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store_path = temp.path().join("babel-auth.json");
    let clerk = MockServer::start(vec![
        MockResponse::with_headers(
            r#"{"response":{"object":"sign_in_attempt","id":"si_native","supported_first_factors":[{"strategy":"phone_code","phone_number_id":"idn_phone"}]}}"#,
            vec!["set-cookie: __client=start-client; Max-Age=3600; Path=/; SameSite=None; Secure"],
        ),
        MockResponse::with_headers(
            r#"{"response":{"object":"sign_in_attempt","id":"si_native","status":"needs_first_factor"}}"#,
            vec![
                "set-cookie: __client=prepared-client; Max-Age=3600; Path=/; SameSite=None; Secure",
            ],
        ),
    ]);

    let result = auth::start_phone_code_login(
        &store_path,
        "tester@example.com",
        "https://dashboard.babel.audio",
        &clerk.base_url,
        true,
        NOW,
    )
    .await
    .expect("start login");

    assert_eq!(result.sign_in_id, "si_native");
    assert_eq!(result.phone_number_id, "idn_phone");
    let store = auth::read_store(&store_path).expect("store");
    assert_eq!(store.pending_sign_in_id.as_deref(), Some("si_native"));
    assert_eq!(store.pending_phone_number_id.as_deref(), Some("idn_phone"));
    assert_eq!(
        auth::cookie_header_for_url(&store, &format!("{}/v1/client", clerk.base_url), NOW + 1)
            .expect("header"),
        "__client=prepared-client"
    );
    let requests = clerk.join();
    assert!(contains(&requests[0], "POST /v1/client/sign_ins?"));
    assert!(contains(&requests[0], "identifier=tester%40example.com"));
    assert!(contains(
        &requests[1],
        "POST /v1/client/sign_ins/si_native/prepare_first_factor?"
    ));
    assert!(contains(&requests[1], "strategy=phone_code"));
    assert!(contains(&requests[1], "phone_number_id=idn_phone"));
    assert!(contains(&requests[1], "cookie: __client=start-client"));
}

#[tokio::test]
async fn verify_login_completes_session_and_clears_pending_sign_in() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store_path = temp.path().join("babel-auth.json");
    std::fs::write(
        &store_path,
        r#"{
            "version": 1,
            "cookies": [{ "name": "__client", "value": "prepared-client", "domain": "127.0.0.1", "path": "/", "expiresAt": 4102444800000 }],
            "pendingSignIn": { "id": "si_native", "phoneNumberId": "idn_phone" }
        }"#,
    )
    .expect("write store");
    let clerk = MockServer::start(vec![
        MockResponse::with_headers(
            r#"{"response":{"object":"sign_in_attempt","id":"si_native","status":"complete","created_session_id":"sess_native"}}"#,
            vec![
                "set-cookie: __client=verified-client; Max-Age=3600; Path=/; SameSite=None; Secure",
            ],
        ),
        r#"{"object":"token","jwt":"jwt-from-clerk"}"#.into(),
    ]);

    let result = auth::verify_phone_code_login(
        &store_path,
        "123456",
        "https://dashboard.babel.audio",
        &clerk.base_url,
        NOW,
    )
    .await
    .expect("verify login");

    assert_eq!(result.session_id, "sess_native");
    assert_eq!(
        result.cookie,
        "__session=jwt-from-clerk; __session_v1aVht_F=jwt-from-clerk; __client_uat=1782754800; __client_uat_v1aVht_F=1782754800"
    );
    let store = auth::read_store(&store_path).expect("store");
    assert_eq!(store.pending_sign_in_id, None);
    assert_eq!(store.pending_phone_number_id, None);
    let requests = clerk.join();
    assert!(contains(
        &requests[0],
        "POST /v1/client/sign_ins/si_native/attempt_first_factor?"
    ));
    assert!(contains(&requests[0], "code=123456"));
    assert!(contains(
        &requests[1],
        "POST /v1/client/sessions/sess_native/tokens?"
    ));
    assert!(contains(&requests[1], "cookie: __client=verified-client"));
}

#[tokio::test]
async fn mint_authorization_refreshes_cookies_and_reuses_cached_jwt() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store_path = temp.path().join("babel-auth.json");
    std::fs::write(
        &store_path,
        r#"{"version":1,"cookies":[{"name":"__client","value":"client-cookie","domain":"127.0.0.1","path":"/","expiresAt":4102444800000}]}"#,
    )
    .expect("write store");
    // Exactly two responses: a second mint that re-hit Clerk would fail.
    let clerk = MockServer::start(vec![
        MockResponse::with_headers(
            r#"{"response":{"object":"client","last_active_session_id":"sess_native","sessions":[{"id":"sess_native","status":"active"}]}}"#,
            vec![
                "set-cookie: __client=refreshed-client; Max-Age=3600; Path=/; SameSite=None; Secure",
            ],
        ),
        r#"{"object":"token","jwt":"jwt-from-clerk"}"#.into(),
    ]);

    let first = auth::mint_babel_authorization(
        &store_path,
        "https://dashboard.babel.audio",
        &clerk.base_url,
        NOW,
    )
    .await
    .expect("first mint");
    let second = auth::mint_babel_authorization(
        &store_path,
        "https://dashboard.babel.audio",
        &clerk.base_url,
        NOW + 10,
    )
    .await
    .expect("second mint served from cache");

    assert_eq!(first, "Bearer jwt-from-clerk");
    assert_eq!(first, second);
    let requests = clerk.join();
    assert_eq!(requests.len(), 2);
    assert!(contains(&requests[0], "GET /v1/client?"));
    assert!(contains(&requests[0], "cookie: __client=client-cookie"));
    assert!(contains(&requests[1], "cookie: __client=refreshed-client"));
    assert!(contains(&requests[1], "organization_id="));
    let saved = std::fs::read_to_string(&store_path).expect("saved store");
    assert!(saved.contains("\"value\": \"refreshed-client\""));
    assert!(!saved.contains("\"value\": \"client-cookie\""));
}

#[tokio::test]
async fn mint_without_cookies_explains_how_to_log_in() {
    let temp = tempfile::tempdir().expect("temp dir");
    let store_path = temp.path().join("babel-auth.json");
    auth::save_store(&store_path, &AuthStore::default()).expect("save");
    let error = auth::mint_babel_authorization(
        &store_path,
        "https://dashboard.babel.audio",
        "https://clerk.invalid",
        NOW,
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("--babel-auth-start-login"));
}

#[tokio::test]
async fn mint_without_an_auth_store_needs_sign_in() {
    let temp = tempfile::tempdir().expect("temp dir");
    let error = auth::mint_babel_authorization(
        &temp.path().join("babel-auth.json"),
        "https://dashboard.babel.audio",
        "https://clerk.invalid",
        NOW,
    )
    .await
    .unwrap_err();
    assert!(
        error.downcast_ref::<auth::NeedsSignIn>().is_some(),
        "{error:#}"
    );
}

#[test]
fn cli_parses_start_login_with_reset() {
    assert_eq!(
        parse_args([
            "studio-app",
            "--babel-auth-store",
            "C:/accounts/tester/auth.json",
            "--babel-auth-start-login",
            "tester@example.com",
            "--babel-auth-reset",
        ])
        .expect("parse"),
        Some(AuthCli {
            store: "C:/accounts/tester/auth.json".into(),
            command: AuthCliCommand::StartLogin {
                identifier: "tester@example.com".to_string(),
                reset: true,
            }
        })
    );
}

#[test]
fn cli_parses_verify_login_code() {
    assert_eq!(
        parse_args([
            "studio-app",
            "--babel-auth-verify-login=123456",
            "--babel_auth_store=/tmp/auth.json"
        ])
        .expect("parse"),
        Some(AuthCli {
            store: "/tmp/auth.json".into(),
            command: AuthCliCommand::VerifyLogin {
                code: "123456".to_string()
            }
        })
    );
}

#[test]
fn cli_requires_a_store_for_login_commands() {
    assert!(parse_args(["studio-app", "--babel-auth-verify-login=123456"]).is_err());
    assert!(parse_args(["studio-app", "--babel-auth-store=x", "--babel-auth-store=y"]).is_err());
    assert!(parse_args(["studio-app", "--babel-auth-store=/tmp/auth.json"]).is_err());
}

#[test]
fn cli_ignores_regular_args_and_rejects_bad_combinations() {
    assert_eq!(
        parse_args(["studio-app", "--reviewActionId=abc"]).expect("parse"),
        None
    );
    assert!(parse_args(["studio-app", "--babel-auth-start-login"]).is_err());
    assert!(parse_args(["studio-app", "--babel-auth-start-login= "]).is_err());
    assert!(
        parse_args([
            "studio-app",
            "--babel-auth-start-login=a",
            "--babel-auth-verify-login=1"
        ])
        .is_err()
    );
    assert!(
        parse_args([
            "studio-app",
            "--babel-auth-verify-login=1",
            "--babel_auth_verify_login=2"
        ])
        .is_err()
    );
    assert!(exaudio_babel_services::auth::cli::run_from_args(["studio-app", "--other"]).is_none());
}

#[test]
fn startup_without_store_prompts_for_login() {
    assert_eq!(
        AuthFlow::startup_from_store(None),
        AuthStartup::Flow(AuthFlow::login_prompt())
    );
}

#[test]
fn startup_with_pending_sign_in_opens_code_step() {
    let store = AuthStore {
        pending_sign_in_id: Some("si_native".to_string()),
        pending_phone_number_id: Some("phone_native".to_string()),
        ..AuthStore::default()
    };
    let AuthStartup::Flow(flow) = AuthFlow::startup_from_store(Some(&store)) else {
        panic!("pending store should show the auth flow");
    };
    assert_eq!(flow.step, AuthStep::Code);
    assert_eq!(flow.pending_sign_in_id.as_deref(), Some("si_native"));
    assert_eq!(
        flow.pending_phone_number_id.as_deref(),
        Some("phone_native")
    );
    assert!(!flow.can_submit_code());
}

#[test]
fn startup_with_cookies_is_authenticated() {
    let store = AuthStore {
        cookies: vec![cookie("__client", "c", "clerk.babel.audio", None)],
        ..AuthStore::default()
    };
    assert_eq!(
        AuthFlow::startup_from_store(Some(&store)),
        AuthStartup::Authenticated
    );
}

#[test]
fn flow_submit_guards_require_input_and_idle() {
    let mut flow = AuthFlow::pending_code("si".to_string(), None);
    flow.code = " 123456 ".to_string();
    assert!(flow.can_submit_code());
    flow.is_pending = true;
    assert!(!flow.can_submit_code());
    flow.is_pending = false;
    flow.code = "   ".to_string();
    assert!(!flow.can_submit_code());

    let mut flow = AuthFlow::login_prompt();
    assert!(!flow.can_start_login());
    flow.identifier = " user@example.com ".to_string();
    assert!(flow.can_start_login());
    flow.is_pending = true;
    assert!(!flow.can_start_login());

    let mut flow = AuthFlow::subaccount_login_prompt();
    flow.subaccount_host_url = "https://relay.example".to_string();
    flow.subaccount_code = "code".to_string();
    assert!(flow.can_submit_subaccount_login());
    assert!(!flow.can_start_login());
}
