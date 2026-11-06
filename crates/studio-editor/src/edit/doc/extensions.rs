//! Revision-checked extension commands and transactions; no provider is implicit.
use std::borrow::Cow;

use exaudio_extension::document::{
    CommandRequest, DocumentEdit, DocumentTransaction, QuickFix, TextRange,
};
use studio_core::{Project, Segment};

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::draft::{TextSelection, byte_offset};

impl EditorDoc {
    /// Opens the contributed editor for non-text rows, or focuses the native text editor.
    pub fn open_segment_editor(&mut self, segment_id: &str) -> EditResult {
        self.guard("Edit segment")?;
        let segment = self
            .project()
            .segment(segment_id)
            .ok_or_else(|| EditError::SegmentNotFound(segment_id.into()))?;
        if !self.environment().is_text_segment(segment) {
            let command = self
                .environment()
                .policy
                .segment_edit_command
                .as_ref()
                .ok_or_else(|| {
                    EditError::Unavailable("No editor is registered for this segment.".into())
                })?;
            return Ok(Outcome::none().with(Effect::ExecuteCommand {
                command: command.clone(),
                arguments: serde_json::json!({ "segment_id": segment_id }),
            }));
        }
        let caret = self
            .drafts
            .get(segment_id)
            .map_or(0, |draft| draft.selection().head);
        Ok(self
            .select_segment_without_seek(segment_id)
            .with(Effect::FocusText {
                segment_id: segment_id.into(),
                caret,
            }))
    }

    pub fn execute_command(&mut self, command: &str, arguments: &serde_json::Value) -> EditResult {
        self.execute_command_for_segment(command, arguments, None)
    }

    pub fn execute_command_for_segment(
        &mut self,
        command: &str,
        arguments: &serde_json::Value,
        segment_id: Option<&str>,
    ) -> EditResult {
        self.guard("Extension command")?;
        let provider = self
            .environment()
            .providers
            .commands
            .iter()
            .find(|provider| provider.command_ids().contains(&command))
            .cloned()
            .ok_or_else(|| {
                EditError::Unavailable(format!(
                    "Command `{command}` is not registered for this document."
                ))
            })?;
        let segment_id = segment_id
            .map(str::to_owned)
            .or_else(|| self.helper.focused_segment_id.clone())
            .or_else(|| self.session.selected_segment_id.clone());
        let draft = segment_id.as_deref().and_then(|id| self.drafts.get(id));
        let selection = draft.map(|draft| {
            let range = draft.selection().range();
            TextRange {
                start: byte_offset(draft.text(), range.start),
                end: byte_offset(draft.text(), range.end),
            }
        });
        let mut project = Cow::Borrowed(self.project());
        if self.has_pending_drafts() {
            let snapshot = project.to_mut();
            for segment in &mut snapshot.segments {
                if let Some(draft) = self.drafts.get(&segment.id) {
                    draft.text().clone_into(&mut segment.text);
                }
            }
        }
        let request = CommandRequest {
            project: &project,
            revision: self.session.current_edit_revision(),
            segment_id: segment_id.as_deref(),
            text: draft.map(|draft| draft.text()),
            selection,
            caret_offset: draft.map(|draft| byte_offset(draft.text(), draft.selection().head)),
            arguments,
        };
        let result = provider
            .execute(command, &request)
            .map_err(EditError::Unavailable)?;
        if result.transaction.is_some() && !result.drafts.is_empty() {
            return Err(EditError::Unavailable(
                "A command must return either a document transaction or draft edits, not both."
                    .into(),
            ));
        }
        // Validate all draft outputs before modifying any state.
        for replacement in &result.drafts {
            if self.drafts.get(&replacement.segment_id).is_none() {
                return Err(EditError::NoDraft(replacement.segment_id.clone()));
            }
            if let Some(range) = replacement.selection {
                validate_range(&replacement.text, range).map_err(EditError::Unavailable)?;
            }
        }
        let mut outcome = if let Some(transaction) = result.transaction {
            self.apply_transaction(transaction)?
        } else {
            Outcome::none()
        };
        let mut changed = false;
        for replacement in result.drafts {
            let draft = self
                .drafts
                .get_mut(&replacement.segment_id)
                .ok_or_else(|| EditError::NoDraft(replacement.segment_id.clone()))?;
            let selection = match replacement.selection {
                Some(range) => TextSelection::new(
                    replacement.text[..range.start].chars().count(),
                    replacement.text[..range.end].chars().count(),
                ),
                None => draft.selection(),
            };
            if draft.edit(replacement.text, selection) {
                changed = true;
                self.clear_tag_ui_for(&replacement.segment_id);
                outcome.effects.push(Effect::DraftChanged {
                    segment_id: replacement.segment_id,
                });
            }
        }
        if changed {
            self.mark_dirty();
        }
        // A provider may change its decorations without changing document contents.
        self.touch();
        outcome.status = result.status.or(outcome.status);
        Ok(outcome)
    }

