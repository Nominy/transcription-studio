//! Which language's Gold lint/tag configuration a project uses.
//!
//! Explicitly registered Babel providers use the gold-non-bg configuration (see `linter::gold`).
//! The language comes from the metadata recorded when opening a Babel task, else from a
//! language code in the task metadata, else from the language code a Babel project slug
//! starts with (`RU-tx-gold`, `ar_tx_silver`), else Russian. Local documents install no provider.

use std::sync::LazyLock;

use serde_json::Value;
use studio_core::Project;

use crate::linter::gold::{GoldLanguage, gold_language, gold_languages};

pub const META_BABEL_PROJECT_ID: &str = "studio.babel.projectId";
pub const META_BABEL_PROJECT_SLUG: &str = "studio.babel.projectSlug";
pub const META_BABEL_LINT_TAG_LANGUAGE: &str = "studio.babel.lintTagLanguage";

const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const DEFAULT_LANGUAGE: &str = "RU";

const SLUG_METADATA_KEYS: &[&str] = &[
    META_BABEL_PROJECT_SLUG,
    "projectSlug",
    "project_slug",
    "projectName",
    "project_name",
    "project",
];
const EXTRA_PACKAGE_SLUG_KEYS: &[&str] = &[
    META_BABEL_PROJECT_SLUG,
    "projectSlug",
    "project_slug",
    "projectName",
    "project_name",
    "project",
    "projectType",
    "project_type",
    "slug",
    "name",
];
const LANGUAGE_METADATA_KEYS: &[&str] = &["language", "languageCode", "studio.babel.language"];
const EXTRA_PACKAGE_LANGUAGE_KEYS: &[&str] = &["language", "languageCode"];

/// `{KEYWORD: word}` completions per language: the language's MIS and PRO keywords, then the
/// universal `MIS`/`PRO`, as Babel accepts them (module 78100 `cn`).
static CURLY_TAGS: LazyLock<Vec<(&'static str, Vec<String>)>> = LazyLock::new(|| {
    gold_languages()
        .iter()
        .map(|language| {
            let mut tags: Vec<String> = Vec::new();
            for keyword in [language.mis_keyword, language.pro_keyword, "MIS", "PRO"] {
                let tag = format!("{keyword}: ");
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            (language.language, tags)
        })
        .collect()
});

/// A project's Gold lint and tag configuration.
#[derive(Clone, Copy)]
pub struct BabelProjectLintTagConfig {
    gold: &'static GoldLanguage,
}

impl std::fmt::Debug for BabelProjectLintTagConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BabelProjectLintTagConfig")
            .field("language", &self.gold.language)
            .finish()
    }
}

impl BabelProjectLintTagConfig {
    pub fn language(&self) -> &'static str {
        self.gold.language
    }

    pub fn bracket_tags(&self) -> &'static [&'static str] {
        &self.gold.bracket_tags
    }

    pub fn style_tags(&self) -> &'static [&'static str] {
        &self.gold.style_tags
    }

    pub fn curly_tags(&self) -> &'static [String] {
        CURLY_TAGS
            .iter()
            .find(|(language, _)| *language == self.gold.language)
            .map(|(_, tags)| tags.as_slice())
            .unwrap_or_default()
    }

    pub(crate) fn gold(&self) -> &'static GoldLanguage {
        self.gold
    }
}

impl Default for BabelProjectLintTagConfig {
    fn default() -> Self {
        Self {
            gold: gold_language(DEFAULT_LANGUAGE)
                .expect("Gold lint data covers the default language"),
        }
    }
}

pub fn resolve_babel_project_lint_tag_config(project: &Project) -> BabelProjectLintTagConfig {
    recorded_language(project)
        .or_else(|| metadata_language(project))
        .or_else(|| slug_language(project))
        .map_or_else(BabelProjectLintTagConfig::default, |gold| {
            BabelProjectLintTagConfig { gold }
        })
}

/// Metadata Studio records on a task it opens, so the project keeps its language.
pub fn babel_project_lint_tag_metadata(project: &Project) -> Vec<(String, String)> {
    let config = resolve_babel_project_lint_tag_config(project);
    vec![(
        META_BABEL_LINT_TAG_LANGUAGE.to_string(),
        config.language().to_string(),
    )]
}

fn recorded_language(project: &Project) -> Option<&'static GoldLanguage> {
    project
        .metadata
        .get(META_BABEL_LINT_TAG_LANGUAGE)
        .and_then(|value| gold_language(value.trim()))
}

fn slug_language(project: &Project) -> Option<&'static GoldLanguage> {
    SLUG_METADATA_KEYS
        .iter()
        .filter_map(|key| project.metadata.get(*key))
        .find_map(|value| language_for_slug(value))
        .or_else(|| {
            first_string_by_keys(&extra_package(project)?, EXTRA_PACKAGE_SLUG_KEYS)
                .and_then(|value| language_for_slug(&value))
        })
}

