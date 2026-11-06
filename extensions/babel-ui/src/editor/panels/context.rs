//! Script Context and Original Script panels above the editor. Each collapses to its header;
//! the collapsed state persists in UI storage (`tx:hide-script-context`,
//! `tx:hide-original-script`).

use std::time::Duration;

use exaudio_babel_domain::editor_model::context_panels::{
    ScriptContext, original_script, script_context,
};
use exaudio_babel_services::prefs::{ORIGINAL_SCRIPT_HIDDEN_KEY, SCRIPT_CONTEXT_HIDDEN_KEY};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::text::{failed, localized};
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::services::AppServices;

const COPY_FEEDBACK: Duration = Duration::from_secs(2);

pub struct ContextPanels {
    model: Entity<EditorModel>,
    /// Extracted once per doc generation (metadata only changes on load/merge).
    cache: Option<(u64, Option<ScriptContext>, Option<SharedString>)>,
    copied: bool,
    copy_reset: Option<Task<()>>,
    _observe: Subscription,
}

impl ContextPanels {
    pub fn new(model: Entity<EditorModel>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&model, |_, _, cx| cx.notify()),
            model,
            cache: None,
            copied: false,
            copy_reset: None,
        }
    }

    fn content(&mut self, cx: &App) -> (Option<ScriptContext>, Option<SharedString>) {
        let doc = self.model.read(cx).doc();
        let generation = doc.generation();
        if self
            .cache
            .as_ref()
            .is_none_or(|(cached, ..)| *cached != generation)
        {
            let project = doc.project();
            let context = script_context(project).filter(|context| {
                context
                    .emotion
                    .as_deref()
                    .is_some_and(|e| !e.trim().is_empty())
                    || !context.fields.is_empty()
            });
            let script = original_script(project)
                .filter(|script| !script.trim().is_empty())
                .map(SharedString::from);
            self.cache = Some((generation, context, script));
        }
        let (_, context, script) = self.cache.as_ref().expect("cache filled above");
        (context.clone(), script.clone())
    }

    fn toggle(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let collapsed = !cx.global::<AppServices>().storage.is_collapsed(key);
        let result = cx
            .global_mut::<AppServices>()
            .storage
            .set_collapsed(key, collapsed);
        if let Err(error) = result {
            let message = failed(
                cx,
                "Couldn't save the panel setting.",
                "Не удалось сохранить настройку панели.",
                error,
            );
            self.model.update(cx, |model, cx| {
                model.set_status(message, cx);
            });
        }
        cx.notify();
    }

    fn copy_submission_id(&mut self, submission_id: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(submission_id));
        let message = localized(cx, "ID copied", "ID скопирован");
        self.model.update(cx, |model, cx| {
            model.set_status(message, cx);
        });
        self.copied = true;
        self.copy_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPY_FEEDBACK).await;
            this.update(cx, |this, cx| {
                this.copied = false;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn card(
        &self,
        key: &'static str,
        title: &'static str,
        body: impl IntoElement,
        cx: &mut Context<Self>,
    ) -> Div {
        let collapsed = cx.global::<AppServices>().storage.is_collapsed(key);
        let theme = cx.theme();
        let tooltip = match (key, collapsed) {
            (SCRIPT_CONTEXT_HIDDEN_KEY, true) => {
                localized(cx, "Show script context", "Показать контекст сценария")
            }
            (SCRIPT_CONTEXT_HIDDEN_KEY, false) => {
                localized(cx, "Hide script context", "Скрыть контекст сценария")
            }
            (_, true) => localized(cx, "Show original script", "Показать исходный сценарий"),
            (_, false) => localized(cx, "Hide original script", "Скрыть исходный сценарий"),
        };
        v_flex()
            .flex_1()
            .min_w_0()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .child(
                h_flex()
                    .h(rems(2.))
                    .pl_3()
                    .pr_1()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        Button::new(key)
                            .ghost()
                            .xsmall()
                            .icon(if collapsed {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronUp
                            })
                            .tooltip(tooltip)
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle(key, cx))),
                    ),
            )
            .when(!collapsed, |this| this.child(body))
    }

    fn script_context_body(
        &self,
        context: &ScriptContext,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let line = |label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .w(rems(5.25))
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(div().flex_1().min_w_0().text_sm().child(value))
        };
        let submission = context.submission_id.clone().map(|submission_id| {
            let copied = self.copied;
            let id = submission_id.clone();
            h_flex()
                .gap_2()
                .child(
                    div()
                        .w(rems(5.25))
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(localized(cx, "Submission ID", "ID задачи")),
                )
                .child(
                    div()
                        .text_sm()
                        .font_family(theme.mono_font_family.clone())
                        .child(submission_id),
                )
                .child(
                    Button::new("copy-submission-id")
                        .ghost()
                        .xsmall()
                        .icon(if copied {
                            IconName::Check
                        } else {
                            IconName::Copy
                        })
                        .label(if copied {
                            localized(cx, "Copied", "Скопировано")
                        } else {
                            localized(cx, "Copy", "Копировать")
                        })
                        .tooltip(localized(cx, "Copy submission ID", "Копировать ID задачи"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_submission_id(id.clone(), cx);
                        })),
                )
        });
        v_flex()
            .id("script-context-body")
            .max_h(rems(10.))
            .overflow_y_scrollbar()
            .px_3()
            .pb_2()
            .gap_1()
            .children(
                context
                    .emotion
                    .clone()
                    .filter(|emotion| !emotion.trim().is_empty())
                    .map(|emotion| line(localized(cx, "Emotion", "Эмоция"), emotion)),
            )
            .children(
                context
                    .fields
                    .iter()
                    .map(|field| line(field.label, field.value.clone())),
            )
            .children(submission)
    }
}

impl Render for ContextPanels {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (context, script) = self.content(cx);
        if context.is_none() && script.is_none() {
            return div().into_any_element();
        }
        let context_card = context.map(|context| {
            let body = self.script_context_body(&context, cx);
            let title = localized(cx, "Script context", "Контекст сценария");
            self.card(SCRIPT_CONTEXT_HIDDEN_KEY, title, body, cx)
        });
        let script_card = script.map(|script| {
            let body = div()
                .id("original-script-body")
                .max_h(rems(10.))
                .overflow_y_scrollbar()
                .px_3()
                .pb_2()
                .text_sm()
                .child(script);
            let title = localized(cx, "Original script", "Исходный сценарий");
            self.card(ORIGINAL_SCRIPT_HIDDEN_KEY, title, body, cx)
        });
        h_flex()
            .w_full()
            .flex_none()
            .gap_2()
            .items_start()
            .children(context_card)
            .children(script_card)
            .into_any_element()
    }
}
