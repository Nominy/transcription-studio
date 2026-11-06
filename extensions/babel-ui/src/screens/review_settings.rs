//! Review preferences belong to Babel and remain available without Gold UI enabled.

use gpui_kit::component::button::{Button, ButtonGroup};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Selectable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sonus_gold::prefs::{AiPreferences, AiSettings};
use sonus_shell::{screens::ui, services::AppServices};

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
                .placeholder("Review OpenRouter API key (sk-or-…)")
                .masked(true)
                .default_value(settings.review_openrouter_key)
        });
        let model = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Review model (backend default)")
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
                Notification::warning(format!("Review settings could not be saved: {error}")),
                cx,
            );
        }
        cx.notify();
    }
}

fn key_hint(key: &str) -> Option<&'static str> {
    if key.is_empty() {
        return None;
    }
    let valid =
        key.starts_with("sk-or-") && key.len() <= 512 && !key.chars().any(char::is_whitespace);
    (!valid).then_some("OpenRouter keys start with sk-or- and contain no spaces.")
}

/// Review Helper model IDs use provider/model syntax, at most 128 bytes.
fn valid_review_model(model: &str) -> bool {
    let Some((provider, name)) = model.split_once('/') else {
        return false;
    };
    let part = |text: &str, extra: &[char]| {
        let mut chars = text.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
            && chars.all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
    };
    model.len() <= 128 && part(provider, &['.', '_', '-']) && part(name, &['.', '_', ':', '-'])
}

impl Render for ReviewSettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = cx.global::<AppServices>().storage.ai_settings();
        let model_hint = (!settings.review_model.is_empty()
            && !valid_review_model(&settings.review_model))
        .then_some("Model ids look like provider/model.");
        ui::card(
            "Review Helper",
            Some("Magic Review and the Grader use this OpenRouter key and model."),
            cx,
        )
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(div().text_sm().child("Magic Review mode"))
                .child(
                    ButtonGroup::new("ai-review-mode")
                        .outline()
                        .small()
                        .child(
                            Button::new("ai-review-interactive")
                                .label("Interactive")
                                .selected(!settings.review_fast_mode),
                        )
                        .child(
                            Button::new("ai-review-fast")
                                .label("Fast")
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
                .when_some(key_hint(&settings.review_openrouter_key), |view, hint| {
                    view.child(div().text_xs().text_color(cx.theme().warning).child(hint))
                })
                .child(Input::new(&self.model))
                .child(ui::muted(
                    format!("Suggested: {REVIEW_MODEL_SUGGESTIONS}"),
                    cx,
                ))
                .when_some(model_hint, |view, hint| {
                    view.child(div().text_xs().text_color(cx.theme().warning).child(hint))
                }),
        )
    }
}
