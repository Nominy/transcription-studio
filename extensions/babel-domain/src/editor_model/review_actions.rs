//! Which review actions the action strip offers for the loaded task.

use studio_core::Project;

use crate::babel_metadata::{
    AssessmentPhase, META_ASSESSMENT_FINAL_SUBMITTED, META_ASSESSMENT_IS_TRANSITIONING,
    META_ASSESSMENT_READ_ONLY, META_ASSESSMENT_RETRY_COUNT, META_DROP_CLAIM_AVAILABLE,
    META_DROP_CLAIM_DROPPING, assessment_phase, metadata_bool, metadata_usize,
    project_is_read_only,
};

use super::EditorSession;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewActionStrip {
    /// The task is editable; `drop_claim` is present when the claim can be released.
    Editable { drop_claim: Option<DropClaimState> },
    ReadOnly {
        notice: Option<ReadOnlyNotice>,
        actions: Vec<ReadOnlyAction>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropClaimState {
    /// A drop request is in flight; the control should be disabled.
    pub is_dropping: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyNotice {
    /// The final assessment was submitted.
    FinalSubmitted,
    /// Assessment session: nothing the user changes persists.
    AssessmentSession,
    /// Plain read-only session: nothing the user changes persists.
    ReadOnlySession,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOnlyAction {
    pub command: ReadOnlyActionCommand,
    pub disabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOnlyActionCommand {
    RetryPractice,
    /// Disabled while the switch to the assessment is in flight.
    TakeAssessment,
    RetryAssessment,
    BackToProjects,
}

impl EditorSession<'_> {
    pub fn project_is_read_only(project: &Project) -> bool {
        project_is_read_only(project)
    }

    pub fn review_action_strip(project: &Project) -> ReviewActionStrip {
        if !project_is_read_only(project) {
            return ReviewActionStrip::Editable {
                drop_claim: metadata_bool(project, META_DROP_CLAIM_AVAILABLE).then(|| {
                    DropClaimState {
                        is_dropping: metadata_bool(project, META_DROP_CLAIM_DROPPING),
                    }
                }),
            };
        }

        if metadata_bool(project, META_ASSESSMENT_FINAL_SUBMITTED) {
            return ReviewActionStrip::ReadOnly {
                notice: Some(ReadOnlyNotice::FinalSubmitted),
                actions: vec![action(ReadOnlyActionCommand::BackToProjects, false)],
            };
        }

        let (notice, actions) = match assessment_phase(project) {
            Some(AssessmentPhase::Practice) => {
                let mut actions = vec![action(ReadOnlyActionCommand::RetryPractice, false)];
                if metadata_usize(project, META_ASSESSMENT_RETRY_COUNT) > 0 {
                    actions.push(action(
                        ReadOnlyActionCommand::TakeAssessment,
                        metadata_bool(project, META_ASSESSMENT_IS_TRANSITIONING),
                    ));
                }
                (None, actions)
            }
            Some(AssessmentPhase::Assessment) => (
                Some(ReadOnlyNotice::AssessmentSession),
                vec![action(ReadOnlyActionCommand::RetryAssessment, false)],
            ),
            None if metadata_bool(project, META_ASSESSMENT_READ_ONLY) => {
                (Some(ReadOnlyNotice::AssessmentSession), Vec::new())
            }
            None => (Some(ReadOnlyNotice::ReadOnlySession), Vec::new()),
        };
        ReviewActionStrip::ReadOnly { notice, actions }
    }
}

fn action(command: ReadOnlyActionCommand, disabled: bool) -> ReadOnlyAction {
    ReadOnlyAction { command, disabled }
}