fn metadata_language(project: &Project) -> Option<&'static GoldLanguage> {
    LANGUAGE_METADATA_KEYS
        .iter()
        .filter_map(|key| project.metadata.get(*key))
        .find_map(|value| gold_language(value.trim()))
        .or_else(|| {
            first_string_by_keys(&extra_package(project)?, EXTRA_PACKAGE_LANGUAGE_KEYS)
                .and_then(|value| gold_language(&value))
        })
}

/// The language of a Babel project slug: its leading two-letter language code, as in
/// `RU-tx-gold` or `AR_TX_SILVER`.
fn language_for_slug(value: &str) -> Option<&'static GoldLanguage> {
    let (code, rest) = value.trim().split_once(['-', '_'])?;
    if code.len() != 2 || !code.chars().all(|c| c.is_ascii_alphabetic()) || rest.is_empty() {
        return None;
    }
    gold_language(code)
}

fn extra_package(project: &Project) -> Option<Value> {
    serde_json::from_str(project.metadata.get(META_EXTRA_PACKAGE)?).ok()
}

fn first_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => keys
            .iter()
            .find_map(|key| {
                map.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                map.values()
                    .find_map(|child| first_string_by_keys(child, keys))
            }),
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_string_by_keys(child, keys)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use studio_core::Project;

    use super::{
        META_BABEL_LINT_TAG_LANGUAGE, META_BABEL_PROJECT_ID, META_BABEL_PROJECT_SLUG,
        babel_project_lint_tag_metadata, language_for_slug, resolve_babel_project_lint_tag_config,
    };

    #[test]
    fn project_slugs_resolve_to_their_language_code() {
        for (slug, language) in [
            ("RU-tx-gold", "RU"),
            ("ru-tx-silver-dg", "RU"),
            ("AR_TX_GOLD_NON_BG", "AR"),
            ("de-emotion-tx", "DE"),
        ] {
            let project = project_with_metadata(&[(META_BABEL_PROJECT_SLUG, slug)]);
            assert_eq!(
                resolve_babel_project_lint_tag_config(&project).language(),
                language,
                "{slug}"
            );
        }
        for not_a_slug in [
            "Russian-Podcast",
            "ru",
            "ru-",
            "xx-tx-gold",
            "a1b2c3d4-0000-4000",
        ] {
            assert!(language_for_slug(not_a_slug).is_none(), "{not_a_slug}");
        }
        let english = project_with_metadata(&[("projectName", "EN-tx-gold")]);
        assert_eq!(
            resolve_babel_project_lint_tag_config(&english).language(),
            "EN"
        );
    }

    #[test]
    fn russian_projects_lint_with_gold_non_background_tags() {
        for slug in ["RU-tx-gold", "ru-tx-silver"] {
            let config = resolve_babel_project_lint_tag_config(&project_with_metadata(&[(
                META_BABEL_PROJECT_SLUG,
                slug,
            )]));

            assert_eq!(config.language(), "RU");
            assert!(config.bracket_tags().contains(&"другой-звук"));
            assert!(!config.bracket_tags().contains(&"музыка"));
            assert!(config.style_tags().contains(&"иностранная-речь"));
            assert_eq!(config.curly_tags(), ["ИСКАЖ: ", "СКАЗ: ", "MIS: ", "PRO: "]);
        }
    }

    #[test]
    fn recorded_language_wins_and_unknown_projects_default_to_russian() {
        let recorded = project_with_metadata(&[
            (META_BABEL_LINT_TAG_LANGUAGE, "EN"),
            (META_BABEL_PROJECT_SLUG, "RU-tx-gold"),
            (
                META_BABEL_PROJECT_ID,
                "00000000-0000-4000-8000-000000000001",
            ),
        ]);
        assert_eq!(
            resolve_babel_project_lint_tag_config(&recorded).language(),
            "EN"
        );

        let by_language = project_with_metadata(&[("language", "de")]);
        assert_eq!(
            resolve_babel_project_lint_tag_config(&by_language).language(),
            "DE"
        );

        let unknown = project_with_metadata(&[]);
        assert_eq!(
            resolve_babel_project_lint_tag_config(&unknown).language(),
            "RU"
        );
        assert_eq!(
            babel_project_lint_tag_metadata(&unknown),
            vec![(META_BABEL_LINT_TAG_LANGUAGE.to_string(), "RU".to_string())]
        );
    }

    fn project_with_metadata(metadata: &[(&str, &str)]) -> Project {
        Project {
            id: "task".to_string(),
            name: "Task".to_string(),
            speakers: Vec::new(),
            lanes: Vec::new(),
            segments: Vec::new(),
            metadata: metadata
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect(),
        }
    }
}
