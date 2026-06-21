use std::{collections::BTreeMap, fs, path::Path};

use iced::widget::{
    Column, Row, button, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Element, Fill, Length, Theme};
use studio_app::{StudioWorkflow, load_sample_project, timeline_lane_rows, transport_status};
use studio_core::{Project, Segment};

const DEFAULT_IMPORT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/sample-task.json");
const DEFAULT_LOCAL_PROJECT_PATH: &str = ".studio-dev/session";
const DEFAULT_EXPORT_PATH: &str = ".studio-dev/export/babel-task.json";

pub fn main() -> iced::Result {
    iced::application("Transcription Studio", update, view)
        .theme(|_| Theme::Light)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    ImportPathChanged(String),
    LocalProjectPathChanged(String),
    ExportPathChanged(String),
    ImportBabelJson,
    LoadLocalProject,
    SaveLocalProject,
    ExportBabelJson,
    Undo,
    Redo,
    SegmentStartChanged(String, String),
    SegmentEndChanged(String, String),
    SegmentSplitChanged(String, String),
    SegmentTextChanged(String, String),
    SegmentTrackChanged(String, String),
    ApplySegment(String),
    SplitSegment(String),
    MergeSegment(String),
    DeleteSegment(String),
    CreateSegmentAfter(String),
}

#[derive(Debug, Clone)]
struct SegmentDraft {
    start_seconds: String,
    end_seconds: String,
    split_seconds: String,
    text: String,
}

impl SegmentDraft {
    fn from_segment(segment: &Segment) -> Self {
        let split_seconds = (segment.start_seconds + segment.end_seconds) / 2.0;
        Self {
            start_seconds: format_seconds_for_edit(segment.start_seconds),
            end_seconds: format_seconds_for_edit(segment.end_seconds),
            split_seconds: format_seconds_for_edit(split_seconds),
            text: segment.text.clone(),
        }
    }
}

struct StudioState {
    workflow: Result<StudioWorkflow, String>,
    segment_drafts: BTreeMap<String, SegmentDraft>,
    import_path: String,
    local_project_path: String,
    export_path: String,
    status: String,
}

impl Default for StudioState {
    fn default() -> Self {
        let workflow = load_sample_project()
            .map(StudioWorkflow::from_project)
            .map_err(|error| error.to_string());
        let status = match &workflow {
            Ok(_) => "Loaded bundled sample project.".to_string(),
            Err(error) => format!("Could not load bundled sample project: {error}"),
        };
        let mut state = Self {
            workflow,
            segment_drafts: BTreeMap::new(),
            import_path: DEFAULT_IMPORT_PATH.to_string(),
            local_project_path: DEFAULT_LOCAL_PROJECT_PATH.to_string(),
            export_path: DEFAULT_EXPORT_PATH.to_string(),
            status,
        };
        state.reset_segment_drafts();
        state
    }
}

impl StudioState {
    fn project(&self) -> Option<&Project> {
        self.workflow.as_ref().ok().map(StudioWorkflow::project)
    }

    fn reset_segment_drafts(&mut self) {
        self.segment_drafts.clear();
        if let Some(project) = self.project() {
            self.segment_drafts = project
                .segments
                .iter()
                .map(|segment| (segment.id.clone(), SegmentDraft::from_segment(segment)))
                .collect();
        }
    }

    fn sync_segment_drafts_preserving_existing(&mut self) {
        let Some(project) = self.project() else {
            self.segment_drafts.clear();
            return;
        };
        let current_segments: Vec<_> = project
            .segments
            .iter()
            .map(|segment| {
                (
                    segment.id.clone(),
                    SegmentDraft::from_segment(segment),
                )
            })
            .collect();
        for (segment_id, draft) in &current_segments {
            if !self.segment_drafts.contains_key(segment_id) {
                self.segment_drafts
                    .insert(segment_id.clone(), draft.clone());
            }
        }
        self.segment_drafts.retain(|segment_id, _| {
            current_segments
                .iter()
                .any(|(current_id, _)| current_id == segment_id)
        });
    }

