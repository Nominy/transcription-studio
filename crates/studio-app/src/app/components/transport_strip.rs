use std::{
    hash::{Hash, Hasher},
    sync::{Arc, LazyLock},
};

use iced::widget::pick_list::{Status as PickListStatus, Style as PickListStyle};
use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{Space, button, container, lazy, pick_list, row, text, tooltip, vertical_rule};
use iced::{Background, Color, Element, Fill, Length, Theme, border};
use studio_app::{
    AdminFlagControlView, EditorSession, ReadOnlyActionCommand, ReviewActionStripView,
    SkipChunkControlView, SubmitControlView, TransportStripView,
};
use studio_core::Project;

use super::super::Message;
use super::super::StudioState;

const TRANSPORT_TRACK_FILTER_WIDTH: f32 = 180.0;
const TRANSPORT_ICON_BUTTON_WIDTH: f32 = 42.0;
const TRANSPORT_BUTTON_HEIGHT: f32 = 32.0;
const TRANSPORT_CLUSTER_GAP: u16 = 8;
const REVIEW_ACTION_BUTTON_WIDTH: f32 = 118.0;
const REVIEW_SAVE_BUTTON_WIDTH: f32 = 112.0;
const REVIEW_SUBMIT_BUTTON_WIDTH: f32 = 132.0;
const TRANSPORT_STRIP_HEIGHT: f32 = 58.0;
const TRANSPORT_RULE_HEIGHT: f32 = 30.0;
const TRANSPORT_ICON_SIZE: f32 = 16.0;
const TRANSPORT_PROBLEM_BUTTON_WIDTH: f32 = 24.0;
const TRANSPORT_PROBLEM_ICON_SIZE: f32 = 14.0;
const TRANSPORT_ICON_PREVIOUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M5 5h2v14H5z"/><path d="M19 5v14L8 12z"/></svg>"#;
const TRANSPORT_ICON_PLAY: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>"#;
const TRANSPORT_ICON_PAUSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M7 5h3v14H7z"/><path d="M14 5h3v14h-3z"/></svg>"#;
const TRANSPORT_ICON_NEXT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="currentColor"><path d="M5 5v14l11-7z"/><path d="M17 5h2v14h-2z"/></svg>"#;
const TRANSPORT_ICON_UNDO: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-2"/></svg>"#;
const TRANSPORT_ICON_REDO: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m15 14 5-5-5-5"/><path d="M20 9H10a6 6 0 0 0 0 12h2"/></svg>"#;
const TRANSPORT_ICON_PROBLEM_PREVIOUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m18 15-6-6-6 6"/></svg>"#;
const TRANSPORT_ICON_PROBLEM_NEXT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
static TRANSPORT_ICON_PREVIOUS_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_PREVIOUS));
static TRANSPORT_ICON_PLAY_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_PLAY));
static TRANSPORT_ICON_PAUSE_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_PAUSE));
static TRANSPORT_ICON_NEXT_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_NEXT));
static TRANSPORT_ICON_UNDO_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_UNDO));
static TRANSPORT_ICON_REDO_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_REDO));
static TRANSPORT_ICON_PROBLEM_PREVIOUS_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_PROBLEM_PREVIOUS));
static TRANSPORT_ICON_PROBLEM_NEXT_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(TRANSPORT_ICON_PROBLEM_NEXT));

struct TransportStripDependency {
    strip: Arc<TransportStripView>,
    submit: Arc<SubmitControlView>,
    skip_chunk: Option<SkipChunkControlView>,
    admin_flag: Option<AdminFlagControlView>,
    show_track_filter: bool,
    drop_claim_is_pending: bool,
}

impl std::hash::Hash for TransportStripDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.strip).hash(state);
        Arc::as_ptr(&self.submit).hash(state);
        self.skip_chunk.is_some().hash(state);
        if let Some(skip_chunk) = &self.skip_chunk {
            hash_skip_chunk(skip_chunk, state);
        }
        self.admin_flag.is_some().hash(state);
        if let Some(admin_flag) = &self.admin_flag {
            hash_admin_flag(admin_flag, state);
        }
        self.show_track_filter.hash(state);
        self.drop_claim_is_pending.hash(state);
    }
}

