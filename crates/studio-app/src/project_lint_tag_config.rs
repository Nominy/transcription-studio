use serde_json::Value;
use studio_core::Project;

mod recovered_project_profiles;

use recovered_project_profiles::{RECOVERED_PROJECT_PROFILES, RecoveredProjectProfile};

pub const META_BABEL_PROJECT_ID: &str = "studio.babel.projectId";
pub const META_BABEL_PROJECT_SLUG: &str = "studio.babel.projectSlug";
pub const META_BABEL_LINT_TAG_PROFILE: &str = "studio.babel.lintTagProfile";
pub const META_BABEL_LINT_TAG_LANGUAGE: &str = "studio.babel.lintTagLanguage";
pub const META_BABEL_ENABLED_LINTER_IDS: &str = "studio.babel.linter.enabledRuleIds";
pub const META_BABEL_BRACKET_TAGS: &str = "studio.babel.tags.bracket";
pub const META_BABEL_STYLE_TAGS: &str = "studio.babel.tags.style";

const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_ACTIVE_QUEUE_ID: &str = "studio.babel.activeQueueId";
const META_QUEUE_ID: &str = "studio.babel.queueId";
const META_CHUNKED_QUEUE_ID: &str = "studio.babel.chunkedQueueId";
const META_STITCHED_QUEUE_ID: &str = "studio.babel.stitchedQueueId";

pub const RUSSIAN_TRANSCRIPTION_BRACKET_TAGS: &[&str] = &[
    "вдох",
    "вздох",
    "вибрация-губами",
    "втягивание-воздуха-сквозь-зубы",
    "выдох",
    "выдувание-воздуха",
    "глотание",
    "дыхание",
    "зевок",
    "икота",
    "кашель",
    "кряхтенье",
    "напевание",
    "насмешка",
    "неразборчиво",
    "плач",
    "причмокивание",
    "прочищение-горла",
    "радостный-возглас",
    "резкий-вздох",
    "свист",
    "смех",
    "смешок",
    "стон",
    "фырканье",
    "хихиканье",
    "цоканье-языком",
    "чихание",
    "шиканье",
    "шмыганье-носом",
    "щелчок-языком",
];

pub const RUSSIAN_TRANSCRIPTION_STYLE_TAGS: &[&str] =
    &["протяжная-речь", "речь-нараспев", "смех-в-речи"];

const RUSSIAN_GOLD_BRACKET_TAGS: &[&str] = &[
    "вдох",
    "вздох",
    "вибрация-губами",
    "втягивание-воздуха-сквозь-зубы",
    "выдох",
    "выдувание-воздуха",
    "глотание",
    "дыхание",
    "зевок",
    "икота",
    "кашель",
    "кряхтенье",
    "напевание",
    "насмешка",
    "неразборчиво",
    "плач",
    "причмокивание",
    "прочищение-горла",
    "радостный-возглас",
    "резкий-вздох",
    "свист",
    "смех",
    "смешок",
    "стон",
    "фырканье",
    "хихиканье",
    "цоканье-языком",
    "чихание",
    "шиканье",
    "шмыганье-носом",
    "щелчок-языком",
    "музыка",
    "ветер",
    "другая-погода",
    "лай-собаки",
    "птица",
    "другое-животное",
    "клаксон",
    "сирена",
    "поезд",
    "улица-другое",
    "вентилятор",
    "сигнал",
    "дверь",
    "фон-речь",
    "плач-ребенка",
    "иностранная-речь",
    "другой-звук-тихо",
    "другой-звук-отчетливо",
    "другой-звук-громко",
    "стук-тихо",
    "стук-отчетливо",
    "стук-громко",
    "фон-другое-тихо",
    "фон-другое-отчетливо",
    "фон-другое-громко",
];

const RUSSIAN_GOLD_NON_BACKGROUND_BRACKET_TAGS: &[&str] = &[
    "вдох",
    "вздох",
    "вибрация-губами",
    "втягивание-воздуха-сквозь-зубы",
    "выдох",
    "выдувание-воздуха",
    "глотание",
    "дыхание",
    "зевок",
    "икота",
    "кашель",
    "кряхтенье",
    "напевание",
    "насмешка",
    "неразборчиво",
    "плач",
    "причмокивание",
    "прочищение-горла",
    "радостный-возглас",
    "резкий-вздох",
    "свист",
    "смех",
    "смешок",
    "стон",
    "фырканье",
    "хихиканье",
    "цоканье-языком",
    "чихание",
    "шиканье",
    "шмыганье-носом",
    "щелчок-языком",
    "иностранная-речь",
    "другой-звук-тихо",
    "другой-звук-отчетливо",
    "другой-звук-громко",
];