    fn refresh_segment_draft(&mut self, segment_id: &str) {
        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.segment_drafts.remove(segment_id);
            return;
        };
        self.segment_drafts
            .insert(segment_id.to_string(), SegmentDraft::from_segment(&segment));
    }

    fn replace_workflow(&mut self, workflow: StudioWorkflow, status: String) {
        self.workflow = Ok(workflow);
        self.status = status;
        self.reset_segment_drafts();
    }

    fn with_workflow_mut(
        &mut self,
        action_name: &str,
        action: impl FnOnce(&mut StudioWorkflow) -> Result<String, String>,
    ) -> bool {
        match &mut self.workflow {
            Ok(workflow) => match action(workflow) {
                Ok(status) => {
                    self.status = status;
                    self.sync_segment_drafts_preserving_existing();
                    true
                }
                Err(error) => {
                    self.status = format!("{action_name} failed: {error}");
                    false
                }
            },
            Err(error) => {
                self.status = format!("{action_name} unavailable: {error}");
                false
            }
        }
    }

    fn import_babel_json(&mut self) {
        match fs::read_to_string(&self.import_path)
            .map_err(|error| format!("failed to read `{}`: {error}", self.import_path))
            .and_then(|source| {
                StudioWorkflow::from_babel_json(&source).map_err(|error| error.to_string())
            }) {
            Ok(workflow) => {
                self.replace_workflow(workflow, format!("Imported `{}`.", self.import_path));
            }
            Err(error) => {
                self.status = format!("Import failed: {error}");
            }
        }
    }

    fn load_local_project(&mut self) {
        match StudioWorkflow::load_local_project(&self.local_project_path) {
            Ok(workflow) => {
                self.replace_workflow(
                    workflow,
                    format!("Loaded Studio project `{}`.", self.local_project_path),
                );
            }
            Err(error) => {
                self.status = format!("Load failed: {error}");
            }
        }
    }

    fn save_local_project(&mut self) {
        let path = self.local_project_path.clone();
        self.with_workflow_mut("Save", |workflow| {
            workflow
                .save_local_project(&path)
                .map_err(|error| error.to_string())?;
            Ok(format!("Saved Studio project `{path}`."))
        });
    }

    fn export_babel_json(&mut self) {
        let path = self.export_path.clone();
        self.with_workflow_mut("Export", |workflow| {
            let json = workflow
                .export_babel_json()
                .map_err(|error| error.to_string())?;
            write_text_file(&path, &json)?;
            Ok(format!("Exported Babel JSON `{path}`."))
        });
    }

    fn undo(&mut self) {
        if self.with_workflow_mut("Undo", |workflow| {
            workflow.undo().map_err(|error| error.to_string())?;
            Ok("Undid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
        }
    }

    fn redo(&mut self) {
        if self.with_workflow_mut("Redo", |workflow| {
            workflow.redo().map_err(|error| error.to_string())?;
            Ok("Redid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
        }
    }

    fn apply_segment(&mut self, segment_id: &str) {
        let Some(draft) = self.segment_drafts.get(segment_id).cloned() else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return;
        };

        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };

        let start_seconds = match parse_seconds("Start", &draft.start_seconds) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        let end_seconds = match parse_seconds("End", &draft.end_seconds) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if start_seconds >= end_seconds {
            self.status = "Start must be before end.".to_string();
            return;
        }

        let timing_changed =
            segment.start_seconds != start_seconds || segment.end_seconds != end_seconds;
        let text_changed = segment.text != draft.text;
        if !timing_changed && !text_changed {
            self.status = format!("No changes to apply for `{segment_id}`.");
            return;
        }

        let text = draft.text;
        let segment_id = segment_id.to_string();
        let applied = self.with_workflow_mut("Apply", |workflow| {
            workflow
                .apply_segment_edit(&segment_id, start_seconds, end_seconds, text)
                .map_err(|error| error.to_string())?;
            Ok(format!("Applied edits to `{segment_id}`."))
        });
        if applied {
            self.refresh_segment_draft(&segment_id);
        }
    }

    fn split_segment(&mut self, segment_id: &str) {
        let Some(segment) = self
            .project()
            .and_then(|project| project.segment(segment_id))
            .cloned()
        else {
            self.status = format!("Segment `{segment_id}` was not found.");
            return;
        };
        let Some(draft) = self.segment_drafts.get(segment_id) else {
            self.status = format!("Segment `{segment_id}` has no editable draft.");
            return;
        };
        let split_at = match parse_seconds("Split", &draft.split_seconds) {
            Ok(value) => value,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if split_at <= segment.start_seconds || split_at >= segment.end_seconds {
            self.status = "Split must be inside the segment.".to_string();
            return;
        }
        let segment_id = segment_id.to_string();
        let split_segment_id = segment_id.clone();
        if self.with_workflow_mut("Split", |workflow| {
            let new_segment_id = workflow
                .split_segment(&segment_id, split_at)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Split `{segment_id}` at {split_at:.3}s into `{new_segment_id}`."
            ))
        }) {
            self.refresh_segment_draft(&split_segment_id);
        }
    }

    fn merge_segment(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        if self.with_workflow_mut("Merge", |workflow| {
            workflow
                .merge_with_next(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Merged `{segment_id}` with the next segment."))
        }) {
            self.refresh_segment_draft(&segment_id);
        }
    }

    fn delete_segment(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        if self.with_workflow_mut("Delete", |workflow| {
            workflow
                .delete_segment(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Deleted `{segment_id}`."))
        }) {
            self.segment_drafts.remove(&segment_id);
        }
    }

    fn create_segment_after(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        self.with_workflow_mut("Create", |workflow| {
            let new_segment_id = workflow
                .create_segment_after(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Created `{new_segment_id}` after `{segment_id}`."))
        });
    }

    fn change_segment_track(&mut self, segment_id: &str, lane_id: &str) {
        let segment_id = segment_id.to_string();
        let lane_id = lane_id.to_string();
        if self.with_workflow_mut("Track change", |workflow| {
            workflow
                .change_segment_track(&segment_id, &lane_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Moved `{segment_id}` to `{lane_id}`."))
        }) {
            self.refresh_segment_draft(&segment_id);
        }
    }
}

fn update(state: &mut StudioState, message: Message) {
    match message {
        Message::ImportPathChanged(path) => {
            state.import_path = path;
        }
        Message::LocalProjectPathChanged(path) => {
            state.local_project_path = path;
        }
        Message::ExportPathChanged(path) => {
            state.export_path = path;
        }
        Message::ImportBabelJson => state.import_babel_json(),
        Message::LoadLocalProject => state.load_local_project(),
        Message::SaveLocalProject => state.save_local_project(),
        Message::ExportBabelJson => state.export_babel_json(),
        Message::Undo => state.undo(),
        Message::Redo => state.redo(),
        Message::SegmentStartChanged(segment_id, value) => {
            if let Some(draft) = state.segment_drafts.get_mut(&segment_id) {
                draft.start_seconds = value;
            }
        }
        Message::SegmentEndChanged(segment_id, value) => {
            if let Some(draft) = state.segment_drafts.get_mut(&segment_id) {
                draft.end_seconds = value;
            }
        }
        Message::SegmentSplitChanged(segment_id, value) => {
            if let Some(draft) = state.segment_drafts.get_mut(&segment_id) {
                draft.split_seconds = value;
            }
        }
        Message::SegmentTextChanged(segment_id, value) => {
            if let Some(draft) = state.segment_drafts.get_mut(&segment_id) {
                draft.text = value;
            }
        }
        Message::SegmentTrackChanged(segment_id, lane_id) => {
            state.change_segment_track(&segment_id, &lane_id)
        }
        Message::ApplySegment(segment_id) => state.apply_segment(&segment_id),
        Message::SplitSegment(segment_id) => state.split_segment(&segment_id),
        Message::MergeSegment(segment_id) => state.merge_segment(&segment_id),
        Message::DeleteSegment(segment_id) => state.delete_segment(&segment_id),
        Message::CreateSegmentAfter(segment_id) => state.create_segment_after(&segment_id),
    }
}

fn view(state: &StudioState) -> Element<'_, Message> {
    let mut content = Column::new()
        .spacing(12)
        .push(row![text("Transcription Studio").size(26)].spacing(24))
        .push(view_toolbar(state))
        .push(text(&state.status).size(14));

    content = match &state.workflow {
        Ok(workflow) => content.push(view_project(workflow.project(), &state.segment_drafts)),
        Err(error) => content.push(
            container(
                column![
                    text("No project is currently loaded.").size(18),
                    text(error).size(14),
                ]
                .spacing(8),
            )
            .padding(12)
            .width(Fill),
        ),
    };

    container(content).padding(18).into()
}

fn view_toolbar(state: &StudioState) -> Element<'_, Message> {
    let import_row = row![
        text("Babel").width(Length::Fixed(56.0)),
        text_input(DEFAULT_IMPORT_PATH, &state.import_path)
            .on_input(Message::ImportPathChanged)
            .on_submit(Message::ImportBabelJson)
            .padding(6)
            .width(Fill),
        button(text("Import")).on_press(Message::ImportBabelJson),
    ]
    .spacing(8);

    let project_row = row![
        text("Studio").width(Length::Fixed(56.0)),
        text_input(DEFAULT_LOCAL_PROJECT_PATH, &state.local_project_path)
            .on_input(Message::LocalProjectPathChanged)
            .on_submit(Message::LoadLocalProject)
            .padding(6)
            .width(Fill),
        button(text("Load")).on_press(Message::LoadLocalProject),
        button(text("Save")).on_press(Message::SaveLocalProject),
    ]
    .spacing(8);

    let export_row = row![
        text("Export").width(Length::Fixed(56.0)),
        text_input(DEFAULT_EXPORT_PATH, &state.export_path)
            .on_input(Message::ExportPathChanged)
            .on_submit(Message::ExportBabelJson)
            .padding(6)
            .width(Fill),
        button(text("Export")).on_press(Message::ExportBabelJson),
        button(text("Undo")).on_press(Message::Undo),
        button(text("Redo")).on_press(Message::Redo),
    ]
    .spacing(8);

    container(column![import_row, project_row, export_row].spacing(8))
        .padding(10)
        .width(Fill)
        .into()
}

fn view_project<'a>(
    project: &'a Project,
    segment_drafts: &'a BTreeMap<String, SegmentDraft>,
) -> Element<'a, Message> {
    let transport = container(
        row![
            text("Play/Pause"),
            text("00:00.000"),
            text("Speed 1.00x"),
            text("Zoom 100%"),
            text(transport_status(project)),
        ]
        .spacing(18),
    )
    .padding(10)
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
                container(text(&lane_row.lane.label)).width(Length::Fixed(150.0)),
                scrollable(segments).width(Fill),
            ]
            .spacing(10),
        );
    }

    let mut transcript_rows = Column::new().spacing(6);
    transcript_rows = transcript_rows.push(row![
        text("Speaker").width(Length::Fixed(112.0)),
        text("Track").width(Length::Fixed(156.0)),
        text("Start").width(Length::Fixed(84.0)),
        text("End").width(Length::Fixed(84.0)),
        text("Split").width(Length::Fixed(84.0)),
        text("Text").width(Fill),
        text("Actions").width(Length::Fixed(360.0)),
    ]);

    for segment in &project.segments {
        if let Some(draft) = segment_drafts.get(&segment.id) {
            transcript_rows = transcript_rows.push(view_segment_row(project, segment, draft));
        }
    }

    container(
        column![
            transport,
            text("Timeline").size(18),
            lanes,
            text("Transcript Rows").size(18),
            scrollable(transcript_rows).height(Fill),
        ]
        .spacing(12),
    )
    .into()
}

