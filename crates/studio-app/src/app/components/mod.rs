mod admin_flag_dialog;

pub(super) use admin_flag_dialog::admin_flag_reason_input_id;
mod context_panels;
mod create_region_dialog;
mod dashboard;
mod drag_capture;
mod edit_annotation_dialog;
mod exit_confirm_dialog;
mod feedback_panel;
pub(super) mod focusable_text_editor;
pub(super) mod hotkeys_dialog;
mod native_auth_screen;
mod native_capability_panel;
mod no_work_dialog;
mod playhead_line;
mod skip_chunk_dialog;
mod submit_dialog;
mod task_lookup_dialog;
mod timed_widget;
mod timeline;
pub(super) use timeline::lane_vertical_zoom_input_id;
mod toolbar;
mod transcript_table;
mod transport_strip;
mod wavesurfer_canvas;

use iced::widget::{column, container, horizontal_rule, mouse_area, opaque, stack};
use iced::{Element, Fill};
use studio_app::EditorSession;
use studio_core::Project;

use super::browser_input::OutsideInteractionContext;
use super::styles::{app_background_style, babel_editor_card_style, modal_scrim_style};
use super::{FeedbackPanelState, Message, StudioState};
use timed_widget::view_timed;

const WORKBENCH_PADDING: u16 = 24;

pub(crate) fn view(state: &StudioState) -> Element<'_, Message> {
    let base = view_surface(state);
    if state.show_exit_confirm_dialog {
        return stack![
            base,
            view_modal_layer(
                exit_confirm_dialog::view_exit_confirm_dialog(),
                Some(Message::CancelExit),
            ),
        ]
        .into();
    }
    base
}

fn view_surface(state: &StudioState) -> Element<'_, Message> {
    if let Some(flow) = state.native_auth_flow.as_ref() {
        return native_auth_screen::view_native_auth_screen(flow);
    }

    if state.show_dashboard() {
        return dashboard::view_dashboard(state);
    }

    let content = match (&state.workflow, &state.editor) {
        (Ok(workflow), Some(editor)) => view_editor(state, workflow.project(), editor),
        (Err(error), _) => dashboard::view_dashboard_error(state, error),
        _ => dashboard::view_dashboard_error(state, "Editor session is not available."),
    };

    container(content)
        .padding(0)
        .width(Fill)
        .height(Fill)
        .style(app_background_style)
        .into()
}