const RUSSIAN_GOLD_STYLE_TAGS: &[&str] = &[
    "протяжная-речь",
    "речь-нараспев",
    "смех-в-речи",
    "подражание",
    "радостно",
    "грустно",
    "сердито",
    "испуганно",
    "доверительно",
    "брезгливо",
    "предвкушая",
    "удивленно",
    "иностранная-речь",
];

const RUSSIAN_SILVER_BRACKET_TAGS: &[&str] = &["неразборчиво"];
const EMPTY_STYLE_TAGS: &[&str] = &[];
const EMPTY_CURLY_TAGS: &[&str] = &[];

const ENGLISH_BRACKET_TAGS: &[&str] = &[
    "breath",
    "inhale",
    "exhale",
    "sigh",
    "sniff",
    "gasp",
    "blow",
    "laugh",
    "chuckle",
    "giggle",
    "snort",
    "scoff",
    "grunt",
    "groan",
    "cry",
    "hum-tune",
    "whoop",
    "whistle",
    "tongue-click",
    "tsk",
    "lip-smack",
    "teeth-suck",
    "lip-trill",
    "shush",
    "swallow",
    "clear-throat",
    "cough",
    "sneeze",
    "yawn",
    "hiccup",
    "unintelligible",
];

const ENGLISH_STYLE_TAGS: &[&str] = &["laugh-speech", "singing", "elongate"];
const ENGLISH_CURLY_TAGS: &[&str] = &["MIS: ", "PRO: "];
const RUSSIAN_CURLY_TAGS: &[&str] = &["MIS: ", "PRO: ", "ИСКАЖ: ", "СКАЗ: "];

const LEGACY_RUSSIAN_ENABLED_LINTER_IDS: &[&str] = &[
    "empty-content",
    "multiple-asterisks",
    "unpaired-asterisks",
    "empty-emphasis",
    "punctuation-in-emphasis",
    "bracket-validation",
    "style-tag-validation",
    "curly-bracket-validation",
    "double-spaces",
    "leading-trailing-spaces",
    "overlapping-segments",
    "distance-between-annotations",
    "triple-dash",
    "spaced-double-dash",
    "double-dash-punctuation",
    "double-dash-missing-space",
    "unresolved-low-confidence",
    "newline-characters",
    "consecutive-punctuation",
    "intra-segment-duplicate",
    "duplicated-segments",
    "duplicated-segments-adjacent",
    "unbracketed-annotation-tags",
    "malformed-curly-bracket-tags",
    "banned-characters",
    "emoji",
    "em-dash",
    "unsupported-unicode",
    "consecutive-capitals",
    "consecutive-chars",
    "excessive-digits",
    "banned-words",
    "parentheses-usage",
    "unpaired-quotes",
    "tag-only-punctuation",
    "long-segment",
];

const RUSSIAN_TRANSCRIPTION_ENABLED_LINTER_IDS: &[&str] = &[
    "empty-content",
    "multiple-asterisks",
    "unpaired-asterisks",
    "empty-emphasis",
    "punctuation-in-emphasis",
    "bracket-validation",
    "style-tag-validation",
    "curly-bracket-validation",
    "double-spaces",
    "leading-trailing-spaces",
    "overlapping-segments",
    "distance-between-annotations",
    "triple-dash",
    "spaced-double-dash",
    "double-dash-punctuation",
    "double-dash-missing-space",
    "unresolved-low-confidence",
    "newline-characters",
    "consecutive-punctuation",
    "intra-segment-duplicate",
    "duplicated-segments",
    "duplicated-segments-adjacent",
    "unbracketed-annotation-tags",
    "banned-characters",
    "emoji",
    "em-dash",
    "unsupported-unicode",
    "consecutive-capitals",
    "consecutive-chars",
    "excessive-digits",
    "banned-words",
    "parentheses-usage",
    "unpaired-quotes",
    "tag-only-punctuation",
    "long-segment",
];

