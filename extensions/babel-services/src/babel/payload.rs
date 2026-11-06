//! Project-metadata keys/lookups and outbound annotation payload builders.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, anyhow, bail};
use exaudio_babel_domain::workflow::BabelWorkflowExt;
use serde_json::{Map, Value};
use studio_core::{Project, Segment};
use studio_editor::workflow::StudioWorkflow;

pub use exaudio_babel_lint::project_lint_tag_config::{
    META_BABEL_PROJECT_ID as META_PROJECT_ID, META_BABEL_PROJECT_SLUG as META_PROJECT_SLUG,
};

pub const META_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
pub const META_ACTIVE_QUEUE_ID: &str = "studio.babel.activeQueueId";
pub const META_QUEUE_ID: &str = "studio.babel.queueId";
pub const META_CHUNKED_QUEUE_ID: &str = "studio.babel.chunkedQueueId";
pub const META_STITCHED_QUEUE_ID: &str = "studio.babel.stitchedQueueId";
pub const META_IS_STITCHED_QUEUE: &str = "studio.babel.isStitchedQueue";
pub const META_DISPLAY_FEEDBACK: &str = "studio.babel.displayFeedback";
pub const META_FEEDBACK_INITIAL_OPEN: &str = "studio.babel.feedback.initialOpen";
pub const META_TASK_LOOKUP_PROCESSED_ID: &str = "studio.babel.taskLookup.processedTranscriptionId";
pub const META_TASK_LOOKUP_REVIEW_ACTION_ID: &str = "studio.babel.taskLookup.reviewActionId";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";

/// Route keys carried from the current project into a newly loaded task.
pub const ROUTE_METADATA_KEYS: &[&str] = &[
    META_PROJECT_ID,
    META_PROJECT_SLUG,
    META_REVIEW_ACTION_ID,
    META_ACTIVE_QUEUE_ID,
    META_QUEUE_ID,
    META_CHUNKED_QUEUE_ID,
    META_STITCHED_QUEUE_ID,
    META_DISPLAY_FEEDBACK,
    META_FEEDBACK_INITIAL_OPEN,
    META_TASK_LOOKUP_PROCESSED_ID,
    META_TASK_LOOKUP_REVIEW_ACTION_ID,
];

/// The dashboard/lookup ids a task is opened with; blank fields are skipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteContext {
    pub project_id: String,
    /// Dashboard project name (`RU-tx-gold`): the task's language comes from its code.
    pub project_slug: String,
    pub chunked_queue_id: String,
    pub stitched_queue_id: String,
    pub review_action_id: String,
    pub processed_transcription_id: String,
}

impl RouteContext {
    /// Metadata entries for these ids (chunked queue also becomes the active queue).
    pub fn metadata(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();
        let mut push = |key: &str, value: &str| {
            let value = value.trim();
            if !value.is_empty() {
                entries.push((key.to_string(), value.to_string()));
            }
        };
        push(META_PROJECT_ID, &self.project_id);
        push(META_PROJECT_SLUG, &self.project_slug);
        push(META_CHUNKED_QUEUE_ID, &self.chunked_queue_id);
        push(META_ACTIVE_QUEUE_ID, &self.chunked_queue_id);
        push(META_STITCHED_QUEUE_ID, &self.stitched_queue_id);
        push(META_REVIEW_ACTION_ID, &self.review_action_id);
        push(META_TASK_LOOKUP_REVIEW_ACTION_ID, &self.review_action_id);
        push(
            META_TASK_LOOKUP_PROCESSED_ID,
            &self.processed_transcription_id,
        );
        entries
    }

    /// The current project's route keys overlaid with this context — what a
    /// newly loaded task inherits.
    pub fn merged_over(&self, current: Option<&Project>) -> Vec<(String, String)> {
        let mut merged: BTreeMap<String, String> = current
            .map(route_metadata_from_project)
            .unwrap_or_default()
            .into_iter()
            .collect();
        merged.extend(self.metadata());
        merged.into_iter().collect()
    }
}

