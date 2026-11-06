use exaudio_babel_domain::editor_model::context_panels::{original_script, script_context};
use exaudio_babel_domain::editor_model::diff_toolbar::{DiffCompareOption, DiffMetrics};
use exaudio_babel_domain::editor_model::feedback_assessment::{
    FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraft, FeedbackInput, FeedbackInputResponse,
    FeedbackRubricEntry, FeedbackSaveRequestInput, compute_feedback_draft_save_request,
    get_rubric_fields_for_flags, has_other_feedback_flag, parse_feedback_responses,
    serialize_feedback_draft,
};
use exaudio_babel_domain::editor_model::review_actions::{
    DropClaimState, ReadOnlyAction, ReadOnlyActionCommand, ReadOnlyNotice, ReviewActionStrip,
};
use exaudio_babel_domain::editor_model::submit_control::{
    AdminFlagPayload, SubmitBlocker, SubmitEligibility, SubmitKind,
};
use exaudio_babel_domain::editor_model::task_lookup::{
    TaskLookupEffect, TaskLookupEvent, TaskLookupState,
};
use exaudio_babel_domain::editor_model::{
    AssessmentPhase, BabelDocumentState, EditorSession as DomainSession, ReviewState,
};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use exaudio_babel_lint::linter::{LintSeverity, ServerLintIssue, ServerLintSnapshot};
use exaudio_extension::document::Severity;
use studio_babel::import_json;
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_editor::editor_model::buffered_playback::{
    BufferedPlaybackDirection, compute_buffered_playback_range,
};
use studio_editor::editor_model::{EditorSession as CoreSession, ProblemCounts, SaveStatus};
use studio_editor::timeline_model::{LaneColor, LaneWaveform, TimeRange, WaveformPeakSource};
use studio_editor::track_filter::TrackFilter;
use studio_editor::transcript_model::{
    AnnotationDiffStatus, TranscriptActionCommand, TranscriptActionMenuEntry,
    TranscriptAnnotationKind, TranscriptDiffCellKind, TranscriptDiffView, TranscriptRowKind,
    TranscriptRowVisibleCells, TranscriptWordDiffStatus,
};
use studio_editor::workflow::StudioWorkflow;
use studio_waveform::DecodedAudio;

fn test_environment(project: &Project) -> exaudio_extension::document::DocumentEnvironment {
    let mut environment =
        exaudio_babel_domain::document_environment(project, project.id.clone(), "exaudio.babel")
            .expect("registered Babel mode");
    environment.providers = exaudio_babel_domain::domain_providers(project);
    exaudio_babel_lint::register(&mut environment.providers);
    exaudio_babel_tags::register(&mut environment.providers);
    environment
}

/// Test composition mirrors the shell: core document state and external review state are distinct.
struct EditorSession {
    core: CoreSession,
    domain: BabelDocumentState,
}
impl std::ops::Deref for EditorSession {
    type Target = CoreSession;
    fn deref(&self) -> &CoreSession {
        &self.core
    }
}
impl std::ops::DerefMut for EditorSession {
    fn deref_mut(&mut self) -> &mut CoreSession {
        &mut self.core
    }
}
impl EditorSession {
    fn for_project(project: &Project) -> Self {
        Self {
            core: CoreSession::for_project_with_environment(project, test_environment(project)),
            domain: BabelDocumentState::for_project(project),
        }
    }
    fn mark_dirty(&mut self) {
        self.domain.bind(&mut self.core).mark_dirty();
    }
    fn submit_eligibility(&self, project: &Project) -> SubmitEligibility {
        self.domain.view(&self.core).submit_eligibility(project)
    }
    fn diff_toolbar(
        &self,
        project: &Project,
    ) -> exaudio_babel_domain::editor_model::diff_toolbar::DiffToolbarState {
        self.domain.view(&self.core).diff_toolbar(project)
    }
    fn can_submit_review(&self, project: &Project) -> bool {
        self.domain.view(&self.core).can_submit_review(project)
    }
    fn submit_review(&mut self, project: &Project) -> Result<(), String> {
        self.domain.bind(&mut self.core).submit_review(project)
    }
    fn toggle_warning_assertion(&mut self, project: &Project) -> bool {
        self.domain
            .bind(&mut self.core)
            .toggle_warning_assertion(project)
    }
    fn server_lint_snapshot(&self) -> Option<&ServerLintSnapshot> {
        self.domain.server_lint_snapshot(&self.core)
    }
    fn set_server_lint_snapshot(
        &mut self,
        project: &Project,
        snapshot: ServerLintSnapshot,
    ) -> bool {
        self.domain
            .bind(&mut self.core)
            .set_server_lint_snapshot(project, snapshot)
    }
}

const STABLE_WORKFLOW_TASK_JSON: &str =
    include_str!("../../../crates/studio-editor/tests/fixtures/stable-workflow-task.json");

fn sample_project() -> Project {
    import_json(STABLE_WORKFLOW_TASK_JSON).expect("stable workflow fixture should import")
}

fn project_with_segments(segments: Vec<Segment>) -> Project {
    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
        speakers: vec![Speaker {
            id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
        }],
        lanes: vec![AudioLane {
            id: "track-1".to_string(),
            speaker_id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
            source_path: "audio/speaker-1.wav".to_string(),
        }],
        segments,
        metadata: Default::default(),
    }
}

fn two_lane_project(mut segments: Vec<Segment>) -> Project {
    for segment in &mut segments {
        if segment.lane_id == "track-2" {
            segment.speaker_id = "speaker-2".to_string();
        }
    }

    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
        speakers: vec![
            Speaker {
                id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
            },
            Speaker {
                id: "speaker-2".to_string(),
                label: "Speaker 2".to_string(),
            },
        ],
        lanes: vec![
            AudioLane {
                id: "track-1".to_string(),
                speaker_id: "speaker-1".to_string(),
                label: "Speaker 1".to_string(),
                source_path: "audio/speaker-1.wav".to_string(),
            },
            AudioLane {
                id: "track-2".to_string(),
                speaker_id: "speaker-2".to_string(),
                label: "Speaker 2".to_string(),
                source_path: "audio/speaker-2.wav".to_string(),
            },
        ],
        segments,
        metadata: Default::default(),
    }
}

fn segment(id: &str, start_seconds: f64, end_seconds: f64, text: &str) -> Segment {
    Segment {
        id: id.to_string(),
        source_annotation_id: Some(id.to_string()),
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds,
        end_seconds,
        text: text.to_string(),
        metadata: Default::default(),
    }
}

fn action_menu_entry_summary(
    entry: &TranscriptActionMenuEntry,
) -> Option<(TranscriptActionCommand, bool)> {
    match *entry {
        TranscriptActionMenuEntry::Item { command, enabled } => Some((command, enabled)),
        TranscriptActionMenuEntry::Separator => None,
    }
}

fn word_diff_summary(diff: &TranscriptDiffView) -> Vec<(TranscriptWordDiffStatus, &str)> {
    diff.words
        .iter()
        .map(|word| (word.status, word.value.as_str()))
        .collect()
}

#[test]
fn editor_session_defaults_to_babel_workspace_state() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    assert_eq!(session.speed, 1.0);
    assert_eq!(session.zoom, 1.0);
    assert!(!session.is_playing);
    assert!(!session.diff_view);
    assert!(session.loop_range().is_none());
    assert_eq!(session.track_filter, TrackFilter::All);
    assert_eq!(session.domain.review_state, ReviewState::Draft);
    assert_eq!(session.selected_segment_id.as_deref(), Some("annotation-1"));
    assert_eq!(session.viewport.start_seconds, 0.0);
    assert!((session.viewport.duration_seconds() - 5.0).abs() < 0.001);
    assert_eq!(session.viewport.end_seconds, 5.0);
}

#[test]
fn editor_session_initializes_review_state_from_babel_action_decision() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    project
        .metadata
        .insert("babel.format".to_string(), "review-action".to_string());
    project.metadata.insert(
        "babel.review_action.actionDecision".to_string(),
        "in-progress".to_string(),
    );

    assert_eq!(
        EditorSession::for_project(&project).domain.review_state,
        ReviewState::InProgress
    );

    project.metadata.insert(
        "babel.review_action.actionDecision".to_string(),
        "pass".to_string(),
    );
    assert_eq!(
        EditorSession::for_project(&project).domain.review_state,
        ReviewState::Submitted
    );
}

#[test]
fn editor_session_support_id_uses_babel_review_action_id() {
    let project = StudioWorkflow::from_babel_json(include_str!(
        "../../../crates/studio-babel/tests/fixtures/review-action-final.json"
    ))
    .expect("review action fixture should import")
    .project()
    .clone();

    let session = EditorSession::for_project(&project);

    assert_eq!(
        session.domain.support_id,
        "e823c3bb-2697-55fc-9272-c083a55874da"
    );
}

#[test]
fn track_filter_options_include_all_tracks_and_each_lane() {
    let project = sample_project();
    let options = CoreSession::track_filter_options(&project);

    assert_eq!(options[0], TrackFilter::All);
    assert_eq!(
        options.iter().map(TrackFilter::label).collect::<Vec<_>>(),
        vec!["All Tracks", "Speaker 1", "Speaker 2"]
    );
}

#[test]
fn action_strip_state_reflects_save_problems_and_review_actions() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.mark_dirty();

    assert_eq!(
        CoreSession::track_filter_options(&project)
            .iter()
            .map(TrackFilter::label)
            .collect::<Vec<_>>(),
        vec!["All Tracks", "Speaker 1", "Speaker 2"]
    );
    assert_eq!(session.save_status(), SaveStatus::Dirty);
    assert_eq!(
        session.submit_eligibility(&project).kind,
        SubmitKind::Review
    );
    session.refresh_diagnostics(&project);
    assert!(!session.problem_counts(&project).is_empty());

    session.mark_saved();
    session.domain.review_state = ReviewState::Submitted;

    assert_eq!(session.save_status(), SaveStatus::Saved);
    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::Editable { drop_claim: None }
    );
}

#[test]
fn save_state_matches_babel_completion_rules() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    let _ = &project;

    assert_eq!(session.save_status(), SaveStatus::Saved);

    session.mark_dirty();

    session.begin_save();
    assert_eq!(session.save_status(), SaveStatus::Saving);

    session.complete_save(false, "remote save timed out");
    assert_eq!(
        session.save_status(),
        SaveStatus::Failed {
            message: Some("remote save timed out".to_string())
        }
    );
    assert!(session.is_dirty);
    assert!(session.last_saved_at.is_none());

    session.begin_save();
    assert!(!session.save_has_error);
    session.complete_save(true, "2026-06-22T12:01:00Z");
    assert_eq!(session.save_status(), SaveStatus::Saved);
    assert!(!session.is_dirty);
    assert_eq!(
        session.last_saved_at.as_deref(),
        Some("2026-06-22T12:01:00Z")
    );
}

#[test]
fn save_completion_only_clears_the_revision_that_was_sent() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.mark_dirty();
    session.begin_save();
    let saved_revision = session.saving_revision().expect("save revision");
    session.mark_dirty();

    assert!(session.complete_save_for_revision(saved_revision, true, "2026-07-09T10:00:00Z"));
    assert!(session.is_dirty);
    assert!(!session.is_saving);
    assert_eq!(session.edit_revision(), saved_revision + 1);
    assert_eq!(
        session.last_saved_at.as_deref(),
        Some("2026-07-09T10:00:00Z")
    );
}

#[test]
fn mismatched_save_completion_cannot_finish_the_active_save() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.mark_dirty();
    session.begin_save();
    let active_revision = session.saving_revision().expect("save revision");

    assert!(!session.complete_save_for_revision(active_revision + 1, true, "stale callback"));
    assert!(session.is_dirty);
    assert!(session.is_saving);
    assert_eq!(session.saving_revision(), Some(active_revision));
    assert!(session.last_saved_at.is_none());
}

#[test]
fn failed_save_without_detail_reports_no_message() {
    let mut session = EditorSession::for_project(&sample_project());

    session.mark_dirty();
    session.begin_save();
    session.complete_save(false, "   ");

    assert_eq!(session.save_status(), SaveStatus::Failed { message: None });
}

#[test]
fn buffered_playback_range_matches_babel_timeout_padding() {
    let before = compute_buffered_playback_range(10.0, 500.0, BufferedPlaybackDirection::Before)
        .expect("finite before range should resolve");

    assert_eq!(before.start_seconds, 9.5);
    assert_eq!(before.end_seconds, 10.0);
    assert_eq!(before.timeout_ms, 700.0);

    let clamped_before =
        compute_buffered_playback_range(0.25, 500.0, BufferedPlaybackDirection::Before)
            .expect("before range should clamp at zero");
    assert_eq!(clamped_before.start_seconds, 0.0);
    assert_eq!(clamped_before.end_seconds, 0.25);
    assert_eq!(clamped_before.timeout_ms, 450.0);

    let after = compute_buffered_playback_range(10.0, -500.0, BufferedPlaybackDirection::After)
        .expect("after range should use absolute buffer length");
    assert_eq!(after.start_seconds, 10.0);
    assert_eq!(after.end_seconds, 10.5);
    assert_eq!(after.timeout_ms, 700.0);

    assert!(
        compute_buffered_playback_range(f64::NAN, 500.0, BufferedPlaybackDirection::After)
            .is_none()
    );
    assert!(
        compute_buffered_playback_range(10.0, f64::INFINITY, BufferedPlaybackDirection::After)
            .is_none()
    );
}