const RUSSIAN_SILVER_ENABLED_LINTER_IDS: &[&str] = &[
    "empty-content",
    "bracket-validation",
    "double-spaces",
    "leading-trailing-spaces",
    "overlapping-segments",
    "distance-between-annotations",
    "triple-dash",
    "no-angle-brackets",
    "no-asterisks",
    "no-curly-brackets",
    "newline-characters",
    "consecutive-punctuation",
    "intra-segment-duplicate",
    "duplicated-segments",
    "duplicated-segments-adjacent",
    "banned-characters",
    "emoji",
    "em-dash",
    "unsupported-unicode",
    "long-segment",
];

const ENGLISH_ENABLED_LINTER_IDS: &[&str] = &[
    "empty-content",
    "multiple-asterisks",
    "unpaired-asterisks",
    "empty-emphasis",
    "punctuation-in-emphasis",
    "bracket-validation",
    "style-tag-validation",
    "curly-bracket-validation",
    "double-spaces",
    "leading-trailing-spaces",
    "overlapping-segments",
    "distance-between-annotations",
    "triple-dash",
    "spaced-double-dash",
    "double-dash-punctuation",
    "double-dash-missing-space",
    "single-dash-missing-space",
    "filler-word-spelling",
    "unresolved-low-confidence",
    "newline-characters",
    "consecutive-punctuation",
    "intra-segment-duplicate",
    "duplicated-segments",
    "duplicated-segments-adjacent",
    "unbracketed-annotation-tags",
    "banned-characters",
    "emoji",
    "em-dash",
    "unsupported-unicode",
    "consecutive-capitals",
    "consecutive-chars",
    "excessive-digits",
    "banned-words",
    "parentheses-usage",
    "unpaired-quotes",
    "tag-only-punctuation",
    "long-segment",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BabelProjectLintTagProfile {
    LegacyRussian,
    RussianGold,
    RussianGoldNonBackground,
    RussianTranscription,
    RussianSilver,
    EnglishTranscription,
    Recovered(&'static str),
}

impl BabelProjectLintTagProfile {
    pub fn key(self) -> &'static str {
        match self {
            Self::LegacyRussian => "legacy-russian",
            Self::RussianGold => "ru-tx-gold",
            Self::RussianGoldNonBackground => "ru-tx-gold-non-bg",
            Self::RussianTranscription => "ru-transcription",
            Self::RussianSilver => "ru-tx-silver",
            Self::EnglishTranscription => "english-transcription",
            Self::Recovered(key) => key,
        }
    }

    fn config(self) -> BabelProjectLintTagConfig {
        match self {
            Self::LegacyRussian => BabelProjectLintTagConfig {
                profile: self,
                language: "RU",
                enabled_linter_ids: LEGACY_RUSSIAN_ENABLED_LINTER_IDS,
                bracket_tags: RUSSIAN_TRANSCRIPTION_BRACKET_TAGS,
                style_tags: RUSSIAN_TRANSCRIPTION_STYLE_TAGS,
                curly_tags: RUSSIAN_CURLY_TAGS,
            },
            Self::RussianGold => BabelProjectLintTagConfig {
                profile: self,
                language: "RU",
                enabled_linter_ids: LEGACY_RUSSIAN_ENABLED_LINTER_IDS,
                bracket_tags: RUSSIAN_GOLD_BRACKET_TAGS,
                style_tags: RUSSIAN_GOLD_STYLE_TAGS,
                curly_tags: RUSSIAN_CURLY_TAGS,
            },
            Self::RussianGoldNonBackground => BabelProjectLintTagConfig {
                profile: self,
                language: "RU",
                enabled_linter_ids: LEGACY_RUSSIAN_ENABLED_LINTER_IDS,
                bracket_tags: RUSSIAN_GOLD_NON_BACKGROUND_BRACKET_TAGS,
                style_tags: RUSSIAN_GOLD_STYLE_TAGS,
                curly_tags: RUSSIAN_CURLY_TAGS,
            },
            Self::RussianTranscription => BabelProjectLintTagConfig {
                profile: self,
                language: "RU",
                enabled_linter_ids: RUSSIAN_TRANSCRIPTION_ENABLED_LINTER_IDS,
                bracket_tags: RUSSIAN_TRANSCRIPTION_BRACKET_TAGS,
                style_tags: RUSSIAN_TRANSCRIPTION_STYLE_TAGS,
                curly_tags: RUSSIAN_CURLY_TAGS,
            },
            Self::RussianSilver => BabelProjectLintTagConfig {
                profile: self,
                language: "RU",
                enabled_linter_ids: RUSSIAN_SILVER_ENABLED_LINTER_IDS,
                bracket_tags: RUSSIAN_SILVER_BRACKET_TAGS,
                style_tags: EMPTY_STYLE_TAGS,
                curly_tags: EMPTY_CURLY_TAGS,
            },
            Self::EnglishTranscription => BabelProjectLintTagConfig {
                profile: self,
                language: "EN",
                enabled_linter_ids: ENGLISH_ENABLED_LINTER_IDS,
                bracket_tags: ENGLISH_BRACKET_TAGS,
                style_tags: ENGLISH_STYLE_TAGS,
                curly_tags: ENGLISH_CURLY_TAGS,
            },
            Self::Recovered(key) => recovered_profile_by_key(key)
                .map(config_from_recovered_profile)
                .unwrap_or_else(|| Self::LegacyRussian.config()),
        }
    }
}

fn config_from_recovered_profile(profile: RecoveredProjectProfile) -> BabelProjectLintTagConfig {
    BabelProjectLintTagConfig {
        profile: profile_enum_for_recovered_key(profile.key),
        language: profile.language,
        enabled_linter_ids: profile.enabled_linter_ids,
        bracket_tags: profile.bracket_tags,
        style_tags: profile.style_tags,
        curly_tags: profile.curly_tags,
    }
}

fn profile_enum_for_recovered_key(key: &'static str) -> BabelProjectLintTagProfile {
    match key {
        "ru-tx-gold" => BabelProjectLintTagProfile::RussianGold,
        "ru-tx-gold-non-bg" => BabelProjectLintTagProfile::RussianGoldNonBackground,
        "ru-transcription" => BabelProjectLintTagProfile::RussianTranscription,
        "ru-tx-silver" => BabelProjectLintTagProfile::RussianSilver,
        "english-transcription" => BabelProjectLintTagProfile::EnglishTranscription,
        key => BabelProjectLintTagProfile::Recovered(key),
    }
}

fn recovered_profile_by_key(key: &str) -> Option<RecoveredProjectProfile> {
    RECOVERED_PROJECT_PROFILES
        .iter()
        .copied()
        .find(|profile| profile.key.eq_ignore_ascii_case(key))
}

fn recovered_profile_from_identifier(value: &str) -> Option<RecoveredProjectProfile> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    RECOVERED_PROJECT_PROFILES.iter().copied().find(|profile| {
        profile
            .identifiers
            .iter()
            .any(|candidate| value.eq_ignore_ascii_case(candidate))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BabelProjectLintTagConfig {
    pub profile: BabelProjectLintTagProfile,
    pub language: &'static str,
    pub enabled_linter_ids: &'static [&'static str],
    pub bracket_tags: &'static [&'static str],
    pub style_tags: &'static [&'static str],
    pub curly_tags: &'static [&'static str],
}

