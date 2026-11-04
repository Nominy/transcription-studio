//! The editor screen: composes toolbar, timeline, transcript, transport strip, status bar and
//! side panels around one [`EditorModel`], drives the frame clock, and dispatches keyboard
//! shortcuts that are not handled by a focused transcript row.

pub mod blended_waveforms;
pub mod denoise;
pub mod inference_audio;
pub mod model;
pub mod panels;
pub mod status_bar;
pub mod timeline;
pub mod timing_cache;
pub mod toolbar;
pub mod transcript;

use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use studio_editor::edit::EditorAction;
use studio_editor::edit::keymap::{FocusTarget, KeyFocus, audition_key_for};
use studio_services::prefs::EditorLayout;

use crate::keymap::{ActiveKeymap, keystroke_string};
use crate::services::AppServices;
use model::EditorModel;
use panels::capabilities::CapabilityPanel;
use panels::context::ContextPanels;
use panels::feedback::FeedbackPanel;
use panels::gold_draft::{DraftMode, GoldDraftPanel};
use panels::grader::GraderPanel;
use panels::review_workspace::ReviewWorkspacePanel;
use panels::task::TaskPanels;
use panels::transport::TransportStrip;
use status_bar::StatusBar;
use timeline::TimelineView;
use toolbar::Toolbar;
use transcript::{FindBar, TranscriptView};

/// Narrower windows overlay the side drawer on the editor instead of placing it beside it
/// (the editor needs ~52rem, the drawers are ~28rem wide).
const DRAWER_INLINE_MIN_WIDTH: Rems = rems(80.);

/// Slot of the waveform card above the transcript: toolbar plus the timeline's natural height
/// for the visible lanes, at most half the body.
fn waveform_slot(height: Pixels) -> Div {
    div().h(height).max_h(relative(0.5)).flex_none()
}

/// Emitted by child views (toolbar back button, transport strip) to leave the task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLeave;

/// Emitted by the toolbar's wand: open Gold Draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenGoldDraft;

/// Events for the root view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorScreenEvent {
    /// The user confirmed leaving the task (saved or discarded). The root unloads the model
    /// and shows the dashboard.
    Leave,
}

pub struct EditorScreen {
    model: Entity<EditorModel>,
    toolbar: Entity<Toolbar>,
    timeline: Entity<TimelineView>,
    transcript_pane: Entity<TranscriptPane>,
    status_bar: Entity<StatusBar>,
    context_panels: Entity<ContextPanels>,
    feedback: Entity<FeedbackPanel>,
    capabilities: Entity<CapabilityPanel>,
    gold_draft: Entity<GoldDraftPanel>,
    review: Entity<ReviewWorkspacePanel>,
    grader: Entity<GraderPanel>,
    panels: Entity<TaskPanels>,
    focus_handle: FocusHandle,
    frame_scheduled: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EditorScreenEvent> for EditorScreen {}

impl EditorScreen {
    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let toolbar = cx.new(|cx| Toolbar::new(model.clone(), window, cx));
        let timeline = cx.new(|cx| TimelineView::new(model.clone(), window, cx));
        let transport = cx.new(|cx| TransportStrip::new(model.clone(), window, cx));
        let status_bar = cx.new(|cx| StatusBar::new(model.clone(), window, cx));
        let context_panels = cx.new(|cx| ContextPanels::new(model.clone(), window, cx));
        let feedback = cx.new(|cx| FeedbackPanel::new(model.clone(), window, cx));
        let gold_draft = cx.new(|cx| GoldDraftPanel::new(model.clone(), cx));
        let capabilities =
            cx.new(|cx| CapabilityPanel::new(model.clone(), gold_draft.clone(), window, cx));
        let review = cx.new(|cx| ReviewWorkspacePanel::new(model.clone(), window, cx));
        let grader = cx.new(|cx| GraderPanel::new(model.clone(), window, cx));
        let transcript_pane = cx.new(|cx| {
            TranscriptPane::new(
                model.clone(),
                transport.clone(),
                focus_handle.clone(),
                window,
                cx,
            )
        });
        let panels = TaskPanels::for_model(&model, window, cx);

        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            // The open drawer is mounted from the panel state.
            cx.observe(&panels, |_, _, cx| cx.notify()),
            cx.observe(&gold_draft, |_, _, cx| cx.notify()),
            cx.subscribe_in(&toolbar, window, |this, _, _: &RequestLeave, window, cx| {
                this.leave(window, cx);
            }),
            cx.subscribe_in(
                &toolbar,
                window,
                |this, _, _: &OpenGoldDraft, window, cx| {
                    this.gold_draft.update(cx, |gold, cx| {
                        gold.open(DraftMode::Draft, window, cx);
                    });
                },
            ),
            cx.subscribe_in(
                &transport,
                window,
                |this, _, _: &RequestLeave, window, cx| {
                    this.leave(window, cx);
                },
            ),
        ];

