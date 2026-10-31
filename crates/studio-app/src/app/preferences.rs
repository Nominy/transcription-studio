//! User preferences, mirroring Babel Helper's settings (`core/settings.ts`,
//! `features/registry.ts`). One JSON record stored under
//! [`PREFERENCES_STORAGE_KEY`] in the existing UI storage file.
//!
//! Every field carries a Helper-matching default and `#[serde(default)]`, so a
//! record written by an older build (missing fields) or a newer one (unknown
//! fields) still loads; nothing is migrated in place.

use serde::{Deserialize, Serialize};

pub(super) const PREFERENCES_STORAGE_KEY: &str = "studio.preferences.v1";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Preferences {
    /// Transcript layout. Helper "Editor → Layout"; Babel's turn cards are the default.
    pub(crate) editor_layout: EditorLayout,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            editor_layout: EditorLayout::Babel,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum EditorLayout {
    /// Babel's current editor: stacked turn cards.
    #[default]
    Babel,
    /// The older spreadsheet table (ID / Speaker / Start / End / Text / Linter / Actions).
    Classic,
    /// Two speaker panes, one above the other.
    SplitStacked,
    /// Two speaker panes, side by side.
    SplitSideBySide,
}

impl Preferences {
    pub(super) fn from_storage(raw: Option<&str>) -> Self {
        raw.and_then(|source| serde_json::from_str(source).ok())
            .unwrap_or_default()
    }

    pub(super) fn to_storage(&self) -> String {
        serde_json::to_string(self).expect("preferences always serialize")
    }
}
