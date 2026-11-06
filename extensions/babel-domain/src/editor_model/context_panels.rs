//! Script-context and original-script panel data extracted from Babel task metadata.

use crate::i18n::localized;
use serde_json::Value;
use studio_core::Project;

const META_EXTRA_PACKAGE: &str = "babel.extra.package";

/// Context fields shown in the script-context panel, in display order: `(key, en, ru)`.
pub const SCRIPT_CONTEXT_FIELDS: [(&str, &str, &str); 6] = [
    ("character", "Character", "Персонаж"),
    ("physical", "Physical", "Физическое состояние"),
    ("emotional", "Emotional", "Эмоциональное состояние"),
    ("speech", "Speech", "Речь"),
    ("vocal", "Vocal", "Голос"),
    ("preamble", "Preamble", "Вступление"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContextField {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContext {
    pub emotion: Option<String>,
    /// Non-empty context fields in [`SCRIPT_CONTEXT_FIELDS`] order.
    pub fields: Vec<ScriptContextField>,
    /// Copyable submission/action id, when the task carries one.
    pub submission_id: Option<String>,
}

/// Script context for the loaded task; `None` when the task has neither an emotion nor any
/// context field.
pub fn script_context(project: &Project) -> Option<ScriptContext> {
    let emotion = project_metadata_string(
        project,
        &["studio.babel.emotion", "emotion"],
        &[
            &["emotion"],
            &["openEndedResponse", "emotion"],
            &["recording", "emotion"],
        ],
    );
    let fields = SCRIPT_CONTEXT_FIELDS
        .iter()
        .filter_map(|&(key, en, ru)| {
            let value = project
                .metadata
                .get(&format!("studio.babel.scriptContext.{key}"))
                .and_then(|value| non_empty_string(value.clone()))
                .or_else(|| context_field_from_extra(project, key))?;
            Some(ScriptContextField {
                key,
                label: localized(en, ru),
                value,
            })
        })
        .collect::<Vec<_>>();
    if emotion.is_none() && fields.is_empty() {
        return None;
    }

    Some(ScriptContext {
        emotion,
        fields,
        submission_id: project_metadata_string(
            project,
            &[
                "studio.babel.submissionId",
                "submissionId",
                "submission_id",
                "actionId",
                "action_id",
                "reviewActionId",
            ],
            &[
                &["submissionId"],
                &["submission_id"],
                &["actionId"],
                &["action_id"],
                &["reviewActionId"],
            ],
        ),
    })
}

/// Original script text for the original-script panel.
pub fn original_script(project: &Project) -> Option<String> {
    project_metadata_string(
        project,
        &[
            "studio.babel.originalScript",
            "originalScript",
            "original_script",
            "scriptText",
            "script_text",
        ],
        &[
            &["originalScript"],
            &["original_script"],
            &["scriptText"],
            &["script_text"],
            &["openEndedResponse", "scriptText"],
            &["recording", "scriptText"],
        ],
    )
}

fn context_field_from_extra(project: &Project, key: &str) -> Option<String> {
    let extra = project_extra_package(project)?;
    [
        &["scriptContext", key][..],
        &["script_context", key],
        &["context", key],
        &["openEndedResponse", "scriptContext", key],
        &["openEndedResponse", "context", key],
    ]
    .into_iter()
    .find_map(|path| value_at_path(&extra, path).and_then(value_to_string))
}

fn project_metadata_string(
    project: &Project,
    direct_keys: &[&str],
    extra_paths: &[&[&str]],
) -> Option<String> {
    direct_keys
        .iter()
        .find_map(|key| {
            project
                .metadata
                .get(*key)
                .and_then(|value| non_empty_string(value.clone()))
        })
        .or_else(|| {
            let extra = project_extra_package(project)?;
            extra_paths
                .iter()
                .find_map(|path| value_at_path(&extra, path).and_then(value_to_string))
        })
}

fn project_extra_package(project: &Project) -> Option<Value> {
    project
        .metadata
        .get(META_EXTRA_PACKAGE)
        .and_then(|source| serde_json::from_str(source).ok())
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => non_empty_string(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn non_empty_string(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
