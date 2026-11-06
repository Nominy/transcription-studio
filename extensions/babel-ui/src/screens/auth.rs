//! Sign-in: Babel phone/e-mail code login and sub-account (relay code) login.

use std::sync::{Arc, LazyLock};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Sizable, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use sonus_babel_services::auth::{self, flow::AuthFlow};
use sonus_babel_services::subaccounts;

use crate::root::{BabelController, BabelMode};
use crate::screens::ShellModels;
use sonus_shell::services::spawn_service_in;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Identifier,
    Code,
    HostUrl,
    SubaccountCode,
}

pub struct AuthScreen {
    root: WeakEntity<BabelController>,
    flow: AuthFlow,
    identifier: Entity<InputState>,
    code: Entity<InputState>,
    host_url: Entity<InputState>,
    subaccount_code: Entity<InputState>,
    request: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AuthScreen {
    pub fn new(
        root: WeakEntity<BabelController>,
        flow: AuthFlow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let identifier =
            cx.new(|cx| InputState::new(window, cx).placeholder("Phone number or e-mail"));
        let code = cx.new(|cx| InputState::new(window, cx).placeholder("Verification code"));
        let host_url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Relay URL, e.g. https://relay.example.com")
                .default_value(flow.subaccount_host_url.clone())
        });
        let subaccount_code = cx.new(|cx| InputState::new(window, cx).placeholder("One-time code"));
        let subscriptions = vec![
            Self::watch(&identifier, Field::Identifier, window, cx),
            Self::watch(&code, Field::Code, window, cx),
            Self::watch(&host_url, Field::HostUrl, window, cx),
            Self::watch(&subaccount_code, Field::SubaccountCode, window, cx),
        ];
        let this = Self {
            root,
            flow,
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

    pub fn cancel_request(&mut self) {
        self.request = None;
        self.flow.is_pending = false;
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

    fn start_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flow.can_start_login() {
            return;
        }
        let identifier = self.flow.identifier.trim().to_string();
        self.begin_request(cx);
        self.request = Some(spawn_service_in(
            window,
            cx,
            async move { auth::start_login(&identifier).await },
            |this, result, window, cx| match result {
                Ok(start) => {
                    let mut flow =
                        AuthFlow::pending_code(start.sign_in_id, Some(start.phone_number_id));
                    flow.identifier = this.flow.identifier.clone();
                    this.set_flow(flow, window, cx);
                }
                Err(error) => this.fail(format!("{error:#}"), cx),
            },
        ));
    }

    fn verify_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.flow.can_submit_code() {
            return;
        }
        let code = self.flow.code.trim().to_string();
        self.begin_request(cx);
        self.request = Some(spawn_service_in(
            window,
            cx,
            async move { auth::verify_login(&code).await },
            |this, result, window, cx| match result {
                Ok(_) => this.enter_dashboard(window, cx),
                Err(error) => this.fail(format!("{error:#}"), cx),
            },
        ));
    }

    /// "Use a different number": forget the pending sign-in and start over.
    fn back_to_login(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.flow.is_pending {
            return;
        }
        match auth::reset_store() {
            Ok(()) => self.set_flow(AuthFlow::login_prompt(), window, cx),
            Err(error) => {
                self.flow.error = Some(format!("Sign-in restart failed: {error:#}"));
                cx.notify();
            }
        }
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
                Ok(session) => {
                    let relay = ShellModels::relay(cx);
                    let remembered = relay.update(cx, |relay, cx| {
                        relay.remember_host_url(&session.host_url, cx)
                    });
                    if let Err(error) = remembered {
                        window.push_notification(
                            Notification::warning(format!(
                                "Relay URL could not be remembered: {error}"
                            )),
                            cx,
                        );
                        relay.update(cx, |relay, cx| {
                            relay.adopt_host_url(session.host_url.clone(), cx)
                        });
                    }
                    this.root
                        .update(cx, |root, cx| root.enter_relay(session, window, cx))
                        .ok();
                }
                Err(error) => this.fail(format!("{error:#}"), cx),
            },
        ));
    }

    fn enter_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.request = None;
        self.root
            .update(cx, |root, cx| root.show_dashboard(window, cx))
            .ok();
    }

    fn texts(&self) -> (&'static str, &'static str) {
        if self.flow.is_subaccount_mode() {
            (
                "Sign in to a sub-account",
                "Use the relay address and one-time code from the account owner.",
            )
        } else if self.flow.is_code_step() {
            (
                "Check your phone",
                "Enter the verification code Babel sent you.",
            )
        } else {
            (
                "Welcome back",
                "Sign in with the phone number or e-mail of your Babel account.",
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
                            .label("Use Babel account")
                            .disabled(pending)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if !this.flow.is_pending {
                                    this.root
                                        .update(cx, |root, cx| {
                                            root.enter_mode(BabelMode::Direct, window, cx)
                                        })
                                        .ok();
                                }
                            })),
                        "Sign in",
                        self.flow.can_submit_subaccount_login(),
                        cx,
                    ),
                )
        } else if self.flow.is_code_step() {
            v_flex().gap_3().child(field(&self.code)).child(
                self.actions(
                    Button::new("back-to-login")
                        .ghost()
                        .label("Use a different number")
                        .disabled(pending)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.back_to_login(window, cx)),
                        ),
                    "Verify",
                    self.flow.can_submit_code(),
                    cx,
                ),
            )
        } else {
            v_flex().gap_3().child(field(&self.identifier)).child(
                self.actions(
                    Button::new("use-subaccount")
                        .ghost()
                        .label("Use sub-account code")
                        .disabled(pending)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.flow.is_pending {
                                this.root
                                    .update(cx, |root, cx| {
                                        root.enter_mode(BabelMode::Relay, window, cx)
                                    })
                                    .ok();
                            }
                        })),
                    "Continue",
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
        let (heading, subtitle) = self.texts();
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let danger = theme.danger;
        let card =
            v_flex()
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
                            .child("Signing you in…"),
                    )
                })
                .child(div().text_xs().text_color(muted).child(
                    "Your credentials stay on this device and are only used to talk to Babel.",
                ));
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

/// The Sonus mark, parsed once; gpui rasterizes the SVG and caches it by content hash.
static BRAND_MARK: LazyLock<Arc<Image>> = LazyLock::new(|| {
    Arc::new(Image::from_bytes(
        ImageFormat::Svg,
        include_bytes!("../../../../assets/brand/sonus-mark.svg").to_vec(),
    ))
});

/// App mark: the Sonus medallion.
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
                    .child("Sonus"),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Babel workspace"),
            ),
    )
}
