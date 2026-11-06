//! Velopack self-update: release channels, update checks, download + apply.
//!
//! Two channels: **Stable** reads GitHub Releases of [`STABLE_REPO_URL`] (pre-releases
//! excluded); **Alpha** reads the VPS feed every `main` build is published to.
//!
//! Compile-time env: `STUDIO_UPDATE_CHANNEL` (`stable` | `alpha`, the channel this build
//! was packaged for; unset means `stable`), `STUDIO_ALPHA_FEED_URL` (alpha feed baked into
//! the build, for per-target macOS feeds; defaults to the platform alpha feed),
//! `STUDIO_BUILD_GIT_COMMIT` (defaults to `unknown`). The app passes its own version to
//! [`UpdateState::new`].

use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use velopack::locator::{self, LocationContext};
use velopack::sources::{GithubSource, HttpSource, UpdateSource};
use velopack::{UpdateCheck, UpdateManager, UpdateOptions, VelopackApp};

/// GitHub repository whose (non-pre-)releases form the stable channel.
pub const STABLE_REPO_URL: &str = "https://github.com/Nominy/exaudio";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM_ALPHA_FEED_URL: &str = "https://reviewgen.ovh/exaudio/alpha/mac-arm64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const PLATFORM_ALPHA_FEED_URL: &str = "https://reviewgen.ovh/exaudio/alpha/mac-x64";
#[cfg(not(target_os = "macos"))]
const PLATFORM_ALPHA_FEED_URL: &str = "https://reviewgen.ovh/exaudio/alpha/win-x64";

/// Alpha feed baked into this build (`STUDIO_ALPHA_FEED_URL` or the platform alpha feed).
pub const ALPHA_FEED_URL: &str = match option_env!("STUDIO_ALPHA_FEED_URL") {
    Some(url) => url,
    None => PLATFORM_ALPHA_FEED_URL,
};

/// Velopack channel stable packages are built with (Velopack's per-OS default channel).
#[cfg(target_os = "windows")]
const STABLE_VELOPACK_CHANNEL: &str = "win";
#[cfg(target_os = "macos")]
const STABLE_VELOPACK_CHANNEL: &str = "osx";
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const STABLE_VELOPACK_CHANNEL: &str = "linux";

/// Velopack channel alpha packages are built with, on every platform.
const ALPHA_VELOPACK_CHANNEL: &str = "alpha";

/// Packaged git commit (`STUDIO_BUILD_GIT_COMMIT`) or `unknown`.
pub const BUILD_COMMIT: &str = match option_env!("STUDIO_BUILD_GIT_COMMIT") {
    Some(commit) => commit,
    None => "unknown",
};

/// The channel this build was packaged for (`STUDIO_UPDATE_CHANNEL`); the default until
/// the user picks another one. An unknown value fails the build.
pub const BUILD_CHANNEL: UpdateChannel = match option_env!("STUDIO_UPDATE_CHANNEL") {
    None => UpdateChannel::Stable,
    Some(value) => match UpdateChannel::parse_const(value) {
        Some(channel) => channel,
        None => panic!("STUDIO_UPDATE_CHANNEL must be `stable` or `alpha`"),
    },
};

/// Minimum time between two update checks while the app runs.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateChannel {
    Stable,
    Alpha,
}

impl UpdateChannel {
    pub const ALL: [Self; 2] = [Self::Stable, Self::Alpha];

