//! Babel service client: task claim/load, save, submit, skip, drop,
//! admin flag, dashboard projects, reviewer feedback forms and error-category labels.
//!
//! Transport precedence per request: offline fixture
//! (`STUDIO_BABEL_SERVICE_FIXTURE_JSON` / `_PATH`) → external command bridge
//! (`STUDIO_BABEL_SERVICE_COMMAND`) → sub-account relay (active sub-account
//! session) → live Babel tRPC API with native Clerk auth (or the
//! `STUDIO_BABEL_SERVICE_COOKIE` / `_AUTHORIZATION` / `_AUTH_COMMAND` overrides).
//!
//! Stale-response handling is the caller's job: tag each call with the
//! workspace generation / edit revision it was issued for and drop results
//! that no longer match.

mod client;
mod dashboard;
pub mod error_task;
mod payload;
mod request;
mod response;
mod transport;

pub use client::{BabelClient, LoadedErrorTask, LoadedTask, OpenedTask, TaskRequest};
pub use dashboard::{Dashboard, DashboardProject, daily_task_progress, parse_dashboard};
pub use payload::{
    META_ACTIVE_QUEUE_ID, META_CHUNKED_QUEUE_ID, META_DISPLAY_FEEDBACK, META_FEEDBACK_INITIAL_OPEN,
    META_IS_STITCHED_QUEUE, META_PROJECT_ID, META_QUEUE_ID, META_REVIEW_ACTION_ID,
    META_STITCHED_QUEUE_ID, META_TASK_LOOKUP_PROCESSED_ID, META_TASK_LOOKUP_REVIEW_ACTION_ID,
    ROUTE_METADATA_KEYS, RouteContext, SaveAnnotationsRequest, SubmitReviewActionRequest,
    active_queue_id, feedback_project_id, is_stitched_queue, review_action_id,
    route_metadata_from_project, stitched_queue_id,
};
pub use request::BabelRequest;
pub use response::{FeedbackDraftBootstrap, SaveOutcome};
pub(crate) use transport::presigned_chunk_urls_payload;
pub use transport::{BabelConfig, Fixture};

use exaudio_babel_lint::linter::ServerLintIssue;

/// A failed Babel operation. `lint_issues` is set when Babel rejected a
/// save/submit with lint results the editor should still display.
/// `signed_out` is set when the Babel session is missing or expired: the fix is to sign in
/// again, so the UI offers that instead of showing the transport detail.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct BabelError {
    pub message: String,
    pub lint_issues: Option<Vec<ServerLintIssue>>,
    pub signed_out: bool,
}

impl BabelError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            lint_issues: None,
            signed_out: false,
        }
    }
}
