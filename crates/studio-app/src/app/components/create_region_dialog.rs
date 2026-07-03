use iced::widget::{Space, button, column, container, pick_list, row, text, text_input};
use iced::{Color, Element, Fill, Length};
use studio_core::Project;

use super::super::create_region::{
    CreateRegionMode, create_region_intensity_options, create_region_primary_options,
    create_region_secondary_options, create_region_track_options, intensity_label,
    selected_create_region_track_option,
};
use super::super::styles::subtle_panel_style;
use super::super::{CreateRegionDialog, Message};

pub(super) fn view_create_region_dialog<'a>(
    project: &'a Project,
    dialog: &'a CreateRegionDialog,
) -> Element<'a, Message> {
    view_legacy_create_region_dialog(project, dialog)
}

pub(super) fn is_attached_transcription_card(dialog: &CreateRegionDialog) -> bool {
    dialog.mode == CreateRegionMode::Transcription && dialog.created_segment_id.is_some()
}

fn view_legacy_create_region_dialog<'a>(
    project: &'a Project,
    dialog: &'a CreateRegionDialog,
) -> Element<'a, Message> {
    let track_options = create_region_track_options(project);
    let selected_track =
        selected_create_region_track_option(&track_options, &dialog.selected_lane_id);

    let start_input = view_time_input(
        "Start Time (mm:ss.xx)",
        &dialog.start_draft,
        dialog.start_error.as_deref(),
        Message::CreateRegionStartChanged,
        Message::CommitCreateRegionStart,
    );
    let end_input = view_time_input(
        "End Time (mm:ss.xx)",
        &dialog.end_draft,
        dialog.end_error.as_deref(),
        Message::CreateRegionEndChanged,
        Message::CommitCreateRegionEnd,
    );
    let title = match dialog.mode {
        CreateRegionMode::Transcription => "Add New Region",
        CreateRegionMode::Annotation => "Add New Annotation",
    };

    container(
        column![
            text(title).size(16),
            column![
                text("Speaker").size(12),
                pick_list(track_options, selected_track, |option| {
                    Message::CreateRegionTrackChanged(option.lane_id)
                },)
                .on_open(Message::CreateRegionSelectOpenChanged(true))
                .on_close(Message::CreateRegionSelectOpenChanged(false))
                .width(Length::Fixed(220.0)),
            ]
            .spacing(4),
            row![start_input, end_input].spacing(12),
            view_create_region_body(dialog),
            row![
                Space::with_width(Fill),
                button(text("Cancel")).on_press(Message::CancelCreateRegion),
                button(text("Create")).on_press_maybe(
                    dialog
                        .is_create_enabled()
                        .then_some(Message::SubmitCreateRegion)
                ),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(8),
    )
    .padding(12)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}

fn view_create_region_body<'a>(dialog: &'a CreateRegionDialog) -> Element<'a, Message> {
    match dialog.mode {
        CreateRegionMode::Transcription => column![
            text("Text").size(12),
            text_input("Enter text for this region...", &dialog.text)
                .on_input(Message::CreateRegionTextChanged)
                .padding(6)
                .width(Fill),
        ]
        .spacing(4)
        .into(),
        CreateRegionMode::Annotation => view_annotation_fields(dialog),
    }
}

fn view_annotation_fields<'a>(dialog: &'a CreateRegionDialog) -> Element<'a, Message> {
    if !dialog.uses_hierarchical_labels {
        return column![
            column![text("Category").size(12), text("Background Noise").size(13),].spacing(4),
            view_intensity_picker(dialog.intensity, Message::CreateRegionIntensityChanged,),
        ]
        .spacing(6)
        .into();
    }

    let primary_options = create_region_primary_options();
    let selected_primary = non_empty_selected(&dialog.primary_selection);
    let secondary_options = create_region_secondary_options(&dialog.primary_selection);
    let selected_secondary = non_empty_selected(&dialog.secondary_selection);

    let mut fields = column![
        column![
            text("Primary Category *").size(12),
            pick_list(
                primary_options,
                selected_primary,
                Message::CreateRegionPrimaryCategoryChanged
            )
            .placeholder("Select primary category")
            .on_open(Message::CreateRegionSelectOpenChanged(true))
            .on_close(Message::CreateRegionSelectOpenChanged(false))
            .width(Length::Fixed(260.0)),
        ]
        .spacing(4),
        column![
            text("Secondary Category *").size(12),
            pick_list(
                secondary_options,
                selected_secondary,
                Message::CreateRegionSecondaryCategoryChanged,
            )
            .placeholder("Select secondary category")
            .on_open(Message::CreateRegionSelectOpenChanged(true))
            .on_close(Message::CreateRegionSelectOpenChanged(false))
            .width(Length::Fixed(260.0)),
        ]
        .spacing(4),
        view_intensity_picker(dialog.intensity, Message::CreateRegionIntensityChanged),
    ]
    .spacing(6);

    if dialog.primary_selection == "other" || dialog.secondary_selection == "other" {
        fields = fields.push(
            column![
                text("Other").size(12),
                text_input("Describe the noise...", &dialog.custom_description)
                    .on_input(Message::CreateRegionCustomDescriptionChanged)
                    .padding(6)
                    .width(Fill),
            ]
            .spacing(4),
        );
    }

    fields.into()
}

fn view_intensity_picker<'a>(
    intensity: Option<u8>,
    on_change: fn(String) -> Message,
) -> Element<'a, Message> {
    let intensity_options = create_region_intensity_options();
    let selected_intensity = intensity_label(intensity);
    column![
        text("Intensity *").size(12),
        pick_list(intensity_options, selected_intensity, on_change)
            .placeholder("Select intensity")
            .on_open(Message::CreateRegionSelectOpenChanged(true))
            .on_close(Message::CreateRegionSelectOpenChanged(false))
            .width(Length::Fixed(260.0)),
    ]
    .spacing(4)
    .into()
}

fn non_empty_selected(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn view_time_input<'a>(
    label: &'static str,
    value: &'a str,
    error: Option<&'a str>,
    on_input: fn(String) -> Message,
    on_submit: Message,
) -> Element<'a, Message> {
    let mut content = column![
        text(label).size(12),
        text_input("mm:ss.xx", value)
            .on_input(on_input)
            .on_submit(on_submit)
            .padding(6)
            .width(Length::Fixed(140.0)),
    ]
    .spacing(4);

    if let Some(error) = error {
        content = content.push(text(error).size(11).color(Color::from_rgb8(185, 28, 28)));
    }

    content.into()
}
