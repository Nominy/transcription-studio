use iced::widget::{scrollable, text_input};
use iced::{Task, clipboard};
use studio_app::{EditorSession, TaskLookupEvent, TimeRange, resolve_submission_id_copy_success};
use time::OffsetDateTime;

use super::browser_behavior_smoke::{
    browser_behavior_renderer_screenshot_task, browser_behavior_smoke_enabled,
};
use super::components::{
    admin_flag_reason_input_id, focusable_text_editor, lane_vertical_zoom_input_id,
};
use super::diff_smoke::{diff_workbench_renderer_screenshot_task, diff_workbench_smoke_mode};
use super::find;
use super::formatting::{parse_speed, zoom_factor_after_timeline_wheel_pixels};
use super::hotkeys;
use super::large_project_smoke::{
    large_project_renderer_screenshot_task, large_project_smoke_enabled,
};
use super::native_auth::{
    NativeAuthStore, default_native_auth_store_path, detach_native_auth_store_to_executable,
    native_clerk_url, save_native_auth_store, start_phone_code_login, verify_phone_code_login,
};
use super::native_auth_cli::native_babel_url;
use super::state::PendingLeaveIntent;
use super::subaccounts::login_with_subaccount_code;
use super::tag_suggestions;
use super::updater::{check_for_updates, install_available_update};
use super::workbench_shell_smoke::{
    workbench_shell_renderer_screenshot_task, workbench_shell_smoke_enabled,
};
use super::{Message, NativeAuthFlowState, StudioState};

pub(super) fn toolbar_zoom_runtime_trace_line(previous: f32, current: f32) -> String {
    format!("Transcription Studio toolbar: action=zoom previous={previous:.3} current={current:.3}")
}

pub(super) fn toolbar_speed_runtime_trace_line(previous: f32, current: f32) -> String {
    format!(
        "Transcription Studio toolbar: action=speed previous={previous:.2}x current={current:.2}x"
    )
}

pub(super) fn transcript_action_toggle_runtime_trace_line(
    segment_id: &str,
    was_open: bool,
    is_open: bool,
) -> String {
    format!("Transcription Studio action menu: segment={segment_id} open={was_open}->{is_open}")
}

pub(super) fn problem_navigation_runtime_trace_line(
    direction: i32,
    previous_segment_id: Option<&str>,
    selected_segment_id: Option<&str>,
    problem_index: usize,
    problem_count: usize,
) -> String {
    format!(
        "Transcription Studio problem navigation: direction={direction} previous={} selected={} index={problem_index} problems={problem_count}",
        previous_segment_id.unwrap_or("-"),
        selected_segment_id.unwrap_or("-")
    )
}

fn toolbar_trace_enabled() -> bool {
    std::env::var("STUDIO_TRACE_TOOLBAR").is_ok_and(|value| value != "0")
}

fn transcript_scroll_trace_enabled() -> bool {
    std::env::var("STUDIO_TRACE_TRANSCRIPT_SCROLL").is_ok_and(|value| value != "0")
}

fn transcript_action_trace_enabled() -> bool {
    std::env::var("STUDIO_TRACE_ACTION_MENU").is_ok_and(|value| value != "0")
}

fn problem_navigation_trace_enabled() -> bool {
    std::env::var("STUDIO_TRACE_PROBLEM_NAV").is_ok_and(|value| value != "0")
}

pub(super) fn timeline_wheel_runtime_trace_line(
    route: &str,
    delta_x: f32,
    delta_y: f32,
    previous_zoom: f32,
    current_zoom: f32,
    previous_viewport: TimeRange,
    current_viewport: TimeRange,
) -> String {
    format!(
        "Transcription Studio timeline wheel: route={route} delta_x_px={delta_x:.3} delta_y_px={delta_y:.3} zoom={previous_zoom:.3}->{current_zoom:.3} viewport={:.3}-{:.3}s->{:.3}-{:.3}s",
        previous_viewport.start_seconds,
        previous_viewport.end_seconds,
        current_viewport.start_seconds,
        current_viewport.end_seconds
    )
}

fn timeline_wheel_trace_enabled() -> bool {
    std::env::var("STUDIO_TRACE_TIMELINE_WHEEL").is_ok_and(|value| value != "0")
}

fn timeline_wheel_trace_state(state: &StudioState) -> Option<(f32, TimeRange)> {
    state
        .editor
        .as_ref()
        .map(|editor| (editor.zoom, editor.viewport))
}

fn trace_timeline_wheel_route(
    state: &StudioState,
    route: &str,
    delta_x: f32,
    delta_y: f32,
    previous: Option<(f32, TimeRange)>,
) {
    if !timeline_wheel_trace_enabled() {
        return;
    }
    let Some((previous_zoom, previous_viewport)) = previous else {
        return;
    };
    let Some((current_zoom, current_viewport)) = timeline_wheel_trace_state(state) else {
        return;
    };
    eprintln!(
        "{}",
        timeline_wheel_runtime_trace_line(
            route,
            delta_x,
            delta_y,
            previous_zoom,
            current_zoom,
            previous_viewport,
            current_viewport
        )
    );
}

