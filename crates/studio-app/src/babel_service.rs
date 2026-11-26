#![allow(dead_code)]
mod app;

fn main() {
    let result =
        app::run_babel_service_stdio_cli_from_args(["babel-service", "--babel-service-stdio"])
            .expect("service mode");
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
