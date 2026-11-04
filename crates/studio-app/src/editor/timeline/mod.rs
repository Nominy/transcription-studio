//! Multi-lane waveform timeline: loop strip, ruler, lanes (header rail + waveform body) and the
//! horizontal scrollbar.
//!
//! Lane bodies are painted on canvases from the shared peak buffers of the lanes' render
//! models; lane views are rebuilt only when the doc generation, viewport or drag preview
//! change. Pointer gestures map to [`WaveformHit`]s and drive the doc's waveform, loop-strip
//! and autoscroll state machines; drags keep tracking the pointer outside the surface through
//! window-level listeners registered while a capture is active.

pub mod geometry;
pub mod overview;
mod paint;
mod rail;

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::{EditorAction, EditorDoc, PointerModifiers};
use studio_editor::linter::LintSeverity;
use studio_editor::timeline_model::{
    LaneColor, LoopStripHit, LoopStripHitTarget, TimeRange, TimelineLaneView, TimelineSegmentIndex,
    TimelineSegmentPreview, build_babel_timeline_ruler_for_viewport,
};
use studio_waveform::{WaveformHit, WaveformHitTarget};

use crate::editor::model::{EditorEvent, EditorModel};
use crate::keymap::{ActiveKeymap, IS_MAC};
use geometry::{HANDLE_HIT_PX, Thumb, fraction_at, hit_test, x_at};
use paint::{
    LaneScene, Palette, PeakStrip, RegionMark, RegionMarks, paint_lane, paint_loop_strip,
    paint_ruler, paint_scrollbar,
};
use rail::{LaneHeader, LoopToggle, rail_cell_style};

const RAIL_WIDTH: Rems = rems(11.);
const LOOP_HEIGHT: Rems = rems(1.375);
const RULER_HEIGHT: Rems = rems(1.375);
const SCROLLBAR_HEIGHT: Rems = rems(0.875);
const COLLAPSED_LANE_HEIGHT: Rems = rems(2.5);
/// Fits the rail header (title and button row) with its padding.
const MIN_LANE_HEIGHT: Rems = rems(3.5);
/// Height of an expanded lane in the stacked layouts. Two lanes reproduce the former fixed
/// 22.5rem waveform card; as in Babel's browser editor, hiding a speaker shrinks the card
/// instead of stretching the remaining lane over the freed space.
const LANE_HEIGHT: Rems = rems(7.8125);
/// Pixels per wheel "line" (Windows reports three lines per notch).
const WHEEL_LINE_PIXELS: f32 = 20.0;
const TOOLTIP_WIDTH: Rems = rems(20.);

/// Visible lanes as `(expanded, collapsed)` counts (at least one expanded row).
pub fn visible_lane_rows(doc: &EditorDoc) -> (usize, usize) {
    let session = doc.session();
    let (mut expanded, mut collapsed) = (0_usize, 0_usize);
    for lane in doc
        .project()
        .lanes
        .iter()
        .filter(|lane| session.track_filter.matches_lane(&lane.id))
    {
        if session.collapsed_lane_ids.contains(&lane.id) {
            collapsed += 1;
        } else {
            expanded += 1;
        }
    }
    if expanded + collapsed == 0 {
        expanded = 1;
    }
    (expanded, collapsed)
}

/// Height the timeline needs to show `rows` (see [`visible_lane_rows`]) at [`LANE_HEIGHT`]
/// (collapsed lanes at [`COLLAPSED_LANE_HEIGHT`]) under the loop strip and ruler, above the
/// scrollbar.
pub fn natural_height((expanded, collapsed): (usize, usize), rem_size: Pixels) -> Pixels {
    let rows = LOOP_HEIGHT.0
        + RULER_HEIGHT.0
        + SCROLLBAR_HEIGHT.0
        + expanded as f32 * LANE_HEIGHT.0
        + collapsed as f32 * COLLAPSED_LANE_HEIGHT.0;
    // Outer border (top and bottom), the loop row's bottom border and one per lane.
    rem_size * rows + px((3 + expanded + collapsed) as f32)
}

