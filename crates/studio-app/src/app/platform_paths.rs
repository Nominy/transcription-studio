use std::path::PathBuf;

const DEFAULT_APP_SUPPORT_DIR_NAME: &str = "Transcription Studio";

pub(super) fn application_support_dir() -> PathBuf {
    application_support_dir_named(DEFAULT_APP_SUPPORT_DIR_NAME)
}

pub(super) fn application_support_dir_named(app_dir_name: &str) -> PathBuf {
    application_support_dir_named_for_env(
        app_dir_name,
        std::env::consts::OS,
        std::env::var_os("APPDATA").and_then(|value| value.into_string().ok()),
        std::env::var_os("XDG_CONFIG_HOME").and_then(|value| value.into_string().ok()),
        std::env::var_os("HOME").and_then(|value| value.into_string().ok()),
    )
}

#[cfg(test)]
fn application_support_dir_for_env(
    os: &str,
    appdata: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> PathBuf {
    application_support_dir_named_for_env(
        DEFAULT_APP_SUPPORT_DIR_NAME,
        os,
        appdata.map(str::to_string),
        xdg_config_home.map(str::to_string),
        home.map(str::to_string),
    )
}

fn application_support_dir_named_for_env(
    app_dir_name: &str,
    os: &str,
    appdata: Option<String>,
    xdg_config_home: Option<String>,
    home: Option<String>,
) -> PathBuf {
    if let Some(path) = non_empty_path(appdata.as_deref()) {
        return path.join(app_dir_name);
    }

    if os == "macos"
        && let Some(home) = non_empty_path(home.as_deref())
    {
        return home
            .join("Library")
            .join("Application Support")
            .join(app_dir_name);
    }

    if let Some(path) = non_empty_path(xdg_config_home.as_deref()) {
        return path.join(app_dir_name);
    }

    non_empty_path(home.as_deref())
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

    use super::{application_support_dir_for_env, application_support_dir_named_for_env};

    #[test]
    fn application_support_dir_uses_windows_appdata_when_present() {
        assert_eq!(
            application_support_dir_for_env(
                "windows",
                Some(r"C:\Users\me\AppData\Roaming"),
                None,
                None
            ),
            PathBuf::from(r"C:\Users\me\AppData\Roaming").join("Transcription Studio")
        );
    }

    #[test]
    fn application_support_dir_uses_macos_application_support() {
        assert_eq!(
            application_support_dir_for_env("macos", None, None, Some("/Users/me")),
            PathBuf::from("/Users/me")
                .join("Library")
                .join("Application Support")
                .join("Transcription Studio")
        );
    }

    #[test]
    fn application_support_dir_uses_xdg_config_home_on_linux() {
        assert_eq!(
            application_support_dir_for_env(
                "linux",
                None,
                Some("/home/me/.config"),
                Some("/home/me")
            ),
            PathBuf::from("/home/me/.config").join("Transcription Studio")
        );
    }

    #[test]
    fn application_support_dir_falls_back_to_home_config_outside_macos() {
        assert_eq!(
            application_support_dir_for_env("linux", None, None, Some("/home/me")),
            PathBuf::from("/home/me")
                .join(".config")
                .join("Transcription Studio")
        );
    }

    #[test]
    fn application_support_dir_preserves_custom_app_folder_names() {
        assert_eq!(
            application_support_dir_named_for_env(
                "TranscriptionStudio",
                "macos",
                None,
                None,
                Some("/Users/me".to_string())
            ),
            PathBuf::from("/Users/me")
                .join("Library")
                .join("Application Support")
                .join("TranscriptionStudio")
        );
    }
}
