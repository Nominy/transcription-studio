use std::{collections::BTreeMap, fs, path::Path};

use iced::widget::{
    Column, Row, Space, button, column, container, horizontal_rule, pick_list, row, scrollable,
    slider, text, text_input, vertical_rule,
};
use iced::{Color, Element, Fill, Length, Theme, border};
use studio_app::{
    EditorSession, LintSeverity, ReviewState, StudioWorkflow, TimelineLaneView, TrackFilter,
    TranscriptRowView, WaveformBlock, format_mm_ss, load_sample_project,
};
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
    SubmitReview,
    CopySupportId,
    ShowHotkeys,
    TogglePlayback,
    PreviousSegment,
    NextSegment,
    SpeedChanged(String),
    ZoomChanged(f32),
    ToggleDiffView,
    ToggleLoop,
    ToggleLaneMute(String),
    TrackFilterChanged(TrackFilter),
    SelectSegment(String),
    ToggleActions(String),
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
    editor: Option<EditorSession>,
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
        let editor = workflow
            .as_ref()
            .ok()
            .map(|workflow| EditorSession::for_project(workflow.project()));
        let status = match &workflow {
            Ok(_) => "Loaded bundled Babel sample project.".to_string(),
            Err(error) => format!("Could not load bundled Babel sample project: {error}"),
        };
        let mut state = Self {
            workflow,
            editor,
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
            .map(|segment| (segment.id.clone(), SegmentDraft::from_segment(segment)))
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
        self.editor = Some(EditorSession::for_project(workflow.project()));
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

    fn with_editor_mut(&mut self, action: impl FnOnce(&mut EditorSession)) {
        if let Some(editor) = &mut self.editor {
            action(editor);
        }
    }

    fn mark_dirty(&mut self) {
        self.with_editor_mut(EditorSession::mark_dirty);
    }

    fn mark_saved(&mut self) {
        self.with_editor_mut(EditorSession::mark_saved);
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
        if self.with_workflow_mut("Save", |workflow| {
            workflow
                .save_local_project(&path)
                .map_err(|error| error.to_string())?;
            Ok(format!("Saved Studio project `{path}`."))
        }) {
            self.mark_saved();
        }
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

    fn submit_review(&mut self) {
        let Some(project) = self.project().cloned() else {
            self.status = "Submit unavailable: no project loaded.".to_string();
            return;
        };
        let Some(editor) = &mut self.editor else {
            self.status = "Submit unavailable: no editor session.".to_string();
            return;
        };

        match editor.submit_review(&project) {
            Ok(()) => {
                self.status =
                    "Review submitted locally; export Babel JSON for handoff.".to_string();
            }
            Err(error) => {
                self.status = format!("Submit blocked: {error}");
            }
        }
    }

    fn undo(&mut self) {
        if self.with_workflow_mut("Undo", |workflow| {
            workflow.undo().map_err(|error| error.to_string())?;
            Ok("Undid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
        }
    }

    fn redo(&mut self) {
        if self.with_workflow_mut("Redo", |workflow| {
            workflow.redo().map_err(|error| error.to_string())?;
            Ok("Redid last edit.".to_string())
        }) {
            self.reset_segment_drafts();
            self.mark_dirty();
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
            self.mark_dirty();
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
            self.mark_dirty();
        }
    }

    fn merge_segment(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        if self.with_workflow_mut("Merge", |workflow| {
            workflow
                .merge_with_next(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Merged `{segment_id}` with the next segment on its track."
            ))
        }) {
            self.refresh_segment_draft(&segment_id);
            self.mark_dirty();
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
            self.mark_dirty();
        }
    }

    fn create_segment_after(&mut self, segment_id: &str) {
        let segment_id = segment_id.to_string();
        if self.with_workflow_mut("Create", |workflow| {
            let new_segment_id = workflow
                .create_segment_after(&segment_id)
                .map_err(|error| error.to_string())?;
            Ok(format!("Created `{new_segment_id}` after `{segment_id}`."))
        }) {
            self.mark_dirty();
        }
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
            self.mark_dirty();
        }
    }

    fn select_segment(&mut self, segment_id: &str) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        self.with_editor_mut(|editor| editor.select_segment(&project, segment_id));
    }

    fn select_adjacent_segment(&mut self, direction: i32) {
        let Some(project) = self.project().cloned() else {
            return;
        };
        let Some(editor) = &mut self.editor else {
            return;
        };
        let rows = editor.transcript_rows(&project);
        if rows.is_empty() {
            return;
        }
        let current_index = rows
            .iter()
            .position(|row| Some(row.segment_id.as_str()) == editor.selected_segment_id.as_deref())
            .unwrap_or(0);
        let next_index = if direction < 0 {
            current_index.saturating_sub(1)
        } else {
            (current_index + 1).min(rows.len() - 1)
        };
        editor.select_segment(&project, &rows[next_index].segment_id);
    }
}

