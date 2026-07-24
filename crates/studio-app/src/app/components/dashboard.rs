use std::collections::BTreeSet;

use iced::widget::{
    Space, button, checkbox, column, container, horizontal_rule, row, scrollable, text, text_input,
};
use iced::{Color, Element, Fill, Font, Length, font};

use super::super::native_auth::{
    default_native_auth_store_path, executable_native_auth_store_path,
};
use super::super::state::DashboardSection;
use super::super::styles::{
    app_background_style, brand_mark_style, dashboard_header_style, dashboard_nav_button_style,
    dashboard_sidebar_style, panel_style, primary_button_style, project_card_style,
    secondary_button_style, subtle_panel_style, success_banner_style, success_pill_style,
    warning_banner_style, warning_pill_style,
};
use super::super::{DEFAULT_EXPORT_PATH, DEFAULT_IMPORT_PATH, Message, StudioState};

const DASHBOARD_SIDEBAR_WIDTH: f32 = 244.0;
const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_dashboard(state: &StudioState) -> Element<'_, Message> {
    let mut section = column![].spacing(16).width(Fill).height(Fill);

    if let Some(notice) = state.dashboard.notice.as_ref() {
        section = section.push(
            container(
                row![
                    text("✓").size(14).font(SEMIBOLD),
                    text(notice.clone()).size(13),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            )
            .padding([11, 14])
            .width(Fill)
            .style(success_banner_style),
        );
    }

    if let Some(error) = state.dashboard.error.as_ref() {
        section = section.push(
            container(
                column![
                    text("We couldn't refresh this workspace")
                        .size(14)
                        .font(SEMIBOLD),
                    text(error.clone()).size(12),
                ]
                .spacing(4),
            )
            .padding([12, 14])
            .width(Fill)
            .style(|_| {
                iced::widget::container::Style::default()
                    .background(Color::from_rgb8(254, 242, 242))
                    .color(Color::from_rgb8(153, 27, 27))
                    .border(
                        iced::border::rounded(8)
                            .width(1)
                            .color(Color::from_rgb8(254, 202, 202)),
                    )
            }),
        );
    }

    section = section.push(match state.dashboard.section {
        DashboardSection::Projects => view_projects_screen(state),
        DashboardSection::Subaccounts => view_subaccounts_screen(state),
        DashboardSection::Settings => view_settings_screen(state),
    });

    let main = column![
        view_dashboard_header(state),
        container(section)
            .padding([24, 28])
            .width(Fill)
            .height(Fill),
    ]
    .spacing(0)
    .width(Fill)
    .height(Fill);

    container(row![view_dashboard_sidebar(state), main].spacing(0))
        .width(Fill)
        .height(Fill)
        .style(app_background_style)
        .into()
}

fn view_dashboard_header(state: &StudioState) -> Element<'_, Message> {
    let (title, subtitle) = match state.dashboard.section {
        DashboardSection::Projects => (
            "Projects",
            if state.subaccount_session.is_some() {
                "Choose an assigned project and continue where you left off."
            } else {
                "Choose a Babel project and claim the next available task."
            },
        ),
        DashboardSection::Subaccounts => (
            "Sub-accounts",
            "Create focused workspaces and control which projects they can access.",
        ),
        DashboardSection::Settings => (
            "Settings",
            "Manage this installation, integrations, updates, and local paths.",
        ),
    };

    let mut actions = row![].spacing(10).align_y(iced::Alignment::Center);
    if state.dashboard.section == DashboardSection::Projects {
        let readiness = if state.dashboard.is_pending {
            let pending_label = if state.status.to_ascii_lowercase().contains("audio") {
                "Preparing audio"
            } else {
                "Syncing projects"
            };
            container(text(pending_label).size(12))
                .padding([5, 10])
                .style(warning_pill_style)
        } else {
            let (completed_today, daily_limit, limited_project_count) =
                dashboard_task_progress_totals(&state.dashboard.projects);
            let readiness_label = if limited_project_count > 0 {
                format!("{completed_today} of {daily_limit} tasks today")
            } else {
                format!("{} available", state.dashboard.projects.len())
            };
            container(text(readiness_label).size(12))
                .padding([5, 10])
                .style(success_pill_style)
        };
        actions = actions.push(readiness);
        if cfg!(debug_assertions) {
            actions = actions.push(
                button(text("Open demo").size(13))
                    .style(secondary_button_style)
                    .on_press(Message::LoadBundledDemo)
                    .padding([9, 14]),
            );
        }
        actions = actions.push(
            button(
                text(if state.dashboard.is_pending {
                    "Refreshing..."
                } else {
                    "Refresh projects"
                })
                .size(13),
            )
            .style(primary_button_style)
            .on_press_maybe(
                (!state.dashboard.is_pending).then_some(Message::DashboardRefreshProjects),
            )
            .padding([9, 14]),
        );
    }

    container(
        row![
            column![
                text(title).size(22).font(SEMIBOLD),
                text(subtitle)
                    .size(13)
                    .color(Color::from_rgb8(100, 116, 139)),
            ]
            .spacing(4),
            Space::with_width(Fill),
            actions,
        ]
        .spacing(16)
        .align_y(iced::Alignment::Center),
    )
    .padding([18, 28])
    .width(Fill)
    .style(dashboard_header_style)
    .into()
}

