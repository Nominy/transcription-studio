//! Rendering of one transcript row: id, track, times, reviewer, text, lint and actions cells,
//! plus read-only diff rows.

use std::ops::Range;
use std::rc::Rc;

use exaudio_extension::document::Severity;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon, IconName, Sizable as _, Theme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::EditorDoc;
use studio_editor::edit::draft::byte_offset;
use studio_editor::edit::time::TimeField;
use studio_editor::editor_model::EditorSession;
use studio_editor::i18n::localized;
use studio_editor::timeline_model::LaneColor;
use studio_editor::transcript_model::{
    AnnotationDiffStatus, TranscriptActionCommand, TranscriptActionMenuEntry,
    TranscriptDiffCellKind, TranscriptDiffView, TranscriptRowKind, TranscriptRowView,
    TranscriptWordDiffStatus,
};

use super::TranscriptView;
use super::layout::{
    ACTIONS_W, CELL_PX, DIFF_BADGE_W, ID_W, LINT_W, REVIEWER_W, ROW_PY, SMALL_LINE_H, TEXT_PX,
    TEXT_PY, TIME_W, TRACK_W, TextMetrics, WRAP_MARGIN, diff_display_text, display_text,
    has_review_chips,
};
use super::wrap::WrappedText;

/// Per-row inputs gathered by the view before rendering.
pub(super) struct RowParams<'a> {
    pub row: &'a TranscriptRowView,
    pub doc: &'a EditorDoc,
    /// The row text wrapped like its editor wraps it (static text paints these rows).
    pub text_wrap: Rc<WrappedText>,
    pub height: Pixels,
    pub metrics: TextMetrics,
    pub lane_color: Hsla,
    pub is_find_match: bool,
    pub is_playing: bool,
    /// The pooled text editor is mounted in this row.
    pub text_editing: bool,
    /// The pooled time editor is mounted in this row's start/end cell.
    pub time_editing: Option<TimeField>,
}

impl TranscriptView {
    pub(super) fn render_row(&self, params: RowParams, cx: &Context<Self>) -> AnyElement {
        if params.row.row_kind == TranscriptRowKind::Annotation {
            self.render_annotation_row(params, cx)
        } else {
            render_diff_only_row(params, cx)
        }
    }

