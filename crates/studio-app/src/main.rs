#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod editor;
mod keymap;
mod root;
mod screens;
mod services;
mod tray;
mod zoom;

use std::process::ExitCode;

use studio_editor::edit::StartupRoute;

fn main() -> ExitCode {
    studio_services::updater::run_startup_hooks();
    if let Some(code) = studio_services::auth::cli::run_from_args(std::env::args()) {
        return code;
    }
    let route = StartupRoute::parse_args(std::env::args().skip(1));
    app::run(route);
    ExitCode::SUCCESS
}
