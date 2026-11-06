//! Launches and supervises the bundled `studio-relay` executable (sub-account relay host).
//!
//! Env: `STUDIO_SUBACCOUNT_RELAY_EXE` (executable, default beside the app exe),
//! `STUDIO_SUBACCOUNT_RELAY_BIND` (default `127.0.0.1:8787`), `STUDIO_SUBACCOUNT_RELAY_STORE`
//! (relay store file), `STUDIO_BABEL_SERVICE_COMMAND` (preferred upstream),
//! `STUDIO_SUBACCOUNT_UPSTREAM_URL` (HTTP upstream), `STUDIO_SUBACCOUNT_TASK_CENTS`.

use std::{
    env, fs,
    future::Future,
    io::{BufRead, BufReader, Read},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

use anyhow::{Context, Result, anyhow, bail};

pub const RELAY_EXECUTABLE_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_EXE";
pub const RELAY_BIND_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_BIND";
pub const RELAY_STORE_ENV: &str = "STUDIO_SUBACCOUNT_RELAY_STORE";
pub const RELAY_UPSTREAM_URL_ENV: &str = "STUDIO_SUBACCOUNT_UPSTREAM_URL";
pub const RELAY_COMMAND_ENV: &str = "STUDIO_BABEL_SERVICE_COMMAND";
pub const RELAY_TASK_CENTS_ENV: &str = "STUDIO_SUBACCOUNT_TASK_CENTS";
pub const DEFAULT_RELAY_BIND: &str = "127.0.0.1:8787";
/// Readiness wait bound after spawn.
pub const RELAY_READY_TIMEOUT: Duration = Duration::from_secs(10);

const RELAY_READY_PREFIX: &str = "studio-relay listening on ";
const DEFAULT_RELAY_STORE_FILE_NAME: &str = "subaccount-relay-store.json";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayLaunchConfig {
    pub executable_path: PathBuf,
    pub bind: String,
    pub store_path: PathBuf,
    pub babel_service_command: Option<String>,
    pub upstream_url: Option<String>,
    pub task_cents: Option<String>,
}

impl RelayLaunchConfig {
    /// Reads the `STUDIO_SUBACCOUNT_RELAY_*` environment; errors on an invalid bind address.
    pub fn from_environment() -> Result<Self> {
        let bind = non_empty_env(RELAY_BIND_ENV).unwrap_or_else(|| DEFAULT_RELAY_BIND.to_string());
        bind.parse::<SocketAddr>()
            .context("relay bind address is invalid")?;
        Ok(Self {
            executable_path: env::var_os(RELAY_EXECUTABLE_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(default_relay_executable_path),
            bind,
            store_path: env::var_os(RELAY_STORE_ENV)
                .map(PathBuf::from)
                .unwrap_or_else(default_relay_store_path),
            babel_service_command: non_empty_env(RELAY_COMMAND_ENV),
            upstream_url: non_empty_env(RELAY_UPSTREAM_URL_ENV),
            task_cents: non_empty_env(RELAY_TASK_CENTS_ENV),
        })
    }

    /// URL shown while the relay is starting (`http://<bind>`).
    pub fn local_url(&self) -> String {
        format!("http://{}", self.bind)
    }

    /// CLI args; a Babel service command wins over an HTTP upstream.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![
            "--bind".to_string(),
            self.bind.clone(),
            "--store".to_string(),
            self.store_path.display().to_string(),
        ];
        if let Some(command) = &self.babel_service_command {
            args.extend(["--babel-service-command".to_string(), command.clone()]);
        } else if let Some(upstream_url) = &self.upstream_url {
            args.extend(["--upstream-url".to_string(), upstream_url.clone()]);
        }
        if let Some(task_cents) = &self.task_cents {
            args.extend(["--task-cents".to_string(), task_cents.clone()]);
        }
        args
    }
}

pub fn default_relay_store_path() -> PathBuf {
    studio_services::paths::app_support_dir().join(DEFAULT_RELAY_STORE_FILE_NAME)
}