    fn render_annotation_row(&self, params: RowParams, cx: &Context<Self>) -> AnyElement {
        let RowParams {
            row,
            doc,
            text_wrap,
            height,
            metrics,
            lane_color,
            is_find_match,
            is_playing,
            text_editing,
            time_editing,
        } = params;
        let theme = cx.theme();
        let cells = row.visible_cells;
        let highlighted = row.is_selected
            || doc.session().selected_segment_id.as_deref() == Some(row.segment_id.as_str());
        let segment_id: SharedString = row.segment_id.clone().into();

        let background = if highlighted && is_playing {
            theme.accent.opacity(0.9)
        } else if highlighted {
            theme.list_active
        } else if is_find_match {
            theme.warning.opacity(0.10)
        } else {
            theme.background
        };
        h_flex()
            .id(SharedString::from(format!(
                "transcript-row:{}",
                row.segment_id
            )))
            .relative()
            .w_full()
            .h(height)
            .items_start()
            .py(ROW_PY)
            .overflow_hidden()
            .border_b_1()
            .border_color(theme.border)
            .bg(background)
            .when(!highlighted, |row| {
                row.hover(|style| style.bg(cx.theme().list_hover))
            })
            .when(highlighted, |row| {
                row.child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .w(px(2.))
                        .bg(theme.primary),
                )
            })
            .on_mouse_down(MouseButton::Left, {
                let segment_id = segment_id.clone();
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.press_row(&segment_id, event.click_count, window, cx);
                })
            })
            .when(cells.id, |el| el.child(id_cell(row, metrics, theme)))
            .when(cells.track, |el| {
                el.child(self.speaker_cell(row, doc, metrics, lane_color, cx))
            })
            .when(cells.start, |el| {
                el.child(self.time_cell(row, doc, TimeField::Start, time_editing, metrics, cx))
            })
            .when(cells.end, |el| {
                el.child(self.time_cell(row, doc, TimeField::End, time_editing, metrics, cx))
            })
            .when(cells.reviewer, |el| {
                el.child(reviewer_cell(row, metrics, theme))
            })
            .when(cells.text, |el| {
                el.child(self.text_cell(row, doc, &text_wrap, metrics, text_editing, cx))
            })
            .when(cells.linter, |el| {
                el.child(self.lint_cell(row, doc, metrics, cx))
            })
            .when(cells.actions, |el| {
                el.child(self.actions_cell(row, metrics, cx))
            })
            .into_any_element()
    }

    /// Speaker cell: a dot in the speaker's colour and the speaker label (lane-coloured). The
    /// press selects the row like a click on its body; the click opens the speaker menu, and
    /// `Rename…` mounts the pooled name editor here.
    fn speaker_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        metrics: TextMetrics,
        lane_color: Hsla,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let project = doc.project();
        let menu_open = self.speaker_menu.as_deref() == Some(row.segment_id.as_str());
        let renaming = self
            .speaker_rename
            .as_ref()
            .is_some_and(|(id, _)| id.as_ref() == row.segment_id);
        let speaker_id = project
            .segment(&row.segment_id)
            .map(|segment| segment.speaker_id.as_str());
        let speaker_index =
            speaker_id.and_then(|id| project.speakers.iter().position(|speaker| speaker.id == id));
        let dot_color =
            speaker_index.map_or(theme.muted_foreground, |index| speaker_color(index, theme));

        let content = if renaming {
            div()
                .w_full()
                .h(metrics.line_height)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    Input::new(&self.speaker_input)
                        .xsmall()
                        .h(metrics.line_height),
                )
                .into_any_element()
        } else {
            let segment_id: SharedString = row.segment_id.clone().into();
            h_flex()
                .id(SharedString::from(format!(
                    "transcript-speaker:{}",
                    row.segment_id
                )))
                .w_full()
                .h(metrics.line_height)
                .gap_1p5()
                .items_center()
                .px_1()
                .rounded(theme.radius)
                .child(
                    div()
                        .flex_none()
                        .size(rems(0.5))
                        .rounded_full()
                        .bg(dot_color),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_sm()
                        .line_height(metrics.line_height)
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(lane_color)
                        .truncate()
                        .child(row.speaker_label.clone()),
                )
                .when(!row.is_read_only, |label| {
                    label
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.secondary))
                        .when(menu_open, |label| label.bg(theme.secondary))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.open_speaker_menu(&segment_id, window, cx);
                        }))
                })
                .into_any_element()
        };

        v_flex()
            .w(TRACK_W)
            .flex_none()
            .px(rems(CELL_PX.0 - 0.25))
            .child(content)
            .when(!row.annotation_kind.is_transcription(), |cell| {
                cell.child(
                    div()
                        .px_1()
                        .text_xs()
                        .line_height(SMALL_LINE_H)
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(row.annotation_type_label.clone()),
                )
            })
            .when_some(row.annotation_diff.as_ref(), |cell, diff| {
                let (label, color) = annotation_diff_status(diff.status, theme);
                cell.child(
                    div()
                        .px_1()
                        .text_xs()
                        .line_height(SMALL_LINE_H)
                        .text_color(color)
                        .child(label),
                )
            })
            .when(menu_open && !row.is_read_only, |cell| {
                cell.child(self.speaker_menu(row, doc, speaker_id, cx))
            })
            .into_any_element()
    }

    /// Menu under the speaker cell: every project speaker (check on the row's), `Rename…` for
    /// the row's speaker and, unless speakers are the audio tracks, `New speaker`.
    fn speaker_menu(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        current_speaker_id: Option<&str>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let segment_id: SharedString = row.segment_id.clone().into();
        let can_add = !doc.environment().policy.speakers_bound_to_lanes;
        let item = |id: SharedString| {
            div()
                .id(id)
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .text_sm()
                .hover(|style| style.bg(theme.accent))
                .cursor_pointer()
        };

        let speakers = doc
            .project()
            .speakers
            .iter()
            .enumerate()
            .map(|(index, speaker)| {
                let is_current = current_speaker_id == Some(speaker.id.as_str());
                let speaker_id: SharedString = speaker.id.clone().into();
                let segment_id = segment_id.clone();
                item(SharedString::from(format!(
                    "transcript-speaker-option:{}:{}",
                    row.segment_id, speaker.id
                )))
                .flex()
                .items_center()
                .gap_2()
                .when(is_current, |item| item.bg(theme.list_active))
                .child(
                    div()
                        .flex_none()
                        .size(rems(0.5))
                        .rounded_full()
                        .bg(speaker_color(index, theme)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(speaker.label.clone()),
                )
                .child(div().flex_none().w(rems(1.)).when(is_current, |slot| {
                    slot.child(Icon::new(IconName::Check).xsmall())
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.choose_speaker(&segment_id, &speaker_id, cx);
                }))
            })
            .collect::<Vec<_>>();

        let rename = current_speaker_id.map(|speaker_id| {
            let speaker_id = speaker_id.to_string();
            let segment_id = segment_id.clone();
            item(SharedString::from(format!(
                "transcript-speaker-rename:{}",
                row.segment_id
            )))
            .child(localized("Rename…", "Переименовать…"))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.begin_speaker_rename(&segment_id, &speaker_id, window, cx);
            }))
        });
        let add = can_add.then(|| {
            let segment_id = segment_id.clone();
            item(SharedString::from(format!(
                "transcript-speaker-add:{}",
                row.segment_id
            )))
            .child(localized("New speaker", "Новый спикер"))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.add_speaker(&segment_id, window, cx);
            }))
        });

        deferred(
            anchored().snap_to_window_with_margin(px(8.)).child(
                v_flex()
                    .id(SharedString::from(format!(
                        "transcript-speaker-menu:{}",
                        row.segment_id
                    )))
                    .occlude()
                    .mt_1()
                    .min_w(rems(11.))
                    .p_1()
                    .gap_0p5()
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .border_1()
                    .border_color(theme.border)
                    .rounded(theme.radius)
                    .shadow_md()
                    .children(speakers)
                    .child(div().my_1().h(px(1.)).bg(theme.border))
                    .children(rename)
                    .children(add)
                    .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                        this.close_speaker_menu(cx);
                    })),
            ),
        )
        .with_priority(1)
    }

    fn time_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        field: TimeField,
        editing: Option<TimeField>,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let draft = doc.draft(&row.segment_id);
        let value: SharedString = match (draft, field) {
            (Some(draft), field) => draft.time_value(field).to_string().into(),
            (None, TimeField::Start) => row.start_time.clone().into(),
            (None, TimeField::End) => row.end_time.clone().into(),
        };
        let error = draft.and_then(|draft| draft.time_error(field));

        let content = if editing == Some(field) {
            div()
                .w_full()
                .h(metrics.line_height)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(&self.time_input).xsmall().h(metrics.line_height))
                .into_any_element()
        } else {
            let segment_id: SharedString = row.segment_id.clone().into();
            div()
                .id(SharedString::from(format!(
                    "transcript-time:{}:{}",
                    row.segment_id,
                    field_key(field)
                )))
                // The time input's box (line-height tall, 1px border, 4px padding, centred
                // text), so the value does not move when the cell opens for editing.
                .w_full()
                .h(metrics.line_height)
                .flex()
                .items_center()
                .px_1()
                .border_1()
                .border_color(transparent_black())
                .text_xs()
                .line_height(metrics.line_height)
                .text_color(theme.foreground.opacity(0.85))
                .child(value)
                .when(!row.is_read_only, |cell| {
                    cell.cursor_text()
                        .rounded(theme.radius)
                        .hover(|style| style.bg(theme.secondary))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.press_time_cell(
                                    &segment_id,
                                    field,
                                    event.click_count,
                                    window,
                                    cx,
                                );
                            }),
                        )
                })
                .into_any_element()
        };

        v_flex()
            .w(TIME_W)
            .flex_none()
            .px(rems(CELL_PX.0 - 0.25))
            .child(content)
            .when_some(error, |cell, error| {
                cell.child(
                    div()
                        .px_1()
                        .text_xs()
                        .line_height(SMALL_LINE_H)
                        .text_color(theme.danger)
                        .truncate()
                        .child(error),
                )
            })
            .into_any_element()
    }

    fn text_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_wrap: &Rc<WrappedText>,
        metrics: TextMetrics,
        text_editing: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let review_chips = has_review_chips(row);
        let reviewing = !row.is_read_only && !row.word_views.is_empty();
        let extension_decorations =
            crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx);
        let decoration_label: SharedString = extension_decorations
            .iter()
            .map(|(range, _, label)| format!("{} [{}..{}]", label, range.start, range.end))
            .collect::<Vec<_>>()
            .join("\n")
            .into();

        let text = if text_editing {
            div()
                .relative()
                .w_full()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Textarea::new(&self.text_input).appearance(false).small())
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .rounded(cx.theme().radius)
                        .border_1()
                        .border_color(cx.theme().ring.opacity(0.6)),
                )
                .into_any_element()
        } else if reviewing {
            self.low_confidence_text(row, doc, text_wrap, metrics, cx)
        } else {
            self.static_text(row, doc, text_wrap, metrics, cx)
        };

        v_flex()
            .relative()
            .flex_1()
            .min_w_0()
            .child(text)
            .when(!extension_decorations.is_empty(), |cell| {
                cell.child(
                    div()
                        .id(SharedString::from(format!(
                            "extension-decorations:{}",
                            row.segment_id
                        )))
                        .absolute()
                        .top_0()
                        .right_0()
                        .px_1()
                        .rounded_sm()
                        .text_xs()
                        .bg(cx.theme().muted)
                        .child(format!("{}", extension_decorations.len()))
                        .tooltip(move |window, cx| {
                            Tooltip::new(decoration_label.clone()).build(window, cx)
                        }),
                )
            })
            .when(review_chips, |cell| cell.child(self.review_chips(row, cx)))
            .when_some(row.diff.as_ref(), |cell, diff| {
                cell.child(
                    div()
                        .mt_1()
                        .pl(TEXT_PX)
                        .pr(TEXT_PX + WRAP_MARGIN)
                        .child(diff_cell(row, diff, metrics, cx.theme())),
                )
            })
            .into_any_element()
    }

    /// Read-only rendering of a row's text, broken into the editor's rows; a click mounts the
    /// editor at the clicked caret.
    fn static_text(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_wrap: &Rc<WrappedText>,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let text = display_text(row, doc);
        let editable = !row.is_read_only && !doc.is_read_only();
        let segment_id: SharedString = row.segment_id.clone().into();

        let body = if text.is_empty() {
            div()
                .text_color(theme.muted_foreground)
                .child(if row.annotation_kind.is_transcription() {
                    localized("What was said…", "Что было сказано…")
                } else {
                    localized("Annotation…", "Аннотация…")
                })
                .into_any_element()
        } else {
            let highlights = crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx)
                .into_iter()
                .map(|(range, style, _)| (text_wrap.to_display_range(range), style))
                .collect();
            let styled = StyledText::new(text_wrap.display_text(text))
                .with_highlights(crate::wasm_ui::disjoint_highlights(highlights));
            let layout = styled.layout().clone();
            let ghost = doc
                .ghost_cursor(&row.segment_id)
                .map(|ghost| text_wrap.to_display(byte_offset(text, ghost.char_offset)));
            let caret_layout = layout.clone();
            let ghost_color = theme.warning;
            let line_height = metrics.line_height;
            let text_wrap = Rc::clone(text_wrap);

            div()
                .id(SharedString::from(format!(
                    "transcript-text:{}",
                    row.segment_id
                )))
                .relative()
                .whitespace_nowrap()
                .child(styled)
                .when_some(ghost, |el, ghost| {
                    el.child(
                        canvas(
                            |_, _, _| {},
                            move |_, _, window, _| {
                                if let Some(origin) = caret_layout.position_for_index(ghost) {
                                    window.paint_quad(fill(
                                        Bounds::new(origin, size(px(2.), line_height)),
                                        ghost_color,
                                    ));
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                        let byte =
                            text_wrap.to_source(match layout.index_for_position(event.position) {
                                Ok(index) | Err(index) => index,
                            });
                        let modifiers = event.modifiers;
                        // Helper word seek: a plain Alt+Click (no Ctrl/Shift/Meta).
                        if modifiers.alt
                            && !modifiers.control
                            && !modifiers.shift
                            && !modifiers.platform
                        {
                            cx.stop_propagation();
                            this.seek_to_word_at_byte(&segment_id, byte, cx);
                        } else if editable {
                            cx.stop_propagation();
                            this.activate_text_at_byte(&segment_id, byte, window, cx);
                        }
                    }),
                )
                .into_any_element()
        };

        let segment_id: SharedString = row.segment_id.clone().into();
        div()
            .id(SharedString::from(format!(
                "transcript-text-cell:{}",
                row.segment_id
            )))
            .w_full()
            .pl(TEXT_PX)
            .pr(TEXT_PX + WRAP_MARGIN)
            .py(TEXT_PY)
            .text_sm()
            .line_height(metrics.line_height)
            .text_color(theme.foreground)
            .when(editable, |el| {
                el.cursor_text().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        this.activate_text_at_end(&segment_id, window, cx);
                    }),
                )
            })
            .child(body)
            .into_any_element()
    }

    /// Text of a row with unreviewed low-confidence words: words are underlined and seek on
    /// click; editing resumes once every word is reviewed. Broken into the editor's rows like
    /// [`Self::static_text`].
    fn low_confidence_text(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        text_wrap: &WrappedText,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let text = display_text(row, doc);
        let tokens = word_ranges(text);
        let mut ranges = Vec::new();
        let mut seeks = Vec::new();
        let mut highlights = Vec::new();
        for word in row.word_views.iter().filter(|word| word.is_low_confidence) {
            let Some(range) = tokens
                .get(word.index)
                .map(|range| text_wrap.to_display_range(range.clone()))
            else {
                continue;
            };
            let acknowledged = word.is_acknowledged;
            if !acknowledged {
                highlights.push((
                    range.clone(),
                    HighlightStyle {
                        underline: Some(UnderlineStyle {
                            thickness: px(1.),
                            color: Some(theme.warning),
                            wavy: true,
                        }),
                        ..Default::default()
                    },
                ));
            }
            if let Some(start) = word.start_seconds {
                ranges.push(range);
                seeks.push(start);
            }
        }
        highlights.extend(
            crate::wasm_ui::transcript_decorations(doc, &row.segment_id, cx)
                .into_iter()
                .map(|(range, style, _)| (text_wrap.to_display_range(range), style)),
        );
        highlights.sort_by_key(|(range, _)| range.start);
        let styled = StyledText::new(text_wrap.display_text(text))
            .with_highlights(crate::wasm_ui::disjoint_highlights(highlights));
        let model = self.model.clone();

        div()
            .w_full()
            .pl(TEXT_PX)
            .pr(TEXT_PX + WRAP_MARGIN)
            .py(TEXT_PY)
            .text_sm()
            .line_height(metrics.line_height)
            .text_color(theme.foreground)
            .whitespace_nowrap()
            .child(
                InteractiveText::new(
                    SharedString::from(format!("transcript-lowconf:{}", row.segment_id)),
                    styled,
                )
                .on_click(ranges, move |index, _, cx| {
                    let Some(&start) = seeks.get(index) else {
                        return;
                    };
                    model.update(cx, |model, cx| model.run(cx, |doc| doc.seek(start)));
                }),
            )
            .into_any_element()
    }

    fn review_chips(&self, row: &TranscriptRowView, cx: &Context<Self>) -> impl IntoElement {
        let chips = row
            .word_views
            .iter()
            .filter(|word| {
                word.is_low_confidence && !word.is_acknowledged && word.command.is_some()
            })
            .map(|word| {
                let command = word.command.clone().expect("filtered command");
                let arguments = word.command_arguments.clone();
                Button::new(SharedString::from(format!(
                    "transcript-ack:{}:{}",
                    row.segment_id, word.index
                )))
                .label(word.text.clone())
                .icon(IconName::Check)
                .xsmall()
                .outline()
                .tooltip(localized("Mark as reviewed", "Отметить как проверенное"))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.model.update(cx, |model, cx| {
                        model.edit(cx, |doc| doc.execute_command(&command, &arguments))
                    });
                }))
            });

        h_flex()
            .pl(TEXT_PX)
            .pt_1()
            .gap_1()
            .overflow_hidden()
            .children(chips)
    }

    fn lint_cell(
        &self,
        row: &TranscriptRowView,
        doc: &EditorDoc,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let Some((severity, message)) = doc.row_lint(row) else {
            return div().w(LINT_W).flex_none().into_any_element();
        };
        let color = lint_color(severity, theme);
        let tooltip: SharedString = message.into();
        let dot = div()
            .id(SharedString::from(format!(
                "transcript-diagnostic:{}",
                row.segment_id
            )))
            .size(rems(0.625))
            .rounded_full()
            .bg(color)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx));

        h_flex()
            .w(LINT_W)
            .flex_none()
            .h(metrics.line_height + TEXT_PY * 2.)
            .justify_center()
            .child(dot)
            .into_any_element()
    }

    fn actions_cell(
        &self,
        row: &TranscriptRowView,
        metrics: TextMetrics,
        cx: &Context<Self>,
    ) -> AnyElement {
        let cell = h_flex()
            .w(ACTIONS_W)
            .flex_none()
            .h(metrics.line_height + TEXT_PY * 2.)
            .justify_center();
        if row.is_read_only {
            return cell.into_any_element();
        }
        let entries = EditorSession::transcript_action_menu(row);
        let segment_id: SharedString = row.segment_id.clone().into();
        let view = cx.entity().downgrade();
        let fixes: Vec<_> = row
            .lint_issues
            .iter()
            .flat_map(|issue| issue.fixes.iter().cloned())
            .collect();
        let revision = self.model.read(cx).revision();
        let model = self.model.clone();

        cell.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new(SharedString::from(format!(
                    "transcript-actions:{}",
                    row.segment_id
                )))
                .icon(IconName::EllipsisVertical)
                .ghost()
                .xsmall()
                .dropdown_menu(move |mut menu, _, cx| {
                    for entry in &entries {
                        if matches!(
                            *entry,
                            TranscriptActionMenuEntry::Item {
                                command: TranscriptActionCommand::MergeAbove,
                                ..
                            }
                        ) {
                            // Enabled when the playhead is inside the row, read when the menu
                            // opens (the row does not re-render per playback frame).
                            let doc = model.read(cx).doc();
                            let playhead = doc.session().playhead_seconds;
                            let inside =
                                doc.project().segment(&segment_id).is_some_and(|segment| {
                                    segment.start_seconds <= playhead
                                        && playhead < segment.end_seconds
                                });
                            let view = view.clone();
                            let segment_id = segment_id.clone();
                            menu = menu.item(
                                PopupMenuItem::new(localized(
                                    "Split at playhead",
                                    "Разделить по курсору",
                                ))
                                .icon(Icon::from(gpui_kit::assets::IconName::Scissors))
                                .disabled(!inside)
                                .on_click(move |_, window, cx| {
                                    view.update(cx, |this, cx| {
                                        this.split_row_at_playhead(&segment_id, window, cx);
                                    })
                                    .ok();
                                }),
                            );
                        }
                        menu = match *entry {
                            TranscriptActionMenuEntry::Separator => menu.separator(),
                            TranscriptActionMenuEntry::Item { command, enabled } => {
                                let view = view.clone();
                                let segment_id = segment_id.clone();
                                menu.item(
                                    PopupMenuItem::new(command_label(command))
                                        .icon(command_icon(command))
                                        .disabled(!enabled)
                                        .on_click(move |_, window, cx| {
                                            view.update(cx, |this, cx| {
                                                this.run_row_command(
                                                    command,
                                                    &segment_id,
                                                    window,
                                                    cx,
                                                );
                                            })
                                            .ok();
                                        }),
                                )
                            }
                        };
                    }
                    for fix in &fixes {
                        let fix = fix.clone();
                        let model = model.clone();
                        menu = menu.item(
                            PopupMenuItem::new(
                                localized("Fix: {title}", "Исправить: {title}")
                                    .replace("{title}", &fix.title),
                            )
                            .on_click(move |_, _, cx| {
                                model.update(cx, |model, cx| {
                                    model.edit(cx, |doc| {
                                        doc.apply_transaction(
                                            exaudio_extension::document::DocumentTransaction {
                                                label: fix.title.clone(),
                                                expected_revision: revision,
                                                edits: fix.edits.clone(),
                                            },
                                        )
                                    })
                                });
                            }),
                        );
                    }
                    menu
                }),
            )
            .into_any_element()
    }
}

