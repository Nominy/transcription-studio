use std::{
    hash::Hasher,
    sync::{Arc, LazyLock},
};

use iced::widget::pick_list::{Status as PickListStatus, Style as PickListStyle};
use iced::widget::svg::{self, Handle as SvgHandle, Svg};
use iced::widget::{
    Column, Row, Space, button, container, lazy, pick_list, responsive, row, slider, stack, text,
    tooltip, vertical_rule,
};
use iced::{Background, Color, Element, Fill, Length, Size, Theme, border};
use studio_app::{
    DiffAnnotationMetricTooltipView, DiffAnnotationMetricView, DiffCompareOptionView,
    DiffTimestampMetricTooltipView, DiffToolbarMetricsView, EditorSession,
    OverviewLaneWaveformView, TimeRange, build_toolbar_feedback_button,
};
use studio_core::Project;

use super::super::Message;
use super::super::formatting::{
    BABEL_ZOOM_SLIDER_MAX, BABEL_ZOOM_SLIDER_MIN, babel_slider_from_zoom_factor,
    babel_speed_labels, format_speed, zoom_factor_from_babel_slider,
};
use super::super::styles::{
    diff_switch_knob_style, diff_switch_style, editor_band_style, outline_button_style,
    zoom_slider_style,
};
use super::drag_capture::view_horizontal_drag_capture;
use super::timed_widget::view_timed;
use super::wavesurfer_canvas::{
    view_overview_blocks_canvas, view_overview_playhead_canvas, view_overview_window_quad,
    view_wavesurfer_line_waveform, waveform_color,
};

const DIFF_SWITCH_WIDTH: f32 = 44.0;
const DIFF_SWITCH_HEIGHT: f32 = 24.0;
const DIFF_SWITCH_KNOB_SIZE: f32 = 14.0;
const TOP_TOOLBAR_HEIGHT: f32 = 64.0;
const TOP_TOOLBAR_RULE_HEIGHT: f32 = 34.0;
const TOP_TOOLBAR_CONTROL_HEIGHT: f32 = 32.0;
const TOP_TOOLBAR_SPEED_WIDTH: f32 = 78.0;
const OVERVIEW_WAVEFORM_LAYER_CAPACITY: usize = 4;
const ZOOM_ICON_SIZE: f32 = 20.0;
const TOOLBAR_ACTION_ICON_SIZE: f32 = 16.0;
const BACK_TO_DASHBOARD_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 19-7-7 7-7"/><path d="M19 12H5"/></svg>"#;
const ZOOM_OUT_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="10.5" cy="10.5" r="7"/><path d="M7.5 10.5h6"/><path d="m16 16 5 5"/></svg>"#;
const ZOOM_IN_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="10.5" cy="10.5" r="7"/><path d="M7.5 10.5h6"/><path d="M10.5 7.5v6"/><path d="m16 16 5 5"/></svg>"#;
const COPY_SUPPORT_ID_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>"#;
const FEEDBACK_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a4 4 0 0 1-4 4H8l-5 3V7a4 4 0 0 1 4-4h10a4 4 0 0 1 4 4z"/></svg>"#;
const CAPABILITIES_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 7h16"/><path d="M4 12h16"/><path d="M4 17h16"/><circle cx="8" cy="7" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="17" r="2"/></svg>"#;
const HOTKEYS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="20" height="14" rx="2"/><path d="M6 9h.01"/><path d="M10 9h.01"/><path d="M14 9h.01"/><path d="M18 9h.01"/><path d="M8 13h8"/></svg>"#;
static BACK_TO_DASHBOARD_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(BACK_TO_DASHBOARD_ICON));
static ZOOM_OUT_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ZOOM_OUT_ICON));
static ZOOM_IN_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(ZOOM_IN_ICON));
static COPY_SUPPORT_ID_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(COPY_SUPPORT_ID_ICON));
static FEEDBACK_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(FEEDBACK_ICON));
static CAPABILITIES_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(CAPABILITIES_ICON));
static HOTKEYS_ICON_HANDLE: LazyLock<SvgHandle> =
    LazyLock::new(|| SvgHandle::from_memory(HOTKEYS_ICON));

struct OverviewWaveformPeaksDependency {
    peaks: Arc<[f64]>,
    color_token: String,
    height: f32,
    vertical_scale: f32,
    span_fraction: f32,
}

