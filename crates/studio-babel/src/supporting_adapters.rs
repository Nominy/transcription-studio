use std::collections::BTreeMap;

use serde_json::{Map, Value};

pub const PRESIGNED_URL_STALE_TIME_MS: u64 = 600_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalyticsAdapterNames {
    pub debug: &'static str,
    pub gtm: &'static str,
    pub intercom: &'static str,
}

pub const ANALYTICS_ADAPTER_NAMES: AnalyticsAdapterNames = AnalyticsAdapterNames {
    debug: "debug",
    gtm: "gtm",
    intercom: "intercom",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeAdapterPolicy {
    RecoveredRuntimeOnly,
    NativeServiceBoundary,
    NativeStateBoundary,
    NativeStatusSurface,
    NativeAssetEquivalent,
    NativeIconEquivalent,
    BrowserOnlyNoop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveredRuntimeAdapterPolicy {
    pub module_id: &'static str,
    pub recovered_package: &'static str,
    pub native_policy: NativeAdapterPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrowserOnlyAnalyticsPolicy {
    pub adapter_name: &'static str,
    pub browser_global: &'static str,
    pub native_policy: NativeAdapterPolicy,
    pub default_enabled: bool,
}

pub const RECOVERED_RUNTIME_ADAPTER_POLICIES: &[RecoveredRuntimeAdapterPolicy] = &[
    RecoveredRuntimeAdapterPolicy {
        module_id: "33158",
        recovered_package: "react/jsx-runtime",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "37478",
        recovered_package: "react",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "32166",
        recovered_package: "zustand",
        native_policy: NativeAdapterPolicy::NativeStateBoundary,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "62330",
        recovered_package: "clsx",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "76728",
        recovered_package: "tailwind-merge",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "18991",
        recovered_package: "class-variance-authority",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "36460",
        recovered_package: "local cn(clsx, tailwind-merge)",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "19062",
        recovered_package: "next/navigation",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "13924",
        recovered_package: "next/navigation",
        native_policy: NativeAdapterPolicy::RecoveredRuntimeOnly,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "65819",
        recovered_package: "local-trpc-fixture-adapter",
        native_policy: NativeAdapterPolicy::NativeServiceBoundary,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "36915",
        recovered_package: "local-toast-adapter",
        native_policy: NativeAdapterPolicy::NativeStatusSurface,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "57653",
        recovered_package: "local-worker-store-adapter",
        native_policy: NativeAdapterPolicy::NativeStateBoundary,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "8600",
        recovered_package: "local-worker-store-adapter",
        native_policy: NativeAdapterPolicy::NativeStateBoundary,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "90771",
        recovered_package: "local-next-image-adapter",
        native_policy: NativeAdapterPolicy::NativeAssetEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "769",
        recovered_package: "lucide-react/X",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "2437",
        recovered_package: "lucide-react/Circle",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "5170",
        recovered_package: "lucide-react/ChevronDown",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "23142",
        recovered_package: "lucide-react/ArrowUp",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "23365",
        recovered_package: "lucide-react/EyeOff",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "24505",
        recovered_package: "lucide-react/Flag",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "25394",
        recovered_package: "lucide-react/Undo2",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "29471",
        recovered_package: "lucide-react/TriangleAlert",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "30277",
        recovered_package: "lucide-react/Redo2",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "31577",
        recovered_package: "lucide-react/StepBack",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "36717",
        recovered_package: "lucide-react/MessageSquarePlus",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "37971",
        recovered_package: "lucide-react/Trash2",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "39806",
        recovered_package: "lucide-react/StepForward",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "40460",
        recovered_package: "lucide-react/Eye",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "49263",
        recovered_package: "lucide-react/Play",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "52748",
        recovered_package: "lucide-react/ChevronUp",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "66047",
        recovered_package: "lucide-react/Plus",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "80512",
        recovered_package: "lucide-react/Check",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "83993",
        recovered_package: "lucide-react/ArrowLeft",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "88763",
        recovered_package: "lucide-react/Mic",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "88908",
        recovered_package: "lucide-react/ChevronRight",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "89296",
        recovered_package: "lucide-react/MicOff",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "90418",
        recovered_package: "lucide-react/ZoomIn",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "92776",
        recovered_package: "lucide-react/EllipsisVertical",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "94021",
        recovered_package: "lucide-react/ZoomOut",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "95005",
        recovered_package: "lucide-react/SkipForward",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "96576",
        recovered_package: "lucide-react/ArrowDown",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "96645",
        recovered_package: "lucide-react/Pause",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "97578",
        recovered_package: "lucide-react/LoaderCircle",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
    RecoveredRuntimeAdapterPolicy {
        module_id: "97696",
        recovered_package: "lucide-react/Copy",
        native_policy: NativeAdapterPolicy::NativeIconEquivalent,
    },
];

pub const BROWSER_ONLY_ANALYTICS_POLICIES: &[BrowserOnlyAnalyticsPolicy] = &[
    BrowserOnlyAnalyticsPolicy {
        adapter_name: "debug",
        browser_global: "window.console",
        native_policy: NativeAdapterPolicy::BrowserOnlyNoop,
        default_enabled: false,
    },
    BrowserOnlyAnalyticsPolicy {
        adapter_name: "gtm",
        browser_global: "window.dataLayer",
        native_policy: NativeAdapterPolicy::BrowserOnlyNoop,
        default_enabled: false,
    },
    BrowserOnlyAnalyticsPolicy {
        adapter_name: "intercom",
        browser_global: "window.Intercom",
        native_policy: NativeAdapterPolicy::BrowserOnlyNoop,
        default_enabled: false,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresignedUrlQueryOptions {
    pub variables: PresignedUrlQueryVariables,
    pub options: PresignedUrlQueryConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresignedUrlQueryVariables {
    pub s3_urls: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresignedUrlQueryConfig {
    pub enabled: bool,
    pub stale_time_ms: u64,
    pub refetch_on_window_focus: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedPresignedUrlQueryResult {
    pub presigned_urls: BTreeMap<String, String>,
    pub is_loading: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DebugAnalyticsLog {
    pub level: String,
    pub prefix: String,
    pub style: String,
    pub args: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnalyticsValue {
    Json(Value),
    Undefined,
}

pub type GtmPush = BTreeMap<String, AnalyticsValue>;

#[derive(Clone, Debug, PartialEq)]
pub struct IntercomCall {
    pub method: String,
    pub args: Vec<Value>,
}

pub fn build_presigned_url_query_options(s3_urls: Vec<String>) -> PresignedUrlQueryOptions {
    PresignedUrlQueryOptions {
        options: PresignedUrlQueryConfig {
            enabled: !s3_urls.is_empty(),
            stale_time_ms: PRESIGNED_URL_STALE_TIME_MS,
            refetch_on_window_focus: false,
        },
        variables: PresignedUrlQueryVariables { s3_urls },
    }
}

pub fn recovered_runtime_adapter_policies() -> &'static [RecoveredRuntimeAdapterPolicy] {
    RECOVERED_RUNTIME_ADAPTER_POLICIES
}

pub fn browser_only_analytics_policies() -> &'static [BrowserOnlyAnalyticsPolicy] {
    BROWSER_ONLY_ANALYTICS_POLICIES
}

pub fn normalize_presigned_url_query_result(
    data: Option<BTreeMap<String, String>>,
    is_loading: bool,
    error: Option<String>,
) -> NormalizedPresignedUrlQueryResult {
    NormalizedPresignedUrlQueryResult {
        presigned_urls: data.unwrap_or_default(),
        is_loading,
        error,
    }
}

pub fn build_debug_analytics_log(method: &str, args: Vec<Value>) -> DebugAnalyticsLog {
    DebugAnalyticsLog {
        level: "log".to_string(),
        prefix: format!("%c[Analytics:{method}]"),
        style: "color: #8B5CF6; font-weight: bold;".to_string(),
        args,
    }
}

pub fn build_gtm_track_pushes(event: &str, properties: BTreeMap<String, Value>) -> Vec<GtmPush> {
    let mut pushes = Vec::new();
    if properties.contains_key("ecommerce") || properties.contains_key("value") {
        pushes.push(BTreeMap::from([(
            "ecommerce".to_string(),
            AnalyticsValue::Json(Value::Null),
        )]));
    }

    let mut event_push = BTreeMap::from([(
        "event".to_string(),
        AnalyticsValue::Json(Value::String(event.to_string())),
    )]);
    extend_analytics_payload(&mut event_push, properties);
    pushes.push(event_push);
    pushes
}

pub fn build_gtm_identify_push(user_id: &str, traits: BTreeMap<String, Value>) -> GtmPush {
    let mut user_data = object_from_properties(&traits);
    if let Some(email) = traits.get("email") {
        user_data.insert("email_address".to_string(), email.clone());
    }
    if let Some(phone) = traits.get("phone") {
        user_data.insert("phone_number".to_string(), phone.clone());
    }

    BTreeMap::from([
        (
            "event".to_string(),
            AnalyticsValue::Json(Value::String("identify".to_string())),
        ),
        (
            "user_id".to_string(),
            AnalyticsValue::Json(Value::String(user_id.to_string())),
        ),
        (
            "user_data".to_string(),
            AnalyticsValue::Json(Value::Object(user_data)),
        ),
    ])
}

pub fn build_gtm_page_push(page_name: &str, properties: BTreeMap<String, Value>) -> GtmPush {
    let mut page_push = BTreeMap::from([
        (
            "event".to_string(),
            AnalyticsValue::Json(Value::String("page_view".to_string())),
        ),
        (
            "page_name".to_string(),
            AnalyticsValue::Json(Value::String(page_name.to_string())),
        ),
    ]);
    extend_analytics_payload(&mut page_push, properties);
    page_push
}

pub fn build_gtm_reset_push() -> GtmPush {
    BTreeMap::from([
        (
            "event".to_string(),
            AnalyticsValue::Json(Value::String("logout".to_string())),
        ),
        ("user_id".to_string(), AnalyticsValue::Undefined),
        ("user_data".to_string(), AnalyticsValue::Undefined),
    ])
}

pub fn build_intercom_track_call(
    intercom_available: bool,
    event: &str,
    properties: BTreeMap<String, Value>,
) -> Option<IntercomCall> {
    intercom_available.then(|| IntercomCall {
        method: "trackEvent".to_string(),
        args: vec![
            Value::String(event.to_string()),
            Value::Object(object_from_properties(&properties)),
        ],
    })
}

pub fn build_intercom_identify_call(
    intercom_available: bool,
    user_id: &str,
    traits: BTreeMap<String, Value>,
) -> Option<IntercomCall> {
    if !intercom_available {
        return None;
    }

    let mut payload = object_from_properties(&traits);
    payload.insert("user_id".to_string(), Value::String(user_id.to_string()));
    if let Some(email) = traits.get("email") {
        payload.insert("email".to_string(), email.clone());
    }
    if let Some(name) = traits.get("name") {
        payload.insert("name".to_string(), name.clone());
    }
    if let Some(phone) = traits.get("phone") {
        payload.insert("phone".to_string(), phone.clone());
    }
    if let Some(created_at) = traits.get("createdAt") {
        payload.insert("created_at".to_string(), created_at.clone());
    }

    Some(IntercomCall {
        method: "update".to_string(),
        args: vec![Value::Object(payload)],
    })
}

pub fn build_intercom_page_call(
    intercom_available: bool,
    page_name: &str,
    properties: BTreeMap<String, Value>,
) -> Option<IntercomCall> {
    if !intercom_available {
        return None;
    }

    let mut payload = Map::new();
    payload.insert(
        "last_page_viewed".to_string(),
        Value::String(page_name.to_string()),
    );
    payload.extend(object_from_properties(&properties));

    Some(IntercomCall {
        method: "update".to_string(),
        args: vec![Value::Object(payload)],
    })
}

pub fn build_intercom_reset_call(intercom_available: bool) -> Option<IntercomCall> {
    intercom_available.then(|| IntercomCall {
        method: "shutdown".to_string(),
        args: Vec::new(),
    })
}

fn extend_analytics_payload(payload: &mut GtmPush, properties: BTreeMap<String, Value>) {
    for (key, value) in properties {
        payload.insert(key, AnalyticsValue::Json(value));
    }
}

fn object_from_properties(properties: &BTreeMap<String, Value>) -> Map<String, Value> {
    properties
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