fn view_segment_row<'a>(
    project: &'a Project,
    segment: &'a Segment,
    draft: &'a SegmentDraft,
) -> Element<'a, Message> {
    let segment_id = segment.id.clone();
    let speaker = speaker_label(project, &segment.speaker_id);
    let track_options: Vec<String> = project.lanes.iter().map(|lane| lane.id.clone()).collect();
    let selected_track = Some(segment.lane_id.clone());
    let track_segment_id = segment_id.clone();
    let track_picker = pick_list(track_options, selected_track, move |lane_id| {
        Message::SegmentTrackChanged(track_segment_id.clone(), lane_id)
    })
    .width(Length::Fixed(156.0));

    let start_id = segment_id.clone();
    let start_input = text_input("0.000", &draft.start_seconds)
        .on_input(move |value| Message::SegmentStartChanged(start_id.clone(), value))
        .on_submit(Message::ApplySegment(segment_id.clone()))
        .padding(6)
        .width(Length::Fixed(84.0));

    let segment_id = segment.id.clone();
    let end_id = segment_id.clone();
    let end_input = text_input("0.000", &draft.end_seconds)
        .on_input(move |value| Message::SegmentEndChanged(end_id.clone(), value))
        .on_submit(Message::ApplySegment(segment_id.clone()))
        .padding(6)
        .width(Length::Fixed(84.0));

    let segment_id = segment.id.clone();
    let text_id = segment_id.clone();
    let transcript_text_input = text_input("Transcript text", &draft.text)
        .on_input(move |value| Message::SegmentTextChanged(text_id.clone(), value))
        .on_submit(Message::ApplySegment(segment_id.clone()))
        .padding(6)
        .width(Fill);

    let segment_id = segment.id.clone();
    let split_id = segment_id.clone();
    let split_input = text_input("0.000", &draft.split_seconds)
        .on_input(move |value| Message::SegmentSplitChanged(split_id.clone(), value))
        .on_submit(Message::SplitSegment(segment_id.clone()))
        .padding(6)
        .width(Length::Fixed(84.0));

    let actions = row![
        action_button("Apply", Message::ApplySegment(segment.id.clone())),
        action_button("Split", Message::SplitSegment(segment.id.clone())),
        action_button("Merge", Message::MergeSegment(segment.id.clone())),
        action_button("Delete", Message::DeleteSegment(segment.id.clone())),
        action_button("After", Message::CreateSegmentAfter(segment.id.clone())),
    ]
    .spacing(4)
    .width(Length::Fixed(360.0));

    row![
        text(speaker).width(Length::Fixed(112.0)),
        track_picker,
        start_input,
        end_input,
        split_input,
        transcript_text_input,
        actions,
    ]
    .spacing(6)
    .into()
}