impl Default for BabelProjectLintTagConfig {
    fn default() -> Self {
        BabelProjectLintTagProfile::LegacyRussian.config()
    }
}

impl BabelProjectLintTagConfig {
    pub fn rule_enabled(self, rule_id: &str) -> bool {
        self.enabled_linter_ids.contains(&rule_id)
    }
}

pub fn resolve_babel_project_lint_tag_config(project: &Project) -> BabelProjectLintTagConfig {
    recovered_profile_from_project(project)
        .map(config_from_recovered_profile)
        .unwrap_or_else(|| {
            profile_from_project(project)
                .unwrap_or(BabelProjectLintTagProfile::LegacyRussian)
                .config()
        })
}

pub fn babel_project_lint_tag_metadata(project: &Project) -> Vec<(String, String)> {
    let config = resolve_babel_project_lint_tag_config(project);
    vec![
        (
            META_BABEL_LINT_TAG_PROFILE.to_string(),
            config.profile.key().to_string(),
        ),
        (
            META_BABEL_LINT_TAG_LANGUAGE.to_string(),
            config.language.to_string(),
        ),
        (
            META_BABEL_ENABLED_LINTER_IDS.to_string(),
            config.enabled_linter_ids.join(","),
        ),
        (
            META_BABEL_BRACKET_TAGS.to_string(),
            config.bracket_tags.join(","),
        ),
        (
            META_BABEL_STYLE_TAGS.to_string(),
            config.style_tags.join(","),
        ),
    ]
}

