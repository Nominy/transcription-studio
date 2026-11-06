//! Structural edits: split, merge, delete, insert rows, annotation type, cut-preview commit and
//! loop, decoded-audio trimming, auto-insert and silence auto-segmentation.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::silence::{
    AudioTrimPlan, AutoSegmentMergePlan, audio_trim_plan_for_segment, auto_insert_plans_for_caret,
    auto_segment_merge_plans, auto_segment_split_plans, cut_preview_scroll_target,
    merge_auto_insert_adjacent_segments, nearest_same_lane_segment, segment_is_silent,
    visible_transcription_segments,
};
use crate::edit::structure::{
    CUT_PREVIEW_MIN_DURATION_SECONDS, SplitError, merge_neighbor_segment_id, split_ratio,
    split_text_by_word_ratio,
};
use crate::edit::text::join_segment_text;
use crate::edit::text_placement::{baseline_groups, place_group_text};
use crate::timing::{SplitSource, TimeRange, smart_split_text};
use studio_core::SegmentUpdate;

/// Smart splits must leave this much of the row on either side (Helper's split edge guard).
pub const SMART_SPLIT_EDGE_GUARD_SECONDS: f64 = 0.05;

/// Helper's cap on merge attempts in one auto-segmentation run.
const AUTO_SEGMENT_MERGE_ATTEMPTS: usize = 200;

/// What the silent-row phase of auto-segmentation did.
#[derive(Debug, Default)]
struct SilentCleanup {
    removed: usize,
    /// Silent rows left in place (text with no same-lane row to take it, or a refused edit).
    kept: usize,
    /// Rows that took a removed row's text.
    stitched: Vec<String>,
}

impl EditorDoc {
    /// Ctrl+Click: splits a row at `at_seconds`, allocating its (draft) words by time ratio;
    /// selects the new right-hand row. Rejects points outside the row or within 100ms of its
    /// end.
    pub fn split_segment_at(&mut self, segment_id: &str, at_seconds: f64) -> EditResult {
        self.guard("Split")?;
        let segment = self.segment(segment_id)?;
        let ratio = split_ratio(&segment, at_seconds)?;
        let text = self.drafts.text_or(&segment).to_string();
        let (left, right) = split_text_by_word_ratio(&text, ratio);
        self.split_with_texts("Split", segment_id, at_seconds, left, right, "")
    }

    /// `S` outside a cut selection: splits the selected row at the playhead (rejected when the
    /// playhead is outside it, as [`Self::split_segment_at`] does).
    pub fn split_selected_at_playhead(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id("Split")?;
        let at_seconds = self.session.playhead_seconds;
        self.split_segment_at(&segment_id, at_seconds)
    }

    /// Shift+Ctrl+Click: splits a row and allocates its (draft) text with the lane's word
    /// timings — the exact completed-word boundary, else the interpolated timing offset, else
    /// the word ratio (Helper smart split).
    pub fn smart_split_segment_at(&mut self, segment_id: &str, at_seconds: f64) -> EditResult {
        self.guard("Smart split")?;
        let segment = self.segment(segment_id)?;
        split_ratio(&segment, at_seconds)?;
        if at_seconds <= segment.start_seconds + SMART_SPLIT_EDGE_GUARD_SECONDS {
            return Err(SplitError::OutsideSegment.into());
        }
        let text = self.drafts.text_or(&segment).to_string();
        let parts = smart_split_text(
            &text,
            self.word_timings.lane(&segment.lane_id),
            TimeRange::new(segment.start_seconds, segment.end_seconds),
            at_seconds,
        )
        .ok_or(EditError::Split(SplitError::OutsideSegment))?;
        let note = split_source_note(parts.source);
        self.split_with_texts(
            "Smart split",
            segment_id,
            at_seconds,
            parts.first,
            parts.second,
            note,
        )
    }