#[test]
fn review_actions_switch_to_read_only_notice() {
    let mut project = sample_project();
    project
        .metadata
        .insert("studio.babel.readOnly".to_string(), "true".to_string());

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::ReadOnly {
            notice: Some(ReadOnlyNotice::ReadOnlySession),
            actions: Vec::new(),
        }
    );
}

#[test]
fn review_actions_expose_drop_claim_when_available() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.dropClaim.available".to_string(),
        "true".to_string(),
    );

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::Editable {
            drop_claim: Some(DropClaimState { is_dropping: false })
        }
    );

    project.metadata.insert(
        "studio.babel.dropClaim.isDropping".to_string(),
        "true".to_string(),
    );

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::Editable {
            drop_claim: Some(DropClaimState { is_dropping: true })
        }
    );
}

#[test]
fn review_actions_offer_practice_retry_and_assessment() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.phase".to_string(),
        "practice".to_string(),
    );

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::ReadOnly {
            notice: None,
            actions: vec![ReadOnlyAction {
                command: ReadOnlyActionCommand::RetryPractice,
                disabled: false,
            }],
        }
    );

    project.metadata.insert(
        "studio.babel.assessment.retryCount".to_string(),
        "1".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.isTransitioning".to_string(),
        "true".to_string(),
    );

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::ReadOnly {
            notice: None,
            actions: vec![
                ReadOnlyAction {
                    command: ReadOnlyActionCommand::RetryPractice,
                    disabled: false,
                },
                ReadOnlyAction {
                    command: ReadOnlyActionCommand::TakeAssessment,
                    disabled: true,
                },
            ],
        }
    );
}

#[test]
fn review_actions_offer_assessment_retry() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.phase".to_string(),
        "assessment".to_string(),
    );

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::ReadOnly {
            notice: Some(ReadOnlyNotice::AssessmentSession),
            actions: vec![ReadOnlyAction {
                command: ReadOnlyActionCommand::RetryAssessment,
                disabled: false,
            }],
        }
    );
}

#[test]
fn review_actions_after_final_submission_only_go_back() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.phase".to_string(),
        "assessment".to_string(),
    );
    project.metadata.insert(
        "studio.babel.assessment.isFinalSubmitted".to_string(),
        "true".to_string(),
    );

    assert_eq!(
        DomainSession::review_action_strip(&project),
        ReviewActionStrip::ReadOnly {
            notice: Some(ReadOnlyNotice::FinalSubmitted),
            actions: vec![ReadOnlyAction {
                command: ReadOnlyActionCommand::BackToProjects,
                disabled: false,
            }],
        }
    );
}

#[test]
fn transcript_rows_mark_read_only_state() {
    let mut project = sample_project();
    project
        .metadata
        .insert("studio.babel.readOnly".to_string(), "true".to_string());
    let session = EditorSession::for_project(&project);

    assert!(
        session
            .transcript_rows(&project)
            .iter()
            .all(|row| row.is_read_only)
    );

    project.metadata.remove("studio.babel.readOnly");
    project.metadata.insert(
        "studio.babel.assessment.isAssessmentReadOnly".to_string(),
        "true".to_string(),
    );

    assert!(
        session
            .transcript_rows(&project)
            .iter()
            .all(|row| row.is_read_only)
    );
}

#[test]
fn transcript_rows_mark_only_the_selected_row() {
    let mut track_two = segment("track-two", 1.0, 2.0, "track two");
    track_two.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("selected-track-one", 0.0, 0.8, "track one"),
        track_two,
    ]);
    let mut session = EditorSession::for_project(&project);
    session.selected_segment_id = Some("selected-track-one".to_string());

    let selected = session
        .transcript_rows(&project)
        .into_iter()
        .filter(|row| row.is_selected)
        .map(|row| row.segment_id)
        .collect::<Vec<_>>();

    assert_eq!(selected, vec!["selected-track-one"]);
}

#[test]
fn problem_counts_report_blocking_lint() {
    let project = project_with_segments(vec![Segment {
        id: "annotation-1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 0.0,
        end_seconds: 1.0,
        text: String::new(),
        metadata: Default::default(),
    }]);
    let session = EditorSession::for_project(&project);

    assert_eq!(
        session.problem_counts(&project),
        ProblemCounts {
            errors: 1,
            warnings: 0
        }
    );
}

#[test]
fn submit_is_allowed_for_clean_review() {
    let project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    let session = EditorSession::for_project(&project);

    assert_eq!(
        session.submit_eligibility(&project),
        SubmitEligibility {
            kind: SubmitKind::Review,
            blocker: None,
        }
    );
}

#[test]
fn submit_is_blocked_by_lint_errors() {
    let project = project_with_segments(vec![segment("empty", 1.0, 2.0, "")]);
    let session = EditorSession::for_project(&project);

    let eligibility = session.submit_eligibility(&project);

    assert_eq!(eligibility.blocker, Some(SubmitBlocker::LintErrors));
    assert!(!eligibility.is_allowed());
}

#[test]
fn submit_kind_follows_assessment_phase() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    project.metadata.insert(
        "studio.babel.diff.assessmentPhase".to_string(),
        "practice".to_string(),
    );
    let session = EditorSession::for_project(&project);
    assert_eq!(
        session.submit_eligibility(&project).kind,
        SubmitKind::Practice
    );

    project.metadata.insert(
        "studio.babel.submit.assessmentPhase".to_string(),
        "assessment".to_string(),
    );
    assert_eq!(
        session.submit_eligibility(&project).kind,
        SubmitKind::FinalAssessment
    );
}

#[test]
fn annotation_submit_is_blocked_by_missing_intensity() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    project.metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    project.metadata.insert(
        "studio.babel.submit.hasMissingIntensity".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    assert_eq!(
        session.submit_eligibility(&project).blocker,
        Some(SubmitBlocker::MissingIntensity)
    );
}

#[test]
fn submit_blockers_follow_babel_priority_chain() {
    let mut annotation_project = project_with_segments(vec![segment("empty", 1.0, 2.0, "")]);
    annotation_project.metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    annotation_project.metadata.insert(
        "studio.babel.submit.hasFeedbackIncomplete".to_string(),
        "true".to_string(),
    );
    let annotation_session = EditorSession::for_project(&annotation_project);

    assert_eq!(
        annotation_session
            .submit_eligibility(&annotation_project)
            .blocker,
        None
    );

    annotation_project.metadata.insert(
        "studio.babel.submit.hasMissingIntensity".to_string(),
        "true".to_string(),
    );
    assert_eq!(
        annotation_session
            .submit_eligibility(&annotation_project)
            .blocker,
        Some(SubmitBlocker::MissingIntensity)
    );

    let warning_project =
        project_with_segments(vec![segment("warning", 1.0, 60.0, "Это (пример).")]);
    let warning_session = EditorSession::for_project(&warning_project);
    assert_eq!(
        warning_session.submit_eligibility(&warning_project).blocker,
        Some(SubmitBlocker::UnacknowledgedWarnings)
    );

    let mut low_confidence = segment("low-confidence", 1.0, 2.0, "Alpha beta.");
    low_confidence.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "word": "beta", "start": 1.2, "end": 1.4, "isYellow": true }
        ]),
    );
    let mut low_confidence_project = project_with_segments(vec![low_confidence]);
    low_confidence_project.metadata.insert(
        "studio.babel.submit.isFeedbackSaving".to_string(),
        "true".to_string(),
    );
    low_confidence_project.metadata.insert(
        "studio.babel.submit.hasFeedbackIncomplete".to_string(),
        "true".to_string(),
    );
    let low_confidence_session = EditorSession::for_project(&low_confidence_project);
    assert_eq!(
        low_confidence_session
            .submit_eligibility(&low_confidence_project)
            .blocker,
        Some(SubmitBlocker::UnresolvedLowConfidence)
    );

    let mut feedback_project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    feedback_project.metadata.insert(
        "studio.babel.submit.isFeedbackSaving".to_string(),
        "true".to_string(),
    );
    feedback_project.metadata.insert(
        "studio.babel.submit.hasFeedbackIncomplete".to_string(),
        "true".to_string(),
    );
    let feedback_session = EditorSession::for_project(&feedback_project);
    assert_eq!(
        feedback_session
            .submit_eligibility(&feedback_project)
            .blocker,
        Some(SubmitBlocker::FeedbackSaving)
    );

    feedback_project
        .metadata
        .remove("studio.babel.submit.isFeedbackSaving");
    assert_eq!(
        feedback_session
            .submit_eligibility(&feedback_project)
            .blocker,
        Some(SubmitBlocker::FeedbackIncomplete)
    );
}

#[test]
fn admin_flag_payload_trims_reason_and_omits_blank() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    assert!(!DomainSession::admin_flag_available(&project));
    project.metadata.insert(
        "studio.babel.adminFlag.available".to_string(),
        "true".to_string(),
    );
    assert!(DomainSession::admin_flag_available(&project));

    assert_eq!(
        DomainSession::admin_flag_payload("  poor acting  "),
        AdminFlagPayload {
            rating: 1,
            rating_reason: Some("poor acting".to_string()),
        }
    );
    assert_eq!(
        DomainSession::admin_flag_payload("   "),
        AdminFlagPayload {
            rating: 1,
            rating_reason: None,
        }
    );
}

#[test]
fn skip_chunk_is_available_but_disallowed_until_backend_allows_it() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    assert!(!DomainSession::skip_chunk_available(&project));
    assert!(DomainSession::skip_chunk_allowed(&project));

    project
        .metadata
        .insert("studio.babel.skip.canSkip".to_string(), "false".to_string());
    assert!(DomainSession::skip_chunk_available(&project));
    assert!(!DomainSession::skip_chunk_allowed(&project));

    project
        .metadata
        .insert("studio.babel.skip.canSkip".to_string(), "true".to_string());
    assert!(DomainSession::skip_chunk_allowed(&project));
}

#[test]
fn diff_toolbar_without_versions_disables_toggle_and_shows_hint() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    let toolbar = session.diff_toolbar(&project);

    assert_eq!(toolbar.assessment_phase, None);
    assert!(!toolbar.annotation_mode);
    assert!(!toolbar.show_reviewers_control);
    assert!(!toolbar.reviewers_checked);
    assert!(!toolbar.is_diff_mode);
    assert!(toolbar.diff_toggle_visible());
    assert!(!toolbar.diff_toggle_enabled());
    assert!(toolbar.no_versions_hint_visible());
    assert!(!toolbar.compare_select_visible());
    assert!(!toolbar.unsaved_changes_badge_visible());
    assert_eq!(toolbar.metrics, None);
}

#[test]
fn diff_toolbar_gates_reviewer_control_like_babel() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.diff.hasReviewerToggle".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let toolbar = session.diff_toolbar(&project);

    assert!(toolbar.show_reviewers_control);
    assert!(toolbar.reviewers_checked);

    project.metadata.insert(
        "studio.babel.diff.assessmentPhase".to_string(),
        "practice".to_string(),
    );
    let toolbar = session.diff_toolbar(&project);

    assert_eq!(toolbar.assessment_phase, Some(AssessmentPhase::Practice));
    assert!(!toolbar.show_reviewers_control);
    assert!(!toolbar.diff_toggle_visible());
    assert!(!toolbar.compare_select_visible());
}

