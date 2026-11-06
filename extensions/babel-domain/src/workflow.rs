use std::{collections::BTreeMap, sync::Arc};
use time::OffsetDateTime;

use crate::editor_model::submit_control::AdminFlagPayload;
use anyhow::{Context, Result};
use exaudio_babel_lint::linter::{LintSeverity, ServerLintIssue};
use serde_json::{Value, json};
use studio_babel::{META_REVIEW_ACTION_DECISION, export_json, import_json};
use studio_core::{EditorCommand, Project, Segment, SegmentMutationPolicy};

use crate::babel_metadata::META_DROP_CLAIM_DROPPED;
use crate::i18n::{localized, localized_fmt};
use studio_editor::workflow::StudioWorkflow;

const META_SELECTED_COMPARE_ACTION_ID: &str = "studio.babel.diff.selectedCompareActionId";
const META_SHOW_REVIEWERS: &str = "studio.babel.diff.showReviewers";
const META_ADMIN_FLAG_RATING: &str = "studio.babel.adminFlag.rating";
const META_ADMIN_FLAG_REASON: &str = "studio.babel.adminFlag.reason";
const META_SKIP_REQUESTED: &str = "studio.babel.skip.requested";
const META_SUBMIT_FEEDBACK_INCOMPLETE: &str = "studio.babel.submit.hasFeedbackIncomplete";
const META_SUBMIT_FEEDBACK_SAVING: &str = "studio.babel.submit.isFeedbackSaving";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_REVIEW_ACTION_ENVELOPE: &str = "babel.review_action.envelope";

/// Babel codec, review state, and annotation operations for a generic workflow.
pub trait BabelWorkflowExt: Sized {
    fn from_babel_project(project: Project) -> Self;
    fn from_babel_json(source: &str) -> Result<Self>;
    fn export_babel_json(&self) -> Result<String>;
    fn clear_babel_lint_snapshot_metadata(&mut self);
    fn set_babel_lint_snapshot_metadata(&mut self, issues: &[ServerLintIssue]);
    fn set_diff_compare_action(&mut self, action_id: impl Into<String>);
    fn toggle_diff_reviewers(&mut self) -> bool;
    fn record_admin_flag(&mut self, payload: AdminFlagPayload);
    fn mark_skip_requested(&mut self);
    fn mark_claim_dropped(&mut self);
    fn set_feedback_submit_state(&mut self, is_saving: bool, is_incomplete: bool);
    fn mark_review_passed(&mut self);
    fn resolve_low_confidence(&mut self, segment_id: &str) -> Result<()>;
    fn set_segment_annotation_type(
        &mut self,
        segment_id: &str,
        annotation_type: &str,
    ) -> Result<()>;
    fn update_annotation_metadata(
        &mut self,
        segment_id: &str,
        text: impl Into<String>,
        intensity: Option<u8>,
        content: Value,
    ) -> Result<()>;
}

impl BabelWorkflowExt for StudioWorkflow {
    fn from_babel_project(project: Project) -> Self {
        Self::from_project_with_mutation_policy(project, Arc::new(BabelMutationPolicy::default()))
    }

    fn mark_review_passed(&mut self) {
        self.clear_pending_boundary_edit();
        self.project_mut()
            .metadata
            .insert(META_REVIEW_ACTION_DECISION.to_string(), "pass".to_string());
    }

    fn resolve_low_confidence(&mut self, segment_id: &str) -> Result<()> {
        self.apply(EditorCommand::UpdateSegmentMetadata {
            segment_id: segment_id.to_string(),
            text: None,
            entries: BTreeMap::from([("lowConfidenceResolved".to_string(), json!(true))]),
        })
    }

    fn set_segment_annotation_type(
        &mut self,
        segment_id: &str,
        annotation_type: &str,
    ) -> Result<()> {
        self.apply(EditorCommand::UpdateSegmentMetadata {
            segment_id: segment_id.to_string(),
            text: None,
            entries: BTreeMap::from([("type".to_string(), json!(annotation_type))]),
        })
    }

    fn update_annotation_metadata(
        &mut self,
        segment_id: &str,
        text: impl Into<String>,
        intensity: Option<u8>,
        content: Value,
    ) -> Result<()> {
        self.apply(EditorCommand::UpdateSegmentMetadata {
            segment_id: segment_id.to_string(),
            text: Some(text.into()),
            entries: BTreeMap::from([
                ("intensity".to_string(), json!(intensity)),
                ("content".to_string(), content),
            ]),
        })
    }

    fn from_babel_json(source: &str) -> Result<Self> {
        let project = import_json(source).context("failed to import Babel JSON")?;
        Ok(Self::from_babel_project(project))
    }

    fn export_babel_json(&self) -> Result<String> {
        self.clear_pending_boundary_edit();
        let value = export_json(self.project()).context("failed to export Babel JSON")?;
        serde_json::to_string_pretty(&value).context("failed to encode Babel task JSON")
    }

    fn clear_babel_lint_snapshot_metadata(&mut self) {
        update_babel_lint_snapshot_metadata(self.project_mut(), None);
    }