impl std::hash::Hash for OverviewWaveformPeaksDependency {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.peaks).hash(state);
        self.peaks.len().hash(state);
        self.color_token.hash(state);
        self.height.to_bits().hash(state);
        self.vertical_scale.to_bits().hash(state);
        self.span_fraction.to_bits().hash(state);
    }
}

pub(super) fn view_top_toolbar<'a>(
    state: &'a super::super::StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let zoom = row![
        zoom_svg_icon(ZOOM_OUT_ICON_HANDLE.clone()),
        slider(
            BABEL_ZOOM_SLIDER_MIN..=BABEL_ZOOM_SLIDER_MAX,
            babel_slider_from_zoom_factor(editor.zoom),
            |value| Message::ZoomChanged(zoom_factor_from_babel_slider(value)),
        )
        .step(0.001)
        .width(Length::Fixed(260.0))
        .style(zoom_slider_style),
        zoom_svg_icon(ZOOM_IN_ICON_HANDLE.clone()),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    let speed_options = babel_speed_labels();
    let speed = pick_list(
        speed_options,
        Some(format_speed(editor.speed)),
        Message::SpeedChanged,
    )
    .text_size(14)
    .padding([5, 10])
    .style(toolbar_select_style)
    .width(Length::Fixed(TOP_TOOLBAR_SPEED_WIDTH));

    let support_id = editor.support_id_button(state.support_id_copied);
    let support_id_button = toolbar_labeled_icon_button(
        COPY_SUPPORT_ID_ICON_HANDLE.clone(),
        support_id.button_label,
        Message::CopySupportId,
    );

    let mut toolbar = Row::new()
        .spacing(12)
        .align_y(iced::Alignment::Center)
        .push(toolbar_labeled_icon_button(
            BACK_TO_DASHBOARD_ICON_HANDLE.clone(),
            "Back",
            Message::BackToProjects,
        ))
        .push(toolbar_separator())
        .push(zoom)
        .push(toolbar_separator())
        .push(speed)
        .push(toolbar_separator())
        .push(view_timed(
            "overview-waveform",
            view_overview_waveform(
                state.overview_waveform_snapshot_for_render(project, editor),
                editor.playhead_seconds,
            ),
        ))
        .push(toolbar_separator())
        .push(
            tooltip(
                support_id_button,
                container(text(support_id.tooltip_content).size(12)).padding(6),
                tooltip::Position::FollowCursor,
            )
            .style(container::rounded_box),
        );

    let feedback_button = build_toolbar_feedback_button(
        state.feedback_panel.is_some(),
        state.feedback_panel.is_some(),
    );
    if feedback_button.visible {
        toolbar = toolbar.push(
            tooltip(
                toolbar_labeled_icon_button(
                    FEEDBACK_ICON_HANDLE.clone(),
                    feedback_button.label,
                    Message::OpenFeedbackPanel,
                ),
                container(text(feedback_button.tooltip_content).size(12)).padding(6),
                tooltip::Position::FollowCursor,
            )
            .style(container::rounded_box),
        );
    }

    if cfg!(debug_assertions) {
        toolbar = toolbar.push(toolbar_labeled_icon_button(
            CAPABILITIES_ICON_HANDLE.clone(),
            "Capabilities",
            Message::ToggleNativeCapabilityPanel,
        ));
    }

    let toolbar = toolbar.push(toolbar_labeled_icon_button(
        HOTKEYS_ICON_HANDLE.clone(),
        "Hotkeys",
        Message::ShowHotkeys,
    ));

    container(toolbar)
        .padding(10)
        .width(Fill)
        .height(Length::Fixed(TOP_TOOLBAR_HEIGHT))
        .style(editor_band_style)
        .into()
}

fn toolbar_separator<'a>() -> Element<'a, Message> {
    container(vertical_rule(1))
        .height(Length::Fixed(TOP_TOOLBAR_RULE_HEIGHT))
        .into()
}

fn zoom_svg_icon<'a>(icon: SvgHandle) -> Element<'a, Message> {
    let icon = Svg::new(icon)
        .width(Length::Fixed(ZOOM_ICON_SIZE))
        .height(Length::Fixed(ZOOM_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::from_rgb8(96, 96, 96)),
        });

    container(icon)
        .width(Length::Fixed(ZOOM_ICON_SIZE))
        .center_y(Length::Fixed(22.0))
        .into()
}

