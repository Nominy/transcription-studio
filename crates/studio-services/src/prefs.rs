//! Persistent UI settings: one flat string key/value JSON file
//! (`<app support>/ui-local-storage.json`) holding the preferences record,
//! hotkey bindings and context-panel collapse flags.

use std::{
    collections::BTreeMap,
    fs,
    io::{self, Error, ErrorKind},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::paths::app_support_dir;

/// Key of the serialized [`Preferences`] record.
pub const PREFERENCES_KEY: &str = "studio.preferences.v1";
/// Key of the serialized hotkey bindings (format owned by the UI's keybind model).
pub const HOTKEY_BINDINGS_KEY: &str = "studio.native.hotkeyBindings.v1";

const STORAGE_FILE_NAME: &str = "ui-local-storage.json";

/// String key/value store persisted as pretty JSON; every change is written through.
#[derive(Debug, Clone)]
pub struct UiStorage {
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl UiStorage {
    /// Loads the default file; a missing or unreadable file yields an empty store at that path.
    pub fn load_default() -> Self {
        let path = app_support_dir().join(STORAGE_FILE_NAME);
        Self::load(&path).unwrap_or_else(|_| Self {
            path,
            values: BTreeMap::new(),
        })
    }

    /// Loads `path`; a missing file is an empty store, malformed JSON is `InvalidData`.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let values = match fs::read_to_string(&path) {
            Ok(source) => serde_json::from_str(&source).map_err(|source| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("failed to decode UI storage: {source}"),
                )
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, values })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    /// Stores `value` and writes the file if it changed.
    pub fn set_value(&mut self, key: &str, value: String) -> io::Result<()> {
        if self.values.get(key) == Some(&value) {
            return Ok(());
        }
        self.values.insert(key.to_string(), value);
        self.save()
    }

    /// Collapsed only when the stored value is exactly `"true"`.
    pub fn is_collapsed(&self, key: &str) -> bool {
        self.value(key) == Some("true")
    }

    pub fn set_collapsed(&mut self, key: &str, collapsed: bool) -> io::Result<()> {
        self.set_value(key, collapsed.to_string())
    }

    /// The stored preferences, or defaults when missing/undecodable.
    pub fn preferences(&self) -> Preferences {
        Preferences::from_storage(self.value(PREFERENCES_KEY))
    }

    pub fn set_preferences(&mut self, preferences: &Preferences) -> io::Result<()> {
        self.set_value(PREFERENCES_KEY, preferences.to_storage())
    }

    fn save(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let source = serde_json::to_string_pretty(&self.values).map_err(|source| {
            Error::new(
                ErrorKind::InvalidData,
                format!("failed to encode UI storage: {source}"),
            )
        })?;
        fs::write(&self.path, source)
    }
}

/// User interface preferences. Defaults and `#[serde(default)]` keep old records readable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    /// Transcript layout ("Editor → Layout").
    pub editor_layout: EditorLayout,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorLayout {
    /// Waveform card above one transcript table with every speaker.
    #[default]
    Classic,
    /// Waveform card above per-speaker transcript panes, one above the other.
    SplitStacked,
    /// Waveform card above per-speaker transcript panes, side by side.
    SplitSideBySide,
}

impl Preferences {
    pub fn from_storage(raw: Option<&str>) -> Self {
        raw.and_then(|source| serde_json::from_str(source).ok())
            .unwrap_or_default()
    }

    pub fn to_storage(&self) -> String {
        serde_json::to_string(self).expect("preferences always serialize")
    }
}