#[test]
fn diff_toolbar_exposes_compare_options_and_transcription_metrics() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.diff.availableReviewActions".to_string(),
        serde_json::json!([
            { "id": "l0", "label": "L0" },
            { "id": "l1", "label": "L1" }
        ])
        .to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.metrics".to_string(),
        serde_json::json!({
            "timestampMetrics": {
                "timeline": { "f1": 0.9, "recall": 0.8, "precision": 0.7 },
                "segments": {
                    "matchedSegments": 3,
                    "totalSegments": 4,
                    "within50ms": 25,
                    "within100ms": 50,
                    "within200ms": 75,
                    "avgShiftMs": 42
                }
            },
            "overallWordErrorRate": 0.12345,
            "totalSubstitutions": 2,
            "totalInsertions": 3,
            "totalDeletions": 4
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();
    session.mark_dirty();

    let toolbar = session.diff_toolbar(&project);

    assert!(toolbar.is_diff_mode);
    assert!(toolbar.diff_toggle_enabled());
    assert!(toolbar.unsaved_changes_badge_visible());
    assert!(toolbar.compare_select_visible());
    assert_eq!(toolbar.selected_compare_id.as_deref(), Some("l0"));
    assert_eq!(
        toolbar.compare_options,
        vec![
            DiffCompareOption {
                id: "l0".to_string(),
                label: "L0".to_string()
            },
            DiffCompareOption {
                id: "l1".to_string(),
                label: "L1".to_string()
            },
        ]
    );

    let Some(DiffMetrics::Transcription(metrics)) = toolbar.metrics else {
        panic!("transcription metrics should be parsed in diff mode");
    };
    assert_eq!(metrics.timestamp.avg_shift_ms, 42);
    assert_eq!(metrics.overall_word_error_rate, 0.12345);
    assert_eq!(metrics.total_substitutions, 2);
    assert_eq!(metrics.total_insertions, 3);
    assert_eq!(metrics.total_deletions, 4);
    assert_eq!(metrics.timestamp.timeline.f1, 0.9);
    assert_eq!(metrics.timestamp.timeline.recall, 0.8);
    assert_eq!(metrics.timestamp.timeline.precision, 0.7);
    assert_eq!(
        (
            metrics.timestamp.matched_segments,
            metrics.timestamp.total_segments
        ),
        (3, 4)
    );
    assert_eq!(
        (
            metrics.timestamp.within_50_ms,
            metrics.timestamp.within_100_ms,
            metrics.timestamp.within_200_ms
        ),
        (25, 50, 75)
    );

    session.toggle_diff_view();
    assert_eq!(session.diff_toolbar(&project).metrics, None);
}

#[test]
fn diff_toolbar_exposes_annotation_options_and_metrics() {
    let mut project = sample_project();
    project.metadata.insert(
        "studio.babel.diff.annotationToolbar".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.availableReviewActions".to_string(),
        serde_json::json!([
            { "id": "l0", "label": "L0", "workerName": "Ira" },
            { "id": "l1", "label": "L1" }
        ])
        .to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.annotationMetrics".to_string(),
        serde_json::json!({
            "eventDetection": { "f1": 0.91, "precision": 0.82, "recall": 0.73 },
            "labelMacroAverage": { "f1": 0.64, "precision": 0.55, "recall": 0.46 },
            "intensityCalibration": { "f1": 0.37, "precision": 0.28, "recall": 0.19 }
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let toolbar = session.diff_toolbar(&project);

    assert!(toolbar.annotation_mode);
    assert_eq!(
        toolbar
            .compare_options
            .iter()
            .map(|option| option.label.as_str())
            .collect::<Vec<_>>(),
        vec!["L0 · Ira", "L1"]
    );
    let Some(DiffMetrics::Annotation(metrics)) = toolbar.metrics else {
        panic!("annotation metrics should be parsed in diff mode");
    };
    assert_eq!(metrics.event_detection.f1, 0.91);
    assert_eq!(metrics.event_detection.precision, 0.82);
    assert_eq!(metrics.event_detection.recall, 0.73);
    assert_eq!(metrics.label_macro_average.f1, 0.64);
    assert_eq!(metrics.intensity_calibration.f1, 0.37);
}

#[test]
fn transcript_rows_respect_selected_track_filter_and_format_times() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.track_filter = TrackFilter::lane("track-speaker-2", "Speaker 2");

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 9);
    assert_eq!(rows[0].display_id, "1");
    assert_eq!(rows[0].speaker_label, "Speaker 2");
    assert_eq!(rows[0].start_time, "00:03.50");
    assert_eq!(rows[0].end_time, "00:05.10");
    assert!(rows[0].lint.is_none());
}

#[test]
fn transcript_rows_expose_babel_inline_track_selector_state() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 2.0, 3.0, "second"),
    ]);
    let session = EditorSession::for_project(&project);

    let row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "second")
        .unwrap();

    assert_eq!(row.selected_lane_id, "track-1");
    assert_eq!(
        row.track_options
            .iter()
            .map(|option| (option.lane_id.as_str(), option.label.as_str()))
            .collect::<Vec<_>>(),
        vec![("track-1", "Speaker 1"), ("track-2", "Speaker 2")]
    );
    assert_eq!(
        row.selected_track_option
            .as_ref()
            .map(|option| (option.lane_id.as_str(), option.label.as_str())),
        Some(("track-1", "Speaker 1"))
    );
}

#[test]
fn transcript_rows_expose_babel_reviewer_intervals_when_enabled() {
    let mut project = project_with_segments(vec![
        segment("first", 1.0, 2.0, "first"),
        segment("second", 5.0, 6.0, "second"),
    ]);
    project.metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.reviewerIntervals".to_string(),
        serde_json::json!([
            {
                "startOffsetInSeconds": 0.0,
                "endOffsetInSeconds": 3.0,
                "lastReviewer": {
                    "workerId": "worker-alpha",
                    "level": 1,
                    "discourseUsername": "alice"
                }
            },
            {
                "startOffsetInSeconds": 3.0,
                "endOffsetInSeconds": 10.0,
                "lastReviewer": {
                    "workerId": "worker-beta",
                    "level": 2
                }
            }
        ])
        .to_string(),
    );
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert!(rows.iter().all(|row| row.show_reviewer));
    assert_eq!(rows[0].reviewer.as_ref().unwrap().label, "L1: alice");
    assert_eq!(rows[0].reviewer.as_ref().unwrap().color_hue, Some(49));
    assert_eq!(rows[1].reviewer.as_ref().unwrap().label, "L2: \u{2014}");
    assert_eq!(rows[1].reviewer.as_ref().unwrap().color_hue, Some(287));
}

#[test]
fn transcript_rows_hide_reviewer_intervals_when_babel_toggle_is_off() {
    let mut project = project_with_segments(vec![segment("first", 1.0, 2.0, "first")]);
    project.metadata.insert(
        "studio.babel.diff.reviewerIntervals".to_string(),
        serde_json::json!([
            {
                "startOffsetInSeconds": 0.0,
                "endOffsetInSeconds": 3.0,
                "lastReviewer": {
                    "workerId": "worker-alpha",
                    "level": 1,
                    "discourseUsername": "alice"
                }
            }
        ])
        .to_string(),
    );
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert!(rows.iter().all(|row| !row.show_reviewer));
    assert!(rows.iter().all(|row| row.reviewer.is_none()));
}

#[test]
fn transcript_rows_expose_babel_hidden_timestamp_visible_cells() {
    let mut project = project_with_segments(vec![segment("first", 1.0, 2.0, "first")]);
    project.metadata.insert(
        "studio.babel.table.hideTimestamps".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.showReviewers".to_string(),
        "true".to_string(),
    );
    let session = EditorSession::for_project(&project);

    let row = session.transcript_rows(&project).remove(0);

    assert_eq!(
        row.visible_cells,
        TranscriptRowVisibleCells {
            id: false,
            track: false,
            start: false,
            end: false,
            reviewer: true,
            text: true,
            linter: true,
            actions: false,
        }
    );
    assert_eq!(
        session.transcript_visible_cells(&project),
        row.visible_cells
    );
}

#[test]
fn timeline_lanes_include_deterministic_waveform_blocks_and_colors() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    let lanes = session.timeline_lanes(&project);

    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0].label, "Speaker 1");
    assert_eq!(lanes[0].color, LaneColor::Blue);
    assert_eq!(lanes[0].blocks.len(), 1);
    assert_eq!(lanes[1].label, "Speaker 2");
    assert_eq!(lanes[1].color, LaneColor::Purple);
    assert_eq!(lanes[1].blocks.len(), 1);
    assert_eq!(lanes.iter().map(|lane| lane.blocks.len()).sum::<usize>(), 2);
    assert!(lanes.iter().all(|lane| lane.scrollbar.is_visible));
    assert!(lanes.iter().all(|lane| lane.scrollbar.left_percent == 0.0));
    assert!(
        lanes
            .iter()
            .all(|lane| (lane.scrollbar.width_percent - 4.016).abs() < 0.01)
    );
    let speaker_one = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    assert!(speaker_one.blocks[0].left_percent > 0.0);
    assert!(speaker_one.blocks[0].width_percent > 0.0);
    assert_eq!(speaker_one.blocks[0].peaks.len(), 256);
    assert_eq!(
        speaker_one.blocks[0].peak_source,
        WaveformPeakSource::Synthetic
    );
    assert_eq!(speaker_one.blocks[0].peaks, speaker_one.blocks[0].peaks);
}

#[test]
fn timeline_scrollbar_drag_pans_babel_scroll_window_without_moving_playhead() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.set_zoom(&project, 2.0);
    let playhead = session.playhead_seconds;
    let before = session.viewport;

    session.drag_timeline_scrollbar_thumb(&project, 0.45, 0.0);

    assert_eq!(session.playhead_seconds, playhead);
    assert!((session.viewport.duration_seconds() - before.duration_seconds()).abs() < 0.001);
    assert!(session.viewport.start_seconds > before.start_seconds);
    assert_eq!(
        session.timeline_lanes(&project)[0].scrollbar.left_percent,
        45.0
    );
}

#[test]
fn timeline_lanes_expose_babel_track_control_state() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    let lanes = session.timeline_lanes(&project);
    let speaker_one = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    assert!(!speaker_one.is_collapsed);
    assert!(!speaker_one.is_muted);
    assert_eq!(speaker_one.vertical_zoom, 20.0);
    assert_eq!(speaker_one.track_height, 165.0);

    session.toggle_lane_collapse("track-speaker-1");
    session.toggle_lane_mute("track-speaker-2");
    session.set_lane_vertical_zoom("track-speaker-2", 7.5);

    let lanes = session.timeline_lanes(&project);
    let speaker_one = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap();
    let speaker_two = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap();
    assert!(speaker_one.is_collapsed);
    assert_eq!(speaker_one.track_height, 52.0);
    assert!(speaker_two.is_muted);
    assert_eq!(speaker_two.vertical_zoom, 7.5);
}

#[test]
fn one_second_overlay_opens_on_the_selected_rows_lane_and_clamps_to_the_timeline() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.seek_to_time(&project, 12.25);
    let selected_lane = session
        .selected_segment_id
        .as_deref()
        .and_then(|id| project.segment(id))
        .map(|segment| segment.lane_id.clone())
        .expect("a selected row");

    session.toggle_overlay(&project);

    let overlay = session.overlay.clone().expect("T opens the overlay");
    assert_eq!(overlay.lane_id, selected_lane);
    assert_eq!(overlay.range.start_seconds, 12.25);
    assert_eq!(overlay.range.end_seconds, 13.25);
    assert_eq!(session.overlay_on(&selected_lane), Some(overlay.range));

    let bounds = TimeRange::for_project(&project);
    session.move_overlay_to_time(&project, bounds.end_seconds - 0.2);

    let overlay = session
        .overlay
        .clone()
        .expect("overlay state should still be stored outside the visible viewport");
    assert_eq!(overlay.lane_id, selected_lane);
    assert_eq!(overlay.range.end_seconds, bounds.end_seconds);
    assert!((overlay.range.end_seconds - overlay.range.start_seconds - 1.0).abs() < 0.001);

    session.toggle_overlay(&project);

    assert!(session.overlay.is_none());
}

#[test]
fn lane_vertical_zoom_is_clamped_to_waveform_scale_unlock_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.set_lane_vertical_zoom("track-speaker-1", 0.1);
    assert_eq!(
        session
            .timeline_lanes(&project)
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .vertical_zoom,
        0.3
    );

    session.set_lane_vertical_zoom("track-speaker-1", 50.0);
    assert_eq!(
        session
            .timeline_lanes(&project)
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .vertical_zoom,
        50.0
    );

    session.set_lane_vertical_zoom("track-speaker-1", 1_500.0);
    assert_eq!(
        session
            .timeline_lanes(&project)
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .vertical_zoom,
        1_000.0
    );
}

