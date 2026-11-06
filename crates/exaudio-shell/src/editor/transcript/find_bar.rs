//! Find bar floating at the top right of the transcript panes (over the grids, so opening or
//! closing it never moves a row): query input, match summary, previous/next and close. Find
//! state is doc-wide, so one bar serves every pane of the split layouts.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, IconName, Sizable as _, h_flex};
use gpui_kit::*;
use studio_editor::edit::EditorDoc;

use crate::editor::model::EditorModel;
use crate::screens::ui::localized;

pub struct FindBar {
    model: Entity<EditorModel>,
    input: Entity<InputState>,
    /// Doc state the open bar renders from; `None` while closed. The bar re-renders only when
    /// it changes (the model notifies every frame while playing).
    key: Option<FindKey>,
    /// Takes keyboard focus when the bar closes while its input has it.
    return_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Doc state behind the match summary.
#[derive(PartialEq)]
struct FindKey {
    generation: u64,
    selected: Option<String>,
}

impl FindKey {
    fn of(doc: &EditorDoc) -> Option<Self> {
        doc.find_state().open.then(|| Self {
            generation: doc.generation(),
            selected: doc.session().selected_segment_id.clone(),
        })
    }
}

impl FindBar {
    pub fn new(
        model: Entity<EditorModel>,
        return_focus: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(localized(
                cx,
                "Find in transcript",
                "Поиск в расшифровке",
            ))
        });
        let subscriptions = vec![
            cx.observe_in(&model, window, |this, _, window, cx| {
                this.on_model_changed(window, cx);
            }),
            cx.subscribe_in(&input, window, Self::on_input_event),
        ];
        let key = FindKey::of(model.read(cx).doc());
        Self {
            model,
            input,
            key,
            return_focus,
            _subscriptions: subscriptions,
        }
    }

    fn on_model_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let doc = self.model.read(cx).doc();
        let key = FindKey::of(doc);
        let opened = key.is_some() && self.key.is_none();
        let closed = key.is_none() && self.key.is_some();
        let query = opened.then(|| doc.find_state().query.clone());

        if let Some(query) = query {
            self.input.update(cx, |input, cx| {
                input.set_value(query, window, cx);
                input.select_all(window, cx);
                input.focus(window, cx);
            });
        } else if closed && self.input.focus_handle(cx).is_focused(window) {
            self.return_focus.focus(window, cx);
        }
        if key != self.key {
            self.key = key;
            cx.notify();
        }
    }

    fn step(&mut self, direction: i32, cx: &mut Context<Self>) {
        self.model.update(cx, |model, cx| {
            model.run(cx, |doc| {
                if direction < 0 {
                    doc.find_previous()
                } else {
                    doc.find_next()
                }
            });
        });
    }

    fn on_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = input.read(cx).value().to_string();
                if query == self.model.read(cx).doc().find_state().query {
                    return;
                }
                self.model.update(cx, |model, cx| {
                    model.run(cx, |doc| doc.set_find_query(query));
                });
                cx.notify();
            }
            InputEvent::PressEnter { shift, .. } => {
                self.step(if *shift { -1 } else { 1 }, cx);
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }
}

impl Render for FindBar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.key.is_none() {
            return div().into_any_element();
        }
        let theme = cx.theme();
        let summary: SharedString = match self.model.read(cx).doc().find_summary() {
            None => localized(cx, "Type to search", "Введите текст").into(),
            Some(summary) if summary.total == 0 => {
                localized(cx, "No matches", "Ничего не найдено").into()
            }
            Some(summary) => localized(cx, "{i} of {n}", "{i} из {n}")
                .replace("{i}", &summary.current.to_string())
                .replace("{n}", &summary.total.to_string())
                .into(),
        };

        h_flex()
            .id("find-bar")
            .occlude()
            .w(rems(32.))
            .max_w_full()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .shadow_md()
            .child(
                div()
                    .w(rems(17.5))
                    .min_w(rems(8.))
                    .child(Input::new(&self.input).small().cleanable(true)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(summary),
            )
            .child(div().flex_1())
            .child(
                Button::new("find-previous")
                    .icon(IconName::ChevronUp)
                    .ghost()
                    .small()
                    .tooltip(localized(cx, "Previous match", "Предыдущее"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.step(-1, cx);
                    })),
            )
            .child(
                Button::new("find-next")
                    .icon(IconName::ChevronDown)
                    .ghost()
                    .small()
                    .tooltip(localized(cx, "Next match", "Следующее"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.step(1, cx);
                    })),
            )
            .child(
                Button::new("find-close")
                    .icon(IconName::Close)
                    .ghost()
                    .small()
                    .tooltip(localized(cx, "Close", "Закрыть"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| doc.close_find());
                        });
                    })),
            )
            .into_any_element()
    }
}
