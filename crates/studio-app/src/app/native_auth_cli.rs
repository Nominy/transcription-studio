use std::env;

use serde_json::json;
use time::OffsetDateTime;

use super::native_auth::{
    default_native_auth_store_path, native_clerk_url, start_phone_code_login,
    verify_phone_code_login,
};

const DEFAULT_BABEL_SERVICE_BASE_URL: &str = "https://dashboard.babel.audio";
const STUDIO_BABEL_SERVICE_BASE_URL: &str = "STUDIO_BABEL_SERVICE_BASE_URL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NativeAuthCliCommand {
    StartLogin { identifier: String, reset: bool },
    VerifyLogin { code: String },
}

pub(super) fn parse_native_auth_cli_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Result<Option<NativeAuthCliCommand>, String> {
    let mut start_identifier: Option<String> = None;
    let mut verify_code: Option<String> = None;
    let mut reset = false;
    let mut pending_key: Option<String> = None;

    for arg in args.into_iter().skip(1) {
        let arg = arg.as_ref();
        if let Some(key) = pending_key.take() {
            set_cli_value(&mut start_identifier, &mut verify_code, &key, arg)?;
            continue;
        }

        let Some(flag) = arg.strip_prefix("--") else {
            continue;
        };
        if let Some((key, value)) = flag.split_once('=') {
            set_cli_value(&mut start_identifier, &mut verify_code, key, value)?;
            continue;
        }

        match normalize_key(flag).as_str() {
            "babelauthreset" => reset = true,
            "babelauthstartlogin" | "babelauthverifylogin" => {
                pending_key = Some(flag.to_string());
            }
            _ => {}
        }
    }

    if let Some(key) = pending_key {
        return Err(format!("--{key} requires a value"));
    }
    match (start_identifier, verify_code) {
        (Some(_), Some(_)) => Err(
            "--babel-auth-start-login and --babel-auth-verify-login cannot be used together"
                .to_string(),
        ),
        (Some(identifier), None) if identifier.trim().is_empty() => {
            Err("--babel-auth-start-login requires a non-empty identifier".to_string())
        }
        (Some(identifier), None) => Ok(Some(NativeAuthCliCommand::StartLogin {
            identifier: identifier.trim().to_string(),
            reset,
        })),
        (None, Some(code)) if code.trim().is_empty() => {
            Err("--babel-auth-verify-login requires a non-empty code".to_string())
        }
        (None, Some(code)) => Ok(Some(NativeAuthCliCommand::VerifyLogin {
            code: code.trim().to_string(),
        })),
        (None, None) => Ok(None),
    }
}

pub(super) fn run_native_auth_cli_from_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<Result<(), String>> {
    let command = match parse_native_auth_cli_args(args) {
        Ok(Some(command)) => command,
        Ok(None) => return None,
        Err(error) => return Some(Err(error)),
    };
    Some(run_native_auth_cli_command(command))
}

fn run_native_auth_cli_command(command: NativeAuthCliCommand) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to start native auth runtime: {error}"))?;
    runtime.block_on(async move {
        let store_path = default_native_auth_store_path();
        let babel_url = native_babel_url();
        let clerk_url = native_clerk_url();
        let now = OffsetDateTime::now_utc().unix_timestamp();
        match command {
            NativeAuthCliCommand::StartLogin { identifier, reset } => {
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
            NativeAuthCliCommand::VerifyLogin { code } => {
                let result =
                    verify_phone_code_login(&store_path, &code, &babel_url, &clerk_url, now)
                        .await?;
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
    })
}

fn set_cli_value(
    start_identifier: &mut Option<String>,
    verify_code: &mut Option<String>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    match normalize_key(key).as_str() {
        "babelauthstartlogin" => {
            if start_identifier.replace(value.to_string()).is_some() {
                return Err("--babel-auth-start-login was provided more than once".to_string());
            }
        }
        "babelauthverifylogin" => {
            if verify_code.replace(value.to_string()).is_some() {
                return Err("--babel-auth-verify-login was provided more than once".to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn native_babel_url() -> String {
    env::var(STUDIO_BABEL_SERVICE_BASE_URL)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_BABEL_SERVICE_BASE_URL.to_string())
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| *character != '-' && *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_start_login_with_reset() {
        assert_eq!(
            parse_native_auth_cli_args([
                "studio-app",
                "--babel-auth-start-login",
                "tester@example.com",
                "--babel-auth-reset",
            ])
            .expect("parse")
            .expect("auth command"),
            NativeAuthCliCommand::StartLogin {
                identifier: "tester@example.com".to_string(),
                reset: true,
            }
        );
    }

    #[test]
    fn parses_verify_login_code() {
        assert_eq!(
            parse_native_auth_cli_args(["studio-app", "--babel-auth-verify-login=123456"])
                .expect("parse")
                .expect("auth command"),
            NativeAuthCliCommand::VerifyLogin {
                code: "123456".to_string(),
            }
        );
    }

    #[test]
    fn ignores_regular_studio_args() {
        assert_eq!(
            parse_native_auth_cli_args(["studio-app", "--reviewActionId=abc"]).expect("parse"),
            None
        );
    }
}