/// Diff rows of the compare version (added/deleted/removed annotations): read-only.
fn render_diff_only_row(params: RowParams, cx: &Context<TranscriptView>) -> AnyElement {
    let RowParams {
        row,
        height,
        metrics,
        ..
    } = params;
    let theme = cx.theme();
    let cells = row.visible_cells;
    let added = row.row_kind == TranscriptRowKind::ModifiedAdded;
    let tone = if added { theme.success } else { theme.danger };
    let dash = || {
        div()
            .text_xs()
            .line_height(metrics.line_height)
            .text_color(theme.muted_foreground)
            .child("–")
    };

    h_flex()
        .w_full()
        .h(height)
        .items_start()
        .py(ROW_PY)
        .overflow_hidden()
        .border_b_1()
        .border_color(theme.border)
        .bg(tone.opacity(0.06))
        .when(cells.id, |el| el.child(id_cell(row, metrics, theme)))
        .when(cells.track, |el| {
            el.child(
                div()
                    .w(TRACK_W)
                    .flex_none()
                    .px(CELL_PX)
                    .text_sm()
                    .line_height(metrics.line_height)
                    .text_color(tone)
                    .truncate()
                    .child(row.speaker_label.clone()),
            )
        })
        .when(cells.start, |el| {
            el.child(static_time(row.start_time.clone(), metrics, theme))
        })
        .when(cells.end, |el| {
            el.child(static_time(row.end_time.clone(), metrics, theme))
        })
        .when(cells.reviewer, |el| {
            el.child(reviewer_cell(row, metrics, theme))
        })
        .when(cells.text, |el| {
            let content = match &row.diff {
                Some(diff) => diff_cell(row, diff, metrics, theme).into_any_element(),
                None => div()
                    .text_sm()
                    .line_height(metrics.line_height)
                    .text_color(theme.muted_foreground)
                    .child(row.text.clone())
                    .into_any_element(),
            };
            el.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(TEXT_PX)
                    .pr(TEXT_PX + WRAP_MARGIN)
                    .py(TEXT_PY)
                    .child(content),
            )
        })
        .when(cells.linter, |el| {
            el.child(
                h_flex()
                    .w(LINT_W)
                    .flex_none()
                    .justify_center()
                    .child(dash()),
            )
        })
        .when(cells.actions, |el| {
            el.child(
                h_flex()
                    .w(ACTIONS_W)
                    .flex_none()
                    .justify_center()
                    .child(dash()),
            )
        })
        .into_any_element()
}

