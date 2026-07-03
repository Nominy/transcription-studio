use std::collections::BTreeMap;

use serde_json::json;
use studio_babel::{
    ANALYTICS_ADAPTER_NAMES, AnalyticsValue, NativeAdapterPolicy, PRESIGNED_URL_STALE_TIME_MS,
    browser_only_analytics_policies, build_debug_analytics_log, build_gtm_identify_push,
    build_gtm_page_push, build_gtm_reset_push, build_gtm_track_pushes,
    build_intercom_identify_call, build_intercom_page_call, build_intercom_reset_call,
    build_intercom_track_call, build_presigned_url_query_options,
    normalize_presigned_url_query_result, recovered_runtime_adapter_policies,
};

#[test]
fn presigned_url_query_options_match_recovered_babel_adapter_contract() {
    let empty = build_presigned_url_query_options(Vec::<String>::new());
    assert!(empty.variables.s3_urls.is_empty());
    assert!(!empty.options.enabled);
    assert_eq!(empty.options.stale_time_ms, PRESIGNED_URL_STALE_TIME_MS);
    assert!(!empty.options.refetch_on_window_focus);

    let populated = build_presigned_url_query_options(vec![
        "s3://bucket/a.wav".to_string(),
        "s3://bucket/b.wav".to_string(),
    ]);
    assert!(populated.options.enabled);
    assert_eq!(
        populated.variables.s3_urls,
        vec!["s3://bucket/a.wav", "s3://bucket/b.wav"]
    );

    let normalized = normalize_presigned_url_query_result(
        Some(BTreeMap::from([(
            "s3://bucket/a.wav".to_string(),
            "https://signed/a.wav".to_string(),
        )])),
        true,
        Some("network".to_string()),
    );
    assert_eq!(
        normalized.presigned_urls["s3://bucket/a.wav"],
        "https://signed/a.wav"
    );
    assert!(normalized.is_loading);
    assert_eq!(normalized.error.as_deref(), Some("network"));

    let empty_normalized = normalize_presigned_url_query_result(None, false, None);
    assert!(empty_normalized.presigned_urls.is_empty());
}

#[test]
fn debug_and_gtm_analytics_payloads_match_recovered_babel_adapter_contract() {
    assert_eq!(ANALYTICS_ADAPTER_NAMES.debug, "debug");
    assert_eq!(ANALYTICS_ADAPTER_NAMES.gtm, "gtm");
    assert_eq!(ANALYTICS_ADAPTER_NAMES.intercom, "intercom");

    let debug = build_debug_analytics_log("track", vec![json!("Signup"), json!({"plan": "pro"})]);
    assert_eq!(debug.level, "log");
    assert_eq!(debug.prefix, "%c[Analytics:track]");
    assert_eq!(debug.style, "color: #8B5CF6; font-weight: bold;");
    assert_eq!(debug.args, vec![json!("Signup"), json!({"plan": "pro"})]);

    let pushes = build_gtm_track_pushes(
        "purchase",
        BTreeMap::from([
            ("value".to_string(), json!(12)),
            ("currency".to_string(), json!("USD")),
        ]),
    );
    assert_eq!(pushes.len(), 2);
    assert_eq!(
        pushes[0].get("ecommerce"),
        Some(&AnalyticsValue::Json(json!(null)))
    );
    assert_eq!(
        pushes[1].get("event"),
        Some(&AnalyticsValue::Json(json!("purchase")))
    );
    assert_eq!(
        pushes[1].get("value"),
        Some(&AnalyticsValue::Json(json!(12)))
    );

    let identify = build_gtm_identify_push(
        "user-1",
        BTreeMap::from([
            ("email".to_string(), json!("a@example.com")),
            ("phone".to_string(), json!("555")),
        ]),
    );
    assert_eq!(
        identify.get("event"),
        Some(&AnalyticsValue::Json(json!("identify")))
    );
    assert_eq!(
        identify.get("user_id"),
        Some(&AnalyticsValue::Json(json!("user-1")))
    );
    let AnalyticsValue::Json(user_data) = identify.get("user_data").unwrap() else {
        panic!("user_data should be JSON object");
    };
    assert_eq!(user_data["email_address"], json!("a@example.com"));
    assert_eq!(user_data["phone_number"], json!("555"));

    let page = build_gtm_page_push(
        "Editor",
        BTreeMap::from([("path".to_string(), json!("/x"))]),
    );
    assert_eq!(
        page.get("event"),
        Some(&AnalyticsValue::Json(json!("page_view")))
    );
    assert_eq!(
        page.get("page_name"),
        Some(&AnalyticsValue::Json(json!("Editor")))
    );
    assert_eq!(page.get("path"), Some(&AnalyticsValue::Json(json!("/x"))));

    let reset = build_gtm_reset_push();
    assert_eq!(
        reset.get("event"),
        Some(&AnalyticsValue::Json(json!("logout")))
    );
    assert_eq!(reset.get("user_id"), Some(&AnalyticsValue::Undefined));
    assert_eq!(reset.get("user_data"), Some(&AnalyticsValue::Undefined));
}

