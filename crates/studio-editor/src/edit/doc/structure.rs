//! Structural edits: split, merge, delete, insert rows, annotation type, cut-preview commit and
//! loop, decoded-audio trimming, auto-insert and silence auto-segmentation.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::silence::{
    AudioTrimPlan, CUT_PREVIEW_MIN_SECONDS, audio_trim_plan_for_segment,
    auto_insert_plans_for_caret, auto_segment_split_plans, cut_preview_lane_id,
    cut_preview_scroll_target, merge_auto_insert_adjacent_segments, visible_transcription_segments,
};
use crate::edit::structure::{merge_neighbor_segment_id, split_ratio, split_text_by_word_ratio};
use studio_core::SegmentUpdate;

impl EditorDoc {
    /// Splits a row at `at_seconds`, allocating its (draft) words by time ratio; selects the
    /// new right-hand row. Rejects points outside the row or within 100ms of its end.
    pub fn split_segment_at(&mut self, segment_id: &str, at_seconds: f64) -> EditResult {
        self.guard("Split")?;
        let segment = self.segment(segment_id)?;
        let ratio = split_ratio(&segment, at_seconds)?;
        let text = self.drafts.text_or(&segment).to_string();
        let (left, right) = split_text_by_word_ratio(&text, ratio);
        let created = self.apply("Split", |workflow| {
            workflow.split_segment_with_text(segment_id, at_seconds, left, right)
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.mark_dirty();
        self.reconcile(true);
        Ok(self
            .select_segment_without_seek(&created)
            .with_status(format!(
                "Split `{segment_id}` at {at_seconds:.3}s into `{created}`."
            ))
            .with(Effect::DraftChanged {
                segment_id: segment_id.to_string(),
            }))
    }

    /// Merges a row into the previous same-speaker row on its lane (pending text drafts of both
    /// rows are applied first).
    pub fn merge_above(&mut self, segment_id: &str) -> EditResult {
        self.merge(segment_id, true)
    }

    /// Merges the next same-speaker row on the lane into this row.
    pub fn merge_below(&mut self, segment_id: &str) -> EditResult {
        self.merge(segment_id, false)
    }

    pub fn merge_selected_above(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id("Merge")?;
        self.merge(&segment_id, true)
    }

    pub fn merge_selected_below(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id("Merge")?;
        self.merge(&segment_id, false)
    }

    fn merge(&mut self, segment_id: &str, previous: bool) -> EditResult {
        self.session.open_actions_segment_id = None;
        self.guard("Merge")?;
        let pending: Vec<SegmentUpdate> =
            merge_neighbor_segment_id(self.workflow.project(), segment_id, previous)
                .map(|neighbor| self.pending_text_updates(&[segment_id, &neighbor]))
                .unwrap_or_default();
        self.low_confidence_acks.remove(segment_id);
        self.apply("Merge", |workflow| {
            if !pending.is_empty() {
                workflow.apply_segment_edits(pending)?;
            }
            if previous {
                workflow.merge_with_previous(segment_id)
            } else {
                workflow.merge_with_next(segment_id)
            }
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(if previous {
            format!("Merged `{segment_id}` into the previous segment on its track.")
        } else {
            format!("Merged `{segment_id}` with the next segment on its track.")
        }))
    }

    fn pending_text_updates(&self, segment_ids: &[&str]) -> Vec<SegmentUpdate> {
        let project = self.workflow.project();
        segment_ids
            .iter()
            .filter_map(|id| {
                let segment = project.segment(id)?;
                let draft = self.drafts.get(id)?;
                (draft.text() != segment.text).then(|| SegmentUpdate {
                    segment_id: segment.id.clone(),
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                    text: draft.text().to_string(),
                })
            })
            .collect()
    }

    pub fn delete_segment(&mut self, segment_id: &str) -> EditResult {
        self.session.open_actions_segment_id = None;
        self.guard("Delete")?;
        self.low_confidence_acks.remove(segment_id);
        self.apply("Delete", |workflow| workflow.delete_segment(segment_id))?;
        self.drafts.remove(segment_id);
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(format!("Deleted `{segment_id}`.")))
    }

    /// D outside text inputs.
    pub fn delete_selected(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id("Delete")?;
        self.delete_segment(&segment_id)
    }

    /// Inserts an empty row in the gap before `segment_id` on its lane and selects it.
    pub fn create_segment_before(&mut self, segment_id: &str) -> EditResult {
        self.create_adjacent(segment_id, true)
    }

    /// Inserts an empty row in the gap after `segment_id` on its lane and selects it.
    pub fn create_segment_after(&mut self, segment_id: &str) -> EditResult {
        self.create_adjacent(segment_id, false)
    }

    fn create_adjacent(&mut self, segment_id: &str, before: bool) -> EditResult {
        self.session.open_actions_segment_id = None;
        let created = self.apply("Create", |workflow| {
            if before {
                workflow.create_segment_before(segment_id)
            } else {
                workflow.create_segment_after(segment_id)
            }
        })?;
        self.mark_dirty();
        self.reconcile(true);
        let side = if before { "before" } else { "after" };
        Ok(self
            .select_segment_without_seek(&created)
            .with_status(format!("Created `{created}` {side} `{segment_id}`.")))
    }

    /// Changes a row's Babel annotation type (`"transcription"`, `"background_noise"`, ...).
    pub fn set_segment_annotation_type(
        &mut self,
        segment_id: &str,
        annotation_type: &str,
    ) -> EditResult {
        self.apply("Annotation type change", |workflow| {
            workflow.set_segment_annotation_type(segment_id, annotation_type)
        })?;
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(format!(
            "Set `{segment_id}` annotation type to `{annotation_type}`."
        )))
    }

    // ---- cut preview ------------------------------------------------------------------

    /// S / Shift+S: cuts the overlay range out of the selected (or first overlapping) lane,
    /// optionally re-splitting text by time ratio; clears the overlay and selects the row after
    /// the cut.
    pub fn commit_cut_preview(&mut self, smart_split: bool) -> EditResult {
        self.guard("Cut preview")?;
        let range = self.session.overlay_range.ok_or_else(|| {
            EditError::Unavailable("Cut preview unavailable: no active overlay range.".into())
        })?;
        if range.duration_seconds() < CUT_PREVIEW_MIN_SECONDS {
            return Err(EditError::Unavailable(
                "Cut preview must be at least 1.00s.".into(),
            ));
        }
        let project = self.workflow.project();
        let lane_id =
            cut_preview_lane_id(project, Some(&self.session), range).ok_or_else(|| {
                EditError::Unavailable(
                    "Cut preview unavailable: no selected or overlapping lane.".into(),
                )
            })?;
        if !project.segments.iter().any(|segment| {
            segment.lane_id == lane_id
                && segment.end_seconds > range.start_seconds
                && segment.start_seconds < range.end_seconds
        }) {
            return Err(EditError::Unavailable(
                "Cut preview found no overlapping segment on the target lane.".into(),
            ));
        }
        self.apply("Cut preview", |workflow| {
            workflow.cut_lane_range(
                &lane_id,
                range.start_seconds,
                range.end_seconds,
                smart_split,
            )
        })?;
        self.mark_dirty();
        self.reconcile(true);
        self.session.overlay_range = None;
        let status = format!(
            "Cut {:.3}s-{:.3}s on `{lane_id}`{}.",
            range.start_seconds,
            range.end_seconds,
            if smart_split { " with smart split" } else { "" }
        );
        Ok(
            match cut_preview_scroll_target(self.workflow.project(), &lane_id, range) {
                Some(target) => self
                    .select_segment_without_seek(&target)
                    .with_status(status),
                None => Outcome::status(status),
            },
        )
    }

    /// L with an overlay: loops the overlay range (no project edit).
    pub fn loop_cut_preview(&mut self) -> EditResult {
        let range = self.session.overlay_range.ok_or_else(|| {
            EditError::Unavailable("Selection loop unavailable: no active overlay range.".into())
        })?;
        self.session.set_loop_range_from_drag(
            self.workflow.project(),
            range.start_seconds,
            range.end_seconds,
        );
        Ok(Outcome::status(format!(
            "Looping cut preview {:.3}s-{:.3}s.",
            range.start_seconds, range.end_seconds
        ))
        .extend(self.restart_if_playing()))
    }

    // ---- decoded-audio timing ---------------------------------------------------------

    /// Alt+R: trims the selected row to nearby speech in its lane's decoded audio.
    pub fn trim_current_segment_to_audio(&mut self) -> EditResult {
        self.guard("Audio trim")?;
        let segment_id = self.selected_segment_id("Audio trim")?;
        let segment = self.segment(&segment_id)?;
        let plan =
            match audio_trim_plan_for_segment(self.workflow.project(), &self.session, &segment) {
                Some(Ok(plan)) => plan,
                Some(Err(reason)) => {
                    return Err(EditError::Unavailable(format!(
                        "Audio trim skipped `{segment_id}`: {reason}."
                    )));
                }
                None => {
                    return Ok(Outcome::status(format!(
                        "Audio trim made no timestamp changes for `{segment_id}`."
                    )));
                }
            };
        self.apply_trim_plans(vec![plan], "Trim current segment")?;
        Ok(Outcome::status(format!(
            "Trimmed `{segment_id}` to nearby decoded audio."
        )))
    }

    /// Alt+Shift+R: trims every visible transcription row with decoded audio.
    pub fn trim_visible_segments_to_audio(&mut self) -> EditResult {
        self.guard("Audio trim")?;
        let plans = self.visible_trim_plans();
        if plans.is_empty() {
            return Err(EditError::Unavailable(
                "Audio trim found no visible segments with decoded speech.".into(),
            ));
        }
        let count = plans.len();
        self.apply_trim_plans(plans, "Trim visible segments")?;
        Ok(Outcome::status(format!(
            "Trimmed {count} visible segments to nearby decoded audio."
        )))
    }

    fn visible_trim_plans(&self) -> Vec<AudioTrimPlan> {
        let project = self.workflow.project();
        visible_transcription_segments(
            project,
            &self.session.track_filter,
            &self.session.collapsed_lane_ids,
        )
        .into_iter()
        .filter_map(|segment| audio_trim_plan_for_segment(project, &self.session, segment)?.ok())
        .collect()
    }

    fn apply_trim_plans(
        &mut self,
        plans: Vec<AudioTrimPlan>,
        action: &'static str,
    ) -> Result<(), EditError> {
        let ids: Vec<String> = plans.iter().map(|plan| plan.segment_id.clone()).collect();
        let updates: Vec<SegmentUpdate> = plans.into_iter().map(AudioTrimPlan::update).collect();
        self.apply(action, |workflow| workflow.apply_segment_edits(updates))?;
        for id in &ids {
            self.drafts.refresh(self.workflow.project(), id);
        }
        self.mark_dirty();
        self.reconcile(true);
        Ok(())
    }

    /// Alt+C: creates an empty row around the nearest uncovered speech within 1s of the
    /// playhead on each visible lane, merging with same-speaker rows closer than 1s.
    pub fn auto_insert_segment_at_playhead(&mut self) -> EditResult {
        self.guard("Auto insert segment")?;
        let plans = auto_insert_plans_for_caret(self.workflow.project(), &self.session);
        if plans.is_empty() {
            return Err(EditError::Unavailable(
                "Auto-insert found no uncovered speech near caret.".into(),
            ));
        }
        let mut created = Vec::new();
        let mut merges = 0;
        self.apply("Auto insert segment", |workflow| {
            for plan in &plans {
                let id = workflow.create_segment_in_lane(
                    &plan.lane_id,
                    plan.start_seconds,
                    plan.end_seconds,
                )?;
                merges += merge_auto_insert_adjacent_segments(workflow, &id)?;
                created.push(id);
            }
            Ok(())
        })?;
        self.mark_dirty();
        self.reconcile(true);
        for id in &created {
            self.drafts.refresh(self.workflow.project(), id);
        }
        let status = format!(
            "Auto-inserted {} segment(s) around nearest uncovered speech, merged {merges} adjacent row(s).",
            created.len()
        );
        Ok(
            match created
                .last()
                .filter(|id| self.workflow.project().has_segment(id))
            {
                Some(last) => {
                    let last = last.clone();
                    self.select_segment_without_seek(&last).with_status(status)
                }
                None => Outcome::status(status),
            },
        )
    }

    /// Alt+Shift+S: splits visible rows in the middle of decoded silences of 1s or more, then
    /// trims all visible rows to audio.
    pub fn auto_segment_visible_silences(&mut self) -> EditResult {
        self.guard("Auto segmentation")?;
        let mut plans = auto_segment_split_plans(self.workflow.project(), &self.session);
        if plans.is_empty() {
            return Err(EditError::Unavailable(
                "Auto-segmentation found no visible silence runs over 1000ms.".into(),
            ));
        }
        // Split later points first so earlier split ids stay valid.
        plans.sort_by(|left, right| {
            left.segment_id
                .cmp(&right.segment_id)
                .then_with(|| right.split_seconds.total_cmp(&left.split_seconds))
        });
        let count = plans.len();
        let mut created = Vec::new();
        self.apply("Auto segmentation", |workflow| {
            for plan in &plans {
                created.push(
                    workflow.split_segment_with_project_text_ratio(
                        &plan.segment_id,
                        plan.split_seconds,
                    )?,
                );
            }
            Ok(())
        })?;
        self.mark_dirty();
        self.reconcile(true);
        let plans = self.visible_trim_plans();
        if !plans.is_empty() {
            self.apply_trim_plans(plans, "Trim visible segments")?;
        }
        for id in &created {
            self.drafts.refresh(self.workflow.project(), id);
        }
        let status = format!("Auto-segmented {count} silence run(s) over 1000ms and trimmed all.");
        Ok(match created.last() {
            Some(last) => {
                let last = last.clone();
                self.select_segment_without_seek(&last).with_status(status)
            }
            None => Outcome::status(status),
        })
    }
}