        focus_handle.focus(window, cx);

        Self {
            model,
            toolbar,
            timeline,
            transcript_pane,
            status_bar,
            context_panels,
            feedback,
            capabilities,
            gold_draft,
            review,
            grader,
            panels,
            focus_handle,
            frame_scheduled: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn model(&self) -> &Entity<EditorModel> {
        &self.model
    }

    /// Back to projects: confirm/save via the flows, then hand control to the root.
    fn leave(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editor = cx.entity().downgrade();
        panels::flows::request_leave(&self.model, window, cx, move |_, cx| {
            editor
                .update(cx, |_, cx| cx.emit(EditorScreenEvent::Leave))
                .ok();
        });
    }

    fn key_focus(&self, window: &mut Window, cx: &mut App) -> KeyFocus {
        let focused_row = self.transcript_pane.read(cx).focused_segment_id(window, cx);
        let target = if focused_row.is_some() {
            FocusTarget::TranscriptText
        } else if window.has_focused_input(cx) {
            FocusTarget::Input
        } else {
            FocusTarget::Workspace
        };
        KeyFocus {
            target,
            cut_preview_active: self.model.read(cx).doc().session().overlay_range.is_some(),
            tag_suggestions_open: false,
        }
    }

    /// The Magic Review workspace, the Grader or Gold Draft covers the editor; it gets no
    /// shortcuts.
    fn review_modal_open(&self, cx: &App) -> bool {
        let panels = self.panels.read(cx);
        ReviewWorkspacePanel::is_open(panels)
            || GraderPanel::is_open(panels)
            || self.gold_draft.read(cx).is_open()
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) || window.has_active_sheet(cx) || self.review_modal_open(cx)
        {
            return;
        }
        let focus = self.key_focus(window, cx);
        if focus.target == FocusTarget::Workspace
            && let Some(key) = audition_key_for(&keystroke_string(&event.keystroke))
        {
            self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| doc.set_audition_key(key, true));
            });
        }
        let Some(action) = ActiveKeymap::resolve(cx, &event.keystroke, focus) else {
            return;
        };
        let focused_row = self.transcript_pane.read(cx).focused_segment_id(window, cx);
        let handled = self.model.update(cx, |model, cx| {
            model.perform(action, focused_row.as_deref(), cx)
        });
        if !handled {
            self.perform_app_action(action, window, cx);
        }
        cx.stop_propagation();
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = audition_key_for(&keystroke_string(&event.keystroke)) {
            self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| doc.set_audition_key(key, false));
            });
        }
    }

    fn on_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = studio_editor::edit::PointerModifiers {
            shift: event.modifiers.shift,
            alt: event.modifiers.alt,
            secondary: event.modifiers.secondary(cx),
        };
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.set_modifiers(modifiers));
        });
    }

    /// Actions the doc returns to the UI (`perform` → `None`).
    fn perform_app_action(
        &mut self,
        action: EditorAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            EditorAction::BackToProjects => self.leave(window, cx),
            EditorAction::DraftTranscribeSelectedSegment => {
                let focused = self.transcript_pane.read(cx).focused_segment_id(window, cx);
                self.gold_draft.update(cx, |gold, cx| {
                    gold.transcribe_segment(focused, window, cx);
                });
            }
            EditorAction::SwapSplitPanes => {
                let swapped = self
                    .transcript_pane
                    .update(cx, |pane, cx| pane.swap_panes(cx));
                if !swapped {
                    self.model.update(cx, |model, cx| {
                        model.set_status(
                            "Swapping speaker panes applies to the Split layouts (Settings → \
                             Editor layout).",
                            cx,
                        );
                    });
                }
            }
            _ => {}
        }
    }

    /// Keeps one frame callback in flight while playback/autoscroll runs.
    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_scheduled || !self.model.read(cx).needs_frames() {
            return;
        }
        self.frame_scheduled = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.frame_scheduled = false;
            this.model.update(cx, |model, cx| model.frame(cx));
            this.schedule_frame(window, cx);
        });
    }

    fn render_body(&self, window: &Window, cx: &App) -> AnyElement {
        // Playback frames re-render this screen (the timeline, overview and status bar live
        // in it); cached children are reused unless they changed themselves.
        let waveform_card = v_flex()
            .size_full()
            .gap_2()
            .child(
                self.toolbar.clone().cached(
                    StyleRefinement::default()
                        .w_full()
                        .h(Toolbar::HEIGHT)
                        .flex_none(),
                ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    self.timeline
                        .clone()
                        .cached(StyleRefinement::default().size_full()),
                ),
            );
        let transcript_card = self
            .transcript_pane
            .clone()
            .cached(StyleRefinement::default().size_full());

        // The waveform card fits its visible lanes (one speaker leaves the rest to the
        // transcript) and is capped to half the body so the transcript keeps room at large
        // interface zoom.
        let rem = window.rem_size();
        let rows = timeline::visible_lane_rows(self.model.read(cx).doc());
        // Toolbar row and the card's `gap_2` above the timeline.
        let slot_height = rem * (Toolbar::HEIGHT.0 + 0.5) + timeline::natural_height(rows, rem);
        v_flex()
            .size_full()
            .gap_2()
            .child(waveform_slot(slot_height).child(waveform_card))
            .child(div().flex_1().min_h_0().child(transcript_card))
            .into_any_element()
    }

    /// The open side drawer, mounted cached at its fixed width so playback frames reuse it.
    /// In windows too narrow to share with the editor it overlays the editor's right edge
    /// instead of squeezing it.
    fn render_drawer(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        let panels = self.panels.read(cx);
        let overlay =
            window.viewport_size().width < DRAWER_INLINE_MIN_WIDTH.to_pixels(window.rem_size());
        let drawer_style = |width: Rems| {
            let style = StyleRefinement::default()
                .w(width)
                .max_w(relative(0.9))
                .h_full()
                .flex_none();
            if overlay {
                style.absolute().top_0().right_0().shadow_lg()
            } else {
                style
            }
        };
        if FeedbackPanel::is_open(panels) {
            Some(
                self.feedback
                    .clone()
                    .cached(drawer_style(FeedbackPanel::WIDTH))
                    .into_any_element(),
            )
        } else if CapabilityPanel::is_open(panels) {
            Some(
                self.capabilities
                    .clone()
                    .cached(drawer_style(CapabilityPanel::WIDTH))
                    .into_any_element(),
            )
        } else {
            None
        }
    }

    /// Full-editor overlays (Magic Review workspace, Grader dialog) while open.
    fn render_overlays(&self, cx: &App) -> Vec<AnyElement> {
        let panels = self.panels.read(cx);
        let overlay = || StyleRefinement::default().absolute().inset_0();
        let mut overlays = Vec::new();
        if ReviewWorkspacePanel::is_open(panels) {
            overlays.push(self.review.clone().cached(overlay()).into_any_element());
        }
        if GraderPanel::is_open(panels) {
            overlays.push(self.grader.clone().cached(overlay()).into_any_element());
        }
        if self.gold_draft.read(cx).is_open() {
            overlays.push(self.gold_draft.clone().into_any_element());
        }
        overlays
    }
}

