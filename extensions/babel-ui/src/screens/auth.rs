//! Babel account setup (the `exaudio.babel.account` provider's sign-in view): phone/e-mail
//! code login or a sub-account relay code. On success the credentials are stored in the
//! account's directory and the shell is told which instance was configured.

use std::sync::{Arc, LazyLock};

use exaudio_babel_services::account::{
    AccountKind, AccountRecord, AccountStore, direct_instance_id, relay_instance_id, relay_label,
};
use exaudio_babel_services::auth;
use exaudio_babel_services::auth::flow::{AuthFlow, AuthStartup};
use exaudio_babel_services::subaccounts;
use exaudio_shell::{ProviderInstance, ProviderSetup};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::screens::ShellModels;
use crate::text::{failed, localized};
use exaudio_shell::services::spawn_service_in;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Identifier,
    Code,
    HostUrl,
    SubaccountCode,
}

/// The account a direct login is being written into (known from the identifier onwards).
struct PendingAccount {
    id: String,
    label: String,
    store: AccountStore,
}

pub struct AuthScreen {
    setup: ProviderSetup,
    flow: AuthFlow,
    pending: Option<PendingAccount>,
    identifier: Entity<InputState>,
    code: Entity<InputState>,
    host_url: Entity<InputState>,
    subaccount_code: Entity<InputState>,
    request: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AuthScreen {
    pub fn new(setup: ProviderSetup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (flow, pending) = Self::initial_flow(&setup);
        let identifier = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Phone or e-mail", "Телефон или e-mail"))
                .default_value(flow.identifier.clone())
        });
        let code =
            cx.new(|cx| InputState::new(window, cx).placeholder(localized(cx, "Code", "Код")));
        let host_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(localized(cx, "Relay address", "Адрес ретранслятора"))
                .default_value(flow.subaccount_host_url.clone())
        });
        let subaccount_code =
            cx.new(|cx| InputState::new(window, cx).placeholder(localized(cx, "Code", "Код")));
        let subscriptions = vec![
            Self::watch(&identifier, Field::Identifier, window, cx),
            Self::watch(&code, Field::Code, window, cx),
            Self::watch(&host_url, Field::HostUrl, window, cx),
            Self::watch(&subaccount_code, Field::SubaccountCode, window, cx),
        ];
        let this = Self {
            setup,
            flow,
            pending,
            identifier,
            code,
            host_url,
            subaccount_code,
            request: None,
            _subscriptions: subscriptions,
        };
        this.focus_current(window, cx);
        this
    }

    /// Re-authenticating an existing account resumes where it left off (pending code →
    /// code step; relay → relay form with the host prefilled); a new account starts fresh.
    fn initial_flow(setup: &ProviderSetup) -> (AuthFlow, Option<PendingAccount>) {
        let Some(instance) = &setup.instance else {
            return (AuthFlow::login_prompt(), None);
        };
        let store = crate::root::account_store(instance);
        let record = store.read_record().ok().flatten();
        match record {
            Some(AccountRecord {
                kind: AccountKind::Relay,
                identifier,
                ..
            }) => {
                let mut flow = AuthFlow::subaccount_login_prompt();
                flow.subaccount_host_url = identifier;
                (flow, None)
            }
            Some(AccountRecord {
                kind: AccountKind::Direct,
                identifier,
                id,
                label,
            }) => {
                let mut flow = match AuthFlow::startup(&store.auth_path()) {
                    AuthStartup::Flow(flow) => flow,
                    AuthStartup::Authenticated => AuthFlow::login_prompt(),
                };
                flow.identifier = identifier;
                (flow, Some(PendingAccount { id, label, store }))
            }
            None => {
                let mut flow = AuthFlow::login_prompt();
                flow.identifier = instance.label.clone();
                (flow, None)
            }
        }
    }

    fn watch(
        input: &Entity<InputState>,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            input,
            window,
            move |this, input, event, window, cx| match event {
                InputEvent::Change => {
                    if this.flow.is_pending {
                        return;
                    }
                    let value = input.read(cx).value().to_string();
                    match field {
                        Field::Identifier => this.flow.identifier = value,
                        Field::Code => this.flow.code = value,
                        Field::HostUrl => this.flow.subaccount_host_url = value,
                        Field::SubaccountCode => this.flow.subaccount_code = value,
                    }
                    this.flow.error = None;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                _ => {}
            },
        )
    }

    fn focus_current(&self, window: &mut Window, cx: &mut Context<Self>) {
        let input = if self.flow.is_subaccount_mode() {
            if self.flow.subaccount_host_url.trim().is_empty() {
                &self.host_url
            } else {
                &self.subaccount_code
            }
        } else if self.flow.is_code_step() {
            &self.code
        } else {
            &self.identifier
        };
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn set_flow(&mut self, flow: AuthFlow, window: &mut Window, cx: &mut Context<Self>) {
        self.request = None;
        self.flow = flow;
        let host_url = self.flow.subaccount_host_url.clone();
        self.host_url
            .update(cx, |input, cx| input.set_value(host_url, window, cx));
        let identifier = self.flow.identifier.clone();
        self.identifier
            .update(cx, |input, cx| input.set_value(identifier, window, cx));
        for input in [&self.code, &self.subaccount_code] {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.flow.code.clear();
        self.flow.subaccount_code.clear();
        self.focus_current(window, cx);
        cx.notify();
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow.is_subaccount_mode() {
            self.subaccount_login(window, cx);
        } else if self.flow.is_code_step() {
            self.verify_code(window, cx);
        } else {
            self.start_login(window, cx);
        }
    }

    fn begin_request(&mut self, cx: &mut Context<Self>) {
        self.flow.is_pending = true;
        self.flow.error = None;
        cx.notify();
    }

    fn fail(&mut self, error: String, cx: &mut Context<Self>) {
        self.request = None;
        self.flow.is_pending = false;
        self.flow.error = Some(error);
        cx.notify();
    }

    /// The directory a new account's credentials go to, derived from the identity the
    /// provider instance will carry (`provider` + `id`); the label is what users see.
    fn account_for(&self, id: String, label: String) -> PendingAccount {
        let instance = ProviderInstance {
            provider: self.setup.provider.clone(),
            id,
            label,
        };
        PendingAccount {
            store: crate::root::account_store(&instance),
            id: instance.id,
            label: instance.label,
        }
    }

    fn start_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flow.can_start_login() {
            return;
        }
        let identifier = self.flow.identifier.trim().to_string();
        let account = self.account_for(direct_instance_id(&identifier), identifier.clone());
        let store_path = account.store.auth_path();
        self.pending = Some(account);
        self.begin_request(cx);
        self.request = Some(spawn_service_in(
            window,
            cx,
            async move { auth::start_login(&store_path, &identifier).await },
            |this, result, window, cx| match result {
                Ok(start) => {
                    let mut flow =
                        AuthFlow::pending_code(start.sign_in_id, Some(start.phone_number_id));
                    flow.identifier = this.flow.identifier.clone();
                    this.set_flow(flow, window, cx);
                }
                Err(error) => this.fail(
                    failed(cx, "Couldn't sign in", "Не удалось войти", error),
                    cx,
                ),
            },
        ));
    }

    fn verify_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flow.can_submit_code() {
            return;
        }
        let Some(account) = &self.pending else {
            self.fail(
                localized(cx, "Start the sign-in again.", "Начните вход заново.").into(),
                cx,
            );
            return;
        };
        let code = self.flow.code.trim().to_string();
        let store_path = account.store.auth_path();
        self.begin_request(cx);
        self.request = Some(spawn_service_in(
            window,
            cx,
            async move { auth::verify_login(&store_path, &code).await },
            |this, result, window, cx| match result {
                Ok(_) => this.finish_direct(window, cx),
                Err(error) => this.fail(
                    failed(cx, "Couldn't sign in", "Не удалось войти", error),
                    cx,
                ),
            },
        ));
    }

    fn finish_direct(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request = None;
        let Some(account) = self.pending.take() else {
            return;
        };
        let record = AccountRecord {
            id: account.id.clone(),
            label: account.label.clone(),
            kind: AccountKind::Direct,
            identifier: self.flow.identifier.trim().to_string(),
        };
        if let Err(error) = account.store.save_record(&record) {
            self.fail(
                failed(
                    cx,
                    "Couldn't save the sign-in",
                    "Не удалось сохранить вход",
                    error,
                ),
                cx,
            );
            return;
        }
        self.setup.done(account.id, account.label, window, cx);
    }

    /// "Use another number": forget the pending sign-in and start over.
    fn back_to_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow.is_pending {
            return;
        }
        let reset = match self.pending.take() {
            Some(account) => auth::reset_store(&account.store.auth_path()),
            None => Ok(()),
        };
        match reset {
            Ok(()) => {
                let mut flow = AuthFlow::login_prompt();
                flow.identifier = self.flow.identifier.clone();
                self.set_flow(flow, window, cx)
            }
            Err(error) => {
                self.flow.error = Some(failed(
                    cx,
                    "Couldn't restart the sign-in",
                    "Не удалось начать вход заново",
                    error,
                ));
                cx.notify();
            }
        }
    }

    fn switch_to_subaccount(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow.is_pending {
            return;
        }
        self.set_flow(AuthFlow::subaccount_login_prompt(), window, cx);
    }

    fn switch_to_babel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow.is_pending {
            return;
        }
        let mut flow = AuthFlow::login_prompt();
        flow.identifier = self.flow.identifier.clone();
        self.set_flow(flow, window, cx);
    }

    fn subaccount_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flow.can_submit_subaccount_login() {
            return;
        }
        let host_url = self.flow.subaccount_host_url.trim().to_string();
        let code = self.flow.subaccount_code.trim().to_string();
        self.begin_request(cx);
        self.request = Some(spawn_service_in(
            window,
            cx,
            async move { subaccounts::login_with_code(&host_url, &code).await },
            |this, result, window, cx| match result {
                Ok(session) => this.finish_relay(session, window, cx),
                Err(error) => this.fail(
                    failed(cx, "Couldn't sign in", "Не удалось войти", error),
                    cx,
                ),
            },
        ));
    }

    fn finish_relay(
        &mut self,
        session: subaccounts::SubaccountSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request = None;
        let account = self.account_for(
            relay_instance_id(&session.host_url, &session.subaccount_id),
            relay_label(&session),
        );
        let saved =
            subaccounts::save_session(&account.store.session_path(), &session).and_then(|()| {
                account.store.save_record(&AccountRecord {
                    id: account.id.clone(),
                    label: account.label.clone(),
                    kind: AccountKind::Relay,
                    identifier: session.host_url.clone(),
                })
            });
        if let Err(error) = saved {
            self.fail(
                failed(
                    cx,
                    "Couldn't save the sign-in",
                    "Не удалось сохранить вход",
                    error,
                ),
                cx,
            );
            return;
        }
        let relay = ShellModels::relay(cx);
        let remembered = relay.update(cx, |relay, cx| {
            relay.remember_host_url(&session.host_url, cx)
        });
        if let Err(error) = remembered {
            window.push_notification(
                Notification::warning(failed(
                    cx,
                    "Couldn't remember the relay address",
                    "Не удалось запомнить адрес ретранслятора",
                    error,
                )),
                cx,
            );
            relay.update(cx, |relay, cx| {
                relay.adopt_host_url(session.host_url.clone(), cx)
            });
        }
        self.setup.done(account.id, account.label, window, cx);
    }

    fn texts(&self, cx: &App) -> (&'static str, &'static str) {
        if self.flow.is_subaccount_mode() {
            (
                localized(cx, "Sub-account sign-in", "Вход в под-аккаунт"),
                localized(
                    cx,
                    "Enter the relay address and the code from the account owner.",
                    "Введите адрес ретранслятора и код от владельца аккаунта.",
                ),
            )
        } else if self.flow.is_code_step() {
            (
                localized(cx, "Enter the code", "Введите код"),
                localized(
                    cx,
                    "Babel sent a code to your phone.",
                    "Babel отправил код на ваш телефон.",
                ),
            )
        } else {
            (
                localized(cx, "Sign in to Babel", "Вход в Babel"),
                localized(
                    cx,
                    "Use the phone number or e-mail of your Babel account.",
                    "Укажите телефон или e-mail аккаунта Babel.",
                ),
            )
        }
    }

    fn render_form(&self, cx: &mut Context<Self>) -> Div {
        let pending = self.flow.is_pending;
        let field = |input: &Entity<InputState>| Input::new(input).large().disabled(pending);
        if self.flow.is_subaccount_mode() {
            v_flex()
                .gap_3()
                .child(field(&self.host_url))
                .child(field(&self.subaccount_code))
                .child(
                    self.actions(
                        Button::new("use-babel")
                            .ghost()
                            .label(localized(
                                cx,
                                "Use a Babel account instead",
                                "Войти через аккаунт Babel",
                            ))
                            .disabled(pending)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.switch_to_babel(window, cx)),
                            ),
                        localized(cx, "Sign in", "Войти"),
                        self.flow.can_submit_subaccount_login(),
                        cx,
                    ),
                )
        } else if self.flow.is_code_step() {
            v_flex().gap_3().child(field(&self.code)).child(
                self.actions(
                    Button::new("back-to-login")
                        .ghost()
                        .label(localized(cx, "Use another number", "Другой номер"))
                        .disabled(pending)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.back_to_login(window, cx)),
                        ),
                    localized(cx, "Sign in", "Войти"),
                    self.flow.can_submit_code(),
                    cx,
                ),
            )
        } else {
            v_flex().gap_3().child(field(&self.identifier)).child(
                self.actions(
                    Button::new("use-subaccount")
                        .ghost()
                        .label(localized(
                            cx,
                            "I have a sub-account code",
                            "У меня код под-аккаунта",
                        ))
                        .disabled(pending)
                        .on_click(
                            cx.listener(|this, _, window, cx| {
                                this.switch_to_subaccount(window, cx)
                            }),
                        ),
                    localized(cx, "Continue", "Продолжить"),
                    self.flow.can_start_login(),
                    cx,
                ),
            )
        }
    }

    fn actions(
        &self,
        secondary: Button,
        primary_label: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        h_flex().justify_between().child(secondary).child(
            Button::new("submit")
                .primary()
                .label(primary_label)
                .loading(self.flow.is_pending)
                .disabled(!enabled)
                .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
        )
    }
}

