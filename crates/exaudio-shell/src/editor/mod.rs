//! Generic editor composition and registered extension surfaces.
pub mod blended_waveforms;
pub mod model;
pub mod panels;
pub mod status_bar;
pub mod timeline;
pub mod toolbar;
pub mod transcript;
use crate::keymap::{ActiveKeymap, keystroke_string};
use crate::services::AppServices;
use crate::{ContributionRegistry, PanelLocation, ShellHandle};
use gpui_kit::component::Sizable;
use gpui_kit::component::button::Button;
use gpui_kit::component::resizable::{h_resizable, resizable_panel, v_resizable};
use gpui_kit::component::{ActiveTheme, WindowExt, h_flex, v_flex};
use gpui_kit::*;
use model::EditorModel;
use panels::transport::TransportStrip;
use status_bar::StatusBar;
use studio_editor::edit::EditorAction;
use studio_editor::edit::keymap::{FocusTarget, KeyFocus, audition_key_for};
use studio_services::prefs::EditorLayout;
use timeline::TimelineView;
use toolbar::Toolbar;
use transcript::{FindBar, TranscriptView};

fn waveform_slot(height: Pixels) -> Div {
    div().h(height).max_h(relative(0.5)).flex_none()
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestLeave;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorScreenEvent {
    Leave,
}
pub struct EditorScreen {
    model: Entity<EditorModel>,
    shell: ShellHandle,
    toolbar: Entity<Toolbar>,
    timeline: Entity<TimelineView>,
    transcript_pane: Entity<TranscriptPane>,
    status_bar: Entity<StatusBar>,
    contributions: Vec<(String, u64, PanelLocation, AnyView)>,
    registry_generation: u64,
    focus_handle: FocusHandle,
    frame_scheduled: bool,
    region_dialog_open: bool,
    _subscriptions: Vec<Subscription>,
}
impl EventEmitter<EditorScreenEvent> for EditorScreen {}
impl EditorScreen {
    pub fn new(
        model: Entity<EditorModel>,
        shell: ShellHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let toolbar = cx.new(|cx| Toolbar::new(model.clone(), window, cx));
        let timeline = cx.new(|cx| TimelineView::new(model.clone(), window, cx));
        let transport = cx.new(|cx| TransportStrip::new(model.clone(), window, cx));
        let status_bar = cx.new(|cx| StatusBar::new(model.clone(), window, cx));
        let transcript_pane = cx.new(|cx| {
            TranscriptPane::new(model.clone(), transport, focus_handle.clone(), window, cx)
        });
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&toolbar, window, |_, _, _: &RequestLeave, _, cx| {
                cx.emit(EditorScreenEvent::Leave)
            }),
            cx.subscribe_in(
                &model,
                window,
                |this, _, event: &model::EditorEvent, window, cx| {
                    if let model::EditorEvent::ExecuteCommand { command, arguments } = event {
                        let shell = this.shell.clone();
                        let command = command.clone();
                        let arguments = arguments.clone();
                        window.defer(cx, move |window, cx| {
                            shell.dispatch_document_command(&command, arguments, window, cx)
                        });
                    }
                },
            ),
        ];
        focus_handle.focus(window, cx);
        Self {
            model,
            shell,
            toolbar,
            timeline,
            transcript_pane,
            status_bar,
            contributions: Vec::new(),
            registry_generation: u64::MAX,
            focus_handle,
            frame_scheduled: false,
            region_dialog_open: false,
            _subscriptions: subscriptions,
        }
    }
    pub fn model(&self) -> &Entity<EditorModel> {
        &self.model
    }
    fn sync_contributions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = cx.global::<ContributionRegistry>().generation();
        if generation == self.registry_generation {
            return;
        }
        let panels: Vec<_> = [
            PanelLocation::Sidebar,
            PanelLocation::Overlay,
            PanelLocation::Inspector,
            PanelLocation::Transport,
        ]
        .into_iter()
        .flat_map(|location| cx.global::<ContributionRegistry>().panels(location))
        .collect();
        self.contributions.retain(|(id, generation, _, _)| {
            panels
                .iter()
                .any(|p| &p.id == id && p.generation == *generation)
        });
        for panel in panels {
            if !self
                .contributions
                .iter()
                .any(|(id, _, _, _)| id == &panel.id)
            {
                let view = (panel.factory)(self.shell.clone(), window, cx);
                self.contributions
                    .push((panel.id, panel.generation, panel.location, view));
            }
        }
        self.registry_generation = generation;
    }
    fn key_focus(&self, window: &mut Window, cx: &mut App) -> KeyFocus {
        let focused = self.transcript_pane.read(cx).focused_segment_id(window, cx);
        KeyFocus {
            target: if focused.is_some() {
                FocusTarget::TranscriptText
            } else if window.has_focused_input(cx) {
                FocusTarget::Input
            } else {
                FocusTarget::Workspace
            },
            cut_preview_active: self.model.read(cx).doc().session().overlay.is_some(),
            completions_open: false,
        }
    }
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) || window.has_active_sheet(cx) {
            return;
        }
        let stroke = keystroke_string(&event.keystroke);
        let focus = self.key_focus(window, cx);
        let selected = self
            .transcript_pane
            .read(cx)
            .focused_segment_id(window, cx)
            .and_then(|id| {
                self.model
                    .read(cx)
                    .doc()
                    .draft(&id)
                    .map(|draft| !draft.selection().is_empty())
            })
            .unwrap_or(false);
        let command = cx.global::<ContributionRegistry>().resolve_keybinding(
            &stroke,
            focus.target != FocusTarget::Workspace,
            selected,
        );
        if let Some(command) = command {
            self.shell.dispatch(&command, window, cx);
            cx.stop_propagation();
            return;
        }
        if focus.target == FocusTarget::Workspace
            && let Some(key) = audition_key_for(&stroke)
        {
            self.model.update(cx, |m, cx| {
                m.update_doc(cx, |d| d.set_audition_key(key, true))
            });
        }
        let Some(action) = ActiveKeymap::resolve(cx, &event.keystroke, focus) else {
            return;
        };
        let row = self.transcript_pane.read(cx).focused_segment_id(window, cx);
        let handled = self
            .model
            .update(cx, |m, cx| m.perform(action, row.as_deref(), cx));
        if !handled {
            match action {
                EditorAction::BackToProjects => cx.emit(EditorScreenEvent::Leave),
                EditorAction::SwapSplitPanes => {
                    self.transcript_pane.update(cx, |p, cx| p.swap_panes(cx));
                }
                _ => {}
            }
        }
        cx.stop_propagation();
    }
    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = audition_key_for(&keystroke_string(&event.keystroke)) {
            self.model.update(cx, |m, cx| {
                m.update_doc(cx, |d| d.set_audition_key(key, false))
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
        self.model
            .update(cx, |m, cx| m.update_doc(cx, |d| d.set_modifiers(modifiers)));
    }
    fn schedule_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.frame_scheduled || !self.model.read(cx).needs_frames() {
            return;
        }
        self.frame_scheduled = true;
        cx.on_next_frame(window, |this, window, cx| {
            this.frame_scheduled = false;
            this.model.update(cx, |m, cx| m.frame(cx));
            this.schedule_frame(window, cx);
        });
    }
    fn render_body(&self, window: &Window, cx: &App) -> AnyElement {
        let rows = timeline::visible_lane_rows(self.model.read(cx).doc());
        let height = window.rem_size() * (Toolbar::HEIGHT.0 + 0.5)
            + timeline::natural_height(rows, window.rem_size());
        v_flex()
            .size_full()
            .gap_2()
            .child(
                waveform_slot(height).child(
                    v_flex()
                        .size_full()
                        .gap_2()
                        .child(self.toolbar.clone())
                        .child(div().flex_1().min_h_0().child(self.timeline.clone())),
                ),
            )
            .child(div().flex_1().min_h_0().child(self.transcript_pane.clone()))
            .into_any_element()
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
        self.sync_contributions(window, cx);
        if self.model.read(cx).doc().create_region().is_none() {
            self.region_dialog_open = false;
        } else if !self.region_dialog_open && !window.has_active_dialog(cx) {
            self.region_dialog_open = true;
            let model = self.model.clone();
            window.defer(cx, move |window, cx| {
                if model.read(cx).doc().create_region().is_some() {
                    panels::dialogs::create_region::open(&model, window, cx);
                }
            });
        }
        let commands = cx.global::<ContributionRegistry>().commands();
        let buttons = commands.into_iter().filter(|c| c.toolbar).map(|command| {
            let shell = self.shell.clone();
            Button::new(SharedString::from(command.id))
                .small()
                .label(command.label)
                .on_click(move |_, window, cx| (command.run)(shell.clone(), window, cx))
        });
        let surfaces = |location| {
            self.contributions
                .iter()
                .filter(move |(_, _, l, _)| *l == location)
                .map(|(_, _, _, view)| view.clone())
                .collect::<Vec<_>>()
        };
        div()
            .id("editor-screen")
            .key_context("ExaudioEditor")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_modifiers_changed(cx.listener(Self::on_modifiers_changed))
            .size_full()
            .bg(cx.theme().background)
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
                            .child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .children(buttons)
                                    .children(surfaces(PanelLocation::Transport)),
                            )
                            .child(div().flex_1().min_h_0().child(self.render_body(window, cx)))
                            .child(self.status_bar.clone()),
                    )
                    .children(surfaces(PanelLocation::Sidebar))
                    .children(surfaces(PanelLocation::Inspector))
                    .children(
                        surfaces(PanelLocation::Overlay)
                            .into_iter()
                            .map(|view| div().absolute().inset_0().child(view)),
                    ),
            )
    }
}