fn view_dashboard_sidebar(state: &StudioState) -> Element<'_, Message> {
    let current = state.dashboard.section;
    let navigation = column![
        text("WORKSPACE")
            .size(10)
            .color(Color::from_rgb8(148, 163, 184)),
        dashboard_tab_button("Projects", DashboardSection::Projects, current),
        dashboard_tab_button("Sub-accounts", DashboardSection::Subaccounts, current),
        dashboard_tab_button("Settings", DashboardSection::Settings, current),
    ]
    .spacing(6);

    container(
        column![
            row![
                container(text("TS").size(14).font(SEMIBOLD))
                    .width(Length::Fixed(38.0))
                    .height(Length::Fixed(38.0))
                    .center_x(Length::Fixed(38.0))
                    .center_y(Length::Fixed(38.0))
                    .style(brand_mark_style),
                column![
                    text("Transcription Studio").size(15).font(SEMIBOLD),
                    text("Babel workspace")
                        .size(11)
                        .color(Color::from_rgb8(148, 163, 184)),
                ]
                .spacing(2),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
            Space::with_height(Length::Fixed(24.0)),
            navigation,
            Space::with_height(Fill),
            view_subaccount_section(state),
            text(format!("Version {}", env!("CARGO_PKG_VERSION")))
                .size(10)
                .color(Color::from_rgb8(100, 116, 139)),
        ]
        .spacing(14),
    )
    .padding([22, 16])
    .width(Length::Fixed(DASHBOARD_SIDEBAR_WIDTH))
    .height(Fill)
    .style(dashboard_sidebar_style)
    .into()
}

fn dashboard_tab_button(
    label: &'static str,
    section: DashboardSection,
    current: DashboardSection,
) -> Element<'static, Message> {
    button(text(label).size(13).font(if section == current {
        SEMIBOLD
    } else {
        Font::DEFAULT
    }))
    .style(dashboard_nav_button_style(section == current))
    .on_press_maybe((section != current).then_some(Message::DashboardSectionChanged(section)))
    .padding([10, 12])
    .width(Fill)
    .into()
}

fn view_projects_screen(state: &StudioState) -> Element<'_, Message> {
    let mut project_list = column![].spacing(10);
    if state.dashboard.projects.is_empty() {
        project_list = project_list.push(
            container(
                column![
                    text(if state.dashboard.is_pending {
                        "Loading your projects..."
                    } else {
                        "No active projects"
                    })
                    .size(17)
                    .font(SEMIBOLD),
                    text(if state.dashboard.is_pending {
                        "This usually takes only a moment."
                    } else {
                        "Refresh the workspace after Babel grants access to a project."
                    })
                    .size(13)
                    .color(Color::from_rgb8(100, 116, 139)),
                ]
                .spacing(6)
                .align_x(iced::Alignment::Center),
            )
            .padding(48)
            .width(Fill)
            .style(project_card_style),
        );
    } else {
        for project in &state.dashboard.projects {
            project_list = project_list.push(
                container(
                    row![
                        column![
                            text(project.name.clone()).size(15).font(SEMIBOLD),
                            row![
                                text("●").size(10).color(
                                    if project.chunked_queue_id.trim().is_empty() {
                                        Color::from_rgb8(148, 163, 184)
                                    } else {
                                        Color::from_rgb8(16, 185, 129)
                                    }
                                ),
                                text(dashboard_project_status(project))
                                    .size(12)
                                    .color(Color::from_rgb8(100, 116, 139)),
                            ]
                            .spacing(6)
                            .align_y(iced::Alignment::Center),
                        ]
                        .push_maybe(project.daily_task_limit.map(|limit| {
                            text(format!(
                                "{} of {limit} tasks today",
                                project.tasks_completed_today
                            ))
                            .size(11)
                            .color(Color::from_rgb8(100, 116, 139))
                        }))
                        .spacing(6),
                        Space::with_width(Fill),
                        button(text("Start task").size(13))
                            .style(primary_button_style)
                            .on_press_maybe(
                                (!state.dashboard.is_pending
                                    && !project.chunked_queue_id.trim().is_empty())
                                .then_some(Message::DashboardStartProject(project.id.clone())),
                            )
                            .padding([9, 16]),
                    ]
                    .spacing(16)
                    .align_y(iced::Alignment::Center),
                )
                .padding([15, 16])
                .width(Fill)
                .style(project_card_style),
            );
        }
    }

    let mut content = column![
        row![
            text("Available projects").size(14).font(SEMIBOLD),
            Space::with_width(Fill),
            text(format!("{} total", state.dashboard.projects.len()))
                .size(12)
                .color(Color::from_rgb8(100, 116, 139)),
        ]
        .align_y(iced::Alignment::Center),
    ]
    .spacing(12)
    .height(Fill);

    if let Some(draft) = state.dashboard.recovery_drafts.first() {
        let count = state.dashboard.recovery_drafts.len();
        content = content.push(
            container(
                row![
                    column![
                        text(if count == 1 {
                            "A local recovery draft is available"
                        } else {
                            "Local recovery drafts are available"
                        })
                        .size(13)
                        .font(SEMIBOLD),
                        text(format!(
                            "Restore {}{} before starting another task.",
                            draft.label,
                            if count > 1 {
                                format!(" or one of {} other drafts", count - 1)
                            } else {
                                String::new()
                            }
                        ))
                        .size(12),
                    ]
                    .spacing(3),
                    Space::with_width(Fill),
                    button(text("Recover latest").size(13))
                        .style(primary_button_style)
                        .on_press(Message::DashboardRecoverLatestDraft)
                        .padding([9, 14]),
                ]
                .spacing(16)
                .align_y(iced::Alignment::Center),
            )
            .padding([12, 14])
            .width(Fill)
            .style(warning_banner_style),
        );
    }

    content.push(scrollable(project_list).height(Fill)).into()
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
        view_relay_settings(state),
        view_auth_store_settings(),
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