fn recovered_profile_from_project(project: &Project) -> Option<RecoveredProjectProfile> {
    project
        .metadata
        .get(META_BABEL_LINT_TAG_PROFILE)
        .and_then(|value| recovered_profile_from_identifier(value))
        .or_else(|| recovered_profile_from_identifier(&project.id))
        .or_else(|| recovered_profile_from_identifier(&project.name))
        .or_else(|| recovered_profile_from_metadata(project))
        .or_else(|| recovered_profile_from_extra_package(project))
}

fn recovered_profile_from_metadata(project: &Project) -> Option<RecoveredProjectProfile> {
    [
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
    ]
    .into_iter()
    .filter_map(|key| project.metadata.get(key))
    .find_map(|value| recovered_profile_from_identifier(value))
}

fn profile_from_project(project: &Project) -> Option<BabelProjectLintTagProfile> {
    project
        .metadata
        .get(META_BABEL_LINT_TAG_PROFILE)
        .and_then(|value| profile_from_identifier(value))
        .or_else(|| profile_from_identifier(&project.id))
        .or_else(|| profile_from_identifier(&project.name))
        .or_else(|| profile_from_metadata(project))
        .or_else(|| profile_from_extra_package(project))
        .or_else(|| language_profile_from_metadata(project))
}

fn profile_from_metadata(project: &Project) -> Option<BabelProjectLintTagProfile> {
    [
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
    ]
    .into_iter()
    .filter_map(|key| project.metadata.get(key))
    .find_map(|value| profile_from_identifier(value))
}

fn language_profile_from_metadata(project: &Project) -> Option<BabelProjectLintTagProfile> {
    ["language", "languageCode", "studio.babel.language"]
        .into_iter()
        .filter_map(|key| project.metadata.get(key))
        .find_map(|value| profile_from_language(value))
}

fn recovered_profile_from_extra_package(project: &Project) -> Option<RecoveredProjectProfile> {
    let value: Value = serde_json::from_str(project.metadata.get(META_EXTRA_PACKAGE)?).ok()?;
    first_string_by_keys(
        &value,
        &[
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
        ],
    )
    .and_then(|value| recovered_profile_from_identifier(&value))
}

fn profile_from_extra_package(project: &Project) -> Option<BabelProjectLintTagProfile> {
    let value: Value = serde_json::from_str(project.metadata.get(META_EXTRA_PACKAGE)?).ok()?;
    first_string_by_keys(
        &value,
        &[
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
        ],
    )
    .and_then(|value| profile_from_identifier(&value))
    .or_else(|| {
        first_string_by_keys(&value, &["language", "languageCode"])
            .and_then(|value| profile_from_language(&value))
    })
}

fn first_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(found.to_string());
                }
            }
            map.values()
                .find_map(|child| first_string_by_keys(child, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| first_string_by_keys(child, keys)),
        _ => None,
    }
}

fn profile_from_identifier(value: &str) -> Option<BabelProjectLintTagProfile> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    if matches_identifier(
        value,
        &[
            "0bf29f0c-371a-506a-8b77-5fffd14e5762",
            "ru-tx-gold",
            "ru_tx_gold",
            "russian-transcription-gold",
            "f95babcc-8681-521c-a39f-66454a16347f",
            "a47543d7-8687-5ffe-95d3-754387fe3426",
        ],
    ) {
        return Some(BabelProjectLintTagProfile::RussianGold);
    }

    if matches_identifier(
        value,
        &[
            "73f8e130-9fb4-5c17-b142-5a2e75d89b6d",
            "ru-tx-silver",
            "ru_tx_silver",
            "russian-transcription-silver",
            "866acc27-eb40-51ee-bb4e-57b4cb15fb0c",
            "a10c7c04-49bc-5376-97d6-03c3d154bfa8",
        ],
    ) {
        return Some(BabelProjectLintTagProfile::RussianSilver);
    }

    if matches_identifier(
        value,
        &[
            "fc3f4db4-a201-5286-9a7c-411365bf6eb4",
            "ru-tx-gold-non-bg",
            "ru_tx_gold_non_bg",
            "russian-transcription-gold-non-bg",
            "40ab3a13-32d2-5095-a67c-eaa1fb109fbd",
            "6d29da26-5417-5e96-a6f5-7235e1dd40e4",
        ],
    ) {
        return Some(BabelProjectLintTagProfile::RussianGoldNonBackground);
    }

    if matches_identifier(
        value,
        &[
            "d6031436-1fd5-4c79-9b8f-2a7452fc8c4b",
            "ru-transcription",
            "ru_transcription",
            "russian-transcription",
            "bab7b5be-eb9e-43a0-b713-16b9d2fed66a",
            "638f1a1f-d811-4140-ad5e-ec92a84c03ae",
        ],
    ) {
        return Some(BabelProjectLintTagProfile::RussianTranscription);
    }

    if matches_identifier(
        value,
        &[
            "english-transcription",
            "english_transcription",
            "en-transcription",
            "en_tx",
            "en",
        ],
    ) {
        return Some(BabelProjectLintTagProfile::EnglishTranscription);
    }

    None
}

