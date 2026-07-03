use std::collections::BTreeMap;

use serde_json::Value;

pub const REVIEW_ACTION_IN_PROGRESS: &str = "in-progress";
pub const BACKGROUND_NOISE_PROJECT_IDS: [&str; 2] = [
    "4c0db8c4-699e-4014-9dbf-27f178c3da8e",
    "7cb04a85-c9f3-4d73-afb9-34156777948f",
];
pub const ARABIC_PROJECT_IDS: [&str; 0] = [];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMode {
    Annotation,
    Transcription,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProjectConfig {
    pub chunked_queue_id: Option<String>,
    pub stitched_tx_queue_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerPermission {
    pub review_queue_id: String,
    pub level: i32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReviewActionPageData {
    pub action_id: String,
    pub action_level: Option<i64>,
    pub action_worker_id: Option<String>,
    pub action_decision: Option<String>,
    pub is_worker_resuming_claim: bool,
    pub transcription_chunk_processed_recordings: Vec<ProcessedRecording>,
    pub processed_recording_uri_map: BTreeMap<String, String>,
    pub annotations: Vec<Value>,
    pub linter_errors: Vec<Value>,
    pub belongs_to_another_worker: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessedRecording {
    pub speaker: Option<i64>,
    pub chunked_processed_recording_id: String,
    pub processed_recording_url: Option<String>,
    pub label: Option<String>,
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageControllerState {
    pub active: bool,
    pub processed_transcription_id: String,
    pub loading_action: bool,
    pub loaded_data: Option<ReviewActionPageData>,
    pub active_queue_id: String,
    pub is_admin: bool,
    pub read_only: bool,
    pub initial_feedback_open: bool,
    pub load_error: Option<String>,
    pub task_lookup_open: bool,
    pub load_guard: bool,
    pub resume_toast_shown: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageControllerEffect {
    LoadReviewAction {
        review_action_id: String,
    },
    LoadLatestByProcessedTranscription {
        processed_transcription_id: String,
        queue_id: String,
    },
    ClaimNext {
        queue_id: String,
    },
    ClaimByProcessedTranscription {
        processed_transcription_id: String,
        queue_id: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageControllerRenderBranch {
    Loader,
    LookupModal,
    Error,
    Workbench,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageControllerInitialLoadInput {
    pub review_action_id: Option<String>,
    pub processed_transcription_id: Option<String>,
    pub display_feedback: bool,
    pub show_task_lookup_modal: bool,
    pub permissions_loading: bool,
    pub active_claim_loading: bool,
    pub worker_id: Option<String>,
    pub project_config: Option<ProjectConfig>,
    pub state: PageControllerState,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageControllerTransition {
    pub state: PageControllerState,
    pub effect: Option<PageControllerEffect>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PageControllerAutoClaimResult {
    Success { data: ReviewActionPageData },
    Failure { error: Option<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkbenchProps {
    pub review_action_id: String,
    pub review_action_level: Option<i64>,
    pub transcription_chunk_processed_recordings: Vec<ProcessedRecording>,
    pub annotations: Vec<Value>,
    pub linter_errors: Vec<Value>,
    pub mode: PageMode,
    pub project_id: String,
    pub project_category: Option<String>,
    pub is_admin: bool,
    pub processed_transcription_id: String,
    pub queue_id: String,
    pub read_only: bool,
    pub is_rtl: bool,
    pub initial_feedback_open: bool,
}

pub fn create_initial_page_controller_state() -> PageControllerState {
    PageControllerState::default()
}

pub fn derive_page_mode(
    project_id: &str,
    is_annotation_project: Option<fn(&str) -> bool>,
) -> PageMode {
    let is_annotation_project = is_annotation_project.unwrap_or(is_background_noise_project);
    if is_annotation_project(project_id) {
        PageMode::Annotation
    } else {
        PageMode::Transcription
    }
}

pub fn derive_is_rtl(project_id: &str, is_rtl_project: Option<fn(&str) -> bool>) -> bool {
    let is_rtl_project = is_rtl_project.unwrap_or(is_arabic_project);
    is_rtl_project(project_id)
}

pub fn resolve_stitched_permission(
    permissions: Option<&[WorkerPermission]>,
    project_id: &str,
    project_configs: &BTreeMap<String, ProjectConfig>,
) -> bool {
    let Some(permissions) = permissions else {
        return false;
    };
    if project_id.is_empty() {
        return false;
    }
    let Some(config) = project_configs.get(project_id) else {
        return false;
    };
    let Some(stitched_queue_id) = config.stitched_tx_queue_id.as_deref() else {
        return false;
    };

    permissions
        .iter()
        .any(|permission| permission.review_queue_id == stitched_queue_id && permission.level >= 1)
}

pub fn compute_initial_load_effect(
    input: PageControllerInitialLoadInput,
) -> PageControllerTransition {
    let mut state = input.state;

    if let (Some(review_action_id), Some(_worker_id), Some(_project_config)) = (
        input.review_action_id.as_ref(),
        input.worker_id.as_ref(),
        input.project_config.as_ref(),
    ) && !state.load_guard
        && !state.active
    {
        state.load_guard = true;
        state.active = true;
        state.loading_action = true;
        return PageControllerTransition {
            state,
            effect: Some(PageControllerEffect::LoadReviewAction {
                review_action_id: review_action_id.clone(),
            }),
        };
    }

    if let (Some(processed_transcription_id), Some(_worker_id), Some(project_config)) = (
        input.processed_transcription_id.as_ref(),
        input.worker_id.as_ref(),
        input.project_config.as_ref(),
    ) && input.review_action_id.is_none()
        && !state.load_guard
        && !state.active
        && let Some(queue_id) = project_config.stitched_tx_queue_id.as_ref()
    {
        state.load_guard = true;
        state.active = true;
        state.loading_action = true;
        return PageControllerTransition {
            state,
            effect: Some(PageControllerEffect::LoadLatestByProcessedTranscription {
                processed_transcription_id: processed_transcription_id.clone(),
                queue_id: queue_id.clone(),
            }),
        };
    }

    if !input.show_task_lookup_modal
        && input.review_action_id.is_none()
        && input.processed_transcription_id.is_none()
        && !input.permissions_loading
        && !input.active_claim_loading
        && !state.active
        && let Some(project_config) = input.project_config.as_ref()
        && let Some(queue_id) = project_config.chunked_queue_id.as_ref()
    {
        state.active = true;
        state.loading_action = true;
        return PageControllerTransition {
            state,
            effect: Some(PageControllerEffect::ClaimNext {
                queue_id: queue_id.clone(),
            }),
        };
    }

    PageControllerTransition {
        state,
        effect: None,
    }
}

pub fn apply_review_action_url_load_success(
    mut state: PageControllerState,
    data: ReviewActionPageData,
    project_config: ProjectConfig,
    worker_id: Option<String>,
    display_feedback: bool,
) -> PageControllerState {
    state.read_only = data.action_worker_id != worker_id
        || data.action_decision.as_deref() != Some(REVIEW_ACTION_IN_PROGRESS);
    state.loaded_data = Some(data);
    state.load_error = None;
    state.active_queue_id = project_config.chunked_queue_id.unwrap_or_default();
    state.is_admin = false;
    state.initial_feedback_open = display_feedback;
    state.loading_action = false;
    state
}

pub fn apply_processed_transcription_url_load_success(
    mut state: PageControllerState,
    data: ReviewActionPageData,
    project_config: ProjectConfig,
    display_feedback: bool,
) -> PageControllerState {
    state.loaded_data = Some(data);
    state.load_error = None;
    state.active_queue_id = project_config.stitched_tx_queue_id.unwrap_or_default();
    state.is_admin = true;
    state.read_only = true;
    state.initial_feedback_open = display_feedback;
    state.loading_action = false;
    state
}

pub fn apply_auto_claim_success(
    mut state: PageControllerState,
    result: PageControllerAutoClaimResult,
    project_config: ProjectConfig,
) -> PageControllerState {
    match result {
        PageControllerAutoClaimResult::Success { data } => {
            state.loaded_data = Some(data);
            state.load_error = None;
            state.active_queue_id = project_config.chunked_queue_id.unwrap_or_default();
            state.is_admin = false;
            state.loading_action = false;
        }
        PageControllerAutoClaimResult::Failure { error } => {
            state.load_error = Some(error.unwrap_or_else(|| "No tasks available".to_string()));
            state.loading_action = false;
        }
    }
    state
}

pub fn choose_task_lookup_read_only_action(
    review_action_id: &str,
    processed_transcription_id: &str,
    project_config: Option<ProjectConfig>,
) -> Option<PageControllerEffect> {
    if !review_action_id.is_empty() {
        return Some(PageControllerEffect::LoadReviewAction {
            review_action_id: review_action_id.to_string(),
        });
    }

    let queue_id = project_config?.stitched_tx_queue_id?;
    if processed_transcription_id.is_empty() {
        return None;
    }

    Some(PageControllerEffect::LoadLatestByProcessedTranscription {
        processed_transcription_id: processed_transcription_id.to_string(),
        queue_id,
    })
}

pub fn build_task_lookup_claim_effect(
    processed_transcription_id: &str,
    project_config: Option<ProjectConfig>,
) -> Option<PageControllerEffect> {
    Some(PageControllerEffect::ClaimByProcessedTranscription {
        processed_transcription_id: processed_transcription_id.to_string(),
        queue_id: project_config?.stitched_tx_queue_id?,
    })
}

pub fn apply_task_lookup_claim_success(
    mut state: PageControllerState,
    data: ReviewActionPageData,
    project_config: ProjectConfig,
) -> PageControllerState {
    if data.belongs_to_another_worker {
        return state;
    }

    state.active = true;
    state.loaded_data = Some(data);
    state.load_error = None;
    state.active_queue_id = project_config.stitched_tx_queue_id.unwrap_or_default();
    state.is_admin = true;
    state.loading_action = false;
    state
}

pub fn map_processed_recordings_for_workbench(
    data: Option<&ReviewActionPageData>,
    presigned_urls: &BTreeMap<String, String>,
) -> Vec<ProcessedRecording> {
    let Some(data) = data else {
        return Vec::new();
    };

    data.transcription_chunk_processed_recordings
        .iter()
        .map(|recording| {
            let source_url = data
                .processed_recording_uri_map
                .get(&recording.chunked_processed_recording_id)
                .map(String::as_str)
                .unwrap_or("");
            let mut recording = recording.clone();
            recording.processed_recording_url =
                Some(presigned_urls.get(source_url).cloned().unwrap_or_default());
            recording.label = Some(match recording.speaker {
                Some(speaker) => format!("Speaker {speaker}"),
                None => "Unknown Speaker".to_string(),
            });
            recording
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
pub fn choose_page_controller_render_branch(
    permissions_loading: bool,
    active_claim_loading: bool,
    show_task_lookup_modal: bool,
    project_config: Option<ProjectConfig>,
    loading_action: bool,
    presigned_loading: bool,
    loaded_data: Option<&ReviewActionPageData>,
    active_queue_id: &str,
    presigned_urls: &BTreeMap<String, String>,
    load_error: Option<&str>,
) -> PageControllerRenderBranch {
    if permissions_loading || active_claim_loading {
        return PageControllerRenderBranch::Loader;
    }

    if !loading_action && show_task_lookup_modal {
        return if project_config.is_some() {
            PageControllerRenderBranch::LookupModal
        } else {
            PageControllerRenderBranch::Loader
        };
    }

    if !loading_action
        && !presigned_loading
        && loaded_data.is_some()
        && !active_queue_id.is_empty()
        && !presigned_urls.is_empty()
    {
        return if load_error.is_some() {
            PageControllerRenderBranch::Error
        } else {
            PageControllerRenderBranch::Workbench
        };
    }

    PageControllerRenderBranch::Loader
}

#[allow(clippy::too_many_arguments)]
pub fn build_workbench_props(
    data: &ReviewActionPageData,
    recordings: Vec<ProcessedRecording>,
    mode: PageMode,
    project_id: &str,
    project_category: Option<String>,
    processed_transcription_id: &str,
    queue_id: &str,
    is_admin: bool,
    read_only: bool,
    is_rtl: bool,
    initial_feedback_open: bool,
) -> WorkbenchProps {
    WorkbenchProps {
        review_action_id: data.action_id.clone(),
        review_action_level: data.action_level,
        transcription_chunk_processed_recordings: recordings,
        annotations: data.annotations.clone(),
        linter_errors: data.linter_errors.clone(),
        mode,
        project_id: project_id.to_string(),
        project_category,
        is_admin,
        processed_transcription_id: processed_transcription_id.to_string(),
        queue_id: queue_id.to_string(),
        read_only,
        is_rtl,
        initial_feedback_open,
    }
}

fn is_background_noise_project(project_id: &str) -> bool {
    BACKGROUND_NOISE_PROJECT_IDS.contains(&project_id)
}

fn is_arabic_project(project_id: &str) -> bool {
    ARABIC_PROJECT_IDS.contains(&project_id)
}