fn update(state: &mut StudioState, message: Message) {
    match message {
        Message::ImportPathChanged(path) => state.import_path = path,
        Message::LocalProjectPathChanged(path) => state.local_project_path = path,
        Message::ExportPathChanged(path) => state.export_path = path,
        Message::ImportBabelJson => state.import_babel_json(),
        Message::LoadLocalProject => state.load_local_project(),
        Message::SaveLocalProject => state.save_local_project(),
        Message::ExportBabelJson => state.export_babel_json(),
        Message::SubmitReview => state.submit_review(),
        Message::CopySupportId => {
            if let Some(editor) = &state.editor {
                state.status = format!("Support ID copied: {}", editor.support_id);
            }
        }
        Message::ShowHotkeys => {
            state.status =
                "Hotkeys: Space play/pause, Ctrl+S save, Ctrl+Z undo, Ctrl+Y redo, Enter apply."
                    .to_string();
        }
        Message::TogglePlayback => state.with_editor_mut(EditorSession::toggle_playback),
        Message::PreviousSegment => state.select_adjacent_segment(-1),
        Message::NextSegment => state.select_adjacent_segment(1),
        Message::SpeedChanged(value) => {
            if let Some(speed) = parse_speed(&value) {
                state.with_editor_mut(|editor| editor.set_speed(speed));
            }
        }
        Message::ZoomChanged(value) => state.with_editor_mut(|editor| editor.set_zoom(value)),
        Message::ToggleDiffView => state.with_editor_mut(EditorSession::toggle_diff_view),
        Message::ToggleLoop => state.with_editor_mut(EditorSession::toggle_loop),
        Message::ToggleLaneMute(lane_id) => {
            state.with_editor_mut(|editor| editor.toggle_lane_mute(&lane_id));
        }
        Message::TrackFilterChanged(filter) => {
            state.with_editor_mut(|editor| editor.track_filter = filter);
        }
        Message::SelectSegment(segment_id) => state.select_segment(&segment_id),
        Message::ToggleActions(segment_id) => {
            state.with_editor_mut(|editor| {
                if editor.open_actions_segment_id.as_deref() == Some(segment_id.as_str()) {
                    editor.open_actions_segment_id = None;
                } else {
                    editor.open_actions_segment_id = Some(segment_id);
                }
            });
        }
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
    let content = match (&state.workflow, &state.editor) {
        (Ok(workflow), Some(editor)) => view_editor(state, workflow.project(), editor),
        (Err(error), _) => container(
            column![
                text("Transcription Studio").size(24),
                text("No project is currently loaded."),
                text(error).size(13),
                view_path_toolbar(state),
            ]
            .spacing(12),
        )
        .padding(18)
        .style(panel_style)
        .into(),
        _ => container(text("Editor session is not available."))
            .padding(18)
            .style(panel_style)
            .into(),
    };

    container(content)
        .padding(12)
        .width(Fill)
        .height(Fill)
        .style(app_background_style)
        .into()
}

fn view_editor<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    column![
        view_top_toolbar(project, editor),
        view_path_toolbar(state),
        view_status_bar(state, project, editor),
        view_timeline(project, editor),
        view_transcript_workspace(state, project, editor),
    ]
    .spacing(8)
    .into()
}

