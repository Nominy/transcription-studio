use iced::widget::{
    Space, button, column, container, horizontal_rule, row, scrollable, text, text_input,
};
use iced::{Color, Element, Fill, Length};

use super::super::styles::{panel_style, subtle_panel_style};
use super::super::{DEFAULT_EXPORT_PATH, DEFAULT_IMPORT_PATH, Message, StudioState};

pub(super) fn view_dashboard(state: &StudioState) -> Element<'_, Message> {
    let mut content = column![
        row![
            column![
                text("Transcription Studio").size(24),
                text("Babel projects").size(13),
            ]
            .spacing(4),
            Space::with_width(Fill),
            button(text("Open demo")).on_press(Message::LoadBundledDemo),
            button(text("Refresh")).on_press_maybe(
                (!state.dashboard.is_pending).then_some(Message::DashboardRefreshProjects)
            ),
        ]
        .spacing(12)
        .align_y(iced::Alignment::Center),
        text(state.status.clone()).size(12),
    ]
    .spacing(12);

    if let Some(error) = state.dashboard.error.as_ref() {
        content = content.push(
            text(error.clone())
                .size(12)
                .color(Color::from_rgb8(185, 28, 28)),
        );
    }

    let mut project_list = column![].spacing(8);
    if state.dashboard.projects.is_empty() {
        project_list = project_list.push(text("No active Babel projects loaded.").size(13));
    } else {
        for project in &state.dashboard.projects {
            project_list = project_list.push(
                container(
                    row![
                        column![
                            text(project.name.clone()).size(15),
                            text(dashboard_project_status(project)).size(12),
                        ]
                        .spacing(3),
                        Space::with_width(Fill),
                        button(text("Start task")).on_press_maybe(
                            (!state.dashboard.is_pending
                                && !project.chunked_queue_id.trim().is_empty())
                            .then_some(Message::DashboardStartProject(project.id.clone()))
                        ),
                    ]
                    .spacing(12)
                    .align_y(iced::Alignment::Center),
                )
                .padding(10)
                .width(Fill)
                .style(subtle_panel_style),
            );
        }
    }

    content = content.push(
        container(scrollable(project_list).height(Length::FillPortion(2)))
            .padding(10)
            .width(Fill)
            .style(subtle_panel_style),
    );

    content = content
        .push(view_capability_settings(state))
        .push(view_update_section(state))
        .push(view_path_toolbar(state));

    container(content)
        .padding(18)
        .width(Fill)
        .height(Fill)
        .style(panel_style)
        .into()
}

pub(super) fn view_dashboard_error<'a>(
    state: &'a StudioState,
    error: &'a str,
) -> Element<'a, Message> {
    container(
        column![
            text("Transcription Studio").size(24),
            text("No project is currently loaded."),
            text(error).size(13),
            view_path_toolbar(state),
            button(text("Reload Demo")).on_press(Message::LoadBundledDemo),
        ]
        .spacing(12),
    )
    .padding(18)
    .style(panel_style)
    .into()
}

fn dashboard_project_status(project: &super::super::state::DashboardProject) -> &'static str {
    if project.chunked_queue_id.trim().is_empty() {
        "No claimable queue"
    } else {
        "Ready to claim next task"
    }
}

fn view_capability_settings(state: &StudioState) -> Element<'_, Message> {
    let provider_row = row![
        text_input("OpenRouter key", &state.draft_openrouter_api_key)
            .on_input(Message::DraftOpenRouterApiKeyChanged)
            .padding(6)
            .width(Fill),
        button(text("Check")).on_press(Message::DraftCheckBackendStatus),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let model_row = row![
        text_input("Model", &state.draft_model)
            .on_input(Message::DraftModelChanged)
            .padding(6)
            .width(Length::FillPortion(2)),
        text_input("Tier", &state.draft_service_tier)
            .on_input(Message::DraftServiceTierChanged)
            .padding(6)
            .width(Length::FillPortion(1)),
        text_input("Reasoning", &state.draft_reasoning_effort)
            .on_input(Message::DraftReasoningEffortChanged)
            .padding(6)
            .width(Length::FillPortion(1)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    container(
        column![
            column![
                text("Capability settings").size(15),
                text(
                    "Reviewer appears on L2 review tasks; drafting uses the backend settings below."
                )
                .size(12),
            ]
            .spacing(3),
            horizontal_rule(1),
            provider_row,
            model_row,
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}

fn view_update_section(state: &StudioState) -> Element<'_, Message> {
    let status_color = if state.updater.is_error() {
        Color::from_rgb8(185, 28, 28)
    } else {
        Color::from_rgb8(75, 85, 99)
    };
    let check_button = button(text("Check for updates")).on_press_maybe(
        state
            .updater
            .can_check()
            .then_some(Message::CheckForUpdates),
    );
    let install_button = button(text("Install update")).on_press_maybe(
        state
            .updater
            .can_install()
            .then_some(Message::InstallAvailableUpdate),
    );

    container(
        column![
            row![
                column![text("Updates").size(15), text("Alpha channel").size(12),].spacing(3),
                Space::with_width(Fill),
                check_button,
                install_button,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            horizontal_rule(1),
            row![
                text("Current version").width(Length::Fixed(104.0)),
                text(state.updater.current_version.clone()).size(12),
            ]
            .spacing(8),
            row![
                text("Update feed").width(Length::Fixed(104.0)),
                text(state.updater.feed_url.clone()).size(12),
            ]
            .spacing(8),
            text(state.updater.status_label())
                .size(12)
                .color(status_color),
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}

fn view_path_toolbar(state: &StudioState) -> Element<'_, Message> {
    let import_row = row![
        text("Babel JSON").width(Length::Fixed(82.0)),
        text_input(DEFAULT_IMPORT_PATH, &state.import_path)
            .on_input(Message::ImportPathChanged)
            .on_submit(Message::ImportBabelJson)
            .padding(5)
            .width(Fill),
        button(text("Import")).on_press(Message::ImportBabelJson),
    ]
    .spacing(8);

    let export_row = row![
        text("Export").width(Length::Fixed(82.0)),
        text_input(DEFAULT_EXPORT_PATH, &state.export_path)
            .on_input(Message::ExportPathChanged)
            .on_submit(Message::ExportBabelJson)
            .padding(5)
            .width(Fill),
        button(text("Export")).on_press(Message::ExportBabelJson),
    ]
    .spacing(8);

    container(column![import_row, export_row].spacing(6))
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}
