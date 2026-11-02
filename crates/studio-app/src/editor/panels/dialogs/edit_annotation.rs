//! Add/Edit Annotation dialog for an annotation row (categories + intensity).

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::annotation::EditAnnotationDialog;
use studio_editor::edit::create_region::{
    OTHER_CATEGORY, intensity_label, primary_category_options, secondary_category_options,
};

use super::create_region::{intensity_picker, string_choices};
use crate::editor::model::EditorModel;
use crate::editor::panels::task::TaskPanels;
use crate::editor::panels::ui::{
    choice, dialog_footer, dialog_header, error_text, field, primary_button,
};

struct EditAnnotationView {
    model: Entity<EditorModel>,
    panels: Entity<TaskPanels>,
    description: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EditAnnotationView {
    fn new(
        model: Entity<EditorModel>,
        panels: Entity<TaskPanels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial = model
            .read(cx)
            .doc()
            .edit_annotation()
            .map(|dialog| dialog.custom_description.clone())
            .unwrap_or_default();
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Describe the noise...")
                .default_value(initial)
        });
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &description,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::Change => {
                        let value = input.read(cx).value().to_string();
                        this.update_dialog(cx, |dialog| dialog.set_custom_description(value));
                    }
                    InputEvent::PressEnter { .. } => this.submit(window, cx),
                    _ => {}
                },
            ),
        ];
        Self {
            model,
            panels,
            description,
            _subscriptions: subscriptions,
        }
    }

    fn update_dialog(&self, cx: &mut App, change: impl FnOnce(&mut EditAnnotationDialog)) {
        update_dialog(&self.model, cx, change);
    }

    fn submit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.edit(cx, |doc| doc.submit_edit_annotation())
        });
    }
}

fn update_dialog(
    model: &Entity<EditorModel>,
    cx: &mut App,
    change: impl FnOnce(&mut EditAnnotationDialog),
) {
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| {
            if let Some(dialog) = doc.edit_annotation_mut() {
                change(dialog);
            }
        });
    });
}

impl Render for EditAnnotationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(dialog) = self.model.read(cx).doc().edit_annotation().cloned() else {
            return div().into_any_element();
        };
        let title = if dialog.is_new() {
            "Add Annotation"
        } else {
            "Edit Annotation"
        };
        let intensity = intensity_picker(
            "edit-annotation-intensity",
            intensity_label(dialog.intensity),
            self.model.clone(),
            |doc, label| {
                if let Some(dialog) = doc.edit_annotation_mut() {
                    dialog.set_intensity_label(&label);
                }
            },
        );
        let categories = if dialog.uses_hierarchical_labels {
            let model = self.model.clone();
            let primary = choice(
                "edit-annotation-primary",
                (!dialog.primary_selection.is_empty()).then_some(&dialog.primary_selection),
                string_choices(primary_category_options().iter().copied()),
                "Select primary category",
                move |value, _, cx| {
                    update_dialog(&model, cx, |dialog| dialog.set_primary_selection(value));
                },
            );
            let show_secondary =
                !dialog.primary_selection.is_empty() && dialog.primary_selection != OTHER_CATEGORY;
            let model = self.model.clone();
            let secondary = choice(
                "edit-annotation-secondary",
                (!dialog.secondary_selection.is_empty()).then_some(&dialog.secondary_selection),
                string_choices(secondary_category_options(&dialog.primary_selection)),
                "Select secondary category",
                move |value, _, cx| {
                    update_dialog(&model, cx, |dialog| dialog.set_secondary_selection(value));
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
                        .when(show_secondary, |this| {
                            this.child(div().flex_1().child(field(
                                "Secondary Category *",
                                secondary,
                                cx,
                            )))
                        }),
                )
                .when(needs_description, |this| {
                    this.child(field("Description *", Input::new(&self.description), cx))
                })
        } else {
            v_flex().child(field(
                "Category",
                div().text_sm().child("Background Noise"),
                cx,
            ))
        };
        let model = self.model.clone();
        let panels = self.panels.clone();
        v_flex()
            .gap_3()
            .child(dialog_header(title, None, cx))
            .child(categories)
            .child(field("Intensity *", intensity, cx))
            .children(dialog.error.map(|error| error_text(error.to_string(), cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("edit-annotation-cancel")
                            .label("Cancel")
                            .on_click(move |_, window, cx| {
                                cancel(&panels, &model, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        primary_button("edit-annotation-save", "Save changes", false, false)
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

fn cancel(panels: &Entity<TaskPanels>, model: &Entity<EditorModel>, cx: &mut App) {
    panels.update(cx, |panels, _| panels.forget_doc_dialog());
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| doc.cancel_edit_annotation())
    });
}

/// Opens the dialog for the doc's current edit-annotation state.
pub fn open(
    model: &Entity<EditorModel>,
    panels: &Entity<TaskPanels>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| EditAnnotationView::new(model.clone(), panels.clone(), window, cx));
    let model = model.clone();
    let panels = panels.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let model = model.clone();
        let panels = panels.clone();
        let submit = view.clone();
        dialog
            .w(rems(32.5).to_pixels(window.rem_size()))
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