fn view_top_toolbar<'a>(project: &'a Project, editor: &'a EditorSession) -> Element<'a, Message> {
    let zoom = row![
        text("-").size(16),
        slider(0.5..=4.0, editor.zoom, Message::ZoomChanged)
            .step(0.1)
            .width(Length::Fixed(260.0)),
        text("+").size(16),
    ]
    .spacing(8);

    let speed_options = vec![
        "0.75x".to_string(),
        "1x".to_string(),
        "1.25x".to_string(),
        "1.5x".to_string(),
        "2x".to_string(),
    ];
    let speed = pick_list(
        speed_options,
        Some(format_speed(editor.speed)),
        Message::SpeedChanged,
    )
    .width(Length::Fixed(78.0));

    let overview = view_overview_waveform(project, editor);

    container(
        row![
            zoom,
            vertical_rule(1),
            speed,
            overview,
            button(text("Copy Support ID")).on_press(Message::CopySupportId),
            button(text("Hotkeys")).on_press(Message::ShowHotkeys),
        ]
        .spacing(12),
    )
    .padding(10)
    .width(Fill)
    .style(panel_style)
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

    let project_row = row![
        text("Studio").width(Length::Fixed(82.0)),
        text_input(DEFAULT_LOCAL_PROJECT_PATH, &state.local_project_path)
            .on_input(Message::LocalProjectPathChanged)
            .on_submit(Message::LoadLocalProject)
            .padding(5)
            .width(Fill),
        button(text("Load")).on_press(Message::LoadLocalProject),
        button(text("Save")).on_press(Message::SaveLocalProject),
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

    container(column![import_row, project_row, export_row].spacing(6))
        .padding(10)
        .width(Fill)
        .style(subtle_panel_style)
        .into()
}

fn view_status_bar<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let blocking = editor.blocking_lint_count(project);
    let save_text = if editor.is_dirty { "Unsaved" } else { "Saved" };
    let review_text = match editor.review_state {
        ReviewState::Draft => "Review Draft",
        ReviewState::Submitted => "Review Submitted",
    };
    let diff_label = if editor.diff_view {
        "Diff View: On"
    } else {
        "Diff View: Off"
    };
    let loop_label = if editor.loop_enabled {
        "Loop: On"
    } else {
        "Loop: Off"
    };

    container(
        row![
            button(text(diff_label)).on_press(Message::ToggleDiffView),
            button(text(loop_label)).on_press(Message::ToggleLoop),
            text(format!(
                "Playhead {}",
                format_mm_ss(editor.playhead_seconds)
            )),
            text(format!("{blocking} blocking lint")),
            text(save_text),
            text(review_text),
            text(&state.status).width(Fill),
        ]
        .spacing(12),
    )
    .padding(8)
    .width(Fill)
    .style(subtle_panel_style)
    .into()
}

fn view_overview_waveform<'a>(
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let lanes = editor.timeline_lanes(project);
    let mut overview = Row::new().spacing(2).width(Fill);
    let mut has_blocks = false;
    for lane in lanes {
        for block in lane.blocks {
            has_blocks = true;
            let color_token = lane.color_token.clone();
            overview = overview.push(
                container(text(""))
                    .height(Length::Fixed(18.0))
                    .width(Length::FillPortion(portion(block.width_percent)))
                    .style(move |_| block_style(&color_token, false)),
            );
        }
    }
    if !has_blocks {
        overview = overview.push(text("No waveform data").size(12));
    }

    container(overview)
        .padding(6)
        .width(Fill)
        .style(waveform_backdrop_style)
        .into()
}

fn view_timeline<'a>(project: &'a Project, editor: &'a EditorSession) -> Element<'a, Message> {
    let mut lanes = Column::new().spacing(0);
    let lane_views = editor.timeline_lanes(project);

    lanes = lanes.push(
        row![
            container(text("Loop")).width(Length::Fixed(162.0)),
            view_time_ruler(editor),
        ]
        .spacing(8),
    );

    for lane in lane_views {
        lanes = lanes.push(view_timeline_lane(editor, lane));
    }

    container(lanes)
        .padding(0)
        .width(Fill)
        .style(panel_style)
        .into()
}

fn view_time_ruler(editor: &EditorSession) -> Element<'_, Message> {
    let start = editor.viewport.start_seconds;
    let end = editor.viewport.end_seconds;
    let step = ((end - start) / 4.0).max(1.0);
    row![
        text(format_mm_ss(start)).width(Length::FillPortion(1)),
        text(format_mm_ss(start + step)).width(Length::FillPortion(1)),
        text(format_mm_ss(start + step * 2.0)).width(Length::FillPortion(1)),
        text(format_mm_ss(start + step * 3.0)).width(Length::FillPortion(1)),
        text(format_mm_ss(end)).width(Length::FillPortion(1)),
    ]
    .spacing(2)
    .width(Fill)
    .into()
}

