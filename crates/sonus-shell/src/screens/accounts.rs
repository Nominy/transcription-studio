//! Provider surfaces: the sign-in step shown while opening a mode (or on demand), the
//! account chooser for `multiple` provider types, and the Settings › Accounts card.
//! Providers themselves render the sign-in form; the shell supplies chrome and exits.

use super::ui::{self, localized};
use crate::{
    ProviderContribution, ProviderInstance, ProviderRegistry, ProviderSetup, ProviderStatus,
    ShellHandle,
};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::{cell::RefCell, rc::Rc};

/// Receives the outcome of a setup step: `Some` after `ProviderSetup::done` (already
/// upserted into the registry), `None` after cancel. Called exactly once.
pub type SetupOutcome = crate::providers::ProviderSetupOutcome;

/// What one setup run needs besides the provider type: which instance to re-authenticate
/// (if any), the cancel wording, and the completion.
pub struct SetupRequest {
    pub instance: Option<ProviderInstance>,
    pub cancel_label: &'static str,
    pub on_done: SetupOutcome,
}
pub type AddHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub type PickHandler = Rc<dyn Fn(ProviderInstance, &mut Window, &mut App)>;

/// Shell chrome around a provider's setup view: heading, the view, and a cancel button.
pub struct SetupPage {
    title: SharedString,
    view: AnyView,
    setup: ProviderSetup,
    cancel_label: &'static str,
}

impl SetupPage {
    pub fn new(
        shell: ShellHandle,
        provider: &ProviderContribution,
        title: impl Into<SharedString>,
        request: SetupRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<Self> {
        let SetupRequest {
            instance,
            cancel_label,
            on_done,
        } = request;
        let outcome = Rc::new(RefCell::new(Some(on_done)));
        let done = {
            let outcome = outcome.clone();
            move |instance: ProviderInstance, window: &mut Window, cx: &mut App| {
                let Some(done) = outcome.borrow_mut().take() else {
                    return;
                };
                cx.update_global::<ProviderRegistry, _>(|registry, cx| {
                    registry.upsert(instance.clone(), cx)
                });
                done(Some(instance), window, cx);
            }
        };
        let cancel = move |window: &mut Window, cx: &mut App| {
            if let Some(done) = outcome.borrow_mut().take() {
                done(None, window, cx);
            }
        };
        let setup = ProviderSetup::new(provider.id.clone(), instance, done, cancel);
        let view = (provider.setup)(shell, setup.clone(), window, cx);
        cx.new(|_| Self {
            title: title.into(),
            view,
            setup,
            cancel_label,
        })
    }
}

impl Render for SetupPage {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let setup = self.setup.clone();
        v_flex()
            .size_full()
            .p_6()
            .gap_4()
            .child(div().text_xl().child(self.title.clone()))
            .child(div().flex_1().min_h_0().child(self.view.clone()))
            .child(
                h_flex().justify_end().child(
                    Button::new("provider-setup-cancel")
                        .outline()
                        .small()
                        .label(self.cancel_label)
                        .on_click(move |_, window, cx| setup.cancel(window, cx)),
                ),
            )
    }
}

