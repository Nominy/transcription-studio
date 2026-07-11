use std::{
    env, fs,
    io::{BufRead, BufReader, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

use super::platform_paths::application_support_dir;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

const RELAY_READY_PREFIX: &str = "studio-relay listening on ";
const RELAY_EXECUTABLE_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_EXE";
const RELAY_BIND_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_BIND";
const RELAY_STORE_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_STORE";
const RELAY_UPSTREAM_URL_ENV: &str = "STUDIO_SUBACCOUNT_UPSTREAM_URL";
const RELAY_COMMAND_ENV: &str = "STUDIO_BABEL_SERVICE_COMMAND";
const RELAY_TASK_CENTS_ENV: &str = "STUDIO_SUBACCOUNT_TASK_CENTS";
const DEFAULT_RELAY_BIND: &str = "127.0.0.1:8787";
const DEFAULT_RELAY_STORE_FILE_NAME: &str = "subaccount-relay-store.json";
const READY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RelayProcessStatus {
    Stopped,
    Starting,
    Running,
    Failed,
}

pub(super) struct RelayProcessState {
    pub(super) status: RelayProcessStatus,
    pub(super) child: Option<Child>,
    pub(super) url: String,
    pub(super) last_error: Option<String>,
}

impl Default for RelayProcessState {
    fn default() -> Self {
        Self {
            status: RelayProcessStatus::Stopped,
            child: None,
            url: String::new(),
            last_error: None,
        }
    }
}

impl Drop for RelayProcessState {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = stop_relay_child(&mut child);
        }
    }
}

impl RelayProcessState {
    pub(super) fn can_start(&self) -> bool {
        matches!(
            self.status,
            RelayProcessStatus::Stopped | RelayProcessStatus::Failed
        )
    }

    pub(super) fn can_stop(&self) -> bool {
        matches!(
            self.status,
            RelayProcessStatus::Starting | RelayProcessStatus::Running
        ) && self.child.is_some()
    }