    fn split_with_texts(
        &mut self,
        action: &'static str,
        segment_id: &str,
        at_seconds: f64,
        left: String,
        right: String,
        note: &str,
    ) -> EditResult {
        let created = self.apply(action, |workflow| {
            workflow.split_segment_with_text(segment_id, at_seconds, left, right)
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.mark_dirty();
        self.reconcile(true);
        let outcome = self.select_segment_without_seek(&created);
        let status = format!(
            "Split {} at {at_seconds:.3}s into {}{note}.",
            self.segment_label(segment_id),
            self.segment_label(&created)
        );
        Ok(outcome.with_status(status).with(Effect::DraftChanged {
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
        let label = self.segment_label(segment_id);
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
            format!("Merged {label} into the previous segment on its track.")
        } else {
            format!("Merged {label} with the next segment on its track.")
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
        let label = self.segment_label(segment_id);
        self.apply("Delete", |workflow| workflow.delete_segment(segment_id))?;
        self.drafts.remove(segment_id);
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(format!("Deleted {label}.")))
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
        let outcome = self.select_segment_without_seek(&created);
        let status = format!(
            "Created {} {side} {}.",
            self.segment_label(&created),
            self.segment_label(segment_id)
        );
        Ok(outcome.with_status(status))
    }

    // ---- cut preview ------------------------------------------------------------------

    /// Shared by cut validation and the selection's eligibility indicator.
    pub fn minimum_cut_duration_seconds(&self) -> f64 {
        (self.environment().policy.minimum_cut_milliseconds as f64 / 1_000.0)
            .max(CUT_PREVIEW_MIN_DURATION_SECONDS)
    }

    /// S / Shift+S: cuts the overlay range out of the lane the cut preview was drawn on; with
    /// `smart_split` a row covering the whole range gets its text split at the range midpoint
    /// (word timings first, like Ctrl+Shift+Click); clears the overlay and selects the row
    /// after the cut.
    pub fn commit_cut_preview(&mut self, smart_split: bool) -> EditResult {
        self.guard("Cut preview")?;
        let (lane_id, range) = self
            .session
            .overlay
            .as_ref()
            .map(|overlay| (overlay.lane_id.clone(), overlay.range))
            .ok_or_else(|| {
                EditError::Unavailable("Cut preview unavailable: no active overlay range.".into())
            })?;
        let minimum = self.minimum_cut_duration_seconds();
        if range.duration_seconds() < minimum {
            return Err(EditError::Unavailable(format!(
                "Cut needs a selection of at least {minimum} seconds; shorter selections can still loop."
            )));
        }
        let project = self.workflow.project();
        let affected: Vec<String> = project
            .segments
            .iter()
            .filter(|segment| {
                segment.lane_id == lane_id
                    && segment.end_seconds > range.start_seconds
                    && segment.start_seconds < range.end_seconds
            })
            .map(|segment| segment.id.clone())
            .collect();
        if affected.is_empty() {
            return Err(EditError::Unavailable(
                "Cut preview found no overlapping segment on the target lane.".into(),
            ));
        }
        let split = smart_split
            .then(|| {
                project.segments.iter().find(|segment| {
                    segment.lane_id == lane_id
                        && segment.start_seconds < range.start_seconds
                        && segment.end_seconds > range.end_seconds
                })
            })
            .flatten()
            .and_then(|segment| {
                smart_split_text(
                    self.drafts.text_or(segment),
                    self.word_timings.lane(&lane_id),
                    TimeRange::new(segment.start_seconds, segment.end_seconds),
                    (range.start_seconds + range.end_seconds) / 2.0,
                )
                .map(|parts| (segment.id.clone(), parts))
            });
        let note = split
            .as_ref()
            .map_or("", |(_, parts)| split_source_note(parts.source));
        let split_source = split.as_ref().map(|(id, _)| id.clone());
        let split_texts = split.map(|(_, parts)| (parts.first, parts.second));
        self.apply("Cut preview", |workflow| {
            workflow.cut_lane_range(
                &lane_id,
                range.start_seconds,
                range.end_seconds,
                split_texts,
            )
        })?;
        // The cut moved the rows' bounds (and split the source's text): their drafts follow,
        // keeping text typed into trimmed rows, so nothing is left pending and Undo works.
        let mut effects = Vec::new();
        for segment_id in &affected {
            match self.workflow.project().segment(segment_id).cloned() {
                Some(segment) if split_source.as_deref() != Some(segment_id.as_str()) => {
                    if let Some(draft) = self.drafts.get_mut(segment_id) {
                        draft.replace_times(segment.start_seconds, segment.end_seconds);
                    }
                }
                _ => self.drafts.refresh(self.workflow.project(), segment_id),
            }
            effects.push(Effect::DraftChanged {
                segment_id: segment_id.clone(),
            });
        }
        self.mark_dirty();
        self.reconcile(true);
        self.session.overlay = None;
        let status = format!(
            "Cut {:.3}s-{:.3}s on {}{}{note}.",
            range.start_seconds,
            range.end_seconds,
            self.lane_label(&lane_id),
            if smart_split { " with smart split" } else { "" }
        );
        let outcome = match cut_preview_scroll_target(self.workflow.project(), &lane_id, range) {
            Some(target) => self
                .select_segment_without_seek(&target)
                .with_status(status),
            None => Outcome::status(status),
        };
        Ok(effects.into_iter().fold(outcome, Outcome::with))
    }

    /// L with a cut selection: toggles looping playback over it (no project edit). Turning the
    /// loop on moves a playhead outside the selection to its start.
    pub fn loop_cut_preview(&mut self) -> EditResult {
        let overlay = self.session.overlay.as_mut().ok_or_else(|| {
            EditError::Unavailable("Selection loop unavailable: no cut selection.".into())
        })?;
        overlay.looping = !overlay.looping;
        if !overlay.looping {
            return Ok(Outcome::status("Stopped looping the selection."));
        }
        let range = overlay.range;
        let status = format!(
            "Looping selection {:.3}s-{:.3}s (L stops, Escape clears).",
            range.start_seconds, range.end_seconds
        );
        let playhead = self.session.playhead_seconds;
        let outcome = if (range.start_seconds..range.end_seconds).contains(&playhead) {
            Outcome::none()
        } else {
            self.seek(range.start_seconds)
        };
        Ok(outcome.with_status(status))
    }

    // ---- decoded-audio timing ---------------------------------------------------------

    /// Alt+R: trims the selected row to nearby speech in its lane's decoded audio.
    pub fn trim_current_segment_to_audio(&mut self) -> EditResult {
        self.guard("Audio trim")?;
        let segment_id = self.selected_segment_id("Audio trim")?;
        let segment = self.segment(&segment_id)?;
        let label = self.segment_label(&segment_id);
        let plan =
            match audio_trim_plan_for_segment(self.workflow.project(), &self.session, &segment) {
                Some(Ok(plan)) => plan,
                Some(Err(reason)) => {
                    return Err(EditError::Unavailable(format!(
                        "Audio trim skipped {label}: {reason}."
                    )));
                }
                None => {
                    return Ok(Outcome::status(format!(
                        "Audio trim made no timestamp changes for {label}."
                    )));
                }
            };
        self.apply_trim_plans(vec![plan], "Trim current segment")?;
        Ok(Outcome::status(format!(
            "Trimmed {label} to nearby decoded audio."
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

    /// Plans rows in time order against the bounds already planned for earlier rows (Helper
    /// trims one row at a time): two same-lane neighbours never both grow into the gap
    /// between them.
    fn visible_trim_plans(&self) -> Vec<AudioTrimPlan> {
        let mut working = self.workflow.project().clone();
        let ids: Vec<String> = visible_transcription_segments(
            &working,
            &self.session.track_filter,
            &self.session.collapsed_lane_ids,
            self.environment(),
        )
        .into_iter()
        .map(|segment| segment.id.clone())
        .collect();
        let mut plans = Vec::new();
        for id in ids {
            let Some(segment) = working.segment(&id).cloned() else {
                continue;
            };
            let Some(Ok(plan)) = audio_trim_plan_for_segment(&working, &self.session, &segment)
            else {
                continue;
            };
            // Growing to the next silence and trimming back can land on the same bounds.
            if (plan.start_seconds - segment.start_seconds).abs() < 1e-9
                && (plan.end_seconds - segment.end_seconds).abs() < 1e-9
            {
                continue;
            }
            if let Some(planned) = working.segments.iter_mut().find(|row| row.id == id) {
                planned.start_seconds = plan.start_seconds;
                planned.end_seconds = plan.end_seconds;
            }
            plans.push(plan);
        }
        plans
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

    /// Alt+Shift+S, in Helper's phase order (`autoSegmentVisibleSilences`): trims the visible
    /// rows to audio; merges same-lane rows at most 1s apart; captures the rows' text; splits
    /// rows in the middle of decoded silences of 1s or more; trims again; removes rows with no
    /// audio, stitching their text onto the nearest same-lane row; then places each captured
    /// text group back over its rows — through word timings when the lane has them, else by
    /// whole sentences and duration. Text is conserved exactly: a group whose placement would
    /// lose words keeps the split-time allocation and is reported.
    pub fn auto_segment_visible_silences(&mut self) -> EditResult {
        self.guard("Auto segmentation")?;
        if self.visible_segments().is_empty() {
            return Err(EditError::Unavailable(
                "Auto-segmentation found no visible segments.".into(),
            ));
        }
        let pre_trim = self.visible_trim_plans();
        let mut trimmed = pre_trim.len();
        if !pre_trim.is_empty() {
            self.apply_trim_plans(pre_trim, "Trim visible segments")?;
        }
        let merged = self.merge_close_visible_rows()?;

        let baseline = baseline_groups(&self.visible_segments());
        let mut plans = auto_segment_split_plans(self.workflow.project(), &self.session);
        // Split later points first so earlier split ids stay valid.
        plans.sort_by(|left, right| {
            left.segment_id
                .cmp(&right.segment_id)
                .then_with(|| right.split_seconds.total_cmp(&left.split_seconds))
        });
        let count = plans.len();
        let mut created = Vec::new();
        if !plans.is_empty() {
            self.apply("Auto segmentation", |workflow| {
                for plan in &plans {
                    created.push(workflow.split_segment_with_project_text_ratio(
                        &plan.segment_id,
                        plan.split_seconds,
                    )?);
                }
                Ok(())
            })?;
            self.mark_dirty();
            self.reconcile(true);
        }
        let post_trim = self.visible_trim_plans();
        trimmed += post_trim.len();
        if !post_trim.is_empty() {
            self.apply_trim_plans(post_trim, "Trim visible segments")?;
        }
        let cleanup = self.remove_silent_visible_rows()?;

        let placement = place_group_text(&baseline, &self.visible_segments(), &self.word_timings);
        let placed: Vec<String> = placement
            .updates
            .iter()
            .map(|update| update.segment_id.clone())
            .collect();
        if !placement.updates.is_empty() {
            let updates = placement.updates.clone();
            self.apply("Place segment text", |workflow| {
                workflow.apply_segment_edits(updates)
            })?;
            self.mark_dirty();
        }
        if trimmed == 0
            && merged.is_empty()
            && count == 0
            && cleanup.removed == 0
            && placed.is_empty()
        {
            return Err(EditError::Unavailable(
                "Auto-segmentation found no close same-speaker segments, silence runs over \
                 1000ms or silent segments."
                    .into(),
            ));
        }
        let split_sources = plans.iter().map(|plan| &plan.segment_id);
        for id in merged
            .iter()
            .chain(split_sources)
            .chain(&created)
            .chain(&cleanup.stitched)
            .chain(&placed)
        {
            self.drafts.refresh(self.workflow.project(), id);
        }

        let mut status =
            format!("Auto-segmented {count} silence run(s) over 1000ms and trimmed all");
        if !merged.is_empty() {
            status.push_str(&format!(
                "; merged {} close same-speaker segment pair(s) first",
                merged.len()
            ));
        }
        if cleanup.removed > 0 {
            status.push_str(&format!("; removed {} silent segment(s)", cleanup.removed));
        }
        if cleanup.kept > 0 {
            status.push_str(&format!(
                "; kept {} silent segment(s) with text and no same-speaker neighbour",
                cleanup.kept
            ));
        }
        match (placement.timed_groups, placement.sentence_groups) {
            (0, 0) => {}
            (timed, 0) => status.push_str(&format!("; placed text by word timestamps ({timed})")),
            (0, sentences) => status.push_str(&format!("; placed text by sentences ({sentences})")),
            (timed, sentences) => status.push_str(&format!(
                "; placed text by word timestamps ({timed}) and sentences ({sentences})"
            )),
        }
        if let Some((lane, error)) = placement.failed.first() {
            status.push_str(&format!(
                "; {} group(s) kept split text ({}: {error})",
                placement.failed.len(),
                self.lane_label(lane)
            ));
        }
        status.push('.');
        Ok(match created.last() {
            Some(last) if self.workflow.project().has_segment(last) => {
                let last = last.clone();
                self.select_segment_without_seek(&last).with_status(status)
            }
            _ => Outcome::status(status),
        })
    }

    /// Helper `mergeAutoSegmentCloseRows`: merges visible same-lane rows at most 1s apart one
    /// pair at a time (merges cascade), skipping pairs the project refuses. Returns the ids of
    /// the rows that absorbed a neighbour.
    fn merge_close_visible_rows(&mut self) -> Result<Vec<String>, EditError> {
        let track_filter = self.session.track_filter.clone();
        let collapsed = self.session.collapsed_lane_ids.clone();
        if auto_segment_merge_plans(&self.visible_segments()).is_empty() {
            return Ok(Vec::new());
        }
        let mut merged = Vec::new();
        let environment = self.environment().clone();
        self.apply("Auto segmentation", |workflow| {
            let mut skipped: Vec<AutoSegmentMergePlan> = Vec::new();
            for _ in 0..AUTO_SEGMENT_MERGE_ATTEMPTS {
                let rows = visible_transcription_segments(
                    workflow.project(),
                    &track_filter,
                    &collapsed,
                    &environment,
                );
                let Some(plan) = auto_segment_merge_plans(&rows)
                    .into_iter()
                    .find(|plan| !skipped.contains(plan))
                else {
                    break;
                };
                if workflow
                    .merge_segments(&plan.left_id, &plan.right_id)
                    .is_ok()
                {
                    merged.push(plan.left_id);
                    skipped.clear();
                } else {
                    skipped.push(plan);
                }
            }
            Ok(())
        })?;
        if !merged.is_empty() {
            self.mark_dirty();
            self.reconcile(true);
        }
        Ok(merged)
    }

    /// Helper `cleanupAutoSegmentSilentRows`: removes visible rows with no decoded audio above
    /// the trim threshold, latest first. A silent row's text is first stitched onto the nearest
    /// same-lane row (in time order); a silent row with text and no such row is kept.
    fn remove_silent_visible_rows(&mut self) -> Result<SilentCleanup, EditError> {
        let silent: Vec<String> = self
            .visible_segments()
            .into_iter()
            .filter(|segment| segment_is_silent(&self.session, segment))
            .map(|segment| segment.id.clone())
            .collect();
        let mut cleanup = SilentCleanup::default();
        if silent.is_empty() {
            return Ok(cleanup);
        }
        let track_filter = self.session.track_filter.clone();
        let collapsed = self.session.collapsed_lane_ids.clone();
        let environment = self.environment().clone();
        self.apply("Remove silent segments", |workflow| {
            for id in silent.iter().rev() {
                let rows = visible_transcription_segments(
                    workflow.project(),
                    &track_filter,
                    &collapsed,
                    &environment,
                );
                let Some(row) = rows.iter().copied().find(|row| row.id == *id) else {
                    continue;
                };
                let silent_text = row.text.trim().to_string();
                let stitch = match nearest_same_lane_segment(&rows, row) {
                    None if !silent_text.is_empty() => {
                        cleanup.kept += 1;
                        continue;
                    }
                    Some(nearest) if !silent_text.is_empty() => {
                        let text = if row.start_seconds < nearest.start_seconds {
                            join_segment_text(&silent_text, &nearest.text)
                        } else {
                            join_segment_text(&nearest.text, &silent_text)
                        };
                        let update = |text: String| SegmentUpdate {
                            segment_id: nearest.id.clone(),
                            start_seconds: nearest.start_seconds,
                            end_seconds: nearest.end_seconds,
                            text,
                        };
                        (text != nearest.text).then(|| (update(text), update(nearest.text.clone())))
                    }
                    _ => None,
                };
                if let Some((stitched, _)) = &stitch
                    && workflow
                        .apply_segment_edits(vec![stitched.clone()])
                        .is_err()
                {
                    cleanup.kept += 1;
                    continue;
                }
                if workflow.delete_segment(id).is_ok() {
                    cleanup.removed += 1;
                    if let Some((stitched, _)) = stitch {
                        cleanup.stitched.push(stitched.segment_id);
                    }
                } else {
                    // The neighbour must not keep a copy of text still in the silent row.
                    cleanup.kept += 1;
                    if let Some((_, restore)) = stitch {
                        workflow.apply_segment_edits(vec![restore])?;
                    }
                }
            }
            Ok(())
        })?;
        if cleanup.removed > 0 {
            self.mark_dirty();
            self.reconcile(true);
        }
        Ok(cleanup)
    }

    /// Visible transcription rows sorted by start.
    fn visible_segments(&self) -> Vec<&studio_core::Segment> {
        visible_transcription_segments(
            self.workflow.project(),
            &self.session.track_filter,
            &self.session.collapsed_lane_ids,
            self.environment(),
        )
    }
}

/// Status suffix naming how a smart split placed its text.
fn split_source_note(source: SplitSource) -> &'static str {
    match source {
        SplitSource::CompletedWord => " at a word timestamp",
        SplitSource::Interpolated => " between word timestamps",
        SplitSource::WordRatio | SplitSource::Empty => "",
    }
}