    fn set_babel_lint_snapshot_metadata(&mut self, issues: &[ServerLintIssue]) {
        update_babel_lint_snapshot_metadata(self.project_mut(), Some(issues));
    }

    fn set_diff_compare_action(&mut self, action_id: impl Into<String>) {
        self.project_mut().metadata.insert(
            META_SELECTED_COMPARE_ACTION_ID.to_string(),
            action_id.into(),
        );
    }

    fn toggle_diff_reviewers(&mut self) -> bool {
        let next = !self
            .project()
            .metadata
            .get(META_SHOW_REVIEWERS)
            .is_some_and(|value| value == "true" || value == "1");
        self.project_mut()
            .metadata
            .insert(META_SHOW_REVIEWERS.to_string(), next.to_string());
        next
    }

    fn record_admin_flag(&mut self, payload: AdminFlagPayload) {
        self.project_mut().metadata.insert(
            META_ADMIN_FLAG_RATING.to_string(),
            payload.rating.to_string(),
        );
        if let Some(reason) = payload.rating_reason {
            self.project_mut()
                .metadata
                .insert(META_ADMIN_FLAG_REASON.to_string(), reason);
        } else {
            self.project_mut().metadata.remove(META_ADMIN_FLAG_REASON);
        }
    }

    fn mark_skip_requested(&mut self) {
        self.project_mut()
            .metadata
            .insert(META_SKIP_REQUESTED.to_string(), "true".to_string());
    }

    fn mark_claim_dropped(&mut self) {
        self.project_mut()
            .metadata
            .insert(META_DROP_CLAIM_DROPPED.to_string(), "true".to_string());
    }

    fn set_feedback_submit_state(&mut self, is_saving: bool, is_incomplete: bool) {
        self.project_mut().metadata.insert(
            META_SUBMIT_FEEDBACK_SAVING.to_string(),
            is_saving.to_string(),
        );
        self.project_mut().metadata.insert(
            META_SUBMIT_FEEDBACK_INCOMPLETE.to_string(),
            is_incomplete.to_string(),
        );
    }
}

fn update_babel_lint_snapshot_metadata(project: &mut Project, issues: Option<&[ServerLintIssue]>) {
    update_json_metadata_value(project, META_EXTRA_PACKAGE, |value| {
        let Some(object) = value.as_object_mut() else {
            return;
        };
        object.remove("lintErrors");
        match issues {
            Some(issues) => {
                object.insert("lint_errors".to_string(), server_lint_issues_value(issues));
            }
            None => {
                object.remove("lint_errors");
            }
        }
    });
    update_json_metadata_value(project, META_REVIEW_ACTION_ENVELOPE, |envelope| {
        if let Some(data) = review_action_data_object_mut(envelope) {
            data.remove("lint_errors");
            match issues {
                Some(issues) => {
                    data.insert("lintErrors".to_string(), server_lint_issues_value(issues));
                }
                None => {
                    data.remove("lintErrors");
                }
            }
        }
    });
}

fn update_json_metadata_value(project: &mut Project, key: &str, update: impl FnOnce(&mut Value)) {
    let Some(encoded) = project.metadata.get(key).cloned() else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&encoded) else {
        return;
    };
    update(&mut value);
    if let Ok(encoded) = serde_json::to_string(&value) {
        project.metadata.insert(key.to_string(), encoded);
    }
}

fn review_action_data_object_mut(
    envelope: &mut Value,
) -> Option<&mut serde_json::Map<String, Value>> {
    if envelope.is_array() {
        return envelope
            .as_array_mut()?
            .iter_mut()
            .find_map(review_action_data_object_mut);
    }
    if envelope.pointer("/result/data/json").is_some() {
        return envelope
            .pointer_mut("/result/data/json")
            .and_then(Value::as_object_mut);
    }
    if envelope.pointer("/data/json").is_some() {
        return envelope
            .pointer_mut("/data/json")
            .and_then(Value::as_object_mut);
    }
    envelope.pointer_mut("/json").and_then(Value::as_object_mut)
}

fn server_lint_issues_value(issues: &[ServerLintIssue]) -> Value {
    Value::Array(
        issues
            .iter()
            .map(|issue| {
                json!({
                    "annotationId": issue.annotation_id,
                    "reason": issue.reason,
                    "severity": match issue.severity {
                        LintSeverity::Warning => "warning",
                        LintSeverity::Error | LintSeverity::Pass => "error",
                    },
                })
            })
            .collect(),
    )
}

/// Babel mutation provenance. Installed only by the Babel workflow constructors.
#[derive(Debug)]
pub struct BabelMutationPolicy {
    clock: fn() -> String,
}

impl Default for BabelMutationPolicy {
    fn default() -> Self {
        Self::with_clock(current_babel_timestamp)
    }
}

impl BabelMutationPolicy {
    pub fn with_clock(clock: fn() -> String) -> Self {
        Self { clock }
    }
}

impl SegmentMutationPolicy for BabelMutationPolicy {
    fn updated(&self, segment: &mut Segment) {
        segment
            .metadata
            .insert("lowConfidenceResolved".to_string(), json!(true));
        segment
            .metadata
            .insert("updatedAt".to_string(), json!((self.clock)()));
    }