/// Lane accent as a gpui color.
fn lane_color(color: LaneColor) -> Hsla {
    rgb(color.rgb()).into()
}

fn palette(cx: &App) -> Palette {
    let theme = cx.theme();
    Palette {
        text: theme.foreground.opacity(0.9),
        grid: theme.border,
        unplayed: theme.muted_foreground.opacity(0.45),
        playhead: theme.foreground,
        hover: theme.muted_foreground.opacity(0.6),
        accent: theme.info,
        danger: theme.danger,
        warning: theme.warning,
        info: theme.info,
        popover: theme.popover,
        muted: theme.muted_foreground,
    }
}

fn pointer_modifiers(modifiers: &Modifiers, cx: &App) -> PointerModifiers {
    PointerModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        secondary: modifiers.secondary(cx),
    }
}

/// `"{text} ({shortcut})"` with the action's first binding from the user's keymap.
pub(crate) fn shortcut_hint(cx: &App, text: &str, action: EditorAction) -> SharedString {
    match cx.global::<ActiveKeymap>().0.labels(action, IS_MAC).first() {
        Some(label) => format!("{text} ({label})").into(),
        None => SharedString::from(text.to_string()),
    }
}

/// Moves the viewport so it starts at `start_seconds` (clamped to the timeline bounds).
fn scroll_viewport_to(doc: &mut EditorDoc, start_seconds: f64) {
    let bounds = doc.session().timeline_bounds(doc.project());
    let session = doc.session_mut();
    let delta = start_seconds - session.viewport.start_seconds;
    session.pan_viewport_by_time_delta_with_bounds(bounds, delta);
}

#[derive(Clone, PartialEq)]
struct LanesKey {
    generation: u64,
    viewport: [u64; 2],
    preview: Option<TimelineSegmentPreview>,
}

/// Pointer capture of an in-progress drag (the pointer may leave the surface).
#[derive(Clone)]
enum Capture {
    Lane(String),
    LoopStrip,
    Scrollbar { grab: f32 },
}

type SurfaceBounds = Rc<Cell<Bounds<Pixels>>>;

pub struct TimelineView {
    model: Entity<EditorModel>,
    focus_handle: FocusHandle,
    segment_index: Option<(u64, TimelineSegmentIndex)>,
    lanes: Option<(LanesKey, Rc<[TimelineLaneView]>)>,
    marks: Option<(u64, RegionMarks)>,
    lane_bounds: HashMap<String, SurfaceBounds>,
    loop_bounds: SurfaceBounds,
    scrollbar_bounds: SurfaceBounds,
    capture: Option<Capture>,
    lane_headers: HashMap<String, Entity<LaneHeader>>,
    loop_toggle: Entity<LoopToggle>,
    /// Interface zoom of the last render; scales hit tolerances (see [`crate::zoom`]).
    scale: f32,
    _subscriptions: Vec<Subscription>,
}

