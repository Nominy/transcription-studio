mod app;

pub fn main() -> iced::Result {
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
