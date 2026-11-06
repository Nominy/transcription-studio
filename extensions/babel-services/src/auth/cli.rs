//! `--babel-auth-start-login <identifier> [--babel-auth-reset]` and
//! `--babel-auth-verify-login <code>`: headless native login, run from `main`
//! before any window opens. Prints one JSON status line on success.

use std::process::ExitCode;

use anyhow::{Result, bail};
use serde_json::json;

use super::{
    babel_base_url, clerk_url, now_unix_seconds, start_phone_code_login, store_path,
    verify_phone_code_login,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCliCommand {
    StartLogin { identifier: String, reset: bool },
    VerifyLogin { code: String },
}

/// Runs an auth CLI command if `args` (including argv[0]) contain one.
/// `None` → not an auth invocation, continue starting the app.
/// `Some(code)` → exit with `code` (errors are printed to stderr, exit 1).
pub fn run_from_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Option<ExitCode> {
    let result = match parse_args(args) {
        Ok(None) => return None,
        Ok(Some(command)) => studio_services::runtime::block_on(run_command(command)),
        Err(error) => Err(error),
    };
    Some(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(1)
        }
    })
}

/// Parses auth flags (`--flag value` or `--flag=value`; dashes/underscores/case ignored).
pub fn parse_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<Option<AuthCliCommand>> {
    let mut start_identifier: Option<String> = None;
    let mut verify_code: Option<String> = None;
    let mut reset = false;
    let mut pending_key: Option<String> = None;

    for arg in args.into_iter().skip(1) {
        let arg = arg.as_ref();
        if let Some(key) = pending_key.take() {
            set_value(&mut start_identifier, &mut verify_code, &key, arg)?;
            continue;
        }
        let Some(flag) = arg.strip_prefix("--") else {
            continue;
        };
        if let Some((key, value)) = flag.split_once('=') {
            set_value(&mut start_identifier, &mut verify_code, key, value)?;
            continue;
        }
        match normalize_key(flag).as_str() {
            "babelauthreset" => reset = true,
            "babelauthstartlogin" | "babelauthverifylogin" => pending_key = Some(flag.to_string()),
            _ => {}
        }
    }

    if let Some(key) = pending_key {
        bail!("--{key} requires a value");
    }
    match (start_identifier, verify_code) {
        (Some(_), Some(_)) => {
            bail!("--babel-auth-start-login and --babel-auth-verify-login cannot be used together")
        }
        (Some(identifier), None) if identifier.trim().is_empty() => {
            bail!("--babel-auth-start-login requires a non-empty identifier")
        }
        (Some(identifier), None) => Ok(Some(AuthCliCommand::StartLogin {
            identifier: identifier.trim().to_string(),
            reset,
        })),
        (None, Some(code)) if code.trim().is_empty() => {
            bail!("--babel-auth-verify-login requires a non-empty code")
        }
        (None, Some(code)) => Ok(Some(AuthCliCommand::VerifyLogin {
            code: code.trim().to_string(),
        })),
        (None, None) => Ok(None),
    }
}

async fn run_command(command: AuthCliCommand) -> Result<()> {
    let store_path = store_path();
    let babel_url = babel_base_url();
    let clerk_url = clerk_url();
    let now = now_unix_seconds();
    match command {
        AuthCliCommand::StartLogin { identifier, reset } => {
            let result = start_phone_code_login(
                &store_path,
                &identifier,
                &babel_url,
                &clerk_url,
                reset,
                now,
            )
            .await?;
            println!(
                "{}",
                json!({
                    "storePath": store_path,
                    "pending": true,
                    "signInId": result.sign_in_id,
                    "factor": "phone_code",
                    "phoneNumberId": result.phone_number_id,
                    "readyForCode": true,
                })
            );
        }
        AuthCliCommand::VerifyLogin { code } => {
            let result =
                verify_phone_code_login(&store_path, &code, &babel_url, &clerk_url, now).await?;
            println!(
                "{}",
                json!({
                    "storePath": store_path,
                    "sessionId": result.session_id,
                    "ready": true,
                })
            );
        }
    }
    Ok(())
}

fn set_value(
    start_identifier: &mut Option<String>,
    verify_code: &mut Option<String>,
    key: &str,
    value: &str,
) -> Result<()> {
    match normalize_key(key).as_str() {
        "babelauthstartlogin" if start_identifier.replace(value.to_string()).is_some() => {
            bail!("--babel-auth-start-login was provided more than once")
        }
        "babelauthverifylogin" if verify_code.replace(value.to_string()).is_some() => {
            bail!("--babel-auth-verify-login was provided more than once")
        }
        _ => Ok(()),
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| *character != '-' && *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}
