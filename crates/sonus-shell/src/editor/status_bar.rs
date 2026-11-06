//! Status bar: last status message, save detail, lane audio loading, selection, zoom and the
//! playhead position.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex};
use gpui_kit::*;
use studio_editor::edit::format::{MAX_ZOOM_FACTOR, SPEED_OPTIONS, format_speed, format_time};
use studio_editor::editor_model::SaveStatus;

use crate::editor::model::{EditorEvent, EditorModel};
use crate::editor::panels::ui::{stable_text, tabular_figures};

pub struct StatusBar {
    model: Entity<EditorModel>,
    _subscriptions: [Subscription; 2],
}

impl StatusBar {
    pub fn new(model: Entity<EditorModel>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscriptions = [
            cx.observe(&model, |_, _, cx| cx.notify()),
            // The playhead position ticks with playback.
            cx.subscribe(&model, |_, _, event: &EditorEvent, cx| {
                if *event == EditorEvent::Frame {
                    cx.notify();
                }
            }),
        ];
        Self {
            model,
            _subscriptions: subscriptions,
        }
    }
}

impl Render for StatusBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let model = self.model.read(cx);
        let doc = model.doc();
        let session = doc.session();
        let project = doc.project();

        let save = match session.save_status() {
            SaveStatus::Failed { message } => Some((
                message.map_or_else(
                    || "Save failed.".to_string(),
                    |message| format!("Save failed: {message}"),
                ),
                theme.danger,
            )),
            SaveStatus::Saved => session
                .last_saved_at
                .as_ref()
                .filter(|saved| !saved.trim().is_empty())
                .map(|saved| (format!("Saved {saved}"), theme.muted_foreground)),
            SaveStatus::Saving | SaveStatus::Dirty => None,
        };

        let load = model.audio_load();
        let audio = if load.pending > 0 {
            let total = load.pending + load.loaded + load.failed;
            Some(
                h_flex()
                    .gap_1()
                    .child(Spinner::new().xsmall())
                    .child(format!("Loading audio {}/{total}", load.loaded))
                    .into_any_element(),
            )
        } else if load.failed > 0 {
            Some(
                div()
                    .text_color(theme.warning)
                    .child(if load.failed == 1 {
                        "1 lane without audio".to_string()
                    } else {
                        format!("{} lanes without audio", load.failed)
                    })
                    .into_any_element(),
            )
        } else {
            None
        };

        let total = project.segments.len();
        let selection = session
            .selected_segment_id
            .as_deref()
            .and_then(|id| project.segments.iter().position(|segment| segment.id == id))
            .map_or_else(
                || format!("{total} segments"),
                |index| format!("Segment {} of {total}", index + 1),
            );
        let bounds = session.timeline_bounds(project);
        let visible = session.viewport.duration_seconds();
        let position = format!(
            "{} / {}",
            format_time(session.playhead_seconds),
            format_time(bounds.end_seconds)
        );

        h_flex()
            .h(rems(1.75))
            .flex_none()
            .w_full()
            .overflow_hidden()
            .px_3()
            .gap_4()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.status_bar_border)
            .bg(theme.status_bar)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.foreground)
                    .children(model.status().cloned()),
            )
            .children(save.map(|(text, color)| div().flex_none().text_color(color).child(text)))
            .children(audio)
            // Fixed slots (widest value, tabular figures): a changing value never moves the
            // labels to its left. The cluster is anchored to the bar's right edge, so the
            // values align right within their slots.
            .child(
                stable_text(selection, [format!("Segment {total} of {total}")])
                    .items_end()
                    .font_features(tabular_figures()),
            )
            .child(
                stable_text(
                    format!("Zoom {:.2}× · {visible:.1}s visible", session.zoom),
                    [format!(
                        "Zoom {:.2}× · {:.1}s visible",
                        MAX_ZOOM_FACTOR,
                        (bounds.end_seconds - bounds.start_seconds).max(visible)
                    )],
                )
                .items_end()
                .font_features(tabular_figures()),
            )
            .child(
                stable_text(
                    format_speed(session.speed),
                    SPEED_OPTIONS.iter().map(|option| format_speed(*option)),
                )
                .items_end()
                .font_features(tabular_figures()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.foreground)
                    .child(position),
            )
    }
}