fn id_cell(row: &TranscriptRowView, metrics: TextMetrics, theme: &Theme) -> impl IntoElement {
    div()
        .w(ID_W)
        .flex_none()
        .px(CELL_PX)
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(theme.muted_foreground)
        .child(row.display_id.clone())
}

fn static_time(value: String, metrics: TextMetrics, theme: &Theme) -> impl IntoElement {
    div()
        .w(TIME_W)
        .flex_none()
        .px(CELL_PX)
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(theme.muted_foreground)
        .child(value)
}

fn reviewer_cell(row: &TranscriptRowView, metrics: TextMetrics, theme: &Theme) -> impl IntoElement {
    let color = row
        .reviewer
        .as_ref()
        .map_or(theme.muted_foreground, |reviewer| {
            reviewer.color_hue.map_or(theme.foreground, |hue| {
                let lightness = if theme.is_dark() { 0.72 } else { 0.30 };
                hsla(f32::from(hue) / 360., 0.70, lightness, 1.)
            })
        });
    div()
        .w(REVIEWER_W)
        .flex_none()
        .px(CELL_PX)
        .text_xs()
        .line_height(metrics.line_height)
        .text_color(color)
        .truncate()
        .children(row.reviewer.as_ref().map(|reviewer| reviewer.label.clone()))
}