fn toolbar_labeled_icon_button<'a>(
    icon: SvgHandle,
    label: impl Into<String>,
    message: Message,
) -> Element<'a, Message> {
    let label = label.into();
    let icon = Svg::new(icon)
        .width(Length::Fixed(TOOLBAR_ACTION_ICON_SIZE))
        .height(Length::Fixed(TOOLBAR_ACTION_ICON_SIZE))
        .style(|_, _| svg::Style {
            color: Some(Color::from_rgb8(75, 85, 99)),
        });

    button(
        row![icon, text(label)]
            .spacing(8)
            .align_y(iced::Alignment::Center),
    )
    .style(toolbar_outline_button_style)
    .on_press(message)
    .padding([0, 8])
    .height(Length::Fixed(TOP_TOOLBAR_CONTROL_HEIGHT))
    .into()
}

fn toolbar_select_style(_theme: &Theme, status: PickListStatus) -> PickListStyle {
    let background = match status {
        PickListStatus::Hovered | PickListStatus::Opened => Color::from_rgb8(248, 250, 252),
        PickListStatus::Active => Color::WHITE,
    };

    PickListStyle {
        text_color: Color::from_rgb8(71, 85, 105),
        placeholder_color: Color::from_rgb8(100, 116, 139),
        handle_color: Color::from_rgb8(71, 85, 105),
        background: Background::Color(background),
        border: border::rounded(6)
            .width(1)
            .color(Color::from_rgb8(226, 232, 240)),
    }
}

fn toolbar_outline_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb8(248, 250, 252),
        button::Status::Pressed => Color::from_rgb8(241, 245, 249),
        button::Status::Disabled => Color::from_rgb8(248, 250, 252),
        button::Status::Active => Color::WHITE,
    };
    let text_color = match status {
        button::Status::Disabled => Color::from_rgb8(148, 163, 184),
        _ => Color::from_rgb8(71, 85, 105),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: border::rounded(6)
            .width(1)
            .color(Color::from_rgb8(226, 232, 240)),
        shadow: Default::default(),
    }
}

fn view_overview_waveform<'a>(
    snapshot: super::super::state::OverviewWaveformSnapshot,
    playhead_seconds: f64,
) -> Element<'a, Message> {
    let overview = snapshot.overview;
    let bounds = snapshot.bounds;
    let overview = responsive(move |size: Size| {
        let width = size.width.max(1.0);
        let mut layers: Vec<Element<'static, Message>> =
            Vec::with_capacity(OVERVIEW_WAVEFORM_LAYER_CAPACITY);
        if !overview.lane_waveform_models.is_empty() {
            layers.push(view_overview_peaks(
                Arc::clone(&overview.lane_waveform_models),
                bounds.duration_seconds(),
            ));
        }
        layers.push(view_overview_blocks_canvas(
            Arc::clone(&overview.blocks),
            overview.block_lane_rows,
        ));
        layers.push(view_overview_window_quad(
            overview.viewport_left_percent,
            overview.viewport_width_percent,
        ));
        layers.push(view_overview_playhead_canvas(overview_playhead_percent(
            playhead_seconds,
            bounds,
        )));
        let content: Element<'static, Message> =
            stack(layers).height(Length::Fixed(30.0)).width(Fill).into();

        view_horizontal_drag_capture(
            container(content)
                .width(Fill)
                .height(Length::Fixed(30.0))
                .into(),
            move |fraction, _extent| Message::OverviewPointerMoved { fraction, width },
            Message::OverviewPressed,
            Message::OverviewReleased,
            Some(Message::OverviewExited),
        )
    });

    container(overview)
        .width(Fill)
        .height(Length::Fixed(34.0))
        .into()
}

fn overview_playhead_percent(playhead_seconds: f64, bounds: TimeRange) -> f32 {
    let duration = bounds.duration_seconds().max(0.001);
    (((playhead_seconds - bounds.start_seconds) / duration) * 100.0).clamp(0.0, 100.0) as f32
}

fn view_overview_peaks(
    lane_waveform_models: Arc<[OverviewLaneWaveformView]>,
    bounds_duration_seconds: f64,
) -> Element<'static, Message> {
    let lane_height = (26.0 / lane_waveform_models.len().max(1) as f32).max(8.0);
    let mut lane_stack = Column::new()
        .spacing(0)
        .width(Fill)
        .height(Length::Fixed(26.0));
    for lane in lane_waveform_models.iter() {
        let peaks = lane
            .waveform_model
            .channels
            .first()
            .map(|channel| Arc::clone(&channel.peaks))
            .unwrap_or_else(|| Arc::<[f64]>::from(Vec::new()));
        lane_stack = lane_stack.push(
            container(view_lazy_overview_waveform_peaks(
                OverviewWaveformPeaksDependency {
                    peaks,
                    color_token: lane.color_token.clone(),
                    height: lane_height,
                    vertical_scale: 1.0,
                    span_fraction: overview_lane_span_fraction(
                        lane.waveform_model.visible_end_seconds
                            - lane.waveform_model.visible_start_seconds,
                        bounds_duration_seconds,
                    ),
                },
            ))
            .height(Length::Fixed(lane_height))
            .width(Fill),
        );
    }

    container(lane_stack).padding(2).width(Fill).into()
}

