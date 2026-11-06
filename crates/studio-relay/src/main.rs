use std::{net::SocketAddr, path::PathBuf};

use studio_relay::{RelayConfig, RelayState, router};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse(std::env::args().skip(1));
    if args.help {
        print_help();
        return Ok(());
    }

    let mut config = RelayConfig::from_env();
    if let Some(store_path) = args.store_path {
        config.store_path = store_path;
    }
    if let Some(owner_token) = args.owner_token {
        config.owner_token = Some(owner_token);
    }
    if let Some(command) = args.babel_service_command {
        config.babel_service_command = Some(command);
    }
    if let Some(upstream_url) = args.upstream_url {
        config.upstream_url = Some(upstream_url);
    }
    if let Some(task_cents) = args.task_cents {
        config.task_cents = task_cents;
    }

    let bind = args
        .bind
        .unwrap_or_else(|| "127.0.0.1:8787".parse().expect("default bind"));
    let state = RelayState::new(config).await?;
    let listener = TcpListener::bind(bind).await?;
    let local_addr = listener.local_addr()?;
    eprintln!("studio-relay listening on http://{local_addr}");
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[derive(Default)]
struct Args {
    bind: Option<SocketAddr>,
    store_path: Option<PathBuf>,
    owner_token: Option<String>,
    babel_service_command: Option<String>,
    upstream_url: Option<String>,
    task_cents: Option<i64>,
    help: bool,
}

impl Args {
    fn parse(mut args: impl Iterator<Item = String>) -> Self {
        let mut parsed = Self::default();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => parsed.help = true,
                "--bind" => {
                    parsed.bind = args.next().and_then(|value| value.parse().ok());
                }
                "--store" => {
                    parsed.store_path = args.next().map(PathBuf::from);
                }
                "--owner-token" => {
                    parsed.owner_token = args.next().filter(|value| !value.trim().is_empty());
                }
                "--babel-service-command" => {
                    parsed.babel_service_command =
                        args.next().filter(|value| !value.trim().is_empty());
                }
                "--upstream-url" => {
                    parsed.upstream_url = args.next().filter(|value| !value.trim().is_empty());
                }
                "--task-cents" => {
                    parsed.task_cents = args.next().and_then(|value| value.parse().ok());
                }
                _ => {}
            }
        }
        parsed
    }
}

fn print_help() {
    println!(
        "Usage: studio-relay [--bind 127.0.0.1:8787] [--store path] [--owner-token token] [--babel-service-command command] [--upstream-url url] [--task-cents cents]\n\n\
Runs the Exaudio Sub-account relay.\n\
Environment: STUDIO_SUBACCOUNT_RELAY_STORE, STUDIO_SUBACCOUNT_OWNER_TOKEN, STUDIO_BABEL_SERVICE_COMMAND, STUDIO_SUBACCOUNT_UPSTREAM_URL, STUDIO_SUBACCOUNT_TASK_CENTS."
    );
}
