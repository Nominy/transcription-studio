#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bundled;

use std::process::ExitCode;

fn main() -> ExitCode {
    studio_services::updater::run_startup_hooks();
    #[cfg(feature = "babel")]
    if let Some(code) = exaudio_babel_services::auth::cli::run_from_args(std::env::args()) {
        return code;
    }
    exaudio_shell::app::run(bundled::register);
    ExitCode::SUCCESS
}