fn diff_cell(
    row: &TranscriptRowView,
    diff: &TranscriptDiffView,
    metrics: TextMetrics,
    theme: &Theme,
) -> impl IntoElement {
    let (label, tone) = diff_badge(diff, theme);
    let text = diff_display_text(row);
    let words: AnyElement = if diff.words.is_empty() {
        div()
            .italic()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element()
    } else {
        let mut highlights = Vec::with_capacity(diff.words.len());
        let mut offset = 0;
        for (index, word) in diff.words.iter().enumerate() {
            if index > 0 {
                offset += 1;
            }
            let range = offset..offset + word.value.len();
            offset = range.end;
            let style = match word.status {
                TranscriptWordDiffStatus::Unchanged => continue,
                TranscriptWordDiffStatus::Added => HighlightStyle {
                    color: Some(theme.success),
                    background_color: Some(theme.success.opacity(0.16)),
                    ..Default::default()
                },
                TranscriptWordDiffStatus::Removed => HighlightStyle {
                    color: Some(theme.danger),
                    background_color: Some(theme.danger.opacity(0.16)),
                    strikethrough: Some(StrikethroughStyle {
                        thickness: px(1.),
                        color: Some(theme.danger),
                    }),
                    ..Default::default()
                },
                TranscriptWordDiffStatus::Unknown => HighlightStyle {
                    color: Some(theme.muted_foreground),
                    ..Default::default()
                },
            };
            highlights.push((range, style));
        }
        StyledText::new(SharedString::from(text))
            .with_highlights(highlights)
            .into_any_element()
    };

    h_flex()
        .items_start()
        .text_sm()
        .line_height(metrics.line_height)
        .child(
            div().w(DIFF_BADGE_W).flex_none().child(
                div()
                    .px_1p5()
                    .w(rems(DIFF_BADGE_W.0 - 0.5))
                    .rounded(theme.radius)
                    .bg(tone.opacity(0.16))
                    .text_color(tone)
                    .text_xs()
                    .text_center()
                    .child(label),
            ),
        )
        .child(div().flex_1().min_w_0().child(words))
}