fn view_timeline_lane<'a>(
    editor: &'a EditorSession,
    lane: TimelineLaneView,
) -> Element<'a, Message> {
    let mute_label = if lane.is_muted { "Unmute" } else { "Mute" };
    let lane_id = lane.lane_id.clone();
    let label = column![
        text(lane.label.clone()).size(15),
        text(lane.speaker_id.clone()).size(12),
        button(text(mute_label)).on_press(Message::ToggleLaneMute(lane_id)),
    ]
    .spacing(8);

    let waveform = view_lane_waveform(editor, lane);
    row![
        container(label)
            .padding(10)
            .width(Length::Fixed(162.0))
            .height(Length::Fixed(112.0))
            .style(lane_label_style),
        waveform,
    ]
    .spacing(8)
    .into()
}

fn view_lane_waveform<'a>(
    editor: &'a EditorSession,
    lane: TimelineLaneView,
) -> Element<'a, Message> {
    let mut row = Row::new().spacing(0).width(Fill);
    let mut cursor = 0.0_f32;
    for block in lane.blocks {
        let gap = (block.left_percent - cursor).max(0.0);
        if gap > 0.0 {
            row = row.push(Space::with_width(Length::FillPortion(portion(gap))));
        }
        let is_selected = editor.selected_segment_id.as_deref() == Some(block.segment_id.as_str());
        let next_cursor = (block.left_percent + block.width_percent).min(100.0);
        row = row.push(view_waveform_block(
            block,
            lane.color_token.clone(),
            is_selected,
        ));
        cursor = next_cursor;
    }
    if cursor < 100.0 {
        row = row.push(Space::with_width(Length::FillPortion(portion(
            100.0 - cursor,
        ))));
    }

    let playhead_percent = (((editor.playhead_seconds - editor.viewport.start_seconds)
        / editor.viewport.end_seconds.max(0.001))
        * 100.0)
        .clamp(0.0, 100.0) as f32;

    container(
        column![
            row![
                Space::with_width(Length::FillPortion(portion(playhead_percent))),
                container(text(""))
                    .width(Length::Fixed(2.0))
                    .height(Length::Fixed(12.0))
                    .style(playhead_style),
                Space::with_width(Length::FillPortion(portion(100.0 - playhead_percent))),
            ]
            .width(Fill),
            row,
        ]
        .spacing(3),
    )
    .padding(8)
    .width(Fill)
    .height(Length::Fixed(112.0))
    .style(waveform_backdrop_style)
    .into()
}

fn view_waveform_block<'a>(
    block: WaveformBlock,
    color_token: String,
    is_selected: bool,
) -> Element<'a, Message> {
    let mut peaks = Row::new().spacing(1).width(Fill);
    for peak in block.peaks {
        let token = color_token.clone();
        peaks = peaks.push(
            container(text(""))
                .width(Length::FillPortion(1))
                .height(Length::Fixed(18.0 + 62.0 * peak))
                .style(move |_| block_style(&token, is_selected)),
        );
    }

    button(
        container(peaks)
            .padding(2)
            .height(Length::Fixed(86.0))
            .width(Fill),
    )
    .on_press(Message::SelectSegment(block.segment_id.clone()))
    .width(Length::FillPortion(portion(block.width_percent)))
    .into()
}

fn view_transcript_workspace<'a>(
    state: &'a StudioState,
    project: &'a Project,
    editor: &'a EditorSession,
) -> Element<'a, Message> {
    let filter_options = EditorSession::track_filter_options(project);
    let toolbar = row![
        pick_list(
            filter_options,
            Some(editor.track_filter.clone()),
            Message::TrackFilterChanged,
        )
        .width(Length::Fixed(178.0)),
        vertical_rule(1),
        button(text("<<")).on_press(Message::PreviousSegment),
        button(text(if editor.is_playing { "Pause" } else { "Play" }))
            .on_press(Message::TogglePlayback),
        button(text(">>")).on_press(Message::NextSegment),
        vertical_rule(1),
        button(text("Undo")).on_press(Message::Undo),
        button(text("Redo")).on_press(Message::Redo),
        Space::with_width(Fill),
        text(if editor.is_dirty { "Unsaved" } else { "Saved" }),
        button(text("Submit Review")).on_press(Message::SubmitReview),
    ]
    .spacing(10);

    let rows = editor.transcript_rows(project);
    let mut table = Column::new().spacing(0);
    table = table.push(view_table_header());
    for row_view in rows {
        if let Some(draft) = state.segment_drafts.get(&row_view.segment_id) {
            table = table.push(view_transcript_row(project, editor, row_view, draft));
        }
    }

    container(column![toolbar, horizontal_rule(1), scrollable(table).height(Fill)].spacing(8))
        .padding(10)
        .width(Fill)
        .height(Fill)
        .style(panel_style)
        .into()
}