/// Transport strip, find bar and the transcript grids below the waveform card: one grid of
/// every speaker (Classic) or one grid per audio lane, stacked or side by side. The grids
/// follow the editor layout preference at open time. Mounted cached: the editor screen
/// re-renders on every playback frame, this pane only when its children changed or the track
/// filter shows other lanes.
struct TranscriptPane {
    layout: EditorLayout,
    transport: Entity<TransportStrip>,
    find_bar: Entity<FindBar>,
    grids: Vec<TranscriptGrid>,
    /// Split layouts show the lane grids in reverse order (Helper `split.swapPanes`, Ctrl+I).
    /// Like Babel's swap it lasts while the task is open; split sizes stay with their slot.
    swapped: bool,
    _subscription: Subscription,
}

struct TranscriptGrid {
    view: Entity<TranscriptView>,
    /// Whether the track filter shows this grid's lane (always for the grid of every lane).
    shown: bool,
}

impl TranscriptPane {
    fn new(
        model: Entity<EditorModel>,
        transport: Entity<TransportStrip>,
        return_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let layout = cx
            .global::<AppServices>()
            .storage
            .preferences()
            .editor_layout;
        let mut lanes: Vec<Option<String>> = match layout {
            EditorLayout::Classic => Vec::new(),
            EditorLayout::SplitStacked | EditorLayout::SplitSideBySide => {
                let project = model.read(cx).doc().project();
                project
                    .lanes
                    .iter()
                    .map(|lane| Some(lane.id.clone()))
                    .collect()
            }
        };
        // Classic, and a project without lanes, show one grid of every lane.
        if lanes.is_empty() {
            lanes.push(None);
        }
        let grids = lanes
            .into_iter()
            .map(|lane| TranscriptGrid {
                view: cx.new(|cx| TranscriptView::new(model.clone(), lane, window, cx)),
                shown: true,
            })
            .collect();
        let find_bar = cx.new(|cx| FindBar::new(model.clone(), return_focus, window, cx));
        let subscription = cx.observe(&model, |this: &mut Self, model, cx| {
            if this.sync_shown(&model, cx) {
                cx.notify();
            }
        });
        let mut this = Self {
            layout,
            transport,
            find_bar,
            grids,
            swapped: false,
            _subscription: subscription,
        };
        this.sync_shown(&model, cx);
        this
    }