/// Opens the provider's setup in a dialog on top of the current screen. `title` defaults
/// to "Sign in · {provider}". Unknown provider types report an error and resolve the
/// outcome with `None` immediately.
pub fn open_setup_dialog(
    shell: ShellHandle,
    provider_id: &str,
    title: Option<String>,
    request: SetupRequest,
    window: &mut Window,
    cx: &mut App,
) {
    let SetupRequest {
        instance,
        cancel_label,
        on_done,
    } = request;
    let Some(provider) = cx
        .global::<ProviderRegistry>()
        .provider_type(provider_id)
        .cloned()
    else {
        window.push_notification(
            ui::error_notification(
                localized(
                    cx,
                    "This account type isn't installed.",
                    "Этот тип аккаунта не установлен.",
                ),
                Some(provider_id.to_string().into()),
            ),
            cx,
        );
        on_done(None, window, cx);
        return;
    };
    let on_done: SetupOutcome = Box::new(move |instance, window, cx| {
        window.close_dialog(cx);
        on_done(instance, window, cx);
    });
    let title = title
        .unwrap_or_else(|| format!("{} · {}", localized(cx, "Sign in", "Вход"), provider.label));
    let page = SetupPage::new(
        shell,
        &provider,
        title,
        SetupRequest {
            instance,
            cancel_label,
            on_done,
        },
        window,
        cx,
    );
    window.open_dialog(cx, move |dialog, window, _| {
        let width = (window.viewport_size().width - px(64.)).min(px(960.));
        let height = (window.viewport_size().height - px(160.)).max(px(240.));
        dialog
            .overlay_closable(false)
            .keyboard(false)
            .w(width)
            .child(div().h(height).min_h_0().child(page.clone()))
    });
}

pub fn status_detail(status: &ProviderStatus, cx: &App) -> String {
    match status {
        ProviderStatus::Ready { detail } if detail.is_empty() => {
            localized(cx, "Signed in", "Выполнен вход").into()
        }
        ProviderStatus::Ready { detail } => detail.clone(),
        ProviderStatus::NeedsSignIn => localized(cx, "Sign in again", "Войдите снова").into(),
        ProviderStatus::Error(error) => error.clone(),
    }
}

/// One instance with its status detail, computed once per view (provider `status` calls
/// may be slow, e.g. a WASM guest check).
#[derive(Clone)]
pub struct AccountRow {
    pub instance: ProviderInstance,
    pub detail: String,
    pub current: bool,
}
#[derive(Clone)]
pub struct AccountSection {
    pub provider: ProviderContribution,
    pub rows: Vec<AccountRow>,
}

pub fn section(provider: &ProviderContribution, cx: &App) -> AccountSection {
    let registry = cx.global::<ProviderRegistry>();
    let selected = registry.selected(&provider.id);
    let rows = registry
        .instances(&provider.id)
        .into_iter()
        .map(|instance| AccountRow {
            detail: status_detail(&(provider.status)(&instance, cx), cx),
            current: selected.as_ref() == Some(&instance),
            instance,
        })
        .collect();
    AccountSection {
        provider: provider.clone(),
        rows,
    }
}

/// Every provider type that is `multiple` or has at least one instance.
pub fn sections(cx: &App) -> Vec<AccountSection> {
    let registry = cx.global::<ProviderRegistry>();
    registry
        .types()
        .filter(|provider| provider.multiple || !registry.instances(&provider.id).is_empty())
        .map(|provider| section(provider, cx))
        .collect()
}

/// Lists a provider type's instances with their status; picking one calls `on_pick`.
pub struct Chooser {
    title: SharedString,
    section: AccountSection,
    on_pick: PickHandler,
    on_add: Option<AddHandler>,
}

impl Chooser {
    pub fn new(
        title: impl Into<SharedString>,
        provider: &ProviderContribution,
        on_pick: impl Fn(ProviderInstance, &mut Window, &mut App) + 'static,
        on_add: Option<AddHandler>,
        cx: &App,
    ) -> Self {
        Self {
            title: title.into(),
            section: section(provider, cx),
            on_pick: Rc::new(on_pick),
            on_add,
        }
    }

    fn list(&self, cx: &App) -> Div {
        v_flex()
            .gap_2()
            .children(self.section.rows.iter().enumerate().map(|(ix, row)| {
                let on_pick = self.on_pick.clone();
                let instance = row.instance.clone();
                Button::new(("provider-instance", ix))
                    .w_full()
                    .outline()
                    .when(row.current, |b| b.primary())
                    .child(
                        h_flex()
                            .w_full()
                            .gap_3()
                            .justify_between()
                            .child(row.instance.label.clone())
                            .child(ui::muted(row.detail.clone(), cx)),
                    )
                    .on_click(move |_, window, cx| on_pick(instance.clone(), window, cx))
            }))
            .when_some(self.on_add.clone(), |this, on_add| {
                this.child(
                    Button::new("provider-add")
                        .small()
                        .label(localized(cx, "Add another account", "Добавить ещё аккаунт"))
                        .on_click(move |_, window, cx| on_add(window, cx)),
                )
            })
    }
}