    pub(super) fn label(&self) -> String {
        match self.status {
            RelayProcessStatus::Stopped => "Stopped".to_string(),
            RelayProcessStatus::Starting => "Starting...".to_string(),
            RelayProcessStatus::Running => {
                if self.url.trim().is_empty() {
                    "Running".to_string()
                } else {
                    format!("Running at {}", self.url)
                }
            }
            RelayProcessStatus::Failed => self
                .last_error
                .as_ref()
                .map(|error| format!("Failed: {error}"))
                .unwrap_or_else(|| "Failed".to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RelayLaunchConfig {
    pub(super) executable_path: PathBuf,
    pub(super) bind: String,
    pub(super) store_path: PathBuf,
    pub(super) babel_service_command: Option<String>,
    pub(super) upstream_url: Option<String>,
    pub(super) task_cents: Option<String>,
}

pub(super) struct StartedRelay {
    pub(super) child: Child,
    pub(super) stderr: ChildStderr,
}

impl RelayLaunchConfig {
    pub(super) fn from_environment() -> Result<Self, String> {
        let executable_path = env::var_os(RELAY_EXECUTABLE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(default_relay_executable_path);
        let bind = non_empty_env(RELAY_BIND_ENV).unwrap_or_else(|| DEFAULT_RELAY_BIND.to_string());
        validate_bind(&bind)?;
        let store_path = env::var_os(RELAY_STORE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(default_relay_store_path);
        Ok(Self {
            executable_path,
            bind,
            store_path,
            babel_service_command: non_empty_env(RELAY_COMMAND_ENV),
            upstream_url: non_empty_env(RELAY_UPSTREAM_URL_ENV),
            task_cents: non_empty_env(RELAY_TASK_CENTS_ENV),
        })
    }

    pub(super) fn remembered_url(&self) -> String {
        format!("http://{}", self.bind)
    }

    pub(super) fn args(&self) -> Vec<String> {
        let mut args = vec![
            "--bind".to_string(),
            self.bind.clone(),
            "--store".to_string(),
            self.store_path.display().to_string(),
        ];
        if let Some(command) = self.babel_service_command.as_ref() {
            args.push("--babel-service-command".to_string());
            args.push(command.clone());
        } else if let Some(upstream_url) = self.upstream_url.as_ref() {
            args.push("--upstream-url".to_string());
            args.push(upstream_url.clone());
        }
        if let Some(task_cents) = self.task_cents.as_ref() {
            args.push("--task-cents".to_string());
            args.push(task_cents.clone());
        }
        args
    }
}

pub(super) fn default_relay_store_path() -> PathBuf {
    application_support_dir().join(DEFAULT_RELAY_STORE_FILE_NAME)
}

fn default_relay_executable_path() -> PathBuf {
    env::current_exe()
        .ok()
        .as_deref()
        .map(relay_executable_path_for_app_exe)
        .unwrap_or_else(|| PathBuf::from(relay_executable_file_name()))
}

pub(super) fn relay_executable_path_for_app_exe(app_exe: &Path) -> PathBuf {
    app_exe.with_file_name(relay_executable_file_name())
}

fn relay_executable_file_name() -> &'static str {
    if cfg!(windows) {
        "studio-relay.exe"
    } else {
        "studio-relay"
    }
}

pub(super) fn start_relay_process(config: &RelayLaunchConfig) -> Result<StartedRelay, String> {
    if let Some(parent) = config.store_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create relay store directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut command = Command::new(&config.executable_path);
    command
        .args(config.args())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(parent) = config.executable_path.parent() {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|error| {
        format!(
            "failed to start relay executable {}: {error}",
            config.executable_path.display()
        )
    })?;
    let Some(stderr) = child.stderr.take() else {
        let _ = stop_relay_child(&mut child);
        return Err("relay process did not expose stderr for readiness.".to_string());
    };
    Ok(StartedRelay { child, stderr })
}

pub(super) async fn wait_for_relay_ready(stderr: ChildStderr) -> Result<String, String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(read_relay_ready_from_reader(stderr));
    });
    match receiver.recv_timeout(READY_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "relay did not report readiness within {} seconds",
            READY_TIMEOUT.as_secs()
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("relay readiness reader stopped unexpectedly".to_string())
        }
    }
}

pub(super) fn read_relay_ready_from_reader(reader: impl Read) -> Result<String, String> {
    let mut recent_lines = Vec::new();
    for line in BufReader::new(reader).lines() {
        let line = line.map_err(|error| format!("failed to read relay stderr: {error}"))?;
        if let Some(url) = relay_ready_url_from_line(&line) {
            return Ok(url);
        }
        if !line.trim().is_empty() {
            recent_lines.push(line);
            if recent_lines.len() > 5 {
                recent_lines.remove(0);
            }
        }
    }
    if recent_lines.is_empty() {
        Err("relay exited before reporting readiness".to_string())
    } else {
        Err(format!(
            "relay exited before reporting readiness: {}",
            recent_lines.join(" | ")
        ))
    }
}

pub(super) fn relay_ready_url_from_line(line: &str) -> Option<String> {
    line.trim()
        .strip_prefix(RELAY_READY_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

pub(super) fn stop_relay_child(child: &mut Child) -> Result<(), String> {
    match child.try_wait() {
        Ok(Some(_)) => return Ok(()),
        Ok(None) => {}
        Err(error) => return Err(format!("failed to inspect relay process: {error}")),
    }
    child
        .kill()
        .map_err(|error| format!("failed to stop relay process: {error}"))?;
    child
        .wait()
        .map(|_| ())
        .map_err(|error| format!("failed to wait for relay process shutdown: {error}"))
}

fn validate_bind(bind: &str) -> Result<(), String> {
    bind.parse::<SocketAddr>()
        .map(|_| ())
        .map_err(|error| format!("relay bind address is invalid: {error}"))
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::{io::Cursor, path::PathBuf};

    use super::{
        RelayLaunchConfig, read_relay_ready_from_reader, relay_executable_path_for_app_exe,
        relay_ready_url_from_line,
    };

    #[test]
    fn parses_relay_ready_url_from_stderr_line() {
        assert_eq!(
            relay_ready_url_from_line("studio-relay listening on http://127.0.0.1:8787"),
            Some("http://127.0.0.1:8787".to_string())
        );
        assert_eq!(relay_ready_url_from_line("other log"), None);
    }

    #[test]
    fn reads_relay_ready_url_after_prior_stderr_lines() {
        let stderr =
            Cursor::new("warning before bind\nstudio-relay listening on http://127.0.0.1:3001\n");
        assert_eq!(
            read_relay_ready_from_reader(stderr).expect("ready url"),
            "http://127.0.0.1:3001"
        );
    }

    #[test]
    fn relay_executable_resolves_beside_app_executable() {
        let app_exe = if cfg!(windows) {
            PathBuf::from(r"C:\Program Files\TranscriptionStudio\TranscriptionStudio.exe")
        } else {
            PathBuf::from(
                "/Applications/TranscriptionStudio.app/Contents/MacOS/TranscriptionStudio",
            )
        };
        let relay = relay_executable_path_for_app_exe(&app_exe);
        assert_eq!(
            relay.file_name().and_then(|name| name.to_str()),
            Some(if cfg!(windows) {
                "studio-relay.exe"
            } else {
                "studio-relay"
            })
        );
        assert_eq!(relay.parent(), app_exe.parent());
    }

    #[test]
    fn launch_args_use_command_before_http_upstream() {
        let config = RelayLaunchConfig {
            executable_path: PathBuf::from("studio-relay.exe"),
            bind: "127.0.0.1:8787".to_string(),
            store_path: PathBuf::from(r"C:\relay-store.json"),
            babel_service_command: Some("node bridge.js".to_string()),
            upstream_url: Some("https://relay-upstream.example/envelope".to_string()),
            task_cents: Some("42".to_string()),
        };
        assert_eq!(
            config.args(),
            vec![
                "--bind",
                "127.0.0.1:8787",
                "--store",
                r"C:\relay-store.json",
                "--babel-service-command",
                "node bridge.js",
                "--task-cents",
                "42",
            ]
        );
    }
}
