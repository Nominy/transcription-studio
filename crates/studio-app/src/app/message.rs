use std::time::Instant;

use iced::widget::text_editor;
use iced::{Event, Point, event, keyboard::Modifiers, window};
use studio_app::{LoopStripHit, ServerLintIssue, TrackFilter};
use studio_waveform::WaveformHit;

use super::babel_audio::{BabelPayloadLoadKind, PreparedBabelAudioPayload};
use super::browser_input::OutsideInteractionContext;
use super::keybinds::{HotkeyAction, KeyBinding};
use super::review_backend_client::ReviewBackendServiceRequest;
use super::service_callbacks::{
    BabelPayloadRequestToken, FeedbackBootstrapInputs, FeedbackBootstrapToken,
    FeedbackDraftBootstrap, FeedbackDraftSaveToken, SaveRequestToken, SubmitRequestToken,
};
use super::state::{DashboardSection, NativeCapabilityModalTab};
use super::subaccounts::{SubaccountOwnerDashboard, SubaccountOwnerRequest, SubaccountSession};
use super::tag_suggestions::{TagSuggestion, TagSuggestionKeyboardAction};
use super::time_edit::TimeField;
use super::updater::UpdateCheckSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RuntimeGraphicsInfo {
    pub(super) backend: String,
    pub(super) adapter: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum AuditionKey {
    BeforeStart,
    AfterStart,
    BeforeEnd,
    AfterEnd,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Message {
    RuntimeEvent {
        event: Event,
        status: event::Status,
        window: window::Id,
    },
    KeyboardModifiersChanged(Modifiers),
    RightShiftChanged(bool),
    AuditionKeyChanged(AuditionKey, bool),
    AuditionKeyPressed {
        key: AuditionKey,
        release_token: String,
    },
    AuditionKeyReleaseToken(String),
    #[allow(dead_code)]
    ClearAuditionKeys,
    WindowUnfocused,
    GlobalPointerReleased,
    LoadBundledDemo,
    ImportPathChanged(String),
    ExportPathChanged(String),
    ImportBabelJson,
    ExportBabelJson,
    RequestSubmitReview,
    CancelSubmitReview,
    SubmitReview,
    #[allow(dead_code)]
    LintAnnotationsSucceeded {
        token: SubmitRequestToken,
        issues: Vec<ServerLintIssue>,
    },
    #[allow(dead_code)]
    LintAnnotationsFailed {
        token: SubmitRequestToken,
        error: String,
        issues: Option<Vec<ServerLintIssue>>,
    },
    #[allow(dead_code)]
    SubmitReviewSucceeded {
        token: SubmitRequestToken,
    },
    #[allow(dead_code)]
    SubmitReviewFailed {
        token: SubmitRequestToken,
        error: String,
        issues: Option<Vec<ServerLintIssue>>,
    },
    SaveProgress,
    #[allow(dead_code)]
    SaveProgressSucceeded {
        token: SaveRequestToken,
        last_saved: String,
        issues: Option<Vec<ServerLintIssue>>,
    },
    #[allow(dead_code)]
    SaveProgressFailed {
        token: SaveRequestToken,
        error: String,
        issues: Option<Vec<ServerLintIssue>>,
    },
    DropClaim,
    #[allow(dead_code)]
    DropClaimSucceeded,
    #[allow(dead_code)]
    DropClaimFailed(String),
    RequestSkipChunk,
    CancelSkipChunk,
    ConfirmSkipChunk,
    #[allow(dead_code)]
    SkipChunkSucceeded,
    #[allow(dead_code)]
    SkipChunkFailed(String),
    RequestAdminFlag,
    AdminFlagReasonChanged(String),
    CancelAdminFlag,
    ConfirmAdminFlag,
    NativeAuthIdentifierChanged(String),
    NativeAuthCodeChanged(String),
    NativeAuthStartLogin,
    NativeAuthStartLoginFinished(Result<(String, Option<String>), String>),
    NativeAuthVerifyLogin,
    NativeAuthVerifyLoginFinished(Result<(), String>),
    NativeAuthBackToLogin,
    NativeAuthUseSubaccountMode,
    NativeAuthUseBabelMode,
    NativeAuthDetachStore,
    NativeAuthSubaccountHostChanged(String),
    NativeAuthSubaccountCodeChanged(String),
    NativeAuthSubaccountLogin,
    NativeAuthSubaccountLoginFinished(Result<SubaccountSession, String>),
    SubaccountSignOut,
    DashboardSectionChanged(DashboardSection),
    SubaccountOwnerHostUrlChanged(String),
    SaveSubaccountRelayUrl,
    StartSubaccountRelay,
    SubaccountRelayStarted(Result<String, String>),
    StopSubaccountRelay,
    #[cfg_attr(not(windows), allow(dead_code))]
    TrayRestoreRequested,
    #[cfg_attr(not(windows), allow(dead_code))]
    TrayStartRelay,
    #[cfg_attr(not(windows), allow(dead_code))]
    TrayStopRelay,
    #[cfg_attr(not(windows), allow(dead_code))]
    TrayExitRequested,
    SubaccountOwnerCreateLabelChanged(String),
    SubaccountOwnerRefresh,
    SubaccountOwnerCreate,
    SubaccountOwnerRevoke(String),
    SubaccountOwnerAssignmentChanged(String, String),
    SubaccountOwnerSaveAssignment(String),
    SubaccountOwnerProjectToggled(String, String, bool),
    SubaccountOwnerDashboardLoaded(SubaccountOwnerDashboard),
    SubaccountOwnerProjectQueuesPushFinished(SubaccountOwnerRequest, Result<(), String>),
    SubaccountOwnerRequestFailed(String),
    DashboardRefreshProjects,
    DashboardRecoverLatestDraft,
    DashboardProjectsLoaded(String),
    DashboardProjectsLoadFailed(String),
    #[allow(dead_code)]
    DashboardChunkedQueueChanged(String),
    #[allow(dead_code)]
    DashboardStitchedQueueChanged(String),
    #[allow(dead_code)]
    DashboardReviewActionChanged(String),
    #[allow(dead_code)]
    DashboardProcessedTranscriptionChanged(String),
    DashboardStartProject(String),
    #[allow(dead_code)]
    DashboardStartNextTask,
    #[allow(dead_code)]
    DashboardLoadReviewAction,
    #[allow(dead_code)]
    DashboardLoadProcessedTranscription,
    #[allow(dead_code)]
    DashboardClaimProcessedTranscription,
    CheckForUpdates,
    InstallAvailableUpdate,
    UpdateCheckFinished(Result<UpdateCheckSummary, String>),
    UpdateInstallFinished(Result<(), String>),
    RuntimeGraphicsInfoLoaded(RuntimeGraphicsInfo),
    CaptureRendererSmokeScreenshot,
    RendererSmokeScreenshotSaved(Result<String, String>),
    #[allow(dead_code)]
    AdminFlagSucceeded,
    #[allow(dead_code)]
    AdminFlagFailed(String),
    RetryPractice,
    TakeAssessment,
    RetryAssessment,
    BackToProjects,
    OpenFeedbackPanel,
    RetryFeedbackBootstrap,
    #[allow(dead_code)]
    FeedbackRubricFlagsLoaded {
        token: FeedbackBootstrapToken,
        rubric_flag_names: Vec<String>,
    },
    #[allow(dead_code)]
    FeedbackRubricFlagsLoadFailed {
        token: FeedbackBootstrapToken,
        error: String,
    },
    #[allow(dead_code)]
    FeedbackFormLoaded {
        token: FeedbackBootstrapToken,
        form_step_id: String,
    },
    #[allow(dead_code)]
    FeedbackFormLoadFailed {
        token: FeedbackBootstrapToken,
        error: String,
    },
    #[allow(dead_code)]
    FeedbackInputsLoaded {
        token: FeedbackBootstrapToken,
        result: FeedbackBootstrapInputs,
    },
    #[allow(dead_code)]
    FeedbackInputsLoadFailed {
        token: FeedbackBootstrapToken,
        error: String,
    },
    #[allow(dead_code)]
    FeedbackDraftLoaded {
        token: FeedbackBootstrapToken,
        draft: FeedbackDraftBootstrap,
    },
    #[allow(dead_code)]
    FeedbackDraftLoadFailed {
        token: FeedbackBootstrapToken,
        error: String,
    },
    CloseFeedbackPanel,
    FeedbackRatingChanged(String, u8),
    FeedbackCommentEdited(String, text_editor::Action),
    FeedbackOtherFeedbackEdited(text_editor::Action),
    #[allow(dead_code)]
    FeedbackDraftSaved {
        token: FeedbackDraftSaveToken,
        last_saved: String,
    },
    #[allow(dead_code)]
    FeedbackDraftSaveFailed {
        token: FeedbackDraftSaveToken,
        error: String,
    },
    TaskLookupProcessedTranscriptionChanged(String),
    TaskLookupReviewActionChanged(String),
    TaskLookupClickClaimStitched,
    TaskLookupCancelConfirmEdit,
    TaskLookupConfirmEdit,
    TaskLookupClickReadOnly,
    CloseTaskLookup,
    WindowCloseRequested,
    HideWindowRequested(Option<window::Id>),
    RestoreWindowRequested(Option<window::Id>),
    CancelPendingLeave,
    SaveAndContinuePendingLeave,
    DiscardAndContinuePendingLeave,
    #[allow(dead_code)]
    LoadReviewActionSucceeded {
        token: BabelPayloadRequestToken,
        source: String,
    },
    #[allow(dead_code)]
    LoadReviewActionFailed {
        token: BabelPayloadRequestToken,
        error: String,
    },
    #[allow(dead_code)]
    LoadLatestReviewActionSucceeded {
        token: BabelPayloadRequestToken,
        source: String,
    },
    #[allow(dead_code)]
    LoadLatestReviewActionFailed {
        token: BabelPayloadRequestToken,
        error: String,
    },
    #[allow(dead_code)]
    ClaimByProcessedTranscriptionSucceeded {
        token: BabelPayloadRequestToken,
        source: String,
    },
    #[allow(dead_code)]
    ClaimByProcessedTranscriptionFailed {
        token: BabelPayloadRequestToken,
        error: String,
    },
    #[allow(dead_code)]
    ClaimNextFromQueueSucceeded {
        token: BabelPayloadRequestToken,
        source: String,
    },
    #[allow(dead_code)]
    ClaimNextFromQueueFailed {
        token: BabelPayloadRequestToken,
        error: String,
    },
    #[allow(dead_code)]
    BabelAudioPrepared {
        token: BabelPayloadRequestToken,
        kind: BabelPayloadLoadKind,
        route_metadata: Vec<(String, String)>,
        result: Result<PreparedBabelAudioPayload, String>,
    },
    CreateRegionTrackChanged(String),
    CreateRegionStartChanged(String),
    CreateRegionEndChanged(String),
    CreateRegionTextChanged(String),
    CreateRegionPrimaryCategoryChanged(String),
    CreateRegionSecondaryCategoryChanged(String),
    CreateRegionCustomDescriptionChanged(String),
    CreateRegionIntensityChanged(String),
    CreateRegionSelectOpenChanged(bool),
    CreateRegionInteractOutside(OutsideInteractionContext),
    CommitCreateRegionStart,
    CommitCreateRegionEnd,
    CreateRegionInputEscapePressed,
    CancelCreateRegion,
    SubmitCreateRegion,
    RequestEditAnnotation(String),
    EditAnnotationPrimaryCategoryChanged(String),
    EditAnnotationSecondaryCategoryChanged(String),
    EditAnnotationCustomDescriptionChanged(String),
    EditAnnotationIntensityChanged(String),
    EditAnnotationSelectOpenChanged(bool),
    EditAnnotationOutsideMouseDown(OutsideInteractionContext),
    CancelEditAnnotation,
    SubmitEditAnnotation,
    CopySupportId,
    ResetSupportIdCopied,
    ToggleContextPanel(String),
    CopySubmissionId(String),
    ResetSubmissionIdCopied,
    ShowHotkeys,
    CloseHotkeys,
    StartHotkeyCapture(HotkeyAction),
    CancelHotkeyCapture,
    ResetHotkeyBinding(HotkeyAction),
    HotkeyBindingCaptured(HotkeyAction, KeyBinding),
    OpenFind,
    CloseFind,
    FindQueryChanged(String),
    FindNext,
    FindPrevious,
    EscapePressed,
    ActivateDefaultSurfaceAction,
    GhostCursorRowNavigation(i32),
    HelperTogglePlayback,
    TogglePlayback,
    TransportTick(Instant),
    PreviousSegment,
    NextSegment,
    SpeedChanged(String),
    ZoomChanged(f32),
    GlobalPointerMoved(Point),
    OverviewPointerMoved {
        fraction: f32,
        width: f32,
    },
    OverviewExited,
    OverviewPressed,
    OverviewReleased,
    TimelineScrollbarHovered {
        fraction: f32,
        width: f32,
    },
    TimelineScrollbarExited,
    TimelineScrollbarPressed,
    TimelineScrollbarReleased,
    TimelineAutoscrollPressed(String),
    TimelineAutoscrollReleased,
    TimelineWheelScrolled {
        delta_x: f32,
        delta_y: f32,
    },
    SeekToTime(f64),
    ToggleDiffView,
    ToggleDiffReviewers,
    DiffCompareActionChanged(String),
    NavigateProblemRows(i32),
    ToggleLoop,
    ToggleOverlay,
    LoopStripHovered(LoopStripHit),
    LoopStripPressed,
    LoopStripReleased,
    ToggleLaneMute(String),
    LaneVerticalZoomSliderPointerMoved {
        lane_id: String,
        fraction: f32,
        height: f32,
    },
    LaneVerticalZoomSliderPressed(String),
    LaneVerticalZoomSliderReleased,
    LaneVerticalZoomInputChanged(String),
    SubmitLaneVerticalZoomInput,
    TrackFilterChanged(TrackFilter),
    TranscriptScrolled {
        offset_y: f32,
        viewport_height: f32,
    },
    TranscriptTextWheelScrolled {
        delta_y: f32,
    },
    TranscriptRowBodyPressed(String),
    TranscriptRowHovered(String),
    TranscriptRowExited(String),
    WaveformHitHovered(String, WaveformHit),
    WaveformHitExited(String),
    WaveformHitPressed(String),
    WaveformHitReleased(String),
    ToggleLaneCollapse(String),
    ToggleActions(String),
    Undo,
    Redo,
    SegmentTimeChanged(String, TimeField, String),
    CommitSegmentTimeEdit {
        segment_id: String,
        field: TimeField,
        from_blur: bool,
    },
    SegmentTrackCellPressed(String),
    SegmentTrackSelectOpenChanged(String, bool),
    SegmentTrackChanged(String, String),
    SegmentTextEdited(String, text_editor::Action),
    TranscriptEditorEscapePressed(String),
    SegmentTextUndo(String),
    SegmentTextRedo(String),
    AdjustPlaybackSpeed(i32),
    RewindPlaybackOneSecond,
    SwitchSpeakerWorkflowLane(String),
    ResetSpeakerWorkflow,
    RightShiftSegmentNavigation(i32),
    TrimCurrentSegmentToAudio,
    TrimAllVisibleSegmentsToAudio,
    AutoInsertSegmentAtCaret,
    AutoSegmentVisibleSilences,
    CommitCutPreviewOverlay {
        smart_split: bool,
    },
    LoopCutPreviewOverlay,
    DeleteSelectedSegment,
    MergeSelectedSegmentAbove,
    MergeSelectedSegmentBelow,
    MoveTextBeforeCaretToPreviousSegment(String),
    MoveTextAfterCaretToNextSegment(String),
    ConvertSelectedNumberToSkaz,
    ConvertSelectionToSkazWithDigit(String, char),
    AutofixCurrentLintRow,
    AutofixAllLintRows,
    ToggleNativeCapabilityPanel,
    NativeCapabilityModalTabChanged(NativeCapabilityModalTab),
    ReviewerGenerateReview,
    ReviewerCreateSession,
    ReviewerSessionIdChanged(String),
    ReviewerLoadSession,
    ReviewerSessionCommentChanged(String),
    ReviewerCardIdChanged(String),
    ReviewerCardCommentChanged(String),
    ReviewerApplySelectedCardFeedback,
    ReviewerSaveComments,
    ReviewerCommentSaveTick,
    ReviewerTemplateQueryChanged(String),
    ReviewerSearchTemplates,
    ReviewerTemplateIdChanged(String),
    ReviewerSetTemplateMatch,
    ReviewerClearTemplateMatch,
    ReviewerGenerateTemplateSuggestions,
    ReviewerExpandAllCards,
    ReviewerCollapseAllCards,
    ReviewerToggleSuggestionsExpanded,
    ReviewerProposalIdChanged(String),
    ReviewerApproveTemplateSuggestion,
    ReviewerRejectTemplateSuggestion,
    ReviewerFinalizeApply,
    ReviewerFinalizeSkip,
    ReviewerSubmitAnalytics,
    ReviewerHistoryQueryChanged(String),
    ReviewerLoadHistory,
    DraftOpenRouterApiKeyChanged(String),
    DraftModelChanged(String),
    DraftServiceTierChanged(String),
    DraftReasoningEffortChanged(String),
    DraftSessionIdChanged(String),
    DraftCheckBackendStatus,
    DraftGenerateRows,
    DraftRetryAudioCapture,
    DraftUseCapturedAudio,
    DraftContinueTextOnly,
    DraftTranscribeSelectedSegment,
    DraftRedistributeText,
    DraftApplyLastRows,
    DraftApplySegmentText,
    DraftApplyRow(String),
    DraftSkipRow(String),
    DraftApplyRedistributionGroup(String),
    ReviewBackendRequestSucceeded(ReviewBackendServiceRequest, String),
    ReviewBackendRequestFailed(ReviewBackendServiceRequest, String),
    ApplyTagSuggestion(String, TagSuggestion),
    TranscriptTagSuggestionKeyPressed(String, TagSuggestionKeyboardAction),
    AcknowledgeLowConfidenceWord(String, usize),
    ToggleActiveWarningAssertion,
    ToggleWarningAssertion(String),
    MergeSegmentAbove(String),
    MergeSegmentBelow(String),
    DeleteSegment(String),
    CreateSegmentBefore(String),
    CreateSegmentAfter(String),
}