impl Render for Chooser {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_6()
            .gap_4()
            .child(div().text_xl().child(self.title.clone()))
            .child(self.list(cx).max_w(px(560.)))
    }
}

/// Dialog that switches the selected instance of a provider type (Start tile chip).
pub fn open_switch_dialog(provider_id: &str, window: &mut Window, cx: &mut App) {
    let Some(provider) = cx
        .global::<ProviderRegistry>()
        .provider_type(provider_id)
        .cloned()
    else {
        return;
    };
    let title = localized(cx, "Switch account", "Сменить аккаунт");
    let chooser = cx.new(|cx| {
        Chooser::new(
            title,
            &provider,
            |instance, window, cx| {
                cx.update_global::<ProviderRegistry, _>(|registry, cx| {
                    registry.select(&instance, cx)
                });
                window.close_dialog(cx);
                cx.refresh_windows();
            },
            None,
            cx,
        )
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog.w(px(560.)).child(chooser.clone())
    });
}

/// Settings › Accounts: instances per provider type with status, sign-out and, for
/// `multiple` types, "Add another account". `sections` come from [`sections`], cached by
/// the caller; `on_change` runs after a sign-out or a completed sign-in.
pub fn accounts_card(
    shell: ShellHandle,
    sections: &[AccountSection],
    on_change: Rc<dyn Fn(&mut App)>,
    cx: &App,
) -> Div {
    let card = ui::card(localized(cx, "Accounts", "Аккаунты"), None, cx);
    if sections.is_empty() {
        return card.child(ui::muted(
            localized(cx, "No accounts yet.", "Аккаунтов пока нет."),
            cx,
        ));
    }
    card.children(sections.iter().enumerate().map(|(type_ix, section)| {
        let provider = &section.provider;
        let add_shell = shell.clone();
        let add_provider = provider.id.clone();
        let add_change = on_change.clone();
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(provider.label.clone()),
            )
            .children(section.rows.iter().enumerate().map(|(ix, row)| {
                let removed = row.instance.clone();
                let on_change = on_change.clone();
                h_flex()
                    .gap_3()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(row.instance.label.clone())
                            .child(ui::muted(row.detail.clone(), cx))
                            .when(row.current && provider.multiple, |this| {
                                this.child(ui::muted(
                                    localized(cx, "(in use)", "(используется)"),
                                    cx,
                                ))
                            }),
                    )
                    .child(
                        Button::new(("provider-sign-out", type_ix * 64 + ix))
                            .outline()
                            .small()
                            .label(localized(cx, "Sign out", "Выйти"))
                            .on_click(move |_, _, cx| {
                                let removed = removed.clone();
                                cx.update_global::<ProviderRegistry, _>(|registry, cx| {
                                    registry.remove(&removed, cx)
                                });
                                on_change(cx);
                                cx.refresh_windows();
                            }),
                    )
            }))
            .when(provider.multiple, |this| {
                this.child(
                    Button::new(("provider-add", type_ix))
                        .small()
                        .label(localized(cx, "Add another account", "Добавить ещё аккаунт"))
                        .on_click(move |_, window, cx| {
                            let on_change = add_change.clone();
                            open_setup_dialog(
                                add_shell.clone(),
                                &add_provider,
                                None,
                                SetupRequest {
                                    instance: None,
                                    cancel_label: localized(cx, "Cancel", "Отмена"),
                                    on_done: Box::new(move |_, _, cx| {
                                        on_change(cx);
                                        cx.refresh_windows()
                                    }),
                                },
                                window,
                                cx,
                            )
                        }),
                )
            })
    }))
}
