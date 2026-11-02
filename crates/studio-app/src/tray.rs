//! Windows notification-area icon (restore / relay start-stop / exit) and hide-to-tray.
//! Other platforms have no tray: closing the window exits the app.

use gpui_kit::*;

/// A tray menu command, delivered on the UI thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Restore,
    StartRelay,
    StopRelay,
    Exit,
}

/// Keeps the tray icon alive for the app's lifetime.
struct TrayHandle {
    #[cfg(windows)]
    _icon: tray_icon::TrayIcon,
}

impl Global for TrayHandle {}

/// Whether the tray icon exists (closing the window hides instead of exiting).
pub fn is_active(cx: &App) -> bool {
    cx.has_global::<TrayHandle>()
}

/// Creates the tray icon; returns the command stream, or `None` without a tray.
pub fn install(cx: &mut App) -> Option<async_channel::Receiver<TrayCommand>> {
    #[cfg(windows)]
    {
        match windows_tray::create() {
            Ok((icon, receiver)) => {
                cx.set_global(TrayHandle { _icon: icon });
                Some(receiver)
            }
            Err(error) => {
                eprintln!("tray icon unavailable: {error}");
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = cx;
        None
    }
}

/// Hides the window to the tray.
pub fn hide_window(window: &mut Window) {
    #[cfg(windows)]
    windows_tray::show(window, false);
    #[cfg(not(windows))]
    window.minimize_window();
}

/// Shows a hidden/minimized window and brings it to the front.
pub fn show_window(window: &mut Window) {
    #[cfg(windows)]
    windows_tray::show(window, true);
    window.activate_window();
}

#[cfg(windows)]
mod windows_tray {
    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IsIconic, SW_HIDE, SW_RESTORE, SW_SHOW, ShowWindow,
    };

    use super::TrayCommand;

    const RESTORE_ID: &str = "transcription-studio.restore";
    const START_RELAY_ID: &str = "transcription-studio.start-relay";
    const STOP_RELAY_ID: &str = "transcription-studio.stop-relay";
    const EXIT_ID: &str = "transcription-studio.exit";

    pub fn create() -> Result<(TrayIcon, async_channel::Receiver<TrayCommand>), String> {
        let menu = Menu::new();
        let item = |id: &str, label: &str| MenuItem::with_id(MenuId::new(id), label, true, None);
        menu.append_items(&[
            &item(RESTORE_ID, "Open Transcription Studio"),
            &PredefinedMenuItem::separator(),
            &item(START_RELAY_ID, "Start relay"),
            &item(STOP_RELAY_ID, "Stop relay"),
            &PredefinedMenuItem::separator(),
            &item(EXIT_ID, "Exit"),
        ])
        .map_err(|error| format!("failed to build tray menu: {error}"))?;

        let (sender, receiver) = async_channel::unbounded();
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                RESTORE_ID => TrayCommand::Restore,
                START_RELAY_ID => TrayCommand::StartRelay,
                STOP_RELAY_ID => TrayCommand::StopRelay,
                EXIT_ID => TrayCommand::Exit,
                _ => return,
            };
            let _ = menu_sender.try_send(command);
        }));
        TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                let _ = sender.try_send(TrayCommand::Restore);
            }
        }));

        let icon = TrayIconBuilder::new()
            .with_tooltip("Transcription Studio")
            .with_icon(app_icon()?)
            .with_menu(Box::new(menu))
            .build()
            .map_err(|error| format!("failed to create tray icon: {error}"))?;
        Ok((icon, receiver))
    }

    pub fn show(window: &Window, visible: bool) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        let RawWindowHandle::Win32(handle) = handle.as_raw() else {
            return;
        };
        let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        // SAFETY: `hwnd` is the live top-level window owned by this gpui window.
        unsafe {
            if !visible {
                ShowWindow(hwnd, SW_HIDE);
            } else if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            } else {
                ShowWindow(hwnd, SW_SHOW);
            }
        }
    }

    /// 32×32 brand mark: rounded blue square with three white waveform bars.
    fn app_icon() -> Result<Icon, String> {
        const SIZE: i32 = 32;
        const RADIUS: i32 = 7;
        const BARS: [(i32, i32); 3] = [(9, 10), (15, 6), (21, 12)];
        let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let corner_x = (RADIUS - x).max(x - (SIZE - 1 - RADIUS)).max(0);
                let corner_y = (RADIUS - y).max(y - (SIZE - 1 - RADIUS)).max(0);
                let inside = corner_x * corner_x + corner_y * corner_y <= RADIUS * RADIUS;
                let bar = BARS.iter().any(|&(left, top)| {
                    (left..left + 3).contains(&x) && (top..SIZE - top).contains(&y)
                });
                let pixel = match (inside, bar) {
                    (false, _) => [0, 0, 0, 0],
                    (true, true) => [255, 255, 255, 255],
                    (true, false) => [37, 99, 235, 255],
                };
                rgba.extend_from_slice(&pixel);
            }
        }
        Icon::from_rgba(rgba, SIZE as u32, SIZE as u32)
            .map_err(|error| format!("failed to create tray icon: {error}"))
    }
}
