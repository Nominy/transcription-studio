use studio_core::Project;

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

pub(crate) fn project_is_read_only(project: &Project) -> bool {
    metadata_bool(project, META_READ_ONLY) || metadata_bool(project, META_ASSESSMENT_READ_ONLY)
}

pub(crate) fn assessment_phase(project: &Project) -> Option<&str> {
    project
        .metadata
        .get(META_ASSESSMENT_PHASE)
        .or_else(|| project.metadata.get(META_DIFF_ASSESSMENT_PHASE))
        .map(String::as_str)
        .filter(|value| !value.is_empty())
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

fn metadata_value_is_truthy(value: &str) -> bool {
    matches!(
        value,
        "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
    )
}