#[test]
fn timeline_lanes_use_loaded_audio_peaks_for_matching_lane_and_fallback_for_missing_lane() {
    let mut other_lane = segment("other-lane", 0.0, 1.0, "other");
    other_lane.lane_id = "track-2".to_string();
    other_lane.speaker_id = "speaker-2".to_string();
    let project = two_lane_project(vec![segment("audio-backed", 0.0, 1.0, "audio"), other_lane]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(8_000, vec![vec![0.5; 8_000]]).expect("valid audio");

    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let lanes = session.timeline_lanes(&project);
    let audio_block = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .unwrap()
        .blocks[0]
        .clone();
    let fallback_block = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-2")
        .unwrap()
        .blocks[0]
        .clone();

    assert_eq!(audio_block.peak_source, WaveformPeakSource::Audio);
    assert!(
        audio_block
            .peaks
            .iter()
            .all(|peak| (*peak - 0.5).abs() < f32::EPSILON)
    );
    assert_eq!(fallback_block.peak_source, WaveformPeakSource::Synthetic);
}

#[test]
fn timeline_lanes_expose_the_lane_line_waveform_with_segment_blocks() {
    let project = two_lane_project(vec![segment("audio-backed", 0.0, 1.0, "audio")]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(16_000, vec![vec![0.25; 16_000]]).expect("valid audio");

    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let lanes = session.timeline_lanes(&project);
    let lane = lanes.iter().find(|lane| lane.lane_id == "track-1").unwrap();
    let waveform = lane
        .waveform
        .as_ref()
        .expect("audio-backed lane should expose its waveform");

    assert_eq!(waveform.duration_seconds(), 1.0);
    assert_eq!(waveform.signal().samples().len(), 8_000);
    assert_eq!(lane.blocks.len(), 1);
    assert_eq!(lane.blocks[0].segment_id, "audio-backed");
    assert_eq!(lane.blocks[0].peak_source, WaveformPeakSource::Audio);
    let missing = lanes.iter().find(|lane| lane.lane_id == "track-2").unwrap();
    assert!(missing.waveform.is_none());
}

#[test]
fn timeline_rebuilds_share_the_lane_waveform_and_its_canvas_cache() {
    let project = two_lane_project(vec![segment("audio-backed", 0.0, 1.0, "audio")]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(8_000, vec![vec![0.25; 8_000]]).expect("valid audio");
    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let waveform_of = |session: &EditorSession| {
        session
            .timeline_lanes(&project)
            .into_iter()
            .find(|lane| lane.lane_id == "track-1")
            .and_then(|lane| lane.waveform)
            .expect("audio-backed lane should expose its waveform")
    };
    let first = waveform_of(&session);
    session.playhead_seconds = 0.5;
    let second = waveform_of(&session);

    assert!(std::sync::Arc::ptr_eq(&first, &second));
}

#[test]
fn lane_waveform_reuses_segment_peak_cache_across_timeline_rebuilds() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 2.0, 3.0, "second"),
    ]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(10, vec![vec![0.0; 40]]).expect("valid audio");

    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));

    let first_lanes = session.timeline_lanes(&project);
    let first_audio_blocks = first_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("audio lane should render")
        .blocks
        .iter()
        .filter(|block| block.peak_source == WaveformPeakSource::Audio)
        .count();
    let stats_after_first = session
        .lane_waveforms
        .get("track-1")
        .expect("audio waveform should remain attached")
        .segment_peak_cache_stats();

    assert_eq!(first_audio_blocks, 2);
    assert_eq!(stats_after_first.misses, 2);
    assert_eq!(stats_after_first.hits, 0);

    let second_lanes = session.timeline_lanes(&project);
    let second_audio_blocks = second_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("audio lane should render")
        .blocks
        .iter()
        .filter(|block| block.peak_source == WaveformPeakSource::Audio)
        .count();
    let stats_after_second = session
        .lane_waveforms
        .get("track-1")
        .expect("audio waveform should remain attached")
        .segment_peak_cache_stats();

    assert_eq!(second_audio_blocks, first_audio_blocks);
    assert_eq!(stats_after_second.misses, stats_after_first.misses);
    assert_eq!(
        stats_after_second.hits, stats_after_first.hits,
        "unchanged timeline block rebuilds should reuse static waveform blocks before consulting the segment peak cache"
    );
}

#[test]
fn lane_waveform_reuses_overview_render_cache_across_playhead_only_changes() {
    let audio = DecodedAudio::new(4, vec![vec![0.25, -0.5, 0.75, -1.0]]).expect("valid audio");
    let waveform = LaneWaveform::from_decoded(audio);

    let first = waveform
        .overview_render_model(0.25)
        .expect("first overview should build");
    let second = waveform
        .overview_render_model(0.75)
        .expect("second overview should reuse cached peaks");
    let stats = waveform.render_cache_stats();

    assert_eq!(stats.misses, 1);
    assert_eq!(stats.hits, 1);
    assert!(std::sync::Arc::ptr_eq(&first.channels, &second.channels));
    assert!(std::sync::Arc::ptr_eq(
        &first.channels[0].peaks,
        &second.channels[0].peaks
    ));
    assert_ne!(
        first.cursor.progress_percent,
        second.cursor.progress_percent
    );
}

#[test]
fn lane_waveform_can_use_precomputed_peaks_without_decoded_audio() {
    let project = two_lane_project(vec![segment("peak-backed", 0.0, 1.0, "peaks")]);
    let mut session = EditorSession::for_project(&project);
    let waveform = LaneWaveform::from_precomputed_peaks(vec![vec![0.0, 0.5, -1.0, 0.25]], 1.0)
        .expect("valid peak source");

    session.set_lane_waveform("track-1", waveform);

    let lanes = session.timeline_lanes(&project);
    let lane = lanes.iter().find(|lane| lane.lane_id == "track-1").unwrap();
    let waveform = lane.waveform.as_ref().unwrap();

    assert_eq!(lane.blocks[0].peak_source, WaveformPeakSource::Audio);
    assert_eq!(lane.blocks[0].segment_id, "peak-backed");
    assert_eq!(waveform.signal().samples(), &[0.0, 0.5, -1.0, 0.25]);
    assert_eq!(waveform.duration_seconds(), 1.0);
}

#[test]
fn overview_waveform_uses_loaded_audio_and_reports_viewport_window() {
    let project = project_with_segments(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 3.0, 4.0, "second"),
    ]);
    let mut session = EditorSession::for_project(&project);
    let audio = DecodedAudio::new(1, vec![vec![0.1, 0.4, 0.8, 0.2]]).expect("valid audio");
    session.set_lane_waveform("track-1", LaneWaveform::from_decoded(audio));
    session.playhead_seconds = 2.0;

    session.set_zoom(&project, 2.0);
    let overview = session.overview_waveform(&project);
    let model = overview
        .waveform_model
        .as_ref()
        .expect("audio-backed overview should expose a Wavesurfer render model");

    assert_eq!(overview.peak_source, WaveformPeakSource::Audio);
    assert_eq!(model.visible_start_seconds, 0.0);
    assert_eq!(model.visible_end_seconds, 4.0);
    assert_eq!(model.channels[0].peaks.len(), 1_024);
    assert!((overview.viewport_left_percent - 18.75).abs() < 0.001);
    assert!((overview.viewport_width_percent - 62.5).abs() < 0.001);
    assert!((overview.playhead_percent - 50.0).abs() < 0.001);
}

#[test]
fn overview_waveform_exposes_all_visible_lane_audio_models() {
    let mut second_lane_segment = segment("second-lane", 1.0, 2.0, "second");
    second_lane_segment.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("first-lane", 0.0, 1.0, "first"),
        second_lane_segment,
    ]);
    let mut session = EditorSession::for_project(&project);

    session.set_lane_waveform(
        "track-1",
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.1, 0.2, 0.3, 0.4]]).expect("valid audio"),
        ),
    );
    session.set_lane_waveform(
        "track-2",
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.4, 0.3, 0.2, 0.1]]).expect("valid audio"),
        ),
    );

    let overview = session.overview_waveform(&project);

    assert_eq!(overview.peak_source, WaveformPeakSource::Audio);
    assert_eq!(
        overview
            .lane_waveform_models
            .iter()
            .map(|lane| lane.lane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["track-1", "track-2"]
    );
    assert_eq!(
        overview
            .lane_waveform_models
            .iter()
            .map(|lane| lane.color)
            .collect::<Vec<_>>(),
        vec![LaneColor::Blue, LaneColor::Purple],
        "overview lane waveforms should use the same colors as the timeline lanes"
    );

    session.track_filter = TrackFilter::lane("track-2", "Speaker 2");
    let filtered_overview = session.overview_waveform(&project);

    assert_eq!(
        filtered_overview
            .lane_waveform_models
            .iter()
            .map(|lane| lane.lane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["track-2"]
    );
    assert_eq!(
        filtered_overview.lane_waveform_models[0].color,
        LaneColor::Purple,
        "lane colors must stay stable when the track filter hides other lanes"
    );
}

#[test]
fn overview_segment_blocks_use_timeline_lane_color_order() {
    let mut second_lane_segment = segment("second-lane", 1.0, 2.0, "second");
    second_lane_segment.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("first-lane", 0.0, 1.0, "first"),
        second_lane_segment,
    ]);
    let mut session = EditorSession::for_project(&project);

    let overview = session.overview_waveform(&project);
    assert_eq!(
        overview
            .blocks
            .iter()
            .map(|block| block.color)
            .collect::<Vec<_>>(),
        vec![LaneColor::Blue, LaneColor::Purple]
    );
    assert_eq!(
        overview
            .blocks
            .iter()
            .map(|block| block.lane_row)
            .collect::<Vec<_>>(),
        vec![0, 1],
        "each lane's segment blocks should land in that lane's own overview row"
    );
    assert_eq!(overview.block_lane_rows, 2);

    session.track_filter = TrackFilter::lane("track-2", "Speaker 2");
    let filtered_overview = session.overview_waveform(&project);
    assert_eq!(
        filtered_overview
            .blocks
            .iter()
            .map(|block| block.color)
            .collect::<Vec<_>>(),
        vec![LaneColor::Purple]
    );
    assert_eq!(
        filtered_overview
            .blocks
            .iter()
            .map(|block| block.lane_row)
            .collect::<Vec<_>>(),
        vec![0],
        "filtering to one lane should collapse the overview to a single block row"
    );
    assert_eq!(filtered_overview.block_lane_rows, 1);
}

#[test]
fn overview_viewport_start_fraction_pans_window_without_moving_playhead() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-17");
    let playhead = session.playhead_seconds;

    session.set_zoom(&project, 4.0);
    let visible_duration = session.viewport.duration_seconds();
    session.set_overview_viewport_start_fraction(&project, 0.5);

    let bounds = TimeRange::for_project(&project);
    let expected_start = bounds.start_seconds + bounds.duration_seconds() * 0.5;
    assert_eq!(session.playhead_seconds, playhead);
    assert!((session.viewport.duration_seconds() - visible_duration).abs() < 0.001);
    assert!((session.viewport.start_seconds - expected_start).abs() < 0.001);

    session.set_overview_viewport_start_fraction(&project, 1.0);

    assert!((session.viewport.end_seconds - bounds.end_seconds).abs() < 0.001);
}

#[test]
fn every_lint_error_in_a_row_counts_as_a_blocker() {
    let project = project_with_segments(vec![segment("many-issues", 1.0, 2.0, " alpha  ** 😀 ")]);
    let mut session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);
    let errors = rows[0]
        .lint_issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();

    assert!(errors > 1);
    assert_eq!(rows[0].lint.as_ref(), rows[0].lint_issues.first());
    assert_eq!(session.blocking_lint_count(&project), errors);
    assert_eq!(session.problem_counts(&project).errors, errors);
    assert!(session.submit_review(&project).is_err());
}

#[test]
fn non_transcription_babel_annotations_skip_transcription_lint_and_actions() {
    let mut annotation = segment("non-transcription", 1.0, 2.0, "");
    annotation
        .metadata
        .insert("type".to_string(), serde_json::json!("non-transcription"));
    let project = project_with_segments(vec![annotation]);
    let session = EditorSession::for_project(&project);

    let row = session.transcript_rows(&project).remove(0);

    assert_eq!(
        row.annotation_kind,
        TranscriptAnnotationKind::NonTranscription("non-transcription".to_string())
    );
    assert_eq!(row.annotation_type_label, "non-transcription");
    assert!(row.lint.is_none());
    assert!(row.lint_issues.is_empty());

    let menu = CoreSession::transcript_action_menu(&row);
    assert_eq!(
        menu.iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            Some((TranscriptActionCommand::Edit, true)),
            None,
            Some((TranscriptActionCommand::MergeAbove, false)),
            Some((TranscriptActionCommand::MergeBelow, false)),
            None,
            Some((TranscriptActionCommand::Delete, true)),
        ]
    );
}

#[test]
fn submit_review_is_blocked_by_lint_errors_and_marks_clean_project_submitted() {
    let project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    let mut session = EditorSession::for_project(&project);
    session.mark_dirty();
    session.mark_saved();

    session
        .submit_review(&project)
        .expect("clean project should submit");
    assert_eq!(session.domain.review_state, ReviewState::Submitted);

    let bad_project = project_with_segments(vec![segment("bad", 1.0, 2.0, "")]);
    let mut bad_session = EditorSession::for_project(&bad_project);
    bad_session
        .submit_review(&bad_project)
        .expect_err("lint errors should block submit");
    assert_eq!(bad_session.domain.review_state, ReviewState::Draft);
}

#[test]
fn submit_review_checks_hidden_lanes_even_when_filter_is_clean() {
    let mut hidden_bad = segment("annotation-2", 2.0, 3.0, "");
    hidden_bad.lane_id = "track-2".to_string();
    let project = two_lane_project(vec![
        segment("annotation-1", 0.0, 1.0, "Visible clean."),
        hidden_bad,
    ]);
    let mut session = EditorSession::for_project(&project);
    session.track_filter = TrackFilter::lane("track-1", "Speaker 1");

    assert_eq!(session.transcript_rows(&project).len(), 1);
    assert_eq!(session.blocking_lint_count(&project), 1);
    assert!(session.submit_review(&project).is_err());
}

#[test]
fn submit_review_blocks_unacknowledged_warnings_until_asserted() {
    let project = project_with_segments(vec![segment("warning", 1.0, 60.0, "Это (пример).")]);
    let mut session = EditorSession::for_project(&project);

    session
        .submit_review(&project)
        .expect_err("Babel blocks submit while warnings are unacknowledged");
    assert_eq!(session.domain.review_state, ReviewState::Draft);

    session.select_segment(&project, "warning");
    assert!(session.toggle_warning_assertion(&project));
    session
        .submit_review(&project)
        .expect("asserted warnings should no longer block submit");
    assert_eq!(session.domain.review_state, ReviewState::Submitted);
}