/// Sums real Babel worker-reported daily task progress across projects that
/// carry a `daily_task_limit`. Projects without a reported limit are excluded
/// from both totals so the aggregate never implies a fabricated cap.
fn dashboard_task_progress_totals(
    projects: &[super::super::state::DashboardProject],
) -> (u64, u64, usize) {
    projects
        .iter()
        .filter_map(|project| project.daily_task_limit.map(|limit| (project, limit)))
        .fold(
            (0u64, 0u64, 0usize),
            |(completed, limit_total, count), (project, limit)| {
                (
                    completed + project.tasks_completed_today,
                    limit_total + limit,
                    count + 1,
                )
            },
        )
}

fn view_subaccount_section(state: &StudioState) -> Element<'_, Message> {
    let mut content = column![
        text("ACCOUNT")
            .size(10)
            .color(Color::from_rgb8(148, 163, 184))
    ]
    .spacing(6);
    if let Some(session) = state.subaccount_session.as_ref() {
        let name = if session.display_name.trim().is_empty() {
            session.subaccount_id.as_str()
        } else {
            session.display_name.as_str()
        };
        content = content
            .push(text(name).size(13).font(SEMIBOLD))
            .push(
                text("Sub-account workspace")
                    .size(11)
                    .color(Color::from_rgb8(148, 163, 184)),
            )
            .push(
                button(text("Use Babel account").size(12))
                    .style(dashboard_nav_button_style(false))
                    .on_press(Message::SubaccountSignOut)
                    .padding([8, 10])
                    .width(Fill),
            );
    } else {
        content = content
            .push(text("Main Babel account").size(13).font(SEMIBOLD))
            .push(
                text("Full project access")
                    .size(11)
                    .color(Color::from_rgb8(148, 163, 184)),
            )
            .push(
                button(text("Use sub-account code").size(12))
                    .style(dashboard_nav_button_style(false))
                    .on_press(Message::NativeAuthUseSubaccountMode)
                    .padding([8, 10])
                    .width(Fill),
            );
    }

    content.into()
}

