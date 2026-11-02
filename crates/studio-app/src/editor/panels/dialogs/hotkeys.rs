//! Keyboard shortcuts dialog: every editor action by section, with rebinding (key capture),
//! conflict warnings and reset per action or for all. Overrides persist via
//! [`ActiveKeymap::update`].

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::EditorAction;
use studio_editor::edit::keymap::{ACTION_DEFS, ActionDef, KeyChord};

use crate::editor::panels::ui::{caption, dialog_footer, dialog_header, muted};
use crate::keymap::{ActiveKeymap, IS_MAC, keystroke_string};

const HINT: &str =
    "Shortcuts use physical key positions, so the same keys work across keyboard layouts.";
const CAPTURE_HINT: &str =
    "Press the new key combination for the highlighted action, or Cancel. Escape can be bound too.";

/// Keys that only modify another key; a binding needs a real key.
const MODIFIER_KEYS: &[&str] = &[
    "shift", "control", "ctrl", "alt", "option", "platform", "cmd", "super", "fn", "function",
];

#[derive(Clone)]
enum Notice {
    Info(SharedString),
    Warning(SharedString),
    Error(SharedString),
}

struct HotkeysDialog {
    focus_handle: FocusHandle,
    capturing: Option<EditorAction>,
    notice: Option<Notice>,
}

impl HotkeysDialog {
    fn start_capture(&mut self, action: EditorAction, window: &mut Window, cx: &mut Context<Self>) {
        self.capturing = Some(action);
        self.notice = Some(Notice::Info("Press the new key combination.".into()));
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        if self.capturing.take().is_some() {
            self.notice = Some(Notice::Info("Hotkey change canceled.".into()));
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.capturing else {
            return;
        };
        cx.stop_propagation();
        if MODIFIER_KEYS.contains(&event.keystroke.key.as_str()) {
            return;
        }
        let keystroke = keystroke_string(&event.keystroke);
        let conflicts = cx
            .global::<ActiveKeymap>()
            .0
            .conflicts_with(action, &keystroke, IS_MAC);
        let result = ActiveKeymap::update(cx, |keymap| keymap.set_override(action, &keystroke));
        self.notice = Some(match result {
            Ok(Ok(())) if conflicts.is_empty() => Notice::Info("Hotkey updated.".into()),
            Ok(Ok(())) => Notice::Warning(
                format!(
                    "Hotkey updated. {} is also used by: {}.",
                    chord_label(&keystroke),
                    conflicts
                        .iter()
                        .map(|other| other.def().description)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
                .into(),
            ),
            Ok(Err(error)) => Notice::Error(error.to_string().into()),
            Err(error) => {
                Notice::Error(format!("Hotkey preference could not be saved: {error}").into())
            }
        });
        self.capturing = None;
        cx.notify();
    }

    fn reset(&mut self, action: EditorAction, cx: &mut Context<Self>) {
        if self.capturing == Some(action) {
            self.capturing = None;
        }
        self.notice = Some(
            match ActiveKeymap::update(cx, |keymap| keymap.reset_override(action)) {
                Ok(()) => Notice::Info("Hotkey reset to default.".into()),
                Err(error) => {
                    Notice::Error(format!("Hotkey preference could not be saved: {error}").into())
                }
            },
        );
        cx.notify();
    }

    fn reset_all(&mut self, cx: &mut Context<Self>) {
        self.capturing = None;
        let result = ActiveKeymap::update(cx, |keymap| {
            for action in EditorAction::ALL {
                keymap.reset_override(*action);
            }
        });
        self.notice = Some(match result {
            Ok(()) => Notice::Info("All hotkeys reset to defaults.".into()),
            Err(error) => {
                Notice::Error(format!("Hotkey preference could not be saved: {error}").into())
            }
        });
        cx.notify();
    }

    fn render_row(
        &self,
        def: &ActionDef,
        conflicted: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let keymap = &cx.global::<ActiveKeymap>().0;
        let labels = keymap.labels(def.action, IS_MAC);
        let overridden = keymap.is_overridden(def.action);
        let capturing = self.capturing == Some(def.action);
        let action = def.action;
        let theme = cx.theme();
        let keys = h_flex().gap_1().flex_wrap().children(if capturing {
            vec![
                div()
                    .text_xs()
                    .text_color(theme.primary)
                    .child("Press new key combination...")
                    .into_any_element(),
            ]
        } else {
            labels
                .into_iter()
                .map(|label| keycap(label, cx).into_any_element())
                .collect()
        });
        h_flex()
            .id(SharedString::from(format!("hotkey-{}", def.action.id())))
            .px_3()
            .py_1p5()
            .gap_3()
            .rounded(theme.radius)
            .when(capturing, |this| this.bg(theme.accent))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(div().text_sm().child(def.description))
                    .when(overridden, |this| {
                        this.child(div().text_xs().text_color(theme.info).child("custom"))
                    })
                    .when(conflicted, |this| {
                        this.child(div().text_xs().text_color(theme.warning).child("conflict"))
                    }),
            )
            .child(div().w(rems(13.75)).flex_none().child(keys))
            .child(
                h_flex()
                    .w(rems(9.375))
                    .flex_none()
                    .gap_1()
                    .justify_end()
                    .when(def.configurable, |this| {
                        this.child(if capturing {
                            Button::new("cancel")
                                .xsmall()
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_capture(cx)))
                        } else {
                            Button::new("change")
                                .xsmall()
                                .outline()
                                .label("Change")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_capture(action, window, cx);
                                }))
                        })
                        .child(
                            Button::new("reset")
                                .xsmall()
                                .ghost()
                                .label("Reset")
                                .disabled(!overridden)
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.reset(action, cx)),
                                ),
                        )
                    })
                    .when(!def.configurable, |this| this.child(muted("fixed", cx))),
            )
    }
}