#[test]
fn submit_review_blocks_unresolved_low_confidence_until_resolved_or_edited() {
    let mut unresolved = segment("low-confidence", 1.0, 2.0, "Alpha beta.");
    unresolved.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "word": "beta", "start": 1.2, "end": 1.4, "isYellow": true }
        ]),
    );
    let unresolved_project = project_with_segments(vec![unresolved]);
    let mut session = EditorSession::for_project(&unresolved_project);

    let row = session.transcript_rows(&unresolved_project).remove(0);
    assert_eq!(row.lint.as_ref().unwrap().severity, Severity::Warning);
    assert!(
        row.lint
            .as_ref()
            .unwrap()
            .message
            .contains("low-confidence")
    );
    assert!(!session.toggle_warning_assertion(&unresolved_project));
    session
        .submit_review(&unresolved_project)
        .expect_err("low-confidence words require explicit review");

    let mut resolved = unresolved_project.clone();
    resolved
        .segment_mut("low-confidence")
        .unwrap()
        .metadata
        .insert("lowConfidenceResolved".to_string(), serde_json::json!(true));
    let mut resolved_session = EditorSession::for_project(&resolved);
    resolved_session
        .submit_review(&resolved)
        .expect("resolved low-confidence metadata should unblock submit");

    let mut edited = unresolved_project.clone();
    edited.segment_mut("low-confidence").unwrap().text = "Alpha.".to_string();
    let mut edited_session = EditorSession::for_project(&edited);
    edited_session
        .submit_review(&edited)
        .expect("editing out the highlighted word should unblock submit");
}

#[test]
fn transcript_rows_expose_babel_low_confidence_word_views_for_text_cell() {
    let mut segment = segment("low-confidence", 1.0, 2.0, "alpha beta gamma");
    segment.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "index": 3, "finalWord": "alpha", "start": 1.05, "end": 1.2, "isYellow": false },
            { "index": 4, "finalWord": "beta", "start": 1.25, "end": 1.45, "isYellow": true },
            { "index": 5, "finalWord": "missing", "start": 1.5, "end": 1.6, "isYellow": true },
            { "index": 6, "finalWord": "gamma", "start": 1.6, "end": 1.8, "isYellow": false }
        ]),
    );
    let project = project_with_segments(vec![segment]);
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].word_views.len(), 1);
    assert_eq!(rows[0].word_views[0].index, 1);
    assert_eq!(rows[0].word_views[0].text, "beta");
    assert_eq!(rows[0].word_views[0].start_seconds, Some(1.25));
    assert_eq!(rows[0].word_views[0].end_seconds, Some(1.45));
    assert!(rows[0].word_views[0].is_low_confidence);
}

#[test]
fn seek_to_time_moves_playhead_and_selects_segment_under_timestamp() {
    let project = project_with_segments(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 1.0, 2.0, "second"),
    ]);
    let mut session = EditorSession::for_project(&project);

    session.seek_to_time(&project, 1.25);

    assert_eq!(session.playhead_seconds, 1.25);
    assert_eq!(session.selected_segment_id.as_deref(), Some("second"));
}

#[test]
fn zoom_reframes_the_visible_timeline_around_the_playhead() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-17");

    let before = session
        .timeline_lanes(&project)
        .iter()
        .flat_map(|lane| lane.blocks.iter())
        .find(|block| block.segment_id == "annotation-17")
        .unwrap()
        .left_percent;
    let before_duration = session.viewport.duration_seconds();
    session.set_zoom(&project, 2.0);
    let after = session
        .timeline_lanes(&project)
        .iter()
        .flat_map(|lane| lane.blocks.iter())
        .find(|block| block.segment_id == "annotation-17")
        .unwrap()
        .left_percent;

    assert_eq!(session.zoom, 2.0);
    assert!((session.viewport.duration_seconds() - 2.5).abs() < 0.001);
    assert!(session.viewport.duration_seconds() < before_duration);
    assert!(session.viewport.start_seconds > 70.0);
    assert!(session.viewport.end_seconds < 100.0);
    assert!((before - after).abs() < 0.001);
}

#[test]
fn zoom_clamps_to_babel_toolbar_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    session.set_zoom(&project, 0.01);
    assert_eq!(session.zoom, 0.05);

    session.set_zoom(&project, 20.0);
    assert_eq!(session.zoom, 10.0);
}

#[test]
fn the_viewport_shows_the_lane_width_at_babels_200_px_per_second_per_zoom() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.viewport.start_seconds = 10.0;
    session.viewport.end_seconds = 15.0;

    session.set_timeline_width(&project, 1_240.0);
    assert_eq!(session.timeline_width_px(), 1_240.0);
    assert_eq!(session.viewport.start_seconds, 10.0);
    assert!((session.viewport.duration_seconds() - 6.2).abs() < 1e-9);

    session.set_zoom(&project, 2.0);
    assert!((session.viewport.duration_seconds() - 3.1).abs() < 1e-9);
    assert_eq!(session.timeline_min_px_per_sec(), 400.0);
}

#[test]
fn speed_steps_move_between_babel_speed_options_and_saturate() {
    let mut session = EditorSession::for_project(&sample_project());

    session.step_speed(1);
    assert_eq!(session.speed, 1.5);
    session.step_speed(1);
    session.step_speed(1);
    assert_eq!(session.speed, 2.0);

    session.set_speed(0.6);
    session.step_speed(-1);
    assert_eq!(session.speed, 0.5);
    session.step_speed(-1);
    session.step_speed(-1);
    assert_eq!(session.speed, 0.25);
}

#[test]
fn loop_range_is_the_cut_selection_while_it_loops() {
    let project = sample_project();
    let lane_id = project.lanes[0].id.clone();
    let other_lane_id = project.lanes[1].id.clone();
    let mut session = EditorSession::for_project(&project);
    assert_eq!(session.loop_range(), None);

    session.set_overlay_range(&project, &lane_id, 3.5, 5.1);
    assert_eq!(session.loop_range(), None, "a new selection does not loop");

    session.overlay.as_mut().expect("selection").looping = true;
    let looped = TimeRange {
        start_seconds: 3.5,
        end_seconds: 5.1,
    };
    assert_eq!(session.loop_range(), Some(looped));

    session.select_segment(&project, "annotation-3");
    assert_eq!(
        session.loop_range(),
        Some(looped),
        "row selection does not move the loop"
    );

    session.set_overlay_range(&project, &lane_id, 3.5, 4.0);
    assert_eq!(
        session.loop_range(),
        Some(TimeRange {
            start_seconds: 3.5,
            end_seconds: 4.0,
        }),
        "resizing keeps the loop on the new range"
    );

    session.set_overlay_range(&project, &other_lane_id, 3.5, 4.0);
    assert_eq!(
        session.loop_range(),
        None,
        "another lane's selection is new"
    );
}

#[test]
fn transport_advances_playhead_with_speed_and_respects_loop_range() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "annotation-2");
    session.set_speed(2.0);

    session.advance_transport(&project, 1.25);

    assert_eq!(session.playhead_seconds, 3.5);

    session.toggle_playback();
    session.advance_transport(&project, 1.25);

    assert_eq!(session.playhead_seconds, 6.0);
    assert!(session.viewport.start_seconds <= session.playhead_seconds);
    assert!(session.viewport.end_seconds >= session.playhead_seconds);

    let lane_id = project.lanes[0].id.clone();
    session.set_overlay_range(&project, &lane_id, 3.5, 5.1);
    session.overlay.as_mut().expect("selection").looping = true;
    session.seek_to_time(&project, 4.5);
    session.advance_transport(&project, 0.4);

    assert!(session.loop_range().is_some());
    assert!((session.playhead_seconds - 3.7).abs() < f64::EPSILON * 8.0);
    assert!(session.is_playing);
}

#[test]
fn playback_start_preserves_viewport_through_its_midpoint() {
    let project = project_with_segments(vec![segment("long", 0.0, 60.0, "long")]);
    let mut session = EditorSession::for_project(&project);
    let viewport = TimeRange {
        start_seconds: 10.0,
        end_seconds: 15.0,
    };

    session.viewport = viewport;
    session.playhead_seconds = 11.0;
    session.prepare_viewport_for_playback(&project);
    assert_eq!(session.viewport, viewport);

    session.playhead_seconds = 12.5;
    session.prepare_viewport_for_playback(&project);
    assert_eq!(session.viewport, viewport);
}

#[test]
fn playback_start_centers_playhead_when_outside_or_past_viewport_midpoint() {
    let project = project_with_segments(vec![segment("long", 0.0, 60.0, "long")]);
    let mut session = EditorSession::for_project(&project);

    for (playhead_seconds, expected_start_seconds) in [(4.0, 1.5), (13.0, 10.5), (30.0, 27.5)] {
        session.viewport = TimeRange {
            start_seconds: 10.0,
            end_seconds: 15.0,
        };
        session.playhead_seconds = playhead_seconds;

        session.prepare_viewport_for_playback(&project);

        assert!((session.viewport.start_seconds - expected_start_seconds).abs() < 0.000_001);
        assert!((session.viewport.duration_seconds() - 5.0).abs() < 0.000_001);
    }
}

#[test]
fn transport_follow_waits_until_playhead_crosses_viewport_midpoint() {
    let project = project_with_segments(vec![segment("long", 0.0, 60.0, "long")]);
    let mut session = EditorSession::for_project(&project);
    let viewport = TimeRange {
        start_seconds: 10.0,
        end_seconds: 15.0,
    };
    session.viewport = viewport;
    session.playhead_seconds = 11.0;
    session.is_playing = true;

    session.advance_transport(&project, 1.0);
    assert_eq!(session.playhead_seconds, 12.0);
    assert_eq!(session.viewport, viewport);

    session.advance_transport(&project, 1.0);
    assert_eq!(session.playhead_seconds, 13.0);
    assert!((session.viewport.start_seconds - 10.5).abs() < 0.000_001);
    assert!((session.viewport.end_seconds - 15.5).abs() < 0.000_001);
}

#[test]
fn transport_selects_segment_under_playhead_without_rewinding_playhead() {
    let project = project_with_segments(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 1.0, 2.0, "second"),
        segment("third", 3.0, 4.0, "third"),
    ]);
    let mut session = EditorSession::for_project(&project);
    session.select_segment(&project, "first");
    session.toggle_playback();

    session.advance_transport(&project, 1.25);

    assert_eq!(session.selected_segment_id.as_deref(), Some("second"));
    assert_eq!(session.playhead_seconds, 1.25);
}

#[test]
fn diff_view_adds_visible_row_markers() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);

    assert!(session.transcript_rows(&project)[0].diff_marker.is_none());
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    assert_eq!(rows[0].diff_marker.as_deref(), Some("Babel unchanged"));
    assert!(rows[0].diff_source_text.is_none());
}

#[test]
fn diff_view_reports_changed_fields_and_original_text_from_babel_snapshot() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "edited text");
    changed.speaker_id = "speaker-2".to_string();
    changed.lane_id = "track-2".to_string();
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("original text"),
    );
    changed.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(1.0),
    );
    changed.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(2.0),
    );
    changed.metadata.insert(
        "studio.babel.original.speakerId".to_string(),
        serde_json::json!("speaker-1"),
    );
    changed.metadata.insert(
        "studio.babel.original.laneId".to_string(),
        serde_json::json!("track-1"),
    );

    let mut unchanged = segment("annotation-2", 3.0, 4.0, "same text");
    unchanged.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("same text"),
    );
    unchanged.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(3.0),
    );
    unchanged.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(4.0),
    );
    unchanged.metadata.insert(
        "studio.babel.original.speakerId".to_string(),
        serde_json::json!("speaker-1"),
    );
    unchanged.metadata.insert(
        "studio.babel.original.laneId".to_string(),
        serde_json::json!("track-1"),
    );

    let added = Segment {
        id: "annotation-2__studio_1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 5.0,
        end_seconds: 6.0,
        text: "new native row".to_string(),
        metadata: Default::default(),
    };
    let project = two_lane_project(vec![changed, unchanged, added]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(
        rows[0].diff_marker.as_deref(),
        Some("Changed: text, timing, track")
    );
    assert_eq!(rows[0].diff_source_text.as_deref(), Some("original text"));
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Babel unchanged"));
    assert!(rows[1].diff_source_text.is_none());
    assert_eq!(rows[2].diff_marker.as_deref(), Some("New"));
    assert!(rows[2].diff_source_text.is_none());
}

#[test]
fn diff_view_uses_selected_compare_action_snapshot_for_row_diff_source() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "edited text");
    changed.speaker_id = "speaker-2".to_string();
    changed.lane_id = "track-2".to_string();
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("imported original"),
    );
    changed.metadata.insert(
        "studio.babel.diff.compareSnapshots".to_string(),
        serde_json::json!({
            "l0": {
                "text": "first review text",
                "startSeconds": 1.25,
                "endSeconds": 2.25,
                "speakerId": "speaker-1",
                "laneId": "track-1",
                "wordDiffs": [
                    { "status": "removed", "value": "first" },
                    { "status": "added", "value": "edited" },
                    { "status": "unchanged", "value": "text" }
                ]
            },
            "l1": {
                "text": "selected compare text",
                "startSeconds": 1.0,
                "endSeconds": 2.0,
                "speakerId": "speaker-1",
                "laneId": "track-1",
                "wordDiffs": [
                    { "status": "removed", "value": "selected" },
                    { "status": "added", "value": "edited" },
                    { "status": "unchanged", "value": "text" }
                ]
            }
        }),
    );
    let mut project = two_lane_project(vec![changed]);
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l1".to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let diff = rows[0].diff.as_ref().unwrap();

    assert_eq!(
        rows[0].diff_source_text.as_deref(),
        Some("selected compare text")
    );
    assert_eq!(
        rows[0].diff_marker.as_deref(),
        Some("Changed: text, timing, track")
    );
    assert_eq!(diff.change_count, 1);
    assert_eq!(
        word_diff_summary(diff),
        vec![
            (TranscriptWordDiffStatus::Removed, "selected"),
            (TranscriptWordDiffStatus::Added, "edited"),
            (TranscriptWordDiffStatus::Unchanged, "text"),
        ]
    );
}

