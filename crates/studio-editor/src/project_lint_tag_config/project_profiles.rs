//! Babel project lint/tag profiles, loaded from `assets/project-profiles.json`.

use std::sync::LazyLock;

use serde::Deserialize;

const PROFILES_JSON: &str = include_str!("../../assets/project-profiles.json");

#[derive(Clone, Copy, Debug)]
pub(crate) struct ProjectProfile {
    pub(crate) key: &'static str,
    pub(crate) identifiers: &'static [&'static str],
    pub(crate) language: &'static str,
    pub(crate) enabled_linter_ids: &'static [&'static str],
    pub(crate) bracket_tags: &'static [&'static str],
    pub(crate) style_tags: &'static [&'static str],
    pub(crate) curly_tags: &'static [&'static str],
}

#[derive(Deserialize)]
struct RawProfile {
    #[serde(borrow)]
    key: &'static str,
    #[serde(borrow)]
    identifiers: Vec<&'static str>,
    #[serde(borrow)]
    language: &'static str,
    #[serde(borrow)]
    enabled_linter_ids: Vec<&'static str>,
    #[serde(borrow)]
    bracket_tags: Vec<&'static str>,
    #[serde(borrow)]
    style_tags: Vec<&'static str>,
    #[serde(borrow)]
    curly_tags: Vec<&'static str>,
}

/// All known project profiles. Parsed once; strings borrow from the embedded JSON.
pub(crate) static PROJECT_PROFILES: LazyLock<Vec<ProjectProfile>> = LazyLock::new(|| {
    let raw: Vec<RawProfile> =
        serde_json::from_str(PROFILES_JSON).expect("embedded project-profiles.json is valid");
    raw.into_iter()
        .map(|p| ProjectProfile {
            key: p.key,
            identifiers: p.identifiers.leak(),
            language: p.language,
            enabled_linter_ids: p.enabled_linter_ids.leak(),
            bracket_tags: p.bracket_tags.leak(),
            style_tags: p.style_tags.leak(),
            curly_tags: p.curly_tags.leak(),
        })
        .collect()
});