fn profile_from_language(value: &str) -> Option<BabelProjectLintTagProfile> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("EN") || value.eq_ignore_ascii_case("ENG") {
        return Some(BabelProjectLintTagProfile::EnglishTranscription);
    }
    if value.eq_ignore_ascii_case("RU") || value.eq_ignore_ascii_case("RUS") {
        return Some(BabelProjectLintTagProfile::RussianTranscription);
    }
    None
}

fn matches_identifier(value: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| value.eq_ignore_ascii_case(candidate))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use studio_core::Project;

    use super::recovered_project_profiles::{RECOVERED_PROJECT_PROFILES, RecoveredProjectProfile};
    use super::{
        BabelProjectLintTagConfig, BabelProjectLintTagProfile, META_ACTIVE_QUEUE_ID,
        META_BABEL_BRACKET_TAGS, META_BABEL_LINT_TAG_LANGUAGE, META_BABEL_LINT_TAG_PROFILE,
        META_BABEL_PROJECT_ID, META_BABEL_STYLE_TAGS, babel_project_lint_tag_metadata,
        resolve_babel_project_lint_tag_config,
    };

    #[test]
    fn known_russian_gold_project_and_queue_metadata_resolve_to_gold_profile() {
        for (metadata_key, metadata_value) in [
            (
                META_BABEL_PROJECT_ID,
                "0bf29f0c-371a-506a-8b77-5fffd14e5762",
            ),
            (META_ACTIVE_QUEUE_ID, "f95babcc-8681-521c-a39f-66454a16347f"),
        ] {
            let project = project_with_metadata(&[(metadata_key, metadata_value)]);
            let config = resolve_babel_project_lint_tag_config(&project);
            let normalized = babel_project_lint_tag_metadata(&project)
                .into_iter()
                .collect::<BTreeMap<_, _>>();

            assert_eq!(config.profile, BabelProjectLintTagProfile::RussianGold);
            assert_eq!(
                normalized
                    .get(META_BABEL_LINT_TAG_PROFILE)
                    .map(String::as_str),
                Some("ru-tx-gold")
            );
            assert_eq!(
                normalized
                    .get(META_BABEL_LINT_TAG_LANGUAGE)
                    .map(String::as_str),
                Some("RU")
            );
            assert!(
                normalized
                    .get(META_BABEL_BRACKET_TAGS)
                    .is_some_and(|tags| tags.split(',').any(|tag| tag == "музыка"))
            );
            assert!(
                normalized
                    .get(META_BABEL_STYLE_TAGS)
                    .is_some_and(|tags| tags.split(',').any(|tag| tag == "подражание"))
            );
        }
    }

    #[test]
    fn every_recovered_project_profile_resolves_from_generated_project_id() {
        for profile in RECOVERED_PROJECT_PROFILES {
            let project_id = profile
                .identifiers
                .iter()
                .find(|identifier| identifier.matches('-').count() == 4)
                .unwrap_or_else(|| {
                    panic!(
                        "recovered profile {} has no generated Babel project id identifier",
                        profile.key
                    )
                });
            let project = project_with_metadata(&[(META_BABEL_PROJECT_ID, project_id)]);
            let config = resolve_babel_project_lint_tag_config(&project);

            assert_config_matches_recovered_profile(config, *profile);
        }
    }

    #[test]
    fn every_recovered_project_uuid_identifier_resolves_to_exact_profile_config() {
        for profile in RECOVERED_PROJECT_PROFILES {
            let mut uuid_identifier_count = 0;

            for project_id in profile
                .identifiers
                .iter()
                .copied()
                .filter(|identifier| is_uuid_shaped(identifier))
            {
                uuid_identifier_count += 1;
                let project = project_with_metadata(&[(META_BABEL_PROJECT_ID, project_id)]);
                let config = resolve_babel_project_lint_tag_config(&project);

                assert_config_matches_recovered_profile(config, *profile);
            }

            assert!(
                uuid_identifier_count > 0,
                "recovered profile {} has no UUID-shaped Babel project id identifier",
                profile.key
            );
        }
    }

    #[test]
    fn russian_degraded_gold_project_id_resolves_to_degraded_russian_profile_data() {
        let profile = recovered_profile("ru-tx-gold-dg");
        let project = project_with_metadata(&[(
            META_BABEL_PROJECT_ID,
            "1c97f55a-3868-4861-9778-d39ee80cda3d",
        )]);
        let config = resolve_babel_project_lint_tag_config(&project);

        assert_config_matches_recovered_profile(config, profile);
        assert_eq!(config.profile.key(), "ru-tx-gold-dg");
        assert_eq!(config.language, "RU");
        assert!(config.bracket_tags.contains(&"статика-тихо"));
        assert!(config.bracket_tags.contains(&"гул-громко"));
        assert!(config.style_tags.contains(&"искаженная-речь"));
        assert!(config.rule_enabled("malformed-curly-bracket-tags"));
    }

    #[test]
    fn arabic_emotion_project_id_resolves_to_arabic_profile_data() {
        let profile = recovered_profile("ar-emotion-tx");
        let project = project_with_metadata(&[(
            META_BABEL_PROJECT_ID,
            "90f78329-5f76-4895-9066-259b1f5fa740",
        )]);
        let config = resolve_babel_project_lint_tag_config(&project);

        assert_config_matches_recovered_profile(config, profile);
        assert_eq!(config.profile.key(), "ar-emotion-tx");
        assert_eq!(config.language, "AR");
        assert!(config.rule_enabled("arabic-foreign-tag-marker"));
        assert!(!config.rule_enabled("malformed-curly-bracket-tags"));
        assert!(config.bracket_tags.contains(&"أنة-شكوى"));
        assert!(config.style_tags.contains(&"آمِر"));
        assert!(!config.bracket_tags.contains(&"breath"));
        assert!(!config.bracket_tags.contains(&"вдох"));
    }

    fn assert_config_matches_recovered_profile(
        config: BabelProjectLintTagConfig,
        profile: RecoveredProjectProfile,
    ) {
        assert_eq!(config.profile.key(), profile.key);
        assert_eq!(config.language, profile.language);
        assert_eq!(config.enabled_linter_ids, profile.enabled_linter_ids);
        assert_eq!(config.bracket_tags, profile.bracket_tags);
        assert_eq!(config.style_tags, profile.style_tags);
        assert_eq!(config.curly_tags, profile.curly_tags);
    }

    fn is_uuid_shaped(identifier: &str) -> bool {
        let mut parts = identifier.split('-');
        let Some(first) = parts.next() else {
            return false;
        };
        let Some(second) = parts.next() else {
            return false;
        };
        let Some(third) = parts.next() else {
            return false;
        };
        let Some(fourth) = parts.next() else {
            return false;
        };
        let Some(fifth) = parts.next() else {
            return false;
        };

        parts.next().is_none()
            && uuid_part_matches(first, 8)
            && uuid_part_matches(second, 4)
            && uuid_part_matches(third, 4)
            && uuid_part_matches(fourth, 4)
            && uuid_part_matches(fifth, 12)
    }

    fn uuid_part_matches(part: &str, expected_len: usize) -> bool {
        part.len() == expected_len && part.bytes().all(|byte| byte.is_ascii_hexdigit())
    }

    fn recovered_profile(key: &str) -> RecoveredProjectProfile {
        RECOVERED_PROJECT_PROFILES
            .iter()
            .copied()
            .find(|profile| profile.key == key)
            .unwrap_or_else(|| panic!("missing recovered profile {key}"))
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
