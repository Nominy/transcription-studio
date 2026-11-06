//! [`EditorDoc`]: the per-task editing facade the UI owns. It wraps the workflow (project +
//! undo history), the editor session (selection, transport, loop, zoom, viewport), per-row
//! drafts, find state, dialogs, helper focus, pointer drags and a transcript-row cache.
//!
//! Every operation is synchronous and returns an [`EditResult`]: `Ok(Outcome)` carries a status
//! line and [`Effect`]s the UI must perform (audio, scrolling, focus); `Err(EditError)` explains a
//! refusal. Rendering reads accessors (`project()`, `session()`, `drafts()`, `transcript_rows()`).

mod actions;
mod dialogs;
mod drafting;
mod extensions;
mod navigation;
mod structure;
mod text;
mod timeline;

pub use drafting::{
    DraftedRow, RowSnapshot, SEGMENT_BOUNDARY_TOLERANCE_SECONDS, TranscriptionRefusal,
    TranscriptionTarget, capitalize_first_letter, remove_word_final_ellipsis,
};
pub use text::{CompletionKey, CompletionList};

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use studio_core::Project;
use studio_waveform::WaveformHit;

use super::create_region::{CreateRegionDialog, CreateRegionError};
use super::draft::{DraftError, Drafts, SegmentDraft, TextSelection};
use super::find::FindState;
use super::format::format_time;
use super::navigate::{self, HelperFocusRestore};
use super::structure::{
    AuditionKey, OverlayDrag, SplitError, TimelineAutoscroll, WaveformCreateDrag, WaveformDrag,
};
use super::text::TextMoveError;
use super::time::TimeEditError;
use crate::editor_model::{EditorSession, ProblemCounts};
use crate::timing::WordTimings;
use crate::transcript_model::{TranscriptRowView, speaker_label};
use crate::workflow::StudioWorkflow;
use exaudio_extension::document::{DocumentEnvironment, Severity};

/// Side effects the UI performs after an operation.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Playback started (`session().is_playing` became true): start audio at the playhead.
    StartPlayback,
    /// Playback stopped: stop audio.
    StopPlayback,
    /// The playhead jumped (seek, loop wrap, selection) while playing: restart audio at the
    /// playhead. The UI may coalesce bursts.
    RestartPlayback,
    /// One-shot audition of `start..end` (playback state unchanged, not playing).
    PlayRange {
        start_seconds: f64,
        end_seconds: f64,
    },
    /// Playback speed changed; update the audio engine.
    SpeedChanged(f32),
    /// Scroll the transcript to the row: `center` = always center it, otherwise only if hidden.
    RevealSegment { segment_id: String, center: bool },
    /// Focus a row's text editor with the caret at `caret` (`char` offset).
    FocusText { segment_id: String, caret: usize },
    /// Remove keyboard focus from transcript text editors.
    BlurText,
    /// A row's draft text/selection changed programmatically: reload the row's input from
    /// [`EditorDoc::draft`].
    DraftChanged { segment_id: String },
    /// Route a document-scoped contribution through the shell's command registry.
    ExecuteCommand {
        command: String,
        arguments: serde_json::Value,
    },
}

/// Successful operation result.
#[must_use]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// Status-bar message, if any.
    pub status: Option<String>,
    pub effects: Vec<Effect>,
}

impl Outcome {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn status(status: impl Into<String>) -> Self {
        Self {
            status: Some(status.into()),
            effects: Vec::new(),
        }
    }

