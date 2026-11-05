//! Which language's Gold lint/tag configuration a project uses.
//!
//! Studio lints every project with Babel's gold-non-bg configuration (see `linter::gold`); a
//! project only selects the language. The language comes from the metadata Studio records when
//! it opens a task, else from a known Babel project/queue identifier
//! (`assets/project-languages.json`), else from a language code in the task metadata, else Russian.

use std::sync::LazyLock;

use serde::Deserialize;
use serde_json::Value;
use studio_core::Project;

use crate::linter::gold::{GoldLanguage, gold_language};

pub const META_BABEL_PROJECT_ID: &str = "studio.babel.projectId";
pub const META_BABEL_PROJECT_SLUG: &str = "studio.babel.projectSlug";
pub const META_BABEL_LINT_TAG_LANGUAGE: &str = "studio.babel.lintTagLanguage";

const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_ACTIVE_QUEUE_ID: &str = "studio.babel.activeQueueId";
const META_QUEUE_ID: &str = "studio.babel.queueId";
const META_CHUNKED_QUEUE_ID: &str = "studio.babel.chunkedQueueId";
const META_STITCHED_QUEUE_ID: &str = "studio.babel.stitchedQueueId";
const DEFAULT_LANGUAGE: &str = "RU";

const IDENTIFIER_METADATA_KEYS: &[&str] = &[
    META_BABEL_PROJECT_ID,
    META_BABEL_PROJECT_SLUG,
    "projectId",
    "project_id",
    "project",
    "projectSlug",
    "project_slug",
    "projectName",
    "project_name",
    META_ACTIVE_QUEUE_ID,
    META_QUEUE_ID,
    META_CHUNKED_QUEUE_ID,
    META_STITCHED_QUEUE_ID,
];
const EXTRA_PACKAGE_IDENTIFIER_KEYS: &[&str] = &[
    META_BABEL_PROJECT_ID,
    META_BABEL_PROJECT_SLUG,
    "projectId",
    "project_id",
    "project",
    "projectSlug",
    "project_slug",
    "projectName",
    "project_name",
    "projectType",
    "project_type",
    "slug",
    "name",
];
const LANGUAGE_METADATA_KEYS: &[&str] = &["language", "languageCode", "studio.babel.language"];
const EXTRA_PACKAGE_LANGUAGE_KEYS: &[&str] = &["language", "languageCode"];

const PROJECT_LANGUAGES_JSON: &str = include_str!("../assets/project-languages.json");

/// Known Babel project and queue identifiers, by language.
#[derive(Deserialize)]
struct ProjectLanguage {
    #[serde(borrow)]
    language: &'static str,
    #[serde(borrow)]
    identifiers: Vec<&'static str>,
}

static PROJECT_LANGUAGES: LazyLock<Vec<ProjectLanguage>> = LazyLock::new(|| {
    serde_json::from_str(PROJECT_LANGUAGES_JSON).expect("embedded project-languages.json is valid")
});

/// `{KEYWORD: word}` completions per language: the language's MIS and PRO keywords, then the
/// universal `MIS`/`PRO`, as Babel accepts them (module 78100 `cn`).
static CURLY_TAGS: LazyLock<Vec<(&'static str, Vec<String>)>> = LazyLock::new(|| {
    PROJECT_LANGUAGES
        .iter()
        .filter_map(|entry| gold_language(entry.language))
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
        .or_else(|| identified_language(project))
        .or_else(|| metadata_language(project))
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

fn identified_language(project: &Project) -> Option<&'static GoldLanguage> {
    let from_project = [project.id.as_str(), project.name.as_str()]
        .into_iter()
        .chain(
            IDENTIFIER_METADATA_KEYS
                .iter()
                .filter_map(|key| project.metadata.get(*key).map(String::as_str)),
        )
        .find_map(language_for_identifier);
    from_project.or_else(|| {
        first_string_by_keys(&extra_package(project)?, EXTRA_PACKAGE_IDENTIFIER_KEYS)
            .and_then(|value| language_for_identifier(&value))
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

fn language_for_identifier(value: &str) -> Option<&'static GoldLanguage> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    PROJECT_LANGUAGES
        .iter()
        .find(|entry| {
            entry
                .identifiers
                .iter()
                .any(|identifier| value.eq_ignore_ascii_case(identifier))
        })
        .and_then(|entry| gold_language(entry.language))
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
        META_ACTIVE_QUEUE_ID, META_BABEL_LINT_TAG_LANGUAGE, META_BABEL_PROJECT_ID,
        PROJECT_LANGUAGES, babel_project_lint_tag_metadata, gold_language,
        resolve_babel_project_lint_tag_config,
    };

    #[test]
    fn every_known_project_identifier_resolves_to_its_language() {
        for entry in PROJECT_LANGUAGES.iter() {
            assert!(
                gold_language(entry.language).is_some(),
                "no Gold lint data for {}",
                entry.language
            );
            for identifier in &entry.identifiers {
                let project = project_with_metadata(&[(META_BABEL_PROJECT_ID, identifier)]);
                assert_eq!(
                    resolve_babel_project_lint_tag_config(&project).language(),
                    entry.language,
                    "{identifier}"
                );
            }
        }
    }

    #[test]
    fn russian_projects_of_every_variant_lint_with_gold_non_background_tags() {
        // ru-tx-gold (with its background tags), its queue, and ru-tx-silver.
        for (key, value) in [
            (
                META_BABEL_PROJECT_ID,
                "0bf29f0c-371a-506a-8b77-5fffd14e5762",
            ),
            (META_ACTIVE_QUEUE_ID, "f95babcc-8681-521c-a39f-66454a16347f"),
            (
                META_BABEL_PROJECT_ID,
                "73f8e130-9fb4-5c17-b142-5a2e75d89b6d",
            ),
        ] {
            let config =
                resolve_babel_project_lint_tag_config(&project_with_metadata(&[(key, value)]));

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
            (
                META_BABEL_PROJECT_ID,
                "0bf29f0c-371a-506a-8b77-5fffd14e5762",
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
