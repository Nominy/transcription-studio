//! Add/Edit Annotation dialog for an annotation row (categories + intensity).

use exaudio_babel_domain::edit::annotation::EditAnnotationDialog;
use exaudio_babel_domain::edit::create_region::{
    OTHER_CATEGORY, intensity_label, primary_category_options, secondary_category_options,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::create_region::{category_choices, intensity_picker, update_dialog_state};
use crate::services::document_dialogs;
use crate::text::localized;
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::ui::{
    choice, dialog_footer, dialog_header, error_text, field, primary_button,
};

struct EditAnnotationView {
    model: Entity<EditorModel>,
    description: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EditAnnotationView {
    fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial = document_dialogs(&model, cx)
            .borrow()
            .edit_annotation()
            .map(|dialog| dialog.custom_description.clone())
            .unwrap_or_default();
        let description = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Describe the sound…", "Опишите звук…"))
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
            description,
            _subscriptions: subscriptions,
        }
    }

    fn update_dialog(&self, cx: &mut App, change: impl FnOnce(&mut EditAnnotationDialog)) {
        update_dialog(&self.model, cx, change);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialogs = document_dialogs(&self.model, cx);
        let saved = self.model.update(cx, |model, cx| {
            let result =
                model.update_doc(cx, |doc| dialogs.borrow_mut().submit_edit_annotation(doc));
            let saved = result.is_ok();
            model.apply_result(result, cx);
            saved
        });
        if saved {
            window.close_dialog(cx);
        }
    }
}

fn update_dialog(
    model: &Entity<EditorModel>,
    cx: &mut App,
    change: impl FnOnce(&mut EditAnnotationDialog),
) {
    update_dialog_state(model, cx, |dialogs| {
        if let Some(dialog) = dialogs.edit_annotation_mut() {
            change(dialog);
        }
    });
}

impl Render for EditAnnotationView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(dialog) = document_dialogs(&self.model, cx)
            .borrow()
            .edit_annotation()
            .cloned()
        else {
            return div().into_any_element();
        };
        let title = if dialog.is_new() {
            localized(cx, "Add annotation", "Добавить аннотацию")
        } else {
            localized(cx, "Edit annotation", "Изменить аннотацию")
        };
        let intensity = intensity_picker(
            "edit-annotation-intensity",
            intensity_label(dialog.intensity),
            self.model.clone(),
            cx,
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
                category_choices(primary_category_options().iter().copied()),
                localized(cx, "Primary category", "Основная категория"),
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
                category_choices(secondary_category_options(&dialog.primary_selection)),
                localized(cx, "Secondary category", "Дополнительная категория"),
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
                        .child(div().flex_1().child(field(
                            localized(cx, "Primary category *", "Основная категория *"),
                            primary,
                            cx,
                        )))
                        .when(show_secondary, |this| {
                            this.child(div().flex_1().child(field(
                                localized(cx, "Secondary category *", "Дополнительная категория *"),
                                secondary,
                                cx,
                            )))
                        }),
                )
                .when(needs_description, |this| {
                    this.child(field(
                        localized(cx, "Description *", "Описание *"),
                        Input::new(&self.description),
                        cx,
                    ))
                })
        } else {
            v_flex().child(field(
                localized(cx, "Category", "Категория"),
                div()
                    .text_sm()
                    .child(localized(cx, "Background noise", "Фоновый шум")),
                cx,
            ))
        };
        let model = self.model.clone();
        v_flex()
            .gap_3()
            .child(dialog_header(title, None, cx))
            .child(categories)
            .child(field(
                localized(cx, "Intensity *", "Интенсивность *"),
                intensity,
                cx,
            ))
            .children(dialog.error.map(|error| error_text(error.to_string(), cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("edit-annotation-cancel")
                            .label(localized(cx, "Cancel", "Отмена"))
                            .on_click(move |_, window, cx| {
                                cancel(&model, cx);
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        primary_button(
                            "edit-annotation-save",
                            localized(cx, "Save", "Сохранить"),
                            false,
                            false,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
            .into_any_element()
    }
}

fn cancel(model: &Entity<EditorModel>, cx: &mut App) {
    update_dialog_state(model, cx, |dialogs| dialogs.cancel_edit_annotation());
}

/// Opens the document's current Babel edit-annotation state.
pub fn open(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| EditAnnotationView::new(model.clone(), window, cx));
    let model = model.clone();
    window.open_dialog(cx, move |dialog, window, _| {
        let model = model.clone();
        let submit = view.clone();
        dialog
            .w(rems(32.5).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| {
                cancel(&model, cx);
                true
            })
            .on_ok(move |_, window, cx| {
                submit.update(cx, |view, cx| view.submit(window, cx));
                false
            })
            .child(view.clone())
    });
}