fn diff_badge(diff: &TranscriptDiffView, theme: &Theme) -> (SharedString, Hsla) {
    match diff.kind {
        TranscriptDiffCellKind::New => (localized("New", "Новый").into(), theme.info),
        TranscriptDiffCellKind::ModifiedAdded => {
            (localized("Added", "Добавлен").into(), theme.success)
        }
        TranscriptDiffCellKind::ModifiedDeleted => {
            (localized("Deleted", "Удалён").into(), theme.danger)
        }
        TranscriptDiffCellKind::Diff => match diff.change_count {
            0 => (
                localized("No changes", "Без изменений").into(),
                theme.muted_foreground,
            ),
            1 => (localized("1 change", "1 изменение").into(), theme.warning),
            count => (
                localized("{n} changes", "{n} изменений")
                    .replace("{n}", &count.to_string())
                    .into(),
                theme.warning,
            ),
        },
    }
}

fn annotation_diff_status(status: AnnotationDiffStatus, theme: &Theme) -> (&'static str, Hsla) {
    match status {
        AnnotationDiffStatus::Matched => (
            localized("Unchanged", "Без изменений"),
            theme.muted_foreground,
        ),
        AnnotationDiffStatus::LabelMismatch => {
            (localized("Label changed", "Изменена метка"), theme.warning)
        }
        AnnotationDiffStatus::SubcategoryMismatch => (
            localized("Subcategory changed", "Изменена подкатегория"),
            theme.warning,
        ),
        AnnotationDiffStatus::IntensityMismatch => (
            localized("Intensity changed", "Изменена интенсивность"),
            theme.warning,
        ),
        AnnotationDiffStatus::UnmatchedReference => (localized("Removed", "Удалено"), theme.danger),
    }
}