/// The [`ROUTE_METADATA_KEYS`] present on `project`.
pub fn route_metadata_from_project(project: &Project) -> Vec<(String, String)> {
    ROUTE_METADATA_KEYS
        .iter()
        .filter_map(|key| {
            project
                .metadata
                .get(*key)
                .map(|value| ((*key).to_string(), value.clone()))
        })
        .collect()
}

/// Review action id: project metadata, else segment metadata, else the
/// Babel package/envelope. Empty when unknown.
pub fn review_action_id(project: &Project) -> String {
    first_nonempty([
        trimmed_metadata(project, META_REVIEW_ACTION_ID),
        review_action_id_from_segments(project),
        review_action_id_from_package(project),
        review_action_id_from_envelope(project),
    ])
}

/// Queue a claim was taken from (drop-claim target): active, else queue, else chunked queue id.
pub fn active_queue_id(project: &Project) -> String {
    first_nonempty([
        trimmed_metadata(project, META_ACTIVE_QUEUE_ID),
        trimmed_metadata(project, META_QUEUE_ID),
        trimmed_metadata(project, META_CHUNKED_QUEUE_ID),
    ])
}

/// Stitched queue for task lookup: stitched, else active, else queue id.
pub fn stitched_queue_id(project: &Project) -> String {
    first_nonempty([
        trimmed_metadata(project, META_STITCHED_QUEUE_ID),
        trimmed_metadata(project, META_ACTIVE_QUEUE_ID),
        trimmed_metadata(project, META_QUEUE_ID),
    ])
}

/// Babel project id (metadata, else the project id).
pub fn feedback_project_id(project: &Project) -> String {
    first_nonempty([
        trimmed_metadata(project, META_PROJECT_ID),
        Some(project.id.trim().to_string()),
    ])
}

/// `true`/`1`/`yes`/`on` (any common casing) in [`META_IS_STITCHED_QUEUE`].
pub fn is_stitched_queue(project: &Project) -> bool {
    project
        .metadata
        .get(META_IS_STITCHED_QUEUE)
        .is_some_and(|value| {
            matches!(
                value.trim(),
                "true" | "1" | "yes" | "on" | "True" | "TRUE" | "Yes" | "YES" | "On" | "ON"
            )
        })
}

/// `transcriptions.saveAnnotationsByReviewActionId` input.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveAnnotationsRequest {
    pub review_action_id: String,
    pub annotations: Vec<Value>,
}

/// `transcriptions.submitTranscriptReviewAction` input.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmitReviewActionRequest {
    pub review_action_id: String,
    pub annotations: Vec<Value>,
    pub is_stitched_queue: bool,
}

impl SaveAnnotationsRequest {
    /// Exported annotations with `metadata.assertedWarnings` merged from
    /// `asserted_warning_keys` (`"<segment id>::<exact reason>"`).
    pub fn from_workflow(
        workflow: &StudioWorkflow,
        asserted_warning_keys: &BTreeSet<String>,
    ) -> Result<Self> {
        let review_action_id = review_action_id(workflow.project());
        let annotations = exported_annotations(workflow, "save", asserted_warning_keys)?;
        if review_action_id.is_empty() {
            bail!("the task has no Babel task id");
        }
        Ok(Self {
            review_action_id,
            annotations,
        })
    }
}

impl SubmitReviewActionRequest {
    pub fn from_workflow(
        workflow: &StudioWorkflow,
        asserted_warning_keys: &BTreeSet<String>,
    ) -> Result<Self> {
        let project = workflow.project();
        let review_action_id = review_action_id(project);
        let annotations = exported_annotations(workflow, "submit", asserted_warning_keys)?;
        if review_action_id.is_empty() {
            bail!("the task has no Babel task id");
        }
        Ok(Self {
            review_action_id,
            annotations,
            is_stitched_queue: is_stitched_queue(project),
        })
    }
}

fn exported_annotations(
    workflow: &StudioWorkflow,
    operation: &str,
    asserted_warning_keys: &BTreeSet<String>,
) -> Result<Vec<Value>> {
    let exported = workflow
        .export_babel_json()
        .with_context(|| format!("couldn't prepare the {operation} data"))?;
    let value: Value = serde_json::from_str(&exported)
        .with_context(|| format!("couldn't read the prepared {operation} data"))?;
    let mut annotations = find_array_by_key(&value, "annotations")
        .cloned()
        .ok_or_else(|| anyhow!("the prepared data has no annotations"))?;
    merge_asserted_warnings(&mut annotations, workflow.project(), asserted_warning_keys);
    Ok(annotations)
}

