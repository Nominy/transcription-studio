use studio_app::StudioWorkflow;

use super::service_callbacks::{
    BabelServiceRequest, ClaimNextFromQueueRequest, LoadLatestReviewActionRequest,
    LoadReviewActionRequest, META_ACTIVE_QUEUE_ID, META_CHUNKED_QUEUE_ID, META_REVIEW_ACTION_ID,
    META_STITCHED_QUEUE_ID,
};
use super::state::StudioState;

pub(super) const META_DISPLAY_FEEDBACK: &str = "studio.babel.displayFeedback";
pub(super) const META_FEEDBACK_INITIAL_OPEN: &str = "studio.babel.feedback.initialOpen";
pub(super) const META_TASK_LOOKUP_OPEN: &str = "studio.babel.taskLookup.open";
pub(super) const META_TASK_LOOKUP_PROCESSED_ID: &str =
    "studio.babel.taskLookup.processedTranscriptionId";
pub(super) const META_TASK_LOOKUP_REVIEW_ACTION_ID: &str = "studio.babel.taskLookup.reviewActionId";
pub(super) const META_TASK_LOOKUP_SHOW_STITCHED: &str =
    "studio.babel.taskLookup.showStitchedOption";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct StartupRouteConfig {
    pub(super) review_action_id: Option<String>,
    pub(super) processed_transcription_id: Option<String>,
    pub(super) display_feedback: bool,
    pub(super) show_task_lookup_modal: bool,
    pub(super) chunked_queue_id: Option<String>,
    pub(super) stitched_queue_id: Option<String>,
}

impl StartupRouteConfig {
    fn has_route_data(&self) -> bool {
        self.review_action_id.is_some()
            || self.processed_transcription_id.is_some()
            || self.display_feedback
            || self.show_task_lookup_modal
            || self.chunked_queue_id.is_some()
            || self.stitched_queue_id.is_some()
    }
}

pub(super) fn parse_startup_route_config(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> StartupRouteConfig {
    let mut config = StartupRouteConfig::default();
    let mut pending_key: Option<String> = None;

    for arg in args {
        let arg = arg.as_ref();
        if let Some(key) = pending_key.take() {
            set_config_value(&mut config, &key, arg);
            continue;
        }

        if let Some(query_start) = arg.find('?') {
            parse_query_pairs(&mut config, &arg[query_start + 1..]);
            continue;
        }

        let Some(flag) = arg.strip_prefix("--") else {
            continue;
        };
        if let Some((key, value)) = flag.split_once('=') {
            set_config_value(&mut config, key, value);
        } else if is_bool_flag(flag) {
            set_config_value(&mut config, flag, "true");
        } else {
            pending_key = Some(flag.to_string());
        }
    }

    config
}

pub(super) fn apply_startup_route_config(state: &mut StudioState, config: StartupRouteConfig) {
    if !config.has_route_data() {
        return;
    }

    let Some(project) = state.project().cloned() else {
        state.status = "Route unavailable: no project loaded.".to_string();
        return;
    };
    let mut project = project;

    if let Some(review_action_id) = config.review_action_id.as_ref() {
        project
            .metadata
            .insert(META_REVIEW_ACTION_ID.to_string(), review_action_id.clone());
        project.metadata.insert(
            META_TASK_LOOKUP_REVIEW_ACTION_ID.to_string(),
            review_action_id.clone(),
        );
    }
    if let Some(processed_transcription_id) = config.processed_transcription_id.as_ref() {
        project.metadata.insert(
            META_TASK_LOOKUP_PROCESSED_ID.to_string(),
            processed_transcription_id.clone(),
        );
    }
    if let Some(chunked_queue_id) = config.chunked_queue_id.as_ref() {
        project
            .metadata
            .insert(META_CHUNKED_QUEUE_ID.to_string(), chunked_queue_id.clone());
        project
            .metadata
            .insert(META_ACTIVE_QUEUE_ID.to_string(), chunked_queue_id.clone());
    }
    if let Some(stitched_queue_id) = config.stitched_queue_id.as_ref() {
        project.metadata.insert(
            META_STITCHED_QUEUE_ID.to_string(),
            stitched_queue_id.clone(),
        );
    }
    if config.display_feedback {
        project
            .metadata
            .insert(META_DISPLAY_FEEDBACK.to_string(), "true".to_string());
        project
            .metadata
            .insert(META_FEEDBACK_INITIAL_OPEN.to_string(), "true".to_string());
    }
    if config.show_task_lookup_modal {
        project
            .metadata
            .insert(META_TASK_LOOKUP_OPEN.to_string(), "true".to_string());
        project.metadata.insert(
            META_TASK_LOOKUP_SHOW_STITCHED.to_string(),
            config.stitched_queue_id.is_some().to_string(),
        );
    }

    state.babel_service_requests.clear();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Route metadata loaded.".to_string(),
    );

    if config.show_task_lookup_modal {
        state.status = "Task lookup route opened.".to_string();
        return;
    }

    if let Some(review_action_id) = config.review_action_id {
        let token = state.issue_babel_payload_request_token();
        state
            .babel_service_requests
            .push(BabelServiceRequest::LoadReviewAction(
                LoadReviewActionRequest {
                    review_action_id,
                    token,
                },
            ));
        state.status = "Review action route load requested.".to_string();
        return;
    }

    if let (Some(processed_transcription_id), Some(queue_id)) = (
        config.processed_transcription_id,
        config.stitched_queue_id.clone(),
    ) {
        let token = state.issue_babel_payload_request_token();
        state
            .babel_service_requests
            .push(BabelServiceRequest::LoadLatestReviewAction(
                LoadLatestReviewActionRequest {
                    processed_transcription_id,
                    queue_id,
                    token,
                },
            ));
        state.status = "Processed transcription route load requested.".to_string();
        return;
    }

    if let Some(queue_id) = config.chunked_queue_id {
        let token = state.issue_babel_payload_request_token();
        state
            .babel_service_requests
            .push(BabelServiceRequest::ClaimNextFromQueue(
                ClaimNextFromQueueRequest { queue_id, token },
            ));
        state.status = "Default queue claim requested.".to_string();
    }
}

