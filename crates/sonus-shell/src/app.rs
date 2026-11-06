//! Native window composition. The executable explicitly supplies bundled contributions.
use crate::{
    ContributionRegistry, ShellHandle,
    keymap::ActiveKeymap,
    root::StudioRoot,
    screens::{ShellModels, settings},
    services::AppServices,
    tray::{self, TrayCommand},
};
use gpui_kit::component::{WindowExt as _, notification::Notification};
use gpui_kit::*;
pub fn run(register: impl FnOnce(&mut App) -> anyhow::Result<()> + 'static) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_global(AppServices::from_env());
            cx.set_global(ContributionRegistry::default());
            settings::apply_saved_theme(cx);
            ActiveKeymap::init(cx);
            crate::zoom::init(cx);
            ShellModels::init(cx);
            let registration_error = register(cx)
                .err()
                .map(|e| format!("Bundled extension activation failed: {e:#}"));
            let tray_commands = tray::install(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1440.), px(900.)),
                    cx,
                ))),
                window_min_size: Some(size(px(880.), px(600.))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Sonus".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                let root = cx.new(|cx| StudioRoot::new(window, cx));
                let close = root.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    close
                        .update(cx, |root, cx| root.request_close(window, cx))
                        .ok();
                    false
                });
                if let Some(error) = registration_error {
                    window.defer(cx, move |window, cx| {
                        window.push_notification(Notification::error(error), cx)
                    });
                }
                root
            });
            let (window, root) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    eprintln!("Could not open Sonus: {error:#}");
                    cx.quit();
                    return;
                }
            };
            cx.set_global(ShellHandle::from_root(root.downgrade()));
            crate::screens::extensions::startup(ShellHandle::from_root(root.downgrade()), cx);
            let shell = ShellHandle::from_root(root.downgrade());
            let _ = window.update(cx, |_, window, cx| {
                let callbacks = cx
                    .global_mut::<ContributionRegistry>()
                    .take_ready_callbacks();
                for callback in callbacks {
                    callback(shell.clone(), window, cx);
                }
                if std::env::var_os("STUDIO_OPEN_DEMO_ON_START").is_some()
                    && let Err(error) = crate::root::open_demo(&shell, window, cx)
                {
                    window.push_notification(
                        Notification::error(format!("Demo could not be opened: {error}")),
                        cx,
                    );
                }
            });
            if let Some(commands) = tray_commands {
                let shell = ShellHandle::from_root(root.downgrade());
                cx.spawn(async move |cx| {
                    while let Ok(command) = commands.recv().await {
                        let shell = shell.clone();
                        if window
                            .update(cx, |_, window, cx| match command {
                                TrayCommand::Restore => tray::show_window(window),
                                TrayCommand::Command(id) => shell.dispatch(&id, window, cx),
                                TrayCommand::Exit => {
                                    tray::show_window(window);
                                    shell
                                        .0
                                        .update(cx, |root, cx| root.request_exit(window, cx))
                                        .ok();
                                }
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                })
                .detach();
            }
        });
}