/// `studio-relay[.exe]` beside the given app executable.
pub fn relay_executable_path_for_app_exe(app_exe: &Path) -> PathBuf {
    app_exe.with_file_name(if cfg!(windows) {
        "studio-relay.exe"
    } else {
        "studio-relay"
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayStatus {
    Stopped,
    Starting,
    Running,
    Failed,
}

/// Owns the relay child process; dropping it kills the relay.
#[derive(Debug)]
pub struct RelayProcess {
    status: RelayStatus,
    url: String,
    last_error: Option<String>,
    child: Option<Child>,
}

impl Default for RelayProcess {
    fn default() -> Self {
        Self {
            status: RelayStatus::Stopped,
            url: String::new(),
            last_error: None,
            child: None,
        }
    }
}

impl RelayProcess {
    pub fn status(&self) -> RelayStatus {
        self.status
    }

    /// Local URL while starting, reported URL once running.
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn can_start(&self) -> bool {
        matches!(self.status, RelayStatus::Stopped | RelayStatus::Failed)
    }

    pub fn can_stop(&self) -> bool {
        matches!(self.status, RelayStatus::Starting | RelayStatus::Running) && self.child.is_some()
    }

    /// Spawns the relay (no console window on Windows) and returns a runtime-agnostic future
    /// resolving to its listening URL (from `studio-relay listening on <url>` on stderr) within
    /// [`RELAY_READY_TIMEOUT`]. Pass the result to [`RelayProcess::complete_start`].
    pub fn start(
        &mut self,
        config: &RelayLaunchConfig,
    ) -> Result<impl Future<Output = Result<String>> + Send + 'static> {
        if !self.can_start() {
            bail!("Sub-account relay is already running or starting.");
        }
        match spawn_relay(config) {
            Ok((child, stderr)) => {
                self.status = RelayStatus::Starting;
                self.url = config.local_url();
                self.last_error = None;
                self.child = Some(child);
                Ok(wait_for_relay_ready(stderr, RELAY_READY_TIMEOUT))
            }
            Err(error) => {
                self.status = RelayStatus::Failed;
                self.last_error = Some(format!("{error:#}"));
                Err(error)
            }
        }
    }

    /// Running on success; on failure the child is stopped and the error kept.
    pub fn complete_start(&mut self, result: &Result<String>) {
        match result {
            Ok(url) => {
                self.status = RelayStatus::Running;
                self.url = url.clone();
                self.last_error = None;
            }
            Err(error) => {
                if let Some(mut child) = self.child.take() {
                    let _ = stop_child(&mut child);
                }
                self.status = RelayStatus::Failed;
                self.last_error = Some(format!("{error:#}"));
            }
        }
    }

    /// Kills the relay and waits for it to exit.
    pub fn stop(&mut self) -> Result<()> {
        let Some(mut child) = self.child.take() else {
            self.status = RelayStatus::Stopped;
            return Ok(());
        };
        match stop_child(&mut child) {
            Ok(()) => {
                self.status = RelayStatus::Stopped;
                self.last_error = None;
                Ok(())
            }
            Err(error) => {
                self.status = RelayStatus::Failed;
                self.last_error = Some(format!("{error:#}"));
                Err(error)
            }
        }
    }
}

impl Drop for RelayProcess {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = stop_child(&mut child);
        }
    }
}

/// Reads stderr lines until the readiness line; errors with the last few lines when the relay
/// exits first.
pub fn read_relay_ready(reader: impl Read) -> Result<String> {
    let mut recent_lines = Vec::new();
    for line in BufReader::new(reader).lines() {
        let line = line.context("failed to read relay stderr")?;
        if let Some(url) = relay_ready_url(&line) {
            return Ok(url);
        }
        if !line.trim().is_empty() {
            if recent_lines.len() == 5 {
                recent_lines.remove(0);
            }
            recent_lines.push(line);
        }
    }
    if recent_lines.is_empty() {
        bail!("relay exited before reporting readiness")
    }
    bail!(
        "relay exited before reporting readiness: {}",
        recent_lines.join(" | ")
    )
}

/// URL from a `studio-relay listening on <url>` line.
pub fn relay_ready_url(line: &str) -> Option<String> {
    line.trim()
        .strip_prefix(RELAY_READY_PREFIX)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

/// Readiness of a spawned relay from its stderr, bounded by `timeout`. Uses plain threads and a
/// oneshot channel, so any executor can await it.
pub fn wait_for_relay_ready(
    stderr: impl Read + Send + 'static,
    timeout: Duration,
) -> impl Future<Output = Result<String>> + Send + 'static {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = Arc::new(Mutex::new(Some(sender)));
    let send = move |result: Result<String>| {
        if let Some(sender) = sender.lock().ok().and_then(|mut sender| sender.take()) {
            let _ = sender.send(result);
        }
    };
    let timeout_send = send.clone();
    std::thread::spawn(move || send(read_relay_ready(stderr)));
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        timeout_send(Err(anyhow!(
            "relay did not report readiness within {} seconds",
            timeout.as_secs()
        )));
    });
    async move {
        receiver
            .await
            .unwrap_or_else(|_| Err(anyhow!("relay readiness reader stopped unexpectedly")))
    }
}

fn spawn_relay(config: &RelayLaunchConfig) -> Result<(Child, ChildStderr)> {
    if let Some(parent) = config.store_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create relay store directory {}",
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
    if let Some(parent) = config
        .executable_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        command.current_dir(parent);
    }
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to start relay executable {}",
            config.executable_path.display()
        )
    })?;
    match child.stderr.take() {
        Some(stderr) => Ok((child, stderr)),
        None => {
            let _ = stop_child(&mut child);
            bail!("relay process did not expose stderr for readiness.")
        }
    }
}

fn stop_child(child: &mut Child) -> Result<()> {
    if child
        .try_wait()
        .context("failed to inspect relay process")?
        .is_some()
    {
        return Ok(());
    }
    child.kill().context("failed to stop relay process")?;
    child
        .wait()
        .map(drop)
        .context("failed to wait for relay process shutdown")
}

fn default_relay_executable_path() -> PathBuf {
    env::current_exe()
        .map(|exe| relay_executable_path_for_app_exe(&exe))
        .unwrap_or_else(|_| relay_executable_path_for_app_exe(Path::new("studio-relay")))
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}
