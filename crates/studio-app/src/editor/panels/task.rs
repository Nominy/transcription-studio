//! Per-task state shared by the action strip, side drawers, dialogs and service flows.
//!
//! One [`TaskPanels`] entity exists per open [`EditorModel`]; every panel fetches it with
//! [`TaskPanels::for_model`]. It owns what the old app kept on `StudioState` beside the
//! editor session: which drawer is open, the feedback form, the reviewer workspace, pending
//! service flows and the doc-driven dialogs (add region / edit annotation).

use gpui_kit::component::WindowExt;
use gpui_kit::*;
use studio_services::review::{
    FeedbackForm, feedback_initially_open, feedback_is_tts_project, reviewer_capability_available,
};

use super::dialogs;
use super::flows::Flows;
use super::reviewer::{Grader, MagicReview};
use crate::editor::model::EditorModel;

/// Right-hand drawer currently shown next to the editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawer {
    Closed,
    Feedback,
    Assistant,
}

/// Events for the panel views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskPanelsEvent {
    /// A flow finished the task (submitted, skipped, dropped, flagged): go back to projects.
    Leave,
}

/// The doc-owned dialog that currently has a window dialog open for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocDialog {
    CreateRegion,
    EditAnnotation,
}

pub struct TaskPanels {
    pub(super) model: Entity<EditorModel>,
    /// Bumped when the task's pending work is discarded; stale completions compare it.
    pub(super) generation: u64,
    pub drawer: Drawer,
    pub feedback: Option<FeedbackForm>,
    pub(super) feedback_attempt: u64,
    pub(super) feedback_tasks: Vec<Task<()>>,
    /// Magic Review (interactive workspace / fast mode) state.
    pub review: MagicReview,
    /// Grade review dialog state.
    pub grader: Grader,
    pub(super) flows: Flows,
    doc_dialog: Option<DocDialog>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TaskPanelsEvent> for TaskPanels {}

#[derive(Default)]
struct Registry(Option<(EntityId, WeakEntity<TaskPanels>)>);

impl Global for Registry {}

impl TaskPanels {
    /// The panel state of `model`, created on first use. Panels keep the returned handle alive.
    pub fn for_model(
        model: &Entity<EditorModel>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        if let Some(existing) = Self::find(model, cx) {
            return existing;
        }
        let panels = cx.new(|cx| Self::new(model.clone(), window, cx));
        cx.set_global(Registry(Some((model.entity_id(), panels.downgrade()))));
        panels
    }

    /// The panel state of `model`, if its editor is open.
    pub fn find(model: &Entity<EditorModel>, cx: &App) -> Option<Entity<Self>> {
        let (id, panels) = cx.try_global::<Registry>()?.0.as_ref()?;
        (*id == model.entity_id())
            .then(|| panels.upgrade())
            .flatten()
    }

    fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let project = model.read(cx).doc().project();
        let read_only = model.read(cx).doc().is_read_only();
        let drawer = if feedback_initially_open(project) {
            Drawer::Feedback
        } else {
            Drawer::Closed
        };
        let mut feedback = FeedbackForm::from_project(project);
        let bootstrap_feedback = reviewer_capability_available(project)
            && !read_only
            && !feedback.as_ref().is_some_and(|form| form.read_only);
        if bootstrap_feedback {
            feedback = Some(FeedbackForm::loading(feedback_is_tts_project(project)));
        }
        let subscriptions = vec![cx.observe_in(&model, window, |this, _, window, cx| {
            this.on_model_changed(window, cx);
        })];
        let mut this = Self {
            model,
            generation: 0,
            drawer,
            feedback,
            feedback_attempt: 0,
            feedback_tasks: Vec::new(),
            review: MagicReview::default(),
            grader: Grader::default(),
            flows: Flows::default(),
            doc_dialog: None,
            _subscriptions: subscriptions,
        };
        this.sync_feedback_submit_state(cx);
        if bootstrap_feedback {
            this.bootstrap_feedback(window, cx);
        }
        this.start_reviewer_autosave(window, cx);
        this
    }

    pub fn model(&self) -> &Entity<EditorModel> {
        &self.model
    }

    /// Shows a status-bar message.
    pub fn status(&self, message: impl Into<SharedString>, cx: &mut App) {
        let message = message.into();
        self.model
            .update(cx, |model, cx| model.set_status(message, cx));
    }

    pub fn toggle_drawer(&mut self, drawer: Drawer, cx: &mut Context<Self>) {
        self.drawer = if self.drawer == drawer {
            Drawer::Closed
        } else {
            drawer
        };
        cx.notify();
    }

    pub fn open_drawer(&mut self, drawer: Drawer, cx: &mut Context<Self>) {
        self.drawer = drawer;
        cx.notify();
    }

    fn on_model_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_submit_for_edit(cx);
        self.sync_doc_dialog(window, cx);
    }

    /// Opens/closes the window dialog that mirrors the doc's add-region / edit-annotation
    /// dialog state (the doc opens them from waveform drags and row menus).
    fn sync_doc_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.model.read(cx).doc();
        let wanted = if doc.create_region().is_some() {
            Some(DocDialog::CreateRegion)
        } else if doc.edit_annotation().is_some() {
            Some(DocDialog::EditAnnotation)
        } else {
            None
        };
        if wanted == self.doc_dialog {
            return;
        }
        if self.doc_dialog.take().is_some() {
            window.close_dialog(cx);
        }
        let panels = cx.entity();
        match wanted {
            Some(DocDialog::CreateRegion) => {
                dialogs::create_region::open(&self.model, &panels, window, cx);
            }
            Some(DocDialog::EditAnnotation) => {
                dialogs::edit_annotation::open(&self.model, &panels, window, cx);
            }
            None => return,
        }
        self.doc_dialog = wanted;
    }

    /// Called by a doc dialog's cancel path (Escape, close button, Cancel) before it asks the
    /// doc to drop its dialog, so the sync does not close the window dialog a second time.
    pub(super) fn forget_doc_dialog(&mut self) {
        self.doc_dialog = None;
    }
}