pub(crate) fn update(state: &mut StudioState, message: Message) -> Task<Message> {
    match message {
        Message::RuntimeEvent {
            event,
            status,
            window,
        } => {
            if let Some(routed_message) =
                hotkeys::event_message_for_state(state, event, status, window)
            {
                return update(state, routed_message);
            }
        }
        Message::KeyboardModifiersChanged(modifiers) => {
            state.keyboard_modifiers = modifiers;
            if !modifiers.shift() {
                state.right_shift_pressed = false;
            }
        }
        Message::RightShiftChanged(is_pressed) => state.right_shift_pressed = is_pressed,
        Message::AuditionKeyChanged(key, is_pressed) => {
            if is_pressed {
                state.held_audition_keys.insert(key);
            } else {
                state.held_audition_keys.remove(&key);
                forget_audition_key_release_tokens(state, key);
            }
        }
        Message::AuditionKeyPressed { key, release_token } => {
            state.held_audition_keys.insert(key);
            state
                .audition_key_release_tokens
                .entry(release_token)
                .or_default()
                .insert(key);
        }
        Message::AuditionKeyReleaseToken(release_token) => {
            if let Some(keys) = state.audition_key_release_tokens.remove(&release_token) {
                for key in keys {
                    state.held_audition_keys.remove(&key);
                }
            }
        }
        Message::ClearAuditionKeys => {
            state.held_audition_keys.clear();
            state.audition_key_release_tokens.clear();
        }
        Message::WindowUnfocused => {
            state.keyboard_modifiers = iced::keyboard::Modifiers::empty();
            state.right_shift_pressed = false;
            state.held_audition_keys.clear();
            state.audition_key_release_tokens.clear();
            state.release_captured_pointer();
        }
        Message::GlobalPointerReleased => state.release_captured_pointer(),
        Message::LoadBundledDemo => state.load_bundled_demo(),
        Message::ImportPathChanged(path) => state.import_path = path,
        Message::ExportPathChanged(path) => state.export_path = path,
        Message::ImportBabelJson => state.import_babel_json(),
        Message::ExportBabelJson => state.export_babel_json(),
        Message::RequestSubmitReview => state.request_submit_review(),
        Message::CancelSubmitReview => {
            if !state.submit_is_pending {
                state.show_submit_dialog = false;
            }
        }
        Message::SubmitReview => {
            state.begin_submit_review_request();
        }
        Message::LintAnnotationsSucceeded { token, issues } => {
            state.complete_lint_annotations_success(token, issues);
        }
        Message::LintAnnotationsFailed {
            token,
            error,
            issues,
        } => {
            state.complete_lint_annotations_failure(token, error, issues);
        }
        Message::SubmitReviewSucceeded { token } => {
            state.complete_submit_review_success(token);
        }
        Message::SubmitReviewFailed {
            token,
            error,
            issues,
        } => state.complete_submit_review_failure(token, error, issues),
        Message::SaveProgress => state.save_progress(),
        Message::SaveProgressSucceeded {
            token,
            last_saved,
            issues,
        } => {
            if state.complete_save_progress_success(token, last_saved, issues)
                && state.pending_leave_waits_for_save()
            {
                if state.editor.as_ref().is_some_and(|editor| editor.is_dirty) {
                    state.save_progress();
                }
                if let Some(intent) = state.take_ready_pending_leave() {
                    return execute_leave_intent(state, intent);
                }
            }
        }
        Message::SaveProgressFailed {
            token,
            error,
            issues,
        } => {
            state.complete_save_progress_failure(token, error, issues);
        }
        Message::AutoSaveTick => state.auto_save_progress(),
        Message::DropClaim => {
            state.begin_drop_claim_request();
        }
        Message::DropClaimSucceeded => state.complete_drop_claim_success(),
        Message::DropClaimFailed(error) => state.complete_drop_claim_failure(error),
        Message::RequestSkipChunk => state.request_skip_chunk(),
        Message::CancelSkipChunk => state.cancel_skip_chunk(),
        Message::ConfirmSkipChunk => {
            state.begin_skip_chunk_request();
        }
        Message::SkipChunkSucceeded => state.confirm_skip_chunk(),
        Message::SkipChunkFailed(error) => state.fail_skip_chunk(error),
        Message::RequestAdminFlag => {
            state.request_admin_flag();
            if state.show_admin_flag_dialog {
                return dispatch_pending_services(
                    state,
                    text_input::focus(admin_flag_reason_input_id()),
                );
            }
        }
        Message::AdminFlagReasonChanged(reason) => {
            if !state.admin_flag_is_pending {
                state.admin_flag_reason = reason;
            }
        }
        Message::CancelAdminFlag => state.cancel_admin_flag(),
        Message::ConfirmAdminFlag => {
            state.begin_admin_flag_request();
        }
        Message::AdminFlagSucceeded => state.confirm_admin_flag(),
        Message::AdminFlagFailed(error) => state.fail_admin_flag(error),
        Message::NativeAuthIdentifierChanged(identifier) => {
            if let Some(flow) = state.native_auth_flow.as_mut()
                && !flow.is_pending
            {
                flow.identifier = identifier;
                flow.error = None;
            }
        }
        Message::NativeAuthCodeChanged(code) => {
            if let Some(flow) = state.native_auth_flow.as_mut()
                && !flow.is_pending
            {
                flow.code = code;
                flow.error = None;
            }
        }
        Message::NativeAuthStartLogin => {
            if let Some(flow) = state.native_auth_flow.as_mut() {
                if !flow.can_start_login() {
                    return dispatch_pending_services(state, Task::none());
                }
                let identifier = flow.identifier.trim().to_string();
                flow.is_pending = true;
                flow.error = None;
                state.status = "Sending Babel sign-in code...".to_string();
                return dispatch_pending_services(
                    state,
                    Task::perform(
                        start_native_auth_login(identifier),
                        Message::NativeAuthStartLoginFinished,
                    ),
                );
            }
        }
        Message::NativeAuthStartLoginFinished(result) => match result {
            Ok((sign_in_id, phone_number_id)) => {
                state.native_auth_flow = Some(NativeAuthFlowState::pending_code(
                    sign_in_id,
                    phone_number_id,
                ));
                state.status = "Babel sign-in code sent.".to_string();
            }
            Err(error) => {
                let mut flow = state
                    .native_auth_flow
                    .take()
                    .unwrap_or_else(NativeAuthFlowState::login_prompt);
                flow.is_pending = false;
                flow.error = Some(error.clone());
                state.native_auth_flow = Some(flow);
                state.status = format!("Babel sign-in failed: {error}");
            }
        },
        Message::NativeAuthVerifyLogin => {
            if let Some(flow) = state.native_auth_flow.as_mut() {
                if !flow.can_submit_code() {
                    return dispatch_pending_services(state, Task::none());
                }
                let code = flow.code.trim().to_string();
                flow.is_pending = true;
                flow.error = None;
                state.status = "Verifying Babel sign-in code...".to_string();
                return dispatch_pending_services(
                    state,
                    Task::perform(
                        verify_native_auth_code(code),
                        Message::NativeAuthVerifyLoginFinished,
                    ),
                );
            }
        }
        Message::NativeAuthVerifyLoginFinished(result) => match result {
            Ok(()) => {
                state.native_auth_flow = None;
                state.enter_dashboard();
                state.queue_dashboard_projects_refresh();
            }
            Err(error) => {
                if let Some(flow) = state.native_auth_flow.as_mut() {
                    flow.is_pending = false;
                    flow.error = Some(error.clone());
                }
                state.status = format!("Babel auth failed: {error}");
            }
        },
        Message::NativeAuthBackToLogin => {
            if state
                .native_auth_flow
                .as_ref()
                .is_some_and(|flow| !flow.is_pending)
            {
                match reset_native_auth_store() {
                    Ok(()) => {
                        state.native_auth_flow = Some(NativeAuthFlowState::login_prompt());
                        state.status = "Babel sign-in restarted.".to_string();
                    }
                    Err(error) => {
                        if let Some(flow) = state.native_auth_flow.as_mut() {
                            flow.error = Some(error.clone());
                        }
                        state.status = format!("Babel sign-in restart failed: {error}");
                    }
                }
            }
        }
        Message::NativeAuthUseSubaccountMode => {
            if state
                .native_auth_flow
                .as_ref()
                .is_none_or(|flow| !flow.is_pending)
            {
                state.open_subaccount_login();
            }
        }
        Message::NativeAuthUseBabelMode => {
            if state
                .native_auth_flow
                .as_ref()
                .is_none_or(|flow| !flow.is_pending)
            {
                state.native_auth_flow = Some(NativeAuthFlowState::login_prompt());
                state.status = "Babel sign-in selected.".to_string();
            }
        }
        Message::NativeAuthDetachStore => match detach_native_auth_store_to_executable() {
            Ok(path) => {
                let status = format!("Babel auth store detached to {}.", path.display());
                state.open_native_auth_gate_from_store();
                if state.native_auth_flow.is_some() {
                    state.status = format!("{status} Sign in to populate it.");
                } else {
                    state.status = status;
                }
            }
            Err(error) => {
                state.status = format!("Babel auth store detach failed: {error}");
            }
        },
        Message::NativeAuthSubaccountHostChanged(host_url) => {
            if let Some(flow) = state.native_auth_flow.as_mut()
                && !flow.is_pending
            {
                flow.subaccount_host_url = host_url;
                flow.error = None;
            }
        }
        Message::NativeAuthSubaccountCodeChanged(code) => {
            if let Some(flow) = state.native_auth_flow.as_mut()
                && !flow.is_pending
            {
                flow.subaccount_code = code;
                flow.error = None;
            }
        }
        Message::NativeAuthSubaccountLogin => {
            if let Some(flow) = state.native_auth_flow.as_mut() {
                if !flow.can_submit_subaccount_login() {
                    return dispatch_pending_services(state, Task::none());
                }
                let host_url = flow.subaccount_host_url.trim().to_string();
                let code = flow.subaccount_code.trim().to_string();
                flow.is_pending = true;
                flow.error = None;
                state.status = "Signing in to Sub-account relay...".to_string();
                return dispatch_pending_services(
                    state,
                    Task::perform(
                        login_with_subaccount_code(host_url, code),
                        Message::NativeAuthSubaccountLoginFinished,
                    ),
                );
            }
        }
        Message::NativeAuthSubaccountLoginFinished(result) => match result {
            Ok(session) => {
                state.set_subaccount_session(session);
            }
            Err(error) => {
                if let Some(flow) = state.native_auth_flow.as_mut() {
                    flow.is_pending = false;
                    flow.error = Some(error.clone());
                }
                state.status = format!("Sub-account sign-in failed: {error}");
            }
        },
        Message::SubaccountSignOut => {
            if let Err(error) = state.clear_subaccount_session() {
                state.status = format!("Sub-account sign-out failed: {error}");
            }
        }
        Message::DashboardSectionChanged(section) => {
            state.set_dashboard_section(section);
        }
        Message::SubaccountOwnerHostUrlChanged(value) => {
            if !state.subaccount_owner.is_pending {
                state.subaccount_owner.host_url = value;
                state.subaccount_owner.error = None;
            }
        }
        Message::SubaccountOwnerCreateLabelChanged(value) => {
            if !state.subaccount_owner.is_pending {
                state.subaccount_owner.create_label = value;
                state.subaccount_owner.error = None;
            }
        }
        Message::SubaccountOwnerRefresh => {
            state.queue_subaccount_owner_refresh();
        }
        Message::SubaccountOwnerCreate => {
            state.begin_subaccount_owner_create();
        }
        Message::SubaccountOwnerRevoke(subaccount_id) => {
            state.begin_subaccount_owner_revoke(&subaccount_id);
        }
        Message::SubaccountOwnerAssignmentChanged(subaccount_id, value) => {
            if !state.subaccount_owner.is_pending {
                state
                    .subaccount_owner
                    .assignment_drafts
                    .insert(subaccount_id, value);
                state.subaccount_owner.error = None;
            }
        }
        Message::SubaccountOwnerProjectToggled(subaccount_id, project_id, selected) => {
            state.toggle_subaccount_owner_project_assignment(&subaccount_id, &project_id, selected);
        }
        Message::SubaccountOwnerSaveAssignment(subaccount_id) => {
            state.begin_subaccount_owner_assignment(&subaccount_id);
        }
        Message::SubaccountOwnerDashboardLoaded(dashboard) => {
            state.complete_subaccount_owner_dashboard(dashboard);
        }
        Message::SubaccountOwnerRequestFailed(error) => {
            state.fail_subaccount_owner_request(error);
        }
        Message::DashboardRefreshProjects => state.queue_dashboard_projects_refresh(),
        Message::DashboardRecoverLatestDraft => state.recover_latest_draft(),
        Message::DashboardProjectsLoaded(source) => {
            state.complete_dashboard_projects_loaded(source)
        }
        Message::DashboardProjectsLoadFailed(error) => {
            state.complete_dashboard_projects_failed(error);
        }
        Message::DashboardChunkedQueueChanged(value) => {
            state.dashboard.chunked_queue_id = value;
            state.dashboard.error = None;
        }
        Message::DashboardStitchedQueueChanged(value) => {
            state.dashboard.stitched_queue_id = value;
            state.dashboard.error = None;
        }
        Message::DashboardReviewActionChanged(value) => {
            state.dashboard.review_action_id = value;
            state.dashboard.error = None;
        }
        Message::DashboardProcessedTranscriptionChanged(value) => {
            state.dashboard.processed_transcription_id = value;
            state.dashboard.error = None;
        }
        Message::DashboardStartProject(project_id) => {
            state.begin_dashboard_start_project(&project_id);
        }
        Message::DashboardStartNextTask => {
            state.begin_dashboard_claim_next_task();
        }
        Message::DashboardLoadReviewAction => {
            state.begin_dashboard_load_review_action();
        }
        Message::DashboardLoadProcessedTranscription => {
            state.begin_dashboard_load_processed_transcription();
        }
        Message::DashboardClaimProcessedTranscription => {
            state.begin_dashboard_claim_processed_transcription();
        }
        Message::CheckForUpdates => {
            if !state.updater.can_check() {
                return dispatch_pending_services(state, Task::none());
            }
            let feed_url = state.updater.feed_url.clone();
            state.updater.begin_check();
            state.status = "Checking for Transcription Studio updates...".to_string();
            return dispatch_pending_services(
                state,
                Task::perform(check_for_updates(feed_url), Message::UpdateCheckFinished),
            );
        }
        Message::InstallAvailableUpdate => {
            if !state.updater.can_install() {
                return dispatch_pending_services(state, Task::none());
            }
            let feed_url = state.updater.feed_url.clone();
            state.updater.begin_install();
            state.status = "Installing Transcription Studio update...".to_string();
            return dispatch_pending_services(
                state,
                Task::perform(
                    install_available_update(feed_url),
                    Message::UpdateInstallFinished,
                ),
            );
        }
        Message::UpdateCheckFinished(result) => {
            state.updater.finish_check(result);
            state.status = state.updater.status_label();
        }
        Message::UpdateInstallFinished(result) => {
            state.updater.finish_install(result);
            state.status = state.updater.status_label();
        }
        Message::RetryPractice => state.status = "Practice retry requested.".to_string(),
        Message::TakeAssessment => state.status = "Assessment transition requested.".to_string(),
        Message::RetryAssessment => state.status = "Assessment retry requested.".to_string(),
        Message::RuntimeGraphicsInfoLoaded(info) => {
            if std::env::var_os("STUDIO_LOG_RUNTIME_GRAPHICS").is_some() {
                eprintln!(
                    "Transcription Studio runtime graphics: backend={} adapter={}",
                    info.backend, info.adapter
                );
            }
            state.runtime_graphics_info = Some(info);
        }
        Message::CaptureRendererSmokeScreenshot => {
            if state.renderer_smoke_screenshot_captured {
                return Task::none();
            }
            state.renderer_smoke_screenshot_captured = true;
            if let Some(mode) = diff_workbench_smoke_mode() {
                return diff_workbench_renderer_screenshot_task(mode);
            }
            if large_project_smoke_enabled() {
                return large_project_renderer_screenshot_task();
            }
            if browser_behavior_smoke_enabled() {
                return browser_behavior_renderer_screenshot_task();
            }
            if workbench_shell_smoke_enabled() {
                return workbench_shell_renderer_screenshot_task();
            }
        }
        Message::RendererSmokeScreenshotSaved(result) => match result {
            Ok(path) => {
                eprintln!("Transcription Studio renderer screenshot: path={path}");
                state.status = format!("Renderer screenshot saved: {path}");
            }
            Err(error) => {
                eprintln!("Transcription Studio renderer screenshot failed: {error}");
                state.status = format!("Renderer screenshot failed: {error}");
            }
        },
        Message::BackToProjects => {
            if let Some(intent) = state.request_leave(PendingLeaveIntent::BackToProjects) {
                return execute_leave_intent(state, intent);
            }
        }
        Message::OpenFeedbackPanel => state.open_feedback_panel(),
        Message::CloseFeedbackPanel => state.close_feedback_panel(),
        Message::FeedbackRatingChanged(field_key, rating) => {
            state.change_feedback_rating(&field_key, rating);
        }
        Message::FeedbackCommentChanged(field_key, comment) => {
            state.change_feedback_comment(&field_key, comment);
        }
        Message::FeedbackOtherFeedbackChanged(feedback) => {
            state.change_feedback_other_feedback(feedback);
        }
        Message::FeedbackDraftSaved(last_saved) => state.mark_feedback_draft_saved(last_saved),
        Message::FeedbackDraftSaveFailed(error) => state.fail_feedback_draft_save(error),
        Message::TaskLookupProcessedTranscriptionChanged(value) => {
            state
                .apply_task_lookup_event(TaskLookupEvent::ChangeProcessedTranscriptionId { value });
        }
        Message::TaskLookupReviewActionChanged(value) => {
            state.apply_task_lookup_event(TaskLookupEvent::ChangeReviewActionId { value });
        }
        Message::TaskLookupClickClaimStitched => {
            state.apply_task_lookup_event(TaskLookupEvent::ClickClaimStitched);
        }
        Message::TaskLookupCancelConfirmEdit => {
            state.apply_task_lookup_event(TaskLookupEvent::CancelConfirmEdit);
        }
        Message::TaskLookupConfirmEdit => {
            state.apply_task_lookup_event(TaskLookupEvent::ConfirmEdit);
        }
        Message::TaskLookupClickReadOnly => {
            state.apply_task_lookup_event(TaskLookupEvent::ClickReadOnly);
        }
        Message::CloseTaskLookup => state.task_lookup = None,
        Message::WindowCloseRequested => {
            if let Some(intent) = state.request_leave(PendingLeaveIntent::ExitApplication) {
                return execute_leave_intent(state, intent);
            }
        }
        Message::CancelPendingLeave => state.cancel_pending_leave(),
        Message::SaveAndContinuePendingLeave => {
            state.begin_pending_leave_save();
            state.save_progress();
            if let Some(intent) = state.take_ready_pending_leave() {
                return execute_leave_intent(state, intent);
            }
        }
        Message::DiscardAndContinuePendingLeave => {
            if let Some(intent) = state.discard_pending_leave() {
                return execute_leave_intent(state, intent);
            }
        }
        Message::LoadReviewActionSucceeded(source) => {
            let task = state.complete_load_review_action_success(source);
            return dispatch_pending_services(state, task);
        }
        Message::LoadReviewActionFailed(error) => {
            state.fail_load_review_action(error);
        }
        Message::LoadLatestReviewActionSucceeded(source) => {
            let task = state.complete_load_latest_review_action_success(source);
            return dispatch_pending_services(state, task);
        }
        Message::LoadLatestReviewActionFailed(error) => {
            state.fail_load_latest_review_action(error);
        }
        Message::ClaimByProcessedTranscriptionSucceeded(source) => {
            let task = state.complete_claim_by_processed_transcription_success(source);
            return dispatch_pending_services(state, task);
        }
        Message::ClaimByProcessedTranscriptionFailed(error) => {
            state.fail_claim_by_processed_transcription(error);
        }
        Message::ClaimNextFromQueueSucceeded(source) => {
            let task = state.complete_claim_next_from_queue_success(source);
            return dispatch_pending_services(state, task);
        }
        Message::ClaimNextFromQueueFailed(error) => {
            state.complete_claim_next_from_queue_failure(error);
        }
        Message::BabelAudioPrepared {
            kind,
            route_metadata,
            result,
        } => {
            state.complete_babel_audio_prepared(kind, route_metadata, result);
        }
        Message::CreateRegionTrackChanged(lane_id) => state.change_create_region_track(lane_id),
        Message::CreateRegionStartChanged(value) => state.change_create_region_start(value),
        Message::CreateRegionEndChanged(value) => state.change_create_region_end(value),
        Message::CreateRegionTextChanged(value) => state.change_create_region_text(value),
        Message::CreateRegionPrimaryCategoryChanged(value) => {
            state.change_create_region_primary_category(value)
        }
        Message::CreateRegionSecondaryCategoryChanged(value) => {
            state.change_create_region_secondary_category(value)
        }
        Message::CreateRegionCustomDescriptionChanged(value) => {
            state.change_create_region_custom_description(value)
        }
        Message::CreateRegionIntensityChanged(value) => state.change_create_region_intensity(value),
        Message::CreateRegionSelectOpenChanged(open) => {
            state.change_create_region_select_open(open)
        }
        Message::CreateRegionInteractOutside(context) => {
            state.interact_outside_create_region(context)
        }
        Message::CommitCreateRegionStart => state.commit_create_region_start(),
        Message::CommitCreateRegionEnd => state.commit_create_region_end(),
        Message::CreateRegionInputEscapePressed => {
            if state.cancel_lane_vertical_zoom_input() {
            } else if state.find_bar_open {
                state.close_find_bar();
            } else {
                state.reset_active_create_region_time_field();
            }
        }
        Message::CancelCreateRegion => state.cancel_create_region(),
        Message::SubmitCreateRegion => state.submit_create_region(),
        Message::RequestEditAnnotation(segment_id) => state.request_edit_annotation(&segment_id),
        Message::EditAnnotationPrimaryCategoryChanged(value) => {
            state.change_edit_annotation_primary_category(value)
        }
        Message::EditAnnotationSecondaryCategoryChanged(value) => {
            state.change_edit_annotation_secondary_category(value)
        }
        Message::EditAnnotationCustomDescriptionChanged(value) => {
            state.change_edit_annotation_custom_description(value)
        }
        Message::EditAnnotationIntensityChanged(value) => {
            state.change_edit_annotation_intensity(value)
        }
        Message::EditAnnotationSelectOpenChanged(open) => {
            state.change_edit_annotation_select_open(open)
        }
        Message::EditAnnotationOutsideMouseDown(context) => {
            state.mouse_down_outside_edit_annotation(context)
        }
        Message::CancelEditAnnotation => state.cancel_edit_annotation(),
        Message::SubmitEditAnnotation => state.submit_edit_annotation(),
        Message::CopySupportId => {
            if let Some(editor) = &state.editor {
                let support_id = editor.support_id_button(false).clipboard_text;
                state.support_id_copied = true;
                state.status = format!("Support ID copied: {support_id}");
                return dispatch_pending_services(state, clipboard::write(support_id));
            }
        }
        Message::ResetSupportIdCopied => state.support_id_copied = false,
        Message::ToggleContextPanel(storage_key) => state.toggle_context_panel(&storage_key),
        Message::CopySubmissionId(submission_id) => {
            let result = resolve_submission_id_copy_success();
            state.copied_submission_id = result.copied;
            state.status = format!("Submission ID copied: {submission_id}");
            return dispatch_pending_services(state, clipboard::write(submission_id));
        }
        Message::ResetSubmissionIdCopied => state.copied_submission_id = false,
        Message::ShowHotkeys => state.show_hotkeys_dialog = true,
        Message::CloseHotkeys => {
            state.show_hotkeys_dialog = false;
            state.hotkey_capture = None;
        }
        Message::StartHotkeyCapture(action) => state.start_hotkey_capture(action),
        Message::CancelHotkeyCapture => state.cancel_hotkey_capture(),
        Message::ResetHotkeyBinding(action) => state.reset_hotkey_binding(action),
        Message::HotkeyBindingCaptured(action, binding) => {
            state.apply_hotkey_binding(action, binding)
        }
        Message::OpenFind => {
            state.open_find_bar();
            return dispatch_pending_services(
                state,
                Task::batch([
                    text_input::focus(find::find_input_id()),
                    scroll_to_find_match(state),
                ]),
            );
        }
        Message::CloseFind => state.close_find_bar(),
        Message::FindQueryChanged(query) => {
            state.update_find_query(query);
            return dispatch_pending_services(state, scroll_to_find_match(state));
        }
        Message::FindNext => {
            state.select_find_match(1);
            return dispatch_pending_services(state, scroll_to_find_match(state));
        }
        Message::FindPrevious => {
            state.select_find_match(-1);
            return dispatch_pending_services(state, scroll_to_find_match(state));
        }
        Message::EscapePressed => {
            if !state.cancel_timeline_autoscroll() && !state.dismiss_top_surface_or_loop() {
                let was_playing = state
                    .editor
                    .as_ref()
                    .is_some_and(|editor| editor.is_playing);
                state.handle_helper_unfocused_escape();
                let focus_task = if was_playing
                    && state
                        .editor
                        .as_ref()
                        .is_some_and(|editor| !editor.is_playing)
                {
                    focus_restored_transcript_editor(state)
                } else {
                    Task::none()
                };
                return dispatch_pending_services(state, focus_task);
            }
        }
        Message::ActivateDefaultSurfaceAction => {
            if let Some(message) = state.activate_default_surface_action() {
                return dispatch_pending_services(state, Task::done(message));
            }
        }
        Message::GhostCursorRowNavigation(direction) => {
            let scroll_target = state.move_ghost_cursor_row(direction);
            return dispatch_pending_services(state, scroll_to_transcript_offset(scroll_target));
        }
        Message::HelperTogglePlayback => {
            let was_playing = state
                .editor
                .as_ref()
                .is_some_and(|editor| editor.is_playing);
            state.handle_helper_toggle_playback();
            let focus_task = if was_playing
                && state
                    .editor
                    .as_ref()
                    .is_some_and(|editor| !editor.is_playing)
            {
                focus_restored_transcript_editor(state)
            } else {
                Task::none()
            };
            return dispatch_pending_services(state, focus_task);
        }
        Message::TogglePlayback => state.toggle_playback(),
        Message::TransportTick(now) => {
            state.advance_transport_tick(now);
            if let Some(target) = state.playback_follow_transcript_scroll_target() {
                return dispatch_pending_services(state, scroll_to_transcript_offset(Some(target)));
            }
        }
        Message::PreviousSegment => {
            let scroll_target = state.select_adjacent_segment(-1);
            return dispatch_pending_services(state, scroll_to_transcript_offset(scroll_target));
        }
        Message::NextSegment => {
            let scroll_target = state.select_adjacent_segment(1);
            return dispatch_pending_services(state, scroll_to_transcript_offset(scroll_target));
        }
        Message::RightShiftSegmentNavigation(direction) => {
            if state.right_shift_pressed {
                let scroll_target = state.select_adjacent_focused_segment(direction);
                let focus_task = if scroll_target.is_some() {
                    focus_restored_transcript_editor(state)
                } else {
                    Task::none()
                };
                return dispatch_pending_services(
                    state,
                    Task::batch([scroll_to_transcript_offset(scroll_target), focus_task]),
                );
            }
        }
        Message::TrimCurrentSegmentToAudio => state.trim_current_segment_to_audio(),
        Message::TrimAllVisibleSegmentsToAudio => state.trim_all_visible_segments_to_audio(),
        Message::AutoInsertSegmentAtCaret => {
            let scroll_target = state.auto_insert_segment_at_caret();
            return dispatch_pending_services(state, scroll_to_transcript_offset(scroll_target));
        }
        Message::AutoSegmentVisibleSilences => {
            let scroll_target = state.auto_segment_visible_silences();
            return dispatch_pending_services(state, scroll_to_transcript_offset(scroll_target));
        }
        Message::CommitCutPreviewOverlay { smart_split } => {
            let scroll_target = state.commit_cut_preview_overlay(smart_split);
            return dispatch_pending_services(state, scroll_to_transcript_offset(scroll_target));
        }
        Message::LoopCutPreviewOverlay => state.loop_cut_preview_overlay(),
        Message::SpeedChanged(value) => {
            if let Some(speed) = parse_speed(&value) {
                let previous_speed = state.editor.as_ref().map_or(speed, |editor| editor.speed);
                let had_editor = state.editor.is_some();
                state.with_editor_mut(|editor| editor.set_speed(speed));
                if toolbar_trace_enabled() {
                    let current_speed = state.editor.as_ref().map_or(speed, |editor| editor.speed);
                    eprintln!(
                        "{}",
                        toolbar_speed_runtime_trace_line(previous_speed, current_speed)
                    );
                }
                if had_editor {
                    state.persist_timeline_defaults();
                }
                state.sync_audio_playback_speed();
            }
        }
        Message::ZoomChanged(value) => {
            let mut previous_zoom = value;
            let mut current_zoom = value;
            let handled = state.with_project_and_editor_mut(|project, editor| {
                previous_zoom = editor.zoom;
                editor.set_zoom(project, value);
                current_zoom = editor.zoom;
            });
            if handled {
                state.persist_timeline_defaults();
            }
            if handled && toolbar_trace_enabled() {
                eprintln!(
                    "{}",
                    toolbar_zoom_runtime_trace_line(previous_zoom, current_zoom)
                );
            }
        }
        Message::GlobalPointerMoved(position) => state.move_global_pointer(position),
        Message::OverviewPointerMoved { fraction, width } => {
            state.hover_overview_with_geometry(fraction, width);
        }
        Message::OverviewExited => state.exit_overview(),
        Message::OverviewPressed => state.press_overview(),
        Message::OverviewReleased => state.release_overview(),
        Message::TimelineScrollbarHovered { fraction, width } => {
            state.hover_timeline_scrollbar_with_geometry(fraction, width);
        }
        Message::TimelineScrollbarExited => state.exit_timeline_scrollbar(),
        Message::TimelineScrollbarPressed => state.press_timeline_scrollbar(),
        Message::TimelineScrollbarReleased => state.release_timeline_scrollbar(),
        Message::TimelineAutoscrollPressed(lane_id) => {
            state.press_timeline_autoscroll(&lane_id);
        }
        Message::TimelineAutoscrollReleased => {
            state.release_timeline_autoscroll();
        }
        Message::TimelineWheelScrolled { delta_x, delta_y } => {
            if state.keyboard_modifiers.command() {
                let previous = timeline_wheel_trace_state(state);
                let anchor_time_seconds = state
                    .waveform_hit_hover
                    .as_ref()
                    .map(|hover| hover.hit.time_seconds);
                let handled = state.with_project_and_editor_mut(|project, editor| {
                    let zoom = zoom_factor_after_timeline_wheel_pixels(editor.zoom, delta_y);
                    if let Some(anchor_time_seconds) = anchor_time_seconds {
                        editor.set_zoom_around_time(project, zoom, anchor_time_seconds);
                    } else {
                        editor.set_zoom(project, zoom);
                    }
                });
                if handled {
                    state.persist_timeline_defaults();
                    trace_timeline_wheel_route(state, "command-zoom", delta_x, delta_y, previous);
                }
            } else if state.keyboard_modifiers.shift() {
                let previous = timeline_wheel_trace_state(state);
                let handled = state.with_project_and_editor_mut(|project, editor| {
                    editor.pan_viewport_by_wheel_pixels(project, delta_y);
                });
                if handled {
                    trace_timeline_wheel_route(state, "shift-pan", delta_x, delta_y, previous);
                }
            } else if delta_x != 0.0 {
                let previous = timeline_wheel_trace_state(state);
                let handled = state.with_project_and_editor_mut(|project, editor| {
                    editor.pan_viewport_by_wheel_pixels(project, delta_x);
                });
                if handled {
                    trace_timeline_wheel_route(state, "horizontal-pan", delta_x, delta_y, previous);
                }
            } else if delta_y != 0.0 {
                let previous = timeline_wheel_trace_state(state);
                trace_timeline_wheel_route(
                    state,
                    "plain-vertical-noop",
                    delta_x,
                    delta_y,
                    previous,
                );
            }
        }
        Message::SeekToTime(time_seconds) => {
            if let Some(project) = state.project().cloned() {
                state.with_editor_mut(|editor| editor.seek_to_time(&project, time_seconds));
                state.restart_audio_playback_if_playing();
            }
        }
        Message::ToggleDiffView => state.with_editor_mut(EditorSession::toggle_diff_view),
        Message::ToggleDiffReviewers => {
            state.with_workflow_mut("Diff reviewers", |workflow| {
                let shown = workflow.toggle_diff_reviewers();
                Ok(if shown {
                    "Reviewer feedback shown.".to_string()
                } else {
                    "Reviewer feedback hidden.".to_string()
                })
            });
        }
        Message::DiffCompareActionChanged(action_id) => {
            state.with_workflow_mut("Diff compare", |workflow| {
                workflow.set_diff_compare_action(&action_id);
                Ok(format!("Compare version `{action_id}` selected."))
            });
        }
        Message::NavigateProblemRows(direction) => {
            if let Some(project) = state.project().cloned() {
                let previous_segment_id = state
                    .editor
                    .as_ref()
                    .and_then(|editor| editor.selected_segment_id.clone());
                let mut selected_segment_id = None;
                let mut problem_index = 0;
                let mut problem_count = 0;
                state.with_editor_mut(|editor| {
                    editor.navigate_problem_rows(&project, direction);
                    selected_segment_id = editor.selected_segment_id.clone();
                    problem_index = editor.problem_row_index;
                    problem_count = editor.problem_row_count(&project);
                });
                if problem_navigation_trace_enabled() {
                    eprintln!(
                        "{}",
                        problem_navigation_runtime_trace_line(
                            direction,
                            previous_segment_id.as_deref(),
                            selected_segment_id.as_deref(),
                            problem_index,
                            problem_count,
                        )
                    );
                }
                let scroll_target = selected_segment_id.as_deref().and_then(|segment_id| {
                    state.scroll_transcript_to_segment_if_needed(&project, segment_id)
                });
                state.restart_audio_playback_if_playing();
                return dispatch_pending_services(
                    state,
                    scroll_to_transcript_offset(scroll_target),
                );
            }
        }
        Message::ToggleLoop => {
            if let Some(project) = state.project().cloned() {
                state.with_editor_mut(|editor| editor.toggle_loop(&project));
            }
        }
        Message::ToggleOverlay => {
            if let Some(project) = state.project().cloned() {
                state.with_editor_mut(|editor| editor.toggle_overlay(&project));
            }
        }
        Message::LoopStripHovered(hit) => state.hover_loop_strip(hit),
        Message::LoopStripPressed => state.press_loop_strip(),
        Message::LoopStripReleased => state.release_loop_strip(),
        Message::ToggleLaneMute(lane_id) => {
            state.with_editor_mut(|editor| editor.toggle_lane_mute(&lane_id));
            state.persist_timeline_defaults();
            state.restart_audio_playback_if_playing();
        }
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id,
            fraction,
            height,
        } => {
            state.hover_lane_vertical_zoom_slider_with_geometry(lane_id, fraction, height);
        }
        Message::LaneVerticalZoomSliderPressed(lane_id) => {
            if state.press_lane_vertical_zoom_slider(&lane_id) {
                return dispatch_pending_services(
                    state,
                    text_input::focus(lane_vertical_zoom_input_id()),
                );
            }
        }
        Message::LaneVerticalZoomSliderReleased => {
            state.release_lane_vertical_zoom_slider();
            state.persist_timeline_defaults();
        }
        Message::LaneVerticalZoomInputChanged(value) => {
            state.change_lane_vertical_zoom_input(value);
        }
        Message::SubmitLaneVerticalZoomInput => state.submit_lane_vertical_zoom_input(),
        Message::TrackFilterChanged(filter) => {
            let project = state.project().cloned();
            state.with_editor_mut(|editor| {
                editor.track_filter = filter;
                if let Some(project) = project.as_ref() {
                    editor.reconcile_selection_to_visible(project);
                }
            });
            state.persist_timeline_defaults();
        }
        Message::TranscriptScrolled {
            offset_y,
            viewport_height,
        } => {
            if transcript_scroll_trace_enabled() {
                eprintln!(
                    "Transcription Studio transcript scroll: offset_y={offset_y:.1} viewport_height={viewport_height:.1}"
                );
            }
            state.update_transcript_scroll_viewport(offset_y, viewport_height);
        }
        Message::TranscriptTextWheelScrolled { delta_y } => {
            return dispatch_pending_services(
                state,
                scrollable::scroll_by(
                    find::transcript_scroll_id(),
                    scrollable::AbsoluteOffset { x: 0.0, y: delta_y },
                ),
            );
        }
        Message::TranscriptRowBodyPressed(segment_id) => {
            let outcome = state.press_transcript_row_body(&segment_id);
            if outcome.sought {
                state.restart_audio_playback_if_playing();
            }
            return dispatch_pending_services(
                state,
                scroll_to_transcript_offset(outcome.scroll_target),
            );
        }
        Message::TranscriptRowHovered(segment_id) => {
            state.transcript_row_hover_segment_id = Some(segment_id);
        }
        Message::TranscriptRowExited(segment_id) => {
            if state.transcript_row_hover_segment_id.as_deref() == Some(segment_id.as_str()) {
                state.transcript_row_hover_segment_id = None;
            }
        }
        Message::WaveformHitHovered(lane_id, hit) => state.hover_waveform_hit(lane_id, hit),
        Message::WaveformHitExited(lane_id) => {
            if state
                .waveform_hit_hover
                .as_ref()
                .is_some_and(|hover| hover.lane_id == lane_id)
            {
                state.waveform_hit_hover = None;
            }
        }
        Message::WaveformHitPressed(lane_id) => state.press_waveform_hit(&lane_id),
        Message::WaveformHitReleased(lane_id) => state.release_waveform_hit(&lane_id),
        Message::ToggleLaneCollapse(lane_id) => {
            state.with_editor_mut(|editor| editor.toggle_lane_collapse(&lane_id));
            state.persist_timeline_defaults();
        }
        Message::ToggleActions(segment_id) => {
            state.with_editor_mut(|editor| {
                let was_open =
                    editor.open_actions_segment_id.as_deref() == Some(segment_id.as_str());
                if was_open {
                    editor.open_actions_segment_id = None;
                } else {
                    editor.open_actions_segment_id = Some(segment_id.clone());
                }
                if transcript_action_trace_enabled() {
                    eprintln!(
                        "{}",
                        transcript_action_toggle_runtime_trace_line(
                            &segment_id,
                            was_open,
                            !was_open
                        )
                    );
                }
            });
        }
        Message::Undo => {
            if !state.block_read_only_edit("Undo") {
                state.undo();
            }
        }
        Message::Redo => {
            if !state.block_read_only_edit("Redo") {
                state.redo();
            }
        }
        Message::SegmentTimeChanged(segment_id, field, value) => {
            if !state.block_read_only_edit("Time edit") {
                state.update_segment_time_draft(&segment_id, field, value);
            }
        }
        Message::CommitSegmentTimeEdit {
            segment_id,
            field,
            from_blur,
        } => {
            if !state.block_read_only_edit("Time edit") {
                state.commit_segment_time_edit(&segment_id, field, from_blur);
            }
        }
        Message::SegmentTrackCellPressed(segment_id) => {
            if !state.block_read_only_edit("Track edit") {
                state.press_segment_track_cell(&segment_id);
            }
        }
        Message::SegmentTrackSelectOpenChanged(segment_id, is_open) => {
            if !state.block_read_only_edit("Track edit") {
                state.set_segment_track_select_open(&segment_id, is_open);
            }
        }
        Message::SegmentTrackChanged(segment_id, lane_id) => {
            if !state.block_read_only_edit("Track edit") {
                state.select_segment_track(&segment_id, &lane_id);
            }
        }
        Message::SegmentTextEdited(segment_id, action) => {
            let mut layout_height_changed = false;
            let mut text_changed = false;
            let mut clear_tag_suggestions = false;
            if !state.block_read_only_edit("Text edit")
                && let Some(draft) = state.segment_drafts.get_mut(&segment_id)
            {
                let before = draft.text.clone();
                let before_height = draft.text_editor_content_height();
                draft.perform_text_edit(action);
                text_changed = draft.text != before;
                layout_height_changed =
                    draft.text_editor_content_height().to_bits() != before_height.to_bits();
                clear_tag_suggestions = true;
            }
            if text_changed {
                state.mark_dirty();
            }
            if clear_tag_suggestions {
                state.clear_tag_suggestion_state_for_segment(&segment_id);
            }
            if layout_height_changed {
                state.invalidate_transcript_layout_cache();
            }
        }
        Message::TranscriptEditorEscapePressed(segment_id) => {
            let previous_pending = state.helper_focus_restore_pending.clone();
            state.handle_helper_focused_escape(&segment_id);
            if state.helper_focused_segment_id.is_none()
                && state.helper_focus_restore_pending != previous_pending
            {
                return dispatch_pending_services(
                    state,
                    focusable_text_editor::blur_transcript_text_editors(),
                );
            }
        }
        Message::AdjustPlaybackSpeed(direction) => state.adjust_playback_speed(direction),
        Message::RewindPlaybackOneSecond => state.rewind_playback_one_second(),
        Message::SwitchSpeakerWorkflowLane(label) => state.switch_speaker_workflow_lane(&label),
        Message::ResetSpeakerWorkflow => state.reset_speaker_workflow(),
        Message::SegmentTextUndo(segment_id) => {
            let mut layout_height_changed = false;
            let mut text_changed = false;
            let mut clear_tag_suggestions = false;
            if !state.block_read_only_edit("Text edit")
                && !state.undo_cross_row_text_move(&segment_id)
                && let Some(draft) = state.segment_drafts.get_mut(&segment_id)
            {
                let before_height = draft.text_editor_content_height();
                if draft.undo_text_edit() {
                    text_changed = true;
                    layout_height_changed =
                        draft.text_editor_content_height().to_bits() != before_height.to_bits();
                }
                clear_tag_suggestions = true;
            }
            if text_changed {
                state.mark_dirty();
            }
            if clear_tag_suggestions {
                state.clear_tag_suggestion_state_for_segment(&segment_id);
            }
            if layout_height_changed {
                state.invalidate_transcript_layout_cache();
            }
        }
        Message::SegmentTextRedo(segment_id) => {
            let mut layout_height_changed = false;
            let mut text_changed = false;
            let mut clear_tag_suggestions = false;
            if !state.block_read_only_edit("Text edit")
                && !state.redo_cross_row_text_move(&segment_id)
                && let Some(draft) = state.segment_drafts.get_mut(&segment_id)
            {
                let before_height = draft.text_editor_content_height();
                if draft.redo_text_edit() {
                    text_changed = true;
                    layout_height_changed =
                        draft.text_editor_content_height().to_bits() != before_height.to_bits();
                }
                clear_tag_suggestions = true;
            }
            if text_changed {
                state.mark_dirty();
            }
            if clear_tag_suggestions {
                state.clear_tag_suggestion_state_for_segment(&segment_id);
            }
            if layout_height_changed {
                state.invalidate_transcript_layout_cache();
            }
        }
        Message::MoveTextBeforeCaretToPreviousSegment(segment_id) => {
            if !state.block_read_only_edit("Text move") {
                state.move_text_before_caret_to_previous_segment(&segment_id);
            }
        }
        Message::MoveTextAfterCaretToNextSegment(segment_id) => {
            if !state.block_read_only_edit("Text move") {
                state.move_text_after_caret_to_next_segment(&segment_id);
            }
        }
        Message::ConvertSelectedNumberToSkaz => {
            if !state.block_read_only_edit("SKAZ conversion") {
                state.convert_active_selection_to_skaz();
            }
        }
        Message::ConvertSelectionToSkazWithDigit(segment_id, digit) => {
            if !state.block_read_only_edit("SKAZ conversion") {
                state.convert_segment_selection_with_digit(&segment_id, digit);
            }
        }
        Message::AutofixCurrentLintRow => {
            if !state.block_read_only_edit("Lint autofix") {
                state.autofix_current_lint_row();
            }
        }
        Message::AutofixAllLintRows => {
            if !state.block_read_only_edit("Lint autofix") {
                state.autofix_all_lint_rows();
            }
        }
        Message::ToggleNativeCapabilityPanel => state.toggle_native_capability_panel(),
        Message::NativeCapabilityModalTabChanged(tab) => state.set_native_capability_modal_tab(tab),
        Message::ReviewerGenerateReview => state.queue_reviewer_generate(),
        Message::ReviewerCreateSession => state.queue_reviewer_create_session(),
        Message::ReviewerSessionIdChanged(value) => state.update_reviewer_session_id(value),
        Message::ReviewerLoadSession => state.queue_reviewer_load_session(),
        Message::ReviewerSessionCommentChanged(value) => {
            state.update_reviewer_session_comment(value)
        }
        Message::ReviewerCardIdChanged(value) => state.select_reviewer_card(value),
        Message::ReviewerCardCommentChanged(value) => state.update_reviewer_card_comment(value),
        Message::ReviewerApplySelectedCardFeedback => state.apply_selected_reviewer_card_feedback(),
        Message::ReviewerSaveComments => state.queue_reviewer_save_comments(),
        Message::ReviewerCommentSaveTick => state.auto_save_reviewer_comments(),
        Message::ReviewerTemplateQueryChanged(value) => state.reviewer_template_query = value,
        Message::ReviewerSearchTemplates => state.queue_reviewer_template_search(),
        Message::ReviewerTemplateIdChanged(value) => state.reviewer_template_id = value,
        Message::ReviewerSetTemplateMatch => state.queue_reviewer_set_template_match(),
        Message::ReviewerClearTemplateMatch => state.queue_reviewer_clear_template_match(),
        Message::ReviewerGenerateTemplateSuggestions => {
            state.queue_reviewer_template_suggestions();
        }
        Message::ReviewerExpandAllCards => state.set_reviewer_cards_expanded(true),
        Message::ReviewerCollapseAllCards => state.set_reviewer_cards_expanded(false),
        Message::ReviewerToggleSuggestionsExpanded => {
            state.toggle_reviewer_suggestions_expanded();
        }
        Message::ReviewerProposalIdChanged(value) => state.reviewer_proposal_id = value,
        Message::ReviewerApproveTemplateSuggestion => {
            state.queue_reviewer_decide_template_suggestion("approved");
        }
        Message::ReviewerRejectTemplateSuggestion => {
            state.queue_reviewer_decide_template_suggestion("rejected");
        }
        Message::ReviewerFinalizeApply => state.queue_reviewer_finalize("apply"),
        Message::ReviewerFinalizeSkip => state.queue_reviewer_finalize("skip"),
        Message::ReviewerSubmitAnalytics => state.queue_reviewer_submit_analytics(),
        Message::ReviewerHistoryQueryChanged(value) => state.reviewer_history_query = value,
        Message::ReviewerLoadHistory => state.queue_reviewer_history(),
        Message::DraftOpenRouterApiKeyChanged(value) => state.draft_openrouter_api_key = value,
        Message::DraftModelChanged(value) => state.draft_model = value,
        Message::DraftServiceTierChanged(value) => state.draft_service_tier = value,
        Message::DraftReasoningEffortChanged(value) => state.draft_reasoning_effort = value,
        Message::DraftSessionIdChanged(value) => state.draft_session_id = value,
        Message::DraftCheckBackendStatus => state.queue_draft_backend_health(),
        Message::DraftGenerateRows => state.queue_draft_generate_rows(),
        Message::DraftRetryAudioCapture => state.retry_draft_audio_capture(),
        Message::DraftUseCapturedAudio => state.queue_draft_generate_rows_with_available_audio(),
        Message::DraftContinueTextOnly => state.queue_draft_generate_rows_text_only(),
        Message::DraftTranscribeSelectedSegment => state.queue_draft_transcribe_selected_segment(),
        Message::DraftRedistributeText => state.queue_draft_redistribute_text(),
        Message::DraftApplyLastRows => state.apply_last_draft_rows(),
        Message::DraftApplySegmentText => state.apply_last_transcribed_segment_text(),
        Message::DraftApplyRow(row_id) => state.apply_draft_row(&row_id),
        Message::DraftSkipRow(row_id) => state.skip_draft_row(&row_id),
        Message::DraftApplyRedistributionGroup(group_id) => {
            state.apply_redistribution_group(&group_id);
        }
        Message::ReviewBackendRequestSucceeded(request, payload) => {
            state
                .review_backend_requests
                .retain(|queued| queued != &request);
            state
                .dispatched_review_backend_requests
                .retain(|queued| queued != &request);
            state.native_capability_last_response = Some(payload);
            state.native_capability_last_error = None;
            let payload = state
                .native_capability_last_response
                .clone()
                .unwrap_or_default();
            state.handle_native_capability_success(&request, &payload);
            if state.status.is_empty() {
                state.status = "Review backend request completed.".to_string();
            }
        }
        Message::ReviewBackendRequestFailed(request, error) => {
            state
                .review_backend_requests
                .retain(|queued| queued != &request);
            state
                .dispatched_review_backend_requests
                .retain(|queued| queued != &request);
            if state.reconcile_draft_stream_failure(&request, &error) {
                state.native_capability_last_error = Some(error);
                return Task::none();
            }
            state.native_capability_last_error = Some(error.clone());
            state.status = format!("Review backend request failed: {error}");
        }
        Message::ApplyTagSuggestion(segment_id, suggestion) => {
            let mut layout_height_changed = false;
            let mut text_changed = false;
            let mut clear_tag_suggestions = false;
            if !state.block_read_only_edit("Text edit")
                && let Some(draft) = state.segment_drafts.get_mut(&segment_id)
            {
                let before = draft.text.clone();
                let before_height = draft.text_editor_content_height();
                tag_suggestions::apply_tag_suggestion(draft, &suggestion);
                text_changed = draft.text != before;
                layout_height_changed =
                    draft.text_editor_content_height().to_bits() != before_height.to_bits();
                clear_tag_suggestions = true;
            }
            if text_changed {
                state.mark_dirty();
            }
            if clear_tag_suggestions {
                state.clear_tag_suggestion_state_for_segment(&segment_id);
            }
            if layout_height_changed {
                state.invalidate_transcript_layout_cache();
            }
        }
        Message::TranscriptTagSuggestionKeyPressed(segment_id, action) => {
            if !state.block_read_only_edit("Text edit") {
                state.handle_tag_suggestion_keyboard_action(&segment_id, action);
            }
        }
        Message::AcknowledgeLowConfidenceWord(segment_id, word_index) => {
            if !state.block_read_only_edit("Low-confidence review") {
                state.acknowledge_low_confidence_word(&segment_id, word_index);
            }
        }
        Message::ToggleActiveWarningAssertion => {
            if !state.block_read_only_edit("Warning assertion")
                && let Some(project) = state.project().cloned()
            {
                let mut toggled = false;
                state.with_editor_mut(|editor| {
                    toggled = editor.toggle_warning_assertion(&project);
                });
                if toggled {
                    state.invalidate_lint_view_caches();
                    if state.submit_is_pending {
                        state.cancel_pending_submit_for_edit();
                    } else {
                        state.status = "Warning assertion updated.".to_string();
                    }
                }
            }
        }
        Message::ToggleWarningAssertion(segment_id) => {
            if !state.block_read_only_edit("Warning assertion")
                && let Some(project) = state.project().cloned()
            {
                let mut toggled = false;
                state.with_editor_mut(|editor| {
                    editor.select_segment(&project, &segment_id);
                    toggled = editor.toggle_warning_assertion(&project);
                });
                if toggled {
                    state.invalidate_lint_view_caches();
                    if state.submit_is_pending {
                        state.cancel_pending_submit_for_edit();
                    } else {
                        state.status = "Warning assertion updated.".to_string();
                    }
                }
            }
        }
        Message::DeleteSelectedSegment => {
            if let Some(segment_id) = state
                .editor
                .as_ref()
                .and_then(|editor| editor.selected_segment_id.clone())
            {
                state.delete_segment(&segment_id);
            } else {
                state.status = "Select a transcript segment before deleting.".to_string();
            }
        }
        Message::MergeSelectedSegmentAbove => {
            if let Some(segment_id) = state
                .editor
                .as_ref()
                .and_then(|editor| editor.selected_segment_id.clone())
            {
                state.merge_segment_above(&segment_id);
            } else {
                state.status = "Select a transcript segment before merging.".to_string();
            }
        }
        Message::MergeSelectedSegmentBelow => {
            if let Some(segment_id) = state
                .editor
                .as_ref()
                .and_then(|editor| editor.selected_segment_id.clone())
            {
                state.merge_segment_below(&segment_id);
            } else {
                state.status = "Select a transcript segment before merging.".to_string();
            }
        }
        Message::MergeSegmentAbove(segment_id) => state.merge_segment_above(&segment_id),
        Message::MergeSegmentBelow(segment_id) => state.merge_segment_below(&segment_id),
        Message::DeleteSegment(segment_id) => state.delete_segment(&segment_id),
        Message::CreateSegmentBefore(segment_id) => state.create_segment_before(&segment_id),
        Message::CreateSegmentAfter(segment_id) => state.create_segment_after(&segment_id),
    }

    dispatch_pending_services(state, Task::none())
}