pub(super) fn view_transport_strip<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let strip = state.transport_strip_snapshot_for_render(project, editor);
    let submit = state.submit_control_snapshot_for_render(project, editor);
    let skip_chunk = EditorSession::skip_chunk_control_available(project).then(|| {
        EditorSession::skip_chunk_control(
            project,
            state.show_skip_chunk_dialog,
            state.skip_is_pending,
        )
    });
    let admin_flag = EditorSession::admin_flag_control_available(project).then(|| {
        EditorSession::admin_flag_control(
            state.show_admin_flag_dialog,
            state.admin_flag_reason.clone(),
            state.admin_flag_is_pending,
        )
    });
    view_lazy_transport_strip(TransportStripDependency {
        strip,
        submit,
        skip_chunk,
        admin_flag,
        show_track_filter: editor.transcript_visible_cells(project).track,
        drop_claim_is_pending: state.drop_claim_is_pending,
    })
}

fn view_lazy_transport_strip(dependency: TransportStripDependency) -> Element<'static, Message> {
    lazy(dependency, view_transport_strip_content).into()
}

fn view_transport_strip_content(
    dependency: &TransportStripDependency,
) -> Element<'static, Message> {
    let strip = dependency.strip.as_ref();
    let submit = dependency.submit.as_ref();
    let mut controls = row![]
        .spacing(TRANSPORT_CLUSTER_GAP)
        .align_y(iced::Alignment::Center);

    if dependency.show_track_filter {
        controls = controls
            .push(
                pick_list(
                    strip.track_filter_options.clone(),
                    Some(strip.track_filter.clone()),
                    Message::TrackFilterChanged,
                )
                .text_size(14)
                .padding([5, 12])
                .style(transport_track_filter_style)
                .width(Length::Fixed(TRANSPORT_TRACK_FILTER_WIDTH)),
            )
            .push(transport_separator());
    }

    controls = controls
        .push(transport_svg_icon_button(
            TRANSPORT_ICON_PREVIOUS_HANDLE.clone(),
            "Previous Segment",
            false,
            Some(Message::PreviousSegment),
        ))
        .push(transport_svg_icon_button(
            play_icon_handle(&strip.play_label),
            "Play/Pause",
            strip.play_label == "Pause",
            Some(Message::TogglePlayback),
        ))
        .push(transport_svg_icon_button(
            TRANSPORT_ICON_NEXT_HANDLE.clone(),
            "Next Segment",
            false,
            Some(Message::NextSegment),
        ))
        .push(transport_separator())
        .push(transport_svg_icon_button(
            TRANSPORT_ICON_UNDO_HANDLE.clone(),
            "Undo",
            false,
            (!strip.undo_disabled).then_some(Message::Undo),
        ))
        .push(transport_svg_icon_button(
            TRANSPORT_ICON_REDO_HANDLE.clone(),
            "Redo",
            false,
            (!strip.redo_disabled).then_some(Message::Redo),
        ))
        .push(Space::with_width(Fill));

    if let Some(problem_summary_label) = strip.problem_summary_label.as_ref() {
        controls = controls.push(view_problem_summary_group(problem_summary_label.clone()));
    }

    controls = match &strip.review_actions {
        ReviewActionStripView::Editable { drop_claim } => {
            let mut editable_controls = controls;
            let submit_tooltip_content = submit.trigger_tooltip_content.clone();
            if let Some(drop_claim) = drop_claim {
                editable_controls = editable_controls.push(
                    tooltip(
                        fixed_review_button(
                            drop_claim.label.clone(),
                            REVIEW_ACTION_BUTTON_WIDTH,
                            (!drop_claim.disabled && !dependency.drop_claim_is_pending)
                                .then_some(Message::DropClaim),
                        ),
                        container(text(drop_claim.tooltip.clone()).size(12)).padding(6),
                        tooltip::Position::FollowCursor,
                    )
                    .style(container::rounded_box),
                );
            }
            if let Some(skip_chunk) = &dependency.skip_chunk {
                editable_controls = editable_controls.push(
                    tooltip(
                        fixed_review_button(
                            skip_chunk.trigger_label.clone(),
                            REVIEW_ACTION_BUTTON_WIDTH,
                            (!skip_chunk.trigger_disabled).then_some(Message::RequestSkipChunk),
                        ),
                        container(
                            text(skip_chunk.trigger_tooltip.clone().unwrap_or_else(|| {
                                "Archive this chunk and return to the project page.".to_string()
                            }))
                            .size(12),
                        )
                        .padding(6),
                        tooltip::Position::FollowCursor,
                    )
                    .style(container::rounded_box),
                );
            }
            if let Some(admin_flag) = &dependency.admin_flag {
                editable_controls = editable_controls.push(fixed_review_button(
                    admin_flag.trigger_label.clone(),
                    REVIEW_ACTION_BUTTON_WIDTH,
                    Some(Message::RequestAdminFlag),
                ));
            }
            editable_controls
                .push(
                    tooltip(
                        fixed_review_button(
                            strip.save_label.clone(),
                            REVIEW_SAVE_BUTTON_WIDTH,
                            (strip.save_clickable && !strip.save_disabled)
                                .then_some(Message::SaveProgress),
                        ),
                        container(text(strip.save_tooltip_content.clone()).size(12)).padding(6),
                        tooltip::Position::FollowCursor,
                    )
                    .style(container::rounded_box),
                )
                .push(
                    tooltip(
                        fixed_submit_button(
                            submit.trigger_button.clone(),
                            REVIEW_SUBMIT_BUTTON_WIDTH,
                            !submit.trigger_disabled,
                            (!submit.show_pending_overlay).then_some(Message::RequestSubmitReview),
                        ),
                        container(text(submit_tooltip_content).size(12)).padding(6),
                        tooltip::Position::FollowCursor,
                    )
                    .style(container::rounded_box),
                )
        }
        ReviewActionStripView::ReadOnly { notice, actions } => {
            let mut read_only_controls = controls;
            if let Some(notice) = notice {
                read_only_controls = read_only_controls.push(text(notice.clone()).size(12));
            }
            for action in actions {
                read_only_controls = read_only_controls.push(fixed_review_button(
                    action.label.clone(),
                    REVIEW_ACTION_BUTTON_WIDTH,
                    (!action.disabled).then_some(read_only_action_message(action.command)),
                ));
            }
            read_only_controls
        }
    };

    container(controls)
        .padding(0)
        .width(Fill)
        .height(Length::Fixed(TRANSPORT_STRIP_HEIGHT))
        .into()
}