fn view_relay_settings(state: &StudioState) -> Element<'_, Message> {
    let start_button = button(text("Start relay"))
        .style(primary_button_style)
        .on_press_maybe(
            state
                .relay_process
                .can_start()
                .then_some(Message::StartSubaccountRelay),
        );
    let stop_button = button(text("Stop relay"))
        .style(secondary_button_style)
        .on_press_maybe(
            state
                .relay_process
                .can_stop()
                .then_some(Message::StopSubaccountRelay),
        );
    let save_button = button(text("Save URL"))
        .style(secondary_button_style)
        .on_press(Message::SaveSubaccountRelayUrl);

    container(
        column![
            text("Sub-account relay").size(15).font(SEMIBOLD),
            text("Start the bundled relay on this machine and remember its URL for every Transcription Studio screen.")
                .size(12),
            row![
                text_input("Relay URL, e.g. http://127.0.0.1:8787", &state.subaccount_owner.host_url)
                    .on_input(Message::SubaccountOwnerHostUrlChanged)
                    .padding(6)
                    .width(Fill),
                save_button,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            row![
                start_button,
                stop_button,
                text(state.relay_process.label()).size(12),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}
fn view_auth_store_settings() -> Element<'static, Message> {
    let current_path = default_native_auth_store_path();
    let target_label = executable_native_auth_store_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("Unavailable: {error}"));

    let mut details = column![].spacing(3);
    if cfg!(debug_assertions) {
        details = details
            .push(text(format!("Current: {}", current_path.display())).size(12))
            .push(text(format!("Portable target: {target_label}")).size(12));
    }

    container(
        column![
            column![
                text("Session & portability").size(15).font(SEMIBOLD),
                text("Keep your Babel session with this installation when you move it to another device.")
                    .size(12)
                    .color(Color::from_rgb8(100, 116, 139)),
            ]
            .spacing(3),
            horizontal_rule(1),
            details,
            row![
                Space::with_width(Fill),
                button(text("Make installation portable").size(13))
                    .style(secondary_button_style)
                    .on_press(Message::NativeAuthDetachStore),
            ]
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
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

    content = content.push(scrollable(rows).height(Fill));

    container(content)
        .padding(10)
        .width(Fill)
        .height(Fill)
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
    let check_button = button(text("Check for updates").size(13))
        .style(secondary_button_style)
        .on_press_maybe(
            state
                .updater
                .can_check()
                .then_some(Message::CheckForUpdates),
        );
    let install_button = button(text("Install update").size(13))
        .style(primary_button_style)
        .on_press_maybe(
            state
                .updater
                .can_install()
                .then_some(Message::InstallAvailableUpdate),
        );

    let mut details = column![
        row![
            text("Current version").width(Length::Fixed(104.0)),
            text(state.updater.current_version.clone()).size(12),
        ]
        .spacing(8),
        text(state.updater.status_label())
            .size(12)
            .color(status_color),
    ]
    .spacing(8);
    if cfg!(debug_assertions) {
        details = details
            .push(
                row![
                    text("Build commit").width(Length::Fixed(104.0)),
                    text(state.updater.build_commit.clone()).size(12),
                ]
                .spacing(8),
            )
            .push(
                row![
                    text("Update feed").width(Length::Fixed(104.0)),
                    text(state.updater.feed_url.clone()).size(12),
                ]
                .spacing(8),
            );
    }

    container(
        column![
            row![
                column![
                    text("Updates").size(15).font(SEMIBOLD),
                    text("Early Access channel")
                        .size(12)
                        .color(Color::from_rgb8(100, 116, 139)),
                ]
                .spacing(3),
                Space::with_width(Fill),
                check_button,
                install_button,
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
            horizontal_rule(1),
            details,
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

    container(
        column![
            column![
                text("Local task files").size(15).font(SEMIBOLD),
                text("Import or export Babel-compatible JSON for offline work and recovery.")
                    .size(12)
                    .color(Color::from_rgb8(100, 116, 139)),
            ]
            .spacing(3),
            horizontal_rule(1),
            import_row,
            export_row,
        ]
        .spacing(8),
    )
    .padding(10)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}

#[cfg(test)]
mod dashboard_task_progress_tests {
    use super::super::super::state::DashboardProject;
    use super::dashboard_task_progress_totals;

    fn project(name: &str, completed_today: u64, limit: Option<u64>) -> DashboardProject {
        DashboardProject {
            id: name.to_string(),
            name: name.to_string(),
            chunked_queue_id: "queue".to_string(),
            tasks_completed_today: completed_today,
            daily_task_limit: limit,
            ..DashboardProject::default()
        }
    }

    #[test]
    fn sums_only_projects_with_a_reported_daily_limit() {
        let projects = [
            project("RU-tx-gold", 7, Some(25)),
            project("RU-tx-gold-non-bg", 3, Some(20)),
            project("RU-Emotion-VA", 2, None),
        ];

        let (completed, limit, limited_count) = dashboard_task_progress_totals(&projects);

        assert_eq!(completed, 10);
        assert_eq!(limit, 45);
        assert_eq!(limited_count, 2);
    }

    #[test]
    fn reports_no_limited_projects_when_babel_enforces_no_cap() {
        let projects = [project("RU-Emotion-VA", 2, None)];

        let (completed, limit, limited_count) = dashboard_task_progress_totals(&projects);

        assert_eq!(completed, 0);
        assert_eq!(limit, 0);
        assert_eq!(limited_count, 0);
    }

    #[test]
    fn empty_project_list_totals_to_zero() {
        let (completed, limit, limited_count) = dashboard_task_progress_totals(&[]);
        assert_eq!((completed, limit, limited_count), (0, 0, 0));
    }
}
