use velopack::{UpdateCheck, UpdateManager, sources};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const PLATFORM_UPDATE_FEED_URL: &str = "https://reviewgen.ovh/transcription-studio/alpha/mac-arm64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const PLATFORM_UPDATE_FEED_URL: &str = "https://reviewgen.ovh/transcription-studio/alpha/mac-x64";
#[cfg(not(target_os = "macos"))]
const PLATFORM_UPDATE_FEED_URL: &str = "https://reviewgen.ovh/transcription-studio/alpha/win-x64";
pub(super) const DEFAULT_UPDATE_FEED_URL: &str = default_update_feed_url_from(
    option_env!("STUDIO_DEFAULT_UPDATE_FEED_URL"),
    PLATFORM_UPDATE_FEED_URL,
);
const UPDATE_FEED_URL_ENV: &str = "STUDIO_UPDATE_FEED_URL";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdatePhase {
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
pub(super) struct UpdateCheckSummary {
    pub(super) available_version: Option<String>,
    pub(super) remote_is_empty: bool,
}

impl UpdateCheckSummary {
    pub(super) fn available(version: impl Into<String>) -> Self {
        Self {
            available_version: Some(version.into()),
            remote_is_empty: false,
        }
    }

    pub(super) fn up_to_date() -> Self {
        Self {
            available_version: None,
            remote_is_empty: false,
        }
    }

    fn remote_empty() -> Self {
        Self {
            available_version: None,
            remote_is_empty: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UpdatePanelState {
    pub(super) feed_url: String,
    pub(super) current_version: String,
    pub(super) build_commit: String,
    pub(super) phase: UpdatePhase,
    pub(super) available_version: Option<String>,
    last_error: Option<String>,
}

impl Default for UpdatePanelState {
    fn default() -> Self {
        Self {
            feed_url: configured_update_feed_url(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            build_commit: option_env!("STUDIO_BUILD_GIT_COMMIT")
                .unwrap_or("unknown")
                .to_string(),
            phase: UpdatePhase::Idle,
            available_version: None,
            last_error: None,
        }
    }
}

impl UpdatePanelState {
    pub(super) fn can_check(&self) -> bool {
        !matches!(
            self.phase,
            UpdatePhase::Checking | UpdatePhase::Installing | UpdatePhase::Restarting
        )
    }

    pub(super) fn can_install(&self) -> bool {
        self.available_version.is_some()
            && !matches!(
                self.phase,
                UpdatePhase::Checking | UpdatePhase::Installing | UpdatePhase::Restarting
            )
    }

    pub(super) fn is_error(&self) -> bool {
        matches!(self.phase, UpdatePhase::Failed)
    }

    pub(super) fn status_label(&self) -> String {
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

    pub(super) fn begin_check(&mut self) {
        if !self.can_check() {
            return;
        }
        self.phase = UpdatePhase::Checking;
        self.last_error = None;
    }

    pub(super) fn finish_check(&mut self, result: Result<UpdateCheckSummary, String>) {
        match result {
            Ok(summary) if summary.remote_is_empty => {
                self.phase = UpdatePhase::RemoteEmpty;
                self.available_version = None;
                self.last_error = None;
            }
            Ok(summary) => {
                self.available_version = summary.available_version;
                self.phase = if self.available_version.is_some() {
                    UpdatePhase::UpdateAvailable
                } else {
                    UpdatePhase::UpToDate
                };
                self.last_error = None;
            }
            Err(error) => {
                self.phase = UpdatePhase::Failed;
                self.last_error = Some(error);
            }
        }
    }

    pub(super) fn begin_install(&mut self) {
        if !self.can_install() {
            return;
        }
        self.phase = UpdatePhase::Installing;
        self.last_error = None;
    }

    pub(super) fn finish_install(&mut self, result: Result<(), String>) {
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

pub(super) async fn check_for_updates(feed_url: String) -> Result<UpdateCheckSummary, String> {
    tokio::task::spawn_blocking(move || check_for_updates_blocking(&feed_url))
        .await
        .map_err(|error| format!("updater task failed: {error}"))?
}

pub(super) async fn install_available_update(feed_url: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || install_available_update_blocking(&feed_url))
        .await
        .map_err(|error| format!("updater task failed: {error}"))?
}

fn configured_update_feed_url() -> String {
    std::env::var(UPDATE_FEED_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_UPDATE_FEED_URL.to_string())
}

#[cfg(test)]
fn default_update_feed_url_for_os(
    os: &str,
    arch: &str,
    build_override: Option<&'static str>,
) -> &'static str {
    let platform_url = match os {
        "macos" => match arch {
            "aarch64" | "arm64" => "https://reviewgen.ovh/transcription-studio/alpha/mac-arm64",
            "x86_64" => "https://reviewgen.ovh/transcription-studio/alpha/mac-x64",
            _ => "https://reviewgen.ovh/transcription-studio/alpha/mac-arm64",
        },
        _ => "https://reviewgen.ovh/transcription-studio/alpha/win-x64",
    };
    default_update_feed_url_from(build_override, platform_url)
}

const fn default_update_feed_url_from(
    build_override: Option<&'static str>,
    platform_url: &'static str,
) -> &'static str {
    match build_override {
        Some(url) => url,
        None => platform_url,
    }
}

fn check_for_updates_blocking(feed_url: &str) -> Result<UpdateCheckSummary, String> {
    let update_manager = update_manager(feed_url)?;
    match update_manager
        .check_for_updates()
        .map_err(|error| format!("update check failed: {error}"))?
    {
        UpdateCheck::RemoteIsEmpty => Ok(UpdateCheckSummary::remote_empty()),
        UpdateCheck::NoUpdateAvailable => Ok(UpdateCheckSummary::up_to_date()),
        UpdateCheck::UpdateAvailable(update) => Ok(UpdateCheckSummary::available(
            update.TargetFullRelease.Version,
        )),
    }
}

fn install_available_update_blocking(feed_url: &str) -> Result<(), String> {
    let update_manager = update_manager(feed_url)?;
    let update = match update_manager
        .check_for_updates()
        .map_err(|error| format!("update check failed: {error}"))?
    {
        UpdateCheck::UpdateAvailable(update) => *update,
        UpdateCheck::NoUpdateAvailable => return Err("no update is available".to_string()),
        UpdateCheck::RemoteIsEmpty => return Err("the update feed is empty".to_string()),
    };
    update_manager
        .download_updates(&update, None)
        .map_err(|error| format!("update download failed: {error}"))?;
    update_manager
        .apply_updates_and_restart(&update)
        .map_err(|error| format!("update restart failed: {error}"))
}

fn update_manager(feed_url: &str) -> Result<UpdateManager, String> {
    let source = sources::HttpSource::new(feed_url);
    UpdateManager::new(source, None, None).map_err(|error| {
        format!("Velopack updater is available only from an installed build: {error}")
    })
}

#[cfg(test)]
mod tests {
    use super::default_update_feed_url_for_os;

    #[test]
    fn default_update_feed_url_matches_packaged_runtime_family() {
        assert_eq!(
            default_update_feed_url_for_os("windows", "x86_64", None),
            "https://reviewgen.ovh/transcription-studio/alpha/win-x64"
        );
        assert_eq!(
            default_update_feed_url_for_os("macos", "aarch64", None),
            "https://reviewgen.ovh/transcription-studio/alpha/mac-arm64"
        );
        assert_eq!(
            default_update_feed_url_for_os("macos", "x86_64", None),
            "https://reviewgen.ovh/transcription-studio/alpha/mac-x64"
        );
        assert_eq!(
            default_update_feed_url_for_os(
                "macos",
                "x86_64",
                Some("https://reviewgen.ovh/transcription-studio/alpha/mac-sonoma-x64")
            ),
            "https://reviewgen.ovh/transcription-studio/alpha/mac-sonoma-x64"
        );
    }
}