fn dispatch_pending_services(state: &mut StudioState, task: Task<Message>) -> Task<Message> {
    if state.native_auth_flow_blocks_service_dispatch() {
        return task;
    }
    Task::batch([
        task,
        state.dispatch_new_babel_service_requests(),
        state.dispatch_new_review_backend_requests(),
        state.dispatch_new_subaccount_owner_requests(),
    ])
}

fn execute_leave_intent(state: &mut StudioState, intent: PendingLeaveIntent) -> Task<Message> {
    match intent {
        PendingLeaveIntent::ExitApplication => iced::exit(),
        PendingLeaveIntent::BackToProjects => {
            state.no_work_modal = None;
            state.enter_dashboard();
            state.status = "Back to projects requested.".to_string();
            Task::none()
        }
    }
}

async fn start_native_auth_login(identifier: String) -> Result<(String, Option<String>), String> {
    let store_path = default_native_auth_store_path();
    let babel_url = native_babel_url();
    let clerk_url = native_clerk_url();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    let result =
        start_phone_code_login(&store_path, &identifier, &babel_url, &clerk_url, true, now).await?;
    Ok((
        result.sign_in_id,
        Some(result.phone_number_id).filter(|value| !value.trim().is_empty()),
    ))
}

async fn verify_native_auth_code(code: String) -> Result<(), String> {
    let store_path = default_native_auth_store_path();
    let babel_url = native_babel_url();
    let clerk_url = native_clerk_url();
    let now = OffsetDateTime::now_utc().unix_timestamp();
    verify_phone_code_login(&store_path, &code, &babel_url, &clerk_url, now)
        .await
        .map(|_| ())
}