    pub fn apply_quick_fix(&mut self, fix: &QuickFix) -> EditResult {
        self.apply_transaction(DocumentTransaction {
            label: fix.title.clone(),
            expected_revision: fix.expected_revision,
            edits: fix.edits.clone(),
        })
    }

    /// Applies all edits atomically as one undo unit, rejecting stale provider results.
    pub fn apply_transaction(&mut self, transaction: DocumentTransaction) -> EditResult {
        self.apply_transaction_guarded(transaction, || Ok(()))
    }

    /// Checks cancellation/authorization after staging and before the authoritative commit.
    pub fn apply_transaction_guarded(
        &mut self,
        transaction: DocumentTransaction,
        commit_guard: impl FnOnce() -> Result<(), EditError>,
    ) -> EditResult {
        self.guard("Extension transaction")?;
        if transaction.expected_revision != self.session.current_edit_revision() {
            return Err(EditError::Unavailable(
                "The document changed before the extension transaction was applied.".into(),
            ));
        }
        if transaction.edits.is_empty() {
            commit_guard()?;
            return Ok(Outcome::none());
        }
        if self.has_pending_drafts() {
            return Err(EditError::PendingDrafts {
                action: "Extension transaction",
            });
        }
        let mutation_policy = self.workflow.mutation_policy();
        self.workflow
            .apply_project_transaction(&transaction.label, |project| {
                for edit in transaction.edits {
                    match edit {
                        DocumentEdit::ReplaceText {
                            segment_id,
                            range,
                            text,
                        } => {
                            let segment = project.segment_mut(&segment_id).ok_or_else(|| {
                                anyhow::anyhow!("Unknown segment `{segment_id}`.")
                            })?;
                            if let Some(range) = range {
                                validate_range(&segment.text, range).map_err(anyhow::Error::msg)?;
                                segment.text.replace_range(range.start..range.end, &text);
                            } else {
                                segment.text = text;
                            }
                            mutation_policy.updated(segment);
                        }
                        DocumentEdit::InsertSegment { mut segment } => {
                            if project.has_segment(&segment.id) {
                                anyhow::bail!("Duplicate segment `{}`.", segment.id);
                            }
                            validate_segment(project, &segment)?;
                            mutation_policy.created(&mut segment);
                            project.segments.push(segment);
                        }
                        DocumentEdit::RemoveSegment { segment_id } => {
                            let index = project
                                .segments
                                .iter()
                                .position(|segment| segment.id == segment_id)
                                .ok_or_else(|| {
                                    anyhow::anyhow!("Unknown segment `{segment_id}`.")
                                })?;
                            project.segments.remove(index);
                        }
                        DocumentEdit::ReplaceSegment { mut segment } => {
                            validate_segment(project, &segment)?;
                            mutation_policy.updated(&mut segment);
                            let current = project.segment_mut(&segment.id).ok_or_else(|| {
                                anyhow::anyhow!("Unknown segment `{}`.", segment.id)
                            })?;
                            *current = segment;
                        }
                    }
                }
                commit_guard().map_err(anyhow::Error::new)
            })
            .map_err(|source| EditError::Workflow {
                action: "Extension transaction",
                source,
            })?;
        self.drafts.reset(self.workflow.project());
        self.tag_ui = None;
        self.mark_dirty();
        self.reconcile(true);
        Ok(Outcome::status(transaction.label))
    }
}

fn validate_range(text: &str, range: TextRange) -> Result<(), String> {
    if range.start > range.end
        || range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return Err("Extension returned an invalid UTF-8 text range.".into());
    }
    Ok(())
}

fn validate_segment(project: &Project, segment: &Segment) -> anyhow::Result<()> {
    if segment.id.is_empty()
        || !segment.start_seconds.is_finite()
        || !segment.end_seconds.is_finite()
        || segment.start_seconds < 0.0
        || segment.end_seconds <= segment.start_seconds
    {
        anyhow::bail!("Extension returned an invalid segment identity or interval.");
    }
    if !project.lanes.iter().any(|lane| lane.id == segment.lane_id)
        || !project
            .speakers
            .iter()
            .any(|speaker| speaker.id == segment.speaker_id)
    {
        anyhow::bail!("Extension returned an unknown lane or speaker.");
    }
    Ok(())
}
