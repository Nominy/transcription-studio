use std::thread;

use iced::{Subscription, futures::channel::mpsc};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuId, MenuItem},
};

use super::Message;

pub(super) const RESTORE_ID: &str = "transcription-studio.restore";
pub(super) const START_RELAY_ID: &str = "transcription-studio.start-relay";
pub(super) const STOP_RELAY_ID: &str = "transcription-studio.stop-relay";
pub(super) const EXIT_ID: &str = "transcription-studio.exit";

pub(super) struct TrayController {
    _icon: TrayIcon,
}

pub(super) fn create() -> Result<TrayController, String> {
    let menu = Menu::new();
    let restore = MenuItem::with_id(
        MenuId::new(RESTORE_ID),
        "Restore Transcription Studio",
        true,
        None,
    );
    let start_relay = MenuItem::with_id(MenuId::new(START_RELAY_ID), "Start relay", true, None);
    let stop_relay = MenuItem::with_id(MenuId::new(STOP_RELAY_ID), "Stop relay", true, None);
    let exit = MenuItem::with_id(MenuId::new(EXIT_ID), "Exit", true, None);
    menu.append_items(&[&restore, &start_relay, &stop_relay, &exit])
        .map_err(|error| format!("failed to build tray menu: {error}"))?;

    let icon = TrayIconBuilder::new()
        .with_tooltip("Transcription Studio")
        .with_icon(app_icon()?)
        .with_menu(Box::new(menu))
        .build()
        .map_err(|error| format!("failed to create tray icon: {error}"))?;

    Ok(TrayController { _icon: icon })
}

pub(super) fn subscription() -> Subscription<Message> {
    Subscription::run_with_id("transcription-studio-tray-events", tray_event_stream())
}

fn tray_event_stream() -> impl iced::futures::Stream<Item = Message> {
    let (sender, receiver) = mpsc::unbounded();
    thread::spawn(move || {
        let events = MenuEvent::receiver();
        while let Ok(event) = events.recv() {
            let message = match event.id.as_ref() {
                RESTORE_ID => Message::TrayRestoreRequested,
                START_RELAY_ID => Message::TrayStartRelay,
                STOP_RELAY_ID => Message::TrayStopRelay,
                EXIT_ID => Message::TrayExitRequested,
                _ => continue,
            };
            if sender.unbounded_send(message).is_err() {
                break;
            }
        }
    });
    receiver
}

fn app_icon() -> Result<Icon, String> {
    const SIZE: usize = 32;
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);
    for _ in 0..(SIZE * SIZE) {
        rgba.extend_from_slice(&[30, 52, 88, 255]);
    }
    Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
        .map_err(|error| format!("failed to create tray icon: {error}"))
}