impl TimelineView {
    pub fn new(model: Entity<EditorModel>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let loop_toggle = cx.new(|cx| LoopToggle::new(model.clone(), cx));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&model, |_, _, event: &EditorEvent, cx| {
                if *event == EditorEvent::Frame {
                    cx.notify();
                }
            }),
        ];
        Self {
            model,
            focus_handle: cx.focus_handle(),
            segment_index: None,
            lanes: None,
            marks: None,
            lane_bounds: HashMap::new(),
            loop_bounds: SurfaceBounds::default(),
            scrollbar_bounds: SurfaceBounds::default(),
            capture: None,
            lane_headers: HashMap::new(),
            loop_toggle,
            scale: 1.0,
            _subscriptions: subscriptions,
        }
    }

    /// Lane views for the current viewport, rebuilt only when the doc generation, viewport or
    /// drag preview change.
    fn lanes(&mut self, cx: &App) -> Rc<[TimelineLaneView]> {
        let doc = self.model.read(cx).doc();
        let generation = doc.generation();
        let session = doc.session();
        let key = LanesKey {
            generation,
            viewport: [
                session.viewport.start_seconds.to_bits(),
                session.viewport.end_seconds.to_bits(),
            ],
            preview: doc.timeline_preview(),
        };
        if let Some((cached, lanes)) = &self.lanes
            && *cached == key
        {
            return Rc::clone(lanes);
        }
        let project = doc.project();
        if self
            .segment_index
            .as_ref()
            .is_none_or(|(indexed, _)| *indexed != generation)
        {
            self.segment_index = Some((generation, TimelineSegmentIndex::for_project(project)));
        }
        let index = &self
            .segment_index
            .as_ref()
            .expect("segment index built above")
            .1;
        let lanes: Rc<[TimelineLaneView]> = session
            .timeline_lanes_with_segment_index_bounds_and_preview(
                project,
                index,
                session.timeline_bounds(project),
                None,
                key.preview.as_ref(),
            )
            .into();
        self.lanes = Some((key, Rc::clone(&lanes)));
        lanes
    }

    /// Lint / diff decorations per visible row, rebuilt per doc generation.
    fn marks(&mut self, cx: &App) -> RegionMarks {
        let doc = self.model.read(cx).doc();
        let generation = doc.generation();
        if let Some((cached, marks)) = &self.marks
            && *cached == generation
        {
            return Rc::clone(marks);
        }
        let diff_view = doc.session().diff_view;
        let rows = doc.transcript_rows();
        let mut marks = HashMap::new();
        for row in rows.iter() {
            let (severity, _) = doc.row_lint(row);
            let lint = match severity {
                LintSeverity::Error => Some(severity),
                LintSeverity::Warning if !row.has_asserted_warning => Some(severity),
                _ => None,
            };
            let changed = diff_view && row.diff_marker.is_some();
            if lint.is_some() || changed {
                marks.insert(row.segment_id.clone(), RegionMark { lint, changed });
            }
        }
        let marks = Rc::new(marks);
        self.marks = Some((generation, Rc::clone(&marks)));
        marks
    }

    fn lane_bounds(&mut self, lane_id: &str) -> SurfaceBounds {
        Rc::clone(self.lane_bounds.entry(lane_id.to_string()).or_default())
    }

    fn lane_header(&mut self, lane_id: &str, cx: &mut Context<Self>) -> Entity<LaneHeader> {
        if let Some(header) = self.lane_headers.get(lane_id) {
            return header.clone();
        }
        let model = self.model.clone();
        let header = cx.new(|cx| LaneHeader::new(model, lane_id.to_string(), cx));
        self.lane_headers
            .insert(lane_id.to_string(), header.clone());
        header
    }

    // ---- lane pointer ---------------------------------------------------------------------

    fn lane_hit(
        &mut self,
        lane_id: &str,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<WaveformHit> {
        let bounds = self.lane_bounds.get(lane_id)?.get();
        let lanes = self.lanes(cx);
        let lane = lanes.iter().find(|lane| lane.lane_id == lane_id)?;
        let viewport = self.model.read(cx).doc().session().viewport;
        Some(hit_test(
            &lane.blocks,
            viewport,
            f32::from(position.x - bounds.left()),
            f32::from(bounds.size.width),
            HANDLE_HIT_PX * self.scale,
        ))
    }

    fn hover_lane(&mut self, lane_id: &str, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(hit) = self.lane_hit(lane_id, position, cx) else {
            return;
        };
        let unchanged = self
            .model
            .read(cx)
            .doc()
            .pointer()
            .waveform_hover
            .as_ref()
            .is_some_and(|hover| hover.lane_id == lane_id && hover.hit == hit);
        if unchanged {
            return;
        }
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.hover_waveform(lane_id, hit));
        });
    }

    fn lane_moved(&mut self, lane_id: &str, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.capture.is_some() {
            self.captured_move(event.position, cx);
        } else {
            self.hover_lane(lane_id, event.position, cx);
        }
    }

    fn lane_pressed(
        &mut self,
        lane_id: &str,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let Some(hit) = self.lane_hit(lane_id, event.position, cx) else {
            return;
        };
        let modifiers = pointer_modifiers(&event.modifiers, cx);
        let button = event.button;
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.set_modifiers(modifiers);
                doc.hover_waveform(lane_id, hit);
                if button == MouseButton::Middle {
                    doc.press_autoscroll(lane_id);
                }
            });
            if button == MouseButton::Left {
                model.edit(cx, |doc| doc.press_waveform(lane_id));
            }
        });
        if button == MouseButton::Left {
            self.capture = Some(Capture::Lane(lane_id.to_string()));
        }
        cx.notify();
    }

    fn lane_left(&mut self, lane_id: &str, cx: &mut Context<Self>) {
        let pointer = self.model.read(cx).doc().pointer();
        if self.capture.is_some() || pointer.autoscroll.is_some() {
            return;
        }
        if pointer
            .waveform_hover
            .as_ref()
            .is_some_and(|hover| hover.lane_id == lane_id)
        {
            self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| doc.leave_waveform(lane_id));
            });
        }
    }

    // ---- loop strip -----------------------------------------------------------------------

    fn loop_hit(&self, position: Point<Pixels>, cx: &App) -> LoopStripHit {
        let bounds = self.loop_bounds.get();
        let fraction = fraction_at(
            f32::from(position.x - bounds.left()),
            f32::from(bounds.size.width),
        );
        let doc = self.model.read(cx).doc();
        doc.session().loop_strip_hit(doc.project(), fraction)
    }

    fn hover_loop(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let hit = self.loop_hit(position, cx);
        if self.model.read(cx).doc().pointer().loop_hover == Some(hit) {
            return;
        }
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.hover_loop_strip(hit));
        });
    }

    fn loop_pressed(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let hit = self.loop_hit(event.position, cx);
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.hover_loop_strip(hit);
                doc.press_loop_strip();
            });
        });
        self.capture = Some(Capture::LoopStrip);
        cx.notify();
    }

    // ---- scrollbar ------------------------------------------------------------------------

    fn scrollbar_fraction(&self, position: Point<Pixels>) -> f32 {
        let bounds = self.scrollbar_bounds.get();
        fraction_at(
            f32::from(position.x - bounds.left()),
            f32::from(bounds.size.width),
        )
    }

    fn scrollbar_pressed(
        &mut self,
        event: &MouseDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.stop_propagation();
        let doc = self.model.read(cx).doc();
        let Some(thumb) = Thumb::for_viewport(
            doc.session().timeline_bounds(doc.project()),
            doc.session().viewport,
        ) else {
            return;
        };
        let grab = thumb.grab_offset(self.scrollbar_fraction(event.position));
        self.capture = Some(Capture::Scrollbar { grab });
        self.drag_scrollbar(event.position, grab, cx);
    }

    fn drag_scrollbar(&mut self, position: Point<Pixels>, grab: f32, cx: &mut Context<Self>) {
        let fraction = self.scrollbar_fraction(position);
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                let bounds = doc.session().timeline_bounds(doc.project());
                scroll_viewport_to(doc, Thumb::viewport_start_at(bounds, fraction, grab));
            });
        });
    }

    // ---- captured pointer -----------------------------------------------------------------

    fn captured_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        match self.capture.clone() {
            Some(Capture::Lane(lane_id)) => self.hover_lane(&lane_id, position, cx),
            Some(Capture::LoopStrip) => self.hover_loop(position, cx),
            Some(Capture::Scrollbar { grab }) => self.drag_scrollbar(position, grab, cx),
            None => {
                let autoscroll_lane = self
                    .model
                    .read(cx)
                    .doc()
                    .pointer()
                    .autoscroll
                    .as_ref()
                    .map(|autoscroll| autoscroll.lane_id.clone());
                if let Some(lane_id) = autoscroll_lane {
                    self.hover_lane(&lane_id, position, cx);
                }
            }
        }
    }

    /// Element-level fallback for a release that lands before the window capture listeners of
    /// the next frame are registered (very quick clicks).
    fn release_left(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture.is_some() {
            self.captured_release(MouseButton::Left, cx);
        }
    }

    fn release_middle(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.captured_release(MouseButton::Middle, cx);
    }

    fn captured_release(&mut self, button: MouseButton, cx: &mut Context<Self>) {
        match button {
            MouseButton::Left => match self.capture.take() {
                Some(Capture::Lane(lane_id)) => self.model.update(cx, |model, cx| {
                    model.edit(cx, |doc| doc.release_waveform(&lane_id));
                }),
                Some(Capture::LoopStrip) => self.model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| doc.release_loop_strip());
                }),
                Some(Capture::Scrollbar { .. }) | None => {}
            },
            MouseButton::Middle => self.model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| doc.release_autoscroll());
            }),
            _ => {}
        }
        cx.notify();
    }

    // ---- wheel ------------------------------------------------------------------------------

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(WHEEL_LINE_PIXELS));
        // Browser convention (positive = down/right) as the doc expects; Windows reports
        // shift+wheel as a horizontal delta.
        let (mut dx, mut dy) = (-f32::from(delta.x), -f32::from(delta.y));
        if event.modifiers.shift && dy == 0.0 {
            (dx, dy) = (0.0, dx);
        }
        let modifiers = pointer_modifiers(&event.modifiers, cx);
        let handled = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.set_modifiers(modifiers);
                doc.timeline_wheel(dx, dy)
            })
        });
        if handled {
            cx.stop_propagation();
        }
    }

    fn render_lane(
        &mut self,
        index: usize,
        lane: &TimelineLaneView,
        frame: &Frame,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let header = self
            .lane_header(&lane.lane_id, cx)
            .cached(rail_cell_style())
            .into_any_element();
        let bounds_cell = self.lane_bounds(&lane.lane_id);
        let lane_bounds = bounds_cell.get();
        let lane_id: SharedString = lane.lane_id.clone().into();
        let hover = frame
            .hover
            .as_ref()
            .filter(|(hover_lane, _)| *hover_lane == lane.lane_id)
            .map(|(_, hit)| hit);
        let dragging_here = frame.capture_lane.as_deref() == Some(lane.lane_id.as_str());
        let scene = LaneScene {
            viewport: frame.viewport,
            playhead_seconds: frame.playhead_seconds,
            color: lane_color(lane.color),
            collapsed: lane.is_collapsed,
            gain: lane.vertical_zoom,
            peaks: lane.waveform_model.as_ref().and_then(PeakStrip::timeline),
            blocks: Arc::clone(&lane.blocks),
            selected: frame.selected.clone(),
            hovered: hover.and_then(|hit| hit.region_id.clone()),
            marks: Rc::clone(&frame.marks),
            overlay: lane.overlay.map(|overlay| TimeRange {
                start_seconds: overlay.start_seconds,
                end_seconds: overlay.end_seconds,
            }),
            loop_range: frame.loop_range,
            hover_seconds: hover.map(|hit| hit.time_seconds),
            autoscroll_anchor: frame
                .autoscroll
                .as_ref()
                .filter(|(autoscroll_lane, _)| *autoscroll_lane == lane.lane_id)
                .map(|(_, anchor)| *anchor),
            magnifier_seconds: frame
                .magnifier
                .as_ref()
                .filter(|(magnifier_lane, _)| *magnifier_lane == lane.lane_id)
                .map(|(_, seconds)| *seconds),
            palette: frame.palette,
        };
        let cursor =
            if frame.autoscroll.is_some() {
                CursorStyle::ClosedHand
            } else if dragging_here && frame.magnifier.is_some() {
                CursorStyle::ResizeLeftRight
            } else {
                match hover.map(|hit| hit.target) {
                    Some(
                        WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle,
                    ) if !frame.read_only => CursorStyle::ResizeLeftRight,
                    Some(WaveformHitTarget::Waveform) if !frame.read_only => CursorStyle::Crosshair,
                    _ => CursorStyle::Arrow,
                }
            };

        let (background, muted_foreground, border) = {
            let theme = cx.theme();
            (theme.background, theme.muted_foreground, theme.border)
        };
        let mut body = div()
            .id(("lane-body", index))
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_hidden()
            .bg(background)
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, cx| paint_lane(bounds, &scene, window, cx),
                )
                .absolute()
                .inset_0()
                .size_full(),
            );
        if !lane.is_collapsed {
            body = body
                .cursor(cursor)
                .on_mouse_move(cx.listener({
                    let lane_id = lane_id.clone();
                    move |this, event: &MouseMoveEvent, _, cx| this.lane_moved(&lane_id, event, cx)
                }))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener({
                        let lane_id = lane_id.clone();
                        move |this, event: &MouseDownEvent, window, cx| {
                            this.lane_pressed(&lane_id, event, window, cx)
                        }
                    }),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener({
                        let lane_id = lane_id.clone();
                        move |this, event: &MouseDownEvent, window, cx| {
                            this.lane_pressed(&lane_id, event, window, cx)
                        }
                    }),
                )
                .on_mouse_up(MouseButton::Left, cx.listener(Self::release_left))
                .on_mouse_up(MouseButton::Middle, cx.listener(Self::release_middle))
                .on_hover(cx.listener({
                    let lane_id = lane_id.clone();
                    move |this, hovered: &bool, _, cx| {
                        if !*hovered {
                            this.lane_left(&lane_id, cx);
                        }
                    }
                }));
            if lane.waveform_model.is_none() && frame.audio_pending {
                body = body.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(muted_foreground)
                        .child("Loading audio…"),
                );
            }
            if let Some(hit) = hover.filter(|_| frame.capture_lane.is_none()) {
                body = body.children(self.region_overlays(lane, hit, frame, lane_bounds, cx));
            }
        }

        let row = h_flex()
            .w_full()
            .border_b_1()
            .border_color(border)
            .child(header)
            .child(body);
        if lane.is_collapsed {
            row.h(COLLAPSED_LANE_HEIGHT).flex_none().into_any_element()
        } else {
            row.flex_1().min_h(MIN_LANE_HEIGHT).into_any_element()
        }
    }

    /// Text tooltip for the hovered region and the edit affordance of annotation regions.
    fn region_overlays(
        &self,
        lane: &TimelineLaneView,
        hit: &WaveformHit,
        frame: &Frame,
        bounds: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(block) = hit
            .region_id
            .as_deref()
            .and_then(|id| lane.blocks.iter().find(|block| block.segment_id == id))
        else {
            return Vec::new();
        };
        let theme = cx.theme();
        let scale = self.scale;
        let width = f32::from(bounds.size.width);
        let mut overlays = Vec::new();
        let pointer_x = x_at(frame.viewport, hit.time_seconds, width);
        let text = block.tooltip.trim();
        if hit.target == WaveformHitTarget::RegionBody && !text.is_empty() && width > 0.0 {
            let origin = point(
                bounds.left() + px(pointer_x + 14.0 * scale),
                bounds.top() + px(24.0 * scale),
            );
            overlays.push(
                deferred(
                    anchored()
                        .position(origin)
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .w(TOOLTIP_WIDTH)
                                .px_2()
                                .py_1()
                                .rounded(theme.radius)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.popover)
                                .shadow_md()
                                .text_xs()
                                .text_color(theme.popover_foreground)
                                .line_clamp(6)
                                .child(text.to_string()),
                        ),
                )
                .with_priority(1)
                .into_any_element(),
            );
        }
        if !block.is_transcription && !frame.read_only && width > 0.0 {
            let right = x_at(frame.viewport, block.end_seconds, width).min(width);
            let left = x_at(frame.viewport, block.start_seconds, width).max(0.0);
            if right - left > 32.0 * scale {
                let segment_id = block.segment_id.clone();
                overlays.push(
                    div()
                        .absolute()
                        .left(px(right - 28.0 * scale))
                        .top(px(6.0 * scale))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new("edit-annotation")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Pencil)
                                .tooltip("Edit annotation")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.model.update(cx, |model, cx| {
                                        model.edit(cx, |doc| doc.open_edit_annotation(&segment_id));
                                    });
                                })),
                        )
                        .into_any_element(),
                );
            }
        }
        overlays
    }

    fn render_loop_row(&self, frame: &Frame, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let strip = frame.loop_strip;
        let viewport = frame.viewport;
        let palette = frame.palette;
        let bounds_cell = Rc::clone(&self.loop_bounds);
        let cursor = match (frame.loop_drag, frame.loop_hover) {
            (true, Some(LoopStripHitTarget::RangeBody)) => CursorStyle::ClosedHand,
            (_, Some(LoopStripHitTarget::StartHandle | LoopStripHitTarget::EndHandle)) => {
                CursorStyle::ResizeLeftRight
            }
            (_, Some(LoopStripHitTarget::RangeBody)) => CursorStyle::OpenHand,
            _ => CursorStyle::Crosshair,
        };
        let toggle = self.loop_toggle.clone().cached(rail_cell_style());
        h_flex()
            .h(LOOP_HEIGHT)
            .flex_none()
            .border_b_1()
            .border_color(theme.border)
            .child(toggle)
            .child(
                div()
                    .id("loop-strip")
                    .relative()
                    .flex_1()
                    .h_full()
                    .bg(theme.muted.opacity(0.25))
                    .cursor(cursor)
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        if this.capture.is_some() {
                            this.captured_move(event.position, cx);
                        } else {
                            this.hover_loop(event.position, cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::loop_pressed))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::release_left))
                    .child(
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                paint_loop_strip(bounds, &strip, viewport, palette, window)
                            },
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    fn render_ruler_row(&self, frame: &Frame, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let viewport = frame.viewport;
        let min_px_per_sec = frame.min_px_per_sec;
        let playhead = frame.playhead_seconds;
        let palette = frame.palette;
        h_flex()
            .h(RULER_HEIGHT)
            .flex_none()
            .child(
                div()
                    .w(RAIL_WIDTH)
                    .h_full()
                    .flex_none()
                    .bg(theme.muted.opacity(0.4))
                    .border_r_1()
                    .border_b_1()
                    .border_color(theme.border),
            )
            .child(
                canvas(
                    move |bounds, _, _| {
                        build_babel_timeline_ruler_for_viewport(
                            viewport,
                            f64::from(f32::from(bounds.size.width)),
                            min_px_per_sec,
                        )
                    },
                    move |bounds, ruler, window, cx| {
                        let playhead_x = x_at(viewport, playhead, f32::from(bounds.size.width));
                        paint_ruler(bounds, &ruler, playhead_x, palette, window, cx);
                    },
                )
                .flex_1()
                .h_full(),
            )
            .into_any_element()
    }

    fn render_scrollbar_row(&self, frame: &Frame, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let thumb = frame.thumb;
        let active = matches!(self.capture, Some(Capture::Scrollbar { .. }));
        let palette = frame.palette;
        let bounds_cell = Rc::clone(&self.scrollbar_bounds);
        h_flex()
            .h(SCROLLBAR_HEIGHT)
            .flex_none()
            .child(
                div()
                    .w(RAIL_WIDTH)
                    .h_full()
                    .flex_none()
                    .bg(theme.muted.opacity(0.4))
                    .border_r_1()
                    .border_color(theme.border),
            )
            .child(
                div()
                    .id("timeline-scrollbar")
                    .relative()
                    .flex_1()
                    .h_full()
                    .when(thumb.is_some(), |this| {
                        this.on_mouse_down(MouseButton::Left, cx.listener(Self::scrollbar_pressed))
                            .on_mouse_up(MouseButton::Left, cx.listener(Self::release_left))
                            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                                if this.capture.is_some() {
                                    this.captured_move(event.position, cx);
                                }
                            }))
                    })
                    .child(
                        canvas(
                            move |bounds, _, _| bounds_cell.set(bounds),
                            move |bounds, _, window, _| {
                                if let Some(thumb) = thumb {
                                    paint_scrollbar(bounds, thumb, active, palette, window);
                                }
                            },
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    ),
            )
            .into_any_element()
    }

    /// Window-level listeners that keep a drag (or autoscroll) tracking the pointer outside
    /// the surface it started on.
    fn render_capture(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let autoscrolling = self.model.read(cx).doc().pointer().autoscroll.is_some();
        if self.capture.is_none() && !autoscrolling {
            return None;
        }
        let entity = cx.entity().downgrade();
        Some(
            canvas(
                |_, _, _| (),
                move |_, _, window, _| {
                    let move_entity = entity.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            move_entity
                                .update(cx, |this, cx| this.captured_move(event.position, cx))
                                .ok();
                        }
                    });
                    let up_entity = entity.clone();
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Capture {
                            up_entity
                                .update(cx, |this, cx| this.captured_release(event.button, cx))
                                .ok();
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
            .size_full()
            .into_any_element(),
        )
    }
}

