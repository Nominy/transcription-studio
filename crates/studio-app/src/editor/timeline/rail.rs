//! The timeline's left rail controls: one header per lane (label, mute, collapse, vertical
//! zoom) and the loop toggle. Each is its own view, mounted cached in the timeline rows, so
//! the playback frames that repaint the timeline surfaces reuse them; they re-render on doc
//! changes only.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderScale, SliderState};
use gpui_kit::component::{
    ActiveTheme, Disableable as _, Selectable as _, Sizable as _, h_flex, v_flex,
};
use gpui_kit::*;
use studio_editor::edit::EditorAction;
use studio_editor::timeline_model::TimelineLaneHeader;

use super::{RAIL_WIDTH, lane_color, shortcut_hint};
use crate::editor::model::EditorModel;

/// Cache style of a rail cell: the fixed rail width, the full row height.
pub(super) fn rail_cell_style() -> StyleRefinement {
    StyleRefinement::default()
        .w(RAIL_WIDTH)
        .h_full()
        .flex_none()
}

pub(super) struct LaneHeader {
    model: Entity<EditorModel>,
    lane_id: String,
    zoom: Entity<SliderState>,
    _subscriptions: [Subscription; 2],
}

impl LaneHeader {
    pub(super) fn new(model: Entity<EditorModel>, lane_id: String, cx: &mut Context<Self>) -> Self {
        let vertical_zoom =
            Self::header_of(&model, &lane_id, cx).map_or(1.0, |header| header.vertical_zoom);
        let zoom = cx.new(|_| {
            SliderState::new()
                .min(0.3)
                .max(1000.0)
                .scale(SliderScale::Logarithmic)
                .step(0.01)
                .default_value(vertical_zoom)
        });
        let subscriptions = [
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&zoom, |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    let zoom = value.end();
                    let lane = this.lane_id.clone();
                    this.model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| doc.set_lane_vertical_zoom(&lane, zoom));
                    });
                }
            }),
        ];
        Self {
            model,
            lane_id,
            zoom,
            _subscriptions: subscriptions,
        }
    }

    fn header_of(
        model: &Entity<EditorModel>,
        lane_id: &str,
        cx: &App,
    ) -> Option<TimelineLaneHeader> {
        let doc = model.read(cx).doc();
        doc.session()
            .timeline_lane_headers(doc.project())
            .into_iter()
            .find(|header| header.lane_id == lane_id)
    }

    fn sync_zoom(&self, vertical_zoom: f32, window: &mut Window, cx: &mut App) {
        let current = self.zoom.read(cx).value().end();
        if (current - vertical_zoom).abs() > vertical_zoom.abs() * 0.001 {
            self.zoom
                .update(cx, |slider, cx| slider.set_value(vertical_zoom, window, cx));
        }
    }
}

impl Render for LaneHeader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(lane) = Self::header_of(&self.model, &self.lane_id, cx) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let color = lane_color(lane.color);
        let lane_id: SharedString = lane.lane_id.clone().into();
        let collapse = Button::new(SharedString::from(format!("lane-collapse-{lane_id}")))
            .ghost()
            .xsmall()
            .icon(if lane.is_collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            })
            .tooltip(if lane.is_collapsed {
                "Expand lane"
            } else {
                "Collapse lane"
            })
            .on_click(cx.listener({
                let lane_id = lane_id.clone();
                move |this, _, _, cx| {
                    this.model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| doc.toggle_lane_collapse(&lane_id));
                    });
                }
            }));
        let mute = Button::new(SharedString::from(format!("lane-mute-{lane_id}")))
            .ghost()
            .xsmall()
            .selected(lane.is_muted)
            .icon(if lane.is_muted {
                IconName::VolumeX
            } else {
                IconName::Volume2
            })
            .tooltip(if lane.is_muted {
                "Unmute lane"
            } else {
                "Mute lane"
            })
            .on_click(cx.listener({
                let lane_id = lane_id.clone();
                move |this, _, _, cx| {
                    this.model.update(cx, |model, cx| {
                        model.run(cx, |doc| doc.toggle_lane_mute(&lane_id));
                    });
                }
            }));
        let title = h_flex()
            .gap_1p5()
            .min_w_0()
            .child(div().size_2().flex_none().rounded_full().bg(color))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .truncate()
                    .child(lane.label.clone()),
            );
        let header = if lane.is_collapsed {
            h_flex()
                .flex_1()
                .min_w_0()
                .px_2()
                .gap_1()
                .justify_between()
                .child(title)
                .child(h_flex().child(mute).child(collapse))
                .into_any_element()
        } else {
            let zoom_label = format!("{:.1}×", lane.vertical_zoom);
            let zoom_text = theme.muted_foreground;
            self.sync_zoom(lane.vertical_zoom, window, cx);
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .p_2()
                        .justify_between()
                        .child(title)
                        .child(
                            h_flex()
                                .gap_0p5()
                                .child(mute)
                                .child(collapse)
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .id(SharedString::from(format!("lane-zoom-{lane_id}")))
                                        .text_xs()
                                        .text_color(zoom_text)
                                        .child(zoom_label)
                                        .tooltip(|window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(
                                                "Waveform height",
                                            )
                                            .build(window, cx)
                                        }),
                                ),
                        ),
                )
                .child(
                    div()
                        .w(rems(1.125))
                        .h_full()
                        .py_2()
                        .flex()
                        .justify_center()
                        .child(Slider::new(&self.zoom).vertical().h_full()),
                )
                .into_any_element()
        };
        let theme = cx.theme();
        h_flex()
            .w(RAIL_WIDTH)
            .flex_none()
            .h_full()
            .bg(theme.muted.opacity(0.4))
            .border_r_1()
            .border_color(theme.border)
            .child(div().w(rems(0.1875)).h_full().flex_none().bg(color))
            .child(header)
            .into_any_element()
    }
}

/// Loop on/off button in the rail cell of the loop strip row.
pub(super) struct LoopToggle {
    model: Entity<EditorModel>,
    _subscription: Subscription,
}

impl LoopToggle {
    pub(super) fn new(model: Entity<EditorModel>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&model, |_, _, cx| cx.notify());
        Self {
            model,
            _subscription: subscription,
        }
    }
}

impl Render for LoopToggle {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let strip = {
            let doc = self.model.read(cx).doc();
            doc.session().loop_strip(doc.project())
        };
        let toggle = Button::new("loop-toggle")
            .ghost()
            .xsmall()
            .icon(IconName::Repeat)
            .label(if strip.is_enabled { "Loop on" } else { "Loop" })
            .selected(strip.is_enabled)
            .disabled(!strip.has_range)
            .tooltip(if strip.has_range {
                shortcut_hint(cx, "Toggle loop", EditorAction::ToggleLoop)
            } else {
                "Drag on the loop strip to set a loop range".into()
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.model
                    .update(cx, |model, cx| model.run(cx, |doc| doc.toggle_loop()));
            }));
        let theme = cx.theme();
        h_flex()
            .w(RAIL_WIDTH)
            .h_full()
            .flex_none()
            .px_1()
            .bg(theme.muted.opacity(0.4))
            .border_r_1()
            .border_color(theme.border)
            .child(toggle)
    }
}