    /// Applies the track filter to the lane grids; whether any changed.
    fn sync_shown(&mut self, model: &Entity<EditorModel>, cx: &App) -> bool {
        let filter = &model.read(cx).doc().session().track_filter;
        let mut changed = false;
        for grid in &mut self.grids {
            let shown = grid
                .view
                .read(cx)
                .lane_id()
                .is_none_or(|lane| filter.matches_lane(lane));
            changed |= shown != grid.shown;
            grid.shown = shown;
        }
        changed
    }

    /// Reverses the order of the speaker panes in the split layouts; `false` (no change) in
    /// the Classic layout.
    fn swap_panes(&mut self, cx: &mut Context<Self>) -> bool {
        if self.layout == EditorLayout::Classic {
            return false;
        }
        self.swapped = !self.swapped;
        cx.notify();
        true
    }

    /// Segment whose row editor has keyboard focus, in any grid.
    fn focused_segment_id(&self, window: &Window, cx: &App) -> Option<String> {
        self.grids
            .iter()
            .find_map(|grid| grid.view.read(cx).focused_segment_id(window, cx))
    }
}

impl Render for TranscriptPane {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let grid = |view: &Entity<TranscriptView>| {
            view.clone().cached(StyleRefinement::default().size_full())
        };
        let mut shown: Vec<&Entity<TranscriptView>> = self
            .grids
            .iter()
            .filter(|grid| grid.shown)
            .map(|grid| &grid.view)
            .collect();
        if self.swapped {
            shown.reverse();
        }
        let body = if let [view] = shown.as_slice() {
            grid(view).into_any_element()
        } else {
            // Split sizes are kept per number of shown grids.
            let split_id = ("transcript-split", shown.len());
            let split = if self.layout == EditorLayout::SplitSideBySide {
                h_resizable(split_id)
            } else {
                v_resizable(split_id)
            };
            split
                .children(shown.iter().map(|view| resizable_panel().child(grid(view))))
                .into_any_element()
        };
        // The find bar floats over the grids' top-right corner (it never moves a row).
        v_flex().size_full().child(self.transport.clone()).child(
            div().relative().flex_1().min_h_0().child(body).child(
                div()
                    .absolute()
                    .top_1()
                    .right_4()
                    .left_4()
                    .flex()
                    .justify_end()
                    .child(self.find_bar.clone()),
            ),
        )
    }
}

impl Focusable for EditorScreen {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditorScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.schedule_frame(window, cx);
        let theme = cx.theme();
        div()
            .id("editor-screen")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .relative()
                    .size_full()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .p_2()
                            .gap_2()
                            .child(self.context_panels.clone())
                            .child(div().flex_1().min_h_0().child(self.render_body(window, cx)))
                            .child(self.status_bar.clone()),
                    )
                    .children(self.render_drawer(window, cx))
                    .children(self.render_overlays(cx)),
            )
    }
}