/// Per-frame values shared by all timeline rows.
struct Frame {
    viewport: TimeRange,
    playhead_seconds: f64,
    min_px_per_sec: f64,
    selected: Option<String>,
    hover: Option<(String, WaveformHit)>,
    autoscroll: Option<(String, f32)>,
    magnifier: Option<(String, f64)>,
    capture_lane: Option<String>,
    loop_strip: studio_editor::timeline_model::LoopStripView,
    loop_range: Option<TimeRange>,
    loop_hover: Option<LoopStripHitTarget>,
    loop_drag: bool,
    thumb: Option<Thumb>,
    marks: RegionMarks,
    read_only: bool,
    audio_pending: bool,
    palette: Palette,
}

impl Frame {
    fn new(view: &mut TimelineView, cx: &App) -> Self {
        let marks = view.marks(cx);
        let model = view.model.read(cx);
        let doc = model.doc();
        let session = doc.session();
        let project = doc.project();
        let pointer = doc.pointer();
        let hover = pointer
            .waveform_hover
            .as_ref()
            .map(|hover| (hover.lane_id.clone(), hover.hit.clone()));
        let magnifier = pointer
            .waveform_drag
            .as_ref()
            .filter(|drag| {
                matches!(
                    drag.target,
                    WaveformHitTarget::RegionStartHandle | WaveformHitTarget::RegionEndHandle
                )
            })
            .and_then(|drag| {
                hover
                    .as_ref()
                    .filter(|(lane, _)| *lane == drag.lane_id)
                    .map(|(lane, hit)| (lane.clone(), hit.time_seconds))
            });
        let capture_lane = match &view.capture {
            Some(Capture::Lane(lane_id)) => Some(lane_id.clone()),
            _ => None,
        };
        Self {
            viewport: session.viewport,
            playhead_seconds: session.playhead_seconds,
            min_px_per_sec: session.timeline_min_px_per_sec(),
            selected: session.selected_segment_id.clone(),
            autoscroll: pointer
                .autoscroll
                .as_ref()
                .map(|autoscroll| (autoscroll.lane_id.clone(), autoscroll.anchor_fraction)),
            magnifier,
            capture_lane,
            hover,
            loop_strip: session.loop_strip(project),
            loop_range: session.stored_loop_range.filter(|_| session.loop_enabled),
            loop_hover: pointer.loop_hover.map(|hit| hit.target),
            loop_drag: pointer.loop_drag.is_some(),
            thumb: Thumb::for_viewport(session.timeline_bounds(project), session.viewport),
            marks,
            read_only: doc.is_read_only(),
            audio_pending: model.audio_load().pending > 0,
            palette: palette(cx),
        }
    }
}

impl Focusable for TimelineView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TimelineView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.scale = crate::zoom::ui_scale(window);
        let lanes = self.lanes(cx);
        let frame = Frame::new(self, cx);
        let lane_rows: Vec<AnyElement> = lanes
            .iter()
            .enumerate()
            .map(|(index, lane)| self.render_lane(index, lane, &frame, cx))
            .collect();
        let loop_row = self.render_loop_row(&frame, cx);
        let ruler_row = self.render_ruler_row(&frame, cx);
        let scrollbar_row = self.render_scrollbar_row(&frame, cx);
        let capture = self.render_capture(cx);
        let theme = cx.theme();

        v_flex()
            .id("timeline")
            .track_focus(&self.focus_handle)
            // Any press in the timeline takes keyboard focus from a row's text editor (Babel's
            // waveform blurs the editor), so timeline hotkeys (S, Alt+Shift+S, ...) apply.
            .capture_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                this.focus_handle.focus(window, cx);
            }))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .on_scroll(cx.listener(Self::on_scroll))
            .child(loop_row)
            .child(ruler_row)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .children(lane_rows),
            )
            .child(scrollbar_row)
            .children(capture)
    }
}
