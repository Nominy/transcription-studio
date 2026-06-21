use iced::widget::{column, container, row, scrollable, text, Column, Row};
use iced::{Element, Fill, Length, Theme};
use studio_app::{load_sample_project, timeline_lane_rows};
use studio_core::Project;

pub fn main() -> iced::Result {
    iced::application("Transcription Studio", update, view)
        .theme(|_| Theme::Light)
        .run()
}

#[derive(Debug, Clone)]
enum Message {}

struct StudioState {
    project: Result<Project, String>,
}

impl Default for StudioState {
    fn default() -> Self {
        Self {
            project: load_sample_project().map_err(|error| error.to_string()),
        }
    }
}

fn update(_state: &mut StudioState, _message: Message) {}

fn view(state: &StudioState) -> Element<'_, Message> {
    match &state.project {
        Ok(project) => view_project(project),
        Err(error) => container(
            column![
                text("Transcription Studio").size(28),
                text("Could not load the bundled Babel sample task."),
                text(error)
            ]
            .spacing(10),
        )
        .padding(24)
        .into(),
    }
}

fn view_project(project: &Project) -> Element<'_, Message> {
    let header = row![
        text("Transcription Studio").size(26),
        text(format!("Project: {}", project.name)),
        text(format!("Segments: {}", project.segments.len())),
    ]
    .spacing(24);

    let transport = container(row![
        text("Play/Pause"),
        text("00:00.000"),
        text("Speed 1.00x"),
        text("Zoom 100%"),
    ]
    .spacing(18))
    .padding(12)
    .width(Fill);

    let mut lanes = Column::new().spacing(8);
    for lane_row in timeline_lane_rows(project) {
        let mut segments = Row::new().spacing(8);
        for segment in lane_row.segments {
            segments = segments.push(
                container(text(format!(
                    "{:.2}-{:.2}  {}",
                    segment.start_seconds, segment.end_seconds, segment.text
                )))
                .padding(8)
                .width(Length::Shrink),
            );
        }

        lanes = lanes.push(
            row![
                container(text(&lane_row.lane.label)).width(160),
                scrollable(segments).width(Fill)
            ]
            .spacing(12),
        );
    }

    let mut rows = Column::new().spacing(6);
    rows = rows.push(row![
        text("Speaker").width(120),
        text("Start").width(80),
        text("End").width(80),
        text("Text").width(Fill),
    ]);

    for segment in &project.segments {
        rows = rows.push(row![
            text(&segment.speaker_id).width(120),
            text(format!("{:.2}", segment.start_seconds)).width(80),
            text(format!("{:.2}", segment.end_seconds)).width(80),
            text(&segment.text).width(Fill),
        ]);
    }

    container(
        column![
            header,
            transport,
            text("Timeline").size(18),
            lanes,
            text("Transcript Rows").size(18),
            scrollable(rows).height(Fill),
        ]
        .spacing(14),
    )
    .padding(18)
    .into()
}