#[test]
fn diff_view_exposes_babel_annotation_status_from_compare_snapshot() {
    let mut annotation = segment("annotation-1", 1.0, 2.0, "background-noise / rustle");
    annotation
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    annotation.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    annotation
        .metadata
        .insert("intensity".to_string(), serde_json::json!(4));
    annotation.metadata.insert(
        "studio.babel.diff.compareSnapshots".to_string(),
        serde_json::json!({
            "l0": {
                "content": {
                    "primaryLabel": "background-noise",
                    "secondaryLabel": "mic bump"
                },
                "intensity": 2,
                "startTimeInSeconds": 1.0,
                "endTimeInSeconds": 2.0
            }
        }),
    );
    let mut project = project_with_segments(vec![annotation]);
    project.metadata.insert(
        "studio.babel.diff.selectedCompareActionId".to_string(),
        "l0".to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let annotation_diff = rows[0].annotation_diff.as_ref().unwrap();

    assert_eq!(
        annotation_diff.status,
        AnnotationDiffStatus::SubcategoryMismatch
    );
    assert_eq!(
        annotation_diff.other_secondary_label.as_deref(),
        Some("mic bump")
    );
    assert_eq!(annotation_diff.other_intensity, Some(2));
    assert_eq!(annotation_diff.counterpart_start_seconds, Some(1.0));
    assert_eq!(annotation_diff.counterpart_end_seconds, Some(2.0));
}

#[test]
fn diff_view_text_cells_are_separate_from_linter() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "Edited text.");
    changed.speaker_id = "speaker-2".to_string();
    changed.lane_id = "track-2".to_string();
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("Original text."),
    );
    changed.metadata.insert(
        "studio.babel.original.startSeconds".to_string(),
        serde_json::json!(1.0),
    );
    changed.metadata.insert(
        "studio.babel.original.endSeconds".to_string(),
        serde_json::json!(2.0),
    );
    changed.metadata.insert(
        "studio.babel.original.speakerId".to_string(),
        serde_json::json!("speaker-1"),
    );
    changed.metadata.insert(
        "studio.babel.original.laneId".to_string(),
        serde_json::json!("track-1"),
    );

    let added = Segment {
        id: "annotation-2__studio_1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 5.0,
        end_seconds: 6.0,
        text: "New native row.".to_string(),
        metadata: Default::default(),
    };
    let project = two_lane_project(vec![changed, added]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert!(rows[0].lint.is_none());
    assert_eq!(rows[0].diff.as_ref().unwrap().change_count, 1);
    assert_eq!(
        rows[0].diff.as_ref().unwrap().source_text.as_deref(),
        Some("Original text.")
    );
    assert!(rows[1].lint.is_none());
    assert_eq!(
        rows[1].diff.as_ref().unwrap().kind,
        TranscriptDiffCellKind::New
    );
    assert!(rows[1].diff.as_ref().unwrap().source_text.is_none());
}

#[test]
fn diff_view_uses_babel_word_diffs_and_counts_replacements_once() {
    let mut changed = segment("annotation-1", 1.5, 2.5, "alpha gamma new");
    changed.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!("alpha beta old"),
    );
    changed.metadata.insert(
        "studio.babel.diff.wordDiffs".to_string(),
        serde_json::json!([
            { "status": "unchanged", "value": "alpha" },
            { "status": "removed", "value": "beta" },
            { "status": "added", "value": "gamma" },
            { "status": "removed", "value": "old" },
            { "status": "added", "value": "new" }
        ]),
    );
    let project = project_with_segments(vec![changed]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let diff = rows[0].diff.as_ref().unwrap();

    assert_eq!(diff.kind, TranscriptDiffCellKind::Diff);
    assert_eq!(diff.change_count, 2);
    assert_eq!(
        word_diff_summary(diff),
        vec![
            (TranscriptWordDiffStatus::Unchanged, "alpha"),
            (TranscriptWordDiffStatus::Removed, "beta"),
            (TranscriptWordDiffStatus::Added, "gamma"),
            (TranscriptWordDiffStatus::Removed, "old"),
            (TranscriptWordDiffStatus::Added, "new"),
        ]
    );
}

#[test]
fn diff_view_marks_empty_diffs_and_new_rows() {
    let mut unchanged_empty = segment("annotation-1", 1.5, 2.5, "");
    unchanged_empty.metadata.insert(
        "studio.babel.original.text".to_string(),
        serde_json::json!(""),
    );
    unchanged_empty.metadata.insert(
        "studio.babel.diff.wordDiffs".to_string(),
        serde_json::json!([]),
    );

    let added_empty = Segment {
        id: "annotation-2__studio_1".to_string(),
        source_annotation_id: None,
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 5.0,
        end_seconds: 6.0,
        text: "".to_string(),
        metadata: Default::default(),
    };
    let project = project_with_segments(vec![unchanged_empty, added_empty]);
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);
    let unchanged_diff = rows[0].diff.as_ref().unwrap();
    let added_diff = rows[1].diff.as_ref().unwrap();

    assert_eq!(unchanged_diff.kind, TranscriptDiffCellKind::Diff);
    assert_eq!(unchanged_diff.change_count, 0);
    assert!(unchanged_diff.words.is_empty());

    assert_eq!(added_diff.kind, TranscriptDiffCellKind::New);
    assert_eq!(added_diff.change_count, 0);
    assert!(added_diff.words.is_empty());
}

#[test]
fn diff_view_interleaves_babel_deleted_segment_rows() {
    let project = project_with_segments(vec![
        segment("current-a", 1.0, 1.5, "current before"),
        segment("current-b", 4.0, 4.5, "current after"),
    ]);
    let mut project = project;
    project.metadata.insert(
        "studio.babel.diff.result".to_string(),
        serde_json::json!({
            "speakerDiffs": [{
                "segmentMappings": [{
                    "relationship": "deleted",
                    "segmentsA": [{
                        "annotationId": "deleted-reference",
                        "startTimeInSeconds": 2.0,
                        "endTimeInSeconds": 3.0,
                        "trackId": "track-1",
                        "speakerId": "speaker-1"
                    }],
                    "segmentsB": [],
                    "wordDiffs": [
                        { "status": "removed", "value": "old" },
                        { "status": "removed", "value": "missing" }
                    ]
                }]
            }]
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].row_kind, TranscriptRowKind::ModifiedDeleted);
    assert_eq!(rows[1].segment_id, "modified-deleted-reference");
    assert_eq!(rows[1].display_id, "2");
    assert_eq!(rows[1].speaker_label, "Deleted");
    assert_eq!(rows[1].start_time, "00:02.00");
    assert_eq!(rows[1].end_time, "00:03.00");
    assert!(rows[1].is_read_only);
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Deleted"));
    let diff = rows[1].diff.as_ref().unwrap();
    assert_eq!(diff.kind, TranscriptDiffCellKind::ModifiedDeleted);
    assert_eq!(
        word_diff_summary(diff),
        vec![
            (TranscriptWordDiffStatus::Removed, "old"),
            (TranscriptWordDiffStatus::Removed, "missing"),
        ]
    );
}

#[test]
fn diff_view_interleaves_babel_compare_newer_added_segment_rows() {
    let mut project = project_with_segments(vec![
        segment("current-a", 1.0, 1.5, "current before"),
        segment("current-b", 4.0, 4.5, "current after"),
    ]);
    project.metadata.insert(
        "studio.babel.diff.isCompareNewer".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.diff.result".to_string(),
        serde_json::json!({
            "speakerDiffs": [{
                "segmentMappings": [{
                    "relationship": "added",
                    "segmentsA": [],
                    "segmentsB": [{
                        "annotationId": "newer-added",
                        "startTimeInSeconds": 2.0,
                        "endTimeInSeconds": 3.0,
                        "trackId": "track-1",
                        "speakerId": "speaker-1"
                    }],
                    "wordDiffs": [
                        { "status": "added", "value": "newer" },
                        { "status": "added", "value": "line" }
                    ]
                }]
            }]
        })
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].row_kind, TranscriptRowKind::ModifiedAdded);
    assert_eq!(rows[1].segment_id, "modified-newer-added");
    assert_eq!(rows[1].display_id, "2");
    assert_eq!(rows[1].speaker_label, "Added");
    assert!(rows[1].is_read_only);
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Added"));
    let diff = rows[1].diff.as_ref().unwrap();
    assert_eq!(diff.kind, TranscriptDiffCellKind::ModifiedAdded);
    assert_eq!(
        word_diff_summary(diff),
        vec![
            (TranscriptWordDiffStatus::Added, "newer"),
            (TranscriptWordDiffStatus::Added, "line"),
        ]
    );
}

#[test]
fn diff_view_interleaves_babel_removed_annotation_ghost_rows() {
    let mut project = project_with_segments(vec![
        segment("current-a", 1.0, 1.5, "current before"),
        segment("current-b", 4.0, 4.5, "current after"),
    ]);
    project.metadata.insert(
        "studio.babel.diff.annotationGhostRows".to_string(),
        serde_json::json!([{
            "comparisonLabel": "Reference",
            "annotation": {
                "id": "removed-annotation",
                "content": {
                    "primaryLabel": "background-noise",
                    "secondaryLabel": "rustle"
                },
                "intensity": 3,
                "startTimeInSeconds": 2.0,
                "endTimeInSeconds": 3.0
            }
        }])
        .to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    session.toggle_diff_view();

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[1].row_kind, TranscriptRowKind::AnnotationRemovedGhost);
    assert_eq!(rows[1].segment_id, "annotation-ghost-removed-annotation");
    assert_eq!(rows[1].display_id, "2");
    assert_eq!(rows[1].speaker_label, "Reference");
    assert_eq!(
        rows[1].annotation_kind,
        TranscriptAnnotationKind::NonTranscription("removed".to_string())
    );
    assert_eq!(rows[1].annotation_type_label, "Removed");
    assert_eq!(rows[1].start_time, "00:02.00");
    assert_eq!(rows[1].end_time, "00:03.00");
    assert_eq!(rows[1].text, "background-noise / rustle · intensity 3");
    assert!(rows[1].is_read_only);
    assert_eq!(rows[1].diff_marker.as_deref(), Some("Removed"));
    let annotation_diff = rows[1].annotation_diff.as_ref().unwrap();
    assert_eq!(
        annotation_diff.status,
        AnnotationDiffStatus::UnmatchedReference
    );
}

#[test]
fn warning_assertions_hide_active_warnings_from_problem_navigation() {
    let project = project_with_segments(vec![
        segment("warning", 1.0, 60.0, "Это (пример)."),
        segment("clean", 61.0, 62.0, "Clean."),
        segment("error", 63.0, 64.0, ""),
    ]);
    let mut session = EditorSession::for_project(&project);

    assert_eq!(session.problem_row_count(&project), 2);
    session.select_segment(&project, "warning");
    assert!(session.toggle_warning_assertion(&project));

    let warning_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "warning")
        .unwrap();
    assert!(warning_row.lint.is_none());
    assert!(warning_row.lint.is_none());
    assert!(warning_row.has_asserted_warning);
    assert_eq!(session.problem_row_count(&project), 1);

    session.navigate_problem_rows(&project, 1);
    assert_eq!(session.selected_segment_id.as_deref(), Some("error"));
}

#[test]
fn row_views_expose_directional_merge_availability_per_lane() {
    let mut other_lane = segment("other-lane", 2.0, 3.0, "interleaved");
    other_lane.lane_id = "track-2".to_string();
    other_lane.speaker_id = "speaker-2".to_string();
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        other_lane,
        segment("second", 4.0, 5.0, "second"),
        segment("third", 7.0, 8.0, "third"),
    ]);
    let session = EditorSession::for_project(&project);
    let rows = session.transcript_rows(&project);

    let first = rows.iter().find(|row| row.segment_id == "first").unwrap();
    let second = rows.iter().find(|row| row.segment_id == "second").unwrap();
    let third = rows.iter().find(|row| row.segment_id == "third").unwrap();

    assert!(!first.can_merge_above);
    assert!(first.can_merge_below);
    assert!(second.can_merge_above);
    assert!(second.can_merge_below);
    assert!(third.can_merge_above);
    assert!(!third.can_merge_below);
}