/// The overview x-axis spans the project bounds (segments), while each lane's
/// audio can be shorter or longer. Scaling the drawn peaks keeps loud moments
/// aligned with the playhead and the segment blocks.
fn overview_lane_span_fraction(audio_duration_seconds: f64, bounds_duration_seconds: f64) -> f32 {
    if !audio_duration_seconds.is_finite()
        || !bounds_duration_seconds.is_finite()
        || audio_duration_seconds <= 0.0
        || bounds_duration_seconds <= 0.0
    {
        return 1.0;
    }
    (audio_duration_seconds / bounds_duration_seconds) as f32
}

fn view_lazy_overview_waveform_peaks(
    dependency: OverviewWaveformPeaksDependency,
) -> Element<'static, Message> {
    lazy(dependency, |dependency| {
        view_wavesurfer_line_waveform(
            Arc::clone(&dependency.peaks),
            waveform_color(&dependency.color_token),
            dependency.height,
            dependency.vertical_scale,
            dependency.span_fraction,
        )
    })
    .into()
}

pub(super) fn view_diff_toolbar<'a>(
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let toolbar = editor.diff_toolbar(project);

    let mut controls = Row::new()
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .push(view_diff_toggle(toolbar.diff_toggle.checked))
        .push(text(toolbar.diff_toggle.label.clone()));

    if toolbar.show_reviewers_control {
        let reviewers_label = if toolbar.reviewers_checked {
            "Reviewers On"
        } else {
            "Reviewers Off"
        };
        controls =
            controls.push(button(text(reviewers_label)).on_press(Message::ToggleDiffReviewers));
    }

    if toolbar.unsaved_changes_badge.visible {
        controls = controls.push(text(toolbar.unsaved_changes_badge.text.clone()).size(12));
    }

    if toolbar.compare_select.visible && toolbar.diff_toggle.checked {
        controls = controls.push(vertical_rule(1));
        controls = controls.push(text(toolbar.compare_select.label.clone()).size(12));
        controls = controls.push(view_compare_select(
            toolbar.compare_select.options.clone(),
            toolbar.compare_select.value.clone(),
            toolbar.compare_select.disabled,
            toolbar.compare_select.loading_text.clone(),
        ));
    }

    let metrics = view_diff_metrics(toolbar.metrics);

    container(
        row![controls, Space::with_width(Fill), metrics]
            .spacing(12)
            .align_y(iced::Alignment::Center),
    )
    .padding(8)
    .width(Fill)
    .height(Length::Fixed(44.0))
    .style(editor_band_style)
    .into()
}

fn view_compare_select<'a>(
    options: Vec<DiffCompareOptionView>,
    selected_value: Option<String>,
    disabled: bool,
    loading_text: String,
) -> Element<'a, Message> {
    if disabled {
        return text(loading_text).size(12).into();
    }

    let selected = selected_value.and_then(|selected_value| {
        options
            .iter()
            .find(|option| option.value == selected_value)
            .cloned()
    });

    pick_list(options, selected, |option| {
        Message::DiffCompareActionChanged(option.value)
    })
    .width(Length::Fixed(180.0))
    .into()
}

