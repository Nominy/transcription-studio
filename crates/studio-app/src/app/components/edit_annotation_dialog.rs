use iced::widget::{Space, button, column, container, pick_list, row, text, text_input};
use iced::{Color, Element, Fill, Font, Length, font};

use super::super::create_region::{
    create_region_intensity_options, create_region_primary_options,
    create_region_secondary_options, intensity_label,
};
use super::super::styles::{modal_surface_style, primary_button_style, secondary_button_style};
use super::super::{EditAnnotationDialog, Message};

const EDIT_ANNOTATION_DIALOG_WIDTH: f32 = 460.0;
const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_edit_annotation_dialog<'a>(
    dialog: &'a EditAnnotationDialog,
) -> Element<'a, Message> {
    let primary_options = create_region_primary_options();
    let selected_primary = non_empty_selected(&dialog.primary_selection);
    let secondary_options = edit_secondary_options(&dialog.primary_selection);
    let selected_secondary = non_empty_selected(&dialog.secondary_selection);
    let intensity_options = create_region_intensity_options();
    let selected_intensity = intensity_label(dialog.intensity);

    let mut fields = column![text(dialog.title()).size(18).font(SEMIBOLD)].spacing(12);

    if dialog.uses_hierarchical_labels {
        fields = fields.push(
            column![
                text("Primary Category *").size(12),
                pick_list(
                    primary_options,
                    selected_primary,
                    Message::EditAnnotationPrimaryCategoryChanged,
                )
                .placeholder("Select primary category")
                .on_open(Message::EditAnnotationSelectOpenChanged(true))
                .on_close(Message::EditAnnotationSelectOpenChanged(false))
                .width(Length::Fixed(260.0)),
            ]
            .spacing(4),
        );
    } else {
        fields = fields
            .push(column![text("Category").size(12), text("Background Noise").size(13)].spacing(4));
    }

    if dialog.uses_hierarchical_labels
        && !dialog.primary_selection.is_empty()
        && dialog.primary_selection != "other"
    {
        fields = fields.push(
            column![
                text("Secondary Category *").size(12),
                pick_list(
                    secondary_options,
                    selected_secondary,
                    Message::EditAnnotationSecondaryCategoryChanged,
                )
                .placeholder("Select secondary category")
                .on_open(Message::EditAnnotationSelectOpenChanged(true))
                .on_close(Message::EditAnnotationSelectOpenChanged(false))
                .width(Length::Fixed(260.0)),
            ]
            .spacing(4),
        );
    }

    if dialog.uses_hierarchical_labels
        && (dialog.primary_selection == "other" || dialog.secondary_selection == "other")
    {
        fields = fields.push(
            column![
                text("Description *").size(12),
                text_input("Describe the noise...", &dialog.custom_description)
                    .on_input(Message::EditAnnotationCustomDescriptionChanged)
                    .padding(10)
                    .size(13)
                    .width(Fill),
            ]
            .spacing(4),
        );
    }

    fields = fields.push(
        column![
            text("Intensity *").size(12),
            pick_list(
                intensity_options,
                selected_intensity,
                Message::EditAnnotationIntensityChanged,
            )
            .placeholder("Select intensity")
            .on_open(Message::EditAnnotationSelectOpenChanged(true))
            .on_close(Message::EditAnnotationSelectOpenChanged(false))
            .width(Length::Fixed(260.0)),
        ]
        .spacing(4),
    );

    if let Some(error) = &dialog.error {
        fields = fields.push(
            text(error.clone())
                .size(11)
                .color(Color::from_rgb8(185, 28, 28)),
        );
    }

    fields = fields.push(
        row![
            Space::with_width(Fill),
            button(text("Cancel").size(13))
                .style(secondary_button_style)
                .on_press(Message::CancelEditAnnotation),
            button(text("Save changes").size(13))
                .style(primary_button_style)
                .on_press(Message::SubmitEditAnnotation),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center),
    );

    container(fields)
        .padding(22)
        .width(Length::Fixed(EDIT_ANNOTATION_DIALOG_WIDTH))
        .style(modal_surface_style)
        .into()
}

fn edit_secondary_options(primary: &str) -> Vec<String> {
    let mut options = create_region_secondary_options(primary);
    if !primary.is_empty() && primary != "other" && !options.iter().any(|option| option == "other")
    {
        options.push("other".to_string());
    }
    options
}

fn non_empty_selected(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}
