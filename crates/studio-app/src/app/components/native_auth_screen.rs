use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Color, Element, Fill, Length};

use super::super::styles::{app_background_style, panel_style};
use super::super::{Message, NativeAuthFlowState};

pub(super) fn view_native_auth_screen<'a>(flow: &'a NativeAuthFlowState) -> Element<'a, Message> {
    let title = text("Transcription Studio").size(28);
    let subtitle = if flow.is_subaccount_mode() {
        text("Sub-account mode uses your relay host and special login code.").size(13)
    } else if flow.is_code_step() {
        text("Enter the code Babel sent to your phone.").size(13)
    } else {
        text("Sign in with your Babel phone or email.").size(13)
    };

    let mut form = column![title, subtitle]
        .spacing(10)
        .width(Length::Fixed(420.0));

    if flow.is_subaccount_mode() {
        form = form
            .push(
                text_input("Relay host URL", &flow.subaccount_host_url)
                    .on_input_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthSubaccountHostChanged),
                    )
                    .on_submit(Message::NativeAuthSubaccountLogin)
                    .padding(10)
                    .width(Fill),
            )
            .push(
                text_input("Special code", &flow.subaccount_code)
                    .on_input_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthSubaccountCodeChanged),
                    )
                    .on_submit(Message::NativeAuthSubaccountLogin)
                    .padding(10)
                    .width(Fill),
            )
            .push(
                row![
                    button(text("Use Babel account")).on_press_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthUseBabelMode)
                    ),
                    Space::with_width(Fill),
                    button(text("Sign in")).on_press_maybe(
                        flow.can_submit_subaccount_login()
                            .then_some(Message::NativeAuthSubaccountLogin)
                    ),
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
                    .padding(10)
                    .width(Fill),
            )
            .push(
                row![
                    button(text("Use different number")).on_press_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthBackToLogin)
                    ),
                    Space::with_width(Fill),
                    button(text("Verify")).on_press_maybe(
                        flow.can_submit_code()
                            .then_some(Message::NativeAuthVerifyLogin)
                    ),
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
                    .padding(10)
                    .width(Fill),
            )
            .push(
                row![
                    button(text("Use sub-account code")).on_press_maybe(
                        (!flow.is_pending).then_some(Message::NativeAuthUseSubaccountMode)
                    ),
                    Space::with_width(Fill),
                    button(text("Send code")).on_press_maybe(
                        flow.can_start_login()
                            .then_some(Message::NativeAuthStartLogin)
                    ),
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
        form = form.push(text("Working...").size(12));
    }

    container(
        container(form)
            .padding(24)
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