#[test]
fn transcript_action_menu_is_compact_babel_kebab_model() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 2.0, 3.0, "second"),
    ]);
    let session = EditorSession::for_project(&project);
    let second_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "second")
        .unwrap();

    let menu = CoreSession::transcript_action_menu(&second_row);

    assert_eq!(
        menu.iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            Some((TranscriptActionCommand::CreateBefore, true)),
            Some((TranscriptActionCommand::CreateAfter, true)),
            None,
            Some((TranscriptActionCommand::MergeAbove, true)),
            Some((TranscriptActionCommand::MergeBelow, false)),
            None,
            Some((TranscriptActionCommand::Delete, true)),
        ]
    );
}

#[test]
fn transcript_action_menu_exposes_edit_for_non_transcription_babel_annotations() {
    let mut annotation = segment("noise", 0.0, 1.0, "background-noise / rustle");
    annotation
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    let project = project_with_segments(vec![annotation]);
    let session = EditorSession::for_project(&project);
    let row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "noise")
        .unwrap();

    let menu = CoreSession::transcript_action_menu(&row);

    assert_eq!(
        menu.iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            Some((TranscriptActionCommand::Edit, true)),
            None,
            Some((TranscriptActionCommand::MergeAbove, false)),
            Some((TranscriptActionCommand::MergeBelow, false)),
            None,
            Some((TranscriptActionCommand::Delete, true)),
        ]
    );
}

#[test]
fn transcript_action_menu_keeps_babel_insert_actions_even_when_rows_touch() {
    let project = two_lane_project(vec![
        segment("first", 0.0, 1.0, "first"),
        segment("second", 1.0, 2.0, "second"),
    ]);
    let session = EditorSession::for_project(&project);
    let first_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "first")
        .unwrap();
    let second_row = session
        .transcript_rows(&project)
        .into_iter()
        .find(|row| row.segment_id == "second")
        .unwrap();

    let first_menu = CoreSession::transcript_action_menu(&first_row);
    let menu = CoreSession::transcript_action_menu(&second_row);

    assert_eq!(
        first_menu
            .iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            Some((TranscriptActionCommand::CreateBefore, true)),
            Some((TranscriptActionCommand::CreateAfter, true)),
            None,
            Some((TranscriptActionCommand::MergeAbove, false)),
            Some((TranscriptActionCommand::MergeBelow, true)),
            None,
            Some((TranscriptActionCommand::Delete, true)),
        ]
    );
    assert_eq!(
        menu.iter()
            .map(action_menu_entry_summary)
            .collect::<Vec<_>>(),
        vec![
            Some((TranscriptActionCommand::CreateBefore, true)),
            Some((TranscriptActionCommand::CreateAfter, true)),
            None,
            Some((TranscriptActionCommand::MergeAbove, true)),
            Some((TranscriptActionCommand::MergeBelow, false)),
            None,
            Some((TranscriptActionCommand::Delete, true)),
        ]
    );
}

