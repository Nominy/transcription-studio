//! Whole-task overview strip (shown in the toolbar): per-lane peaks and regions, the visible
//! viewport window and the playhead. Click outside the window seeks there and recentres;
//! dragging moves the viewport.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use studio_editor::timeline_model::{OverviewWaveformBlock, TimeRange};

use super::geometry::{Thumb, fraction_at};
use super::paint::{
    MarkerEdges, OverviewLane, OverviewMarkers, OverviewScene, PeakStrip, paint_overview,
};
use super::{lane_color, palette, scroll_viewport_to};
use crate::editor::model::{EditorEvent, EditorModel};

/// Generation-keyed overview data; viewport and playhead are read live every frame.
struct OverviewData {
    generation: u64,
    rows: usize,
    lanes: Rc<[OverviewLane]>,
    blocks: Arc<[OverviewWaveformBlock]>,
    block_colors: Rc<[Hsla]>,
}

pub struct OverviewView {
    model: Entity<EditorModel>,
    data: Option<OverviewData>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Device-pixel edges of the markers as last painted.
    painted_edges: Rc<Cell<Option<MarkerEdges>>>,
    /// Pointer offset inside the viewport window while dragging it.
    drag_grab: Option<f32>,
    _subscriptions: [Subscription; 2],
}

impl OverviewView {
    pub fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = [
            cx.observe(&model, |_, _, cx| cx.notify()),
            // The whole task fits in a few hundred pixels: most playback frames move neither
            // the playhead nor the viewport window by a device pixel, and skipping those keeps
            // the toolbar around the overview from re-rendering.
            cx.subscribe_in(
                &model,
                window,
                |this, _, event: &EditorEvent, window, cx| {
                    if *event == EditorEvent::Frame
                        && this.painted_edges.get() != Some(this.marker_edges(window, cx))
                    {
                        cx.notify();
                    }
                },
            ),
        ];
        Self {
            model,
            data: None,
            bounds: Rc::new(Cell::new(Bounds::default())),
            painted_edges: Rc::new(Cell::new(None)),
            drag_grab: None,
            _subscriptions: subscriptions,
        }
    }

    /// Viewport window and playhead fraction of the overview track.
    fn marker_state(&self, cx: &App) -> (Option<Thumb>, f32) {
        let doc = self.model.read(cx).doc();
        let session = doc.session();
        let range = session.timeline_bounds(doc.project());
        let playhead_fraction =
            ((session.playhead_seconds - range.start_seconds) / range.duration_seconds()) as f32;
        (
            Thumb::for_viewport(range, session.viewport),
            playhead_fraction,
        )
    }

    /// Marker edges the current state would paint at the last painted bounds.
    fn marker_edges(&self, window: &Window, cx: &App) -> MarkerEdges {
        let (thumb, playhead_fraction) = self.marker_state(cx);
        OverviewMarkers::new(self.bounds.get(), thumb, playhead_fraction)
            .device_edges(window.scale_factor())
    }

    fn refresh_data(&mut self, cx: &App) {
        let doc = self.model.read(cx).doc();
        let generation = doc.generation();
        if self
            .data
            .as_ref()
            .is_some_and(|data| data.generation == generation)
        {
            return;
        }
        let project = doc.project();
        let session = doc.session();
        let view = session.overview_waveform(project);
        let rows: Vec<&str> = project
            .lanes
            .iter()
            .filter(|lane| session.track_filter.matches_lane(&lane.id))
            .map(|lane| lane.id.as_str())
            .collect();
        let lanes: Rc<[OverviewLane]> = view
            .lane_waveform_models
            .iter()
            .filter_map(|lane| {
                Some(OverviewLane {
                    row: rows.iter().position(|id| *id == lane.lane_id)?,
                    color: lane_color(lane.color),
                    peaks: PeakStrip::overview(&lane.waveform_model)?,
                })
            })
            .collect();
        let block_colors = view
            .blocks
            .iter()
            .map(|block| lane_color(block.color))
            .collect();
        self.data = Some(OverviewData {
            generation,
            rows: view.block_lane_rows,
            lanes,
            blocks: view.blocks,
            block_colors,
        });
    }

    fn fraction(&self, position: Point<Pixels>) -> f32 {
        let bounds = self.bounds.get();
        fraction_at(
            f32::from(position.x - bounds.left()),
            f32::from(bounds.size.width),
        )
    }

    fn on_press(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let fraction = self.fraction(event.position);
        let (bounds, viewport) = {
            let doc = self.model.read(cx).doc();
            (
                doc.session().timeline_bounds(doc.project()),
                doc.session().viewport,
            )
        };
        let Some(thumb) = Thumb::for_viewport(bounds, viewport) else {
            let seconds = bounds.start_seconds + bounds.duration_seconds() * f64::from(fraction);
            self.model
                .update(cx, |model, cx| model.run(cx, |doc| doc.seek(seconds)));
            return;
        };
        if !thumb.contains(fraction) {
            let seconds = bounds.start_seconds + bounds.duration_seconds() * f64::from(fraction);
            self.model.update(cx, |model, cx| {
                model.run(cx, |doc| {
                    let outcome = doc.seek(seconds);
                    let half = doc.session().viewport.duration_seconds() / 2.0;
                    scroll_viewport_to(doc, seconds - half);
                    outcome
                });
            });
        }
        let viewport = self.model.read(cx).doc().session().viewport;
        self.drag_grab = Thumb::for_viewport(bounds, viewport).map(|thumb| fraction - thumb.left);
        cx.notify();
    }

    fn on_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(grab) = self.drag_grab else {
            return;
        };
        let fraction = self.fraction(position);
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                let bounds = doc.session().timeline_bounds(doc.project());
                scroll_viewport_to(doc, Thumb::viewport_start_at(bounds, fraction, grab));
            });
        });
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag_grab.take().is_some() {
            cx.notify();
        }
    }
}

impl Render for OverviewView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_data(cx);
        let bounds_range: TimeRange = {
            let doc = self.model.read(cx).doc();
            doc.session().timeline_bounds(doc.project())
        };
        let (thumb, playhead_fraction) = self.marker_state(cx);
        let data = self.data.as_ref().expect("overview data refreshed above");
        let scene = OverviewScene {
            bounds_range,
            rows: data.rows,
            lanes: Rc::clone(&data.lanes),
            blocks: Arc::clone(&data.blocks),
            block_colors: Rc::clone(&data.block_colors),
            thumb,
            playhead_fraction,
            palette: palette(cx),
        };
        let theme = cx.theme();
        let bounds_cell = Rc::clone(&self.bounds);
        let painted_edges = Rc::clone(&self.painted_edges);
        let dragging = self.drag_grab.is_some();
        let entity = cx.entity().downgrade();

        div()
            .id("timeline-overview")
            .relative()
            .size_full()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.35))
            .overflow_hidden()
            .cursor(if dragging {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::PointingHand
            })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_press))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_drag(cx)),
            )
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, _| {
                        painted_edges.set(Some(paint_overview(bounds, &scene, window)));
                        if dragging {
                            let move_entity = entity.clone();
                            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture {
                                    move_entity
                                        .update(cx, |this, cx| this.on_drag(event.position, cx))
                                        .ok();
                                }
                            });
                            let up_entity = entity.clone();
                            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                                if phase == DispatchPhase::Capture
                                    && event.button == MouseButton::Left
                                {
                                    up_entity.update(cx, |this, cx| this.end_drag(cx)).ok();
                                }
                            });
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
    }
}