    pub fn with(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn with_status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    pub fn extend(mut self, effects: impl IntoIterator<Item = Effect>) -> Self {
        self.effects.extend(effects);
        self
    }

    pub fn has_effect(&self, effect: &Effect) -> bool {
        self.effects.contains(effect)
    }
}

/// Why an operation was refused. `Display` is the status-bar message.
#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("{action} unavailable: project is read-only.")]
    ReadOnly { action: &'static str },
    #[error("That segment no longer exists.")]
    SegmentNotFound(String),
    #[error("That segment has no editable draft.")]
    NoDraft(String),
    #[error("{action}: select a transcript segment first.")]
    NoSelection { action: &'static str },
    #[error(
        "{action} blocked: use the focused text editor's local undo or save transcript edits first."
    )]
    PendingDrafts { action: &'static str },
    #[error("{action} blocked: {source}")]
    Draft {
        action: &'static str,
        source: DraftError,
    },
    #[error("{0}")]
    Time(#[from] TimeEditError),
    #[error("{0}")]
    Split(#[from] SplitError),
    #[error("{0}")]
    TextMove(#[from] TextMoveError),
    #[error("{0}")]
    CreateRegion(#[from] CreateRegionError),
    #[error("{action} failed: {source}")]
    Workflow {
        action: &'static str,
        source: anyhow::Error,
    },
    /// Precondition not met (no overlay, no decoded audio, nothing to do...).
    #[error("{0}")]
    Unavailable(String),
}

pub type EditResult = Result<Outcome, EditError>;

/// Keyboard modifiers relevant to pointer gestures on the timeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PointerModifiers {
    pub shift: bool,
    pub alt: bool,
    /// Platform command modifier (ctrl on Windows/Linux, cmd on macOS).
    pub secondary: bool,
}

/// Pointer over a lane waveform.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveformHover {
    pub lane_id: String,
    pub hit: WaveformHit,
    /// Position inside the visible viewport (0..1).
    pub fraction: f32,
}

/// Transient pointer/keyboard gesture state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PointerState {
    pub modifiers: PointerModifiers,
    pub held_audition_keys: BTreeSet<AuditionKey>,
    pub waveform_hover: Option<WaveformHover>,
    pub overlay_drag: Option<OverlayDrag>,
    pub waveform_drag: Option<WaveformDrag>,
    pub create_drag: Option<WaveformCreateDrag>,
    pub autoscroll: Option<TimelineAutoscroll>,
}

/// Helper-mode focus tracking (which row owns the caret around playback toggles).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HelperFocus {
    /// Row whose text editor currently holds focus via the helper workflow.
    pub focused_segment_id: Option<String>,
    /// Where the caret returns when playback pauses (ghost cursor while playing).
    pub pending: Option<HelperFocusRestore>,
    /// Last applied restore (its row is the one to focus).
    pub last: Option<HelperFocusRestore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TagSuggestionUi {
    segment_id: String,
    text: String,
    selection: TextSelection,
    highlighted: usize,
    dismissed: bool,
}

impl TagSuggestionUi {
    fn matches(&self, segment_id: &str, draft: &SegmentDraft) -> bool {
        self.segment_id == segment_id
            && self.text == draft.text()
            && self.selection == draft.selection()
    }
}

/// Inputs of `EditorSession::transcript_rows` that change without bumping the doc generation
/// (playback moves the selection).
#[derive(Debug, Clone, PartialEq)]
struct RowsCacheKey {
    generation: u64,
    selected_segment_id: Option<String>,
}

impl RowsCacheKey {
    fn matches(&self, generation: u64, session: &EditorSession) -> bool {
        self.generation == generation && self.selected_segment_id == session.selected_segment_id
    }
}

/// Per-task editing document. Not `Sync`; owned by the UI thread.
#[derive(Debug)]
pub struct EditorDoc {
    workflow: StudioWorkflow,
    session: EditorSession,
    drafts: Drafts,
    tag_ui: Option<TagSuggestionUi>,
    find: FindState,
    create_region: Option<CreateRegionDialog>,
    helper: HelperFocus,
    pointer: PointerState,
    playback_followed_segment_id: Option<String>,
    generation: u64,
    rows_cache: RefCell<Option<(RowsCacheKey, Rc<[TranscriptRowView]>)>>,
    problem_counts_cache: Cell<Option<(u64, ProblemCounts)>>,
    word_timings: WordTimings,
}

impl EditorDoc {
    /// Opens a document, restoring timeline defaults (zoom, speed, filter, collapsed/muted
    /// lanes, lane heights) persisted in project metadata.
    pub fn new(mut workflow: StudioWorkflow, environment: DocumentEnvironment) -> Self {
        workflow.set_mutation_policy(
            environment
                .providers
                .mutation_policy
                .clone()
                .unwrap_or_else(|| std::sync::Arc::new(studio_core::NativeSegmentMutationPolicy)),
        );
        let session = EditorSession::for_project_with_environment(workflow.project(), environment);
        let drafts = Drafts::for_project(workflow.project());
        let mut doc = Self {
            workflow,
            session,
            drafts,
            tag_ui: None,
            find: FindState::default(),
            create_region: None,
            helper: HelperFocus::default(),
            pointer: PointerState::default(),
            playback_followed_segment_id: None,
            generation: 0,
            rows_cache: RefCell::new(None),
            problem_counts_cache: Cell::new(None),
            word_timings: WordTimings::default(),
        };
        doc.hydrate_timeline_defaults();
        doc
    }

    pub fn from_project(project: Project, environment: DocumentEnvironment) -> Self {
        Self::new(StudioWorkflow::from_project(project), environment)
    }

    pub fn environment(&self) -> &DocumentEnvironment {
        self.session.environment()
    }

    pub fn project(&self) -> &Project {
        self.workflow.project()
    }

    pub fn workflow(&self) -> &StudioWorkflow {
        &self.workflow
    }

    /// Direct workflow access for non-editing operations (lint snapshot metadata, diff compare,
    /// admin flags, save/export). Drafts are re-synced and the session reconciled afterwards.
    pub fn with_workflow<T>(&mut self, change: impl FnOnce(&mut StudioWorkflow) -> T) -> T {
        let value = change(&mut self.workflow);
        self.drafts
            .sync_preserving_existing(self.workflow.project());
        self.prune_tag_ui();
        self.reconcile(true);
        value
    }

    pub fn session(&self) -> &EditorSession {
        &self.session
    }

    /// Mutable session access for view-only state the doc does not wrap (overview/scrollbar
    /// dragging, lane heights, diff view, save bookkeeping). Invalidates the row cache.
    pub fn session_mut(&mut self) -> &mut EditorSession {
        self.touch();
        &mut self.session
    }

    /// Borrows the immutable project and mutable view state together without cloning either.
    pub fn with_session<R>(&mut self, update: impl FnOnce(&Project, &mut EditorSession) -> R) -> R {
        self.touch();
        update(self.workflow.project(), &mut self.session)
    }

    pub fn drafts(&self) -> &Drafts {
        &self.drafts
    }

    pub fn draft(&self, segment_id: &str) -> Option<&SegmentDraft> {
        self.drafts.get(segment_id)
    }

    pub fn find_state(&self) -> &FindState {
        &self.find
    }

    pub fn helper(&self) -> &HelperFocus {
        &self.helper
    }

    pub fn pointer(&self) -> &PointerState {
        &self.pointer
    }

    pub fn create_region(&self) -> Option<&CreateRegionDialog> {
        self.create_region.as_ref()
    }

    /// Word timings of this task (Gold timestamps), used by smart split, Alt+Click seek, the
    /// ghost cursor and auto-segmentation.
    pub fn word_timings(&self) -> &WordTimings {
        &self.word_timings
    }

    /// Installs timings for the given lanes (a newer timing run replaces older lanes).
    pub fn install_word_timings(
        &mut self,
        lanes: impl IntoIterator<Item = studio_core::timing::LaneTimings>,
    ) {
        self.word_timings.install(lanes);
        self.touch();
    }

    /// Whether the task is read-only (every edit is refused).
    pub fn is_read_only(&self) -> bool {
        self.environment().policy.read_only
    }

    /// Counter bumped by every change that can affect rendered rows.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn can_undo(&self) -> bool {
        self.workflow.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.workflow.can_redo()
    }

    /// Transcript rows under the track filter, cached until the project, drafts, selection or
    /// any session state change. A cache hit builds nothing.
    pub fn transcript_rows(&self) -> Rc<[TranscriptRowView]> {
        if let Some((key, rows)) = self.rows_cache.borrow().as_ref()
            && key.matches(self.generation, &self.session)
        {
            return Rc::clone(rows);
        }
        let rows: Rc<[TranscriptRowView]> =
            self.session.transcript_rows(self.workflow.project()).into();
        let key = RowsCacheKey {
            generation: self.generation,
            selected_segment_id: self.session.selected_segment_id.clone(),
        };
        *self.rows_cache.borrow_mut() = Some((key, Rc::clone(&rows)));
        rows
    }

    /// Error/warning counts behind the problem summary, cached per generation (linting every
    /// row is far too slow to repeat on each rendered frame).
    pub fn problem_counts(&self) -> ProblemCounts {
        if let Some((generation, counts)) = self.problem_counts_cache.get()
            && generation == self.generation
        {
            return counts;
        }
        let counts = self.session.problem_counts(self.workflow.project());
        self.problem_counts_cache
            .set(Some((self.generation, counts)));
        counts
    }

    /// Lint shown for a row: the draft's validation state when pending/invalid, else the
    /// project lint of the row.
    pub fn row_lint(&self, row: &TranscriptRowView) -> Option<(Severity, String)> {
        self.workflow
            .project()
            .segment(&row.segment_id)
            .and_then(|segment| self.drafts.draft_lint(segment))
            .map_or_else(
                || {
                    row.lint
                        .as_ref()
                        .map(|lint| (lint.severity, lint.message.clone()))
                },
                |lint| Some((lint.severity, lint.message)),
            )
    }

    /// Installs decoded audio / peaks for a lane. The audio may grow the timeline bounds: a
    /// timeline without extent (a project without segments) opens its view on it
    /// ([`EditorSession::show_initial_view`]); one with segments keeps its start and shows the
    /// seconds its zoom gives it ([`EditorSession::fit_viewport_to_zoom`]).
    pub fn set_lane_waveform(
        &mut self,
        lane_id: impl Into<String>,
        waveform: crate::timeline_model::LaneWaveform,
    ) {
        let project = self.workflow.project();
        let bounds = self.session.timeline_bounds(project);
        let had_extent = bounds.end_seconds > bounds.start_seconds;
        self.session.set_lane_waveform(lane_id, waveform);
        if had_extent {
            self.session.fit_viewport_to_zoom(project);
        } else {
            self.session.show_initial_view(project);
        }
        self.touch();
    }

    /// Paints `waveform` for a lane instead of its original (`None` restores the original);
    /// returns the replaced display waveform. See [`EditorSession::set_lane_display_waveform`].
    pub fn set_lane_display_waveform(
        &mut self,
        lane_id: &str,
        waveform: Option<crate::timeline_model::LaneWaveform>,
    ) -> Option<crate::timeline_model::LaneWaveform> {
        let previous = self.session.set_lane_display_waveform(lane_id, waveform);
        self.touch();
        previous
    }

    // ---- shared internals -------------------------------------------------------------

    fn touch(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    fn guard(&self, action: &'static str) -> Result<(), EditError> {
        if self.is_read_only() {
            Err(EditError::ReadOnly { action })
        } else {
            Ok(())
        }
    }

    /// Runs a workflow mutation (read-only guarded), then re-syncs drafts.
    fn apply<T>(
        &mut self,
        action: &'static str,
        change: impl FnOnce(&mut StudioWorkflow) -> anyhow::Result<T>,
    ) -> Result<T, EditError> {
        self.guard(action)?;
        let value =
            change(&mut self.workflow).map_err(|source| EditError::Workflow { action, source })?;
        self.drafts
            .sync_preserving_existing(self.workflow.project());
        self.prune_tag_ui();
        self.touch();
        Ok(value)
    }

    /// Records an edit and refreshes registered providers for the new revision.
    fn mark_dirty(&mut self) {
        self.session.mark_dirty();
        self.session.refresh_diagnostics(self.workflow.project());
        self.touch();
    }

    fn reconcile(&mut self, preserve_viewport: bool) {
        let project = self.workflow.project();
        if preserve_viewport {
            self.session.reconcile_project_preserving_viewport(project);
        } else {
            self.session.reconcile_project(project);
        }
        self.reconcile_helper_focus_to_visible();
        self.touch();
    }

    fn reconcile_selection_to_visible(&mut self) {
        self.session
            .reconcile_selection_to_visible(self.workflow.project());
        self.reconcile_helper_focus_to_visible();
        self.touch();
    }

    fn reconcile_helper_focus_to_visible(&mut self) {
        let project = self.workflow.project();
        let session = &self.session;
        let visible = |segment_id: &str| {
            project
                .segment(segment_id)
                .is_some_and(|segment| navigate::segment_is_visible(session, segment))
        };
        if self
            .helper
            .focused_segment_id
            .as_deref()
            .is_some_and(|id| !visible(id))
        {
            self.helper.focused_segment_id = None;
        }
        if self
            .helper
            .pending
            .as_ref()
            .is_some_and(|restore| !visible(&restore.segment_id))
        {
            self.helper.pending = None;
        }
        if self
            .helper
            .last
            .as_ref()
            .is_some_and(|restore| !visible(&restore.segment_id))
        {
            self.helper.last = None;
        }
    }

    fn prune_tag_ui(&mut self) {
        if self
            .tag_ui
            .as_ref()
            .is_some_and(|ui| self.drafts.get(&ui.segment_id).is_none())
        {
            self.tag_ui = None;
        }
    }

    fn clear_tag_ui_for(&mut self, segment_id: &str) {
        if self
            .tag_ui
            .as_ref()
            .is_some_and(|ui| ui.segment_id == segment_id)
        {
            self.tag_ui = None;
        }
    }

    fn segment(&self, segment_id: &str) -> Result<studio_core::Segment, EditError> {
        self.workflow
            .project()
            .segment(segment_id)
            .cloned()
            .ok_or_else(|| EditError::SegmentNotFound(segment_id.to_string()))
    }

    /// How messages name a segment: its `#` number and speaker as the transcript shows them
    /// (`segment 3 (Speaker 1)`). A row hidden by the track filter is named by speaker and
    /// start time instead.
    pub fn segment_label(&self, segment_id: &str) -> String {
        if let Some(row) = self
            .transcript_rows()
            .iter()
            .find(|row| row.segment_id == segment_id)
        {
            return format!("segment {} ({})", row.display_id, row.speaker_label);
        }
        let project = self.workflow.project();
        match project.segment(segment_id) {
            Some(segment) => format!(
                "the {} segment at {}",
                speaker_label(project, &segment.speaker_id),
                format_time(segment.start_seconds)
            ),
            None => "the segment".to_string(),
        }
    }

    /// How messages name a lane: its speaker (`Speaker 2`).
    pub fn lane_label(&self, lane_id: &str) -> String {
        let project = self.workflow.project();
        project
            .lanes
            .iter()
            .find(|lane| lane.id == lane_id)
            .map_or_else(
                || "the track".to_string(),
                |lane| speaker_label(project, &lane.speaker_id),
            )
    }

    fn selected_segment_id(&self, action: &'static str) -> Result<String, EditError> {
        self.session
            .selected_segment_id
            .clone()
            .ok_or(EditError::NoSelection { action })
    }

    fn restart_if_playing(&self) -> Option<Effect> {
        self.session.is_playing.then_some(Effect::RestartPlayback)
    }
}
