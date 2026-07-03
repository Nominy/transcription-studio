use std::collections::BTreeMap;

use serde_json::Value;
use studio_app::{
    ORIGINAL_SCRIPT_STORAGE_KEY, OriginalScriptPanelInput, OriginalScriptPanelModel,
    SCRIPT_CONTEXT_STORAGE_KEY, ScriptContext, ScriptContextPanelInput, ScriptContextPanelModel,
    build_original_script_panel_model, build_script_context_panel_model, derive_is_rtl,
    reduce_collapsible_panel_toggle,
};
use studio_core::Project;

use super::StudioState;

const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const SCRIPT_CONTEXT_FIELD_KEYS: [&str; 6] = [
    "character",
    "physical",
    "emotional",
    "speech",
    "vocal",
    "preamble",
];

impl StudioState {
    pub(super) fn script_context_panel(
        &self,
        project: &Project,
    ) -> Option<ScriptContextPanelModel> {
        let emotion = project_metadata_string(
            project,
            &["studio.babel.emotion", "emotion"],
            &[
                &["emotion"],
                &["openEndedResponse", "emotion"],
                &["recording", "emotion"],
            ],
        );
        let context = script_context(project);
        if emotion.is_none() && context.is_none() {
            return None;
        }

        build_script_context_panel_model(ScriptContextPanelInput {
            emotion,
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
            context,
            hidden: self.script_context_panel_hidden,
            is_rtl: derive_is_rtl(&project.id, None),
            copied_submission_id: self.copied_submission_id,
        })
    }

    pub(super) fn original_script_panel(
        &self,
        project: &Project,
    ) -> Option<OriginalScriptPanelModel> {
        let script = project_metadata_string(
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
        )?;

        Some(build_original_script_panel_model(
            OriginalScriptPanelInput {
                script,
                hidden: self.original_script_panel_hidden,
                is_rtl: derive_is_rtl(&project.id, None),
            },
        ))
    }

    pub(super) fn toggle_context_panel(&mut self, storage_key: &str) {
        let current_hidden = match storage_key {
            SCRIPT_CONTEXT_STORAGE_KEY => self.script_context_panel_hidden,
            ORIGINAL_SCRIPT_STORAGE_KEY => self.original_script_panel_hidden,
            _ => {
                self.status = format!("Context panel `{storage_key}` is not available.");
                return;
            }
        };

        let toggle = reduce_collapsible_panel_toggle(storage_key, current_hidden);
        match storage_key {
            SCRIPT_CONTEXT_STORAGE_KEY => self.script_context_panel_hidden = toggle.hidden,
            ORIGINAL_SCRIPT_STORAGE_KEY => self.original_script_panel_hidden = toggle.hidden,
            _ => {}
        }

        if let Err(error) = self
            .panel_collapse_storage
            .apply_tts_panel_effects(&toggle.effects)
        {
            self.status = format!("Context panel preference could not be saved: {error}");
        }
    }
}

fn script_context(project: &Project) -> Option<ScriptContext> {
    let mut context = BTreeMap::new();
    for key in SCRIPT_CONTEXT_FIELD_KEYS {
        if let Some(value) = project
            .metadata
            .get(&format!("studio.babel.scriptContext.{key}"))
            .and_then(|value| non_empty_string(value.clone()))
            .or_else(|| context_field_from_extra(project, key))
        {
            context.insert(key.to_string(), Some(value));
        }
    }

    (!context.is_empty()).then_some(context)
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
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => non_empty_string(value.clone()),
        Value::Number(value) => non_empty_string(value.to_string()),
        Value::Bool(value) => non_empty_string(value.to_string()),
        _ => None,
    }
}

fn non_empty_string(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}