fn reset_native_auth_store() -> Result<(), String> {
    let store_path = default_native_auth_store_path();
    save_native_auth_store(&store_path, &NativeAuthStore::default())
}

fn forget_audition_key_release_tokens(state: &mut StudioState, released_key: super::AuditionKey) {
    state.audition_key_release_tokens.retain(|_, keys| {
        keys.remove(&released_key);
        !keys.is_empty()
    });
}

fn scroll_to_find_match(state: &StudioState) -> Task<Message> {
    let (Some(project), Some(editor)) = (state.project(), state.editor.as_ref()) else {
        return Task::none();
    };
    let Some(y) = state.find_scroll_offset_for_selected_match(project, editor) else {
        return Task::none();
    };

    scrollable::scroll_to(
        find::transcript_scroll_id(),
        scrollable::AbsoluteOffset { x: 0.0, y },
    )
}

fn scroll_to_transcript_offset(scroll_target: Option<f32>) -> Task<Message> {
    let Some(y) = scroll_target else {
        return Task::none();
    };

    scrollable::scroll_to(
        find::transcript_scroll_id(),
        scrollable::AbsoluteOffset { x: 0.0, y },
    )
}

pub(super) fn restored_transcript_editor_id(state: &StudioState) -> Option<&str> {
    state
        .helper_last_focus_restore
        .as_ref()
        .map(|restore| restore.segment_id.as_str())
}

fn focus_restored_transcript_editor(state: &StudioState) -> Task<Message> {
    let Some(segment_id) = restored_transcript_editor_id(state) else {
        return Task::none();
    };
    focusable_text_editor::focus_transcript_text_editor(segment_id)
}
