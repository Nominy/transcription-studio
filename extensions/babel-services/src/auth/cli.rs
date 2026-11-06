//! `--babel-auth-store <auth.json> --babel-auth-start-login <identifier> [--babel-auth-reset]`
//! and `--babel-auth-store <auth.json> --babel-auth-verify-login <code>`: headless native
//! login, run from `main` before any window opens. Prints one JSON status line on success.
//! The store path is an account's `auth.json` (see [`crate::account`]); there is no default.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Result, bail};
use serde_json::json;

use super::{
    babel_base_url, clerk_url, now_unix_seconds, start_phone_code_login, verify_phone_code_login,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthCliCommand {
    StartLogin { identifier: String, reset: bool },
    VerifyLogin { code: String },
}

/// A parsed auth invocation: the store to act on and what to do with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthCli {
    pub store: PathBuf,
    pub command: AuthCliCommand,
}

/// Runs an auth CLI command if `args` (including argv[0]) contain one.
/// `None` → not an auth invocation, continue starting the app.
/// `Some(code)` → exit with `code` (errors are printed to stderr, exit 1).
pub fn run_from_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Option<ExitCode> {
    let result = match parse_args(args) {
        Ok(None) => return None,
        Ok(Some(cli)) => studio_services::runtime::block_on(run_command(cli)),
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

#[derive(Default)]
struct ParsedFlags {
    store: Option<String>,
    start_identifier: Option<String>,
    verify_code: Option<String>,
    reset: bool,
}

/// Parses auth flags (`--flag value` or `--flag=value`; dashes/underscores/case ignored).
pub fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Result<Option<AuthCli>> {
    let mut flags = ParsedFlags::default();
    let mut pending_key: Option<String> = None;

    for arg in args.into_iter().skip(1) {
        let arg = arg.as_ref();
        if let Some(key) = pending_key.take() {
            set_value(&mut flags, &key, arg)?;
            continue;
        }
        let Some(flag) = arg.strip_prefix("--") else {
            continue;
        };
        if let Some((key, value)) = flag.split_once('=') {
            set_value(&mut flags, key, value)?;
            continue;
        }
        match normalize_key(flag).as_str() {
            "babelauthreset" => flags.reset = true,
            "babelauthstore" | "babelauthstartlogin" | "babelauthverifylogin" => {
                pending_key = Some(flag.to_string())
            }
            _ => {}
        }
    }

    if let Some(key) = pending_key {
        bail!("--{key} requires a value");
    }
    let command = match (flags.start_identifier, flags.verify_code) {
        (Some(_), Some(_)) => {
            bail!("--babel-auth-start-login and --babel-auth-verify-login cannot be used together")
        }
        (Some(identifier), None) if identifier.trim().is_empty() => {
            bail!("--babel-auth-start-login requires a non-empty identifier")
        }
        (Some(identifier), None) => AuthCliCommand::StartLogin {
            identifier: identifier.trim().to_string(),
            reset: flags.reset,
        },
        (None, Some(code)) if code.trim().is_empty() => {
            bail!("--babel-auth-verify-login requires a non-empty code")
        }
        (None, Some(code)) => AuthCliCommand::VerifyLogin {
            code: code.trim().to_string(),
        },
        (None, None) => {
            if flags.store.is_some() {
                bail!(
                    "--babel-auth-store needs --babel-auth-start-login or --babel-auth-verify-login"
                );
            }
            return Ok(None);
        }
    };
    let store = match flags.store.map(|store| store.trim().to_string()) {
        Some(store) if !store.is_empty() => PathBuf::from(store),
        _ => bail!("--babel-auth-store <path to an account's auth.json> is required"),
    };
    Ok(Some(AuthCli { store, command }))
}

async fn run_command(cli: AuthCli) -> Result<()> {
    let store_path = cli.store;
    let babel_url = babel_base_url();
    let clerk_url = clerk_url();
    let now = now_unix_seconds();
    match cli.command {
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

fn set_value(flags: &mut ParsedFlags, key: &str, value: &str) -> Result<()> {
    match normalize_key(key).as_str() {
        "babelauthstore" if flags.store.replace(value.to_string()).is_some() => {
            bail!("--babel-auth-store was provided more than once")
        }
        "babelauthstartlogin" if flags.start_identifier.replace(value.to_string()).is_some() => {
            bail!("--babel-auth-start-login was provided more than once")
        }
        "babelauthverifylogin" if flags.verify_code.replace(value.to_string()).is_some() => {
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