fn hash_skip_chunk<H: Hasher>(skip_chunk: &SkipChunkControlView, state: &mut H) {
    skip_chunk.open.hash(state);
    skip_chunk.trigger_label.hash(state);
    skip_chunk.trigger_disabled.hash(state);
    skip_chunk.trigger_tooltip.hash(state);
    skip_chunk.title.hash(state);
    skip_chunk.description.hash(state);
    skip_chunk.cancel_label.hash(state);
    skip_chunk.cancel_disabled.hash(state);
    skip_chunk.confirm_label.hash(state);
    skip_chunk.confirm_disabled.hash(state);
}

fn hash_admin_flag<H: Hasher>(admin_flag: &AdminFlagControlView, state: &mut H) {
    admin_flag.open.hash(state);
    admin_flag.trigger_label.hash(state);
    admin_flag.title.hash(state);
    admin_flag.description.hash(state);
    admin_flag.reason_label.hash(state);
    admin_flag.reason_placeholder.hash(state);
    admin_flag.reason_value.hash(state);
    admin_flag.reason_disabled.hash(state);
    admin_flag.cancel_label.hash(state);
    admin_flag.cancel_disabled.hash(state);
    admin_flag.confirm_label.hash(state);
    admin_flag.confirm_disabled.hash(state);
}

