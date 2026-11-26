#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;

pub fn main() -> iced::Result {
    if let Some(result) = app::run_babel_service_stdio_cli_from_args(std::env::args()) {
        if let Err(error) = result {
            eprintln!("{error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    velopack::VelopackApp::build().run();
    if let Some(result) = app::run_native_auth_cli_from_args(std::env::args()) {
        if let Err(error) = result {
            eprintln!("studio-app auth: {error}");
            std::process::exit(1);
        }
        return Ok(());
    }
    app::run()
}
