use iced::widget::{Space, container, row, text};
use iced::{Color, Element, Fill, Length, Theme};
use studio_app::EditorSession;
use studio_core::Project;

use super::super::styles::{
    error_pill_style, status_bar_style, success_pill_style, warning_pill_style,
};
use super::super::{Message, StudioState};

const STATUS_BAR_HEIGHT: f32 = 32.0;
const MAX_STATUS_CHARACTERS: usize = 120;

pub(super) fn view_editor_status_bar<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let (save_label, save_style): (&str, fn(&Theme) -> container::Style) = if editor.is_saving {
        ("Saving changes", warning_pill_style)
    } else if editor.save_has_error {
        ("Save needs attention", error_pill_style)
    } else if editor.is_dirty {
        ("Unsaved changes", warning_pill_style)
    } else {
        ("All changes saved", success_pill_style)
    };

    let save_pill = container(text(save_label).size(11))
        .padding([3, 9])
        .style(save_style);

    let activity = concise_status(&state.status)
        .filter(|status| !is_redundant_status(status, save_label))
        .unwrap_or_else(|| "Autosave is on".to_string());
    let activity_color = if status_is_error(&activity) {
        Color::from_rgb8(185, 28, 28)
    } else {
        Color::from_rgb8(100, 116, 139)
    };

    let selected_position = editor
        .selected_segment_id
        .as_deref()
        .and_then(|selected_id| {
            project
                .segments
                .iter()
                .position(|segment| segment.id == selected_id)
                .map(|index| index + 1)
        });
    let selection_label = selected_position
        .map(|position| format!("Segment {position} of {}", project.segments.len()))
        .unwrap_or_else(|| format!("{} segments", project.segments.len()));

    container(
        row![
            save_pill,
            text(activity).size(11).color(activity_color),
            Space::with_width(Fill),
            text(selection_label)
                .size(11)
                .color(Color::from_rgb8(100, 116, 139)),
            text(format_playhead(editor.playhead_seconds))
                .size(11)
                .color(Color::from_rgb8(71, 85, 105)),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
    )
    .padding([4, 10])
    .width(Fill)
    .height(Length::Fixed(STATUS_BAR_HEIGHT))
    .style(status_bar_style)
    .into()
}

fn concise_status(status: &str) -> Option<String> {
    let status = status.trim();
    if status.is_empty() || status == "Loaded bundled Babel sample project." {
        return None;
    }

    let mut output = status
        .chars()
        .take(MAX_STATUS_CHARACTERS)
        .collect::<String>();
    if status.chars().count() > MAX_STATUS_CHARACTERS {
        output.push('…');
    }
    Some(output)
}

fn is_redundant_status(status: &str, save_label: &str) -> bool {
    let status = status.to_ascii_lowercase();
    match save_label {
        "Saving changes" => status.contains("saving progress"),
        "All changes saved" => status == "progress saved.",
        _ => false,
    }
}

fn status_is_error(status: &str) -> bool {
    let status = status.to_ascii_lowercase();
    status.contains("failed")
        || status.contains("error")
        || status.contains("blocked")
        || status.contains("unavailable")
}

fn format_playhead(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    let minutes = (seconds / 60.0).floor() as u64;
    let whole_seconds = seconds.floor() as u64 % 60;
    let centiseconds = ((seconds.fract() * 100.0).round() as u64).min(99);
    format!("{minutes:02}:{whole_seconds:02}.{centiseconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concise_status_hides_bootstrap_copy_and_truncates_long_messages() {
        assert_eq!(concise_status("Loaded bundled Babel sample project."), None);
        assert_eq!(
            concise_status("  Progress saved.  "),
            Some("Progress saved.".to_string())
        );

        let long = "x".repeat(MAX_STATUS_CHARACTERS + 8);
        let concise = concise_status(&long).expect("long status should stay visible");
        assert_eq!(concise.chars().count(), MAX_STATUS_CHARACTERS + 1);
        assert!(concise.ends_with('…'));
    }

    #[test]
    fn error_status_detection_covers_user_action_failures() {
        assert!(status_is_error("Save failed: offline"));
        assert!(status_is_error("Submit blocked: warnings remain"));
        assert!(!status_is_error("Progress saved."));
    }

    #[test]
    fn playhead_copy_is_stable_and_compact() {
        assert_eq!(format_playhead(0.0), "00:00.00");
        assert_eq!(format_playhead(65.42), "01:05.42");
    }
}