fn view_problem_summary_group(problem_summary_label: String) -> Element<'static, Message> {
    let is_error = problem_summary_label.contains("error");
    let text_color = problem_summary_text_color(is_error);

    container(
        row![
            text(problem_summary_label).size(14).color(text_color),
            row![
                problem_summary_svg_button(
                    TRANSPORT_ICON_PROBLEM_PREVIOUS_HANDLE.clone(),
                    "Previous problem",
                    Message::NavigateProblemRows(-1),
                    is_error,
                ),
                problem_summary_svg_button(
                    TRANSPORT_ICON_PROBLEM_NEXT_HANDLE.clone(),
                    "Next problem",
                    Message::NavigateProblemRows(1),
                    is_error,
                ),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    )
    .padding([0, 8])
    .height(Length::Fixed(32.0))
    .style(problem_summary_group_style(is_error))
    .into()
}

fn problem_summary_svg_button(
    icon: SvgHandle,
    label: &'static str,
    message: Message,
    is_error: bool,
) -> Element<'static, Message> {
    let color = problem_summary_text_color(is_error);
    let icon = Svg::new(icon)
        .width(Length::Fixed(TRANSPORT_PROBLEM_ICON_SIZE))
        .height(Length::Fixed(TRANSPORT_PROBLEM_ICON_SIZE))
        .style(move |_, _| svg::Style { color: Some(color) });

    tooltip(
        button(icon)
            .style(problem_summary_nav_button_style(is_error))
            .on_press(message)
            .padding(0)
            .width(Length::Fixed(TRANSPORT_PROBLEM_BUTTON_WIDTH))
            .height(Length::Fixed(TRANSPORT_PROBLEM_BUTTON_WIDTH)),
        container(text(label).size(12)).padding(6),
        tooltip::Position::FollowCursor,
    )
    .style(container::rounded_box)
    .into()
}

fn problem_summary_group_style(is_error: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| {
        let background = if is_error {
            Color::from_rgb8(254, 242, 242)
        } else {
            Color::from_rgb8(254, 252, 232)
        };
        let border_color = if is_error {
            Color::from_rgb8(254, 202, 202)
        } else {
            Color::from_rgb8(254, 240, 138)
        };

        container::Style::default()
            .background(background)
            .border(border::rounded(6).width(0.5).color(border_color))
    }
}

fn problem_summary_nav_button_style(
    is_error: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover_color = if is_error {
            Color::from_rgb8(254, 226, 226)
        } else {
            Color::from_rgb8(254, 249, 195)
        };
        let background = match status {
            button::Status::Hovered | button::Status::Pressed => {
                Some(Background::Color(hover_color))
            }
            button::Status::Active | button::Status::Disabled => None,
        };

        button::Style {
            background,
            text_color: problem_summary_text_color(is_error),
            border: border::rounded(4).width(0).color(Color::TRANSPARENT),
            shadow: Default::default(),
        }
    }
}

fn problem_summary_text_color(is_error: bool) -> Color {
    if is_error {
        Color::from_rgb8(239, 68, 68)
    } else {
        Color::from_rgb8(202, 138, 4)
    }
}

fn read_only_action_message(command: ReadOnlyActionCommand) -> Message {
    match command {
        ReadOnlyActionCommand::RetryPractice => Message::RetryPractice,
        ReadOnlyActionCommand::TakeAssessment => Message::TakeAssessment,
        ReadOnlyActionCommand::RetryAssessment => Message::RetryAssessment,
        ReadOnlyActionCommand::BackToProjects => Message::BackToProjects,
    }
}

fn transport_svg_icon_button<'a>(
    icon: SvgHandle,
    label: &'static str,
    is_active: bool,
    message: Option<Message>,
) -> Element<'a, Message> {
    let icon = Svg::new(icon)
        .width(Length::Fixed(TRANSPORT_ICON_SIZE))
        .height(Length::Fixed(TRANSPORT_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::from_rgb8(75, 85, 99)),
        });

    tooltip(
        button(icon)
            .style(transport_icon_button_style(is_active))
            .on_press_maybe(message)
            .padding(0)
            .width(Length::Fixed(TRANSPORT_ICON_BUTTON_WIDTH))
            .height(Length::Fixed(TRANSPORT_BUTTON_HEIGHT)),
        container(text(label).size(12)).padding(6),
        tooltip::Position::FollowCursor,
    )
    .style(container::rounded_box)
    .into()
}