fn view_diff_metrics(mut metrics: DiffToolbarMetricsView) -> Element<'static, Message> {
    if !metrics.visible {
        return Space::with_width(Length::Shrink).into();
    }

    let mut row = Row::new().spacing(10).align_y(iced::Alignment::Center);
    if let Some(avg_shift) = metrics.avg_shift {
        let avg_shift_text = text(format!("{}{}", avg_shift.label, avg_shift.value)).size(12);
        if let Some(timestamp_tooltip) = metrics.timestamp_tooltip.take() {
            row = row.push(
                tooltip(
                    avg_shift_text,
                    view_timestamp_metric_tooltip(timestamp_tooltip),
                    tooltip::Position::FollowCursor,
                )
                .style(container::rounded_box),
            );
        } else {
            row = row.push(avg_shift_text);
        }
    }
    if let Some(word_diff) = metrics.word_diff {
        row = row.push(text(format!("{}{}", word_diff.label, word_diff.value)).size(12));
    }
    if let Some(substitutions) = metrics.substitutions {
        row = row.push(text(format!("{} {}", substitutions.label, substitutions.value)).size(12));
    }
    if let Some(additions) = metrics.additions {
        row = row.push(text(format!("{} {}", additions.label, additions.value)).size(12));
    }
    if let Some(removals) = metrics.removals {
        row = row.push(text(format!("{} {}", removals.label, removals.value)).size(12));
    }
    if let Some(event) = metrics.event {
        row = row.push(view_annotation_metric(event));
    }
    if let Some(label_accuracy) = metrics.label_accuracy {
        row = row.push(view_annotation_metric(label_accuracy));
    }
    if let Some(intensity) = metrics.intensity {
        row = row.push(view_annotation_metric(intensity));
    }
    for item in metrics.legend {
        row = row.push(text(item.label).size(11));
    }

    row.into()
}

fn view_annotation_metric(metric: DiffAnnotationMetricView) -> Element<'static, Message> {
    let metric_text = text(format!("{} {}", metric.label, metric.value)).size(12);
    if let Some(metric_tooltip) = metric.tooltip {
        tooltip(
            metric_text,
            view_annotation_metric_tooltip(metric_tooltip),
            tooltip::Position::FollowCursor,
        )
        .style(container::rounded_box)
        .into()
    } else {
        metric_text.into()
    }
}

fn view_annotation_metric_tooltip(
    tooltip: DiffAnnotationMetricTooltipView,
) -> Element<'static, Message> {
    let mut content = Column::new()
        .spacing(4)
        .push(text(tooltip.title).size(12))
        .push(
            text(format!(
                "{} {}",
                tooltip.f1_score.label, tooltip.f1_score.value
            ))
            .size(11),
        );

    for metric in tooltip.secondary_metrics {
        content = content.push(text(format!("{} {}", metric.label, metric.value)).size(11));
    }
    content = content.push(text(tooltip.toggle_label).size(11));
    for description in tooltip.descriptions {
        content =
            content.push(text(format!("{}: {}", description.label, description.desc)).size(11));
    }

    container(content)
        .padding(8)
        .width(Length::Fixed(360.0))
        .into()
}

fn view_timestamp_metric_tooltip(
    tooltip: DiffTimestampMetricTooltipView,
) -> Element<'static, Message> {
    let mut content = Column::new()
        .spacing(4)
        .push(text(tooltip.title).size(12))
        .push(
            text(format!(
                "{} {}",
                tooltip.f1_score.label, tooltip.f1_score.value
            ))
            .size(11),
        );

    for metric in tooltip.secondary_metrics {
        content = content.push(text(format!("{} {}", metric.label, metric.value)).size(11));
    }
    content = content.push(text(tooltip.matched_segments_text).size(11));
    for bar in tooltip.tolerance_bars {
        content = content.push(text(format!("{} {}", bar.label, bar.value_label)).size(11));
    }
    content = content.push(text(tooltip.toggle_label).size(11));
    for description in tooltip.descriptions {
        content =
            content.push(text(format!("{}: {}", description.label, description.desc)).size(11));
    }

    container(content)
        .padding(8)
        .width(Length::Fixed(360.0))
        .into()
}

fn view_diff_toggle<'a>(checked: bool) -> Element<'a, Message> {
    let left_portion = if checked { 1 } else { 0 };
    let right_portion = if checked { 0 } else { 1 };
    let switch = container(
        row![
            Space::with_width(Length::FillPortion(left_portion)),
            container(text(""))
                .width(Length::Fixed(DIFF_SWITCH_KNOB_SIZE))
                .height(Length::Fixed(DIFF_SWITCH_KNOB_SIZE))
                .style(diff_switch_knob_style),
            Space::with_width(Length::FillPortion(right_portion)),
        ]
        .align_y(iced::Alignment::Center)
        .width(Length::Fixed(DIFF_SWITCH_WIDTH - 8.0)),
    )
    .padding(4)
    .width(Length::Fixed(DIFF_SWITCH_WIDTH))
    .height(Length::Fixed(DIFF_SWITCH_HEIGHT))
    .style(move |_| diff_switch_style(checked));

    button(switch)
        .style(outline_button_style)
        .on_press(Message::ToggleDiffView)
        .width(Length::Fixed(DIFF_SWITCH_WIDTH + 8.0))
        .into()
}
