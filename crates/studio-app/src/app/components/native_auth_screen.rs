use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Color, Element, Fill, Font, Length, font};

use super::super::styles::{
    app_background_style, brand_mark_style, panel_style, primary_button_style,
    secondary_button_style, success_pill_style,
};
use super::super::{Message, NativeAuthFlowState};

const SEMIBOLD: Font = Font {
    weight: font::Weight::Semibold,
    ..Font::DEFAULT
};

pub(super) fn view_native_auth_screen<'a>(flow: &'a NativeAuthFlowState) -> Element<'a, Message> {
    let heading = if flow.is_subaccount_mode() {
        "Sign in to a sub-account"
    } else if flow.is_code_step() {
        "Check your phone"
    } else {
        "Welcome back"
    };
    let subtitle = if flow.is_subaccount_mode() {
        "Use the relay address and one-time code provided by the account owner."
    } else if flow.is_code_step() {
        "Enter the verification code Babel sent to your phone."
    } else {
        "Sign in with the phone number or email attached to your Babel account."
    };

    let mut form = column![
        row![
            container(text("TS").size(14).font(SEMIBOLD))
                .width(Length::Fixed(38.0))
                .height(Length::Fixed(38.0))
                .center_x(Length::Fixed(38.0))
                .center_y(Length::Fixed(38.0))
                .style(brand_mark_style),
            column![
                text("Transcription Studio").size(15).font(SEMIBOLD),
                text("Secure Babel workspace")
                    .size(11)
                    .color(Color::from_rgb8(100, 116, 139)),
            ]
            .spacing(2),
        ]
        .spacing(10)
        .align_y(iced::Alignment::Center),
        Space::with_height(Length::Fixed(12.0)),
        text(heading).size(24).font(SEMIBOLD),
        text(subtitle).size(13).color(Color::from_rgb8(71, 85, 105)),
    ]
    .spacing(10)
    .width(Length::Fixed(440.0));

    if flow.is_subaccount_mode() {
        form = form
            .push(
                text_input("Relay host URL", &flow.subaccount_host_url)
                    .on_input_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthSubaccountHostChanged),
                    )
                    .on_submit(Message::NativeAuthSubaccountLogin)
                    .padding(12)
                    .size(14)
                    .width(Fill),
            )
            .push(
                text_input("Special code", &flow.subaccount_code)
                    .on_input_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthSubaccountCodeChanged),
                    )
                    .on_submit(Message::NativeAuthSubaccountLogin)
                    .padding(12)
                    .size(14)
                    .width(Fill),
            )
            .push(
                row![
                    button(text("Use Babel account").size(13))
                        .style(secondary_button_style)
                        .on_press_maybe(
                            (!flow.is_pending).then_some(Message::NativeAuthUseBabelMode)
                        )
                        .padding([10, 14]),
                    Space::with_width(Fill),
                    button(text("Sign in").size(13))
                        .style(primary_button_style)
                        .on_press_maybe(
                            flow.can_submit_subaccount_login()
                                .then_some(Message::NativeAuthSubaccountLogin)
                        )
                        .padding([10, 18]),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
    } else if flow.is_code_step() {
        form = form
            .push(
                text_input("Phone code", &flow.code)
                    .on_input_maybe((!flow.is_pending).then_some(Message::NativeAuthCodeChanged))
                    .on_submit(Message::NativeAuthVerifyLogin)
                    .padding(12)
                    .size(15)
                    .width(Fill),
            )
            .push(
                row![
                    button(text("Use different number").size(13))
                        .style(secondary_button_style)
                        .on_press_maybe(
                            (!flow.is_pending).then_some(Message::NativeAuthBackToLogin)
                        )
                        .padding([10, 14]),
                    Space::with_width(Fill),
                    button(text("Verify code").size(13))
                        .style(primary_button_style)
                        .on_press_maybe(
                            flow.can_submit_code()
                                .then_some(Message::NativeAuthVerifyLogin)
                        )
                        .padding([10, 18]),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
    } else {
        form = form
            .push(
                text_input("Phone or email", &flow.identifier)
                    .on_input_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthIdentifierChanged),
                    )
                    .on_submit(Message::NativeAuthStartLogin)
                    .padding(12)
                    .size(14)
                    .width(Fill),
            )
            .push(
                row![
                    button(text("Use sub-account code").size(13))
                        .style(secondary_button_style)
                        .on_press_maybe(
                            (!flow.is_pending).then_some(Message::NativeAuthUseSubaccountMode)
                        )
                        .padding([10, 14]),
                    Space::with_width(Fill),
                    button(text("Continue").size(13))
                        .style(primary_button_style)
                        .on_press_maybe(
                            flow.can_start_login()
                                .then_some(Message::NativeAuthStartLogin)
                        )
                        .padding([10, 18]),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
    }

    if let Some(error) = flow.error.as_ref() {
        form = form.push(
            text(error.clone())
                .size(12)
                .color(Color::from_rgb8(185, 28, 28)),
        );
    }

    if flow.is_pending {
        form = form.push(
            container(text("Signing you in...").size(12))
                .padding([5, 10])
                .style(success_pill_style),
        );
    }

    form = form.push(
        text("Your credentials stay on this device and are used only to connect to Babel.")
            .size(11)
            .color(Color::from_rgb8(100, 116, 139)),
    );

    container(
        container(form)
            .padding(32)
            .style(panel_style)
            .center_x(Length::Shrink),
    )
    .width(Fill)
    .height(Fill)
    .center_x(Fill)
    .center_y(Fill)
    .style(app_background_style)
    .into()
}