fn parse_query_pairs(config: &mut StartupRouteConfig, query: &str) {
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, "true"));
        set_config_value(config, key, value);
    }
}

fn set_config_value(config: &mut StartupRouteConfig, key: &str, value: &str) {
    let key = normalize_key(key);
    let value = decode_query_component(value).trim().to_string();
    match key.as_str() {
        "reviewactionid" => config.review_action_id = nonempty(value),
        "processedtranscriptionid" => config.processed_transcription_id = nonempty(value),
        "displayfeedback" => config.display_feedback = parse_bool(&value),
        "tasklookup" | "showtasklookup" | "showtasklookupmodal" => {
            config.show_task_lookup_modal = parse_bool(&value)
        }
        "chunkedqueueid" => config.chunked_queue_id = nonempty(value),
        "stitchedqueueid" | "stitchedtxqueueid" => config.stitched_queue_id = nonempty(value),
        _ => {}
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| *character != '-' && *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_bool_flag(flag: &str) -> bool {
    matches!(
        normalize_key(flag).as_str(),
        "displayfeedback" | "tasklookup" | "showtasklookup" | "showtasklookupmodal"
    )
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim(),
        "" | "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
    )
}

fn nonempty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn decode_query_component(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut bytes = value.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        match byte {
            b'+' => decoded.push(' '),
            b'%' => {
                let high = bytes.next();
                let low = bytes.next();
                match (high, low) {
                    (Some(high), Some(low)) => {
                        if let Some(decoded_byte) = decode_hex_pair(high, low) {
                            decoded.push(decoded_byte as char);
                        } else {
                            decoded.push('%');
                            decoded.push(high as char);
                            decoded.push(low as char);
                        }
                    }
                    (Some(high), None) => {
                        decoded.push('%');
                        decoded.push(high as char);
                    }
                    (None, Some(low)) => {
                        decoded.push('%');
                        decoded.push(low as char);
                    }
                    (None, None) => decoded.push('%'),
                }
            }
            _ => decoded.push(byte as char),
        }
    }
    decoded
}

fn decode_hex_pair(high: u8, low: u8) -> Option<u8> {
    Some(from_hex(high)? << 4 | from_hex(low)?)
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