fn view_table_header<'a>() -> Element<'a, Message> {
    row![
        text("ID").width(Length::Fixed(42.0)),
        text("Speaker").width(Length::Fixed(104.0)),
        text("Start").width(Length::Fixed(92.0)),
        text("End").width(Length::Fixed(92.0)),
        text("Split").width(Length::Fixed(92.0)),
        text("Text").width(Fill),
        text("Linter").width(Length::Fixed(124.0)),
        text("Actions").width(Length::Fixed(80.0)),
    ]
    .spacing(8)
    .padding(8)
    .into()
}

fn view_transcript_row<'a>(
    project: &'a Project,
    editor: &'a EditorSession,
    row_view: TranscriptRowView,
    draft: &'a SegmentDraft,
) -> Element<'a, Message> {
    let segment_id = row_view.segment_id.clone();
    let track_options: Vec<String> = project.lanes.iter().map(|lane| lane.id.clone()).collect();

    let start_id = segment_id.clone();
    let start_input = text_input("0.000", &draft.start_seconds)
        .on_input(move |value| Message::SegmentStartChanged(start_id.clone(), value))
        .on_submit(Message::ApplySegment(segment_id.clone()))
        .padding(5)
        .width(Length::Fixed(92.0));

    let segment_id = row_view.segment_id.clone();
    let end_id = segment_id.clone();
    let end_input = text_input("0.000", &draft.end_seconds)
        .on_input(move |value| Message::SegmentEndChanged(end_id.clone(), value))
        .on_submit(Message::ApplySegment(segment_id.clone()))
        .padding(5)
        .width(Length::Fixed(92.0));

    let segment_id = row_view.segment_id.clone();
    let split_id = segment_id.clone();
    let split_input = text_input("0.000", &draft.split_seconds)
        .on_input(move |value| Message::SegmentSplitChanged(split_id.clone(), value))
        .on_submit(Message::SplitSegment(segment_id.clone()))
        .padding(5)
        .width(Length::Fixed(92.0));

    let segment_id = row_view.segment_id.clone();
    let text_id = segment_id.clone();
    let transcript_text_input = text_input("Transcript text", &draft.text)
        .on_input(move |value| Message::SegmentTextChanged(text_id.clone(), value))
        .on_submit(Message::ApplySegment(segment_id.clone()))
        .padding(5)
        .width(Fill);

    let segment_id = row_view.segment_id.clone();
    let track_picker = pick_list(
        track_options,
        Some(row_view.lane_id.clone()),
        move |lane_id| Message::SegmentTrackChanged(segment_id.clone(), lane_id),
    )
    .width(Length::Fixed(104.0));

    let actions_open =
        editor.open_actions_segment_id.as_deref() == Some(row_view.segment_id.as_str());
    let action_button = button(text(if actions_open { "Close" } else { "..." }))
        .on_press(Message::ToggleActions(row_view.segment_id.clone()));
    let mut row_content = Column::new().spacing(4);
    row_content = row_content.push(
        row![
            button(text(row_view.display_id.clone()))
                .on_press(Message::SelectSegment(row_view.segment_id.clone()))
                .width(Length::Fixed(42.0)),
            column![text(row_view.speaker_label.clone()).size(13), track_picker,]
                .width(Length::Fixed(104.0)),
            start_input,
            end_input,
            split_input,
            transcript_text_input,
            view_lint(
                row_view.lint.severity.clone(),
                row_view.lint.message.clone()
            ),
            action_button.width(Length::Fixed(80.0)),
        ]
        .spacing(8),
    );

    if actions_open {
        row_content = row_content.push(
            row![
                Space::with_width(Length::Fixed(42.0 + 104.0 + 92.0 + 92.0 + 92.0 + 40.0)),
                button(text("Apply")).on_press(Message::ApplySegment(row_view.segment_id.clone())),
                button(text("Split")).on_press(Message::SplitSegment(row_view.segment_id.clone())),
                button(text("Merge next"))
                    .on_press(Message::MergeSegment(row_view.segment_id.clone())),
                button(text("Create after"))
                    .on_press(Message::CreateSegmentAfter(row_view.segment_id.clone())),
                button(text("Delete"))
                    .on_press(Message::DeleteSegment(row_view.segment_id.clone())),
            ]
            .spacing(8),
        );
    }

    container(row_content)
        .padding(8)
        .width(Fill)
        .style(if row_view.is_selected {
            selected_row_style
        } else {
            table_row_style
        })
        .into()
}