fn chord_label(keystroke: &str) -> String {
    KeyChord::parse(keystroke).map_or_else(|| keystroke.to_string(), |chord| chord.label(IS_MAC))
}

fn keycap(label: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .py_0p5()
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_xs()
        .font_family(theme.mono_font_family.clone())
        .child(label)
}

impl Render for HotkeysDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The list keeps its height unless the window is too short for it and the dialog
        // chrome (header, notices, footer).
        let rem = window.rem_size();
        let list_height = rems(28.75)
            .to_pixels(rem)
            .min(window.viewport_size().height - rems(16.).to_pixels(rem))
            .max(rems(8.).to_pixels(rem));
        let keymap = &cx.global::<ActiveKeymap>().0;
        let conflicts = keymap.conflicts(IS_MAC);
        let any_overridden = EditorAction::ALL
            .iter()
            .any(|action| keymap.is_overridden(*action));
        let conflicted = |action: EditorAction| {
            conflicts
                .iter()
                .any(|conflict| conflict.first == action || conflict.second == action)
        };
        let conflict_lines: Vec<SharedString> = conflicts
            .iter()
            .map(|conflict| {
                format!(
                    "{}: {} / {}",
                    conflict.chord.label(IS_MAC),
                    conflict.first.def().description,
                    conflict.second.def().description
                )
                .into()
            })
            .collect();

        let mut sections: Vec<(&'static str, Vec<&ActionDef>)> = Vec::new();
        for def in ACTION_DEFS {
            match sections.last_mut() {
                Some((section, defs)) if *section == def.section => defs.push(def),
                _ => sections.push((def.section, vec![def])),
            }
        }
        let mut list = v_flex().gap_3();
        for (section, defs) in sections {
            let mut group = v_flex()
                .gap_0p5()
                .child(div().px_3().child(caption(section, cx)));
            for def in defs {
                let row = self.render_row(def, conflicted(def.action), cx);
                group = group.child(row);
            }
            list = list.child(group);
        }

        let theme = cx.theme();
        let notice = self.notice.clone().map(|notice| {
            let (text, color) = match notice {
                Notice::Info(text) => (text, theme.muted_foreground),
                Notice::Warning(text) => (text, theme.warning),
                Notice::Error(text) => (text, theme.danger),
            };
            div().text_xs().text_color(color).child(text)
        });
        let hint = if self.capturing.is_some() {
            CAPTURE_HINT
        } else {
            HINT
        };
        v_flex()
            .id("hotkeys-dialog")
            .track_focus(&self.focus_handle)
            .capture_key_down(cx.listener(Self::on_key_down))
            .gap_3()
            .child(dialog_header("Keyboard Shortcuts", Some(hint.into()), cx))
            .when(!conflict_lines.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_0p5()
                        .p_2()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.warning)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.warning)
                                .child("Conflicting shortcuts"),
                        )
                        .children(conflict_lines.into_iter().map(|line| muted(line, cx))),
                )
            })
            .children(notice)
            .child(
                div()
                    .id("hotkeys-list")
                    .h(list_height)
                    .overflow_y_scrollbar()
                    .child(list),
            )
            .child(
                dialog_footer()
                    .child(
                        Button::new("hotkeys-reset-all")
                            .ghost()
                            .label("Reset All")
                            .disabled(!any_overridden)
                            .on_click(cx.listener(|this, _, _, cx| this.reset_all(cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("hotkeys-close")
                            .primary()
                            .label("Close")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    }
}

/// Opens the keyboard shortcuts dialog.
pub fn open(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| HotkeysDialog {
        focus_handle: cx.focus_handle(),
        capturing: None,
        notice: None,
    });
    window.open_dialog(cx, move |dialog, window, _| {
        // While a key is being captured Escape is a candidate binding, not "close".
        let guard = view.clone();
        dialog
            .w(rems(47.5).to_pixels(window.rem_size()))
            .close_button(false)
            .on_cancel(move |_, _, cx| guard.read(cx).capturing.is_none())
            .child(view.clone())
    });
}
