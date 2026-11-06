use studio_core::Project;

use serde_json::Value;

pub(crate) use crate::editor_model::AssessmentPhase;
use exaudio_babel_lint::linter::{LintSeverity, ServerLintIssue, ServerLintSnapshot};

const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_EXTRA_PACKAGE_LINT_ERRORS: &str = "babel.extra.package.lint_errors";
const META_EXTRA_PACKAGE_LINT_ERRORS_CAMEL: &str = "babel.extra.package.lintErrors";
const META_BABEL_PROJECT_ID: &str = "studio.babel.projectId";
const META_BABEL_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
const META_BABEL_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";

pub(crate) const META_READ_ONLY: &str = "studio.babel.readOnly";
pub(crate) const META_ASSESSMENT_READ_ONLY: &str = "studio.babel.assessment.isAssessmentReadOnly";
pub(crate) const META_ASSESSMENT_PHASE: &str = "studio.babel.assessment.phase";
pub(crate) const META_DIFF_ASSESSMENT_PHASE: &str = "studio.babel.diff.assessmentPhase";
pub(crate) const META_ASSESSMENT_RETRY_COUNT: &str = "studio.babel.assessment.retryCount";
pub(crate) const META_ASSESSMENT_IS_TRANSITIONING: &str = "studio.babel.assessment.isTransitioning";
pub(crate) const META_ASSESSMENT_FINAL_SUBMITTED: &str = "studio.babel.assessment.isFinalSubmitted";
pub(crate) const META_DROP_CLAIM_AVAILABLE: &str = "studio.babel.dropClaim.available";
pub(crate) const META_DROP_CLAIM_DROPPING: &str = "studio.babel.dropClaim.isDropping";
pub(crate) const META_DROP_CLAIM_DROPPED: &str = "studio.babel.dropClaim.dropped";

pub fn project_is_read_only(project: &Project) -> bool {
    metadata_bool(project, META_READ_ONLY) || metadata_bool(project, META_ASSESSMENT_READ_ONLY)
}

/// Assessment phase from `studio.babel.assessment.phase`, falling back to the diff-toolbar key.
pub(crate) fn assessment_phase(project: &Project) -> Option<AssessmentPhase> {
    project
        .metadata
        .get(META_ASSESSMENT_PHASE)
        .or_else(|| project.metadata.get(META_DIFF_ASSESSMENT_PHASE))
        .and_then(|value| AssessmentPhase::parse(value))
}

pub(crate) fn metadata_bool(project: &Project, key: &str) -> bool {
    project
        .metadata
        .get(key)
        .is_some_and(|value| metadata_value_is_truthy(value))
}

pub(crate) fn metadata_usize(project: &Project, key: &str) -> usize {
    project
        .metadata
        .get(key)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default()
}

pub(crate) fn project_is_babel_backed(project: &Project) -> bool {
    project.metadata.contains_key(META_BABEL_PROJECT_ID)
        || project.metadata.contains_key(META_BABEL_REVIEW_ACTION_ID)
        || project
            .metadata
            .contains_key(META_BABEL_REVIEW_ACTION_ENVELOPE)
        || imported_server_lint_value(project).is_some()
}

pub fn server_lint_snapshot_from_project(
    project: &Project,
    revision: u64,
) -> Option<ServerLintSnapshot> {
    let lint_value = imported_server_lint_value(project)?;
    let issues = match lint_value {
        Ok(Value::Array(values)) => values
            .into_iter()
            .enumerate()
            .map(|(index, value)| parse_server_lint_issue(index, value))
            .collect(),
        Ok(value) => vec![malformed_server_lint_issue(
            0,
            "lint_errors must be an array",
            &value,
        )],
        Err(detail) => vec![malformed_server_lint_issue(
            0,
            detail.as_str(),
            &Value::Null,
        )],
    };
    Some(ServerLintSnapshot::new(revision, issues))
}

fn imported_server_lint_value(project: &Project) -> Option<Result<Value, String>> {
    for key in [
        META_EXTRA_PACKAGE_LINT_ERRORS,
        META_EXTRA_PACKAGE_LINT_ERRORS_CAMEL,
    ] {
        if let Some(encoded) = project.metadata.get(key) {
            return Some(
                serde_json::from_str(encoded)
                    .map_err(|error| format!("{key} is invalid JSON: {error}")),
            );
        }
    }

    let encoded_package = project.metadata.get(META_EXTRA_PACKAGE)?;
    let package = serde_json::from_str::<Value>(encoded_package).ok()?;
    let package = package.as_object()?;
    package
        .get("lint_errors")
        .or_else(|| package.get("lintErrors"))
        .cloned()
        .map(Ok)
}

fn parse_server_lint_issue(index: usize, value: Value) -> ServerLintIssue {
    let Some(issue) = value.as_object() else {
        return malformed_server_lint_issue(index, "entry must be an object", &value);
    };
    let Some(annotation_id) = issue
        .get("annotationId")
        .or_else(|| issue.get("annotation_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return malformed_server_lint_issue(
            index,
            "entry is missing a non-empty annotationId",
            &value,
        );
    };
    let Some(reason) = issue
        .get("reason")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return malformed_server_lint_issue(index, "entry is missing a non-empty reason", &value);
    };
    let Some(severity) = issue.get("severity").and_then(Value::as_str) else {
        return malformed_server_lint_issue(index, "entry is missing severity", &value);
    };
    let severity = if severity.eq_ignore_ascii_case("error") {
        LintSeverity::Error
    } else if severity.eq_ignore_ascii_case("warning") {
        LintSeverity::Warning
    } else {
        return malformed_server_lint_issue(index, "entry has an unsupported severity", &value);
    };

    ServerLintIssue {
        annotation_id: annotation_id.to_string(),
        reason: reason.to_string(),
        severity,
    }
}

fn malformed_server_lint_issue(index: usize, detail: &str, value: &Value) -> ServerLintIssue {
    ServerLintIssue {
        annotation_id: format!("\0malformed-server-lint-{index}"),
        reason: format!(
            "Malformed Babel server lint issue at index {index}: {detail}; raw={value}"
        ),
        severity: LintSeverity::Error,
    }
}

fn metadata_value_is_truthy(value: &str) -> bool {
    matches!(
        value,
        "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
    )
}
