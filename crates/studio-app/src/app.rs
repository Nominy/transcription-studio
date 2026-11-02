//! Application bootstrap: globals, theme, main window, tray.

use gpui_kit::*;
use studio_editor::edit::StartupRoute;

use crate::keymap::ActiveKeymap;
use crate::root::StudioRoot;
use crate::screens::ShellModels;
use crate::screens::settings;
use crate::services::AppServices;
use crate::tray::{self, TrayCommand};

const WINDOW_SIZE: Size<Pixels> = size(px(1440.), px(900.));
/// Small enough to tile two windows side by side on a 1080p screen; layouts wrap or scroll below
/// their comfortable width.
const WINDOW_MIN_SIZE: Size<Pixels> = size(px(880.), px(600.));

pub fn run(route: StartupRoute) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_global(AppServices::from_env());
            settings::apply_saved_theme(cx);
            ActiveKeymap::init(cx);
            crate::zoom::init(cx);
            ShellModels::init(cx);
            let tray_commands = tray::install(cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    WINDOW_SIZE,
                    cx,
                ))),
                window_min_size: Some(WINDOW_MIN_SIZE),
                titlebar: Some(TitlebarOptions {
                    title: Some("Transcription Studio".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let (window, root) = gpui_kit::open_window(options, cx, |window, cx| {
                let root = cx.new(|cx| StudioRoot::new(route, window, cx));
                let close_root = root.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    close_root
                        .update(cx, |root, cx| root.request_close(window, cx))
                        .ok();
                    false
                });
                root
            })
            .expect("failed to open the main window");

            if let Some(commands) = tray_commands {
                let root = root.downgrade();
                cx.spawn(async move |cx| {
                    while let Ok(command) = commands.recv().await {
                        let handled = window.update(cx, |_, window, cx| {
                            handle_tray_command(command, &root, window, cx);
                        });
                        if handled.is_err() {
                            break;
                        }
                    }
                })
                .detach();
            }

            cx.on_app_quit(|cx| {
                ShellModels::relay(cx).update(cx, |relay, _| relay.shutdown());
                async {}
            })
            .detach();
        });
}

fn handle_tray_command(
    command: TrayCommand,
    root: &WeakEntity<StudioRoot>,
    window: &mut Window,
    cx: &mut App,
) {
    match command {
        TrayCommand::Restore => tray::show_window(window),
        TrayCommand::StartRelay => ShellModels::relay(cx).update(cx, |relay, cx| relay.start(cx)),
        TrayCommand::StopRelay => ShellModels::relay(cx).update(cx, |relay, cx| relay.stop(cx)),
        TrayCommand::Exit => {
            tray::show_window(window);
            root.update(cx, |root, cx| root.request_exit(window, cx))
                .ok();
        }
    }
}
