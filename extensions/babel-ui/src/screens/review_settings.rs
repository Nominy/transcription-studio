//! Review preferences belong to Babel and remain available without Gold UI enabled.

use exaudio_gold::prefs::{AiPreferences, AiSettings};
use exaudio_shell::{screens::ui, services::AppServices};
use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::text::{localized, localized_fmt};

const REVIEW_MODEL_SUGGESTIONS: &str = "cloudflare/clef-omni, typesafe/jev-1.14";

pub struct ReviewSettingsView {
    key: Entity<InputState>,
    model: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl ReviewSettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.global::<AppServices>().storage.ai_settings();
        let key = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "OpenRouter key", "Ключ OpenRouter"))
                .masked(true)
                .default_value(settings.review_openrouter_key)
        });
        let model = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Model (optional)", "Модель (необязательно)"))
                .default_value(settings.review_model)
        });
        let subscriptions = vec![
            cx.subscribe_in(&key, window, |this, input, event, window, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().trim().to_string();
                    this.update(window, cx, |settings| {
                        settings.review_openrouter_key = value
                    });
                }
            }),
            cx.subscribe_in(&model, window, |this, input, event, window, cx| {
                if let InputEvent::Change = event {
                    let value = input.read(cx).value().trim().to_string();
                    this.update(window, cx, |settings| settings.review_model = value);
                }
            }),
        ];
        Self {
            key,
            model,
            _subscriptions: subscriptions,
        }
    }

    fn update(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut AiSettings),
    ) {
        let storage = &mut cx.global_mut::<AppServices>().storage;
        let mut settings = storage.ai_settings();
        change(&mut settings);
        if let Err(error) = storage.set_ai_settings(&settings.normalized()) {
            window.push_notification(
                ui::warning_notification(
                    localized(
                        cx,
                        "Couldn't save review settings.",
                        "Не удалось сохранить настройки ревью.",
                    ),
                    Some(error.to_string().into()),
                ),
                cx,
            );
        }
        cx.notify();
    }
}

fn key_hint(key: &str, cx: &App) -> Option<&'static str> {
    if key.is_empty() {
        return None;
    }
    let valid =
        key.starts_with("sk-or-") && key.len() <= 512 && !key.chars().any(char::is_whitespace);
    (!valid).then(|| localized(cx, "Keys start with sk-or-.", "Ключ начинается с sk-or-."))
}

impl Render for ReviewSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<AppServices>().storage.ai_settings();
        ui::card(
            localized(cx, "AI review", "ИИ-ревью"),
            Some(localized(
                cx,
                "Used by Magic Review and the Grader.",
                "Используется Magic Review и Grader.",
            )),
            cx,
        )
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(div().text_sm().child("Magic Review"))
                .child(
                    ButtonGroup::new("ai-review-mode")
                        .outline()
                        .small()
                        .child(
                            Button::new("ai-review-interactive")
                                .label(localized(cx, "Step by step", "Пошагово"))
                                .selected(!settings.review_fast_mode),
                        )
                        .child(
                            Button::new("ai-review-fast")
                                .label(localized(cx, "All at once", "Сразу всё"))
                                .selected(settings.review_fast_mode),
                        )
                        .on_click(cx.listener(|this, clicked: &Vec<usize>, window, cx| {
                            let fast = clicked.contains(&1);
                            this.update(window, cx, |settings| settings.review_fast_mode = fast);
                        })),
                ),
        )
        .child(
            v_flex()
                .gap_2()
                .child(Input::new(&self.key).mask_toggle())
                .when_some(
                    key_hint(&settings.review_openrouter_key, cx),
                    |view, hint| {
                        view.child(div().text_xs().text_color(cx.theme().warning).child(hint))
                    },
                )
                .child(Input::new(&self.model))
                .child(ui::muted(
                    localized_fmt(
                        cx,
                        "Suggested: {models}",
                        "Рекомендуемые: {models}",
                        &[("models", REVIEW_MODEL_SUGGESTIONS)],
                    ),
                    cx,
                )),
        )
    }
}