fn transport_separator<'a>() -> Element<'a, Message> {
    container(vertical_rule(1))
        .height(Length::Fixed(TRANSPORT_RULE_HEIGHT))
        .into()
}

fn transport_track_filter_style(_theme: &Theme, status: PickListStatus) -> PickListStyle {
    let background = match status {
        PickListStatus::Hovered | PickListStatus::Opened => Color::from_rgb8(245, 245, 245),
        PickListStatus::Active => Color::WHITE,
    };

    PickListStyle {
        text_color: Color::from_rgb8(75, 85, 99),
        placeholder_color: Color::from_rgb8(115, 115, 115),
        handle_color: Color::from_rgb8(75, 85, 99),
        background: Background::Color(background),
        border: border::rounded(6)
            .width(1)
            .color(Color::from_rgb8(229, 229, 229)),
    }
}

fn fixed_review_button(
    label: String,
    width: f32,
    message: Option<Message>,
) -> Element<'static, Message> {
    button(text(label).size(13))
        .style(transport_text_button_style)
        .on_press_maybe(message)
        .padding([0, 12])
        .width(Length::Fixed(width))
        .height(Length::Fixed(TRANSPORT_BUTTON_HEIGHT))
        .into()
}

fn fixed_submit_button(
    label: String,
    width: f32,
    visually_enabled: bool,
    message: Option<Message>,
) -> Element<'static, Message> {
    button(text(label).size(13))
        .style(transport_submit_button_style(visually_enabled))
        .on_press_maybe(message)
        .padding([0, 12])
        .width(Length::Fixed(width))
        .height(Length::Fixed(TRANSPORT_BUTTON_HEIGHT))
        .into()
}

fn transport_icon_button_style(
    is_active: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match status {
            button::Status::Hovered => Color::from_rgb8(229, 229, 229),
            button::Status::Pressed => Color::from_rgb8(212, 212, 212),
            button::Status::Disabled => Color::from_rgb8(245, 245, 245),
            button::Status::Active if is_active => Color::from_rgb8(212, 212, 212),
            button::Status::Active => Color::WHITE,
        };
        let text_color = match status {
            button::Status::Disabled => Color::from_rgb8(163, 163, 163),
            _ => Color::from_rgb8(115, 115, 115),
        };
        let border_color = match status {
            button::Status::Disabled => Color::from_rgb8(229, 229, 229),
            _ => Color::from_rgb8(229, 229, 229),
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: border::rounded(6).width(1).color(border_color),
            shadow: Default::default(),
        }
    }
}

fn transport_text_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(229, 229, 229),
        button::Status::Pressed => Color::from_rgb8(212, 212, 212),
        button::Status::Disabled => Color::from_rgb8(245, 245, 245),
        button::Status::Active => Color::WHITE,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(163, 163, 163),
        _ => Color::from_rgb8(115, 115, 115),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: border::rounded(6)
            .width(1)
            .color(Color::from_rgb8(229, 229, 229)),
        shadow: Default::default(),
    }
}

fn transport_submit_button_style(
    is_enabled: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match status {
            button::Status::Active if is_enabled => Color::from_rgb8(9, 9, 11),
            button::Status::Hovered if is_enabled => Color::from_rgb8(39, 39, 42),
            button::Status::Pressed if is_enabled => Color::from_rgb8(63, 63, 70),
            _ => Color::from_rgb8(245, 245, 245),
        };
        let text_color = if is_enabled {
            Color::WHITE
        } else {
            Color::from_rgb8(163, 163, 163)
        };
        let border_color = if is_enabled {
            Color::from_rgb8(9, 9, 11)
        } else {
            Color::from_rgb8(229, 229, 229)
        };

        button::Style {
            background: Some(Background::Color(background)),
            text_color,
            border: border::rounded(6).width(1).color(border_color),
            shadow: Default::default(),
        }
    }
}

fn play_icon_handle(play_label: &str) -> SvgHandle {
    if play_label == "Pause" {
        TRANSPORT_ICON_PAUSE_HANDLE.clone()
    } else {
        TRANSPORT_ICON_PLAY_HANDLE.clone()
    }
}