fn action_button(label: &'static str, message: Message) -> Element<'static, Message> {
    button(text(label).size(14))
        .on_press(message)
        .padding(6)
        .into()
}

fn parse_seconds(label: &str, value: &str) -> Result<f64, String> {
    let parsed = value
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label} must be a number."))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(format!("{label} must be finite."))
    }
}

fn format_seconds_for_edit(value: f64) -> String {
    value.to_string()
}

fn speaker_label(project: &Project, speaker_id: &str) -> String {
    project
        .speakers
        .iter()
        .find(|speaker| speaker.id == speaker_id)
        .map(|speaker| speaker.label.clone())
        .unwrap_or_else(|| speaker_id.to_string())
}

fn write_text_file(path: &str, contents: &str) -> Result<(), String> {
    let path = Path::new(path);
    if path.as_os_str().is_empty() {
        return Err("path is empty".to_string());
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create `{}`: {error}", parent.display()))?;
    }

    fs::write(path, contents)
        .map_err(|error| format!("failed to write `{}`: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use studio_app::StudioWorkflow;

    use super::{DEFAULT_IMPORT_PATH, StudioState};

    #[test]
    fn default_import_path_points_to_existing_fixture() {
        assert!(Path::new(DEFAULT_IMPORT_PATH).exists());
    }

    #[test]
    fn applying_text_only_preserves_precise_timing() {
        let mut state = StudioState::default();
        let project = state.workflow.as_mut().unwrap().project();
        assert!(project.segment("annotation-1").is_some());
        let mut project = project.clone();
        let segment = project.segment_mut("annotation-1").unwrap();
        segment.start_seconds = 1.23456;
        segment.end_seconds = 3.45678;
        state.replace_workflow(
            StudioWorkflow::from_project(project),
            "loaded precise timing fixture".to_string(),
        );

        state
            .segment_drafts
            .get_mut("annotation-1")
            .unwrap()
            .text = "Only text changed.".to_string();
        state.apply_segment("annotation-1");

        let segment = state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .unwrap();
        assert_eq!(segment.start_seconds, 1.23456);
        assert_eq!(segment.end_seconds, 3.45678);
        assert_eq!(segment.text, "Only text changed.");
    }

    #[test]
    fn applying_text_and_timing_undoes_as_one_step() {
        let mut state = StudioState::default();
        let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
        draft.start_seconds = "1.500".to_string();
        draft.end_seconds = "3.750".to_string();
        draft.text = "Updated together.".to_string();

        state.apply_segment("annotation-1");
        state.undo();

        let segment = state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .unwrap();
        assert_eq!(segment.start_seconds, 1.2);
        assert_eq!(segment.end_seconds, 3.4);
        assert_eq!(segment.text, "Здравствуйте.");
    }

    #[test]
    fn unrelated_actions_preserve_unapplied_drafts() {
        let mut state = StudioState::default();
        state
            .segment_drafts
            .get_mut("annotation-1")
            .unwrap()
            .text = "draft still being typed".to_string();

        state.split_segment("annotation-2");

        assert_eq!(
            state.segment_drafts.get("annotation-1").unwrap().text,
            "draft still being typed"
        );
    }

    #[test]
    fn split_uses_typed_split_time() {
        let mut state = StudioState::default();
        state
            .segment_drafts
            .get_mut("annotation-1")
            .unwrap()
            .split_seconds = "2.000".to_string();

        state.split_segment("annotation-1");

        let segment = state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .unwrap();
        assert_eq!(segment.end_seconds, 2.0);
    }

    #[test]
    fn track_change_updates_segment_lane_and_speaker() {
        let mut state = StudioState::default();

        state.change_segment_track("annotation-1", "track-speaker-2");

        let segment = state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .unwrap();
        assert_eq!(segment.lane_id, "track-speaker-2");
        assert_eq!(segment.speaker_id, "speaker-2");
    }
}