/// Sets `metadata.assertedWarnings` on every annotation backing a segment
/// (an empty array clears previously persisted assertions).
fn merge_asserted_warnings(
    annotations: &mut [Value],
    project: &Project,
    asserted_warning_keys: &BTreeSet<String>,
) {
    let warnings_by_annotation = asserted_warnings_by_annotation(project, asserted_warning_keys);
    for annotation in annotations {
        let Some(object) = annotation.as_object_mut() else {
            continue;
        };
        let Some(asserted) = object
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| warnings_by_annotation.get(id))
        else {
            continue;
        };
        let asserted = Value::Array(asserted.iter().cloned().map(Value::String).collect());
        if let Some(metadata) = object
            .entry("metadata")
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
        {
            metadata.insert("assertedWarnings".to_string(), asserted);
        }
    }
}

fn asserted_warnings_by_annotation(
    project: &Project,
    asserted_warning_keys: &BTreeSet<String>,
) -> BTreeMap<String, Vec<String>> {
    let mut warnings = BTreeMap::<String, Vec<String>>::new();
    for segment in &project.segments {
        let prefix = format!("{}::", segment.id);
        let reasons = asserted_warning_keys
            .iter()
            .filter_map(|key| key.strip_prefix(&prefix))
            .collect::<Vec<_>>();
        for annotation_id in segment_annotation_ids(segment) {
            let entry = warnings.entry(annotation_id).or_default();
            for reason in &reasons {
                if !entry.iter().any(|existing| existing == reason) {
                    entry.push((*reason).to_string());
                }
            }
        }
    }
    warnings
}

fn segment_annotation_ids(segment: &Segment) -> impl Iterator<Item = String> + '_ {
    std::iter::once(segment.id.clone()).chain(
        segment
            .source_annotation_id
            .iter()
            .filter(|source_id| *source_id != &segment.id)
            .cloned(),
    )
}

fn review_action_id_from_segments(project: &Project) -> Option<String> {
    project.segments.iter().find_map(|segment| {
        segment
            .metadata
            .get("reviewActionId")
            .and_then(metadata_string)
            .or_else(|| {
                segment
                    .metadata
                    .get("review_action_id")
                    .and_then(metadata_string)
            })
    })
}

fn review_action_id_from_package(project: &Project) -> Option<String> {
    let package = serde_json::from_str::<Value>(project.metadata.get(META_EXTRA_PACKAGE)?).ok()?;
    let package = package.as_object()?;
    [
        "reviewActionId",
        "review_action_id",
        "actionId",
        "action_id",
    ]
    .iter()
    .find_map(|key| package.get(*key).and_then(metadata_string))
}

fn review_action_id_from_envelope(project: &Project) -> Option<String> {
    let envelope =
        serde_json::from_str::<Value>(project.metadata.get(META_REVIEW_ACTION_ENVELOPE)?).ok()?;
    find_string_by_key(&envelope, "actionId")
        .or_else(|| find_string_by_key(&envelope, "reviewActionId"))
}

fn metadata_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.trim().to_string()).filter(|value| !value.is_empty()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn trimmed_metadata(project: &Project, key: &str) -> Option<String> {
    project
        .metadata
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn first_nonempty(values: impl IntoIterator<Item = Option<String>>) -> String {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .unwrap_or_default()
}

fn find_string_by_key(value: &Value, key: &str) -> Option<String> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                map.values()
                    .find_map(|child| find_string_by_key(child, key))
            }),
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_by_key(child, key)),
        _ => None,
    }
}

fn find_array_by_key<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    match value {
        Value::Object(map) => map
            .get(key)
            .and_then(Value::as_array)
            .or_else(|| map.values().find_map(|child| find_array_by_key(child, key))),
        Value::Array(items) => items.iter().find_map(|child| find_array_by_key(child, key)),
        _ => None,
    }
}
