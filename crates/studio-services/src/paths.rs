//! Platform application-support directory resolution.

use std::path::PathBuf;

const DEFAULT_APP_SUPPORT_DIR_NAME: &str = "Exaudio";

/// `%APPDATA%/Exaudio`, `~/Library/Application Support/Exaudio`
/// on macOS, or `$XDG_CONFIG_HOME|~/.config/Exaudio` elsewhere.
pub fn app_support_dir() -> PathBuf {
    app_support_dir_named(DEFAULT_APP_SUPPORT_DIR_NAME)
}

/// Same resolution as [`app_support_dir`] with a custom folder name.
pub fn app_support_dir_named(app_dir_name: &str) -> PathBuf {
    app_support_dir_for_env(
        app_dir_name,
        std::env::consts::OS,
        std::env::var("APPDATA").ok().as_deref(),
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

fn app_support_dir_for_env(
    app_dir_name: &str,
    os: &str,
    appdata: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> PathBuf {
    if let Some(path) = non_empty_path(appdata) {
        return path.join(app_dir_name);
    }

    if os == "macos"
        && let Some(home) = non_empty_path(home)
    {
        return home
            .join("Library")
            .join("Application Support")
            .join(app_dir_name);
    }

    if let Some(path) = non_empty_path(xdg_config_home) {
        return path.join(app_dir_name);
    }

    non_empty_path(home)
        .map(|home| home.join(".config"))
        .unwrap_or_else(|| PathBuf::from(".studio-dev"))
        .join(app_dir_name)
}

fn non_empty_path(value: Option<&str>) -> Option<PathBuf> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{DEFAULT_APP_SUPPORT_DIR_NAME as NAME, app_support_dir_for_env};

    #[test]
    fn uses_windows_appdata_when_present() {
        assert_eq!(
            app_support_dir_for_env(
                NAME,
                "windows",
                Some(r"C:\Users\me\AppData\Roaming"),
                None,
                None
            ),
            PathBuf::from(r"C:\Users\me\AppData\Roaming").join("Exaudio")
        );
    }

    #[test]
    fn uses_macos_application_support() {
        assert_eq!(
            app_support_dir_for_env(NAME, "macos", None, None, Some("/Users/me")),
            PathBuf::from("/Users/me")
                .join("Library")
                .join("Application Support")
                .join("Exaudio")
        );
    }

    #[test]
    fn uses_xdg_config_home_on_linux() {
        assert_eq!(
            app_support_dir_for_env(
                NAME,
                "linux",
                None,
                Some("/home/me/.config"),
                Some("/home/me")
            ),
            PathBuf::from("/home/me/.config").join("Exaudio")
        );
    }

    #[test]
    fn falls_back_to_home_config_outside_macos() {
        assert_eq!(
            app_support_dir_for_env(NAME, "linux", None, None, Some("/home/me")),
            PathBuf::from("/home/me").join(".config").join("Exaudio")
        );
    }

    #[test]
    fn preserves_custom_app_folder_names() {
        assert_eq!(
            app_support_dir_for_env(
                "TranscriptionStudio",
                "macos",
                None,
                None,
                Some("/Users/me")
            ),
            PathBuf::from("/Users/me")
                .join("Library")
                .join("Application Support")
                .join("TranscriptionStudio")
        );
    }
}
