//! Add-region dialog. Two shapes, both mirroring the doc's `CreateRegionDialog`:
//! - the full dialog (speaker, start/end, text or annotation categories + intensity);
//! - the attached "Add Region" card for a row the waveform drag already created in a
//!   transcription project (text only; Cancel keeps the created row).

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::create_region::{
    CreateRegionDialog, CreateRegionMode, CreateRegionTimeField, OTHER_CATEGORY, intensity_label,
    intensity_options, primary_category_options, secondary_category_options, track_options,
};

use crate::editor::model::EditorModel;
use crate::editor::panels::task::TaskPanels;
use crate::editor::panels::ui::{
    Choice, choice, dialog_footer, dialog_header, error_text, field, primary_button,
};

struct CreateRegionView {
    model: Entity<EditorModel>,
    panels: Entity<TaskPanels>,
    attached: bool,
    start: Entity<InputState>,
    end: Entity<InputState>,
    text: Entity<InputState>,
    description: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl CreateRegionView {
    fn new(
        model: Entity<EditorModel>,
        panels: Entity<TaskPanels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let dialog = model.read(cx).doc().create_region().cloned();
        let dialog = dialog.as_ref();
        let attached = dialog.is_some_and(|dialog| {
            dialog.mode == CreateRegionMode::Transcription && dialog.created_segment_id.is_some()
        });
        let input = |value: Option<&str>,
                     placeholder: &'static str,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            let value = value.unwrap_or_default().to_string();
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let start = input(
            dialog.map(|d| d.start_draft.as_str()),
            "mm:ss.xx",
            window,
            cx,
        );
        let end = input(dialog.map(|d| d.end_draft.as_str()), "mm:ss.xx", window, cx);
        let text = input(
            dialog.map(|d| d.text.as_str()),
            "Enter text for this region...",
            window,
            cx,
        );
        let description = input(
            dialog.map(|d| d.custom_description.as_str()),
            "Describe the noise...",
            window,
            cx,
        );

        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&start, window, |this, input, event, window, cx| {
                this.on_time_event(CreateRegionTimeField::Start, input, event, window, cx);
            }),
            cx.subscribe_in(&end, window, |this, input, event, window, cx| {
                this.on_time_event(CreateRegionTimeField::End, input, event, window, cx);
            }),
            cx.subscribe_in(
                &text,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.model.update(cx, |model, cx| {
                            model.run(cx, |doc| doc.set_create_region_text(value));
                        });
                    }
                    InputEvent::PressEnter { secondary, .. } if *secondary || this.attached => {
                        this.submit(window, cx);
                    }
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &description,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.update_dialog(cx, |dialog| dialog.set_custom_description(value));
                    }
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.submit(window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            model,
            panels,
            attached,
            start,
            end,
            text,
            description,
            _subscriptions: subscriptions,
        }
    }

    fn update_dialog(&self, cx: &mut App, change: impl FnOnce(&mut CreateRegionDialog)) {
        self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                if let Some(dialog) = doc.create_region_mut() {
                    change(dialog);
                }
            });
        });
    }

    fn on_time_event(
        &mut self,
        field: CreateRegionTimeField,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let value = input.read(cx).value().to_string();
                self.update_dialog(cx, |dialog| match field {
                    CreateRegionTimeField::Start => dialog.set_start_draft(value),
                    CreateRegionTimeField::End => dialog.set_end_draft(value),
                });
            }
            InputEvent::PressEnter {
                secondary: true, ..
            } => self.submit(window, cx),
            InputEvent::PressEnter { .. } => {
                self.model.update(cx, |model, cx| {
                    model.edit(cx, |doc| doc.commit_create_region_time(field));
                });
                self.sync_time_inputs(window, cx);
            }
            InputEvent::Blur => {
                self.update_dialog(cx, |dialog| {
                    dialog.blur_time_field(field);
                });
                self.sync_time_inputs(window, cx);
            }
            InputEvent::Focus => {}
        }
    }

    /// Escape inside a time field restores its stored value instead of closing the dialog.
    fn on_key_down_capture(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "escape" {
            return;
        }
        let focused = self.start.read(cx).focus_handle(cx).is_focused(window)
            || self.end.read(cx).focus_handle(cx).is_focused(window);
        if !focused {
            return;
        }
        let reset = self.model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| doc.reset_create_region_time())
        });
        if reset {
            self.model.update(cx, |model, cx| {
                model.set_status("Region time edit canceled.", cx)
            });
            self.sync_time_inputs(window, cx);
            cx.stop_propagation();
        }
    }

    fn sync_time_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((start, end)) = self
            .model
            .read(cx)
            .doc()
            .create_region()
            .map(|dialog| (dialog.start_draft.clone(), dialog.end_draft.clone()))
        else {
            return;
        };
        for (input, value) in [(&self.start, start), (&self.end, end)] {
            if input.read(cx).value() != value {
                input.update(cx, |input, cx| input.set_value(value, window, cx));
            }
        }
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.submit_create_region())
        });
    }

    fn render_full(&self, dialog: &CreateRegionDialog, cx: &mut Context<Self>) -> Div {
        let project = self.model.read(cx).doc().project();
        let tracks = track_options(project)
            .into_iter()
            .map(|option| Choice::new(option.lane_id, option.label))
            .collect();
        let model = self.model.clone();
        let speaker = choice(
            "create-region-speaker",
            Some(&dialog.selected_lane_id),
            tracks,
            "Select speaker",
            move |lane_id, _, cx| {
                model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| {
                        if let Some(dialog) = doc.create_region_mut() {
                            dialog.selected_lane_id = lane_id;
                        }
                    });
                });
            },
        );
        let time =
            |label: &'static str, input: &Entity<InputState>, error: Option<String>, cx: &App| {
                v_flex()
                    .flex_1()
                    .gap_1()
                    .child(field(label, Input::new(input).small(), cx))
                    .children(error.map(|error| error_text(error, cx)))
            };
        let title = match dialog.mode {
            CreateRegionMode::Transcription => "Add New Region",
            CreateRegionMode::Annotation => "Add New Annotation",
        };
        let body = match dialog.mode {
            CreateRegionMode::Transcription => {
                v_flex().child(field("Text", Input::new(&self.text), cx))
            }
            CreateRegionMode::Annotation => self.render_annotation_fields(dialog, cx),
        };
        v_flex()
            .gap_3()
            .child(dialog_header(title, None, cx))
            .child(field("Speaker", speaker, cx))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(time(
                        "Start Time (mm:ss.xx)",
                        &self.start,
                        dialog.start_error.as_ref().map(ToString::to_string),
                        cx,
                    ))
                    .child(time(
                        "End Time (mm:ss.xx)",
                        &self.end,
                        dialog.end_error.as_ref().map(ToString::to_string),
                        cx,
                    )),
            )
            .child(body)
            .child(self.footer("Create", !dialog.is_create_enabled(), cx))
    }

    fn render_annotation_fields(&self, dialog: &CreateRegionDialog, cx: &App) -> Div {
        let intensity = intensity_picker(
            "create-region-intensity",
            intensity_label(dialog.intensity),
            self.model.clone(),
            |doc, label| {
                if let Some(dialog) = doc.create_region_mut() {
                    dialog.set_intensity_label(&label);
                }
            },
        );
        if !dialog.uses_hierarchical_labels {
            return v_flex()
                .gap_3()
                .child(field(
                    "Category",
                    div().text_sm().child("Background Noise"),
                    cx,
                ))
                .child(field("Intensity *", intensity, cx));
        }
        let model = self.model.clone();
        let primary = choice(
            "create-region-primary",
            (!dialog.primary_selection.is_empty()).then_some(&dialog.primary_selection),
            string_choices(primary_category_options().iter().copied()),
            "Select primary category",
            move |value, _, cx| {
                model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| {
                        if let Some(dialog) = doc.create_region_mut() {
                            dialog.set_primary_category(value);
                        }
                    });
                });
            },
        );
        let model = self.model.clone();
        let secondary = choice(
            "create-region-secondary",
            (!dialog.secondary_selection.is_empty()).then_some(&dialog.secondary_selection),
            string_choices(secondary_category_options(&dialog.primary_selection)),
            "Select secondary category",
            move |value, _, cx| {
                model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| {
                        if let Some(dialog) = doc.create_region_mut() {
                            dialog.set_secondary_category(value);
                        }
                    });
                });
            },
        );
        let needs_description = dialog.primary_selection == OTHER_CATEGORY
            || dialog.secondary_selection == OTHER_CATEGORY;
        v_flex()
            .gap_3()
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .child(field("Primary Category *", primary, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(field("Secondary Category *", secondary, cx)),
                    ),
            )
            .child(field("Intensity *", intensity, cx))
            .when(needs_description, |this| {
                this.child(field("Other", Input::new(&self.description), cx))
            })
    }

    fn render_attached(&self, cx: &mut Context<Self>) -> Div {
        v_flex()
            .gap_3()
            .child(dialog_header("Add Region", None, cx))
            .child(Input::new(&self.text))
            .child(self.footer("Save", false, cx))
    }

    fn footer(&self, confirm: &'static str, disabled: bool, cx: &mut Context<Self>) -> Div {
        let model = self.model.clone();
        let panels = self.panels.clone();
        dialog_footer()
            .child(
                Button::new("create-region-cancel")
                    .label("Cancel")
                    .on_click(move |_, window, cx| {
                        cancel(&panels, &model, cx);
                        window.close_dialog(cx);
                    }),
            )
            .child(
                primary_button("create-region-submit", confirm, false, disabled)
                    .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
            )
    }
}