#[test]
fn intercom_lifecycle_calls_match_recovered_babel_adapter_contract() {
    assert_eq!(
        build_intercom_track_call(false, "Viewed", BTreeMap::new()),
        None
    );

    let track = build_intercom_track_call(
        true,
        "Viewed",
        BTreeMap::from([("area".to_string(), json!("editor"))]),
    )
    .expect("available intercom should emit track call");
    assert_eq!(track.method, "trackEvent");
    assert_eq!(track.args, vec![json!("Viewed"), json!({"area": "editor"})]);

    let identify = build_intercom_identify_call(
        true,
        "user-1",
        BTreeMap::from([
            ("email".to_string(), json!("a@example.com")),
            ("name".to_string(), json!("Ada")),
            ("phone".to_string(), json!("555")),
            ("createdAt".to_string(), json!(123)),
        ]),
    )
    .expect("available intercom should emit identify call");
    assert_eq!(identify.method, "update");
    assert_eq!(
        identify.args,
        vec![json!({
            "user_id": "user-1",
            "email": "a@example.com",
            "name": "Ada",
            "phone": "555",
            "createdAt": 123,
            "created_at": 123
        })]
    );

    let page = build_intercom_page_call(
        true,
        "Editor",
        BTreeMap::from([("path".to_string(), json!("/editor"))]),
    )
    .expect("available intercom should emit page call");
    assert_eq!(page.method, "update");
    assert_eq!(
        page.args,
        vec![json!({"last_page_viewed": "Editor", "path": "/editor"})]
    );

    let reset = build_intercom_reset_call(true).expect("available intercom should emit reset call");
    assert_eq!(reset.method, "shutdown");
    assert!(reset.args.is_empty());
}

#[test]
fn native_adapter_policy_inventory_matches_recovered_babel_adapter_contract() {
    let policies = recovered_runtime_adapter_policies();
    assert_eq!(policies.len(), 44);

    let lucide: Vec<_> = policies
        .iter()
        .filter(|policy| policy.recovered_package.starts_with("lucide-react/"))
        .collect();
    assert_eq!(lucide.len(), 30);

    let by_id: BTreeMap<_, _> = policies
        .iter()
        .map(|policy| (policy.module_id, policy))
        .collect();
    assert_eq!(by_id.len(), policies.len(), "adapter ids must be unique");

    assert_eq!(by_id["33158"].recovered_package, "react/jsx-runtime");
    assert_eq!(
        by_id["65819"].native_policy,
        NativeAdapterPolicy::NativeServiceBoundary
    );
    assert_eq!(
        by_id["36915"].native_policy,
        NativeAdapterPolicy::NativeStatusSurface
    );
    assert_eq!(
        by_id["90771"].native_policy,
        NativeAdapterPolicy::NativeAssetEquivalent
    );
    assert_eq!(by_id["97696"].recovered_package, "lucide-react/Copy");
    assert_eq!(
        by_id["97696"].native_policy,
        NativeAdapterPolicy::NativeIconEquivalent
    );
}

#[test]
fn browser_only_analytics_policy_is_explicit_noop_without_native_plugin() {
    let policies = browser_only_analytics_policies();
    assert_eq!(policies.len(), 3);

    let by_name: BTreeMap<_, _> = policies
        .iter()
        .map(|policy| (policy.adapter_name, policy))
        .collect();

    for adapter_name in ["debug", "gtm", "intercom"] {
        let policy = by_name[adapter_name];
        assert_eq!(policy.native_policy, NativeAdapterPolicy::BrowserOnlyNoop);
        assert!(!policy.default_enabled);
        assert!(!policy.browser_global.is_empty());
    }
    assert_eq!(by_name["gtm"].browser_global, "window.dataLayer");
    assert_eq!(by_name["intercom"].browser_global, "window.Intercom");
}