fn view_editor<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let waveform_card = view_outside_dismiss_area(
        state,
        view_timed("waveform-card", view_waveform_card(state, project, editor)),
    );
    let mut layout = column![].spacing(8).width(Fill).height(Fill);

    if let Some(context_panels) = context_panels::view_context_panels(state, project) {
        let context_panels = view_outside_dismiss_area(state, context_panels);
        layout = layout.push(context_panels);
    }

    layout = layout.push(waveform_card);

    layout = layout.push(view_outside_dismiss_area(
        state,
        view_timed(
            "transcript-card",
            view_transcript_card(state, project, editor),
        ),
    ));

    let mut content: Element<'a, Message> = layout.into();
    if let Some(dialog) = state
        .create_region_dialog
        .as_ref()
        .filter(|dialog| !create_region_dialog::is_attached_transcription_card(dialog))
    {
        content = stack![
            content,
            create_region_dialog::view_create_region_dialog(project, dialog),
        ]
        .into();
    }
    if state.native_capability_panel_open {
        content = stack![content, view_native_capability_modal_root(state)].into();
    }
    if let Some(panel) = state.feedback_panel.as_ref().filter(|panel| panel.open) {
        content = stack![content, view_feedback_overlay(panel)].into();
    }

    if state.show_submit_dialog {
        let submit = state.submit_control_snapshot_for_render(project, editor);
        let scrim_dismiss = (!submit.cancel_disabled).then_some(Message::CancelSubmitReview);
        content = stack![
            content,
            view_modal_layer(submit_dialog::view_submit_dialog(submit), scrim_dismiss),
        ]
        .into();
    }

    if state.show_skip_chunk_dialog && EditorSession::skip_chunk_control_available(project) {
        let skip_chunk = EditorSession::skip_chunk_control(
            project,
            state.show_skip_chunk_dialog,
            state.skip_is_pending,
        );
        let scrim_dismiss = (!skip_chunk.cancel_disabled).then_some(Message::CancelSkipChunk);
        content = stack![
            content,
            view_modal_layer(
                skip_chunk_dialog::view_skip_chunk_dialog(skip_chunk),
                scrim_dismiss,
            ),
        ]
        .into();
    }

    if state.show_admin_flag_dialog && EditorSession::admin_flag_control_available(project) {
        let admin_flag = EditorSession::admin_flag_control(
            state.show_admin_flag_dialog,
            state.admin_flag_reason.clone(),
            state.admin_flag_is_pending,
        );
        let scrim_dismiss = (!admin_flag.cancel_disabled).then_some(Message::CancelAdminFlag);
        content = stack![
            content,
            view_modal_layer(
                admin_flag_dialog::view_admin_flag_dialog(admin_flag),
                scrim_dismiss,
            ),
        ]
        .into();
    }

    if state.show_hotkeys_dialog {
        content = stack![
            content,
            view_modal_layer(
                hotkeys_dialog::view_hotkeys_dialog(state),
                Some(Message::CloseHotkeys),
            ),
        ]
        .into();
    }

    if let Some(task_lookup) = state.task_lookup.as_ref() {
        let scrim_dismiss = if task_lookup.confirming_edit {
            Message::TaskLookupCancelConfirmEdit
        } else {
            Message::CloseTaskLookup
        };
        content = stack![
            content,
            view_modal_layer(
                task_lookup_dialog::view_task_lookup_dialog(task_lookup),
                Some(scrim_dismiss),
            ),
        ]
        .into();
    }

    if let Some(dialog) = state.edit_annotation_dialog.as_ref() {
        content = stack![
            content,
            view_modal_layer(
                edit_annotation_dialog::view_edit_annotation_dialog(dialog),
                Some(Message::EditAnnotationOutsideMouseDown(
                    OutsideInteractionContext::plain_outside(),
                )),
            ),
        ]
        .into();
    }

    if let Some(no_work_modal) = state.no_work_modal.as_ref().filter(|modal| modal.open) {
        content = stack![
            content,
            view_modal_layer(no_work_dialog::view_no_work_dialog(no_work_modal), None),
        ]
        .into();
    }

    container(content)
        .padding(WORKBENCH_PADDING)
        .width(Fill)
        .height(Fill)
        .style(app_background_style)
        .into()
}

fn view_modal_layer<'a>(
    dialog: Element<'a, Message>,
    scrim_dismiss: Option<Message>,
) -> Element<'a, Message> {
    let scrim = mouse_area(
        container(opaque(dialog))
            .width(Fill)
            .height(Fill)
            .align_x(iced::Alignment::Center)
            .align_y(iced::Alignment::Center)
            .padding(24)
            .style(modal_scrim_style),
    );
    let scrim = match scrim_dismiss {
        Some(message) => scrim.on_press(message),
        None => scrim,
    };
    opaque(scrim)
}

fn view_native_capability_modal_root<'a>(state: &'a StudioState) -> Element<'a, Message> {
    opaque(
        container(opaque(
            native_capability_panel::view_native_capability_panel(state),
        ))
        .width(Fill)
        .height(Fill)
        .align_x(iced::Alignment::Center)
        .align_y(iced::Alignment::Center)
        .padding(24)
        .style(modal_scrim_style),
    )
}

fn view_feedback_overlay<'a>(panel: &'a FeedbackPanelState) -> Element<'a, Message> {
    container(feedback_panel::view_feedback_panel(panel))
        .width(Fill)
        .height(Fill)
        .align_x(iced::Alignment::End)
        .into()
}

fn view_outside_dismiss_area<'a>(
    state: &StudioState,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    if state.edit_annotation_dialog.is_some() {
        mouse_area(content)
            .on_press(Message::EditAnnotationOutsideMouseDown(
                OutsideInteractionContext::plain_outside(),
            ))
            .into()
    } else if state.create_region_dialog.is_some() {
        mouse_area(content)
            .on_press(Message::CreateRegionInteractOutside(
                OutsideInteractionContext::plain_outside(),
            ))
            .into()
    } else {
        content
    }
}

fn view_waveform_card<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    container(
        column![
            view_timed(
                "top-toolbar",
                toolbar::view_top_toolbar(state, project, editor),
            ),
            horizontal_rule(1),
            view_timed("diff-toolbar", toolbar::view_diff_toolbar(project, editor)),
            horizontal_rule(1),
            view_timed("timeline", timeline::view_timeline(state, project, editor),),
        ]
        .spacing(0),
    )
    .width(Fill)
    .style(babel_editor_card_style)
    .into()
}

fn view_transcript_card<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    transcript_table::view_transcript_workspace(state, project, editor)
}