fn view_lint(severity: LintSeverity, message: String) -> Element<'static, Message> {
    let color = lint_color(&severity);
    row![
        container(text(""))
            .width(Length::Fixed(12.0))
            .height(Length::Fixed(12.0))
            .style(move |_| dot_style(color)),
        text(message).size(12),
    ]
    .spacing(6)
    .width(Length::Fixed(124.0))
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

fn parse_speed(value: &str) -> Option<f32> {
    value.trim_end_matches('x').parse::<f32>().ok()
}

fn format_speed(speed: f32) -> String {
    if (speed - 1.0).abs() < f32::EPSILON {
        "1x".to_string()
    } else {
        let mut label = format!("{speed:.2}");
        while label.contains('.') && label.ends_with('0') {
            label.pop();
        }
        if label.ends_with('.') {
            label.pop();
        }
        format!("{label}x")
    }
}

fn format_seconds_for_edit(value: f64) -> String {
    value.to_string()
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

fn portion(percent: f32) -> u16 {
    percent.round().clamp(1.0, 1000.0) as u16
}

fn color_for_token(token: &str) -> Color {
    match token {
        "blue" => Color::from_rgb8(72, 166, 232),
        "green" => Color::from_rgb8(31, 170, 116),
        "amber" => Color::from_rgb8(218, 160, 46),
        _ => Color::from_rgb8(152, 106, 236),
    }
}

fn lint_color(severity: &LintSeverity) -> Color {
    match severity {
        LintSeverity::Pass => Color::from_rgb8(28, 190, 105),
        LintSeverity::Warning => Color::from_rgb8(220, 155, 30),
        LintSeverity::Error => Color::from_rgb8(218, 67, 74),
    }
}

fn app_background_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgb8(246, 248, 251))
}

fn panel_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(6)
            .width(1)
            .color(Color::from_rgb8(210, 216, 226)),
    )
}

fn subtle_panel_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(250, 251, 253))
        .border(
            border::rounded(4)
                .width(1)
                .color(Color::from_rgb8(224, 229, 236)),
        )
}

fn lane_label_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(253, 253, 254))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(224, 229, 236)),
        )
}

fn waveform_backdrop_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(248, 250, 253))
        .border(
            border::rounded(4)
                .width(1)
                .color(Color::from_rgb8(210, 224, 244)),
        )
}

fn table_row_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::WHITE).border(
        border::rounded(0)
            .width(1)
            .color(Color::from_rgb8(234, 237, 243)),
    )
}

fn selected_row_style(_theme: &Theme) -> container::Style {
    container::Style::default()
        .background(Color::from_rgb8(247, 249, 253))
        .border(
            border::rounded(0)
                .width(1)
                .color(Color::from_rgb8(190, 205, 230)),
        )
}

fn block_style(token: &str, is_selected: bool) -> container::Style {
    let color = color_for_token(token);
    let border_color = if is_selected {
        Color::from_rgb8(35, 115, 210)
    } else {
        color
    };
    container::Style::default().background(color).border(
        border::rounded(2)
            .width(if is_selected { 2 } else { 0 })
            .color(border_color),
    )
}

fn playhead_style(_theme: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgb8(20, 20, 20))
}

fn dot_style(color: Color) -> container::Style {
    container::Style::default()
        .background(color)
        .border(border::rounded(20))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use studio_app::StudioWorkflow;

    use super::{DEFAULT_IMPORT_PATH, ReviewState, StudioState};

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

        state.segment_drafts.get_mut("annotation-1").unwrap().text =
            "Only text changed.".to_string();
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
        assert!(state.editor.as_ref().unwrap().is_dirty);
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
        state.segment_drafts.get_mut("annotation-1").unwrap().text =
            "draft still being typed".to_string();

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

    #[test]
    fn submit_review_marks_clean_project_submitted_and_blocks_linted_project() {
        let mut state = StudioState::default();
        state.submit_review();
        assert_eq!(
            state.editor.as_ref().unwrap().review_state,
            ReviewState::Submitted
        );

        state
            .segment_drafts
            .get_mut("annotation-1")
            .unwrap()
            .text
            .clear();
        state.apply_segment("annotation-1");
        state.submit_review();
        assert_eq!(
            state.editor.as_ref().unwrap().review_state,
            ReviewState::Draft
        );
        assert!(state.status.contains("blocked"));
    }
}