impl Render for CreateRegionView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(dialog) = self.model.read(cx).doc().create_region().cloned() else {
            return div().into_any_element();
        };
        let content = if self.attached {
            self.render_attached(cx)
        } else {
            self.render_full(&dialog, cx)
        };
        // Escape inside a time field is handled before the dialog sees it.
        content
            .capture_key_down(cx.listener(Self::on_key_down_capture))
            .into_any_element()
    }
}

fn cancel(panels: &Entity<TaskPanels>, model: &Entity<EditorModel>, cx: &mut App) {
    panels.update(cx, |panels, _| panels.forget_doc_dialog());
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| doc.cancel_create_region())
    });
}

pub(crate) fn string_choices<S: AsRef<str>>(
    options: impl IntoIterator<Item = S>,
) -> Vec<Choice<String>> {
    options
        .into_iter()
        .map(|option| {
            let option = option.as_ref().to_string();
            Choice::new(option.clone(), option)
        })
        .collect()
}

/// Intensity dropdown over the four labelled levels.
pub(crate) fn intensity_picker(
    id: &'static str,
    selected: Option<&'static str>,
    model: Entity<EditorModel>,
    apply: impl Fn(&mut studio_editor::edit::EditorDoc, String) + 'static,
) -> impl IntoElement {
    let selected = selected.map(str::to_string);
    choice(
        id,
        selected.as_ref(),
        string_choices(intensity_options().iter().copied()),
        "Select intensity",
        move |label, _, cx| {
            model.update(cx, |model, cx| {
                model.update_doc(cx, |doc| apply(doc, label))
            });
        },
    )
}

/// Opens the dialog for the doc's current add-region state.
pub fn open(
    model: &Entity<EditorModel>,
    panels: &Entity<TaskPanels>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| CreateRegionView::new(model.clone(), panels.clone(), window, cx));
    let attached = view.read(cx).attached;
    let focus_target = if attached {
        view.read(cx).text.clone()
    } else {
        view.read(cx).start.clone()
    };
    let focus = std::cell::Cell::new(true);
    let model = model.clone();
    let panels = panels.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        if focus.replace(false) {
            focus_target.read(cx).focus_handle(cx).focus(window, cx);
        }
        let model = model.clone();
        let panels = panels.clone();
        let submit = view.clone();
        dialog
            .w(rems(if attached { 26.25 } else { 32.5 }).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| {
                cancel(&panels, &model, cx);
                true
            })
            .on_ok(move |_, window, cx| {
                submit.update(cx, |view, cx| view.submit(window, cx));
                false
            })
            .child(view.clone())
    });
}