impl Render for AuthScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (heading, subtitle) = self.texts(cx);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let card = v_flex()
            .w_full()
            .max_w(rems(26.25))
            .gap_5()
            .p_8()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .shadow_lg()
            .child(brand(cx))
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(heading),
                    )
                    .child(div().text_sm().text_color(muted).child(subtitle)),
            )
            .child(self.render_form(cx))
            .when_some(self.flow.error.clone(), |this, error| {
                this.child(div().text_sm().text_color(danger).child(error))
            })
            .when(self.flow.is_pending, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(muted)
                        .child(Spinner::new().small())
                        .child(localized(cx, "Signing in…", "Вход…")),
                )
            })
            .child(div().text_xs().text_color(muted).child(localized(
                cx,
                "Your sign-in stays on this device.",
                "Данные входа остаются на этом устройстве.",
            )));
        div()
            .id("auth-screen")
            .size_full()
            .bg(cx.theme().muted)
            .overflow_y_scrollbar()
            .child(
                div()
                    .min_h_full()
                    .p_4()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(card),
            )
    }
}

/// The Exaudio icon, parsed once; gpui rasterizes the SVG and caches it by content hash.
static BRAND_MARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../../assets/brand/exaudio-icon.svg").to_vec(),
    ))
});

/// App mark: the Exaudio icon tile.
pub fn brand_mark() -> Img {
    img(BRAND_MARK.clone()).size(rems(2.25)).flex_none()
}

/// App mark + name, shared with the dashboard sidebar.
pub fn brand(cx: &App) -> Div {
    let theme = cx.theme();
    h_flex().gap_3().child(brand_mark()).child(
        v_flex()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Exaudio"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Babel"),
            ),
    )
}