    fn created(&self, segment: &mut Segment) {
        let timestamp = (self.clock)();
        segment
            .metadata
            .entry("createdAt".to_string())
            .or_insert_with(|| json!(timestamp));
        segment
            .metadata
            .insert("updatedAt".to_string(), json!(timestamp));
    }

    fn split(&self, segment: &mut Segment) {
        let timestamp = (self.clock)();
        segment
            .metadata
            .insert("lowConfidenceResolved".to_string(), json!(true));
        segment
            .metadata
            .insert("createdAt".to_string(), json!(timestamp));
        segment
            .metadata
            .insert("updatedAt".to_string(), json!(timestamp));
    }

    fn split_gap_seconds(&self) -> f64 {
        0.1
    }
}

fn current_babel_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond() / 1_000_000
    )
}

/// Per-document review progress. Only completion produces an undoable project edit.
#[derive(Debug, Default)]
pub struct BabelLowConfidenceCommandProvider {
    acknowledgements: std::sync::Mutex<LowConfidenceAcknowledgements>,
}

#[derive(Debug, Default)]
struct LowConfidenceAcknowledgements {
    revision: Option<u64>,
    words: BTreeMap<String, std::collections::BTreeSet<usize>>,
}

impl exaudio_extension::document::CommandProvider for BabelLowConfidenceCommandProvider {
    fn command_ids(&self) -> &[&str] {
        &["babel.acknowledge-low-confidence-word"]
    }

    fn execute(
        &self,
        command: &str,
        request: &exaudio_extension::document::CommandRequest<'_>,
    ) -> std::result::Result<exaudio_extension::document::CommandOutcome, String> {
        use exaudio_extension::document::{CommandOutcome, DocumentEdit, DocumentTransaction};

        if command != "babel.acknowledge-low-confidence-word" {
            return Err(format!("Unknown Babel review command {command}."));
        }
        let segment_id = request
            .segment_id
            .ok_or_else(|| localized("No segment selected.", "Сегмент не выбран.").to_owned())?;
        let segment = request.project.segment(segment_id).ok_or_else(|| {
            localized(
                "The segment no longer exists.",
                "Сегмент больше не существует.",
            )
            .to_owned()
        })?;
        let word_index = request
            .arguments
            .get("word_index")
            .and_then(Value::as_u64)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| {
                localized(
                    "Choose a low-confidence word.",
                    "Выберите слово с низкой уверенностью.",
                )
                .to_owned()
            })?;
        let reviewable: std::collections::BTreeSet<_> =
            crate::presentation::build_transcript_word_views(segment)
                .into_iter()
                .filter(|word| word.is_low_confidence)
                .map(|word| word.index)
                .collect();
        if !reviewable.contains(&word_index) {
            return Err(localized(
                "That word is no longer marked low-confidence.",
                "Это слово больше не помечено как слово с низкой уверенностью.",
            )
            .into());
        }
        let mut state = self.acknowledgements.lock().map_err(|_| {
            localized(
                "Couldn't record the review. Try again.",
                "Не удалось записать проверку. Попробуйте снова.",
            )
        })?;
        if state.revision != Some(request.revision) {
            state.words.clear();
            state.revision = Some(request.revision);
        }
        let acknowledged = state.words.entry(segment_id.to_string()).or_default();
        acknowledged.insert(word_index);
        let reviewed = acknowledged.len();
        let total = reviewable.len();
        if !reviewable.iter().all(|index| acknowledged.contains(index)) {
            return Ok(CommandOutcome {
                status: Some(localized_fmt(
                    "Reviewed {reviewed} of {total} low-confidence words",
                    "Проверено {reviewed} из {total} слов с низкой уверенностью",
                    &[
                        ("reviewed", &reviewed.to_string()),
                        ("total", &total.to_string()),
                    ],
                )),
                ..Default::default()
            });
        }
        let mut segment = segment.clone();
        segment
            .metadata
            .insert("lowConfidenceResolved".into(), json!(true));
        // Keep acknowledgements until the transaction commits and advances the revision.
        // A rejected transaction must not discard review progress.
        Ok(CommandOutcome {
            transaction: Some(DocumentTransaction {
                label: localized(
                    "Low-confidence words reviewed",
                    "Слова с низкой уверенностью проверены",
                )
                .into(),
                expected_revision: request.revision,
                edits: vec![DocumentEdit::ReplaceSegment { segment }],
            }),
            status: Some(
                localized(
                    "All low-confidence words reviewed",
                    "Все слова с низкой уверенностью проверены",
                )
                .into(),
            ),
            ..Default::default()
        })
    }
}

impl BabelLowConfidenceCommandProvider {
    pub fn is_acknowledged(&self, segment_id: &str, word_index: usize, revision: u64) -> bool {
        self.acknowledgements.lock().is_ok_and(|state| {
            state.revision == Some(revision)
                && state
                    .words
                    .get(segment_id)
                    .is_some_and(|words| words.contains(&word_index))
        })
    }
}
