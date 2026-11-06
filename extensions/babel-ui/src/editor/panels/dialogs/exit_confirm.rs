//! "Unsaved changes" confirmation shown when leaving a dirty task.

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{WindowExt, v_flex};
use gpui_kit::*;

use crate::editor::panels::task::TaskPanels;
use crate::text::{failed, localized};
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::editor::panels::ui::{dialog_footer, dialog_header, error_text, primary_button};

struct ExitConfirmDialog {
    panels: Entity<TaskPanels>,
    _subscriptions: Vec<Subscription>,
}

impl Render for ExitConfirmDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panels = self.panels.read(cx);
        let session = panels.model().read(cx).doc().session();
        let saving = session.is_saving;
        let error = session
            .save_has_error
            .then(|| match session.save_error_message.as_deref() {
                Some(message) => failed(cx, "Couldn't save.", "Не удалось сохранить.", message),
                None => localized(
                    cx,
                    "Couldn't save. Try again.",
                    "Не удалось сохранить. Попробуйте ещё раз.",
                )
                .to_string(),
            });
        let description = if saving {
            localized(cx, "Saving…", "Сохранение…")
        } else {
            localized(
                cx,
                "Save your changes before leaving, or discard them.",
                "Сохраните изменения перед выходом или отмените их.",
            )
        };
        let keep = self.panels.clone();
        let discard = self.panels.clone();
        let save = self.panels.clone();
        v_flex()
            .gap_3()
            .child(dialog_header(
                localized(cx, "Unsaved changes", "Несохранённые изменения"),
                Some(description.into()),
                cx,
            ))
            .children(error.map(|error| error_text(error, cx)))
            .child(
                dialog_footer()
                    .child(
                        Button::new("exit-keep")
                            .label(localized(cx, "Keep editing", "Продолжить правку"))
                            .on_click(move |_, window, cx| {
                                keep.update(cx, |panels, cx| panels.keep_editing(cx));
                                window.close_dialog(cx);
                            }),
                    )
                    .child(
                        Button::new("exit-discard")
                            .danger()
                            .label(localized(cx, "Discard changes", "Отменить изменения"))
                            .on_click(move |_, window, cx| {
                                discard
                                    .update(cx, |panels, cx| panels.discard_and_leave(window, cx));
                            }),
                    )
                    .child(
                        primary_button(
                            "exit-save",
                            if saving {
                                localized(cx, "Saving…", "Сохранение…")
                            } else {
                                localized(cx, "Save and leave", "Сохранить и выйти")
                            },
                            saving,
                            false,
                        )
                        .on_click(move |_, window, cx| {
                            save.update(cx, |panels, cx| panels.save_and_leave(window, cx));
                        }),
                    ),
            )
    }
}

/// Opens the dialog. Called from inside a `TaskPanels` update, so `panels` must not be read.
pub fn open(
    panels: Entity<TaskPanels>,
    model: Entity<EditorModel>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| ExitConfirmDialog {
        _subscriptions: vec![
            cx.observe(&panels, |_, _, cx| cx.notify()),
            cx.observe(&model, |_, _, cx| cx.notify()),
        ],
        panels: panels.clone(),
    });
    window.open_dialog(cx, move |dialog, window, _| {
        let keep = panels.clone();
        dialog
            .w(rems(31.25).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| {
                keep.update(cx, |panels, cx| panels.keep_editing(cx));
                true
            })
            .child(view.clone())
    });
}
