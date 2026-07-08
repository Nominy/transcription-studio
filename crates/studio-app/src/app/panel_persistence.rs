use std::collections::BTreeMap;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::{fs, io::Error};

use studio_app::{TtsPanelEffect, read_stored_collapsed_state};

use super::platform_paths::application_support_dir;

#[derive(Debug, Clone)]
pub(super) struct PanelCollapseStorage {
    path: PathBuf,
    values: BTreeMap<String, String>,
}

impl PanelCollapseStorage {
    pub(super) fn load_default() -> Self {
        let path = Self::default_path();
        Self::load(&path).unwrap_or_else(|_| Self::empty_at(path))
    }

    pub(super) fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let values = match fs::read_to_string(&path) {
            Ok(source) => serde_json::from_str(&source).map_err(|source| {
                Error::new(
                    ErrorKind::InvalidData,
                    format!("failed to decode panel collapse storage: {source}"),
                )
            })?,
            Err(error) if error.kind() == ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };

        Ok(Self { path, values })
    }

    fn empty_at(path: PathBuf) -> Self {
        Self {
            path,
            values: BTreeMap::new(),
        }
    }

    fn default_path() -> PathBuf {
        application_support_dir().join("ui-local-storage.json")
    }

    pub(super) fn is_collapsed(&self, key: &str) -> bool {
        read_stored_collapsed_state(self.values.get(key).map(String::as_str))
    }

    pub(super) fn value(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub(super) fn set_value(&mut self, key: &str, value: String) -> io::Result<()> {
        let changed = self.values.insert(key.to_string(), value.clone()).as_ref() != Some(&value);
        if changed {
            self.save()?;
        }
        Ok(())
    }

    pub(super) fn apply_tts_panel_effects(&mut self, effects: &[TtsPanelEffect]) -> io::Result<()> {
        let mut changed = false;
        for effect in effects {
            if let TtsPanelEffect::LocalStorageSet { key, value } = effect {
                changed |= self.values.insert(key.clone(), value.clone()).as_ref() != Some(value);
            }
        }

        if changed {
            self.save()?;
        }

        Ok(())
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
                format!("failed to encode panel collapse storage: {source}"),
            )
        })?;
        fs::write(&self.path, source)
    }
}
