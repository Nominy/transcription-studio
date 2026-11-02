//! Velopack self-update: version/feed metadata, update check, download + apply.
//!
//! Compile-time env: `STUDIO_DEFAULT_UPDATE_FEED_URL` (feed override baked into
//! the build), `STUDIO_BUILD_GIT_COMMIT` (defaults to `unknown`). The app passes
//! its own version to [`UpdateState::new`]. Runtime env: `STUDIO_UPDATE_FEED_URL`
//! overrides the feed.

use anyhow::{Context, Result, anyhow, bail};
use velopack::{UpdateCheck, UpdateManager, VelopackApp, sources};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM_UPDATE_FEED_URL: &str = "https://reviewgen.ovh/transcription-studio/alpha/mac-arm64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const PLATFORM_UPDATE_FEED_URL: &str = "https://reviewgen.ovh/transcription-studio/alpha/mac-x64";
#[cfg(not(target_os = "macos"))]
const PLATFORM_UPDATE_FEED_URL: &str = "https://reviewgen.ovh/transcription-studio/alpha/win-x64";

/// Feed baked into this build (`STUDIO_DEFAULT_UPDATE_FEED_URL` or the platform alpha feed).
pub const DEFAULT_UPDATE_FEED_URL: &str = match option_env!("STUDIO_DEFAULT_UPDATE_FEED_URL") {
    Some(url) => url,
    None => PLATFORM_UPDATE_FEED_URL,
};
/// Packaged git commit (`STUDIO_BUILD_GIT_COMMIT`) or `unknown`.
pub const BUILD_COMMIT: &str = match option_env!("STUDIO_BUILD_GIT_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};
const UPDATE_FEED_URL_ENV: &str = "STUDIO_UPDATE_FEED_URL";

/// Velopack install/uninstall/update hooks. Call first thing in `main`
/// (may exit the process when Velopack invoked us for a hook).
pub fn run_startup_hooks() {
    VelopackApp::build().run();
}

/// `STUDIO_UPDATE_FEED_URL` when set and non-blank, else [`DEFAULT_UPDATE_FEED_URL`].
pub fn configured_feed_url() -> String {
    std::env::var(UPDATE_FEED_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_UPDATE_FEED_URL.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    UpdateAvailable,
    UpToDate,
    RemoteEmpty,
    Installing,
    Restarting,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateCheckOutcome {
    Available { version: String },
    UpToDate,
    RemoteEmpty,
}

/// Settings-panel updater state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateState {
    pub feed_url: String,
    pub current_version: String,
    pub build_commit: String,
    pub phase: UpdatePhase,
    pub available_version: Option<String>,
    pub last_error: Option<String>,
}

impl UpdateState {
    /// Idle state for an app build identified by `current_version` (the binary's version).
    pub fn new(current_version: impl Into<String>) -> Self {
        Self {
            feed_url: configured_feed_url(),
            current_version: current_version.into(),
            build_commit: BUILD_COMMIT.to_string(),
            phase: UpdatePhase::Idle,
            available_version: None,
            last_error: None,
        }
    }
}

impl UpdateState {
    fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking | UpdatePhase::Installing | UpdatePhase::Restarting
        )
    }

    pub fn can_check(&self) -> bool {
        !self.is_busy()
    }

    pub fn can_install(&self) -> bool {
        self.available_version.is_some() && !self.is_busy()
    }

    pub fn is_error(&self) -> bool {
        self.phase == UpdatePhase::Failed
    }

    pub fn status_label(&self) -> String {
        match self.phase {
            UpdatePhase::Idle => "Ready to check for alpha updates.".to_string(),
            UpdatePhase::Checking => "Checking for updates...".to_string(),
            UpdatePhase::UpdateAvailable => match self.available_version.as_ref() {
                Some(version) => format!("Update {version} is available."),
                None => "Update is available.".to_string(),
            },
            UpdatePhase::UpToDate => "You are on the latest alpha build.".to_string(),
            UpdatePhase::RemoteEmpty => "The alpha update feed is empty.".to_string(),
            UpdatePhase::Installing => "Downloading and installing update...".to_string(),
            UpdatePhase::Restarting => "Update installed. Restarting...".to_string(),
            UpdatePhase::Failed => format!(
                "Update failed: {}",
                self.last_error
                    .as_deref()
                    .unwrap_or("unknown updater error")
            ),
        }
    }

    /// Returns false (no-op) while busy.
    pub fn begin_check(&mut self) -> bool {
        if !self.can_check() {
            return false;
        }
        self.phase = UpdatePhase::Checking;
        self.last_error = None;
        true
    }

    pub fn finish_check(&mut self, result: Result<UpdateCheckOutcome, String>) {
        match result {
            Ok(UpdateCheckOutcome::RemoteEmpty) => {
                self.phase = UpdatePhase::RemoteEmpty;
                self.available_version = None;
                self.last_error = None;
            }
            Ok(UpdateCheckOutcome::UpToDate) => {
                self.phase = UpdatePhase::UpToDate;
                self.available_version = None;
                self.last_error = None;
            }
            Ok(UpdateCheckOutcome::Available { version }) => {
                self.phase = UpdatePhase::UpdateAvailable;
                self.available_version = Some(version);
                self.last_error = None;
            }
            Err(error) => {
                self.phase = UpdatePhase::Failed;
                self.last_error = Some(error);
            }
        }
    }

    /// Returns false (no-op) unless an update is available and nothing is running.
    pub fn begin_install(&mut self) -> bool {
        if !self.can_install() {
            return false;
        }
        self.phase = UpdatePhase::Installing;
        self.last_error = None;
        true
    }

    pub fn finish_install(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.phase = UpdatePhase::Restarting;
                self.last_error = None;
            }
            Err(error) => {
                self.phase = UpdatePhase::Failed;
                self.last_error = Some(error);
            }
        }
    }
}