/// Colour of the speaker at `index` in the project's speaker list: the first two match the
/// lane palette (so a one-lane project's first speaker shares its lane colour), later ones
/// spread round the hue circle.
fn speaker_color(index: usize, theme: &Theme) -> Hsla {
    match index {
        0 => Hsla::from(rgb(LaneColor::Blue.rgb())),
        1 => Hsla::from(rgb(LaneColor::Purple.rgb())),
        _ => {
            let hue = ((207 + 137 * index) % 360) as f32 / 360.;
            let lightness = if theme.is_dark() { 0.70 } else { 0.42 };
            hsla(hue, 0.70, lightness, 1.)
        }
    }
}

pub(super) fn lint_color(severity: Severity, theme: &Theme) -> Hsla {
    match severity {
        Severity::Hint | Severity::Info => theme.muted_foreground,
        Severity::Warning => theme.warning,
        Severity::Error => theme.danger,
    }
}

fn command_label(command: TranscriptActionCommand) -> &'static str {
    match command {
        TranscriptActionCommand::Edit => localized("Edit", "Изменить"),
        TranscriptActionCommand::CreateBefore => {
            localized("Add segment above", "Добавить сегмент выше")
        }
        TranscriptActionCommand::CreateAfter => {
            localized("Add segment below", "Добавить сегмент ниже")
        }
        TranscriptActionCommand::MergeAbove => {
            localized("Merge with previous", "Объединить с предыдущим")
        }
        TranscriptActionCommand::MergeBelow => {
            localized("Merge with next", "Объединить со следующим")
        }
        TranscriptActionCommand::Delete => localized("Delete", "Удалить"),
    }
}

fn command_icon(command: TranscriptActionCommand) -> Icon {
    match command {
        TranscriptActionCommand::Edit => gpui_kit::assets::IconName::SquarePen.into(),
        TranscriptActionCommand::CreateBefore | TranscriptActionCommand::CreateAfter => {
            IconName::Plus.into()
        }
        TranscriptActionCommand::MergeAbove => IconName::ArrowUp.into(),
        TranscriptActionCommand::MergeBelow => IconName::ArrowDown.into(),
        TranscriptActionCommand::Delete => IconName::Delete.into(),
    }
}

fn field_key(field: TimeField) -> &'static str {
    match field {
        TimeField::Start => "start",
        TimeField::End => "end",
    }
}

/// Byte ranges of whitespace-separated words, in order (the indexing of
/// `TranscriptWordView::index`).
fn word_ranges(text: &str) -> Vec<Range<usize>> {
    text.split_whitespace()
        .map(|word| {
            let start = word.as_ptr() as usize - text.as_ptr() as usize;
            start..start + word.len()
        })
        .collect()
}