#[test]
fn display_ids_are_visible_row_order_numbers_not_babel_annotation_ids() {
    let project = project_with_segments(vec![
        segment("annotation-20", 0.0, 1.0, "twenty"),
        segment("annotation-2", 2.0, 3.0, "two"),
        Segment {
            id: "annotation-20__studio_1".to_string(),
            source_annotation_id: None,
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-1".to_string(),
            start_seconds: 4.0,
            end_seconds: 5.0,
            text: "derived".to_string(),
            metadata: Default::default(),
        },
    ]);

    let rows = EditorSession::for_project(&project).transcript_rows(&project);

    assert_eq!(
        rows.iter()
            .map(|row| row.segment_id.as_str())
            .collect::<Vec<_>>(),
        vec!["annotation-20", "annotation-2", "annotation-20__studio_1"]
    );
    assert_eq!(
        rows.iter()
            .map(|row| row.display_id.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2", "3"]
    );
}

#[test]
fn script_context_is_extracted_from_metadata_and_extra_package() {
    let mut project = project_with_segments(Vec::new());
    project
        .metadata
        .insert("studio.babel.emotion".to_string(), "calm".to_string());
    project.metadata.insert(
        "studio.babel.scriptContext.speech".to_string(),
        "slow".to_string(),
    );
    project.metadata.insert(
        "studio.babel.scriptContext.physical".to_string(),
        "  ".to_string(),
    );
    project.metadata.insert(
        "babel.extra.package".to_string(),
        serde_json::json!({
            "scriptContext": { "character": "Alice", "preamble": " keep leading " },
            "submissionId": "submission-123"
        })
        .to_string(),
    );

    let context = script_context(&project).expect("emotion and fields should yield a panel");

    assert_eq!(context.emotion.as_deref(), Some("calm"));
    assert_eq!(context.submission_id.as_deref(), Some("submission-123"));
    assert_eq!(
        context
            .fields
            .iter()
            .map(|field| (field.key, field.value.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("character", "Alice"),
            ("speech", "slow"),
            ("preamble", " keep leading "),
        ]
    );
}

#[test]
fn original_script_prefers_metadata_then_extra_package() {
    let mut project = project_with_segments(Vec::new());
    assert_eq!(original_script(&project), None);

    project.metadata.insert(
        "babel.extra.package".to_string(),
        serde_json::json!({ "recording": { "scriptText": "From package" } }).to_string(),
    );
    assert_eq!(original_script(&project).as_deref(), Some("From package"));

    project.metadata.insert(
        "studio.babel.originalScript".to_string(),
        "Original line\nSecond line".to_string(),
    );
    assert_eq!(
        original_script(&project).as_deref(),
        Some("Original line\nSecond line")
    );
}

#[test]
fn script_context_stays_absent_without_emotion_or_fields() {
    let mut project = project_with_segments(Vec::new());
    assert!(script_context(&project).is_none());

    project
        .metadata
        .insert("studio.babel.emotion".to_string(), "   ".to_string());
    project
        .metadata
        .insert("studio.babel.submissionId".to_string(), "sub-1".to_string());
    assert!(script_context(&project).is_none());
}

#[test]
fn task_lookup_trims_inputs_and_requires_confirm_before_claim() {
    let mut state = TaskLookupState::new("", "", true);

    assert_eq!(
        state.apply(TaskLookupEvent::ChangeProcessedTranscriptionId {
            value: "  processed-7  ".to_string(),
        }),
        None
    );
    assert_eq!(state.processed_transcription_id, "processed-7");
    assert!(state.can_claim_stitched());

    assert_eq!(state.apply(TaskLookupEvent::ClickClaimStitched), None);
    assert!(state.confirming_edit);
    assert_eq!(state.apply(TaskLookupEvent::CancelConfirmEdit), None);
    assert!(!state.confirming_edit);

    state.apply(TaskLookupEvent::ClickClaimStitched);
    assert_eq!(
        state.apply(TaskLookupEvent::ConfirmEdit),
        Some(TaskLookupEffect::ClaimById)
    );
    assert!(!state.confirming_edit);

    let mut blocked = TaskLookupState::new("processed-7", "", false);
    assert!(!blocked.can_claim_stitched());
    assert_eq!(blocked.apply(TaskLookupEvent::ClickClaimStitched), None);
    assert!(!blocked.confirming_edit);
}

#[test]
fn task_lookup_read_only_prefers_review_action_id_then_processed_transcription_id() {
    assert_eq!(
        TaskLookupState::new("processed-7", "review-action-4", true)
            .apply(TaskLookupEvent::ClickReadOnly),
        Some(TaskLookupEffect::ViewReadOnlyByReviewActionId)
    );
    assert_eq!(
        TaskLookupState::new("processed-7", "", true).apply(TaskLookupEvent::ClickReadOnly),
        Some(TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId)
    );
    let mut empty = TaskLookupState::new("", "", true);
    assert!(!empty.can_view_read_only());
    assert_eq!(empty.apply(TaskLookupEvent::ClickReadOnly), None);
}

#[test]
fn error_projects_claim_qa_tasks_by_diff_id_without_the_stitched_confirm() {
    let mut state = TaskLookupState::new("", "", true).claiming_qa_tasks(true);
    assert!(state.shows_qa_claim() && !state.shows_stitched_claim());
    assert_eq!(state.apply(TaskLookupEvent::ClickClaimQa), None);
    state.apply(TaskLookupEvent::ChangeQaDiffId {
        value: " diff-9 ".to_string(),
    });
    assert_eq!(state.qa_diff_id, "diff-9");
    state.apply(TaskLookupEvent::ChangeProcessedTranscriptionId {
        value: "processed-7".to_string(),
    });
    assert!(!state.can_claim_stitched());
    assert_eq!(
        state.apply(TaskLookupEvent::ClickClaimQa),
        Some(TaskLookupEffect::ClaimQaTask)
    );
    assert!(!state.confirming_edit);
    // Without a stitched queue there is nothing to claim against.
    let mut no_queue = TaskLookupState::new("", "", false).claiming_qa_tasks(true);
    no_queue.apply(TaskLookupEvent::ChangeQaDiffId {
        value: "diff-9".to_string(),
    });
    assert_eq!(no_queue.apply(TaskLookupEvent::ClickClaimQa), None);
}

#[test]
fn task_lookup_opens_from_startup_route_metadata() {
    let mut project = project_with_segments(Vec::new());
    assert_eq!(TaskLookupState::from_project(&project), None);

    project.metadata.insert(
        "studio.babel.taskLookup.open".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.taskLookup.reviewActionId".to_string(),
        "review-9".to_string(),
    );
    project.metadata.insert(
        "studio.babel.taskLookup.showStitchedOption".to_string(),
        "1".to_string(),
    );

    assert_eq!(
        TaskLookupState::from_project(&project),
        Some(TaskLookupState::new("", "review-9", true))
    );
}

#[test]
fn feedback_rubric_flags_match_babel_contract() {
    assert_eq!(FEEDBACK_MAX_COMMENT_LENGTH, 500);
    assert_eq!(
        get_rubric_fields_for_flags(None)
            .iter()
            .map(|field| field.label)
            .collect::<Vec<_>>(),
        vec![
            "Word Accuracy",
            "Timestamp Accuracy",
            "Punctuation & Formatting",
            "Tags & Emphasis",
            "Segmentation",
        ]
    );
    assert_eq!(
        get_rubric_fields_for_flags(Some(&["tts-style-tag-accuracy", "tts-other-feedback",]))
            .iter()
            .map(|field| (field.key, field.label, field.required))
            .collect::<Vec<_>>(),
        vec![("styleTagAccuracy", "Style Tag Accuracy", true)]
    );
    assert!(has_other_feedback_flag(None));
    assert!(has_other_feedback_flag(Some(&["tts-other-feedback"])));
    assert!(!has_other_feedback_flag(Some(&["tts-style-tag-accuracy"])));
}

#[test]
fn feedback_response_parsing_normalizes_ratings_comments_and_other_feedback() {
    let inputs = vec![
        FeedbackInput {
            id: "word-rating".to_string(),
            label: Some("Word Accuracy".to_string()),
        },
        FeedbackInput {
            id: "word-comment".to_string(),
            label: Some("Word Accuracy Comment".to_string()),
        },
        FeedbackInput {
            id: "timestamp-rating".to_string(),
            label: Some("Timestamp Accuracy".to_string()),
        },
        FeedbackInput {
            id: "other".to_string(),
            label: Some("Other Feedback".to_string()),
        },
    ];
    let responses = vec![
        FeedbackInputResponse {
            form_input_id: "word-rating".to_string(),
            value: "3".to_string(),
        },
        FeedbackInputResponse {
            form_input_id: "word-comment".to_string(),
            value: "Great wording".to_string(),
        },
        FeedbackInputResponse {
            form_input_id: "timestamp-rating".to_string(),
            value: "9".to_string(),
        },
        FeedbackInputResponse {
            form_input_id: "other".to_string(),
            value: "Global note".to_string(),
        },
    ];

    let draft = parse_feedback_responses(&responses, Some(&inputs))
        .expect("responses plus inputs should parse feedback");

    assert_eq!(
        draft.entries.get("wordAccuracy"),
        Some(&FeedbackRubricEntry {
            rating: 3,
            comment: "Great wording".to_string(),
        })
    );
    assert_eq!(
        draft.entries.get("timestampAccuracy"),
        Some(&FeedbackRubricEntry {
            rating: 1,
            comment: String::new(),
        })
    );
    assert_eq!(draft.other_feedback, "Global note");
    assert_eq!(parse_feedback_responses(&responses, None), None);
    assert_eq!(parse_feedback_responses(&[], Some(&inputs)), None);
}

#[test]
fn feedback_draft_serializes_to_form_inputs_and_round_trips() {
    let mut draft = FeedbackDraft::default();
    draft.entries.insert(
        "wordAccuracy".to_string(),
        FeedbackRubricEntry {
            rating: 2,
            comment: "Fix one word".to_string(),
        },
    );
    draft.entries.insert(
        "timestampAccuracy".to_string(),
        FeedbackRubricEntry {
            rating: 3,
            comment: "Good timing".to_string(),
        },
    );
    draft.other_feedback = "Other note".to_string();
    let inputs = vec![
        FeedbackInput {
            id: "word-rating".to_string(),
            label: Some("Word Accuracy".to_string()),
        },
        FeedbackInput {
            id: "word-comment".to_string(),
            label: Some("Word Accuracy Comment".to_string()),
        },
        FeedbackInput {
            id: "timestamp-rating".to_string(),
            label: Some("Timestamp Accuracy".to_string()),
        },
        FeedbackInput {
            id: "other".to_string(),
            label: Some("Other Feedback".to_string()),
        },
        FeedbackInput {
            id: "tts-other".to_string(),
            label: Some("TTS Other Feedback".to_string()),
        },
    ];

    assert_eq!(
        serialize_feedback_draft(&draft, Some(&inputs), None, None, None),
        vec![
            FeedbackInputResponse {
                form_input_id: "word-rating".to_string(),
                value: "2".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "word-comment".to_string(),
                value: "Fix one word".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "timestamp-rating".to_string(),
                value: "3".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "other".to_string(),
                value: "Other note".to_string(),
            },
            FeedbackInputResponse {
                form_input_id: "tts-other".to_string(),
                value: "Other note".to_string(),
            },
        ]
    );
    assert_eq!(
        compute_feedback_draft_save_request(FeedbackSaveRequestInput {
            enabled: false,
            form_step_response_id: Some("response-1".to_string()),
            draft_feedback: draft.clone(),
            inputs: Some(inputs.clone()),
            rubric_fields: None,
            rubric_flag_names: None,
            has_other_feedback: None,
        }),
        None
    );
    let save = compute_feedback_draft_save_request(FeedbackSaveRequestInput {
        enabled: true,
        form_step_response_id: Some("response-1".to_string()),
        draft_feedback: draft.clone(),
        inputs: Some(inputs.clone()),
        rubric_fields: None,
        rubric_flag_names: None,
        has_other_feedback: Some(false),
    })
    .expect("enabled save with response id should serialize");
    assert_eq!(save.form_step_response_id, "response-1");
    assert_eq!(save.input_responses.len(), 3);

    assert_eq!(
        parse_feedback_responses(&save.input_responses, Some(&inputs))
            .expect("saved responses should parse back")
            .entries["wordAccuracy"],
        FeedbackRubricEntry {
            rating: 2,
            comment: "Fix one word".to_string(),
        }
    );
}

#[test]
fn imported_server_lint_maps_by_source_annotation_id() {
    let mut imported = segment("internal-id", 1.0, 2.0, "Clean.");
    imported.source_annotation_id = Some("babel-source-id".to_string());
    let mut project = project_with_segments(vec![imported]);
    project.metadata.insert(
        "babel.extra.package".to_string(),
        serde_json::json!({
            "lintErrors": [{
                "annotationId": "babel-source-id",
                "reason": "Exact server reason",
                "severity": "warning"
            }]
        })
        .to_string(),
    );

    let session = EditorSession::for_project(&project);
    let snapshot = session
        .server_lint_snapshot()
        .expect("imported lint array should hydrate a checked snapshot");
    assert_eq!(snapshot.revision, 0);
    assert_eq!(snapshot.issues.len(), 1);
    assert_eq!(snapshot.issues[0].annotation_id, "babel-source-id");
    assert_eq!(snapshot.issues[0].reason, "Exact server reason");
    assert_eq!(snapshot.issues[0].severity, LintSeverity::Warning);

    let row = session.transcript_rows(&project).remove(0);
    assert_eq!(row.lint.as_ref().unwrap().source, "babel.server");
    assert_eq!(row.lint.as_ref().unwrap().message, "Exact server reason");
}

#[test]
fn imported_empty_server_lint_is_distinct_from_unchecked() {
    let unchecked_project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    let unchecked = EditorSession::for_project(&unchecked_project);
    assert!(unchecked.server_lint_snapshot().is_none());

    let mut checked_project = unchecked_project;
    checked_project.metadata.insert(
        "babel.extra.package".to_string(),
        serde_json::json!({ "lint_errors": [] }).to_string(),
    );
    let checked = EditorSession::for_project(&checked_project);
    let snapshot = checked
        .server_lint_snapshot()
        .expect("an imported empty array is an authoritative clean result");
    assert!(snapshot.issues.is_empty());
}

#[test]
fn edits_invalidate_server_lint_and_reject_stale_installation() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    project.metadata.insert(
        "babel.extra.package".to_string(),
        serde_json::json!({ "lintErrors": [] }).to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    let stale_snapshot = session.server_lint_snapshot().unwrap().clone();

    session.mark_dirty();

    assert_eq!(session.current_edit_revision(), 1);
    assert!(session.server_lint_snapshot().is_none());
    assert!(!session.set_server_lint_snapshot(&project, stale_snapshot));
    assert!(session.set_server_lint_snapshot(&project, ServerLintSnapshot::new(1, Vec::new())));
    assert!(session.server_lint_snapshot().is_some());
}

#[test]
fn server_lint_is_primary_before_native_row_lint() {
    let project = project_with_segments(vec![segment("bad", 1.0, 2.0, "")]);
    let mut session = EditorSession::for_project(&project);
    assert!(session.set_server_lint_snapshot(
        &project,
        ServerLintSnapshot::new(
            0,
            vec![ServerLintIssue {
                annotation_id: "bad".to_string(),
                reason: "Server issue wins".to_string(),
                severity: LintSeverity::Warning,
            }],
        )
    ));

    let row = session.transcript_rows(&project).remove(0);
    assert_eq!(row.lint.as_ref().unwrap().source, "babel.server");
    assert_eq!(row.lint.as_ref().unwrap().message, "Server issue wins");
    assert_eq!(row.lint_issues[1].rule, "empty-content");
}

#[test]
fn unmatched_and_malformed_server_lint_remain_global_blockers() {
    let mut project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    project.metadata.insert(
        "babel.extra.package".to_string(),
        serde_json::json!({
            "lintErrors": [
                {
                    "annotationId": "missing-row",
                    "reason": "Global server blocker",
                    "severity": "error"
                },
                {
                    "annotationId": 17,
                    "severity": "warning"
                }
            ]
        })
        .to_string(),
    );
    let session = EditorSession::for_project(&project);
    let snapshot = session.server_lint_snapshot().unwrap();

    assert_eq!(snapshot.issues.len(), 2);
    assert_eq!(snapshot.issues[0].reason, "Global server blocker");
    assert_eq!(snapshot.issues[1].severity, LintSeverity::Error);
    assert!(
        snapshot.issues[1]
            .reason
            .contains("Malformed Babel server lint issue")
    );
    assert_eq!(session.blocking_lint_count(&project), 2);
    assert!(!session.can_submit_review(&project));
}

#[test]
fn unchecked_babel_project_keeps_native_lint_advisory_for_preflight() {
    let mut project = project_with_segments(vec![segment("native-false-positive", 1.0, 2.0, "")]);
    project.metadata.insert(
        "studio.babel.projectId".to_string(),
        "babel-project".to_string(),
    );
    let session = EditorSession::for_project(&project);

    assert!(session.server_lint_snapshot().is_none());
    assert_eq!(
        session.transcript_rows(&project)[0]
            .lint
            .as_ref()
            .unwrap()
            .severity,
        Severity::Error
    );
    assert!(session.can_submit_review(&project));
    assert!(session.submit_eligibility(&project).is_allowed());
}

#[test]
fn server_warning_assertion_uses_exact_segment_reason_and_invalidates_snapshot() {
    let mut project = project_with_segments(vec![segment("internal-id", 1.0, 2.0, "Clean.")]);
    project.segments[0].source_annotation_id = Some("source-id".to_string());
    project.metadata.insert(
        "studio.babel.projectId".to_string(),
        "babel-project".to_string(),
    );
    let mut session = EditorSession::for_project(&project);
    assert!(session.set_server_lint_snapshot(
        &project,
        ServerLintSnapshot::new(
            0,
            vec![ServerLintIssue {
                annotation_id: "source-id".to_string(),
                reason: "Keep this exact warning".to_string(),
                severity: LintSeverity::Warning,
            }],
        )
    ));

    assert!(session.toggle_warning_assertion(&project));

    assert!(
        session
            .asserted_warning_keys
            .contains("internal-id::babel.server::server::Keep this exact warning")
    );
    assert_eq!(session.current_edit_revision(), 1);
    assert!(session.server_lint_snapshot().is_none());
}

#[test]
fn core_edits_invalidate_external_submitted_review_state() {
    let project = project_with_segments(vec![segment("clean", 1.0, 2.0, "Clean.")]);
    let mut core = CoreSession::for_project_with_environment(&project, test_environment(&project));
    let mut state = BabelDocumentState::for_project(&project);
    state.bind(&mut core).submit_review(&project).unwrap();
    assert_eq!(state.review_state(&core), ReviewState::Submitted);
    core.mark_dirty();
    assert_eq!(state.view(&core).review_state(), ReviewState::Draft);
}

#[test]
fn partial_word_acknowledgements_are_shared_with_neutral_presentation() {
    let mut row = segment("words", 1.0, 2.0, "Alpha beta.");
    row.metadata.insert(
        "babelLowConfidenceWords".into(),
        serde_json::json!([
            { "index": 10, "finalWord": "Alpha", "start": 1.1, "end": 1.3, "isYellow": true },
            { "index": 11, "finalWord": "beta", "start": 1.4, "end": 1.7, "isYellow": true }
        ]),
    );
    let project = project_with_segments(vec![row]);
    let environment = test_environment(&project);
    let session = CoreSession::for_project_with_environment(&project, environment);
    let rows = session.transcript_rows(&project);
    let word = &rows[0].word_views[0];
    assert!(!word.is_acknowledged);
    let command = word.command.as_deref().unwrap();
    let provider = session
        .environment()
        .providers
        .commands
        .iter()
        .find(|provider| provider.command_ids().contains(&command))
        .unwrap();
    let outcome = provider
        .execute(
            command,
            &exaudio_extension::document::CommandRequest {
                project: &project,
                revision: session.edit_revision(),
                segment_id: Some("words"),
                text: None,
                selection: None,
                caret_offset: None,
                arguments: &word.command_arguments,
            },
        )
        .unwrap();
    assert!(outcome.transaction.is_none());
    let rows = session.transcript_rows(&project);
    assert!(rows[0].word_views[0].is_acknowledged);
    assert!(!rows[0].word_views[1].is_acknowledged);
}

#[test]
fn annotation_classification_is_explicitly_installed_by_the_domain() {
    let mut row = segment("annotation", 1.0, 2.0, "Event");
    row.metadata
        .insert("type".into(), serde_json::json!("sound"));
    let project = project_with_segments(vec![row]);
    let local = exaudio_extension::document::DocumentEnvironment::local("native");
    assert!(local.is_text_segment(&project.segments[0]));
    let domain = test_environment(&project);
    assert!(!domain.is_text_segment(&project.segments[0]));
}

#[test]
fn domain_construction_does_not_enable_optional_lint_or_tags() {
    let project = project_with_segments(vec![segment("empty", 1.0, 2.0, "")]);
    let mut environment =
        exaudio_babel_domain::document_environment(&project, "review", "exaudio.babel").unwrap();
    assert!(environment.providers.diagnostics.is_empty());
    assert!(environment.providers.completions.is_empty());
    environment.providers = exaudio_babel_domain::domain_providers(&project);
    assert!(environment.providers.completions.is_empty());
    let core = CoreSession::for_project_with_environment(&project, environment);
    assert!(core.transcript_rows(&project)[0].lint.is_none());
}

#[test]
fn server_arbitration_preserves_raw_local_results_and_unrelated_provider_identity() {
    use exaudio_extension::document::DiagnosticCollection;
    let project = project_with_segments(vec![segment("empty", 1.0, 2.0, "")]);
    let mut core = CoreSession::for_project_with_environment(&project, test_environment(&project));
    let local = core
        .diagnostics()
        .iter()
        .flat_map(|collection| &collection.diagnostics)
        .find(|issue| issue.source == "babel.gold")
        .unwrap()
        .clone();
    let mut third_party = local.clone();
    third_party.source = "independent.provider".into();
    assert!(core.publish_diagnostics(
        &project,
        DiagnosticCollection {
            source: third_party.source.clone(),
            revision: 0,
            diagnostics: vec![third_party.clone()],
        }
    ));
    let mut state = BabelDocumentState::for_project(&project);
    assert!(state.bind(&mut core).set_server_lint_snapshot(
        &project,
        ServerLintSnapshot::new(
            0,
            vec![ServerLintIssue {
                annotation_id: "empty".into(),
                reason: local.message.clone(),
                severity: LintSeverity::Error,
            },]
        )
    ));
    let issues: Vec<_> = core
        .diagnostics()
        .iter()
        .flat_map(|collection| &collection.diagnostics)
        .collect();
    assert_eq!(issues[0].source, "babel.server");
    assert!(!issues.iter().any(|issue| **issue == local));
    assert!(issues.iter().any(|issue| **issue == third_party));
    assert!(
        state
            .bind(&mut core)
            .set_server_lint_snapshot(&project, ServerLintSnapshot::new(0, Vec::new()))
    );
    let issues: Vec<_> = core
        .diagnostics()
        .iter()
        .flat_map(|collection| &collection.diagnostics)
        .collect();
    assert!(issues.iter().any(|issue| **issue == local));
    assert!(issues.iter().any(|issue| **issue == third_party));
}
