use studio_core::Project;

use crate::babel_metadata::{
    META_ASSESSMENT_FINAL_SUBMITTED, META_ASSESSMENT_IS_TRANSITIONING, META_ASSESSMENT_READ_ONLY,
    META_ASSESSMENT_RETRY_COUNT, META_DROP_CLAIM_AVAILABLE, META_DROP_CLAIM_DROPPING,
    assessment_phase, metadata_bool, metadata_usize,
    project_is_read_only as babel_project_is_read_only,
};

use super::EditorSession;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewActionStripView {
    Editable {
        drop_claim: Option<DropClaimControlView>,
    },
    ReadOnly {
        notice: Option<String>,
        actions: Vec<ReadOnlyActionView>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropClaimControlView {
    pub label: String,
    pub disabled: bool,
    pub tooltip: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadOnlyActionView {
    pub label: String,
    pub command: ReadOnlyActionCommand,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyActionCommand {
    RetryPractice,
    TakeAssessment,
    RetryAssessment,
    BackToProjects,
}

impl EditorSession {
    pub fn project_is_read_only(project: &Project) -> bool {
        babel_project_is_read_only(project)
    }

    pub fn review_action_strip(&self, project: &Project) -> ReviewActionStripView {
        let assessment_read_only = metadata_bool(project, META_ASSESSMENT_READ_ONLY);
        if !Self::project_is_read_only(project) {
            return ReviewActionStripView::Editable {
                drop_claim: drop_claim_control(project),
            };
        }

        let phase = assessment_phase(project);
        let is_final_submitted = metadata_bool(project, META_ASSESSMENT_FINAL_SUBMITTED);
        let mut actions = Vec::new();
        let notice = if is_final_submitted {
            actions.push(ReadOnlyActionView {
                label: "Back to Projects".to_string(),
                command: ReadOnlyActionCommand::BackToProjects,
                disabled: false,
            });
            Some(
                "Submitted successfully! Our team will review and get back to you soon."
                    .to_string(),
            )
        } else {
            match phase {
                Some("practice") => {
                    actions.push(ReadOnlyActionView {
                        label: "Retry Practice".to_string(),
                        command: ReadOnlyActionCommand::RetryPractice,
                        disabled: false,
                    });
                    if metadata_usize(project, META_ASSESSMENT_RETRY_COUNT) > 0 {
                        let is_transitioning =
                            metadata_bool(project, META_ASSESSMENT_IS_TRANSITIONING);
                        actions.push(ReadOnlyActionView {
                            label: if is_transitioning {
                                "Loading Assessment..."
                            } else {
                                "Take Assessment"
                            }
                            .to_string(),
                            command: ReadOnlyActionCommand::TakeAssessment,
                            disabled: is_transitioning,
                        });
                    }
                    None
                }
                Some("assessment") => {
                    actions.push(ReadOnlyActionView {
                        label: "Retry".to_string(),
                        command: ReadOnlyActionCommand::RetryAssessment,
                        disabled: false,
                    });
                    Some("No changes will persist during this assessment session.".to_string())
                }
                _ if assessment_read_only => {
                    Some("No changes will persist during this assessment session.".to_string())
                }
                _ => Some("No changes will persist during this read only session.".to_string()),
            }
        };

        ReviewActionStripView::ReadOnly { notice, actions }
    }
}

fn drop_claim_control(project: &Project) -> Option<DropClaimControlView> {
    metadata_bool(project, META_DROP_CLAIM_AVAILABLE).then(|| {
        let is_dropping = metadata_bool(project, META_DROP_CLAIM_DROPPING);
        DropClaimControlView {
            label: if is_dropping {
                "Dropping..."
            } else {
                "Drop Claim"
            }
            .to_string(),
            disabled: is_dropping,
            tooltip: "Drop action for another worker".to_string(),
        }
    })
}