    /// Storage / `STUDIO_UPDATE_CHANNEL` spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Alpha => "alpha",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::parse_const(value.trim())
    }

    const fn parse_const(value: &str) -> Option<Self> {
        match value.as_bytes() {
            b"stable" => Some(Self::Stable),
            b"alpha" => Some(Self::Alpha),
            _ => None,
        }
    }

    /// Where this channel's releases are read from (shown in Settings).
    pub const fn source_url(self) -> &'static str {
        match self {
            Self::Stable => STABLE_REPO_URL,
            Self::Alpha => ALPHA_FEED_URL,
        }
    }

    const fn velopack_channel(self) -> &'static str {
        match self {
            Self::Stable => STABLE_VELOPACK_CHANNEL,
            Self::Alpha => ALPHA_VELOPACK_CHANNEL,
        }
    }

    fn source(self) -> Box<dyn UpdateSource> {
        match self {
            Self::Stable => Box::new(GithubSource::new(STABLE_REPO_URL, None, false)),
            Self::Alpha => Box::new(HttpSource::new(ALPHA_FEED_URL)),
        }
    }
}

/// The user's stored channel choice, else the channel this build was packaged for.
pub fn resolve_channel(stored: Option<&str>) -> UpdateChannel {
    stored
        .and_then(UpdateChannel::parse)
        .unwrap_or(BUILD_CHANNEL)
}

/// How long until the next background check is due, given when the last check (manual or
/// background) started. Zero when none ran yet or the interval has elapsed.
pub fn next_check_delay(last_check: Option<Instant>, now: Instant) -> Duration {
    match last_check {
        None => Duration::ZERO,
        Some(at) => CHECK_INTERVAL.saturating_sub(now.saturating_duration_since(at)),
    }
}

/// Velopack install/uninstall/update hooks. Call first thing in `main`
/// (may exit the process when Velopack invoked us for a hook).
pub fn run_startup_hooks() {
    VelopackApp::build().run();
}

/// True when this process runs from a Velopack install (installer or portable bundle);
/// source builds cannot update themselves.
pub fn is_installed() -> bool {
    locator::auto_locate_app_manifest(LocationContext::FromCurrentExe).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    /// Not a Velopack install: checks and installs are unavailable.
    Unsupported,
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

/// Updater state machine shared by Settings and the update notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateState {
    pub channel: UpdateChannel,
    pub current_version: String,
    pub build_commit: String,
    pub phase: UpdatePhase,
    pub available_version: Option<String>,
    /// Download progress (0–100) while installing, when the source reports it.
    pub download_percent: Option<u8>,
    pub last_error: Option<String>,
}

impl UpdateState {
    /// State for an app build identified by `current_version` (the binary's version);
    /// `installed` is [`is_installed`].
    pub fn new(
        current_version: impl Into<String>,
        channel: UpdateChannel,
        installed: bool,
    ) -> Self {
        Self {
            channel,
            current_version: current_version.into(),
            build_commit: BUILD_COMMIT.to_string(),
            phase: if installed {
                UpdatePhase::Idle
            } else {
                UpdatePhase::Unsupported
            },
            available_version: None,
            download_percent: None,
            last_error: None,
        }
    }

