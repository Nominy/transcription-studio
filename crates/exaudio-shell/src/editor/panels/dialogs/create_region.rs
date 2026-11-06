//! Plain-text segment creation. Domain forms are contributed through command effects.
use crate::editor::model::EditorModel;
use crate::screens::ui::localized;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
struct RegionDialog {
    model: Entity<EditorModel>,
    text: Entity<InputState>,
    start: Entity<InputState>,
    end: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}
impl RegionDialog {
    fn new(model: Entity<EditorModel>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = model
            .read(cx)
            .doc()
            .create_region()
            .expect("open region dialog")
            .clone();
        let text = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(state.text)
                .placeholder(localized(cx, "What was said…", "Что было сказано…"))
        });
        let start = cx.new(|cx| InputState::new(window, cx).default_value(state.start_draft));
        let end = cx.new(|cx| InputState::new(window, cx).default_value(state.end_draft));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe(&text, |this, input, event, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.model.update(cx, |model, cx| {
                        model.run(cx, |doc| doc.set_create_region_text(value))
                    });
                }
            }),
            cx.subscribe(&start, |this, input, event, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| {
                            if let Some(dialog) = doc.create_region_mut() {
                                dialog.set_start_draft(value);
                            }
                        })
                    });
                }
            }),
            cx.subscribe(&end, |this, input, event, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().to_string();
                    this.model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| {
                            if let Some(dialog) = doc.create_region_mut() {
                                dialog.set_end_draft(value);
                            }
                        })
                    });
                }
            }),
        ];
        Self {
            model,
            text,
            start,
            end,
            _subscriptions: subscriptions,
        }
    }
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.model.update(cx, |model, cx| {
            let result = model.update_doc(cx, |doc| doc.submit_create_region());
            let ok = result.is_ok();
            model.apply_result(result, cx);
            ok
        });
        if result {
            window.close_dialog(cx);
        }
    }
}
impl Render for RegionDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.model.read(cx).doc().create_region().cloned();
        let selected = state
            .as_ref()
            .map(|d| d.selected_lane_id.clone())
            .unwrap_or_default();
        let lanes = self.model.read(cx).doc().project().lanes.clone();
        let selected_label = lanes
            .iter()
            .find(|lane| lane.id == selected)
            .map(|lane| lane.label.clone())
            .unwrap_or(selected);
        let model = self.model.clone();
        let tracks = Button::new("region-track")
            .label(selected_label)
            .dropdown_menu(move |mut menu, _, _| {
                for lane in &lanes {
                    let model = model.clone();
                    let id = lane.id.clone();
                    menu = menu.item(PopupMenuItem::new(lane.label.clone()).on_click(
                        move |_, _, cx| {
                            model.update(cx, |model, cx| {
                                model.update_doc(cx, |doc| {
                                    if let Some(dialog) = doc.create_region_mut() {
                                        dialog.selected_lane_id = id.clone();
                                    }
                                })
                            })
                        },
                    ));
                }
                menu
            });
        v_flex()
            .gap_3()
            .child(super::super::ui::dialog_header(
                localized(cx, "New segment", "Новый сегмент"),
                None,
                cx,
            ))
            .child(tracks)
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&self.start))
                    .child(Input::new(&self.end)),
            )
            .child(Input::new(&self.text))
            .children(
                self.model
                    .read(cx)
                    .status()
                    .cloned()
                    .map(|status| super::super::ui::error_text(status, cx)),
            )
            .child(
                super::super::ui::dialog_footer()
                    .child(
                        Button::new("region-cancel")
                            .small()
                            .label(localized(cx, "Cancel", "Отмена"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.model.update(cx, |model, cx| {
                                    model.update_doc(cx, |doc| doc.cancel_create_region())
                                });
                                window.close_dialog(cx);
                            })),
                    )
                    .child(
                        Button::new("region-create")
                            .small()
                            .primary()
                            .label(localized(cx, "Add", "Добавить"))
                            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                    ),
            )
    }
}
pub fn open(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    let model = model.clone();
    let view = cx.new(|cx| RegionDialog::new(model.clone(), window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        let model = model.clone();
        dialog
            .close_button(false)
            .on_cancel(move |_, _, cx| {
                model.update(cx, |model, cx| {
                    model.update_doc(cx, |doc| doc.cancel_create_region())
                });
                true
            })
            .child(view.clone())
    });
}
