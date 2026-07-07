use std::collections::BTreeSet;

use iced::widget::{
    Space, button, checkbox, column, container, horizontal_rule, row, scrollable, text, text_input,
};
use iced::{Color, Element, Fill, Length};

use super::super::state::DashboardSection;
use super::super::styles::{panel_style, subtle_panel_style};
use super::super::{DEFAULT_EXPORT_PATH, DEFAULT_IMPORT_PATH, Message, StudioState};

pub(super) fn view_dashboard(state: &StudioState) -> Element<'_, Message> {
    let mut content = column![
        view_dashboard_header(state),
        text(state.status.clone()).size(12),
        view_dashboard_tabs(state),
    ]
    .spacing(12);

    if let Some(error) = state.dashboard.error.as_ref() {
        content = content.push(
            text(error.clone())
                .size(12)
                .color(Color::from_rgb8(185, 28, 28)),
        );
    }

    content = content.push(match state.dashboard.section {
        DashboardSection::Projects => view_projects_screen(state),
        DashboardSection::Subaccounts => view_subaccounts_screen(state),
        DashboardSection::Settings => view_settings_screen(state),
    });

    container(content)
        .padding(18)
        .width(Fill)
        .height(Fill)
        .style(panel_style)
        .into()
}

fn view_dashboard_header(state: &StudioState) -> Element<'_, Message> {
    row![
        column![
            text("Transcription Studio").size(24),
            text(if state.subaccount_session.is_some() {
                "Assigned projects"
            } else {
                "Babel projects"
            })
            .size(13),
        ]
        .spacing(4),
        Space::with_width(Fill),
        button(text("Open demo")).on_press(Message::LoadBundledDemo),
        button(text("Refresh projects")).on_press_maybe(
            (!state.dashboard.is_pending).then_some(Message::DashboardRefreshProjects)
        ),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

fn view_dashboard_tabs(state: &StudioState) -> Element<'_, Message> {
    let current = state.dashboard.section;
    let mut tabs = row![
        dashboard_tab_button("Projects", DashboardSection::Projects, current),
        dashboard_tab_button("Sub-accounts", DashboardSection::Subaccounts, current),
        dashboard_tab_button("Settings", DashboardSection::Settings, current),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);
    if state.subaccount_session.is_some() {
        tabs = tabs.push(text("Sub-account mode").size(12));
    }
    tabs.into()
}

fn dashboard_tab_button(
    label: &'static str,
    section: DashboardSection,
    current: DashboardSection,
) -> Element<'static, Message> {
    let label = if section == current {
        format!("{label} ✓")
    } else {
        label.to_string()
    };
    button(text(label))
        .on_press_maybe((section != current).then_some(Message::DashboardSectionChanged(section)))
        .into()
}

fn view_projects_screen(state: &StudioState) -> Element<'_, Message> {
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

    column![
        container(scrollable(project_list).height(Length::FillPortion(2)))
            .padding(10)
            .width(Fill)
            .style(subtle_panel_style),
        view_subaccount_section(state),
    ]
    .spacing(12)
    .into()
}

fn view_subaccounts_screen(state: &StudioState) -> Element<'_, Message> {
    if state.subaccount_session.is_some() {
        return container(
            column![
                text("Sub-account device").size(15),
                text("Management is available only from the main Babel account.").size(12),
                button(text("Use Babel account")).on_press(Message::SubaccountSignOut),
            ]
            .spacing(8),
        )
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into();
    }

    view_subaccount_owner_controls(state)
}

fn view_settings_screen(state: &StudioState) -> Element<'_, Message> {
    column![
        view_capability_settings(state),
        view_update_section(state),
        view_path_toolbar(state),
    ]
    .spacing(12)
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