/// Queries the feed (blocking Velopack call on a blocking thread). Fails on non-installed builds.
pub async fn check_for_updates(feed_url: String) -> Result<UpdateCheckOutcome> {
    tokio::task::spawn_blocking(move || check_for_updates_blocking(&feed_url))
        .await
        .context("updater task failed")?
}

/// Downloads the available update, applies it and restarts the app (on success
/// the process is replaced; `Ok` is only observed if the restart is deferred).
pub async fn install_available_update(feed_url: String) -> Result<()> {
    tokio::task::spawn_blocking(move || install_available_update_blocking(&feed_url))
        .await
        .context("updater task failed")?
}

fn check_for_updates_blocking(feed_url: &str) -> Result<UpdateCheckOutcome> {
    let manager = update_manager(feed_url)?;
    Ok(
        match manager
            .check_for_updates()
            .map_err(|error| anyhow!("update check failed: {error}"))?
        {
            UpdateCheck::RemoteIsEmpty => UpdateCheckOutcome::RemoteEmpty,
            UpdateCheck::NoUpdateAvailable => UpdateCheckOutcome::UpToDate,
            UpdateCheck::UpdateAvailable(update) => UpdateCheckOutcome::Available {
                version: update.TargetFullRelease.Version,
            },
        },
    )
}

fn install_available_update_blocking(feed_url: &str) -> Result<()> {
    let manager = update_manager(feed_url)?;
    let update = match manager
        .check_for_updates()
        .map_err(|error| anyhow!("update check failed: {error}"))?
    {
        UpdateCheck::UpdateAvailable(update) => *update,
        UpdateCheck::NoUpdateAvailable => bail!("no update is available"),
        UpdateCheck::RemoteIsEmpty => bail!("the update feed is empty"),
    };
    manager
        .download_updates(&update, None)
        .map_err(|error| anyhow!("update download failed: {error}"))?;
    manager
        .apply_updates_and_restart(&update)
        .map_err(|error| anyhow!("update restart failed: {error}"))
}

fn update_manager(feed_url: &str) -> Result<UpdateManager> {
    UpdateManager::new(sources::HttpSource::new(feed_url), None, None).map_err(|error| {
        anyhow!("Velopack updater is available only from an installed build: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_and_install_transitions() {
        let mut state = UpdateState::new("1.0.0");
        assert!(state.begin_check());
        assert!(!state.begin_check(), "checking blocks a second check");
        state.finish_check(Ok(UpdateCheckOutcome::Available {
            version: "1.2.3".to_string(),
        }));
        assert_eq!(state.phase, UpdatePhase::UpdateAvailable);
        assert!(state.can_install());
        assert!(state.begin_install());
        assert!(!state.can_check());
        state.finish_install(Err("disk full".to_string()));
        assert!(state.is_error());
        assert_eq!(state.last_error.as_deref(), Some("disk full"));
        assert!(state.can_install(), "a failed install can be retried");

        state.finish_check(Ok(UpdateCheckOutcome::RemoteEmpty));
        assert_eq!(state.phase, UpdatePhase::RemoteEmpty);
        assert!(!state.can_install());
    }
}