    fn is_busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking | UpdatePhase::Installing | UpdatePhase::Restarting
        )
    }

    pub fn is_supported(&self) -> bool {
        self.phase != UpdatePhase::Unsupported
    }

    pub fn can_check(&self) -> bool {
        self.is_supported() && !self.is_busy()
    }

    pub fn can_install(&self) -> bool {
        self.available_version.is_some() && self.can_check()
    }

    /// The channel can change whenever nothing is running.
    pub fn can_switch_channel(&self) -> bool {
        !self.is_busy()
    }

    pub fn is_error(&self) -> bool {
        self.phase == UpdatePhase::Failed
    }

    /// The version the unobtrusive update notice offers: an available update the user
    /// has not skipped, while it is not already restarting into it.
    pub fn notice_version(&self, skipped: Option<&str>) -> Option<&str> {
        if self.phase == UpdatePhase::Restarting {
            return None;
        }
        self.available_version
            .as_deref()
            .filter(|version| Some(*version) != skipped)
    }

    /// Switches channel, forgetting what the previous channel offered. Returns false
    /// (no-op) while busy or when the channel is unchanged.
    pub fn switch_channel(&mut self, channel: UpdateChannel) -> bool {
        if !self.can_switch_channel() || self.channel == channel {
            return false;
        }
        self.channel = channel;
        self.available_version = None;
        self.last_error = None;
        if self.is_supported() {
            self.phase = UpdatePhase::Idle;
        }
        true
    }

    /// Returns false (no-op) while busy or unsupported.
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
        self.download_percent = None;
        self.last_error = None;
        true
    }

    /// Records download progress; ignored unless installing.
    pub fn set_download_percent(&mut self, percent: u8) {
        if self.phase == UpdatePhase::Installing {
            self.download_percent = Some(percent.min(100));
        }
    }

    pub fn finish_install(&mut self, result: Result<(), String>) {
        self.download_percent = None;
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

/// Queries `channel` (blocking Velopack call on a blocking thread). Fails on non-installed builds.
pub async fn check_for_updates(channel: UpdateChannel) -> Result<UpdateCheckOutcome> {
    tokio::task::spawn_blocking(move || check_for_updates_blocking(channel))
        .await
        .context("updater task failed")?
}

/// Downloads the update `channel` offers (reporting 0–100 to `on_progress`), applies it and
/// restarts the app (on success the process is replaced; `Ok` is only observed if the
/// restart is deferred).
pub async fn install_available_update(
    channel: UpdateChannel,
    on_progress: impl Fn(u8) + Send + 'static,
) -> Result<()> {
    tokio::task::spawn_blocking(move || install_available_update_blocking(channel, on_progress))
        .await
        .context("updater task failed")?
}

fn check_for_updates_blocking(channel: UpdateChannel) -> Result<UpdateCheckOutcome> {
    let manager = update_manager(channel)?;
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

fn install_available_update_blocking(
    channel: UpdateChannel,
    on_progress: impl Fn(u8) + Send,
) -> Result<()> {
    let manager = update_manager(channel)?;
    let update = match manager
        .check_for_updates()
        .map_err(|error| anyhow!("update check failed: {error}"))?
    {
        UpdateCheck::UpdateAvailable(update) => *update,
        UpdateCheck::NoUpdateAvailable => bail!("no update is available"),
        UpdateCheck::RemoteIsEmpty => bail!("the update feed is empty"),
    };
    let (progress, reports) = std::sync::mpsc::channel::<i16>();
    std::thread::scope(|scope| {
        // Ends once Velopack drops `progress` (when `download_updates` returns).
        scope.spawn(move || {
            for percent in reports {
                on_progress(percent.clamp(0, 100) as u8);
            }
        });
        manager.download_updates(&update, Some(progress))
    })
    .map_err(|error| anyhow!("update download failed: {error}"))?;
    manager
        .apply_updates_and_restart(&update)
        .map_err(|error| anyhow!("update restart failed: {error}"))
}

fn update_manager(channel: UpdateChannel) -> Result<UpdateManager> {
    let options = UpdateOptions {
        ExplicitChannel: Some(channel.velopack_channel().to_string()),
        ..UpdateOptions::default()
    };
    UpdateManager::new_boxed(channel.source(), Some(options), None).map_err(|error| {
        anyhow!("Velopack updater is available only from an installed build: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_and_install_transitions() {
        let mut state = UpdateState::new("1.0.0", UpdateChannel::Stable, true);
        assert!(state.begin_check());
        assert!(!state.begin_check(), "checking blocks a second check");
        state.finish_check(Ok(UpdateCheckOutcome::Available {
            version: "1.2.3".to_string(),
        }));
        assert_eq!(state.phase, UpdatePhase::UpdateAvailable);
        assert!(state.can_install());
        assert!(state.begin_install());
        assert!(!state.can_check());
        assert!(!state.can_switch_channel());
        state.set_download_percent(42);
        assert_eq!(state.download_percent, Some(42));
        state.finish_install(Err("disk full".to_string()));
        assert!(state.is_error());
        assert_eq!(state.download_percent, None);
        assert_eq!(state.last_error.as_deref(), Some("disk full"));
        assert!(state.can_install(), "a failed install can be retried");

        state.finish_check(Ok(UpdateCheckOutcome::RemoteEmpty));
        assert_eq!(state.phase, UpdatePhase::RemoteEmpty);
        assert!(!state.can_install());
    }

    #[test]
    fn source_builds_cannot_check_or_install() {
        let mut state = UpdateState::new("1.0.0", UpdateChannel::Alpha, false);
        assert!(!state.can_check());
        assert!(!state.begin_check());
        assert!(state.switch_channel(UpdateChannel::Stable));
        assert_eq!(state.phase, UpdatePhase::Unsupported);
    }

    #[test]
    fn stored_channel_overrides_the_build_channel() {
        assert_eq!(resolve_channel(Some("alpha")), UpdateChannel::Alpha);
        assert_eq!(resolve_channel(Some(" stable ")), UpdateChannel::Stable);
        assert_eq!(resolve_channel(None), BUILD_CHANNEL);
        assert_eq!(resolve_channel(Some("nightly")), BUILD_CHANNEL);
        for channel in UpdateChannel::ALL {
            assert_eq!(UpdateChannel::parse(channel.as_str()), Some(channel));
        }
    }

    #[test]
    fn channels_map_to_their_velopack_feeds() {
        assert_eq!(UpdateChannel::Alpha.velopack_channel(), "alpha");
        assert_eq!(UpdateChannel::Stable.source_url(), STABLE_REPO_URL);
        assert_eq!(UpdateChannel::Alpha.source_url(), ALPHA_FEED_URL);
        #[cfg(windows)]
        assert_eq!(UpdateChannel::Stable.velopack_channel(), "win");
    }

    #[test]
    fn switching_channel_drops_the_previous_offer() {
        let mut state = UpdateState::new("1.0.0", UpdateChannel::Stable, true);
        state.begin_check();
        assert!(!state.switch_channel(UpdateChannel::Alpha), "busy");
        state.finish_check(Ok(UpdateCheckOutcome::Available {
            version: "1.1.0".to_string(),
        }));
        assert!(!state.switch_channel(UpdateChannel::Stable), "unchanged");
        assert!(state.switch_channel(UpdateChannel::Alpha));
        assert_eq!(state.channel, UpdateChannel::Alpha);
        assert_eq!(state.phase, UpdatePhase::Idle);
        assert_eq!(state.available_version, None);
    }

    #[test]
    fn notice_hides_skipped_and_restarting_versions() {
        let mut state = UpdateState::new("1.0.0", UpdateChannel::Stable, true);
        assert_eq!(state.notice_version(None), None);
        state.begin_check();
        state.finish_check(Ok(UpdateCheckOutcome::Available {
            version: "1.1.0".to_string(),
        }));
        assert_eq!(state.notice_version(None), Some("1.1.0"));
        assert_eq!(state.notice_version(Some("1.0.5")), Some("1.1.0"));
        assert_eq!(state.notice_version(Some("1.1.0")), None);
        state.begin_install();
        assert_eq!(state.notice_version(None), Some("1.1.0"));
        state.finish_install(Ok(()));
        assert_eq!(state.notice_version(None), None);
    }

    #[test]
    fn background_checks_wait_six_hours_after_the_last_check() {
        let start = Instant::now();
        assert_eq!(next_check_delay(None, start), Duration::ZERO);
        assert_eq!(next_check_delay(Some(start), start), CHECK_INTERVAL);
        let two_hours = Duration::from_secs(2 * 60 * 60);
        assert_eq!(
            next_check_delay(Some(start), start + two_hours),
            CHECK_INTERVAL - two_hours
        );
        assert_eq!(
            next_check_delay(Some(start), start + CHECK_INTERVAL + two_hours),
            Duration::ZERO
        );
    }
}
