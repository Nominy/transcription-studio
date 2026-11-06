//! Windows notification-area icon (restore / registered commands / exit).
//! Other platforms have no tray: closing the window exits the app.

use gpui_kit::*;

/// A tray menu command, delivered on the UI thread.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(windows),
    expect(dead_code, reason = "only the Windows tray sends commands")
)]
pub enum TrayCommand {
    Restore,
    Command(String),
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
        let commands = cx
            .global::<crate::ContributionRegistry>()
            .commands()
            .into_iter()
            .map(|c| (c.id, c.label))
            .collect();
        let labels = windows_tray::Labels {
            open: crate::screens::ui::localized(cx, "Open Exaudio", "Открыть Exaudio"),
            quit: crate::screens::ui::localized(cx, "Quit", "Выйти"),
        };
        match windows_tray::create(commands, labels) {
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
        GetSystemMetrics, IsIconic, SM_CXSMICON, SM_CYSMICON, SW_HIDE, SW_RESTORE, SW_SHOW,
        ShowWindow,
    };

    use super::TrayCommand;

    const RESTORE_ID: &str = "transcription-studio.restore";
    const EXIT_ID: &str = "transcription-studio.exit";

    /// Localized labels of the fixed menu items.
    pub struct Labels {
        pub open: &'static str,
        pub quit: &'static str,
    }

    pub fn create(
        commands: Vec<(String, String)>,
        labels: Labels,
    ) -> Result<(TrayIcon, async_channel::Receiver<TrayCommand>), String> {
        let menu = Menu::new();
        let item = |id: &str, label: &str| MenuItem::with_id(MenuId::new(id), label, true, None);
        menu.append_items(&[
            &item(RESTORE_ID, labels.open),
            &PredefinedMenuItem::separator(),
            &item(EXIT_ID, labels.quit),
        ])
        .map_err(|error| format!("failed to build tray menu: {error}"))?;
        for (id, label) in &commands {
            menu.append(&item(id, label))
                .map_err(|error| error.to_string())?;
        }

        let (sender, receiver) = async_channel::unbounded();
        let menu_sender = sender.clone();
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            let command = match event.id.as_ref() {
                RESTORE_ID => TrayCommand::Restore,
                id if commands.iter().any(|(command, _)| command == id) => {
                    TrayCommand::Command(id.to_owned())
                }
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
            .with_tooltip("Exaudio")
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

    /// The Exaudio mark: the exe's icon resource (id 1, `build.rs`), at the system small-icon size
    /// so Windows picks the pixel-hinted 16/20/24 px frame instead of scaling a large one.
    fn app_icon() -> Result<Icon, String> {
        // SAFETY: plain metric queries with no pointers.
        let (width, height) =
            unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
        let size = u32::try_from(width).ok().zip(u32::try_from(height).ok());
        Icon::from_resource(1, size).map_err(|error| format!("failed to load tray icon: {error}"))
    }
}
