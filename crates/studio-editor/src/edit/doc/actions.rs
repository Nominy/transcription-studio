//! Dispatch of keymap actions to document operations.

use super::CompletionKey;
use super::{EditResult, EditorDoc, Outcome};
use crate::edit::keymap::EditorAction;
use crate::workflow::StudioWorkflow;

impl EditorDoc {
    /// Performs a keyboard action. `focused_segment` is the row whose text editor has focus
    /// (required by text-context actions). Returns `None` for app-level actions the UI owns
    /// (back to projects, drafting transcription, swapping split panes, default action when no
    /// doc dialog is open) and for text actions without a focused row.
    pub fn perform(
        &mut self,
        action: EditorAction,
        focused_segment: Option<&str>,
    ) -> Option<EditResult> {
        use EditorAction as A;
        let ok = |outcome: Outcome| Some(Ok(outcome));
        match action {
            A::PlayPause => ok(self.helper_toggle_playback()),
            A::PreviousSegment => ok(self.select_adjacent_segment(-1)),
            A::NextSegment => ok(self.select_adjacent_segment(1)),
            A::FocusPreviousSegment => Some(self.focus_adjacent_segment(-1)),
            A::FocusNextSegment => Some(self.focus_adjacent_segment(1)),
            A::Undo => Some(self.undo()),
            A::Redo => Some(self.redo()),
            A::OpenFind => ok(self.open_find()),
            A::FindNext => ok(self.find_next()),
            A::FindPrevious => ok(self.find_previous()),
            A::DeleteSelectedSegment => Some(self.delete_selected()),
            A::ToggleOverlay => {
                self.toggle_overlay();
                ok(Outcome::none())
            }
            A::SwitchGhostCursorLane => Some(self.switch_ghost_cursor_lane(focused_segment)),
            A::CreateRegionSubmit => self
                .create_region
                .is_some()
                .then(|| self.submit_create_region()),
            A::CreateRegionEscape => {
                if self.find.open {
                    self.close_find();
                } else {
                    self.reset_create_region_time();
                }
                ok(Outcome::none())
            }
            A::ResetSpeakerWorkflow => ok(self.reset_speaker_workflow()),
            A::SwitchSpeakerWorkflowLane1 => Some(self.switch_speaker_workflow_lane("Speaker 1")),
            A::SwitchSpeakerWorkflowLane2 => Some(self.switch_speaker_workflow_lane("Speaker 2")),
            A::RewindPlaybackOneSecond => ok(self.rewind_one_second()),
            A::MoveTextBeforeCaretToPreviousSegment => {
                focused_segment.map(|id| self.move_text_to_previous_row(id))
            }
            A::MoveTextAfterCaretToNextSegment => {
                focused_segment.map(|id| self.move_text_to_next_row(id))
            }
            A::IncreasePlaybackSpeed => ok(self.step_speed(1)),
            A::DecreasePlaybackSpeed => ok(self.step_speed(-1)),
            A::MergeSelectedSegmentAbove => Some(self.merge_selected_above()),
            A::MergeSelectedSegmentBelow => Some(self.merge_selected_below()),
            A::TrimCurrentSegmentToAudio => Some(self.trim_current_segment_to_audio()),
            A::TrimAllVisibleSegmentsToAudio => Some(self.trim_visible_segments_to_audio()),
            A::AutoInsertSegmentAtCaret => Some(self.auto_insert_segment_at_playhead()),
            A::AutoSegmentVisibleSilences => Some(self.auto_segment_visible_silences()),
            A::SplitAtPlayhead => Some(self.split_selected_at_playhead()),
            A::CommitCutPreviewSmartSplit => Some(self.commit_cut_preview(true)),
            A::CommitCutPreviewPlainSplit => Some(self.commit_cut_preview(false)),
            A::LoopCutPreviewOverlay => Some(self.loop_cut_preview()),
            A::EscapePressed => ok(self.escape()),
            A::ActivateDefaultSurfaceAction => {
                if self.create_region.is_some() {
                    Some(self.submit_create_region())
                } else if self.find.open {
                    ok(self.find_next())
                } else {
                    None
                }
            }
            A::TextUndo => focused_segment.map(|id| self.undo_text(id)),
            A::TextRedo => focused_segment.map(|id| self.redo_text(id)),
            A::TranscriptEscape => focused_segment.map(|id| self.transcript_escape(id)),
            A::CompletionPrevious => {
                focused_segment.map(|id| self.completion_key(id, CompletionKey::Previous))
            }
            A::CompletionNext => {
                focused_segment.map(|id| self.completion_key(id, CompletionKey::Next))
            }
            A::CompletionAccept => {
                focused_segment.map(|id| self.completion_key(id, CompletionKey::Accept))
            }
            A::CompletionDismiss => {
                focused_segment.map(|id| self.completion_key(id, CompletionKey::Dismiss))
            }
            A::BackToProjects | A::SwapSplitPanes => None,
        }
    }

    /// Applies an edit made outside the doc's own operations (drafting/reviewer suggestions)
    /// as an undoable, read-only-guarded edit: rows whose drafts matched the project follow the
    /// change (drafts with unapplied edits are kept), then marks dirty and reconciles.
    pub fn edit_workflow<T>(
        &mut self,
        action: &'static str,
        change: impl FnOnce(&mut StudioWorkflow) -> anyhow::Result<T>,
    ) -> Result<T, super::EditError> {
        let clean: Vec<String> = self
            .workflow
            .project()
            .segments
            .iter()
            .filter(|segment| matches!(self.drafts.differs(segment), Ok(false)))
            .map(|segment| segment.id.clone())
            .collect();
        let value = self.apply(action, change)?;
        let project = self.workflow.project();
        for id in &clean {
            if project
                .segment(id)
                .is_some_and(|segment| !matches!(self.drafts.differs(segment), Ok(false)))
            {
                self.drafts.refresh(project, id);
            }
        }
        self.mark_dirty();
        self.reconcile(true);
        Ok(value)
    }
}