fn view_subaccount_section(state: &StudioState) -> Element<'_, Message> {
    let mut content = column![text("Account mode").size(15)].spacing(8);
    if let Some(session) = state.subaccount_session.as_ref() {
        let name = if session.display_name.trim().is_empty() {
            session.subaccount_id.as_str()
        } else {
            session.display_name.as_str()
        };
        content = content.push(
            row![
                column![
                    text("Sub-account relay").size(12),
                    text(format!("{} · {}", name, session.host_url)).size(12),
                ]
                .spacing(3),
                Space::with_width(Fill),
                button(text("Use Babel account")).on_press(Message::SubaccountSignOut),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    } else {
        content = content.push(
            row![
                column![
                    text("Main Babel account").size(12),
                    text("Use Sub-accounts from the dedicated screen; keep this dashboard focused on work.").size(12),
                ]
                .spacing(3),
                Space::with_width(Fill),
                button(text("Manage Sub-accounts")).on_press(Message::DashboardSectionChanged(
                    DashboardSection::Subaccounts
                )),
                button(text("Use sub-account mode")).on_press(Message::NativeAuthUseSubaccountMode),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    }

    container(content)
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}

fn view_subaccount_owner_controls(state: &StudioState) -> Element<'_, Message> {
    let pending = state.subaccount_owner.is_pending;
    let refresh_row = row![
        text_input("Relay host URL", &state.subaccount_owner.host_url)
            .on_input(Message::SubaccountOwnerHostUrlChanged)
            .padding(6)
            .width(Fill),
        button(text("Refresh"))
            .on_press_maybe((!pending).then_some(Message::SubaccountOwnerRefresh)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let create_row = row![
        text_input(
            "Name this Sub-account, e.g. Travel laptop",
            &state.subaccount_owner.create_label
        )
        .on_input(Message::SubaccountOwnerCreateLabelChanged)
        .on_submit(Message::SubaccountOwnerCreate)
        .padding(6)
        .width(Fill),
        button(text("Create")).on_press_maybe((!pending).then_some(Message::SubaccountOwnerCreate)),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let mut content = column![
        column![
            text("Sub-accounts").size(18),
            text("Create relay logins, choose allowed projects from your loaded Babel projects, and review task/time/earnings analytics.").size(12),
        ]
        .spacing(3),
        horizontal_rule(1),
        refresh_row,
        create_row,
    ]
    .spacing(10);

    if let Some(error) = state.subaccount_owner.error.as_ref() {
        content = content.push(
            text(error.clone())
                .size(12)
                .color(Color::from_rgb8(185, 28, 28)),
        );
    }
    if pending {
        content = content.push(text("Working...").size(12));
    }

    let mut rows = column![].spacing(10);
    if state.subaccount_owner.subaccounts.is_empty() {
        rows = rows.push(text("No Sub-accounts loaded. Create one above.").size(12));
    } else {
        for subaccount in &state.subaccount_owner.subaccounts {
            let save_id = subaccount.id.clone();
            let revoke_id = subaccount.id.clone();
            let status = if subaccount.revoked {
                "Revoked"
            } else {
                "Active"
            };
            let code = subaccount
                .code
                .as_deref()
                .map(|code| format!("Login code: {code}"))
                .unwrap_or_else(|| "Login code hidden by relay".to_string());
            rows = rows.push(
                container(
                    column![
                        row![
                            column![
                                text(subaccount.label.clone()).size(15),
                                text(status).size(12),
                                text(code).size(12),
                                text(format!(
                                    "Tasks: {} · Time: {} · Earnings: {}",
                                    subaccount.completed_tasks,
                                    format_worked_time(subaccount.worked_seconds),
                                    format_money_cents(subaccount.earnings_cents),
                                ))
                                .size(12),
                            ]
                            .spacing(3),
                            Space::with_width(Fill),
                            button(text("Revoke")).on_press_maybe(
                                (!pending && !subaccount.revoked)
                                    .then_some(Message::SubaccountOwnerRevoke(revoke_id))
                            ),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                        view_project_access_controls(state, subaccount, pending),
                        button(text("Save project access")).on_press_maybe(
                            (!pending).then_some(Message::SubaccountOwnerSaveAssignment(save_id))
                        ),
                    ]
                    .spacing(10),
                )
                .padding(10)
                .width(Fill)
                .style(subtle_panel_style),
            );
        }
    }

    content = content.push(rows);

    container(content)
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}

fn view_project_access_controls<'a>(
    state: &'a StudioState,
    subaccount: &'a super::super::subaccounts::ManagedSubaccount,
    pending: bool,
) -> Element<'a, Message> {
    let assignment = state
        .subaccount_owner
        .assignment_drafts
        .get(&subaccount.id)
        .map(String::as_str)
        .unwrap_or_default();
    let selected_ids = project_assignment_set(assignment);
    let all_projects = selected_ids.is_empty();

    let mut content = column![
        row![
            column![
                text("Project access").size(13),
                text(if all_projects {
                    "Allowed: every accessible project."
                } else {
                    "Allowed: selected projects only."
                })
                .size(12),
            ]
            .spacing(3),
            Space::with_width(Fill),
            button(text("Allow every project")).on_press_maybe(
                (!pending && !all_projects).then_some(Message::SubaccountOwnerAssignmentChanged(
                    subaccount.id.clone(),
                    String::new(),
                ))
            ),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    ]
    .spacing(6);

    if state.dashboard.projects.is_empty() {
        content = content.push(
            row![
                text("Load projects to choose by name.").size(12),
                Space::with_width(Fill),
                button(text("Load projects")).on_press_maybe(
                    (!state.dashboard.is_pending).then_some(Message::DashboardRefreshProjects)
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        );
    } else {
        for project in &state.dashboard.projects {
            let subaccount_id = subaccount.id.clone();
            let project_id = project.id.clone();
            let selected = all_projects || selected_ids.contains(&project.id);
            content = content.push(
                checkbox(
                    format!("{} — {}", project.name, dashboard_project_status(project)),
                    selected,
                )
                .on_toggle(move |checked| {
                    Message::SubaccountOwnerProjectToggled(
                        subaccount_id.clone(),
                        project_id.clone(),
                        checked,
                    )
                }),
            );
        }
    }

    let unavailable_count = selected_ids
        .iter()
        .filter(|id| {
            !state
                .dashboard
                .projects
                .iter()
                .any(|project| project.id == **id)
        })
        .count();
    if unavailable_count > 0 {
        content = content.push(
            text(format!(
                "{unavailable_count} saved project assignment{} not visible in the current project list; refresh projects to review it.",
                if unavailable_count == 1 { " is" } else { "s are" }
            ))
            .size(12),
        );
    }

    container(content)
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}

fn project_assignment_set(source: &str) -> BTreeSet<String> {
    source
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn format_worked_time(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn format_money_cents(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let absolute = cents.unsigned_abs();
    format!("{sign}${}.{:02}", absolute / 100, absolute % 100)
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
                text("Build commit").width(Length::Fixed(104.0)),
                text(state.updater.build_commit.clone()).size(12),
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
