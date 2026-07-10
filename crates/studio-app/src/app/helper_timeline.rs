use studio_app::StudioWorkflow;
use studio_app::{TrackFilter, TranscriptAnnotationKind};
use studio_core::{Project, Segment, SegmentUpdate};
use studio_waveform::DecodedAudio;

use super::StudioState;

const AUDIO_TRIM_THRESHOLD: f32 = 0.000_794_328_2;
const AUDIO_TRIM_OUTWARD_STEP_SECONDS: f64 = 0.05;
const AUDIO_TRIM_PADDING_SECONDS: f64 = 0.005;
const AUDIO_TRIM_EPSILON_SECONDS: f64 = 0.0015;
const AUDIO_TRIM_NEIGHBOR_GUARD_SECONDS: f64 = 0.01;
const AUTO_INSERT_SEGMENT_SCAN_WINDOW_SECONDS: f64 = 1.0;
const AUTO_INSERT_SEGMENT_PROVISIONAL_PADDING_SECONDS: f64 = 0.05;
const AUTO_INSERT_SEGMENT_MIN_DURATION_SECONDS: f64 = 0.0015;
const AUTO_INSERT_SEGMENT_MERGE_WINDOW_SECONDS: f64 = 1.0;
const AUTO_SEGMENT_MIN_SILENCE_SECONDS: f64 = 1.0;
const CUT_PREVIEW_MIN_SECONDS: f64 = 1.0;

#[derive(Debug, Clone, PartialEq)]
struct AudioTrimPlan {
    segment_id: String,
    start_seconds: f64,
    end_seconds: f64,
    text: String,
}

impl AudioTrimPlan {
    fn update(self) -> SegmentUpdate {
        SegmentUpdate {
            segment_id: self.segment_id,
            start_seconds: self.start_seconds,
            end_seconds: self.end_seconds,
            text: self.text,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct InwardAudioTrimPass {
    start_seconds: f64,
    end_seconds: f64,
    left_changed: bool,
    right_changed: bool,
}

impl InwardAudioTrimPass {
    fn changed(self) -> bool {
        self.left_changed || self.right_changed
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct OutwardAudioTrimPass {
    start_seconds: f64,
    end_seconds: f64,
    changed: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct AutoInsertPlan {
    lane_id: String,
    start_seconds: f64,
    end_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct AutoSegmentSplitPlan {
    segment_id: String,
    split_seconds: f64,
}

impl StudioState {
    pub(super) fn auto_segment_visible_silences(&mut self) -> Option<f32> {
        if self.block_read_only_edit("Auto segmentation") {
            return None;
        }

        let Some((project, editor)) = self.project().cloned().zip(self.editor.clone()) else {
            self.status = "Auto-segmentation unavailable: no project loaded.".to_string();
            return None;
        };
        let mut split_plans = auto_segment_split_plans(&project, &editor);
        if split_plans.is_empty() {
            self.status =
                "Auto-segmentation found no visible silence runs over 1000ms.".to_string();
            return None;
        }
        split_plans.sort_by(|left, right| {
            left.segment_id
                .cmp(&right.segment_id)
                .then_with(|| right.split_seconds.total_cmp(&left.split_seconds))
        });

        let split_count = split_plans.len();
        let mut created_segment_ids = Vec::new();
        let applied = self.with_editable_workflow_mut("Auto segmentation", |workflow| {
            for plan in &split_plans {
                let segment_id = workflow
                    .split_segment_with_project_text_ratio(&plan.segment_id, plan.split_seconds)
                    .map_err(|error| error.to_string())?;
                created_segment_ids.push(segment_id);
            }
            Ok(format!(
                "Auto-segmented {split_count} silence run(s) over 1000ms."
            ))
        });
        if !applied {
            return None;
        }

        self.mark_dirty();
        self.reconcile_editor_with_project_preserving_viewport();
        self.trim_all_visible_segments_to_audio();
        for segment_id in &created_segment_ids {
            self.refresh_segment_draft(segment_id);
        }
        let scroll_target = created_segment_ids
            .last()
            .and_then(|segment_id| self.select_segment_without_seek(segment_id));
        self.status =
            format!("Auto-segmented {split_count} silence run(s) over 1000ms and trimmed all.");
        scroll_target
    }

    pub(super) fn auto_insert_segment_at_caret(&mut self) -> Option<f32> {
        if self.block_read_only_edit("Auto insert segment") {
            return None;
        }

        let Some((project, editor)) = self.project().cloned().zip(self.editor.clone()) else {
            self.status = "Auto-insert unavailable: no project loaded.".to_string();
            return None;
        };
        let plans = auto_insert_plans_for_caret(&project, &editor);
        if plans.is_empty() {
            self.status = "Auto-insert found no uncovered speech near caret.".to_string();
            return None;
        }

        let mut created_segment_ids = Vec::new();
        let mut merge_count = 0;
        let applied = self.with_editable_workflow_mut("Auto insert segment", |workflow| {
            for plan in &plans {
                let segment_id = workflow
                    .create_segment_in_lane(&plan.lane_id, plan.start_seconds, plan.end_seconds)
                    .map_err(|error| error.to_string())?;
                merge_count += merge_auto_insert_adjacent_segments(workflow, &segment_id)?;
                created_segment_ids.push(segment_id);
            }
            Ok(format!(
                "Auto-inserted {} segment(s) near caret, merged {merge_count} adjacent row(s).",
                plans.len(),
            ))
        });

        if !applied {
            return None;
        }

        self.mark_dirty();
        self.reconcile_editor_with_project_preserving_viewport();
        for segment_id in &created_segment_ids {
            self.refresh_segment_draft(segment_id);
        }
        let scroll_target = created_segment_ids
            .last()
            .and_then(|segment_id| self.select_segment_without_seek(segment_id));
        self.status = format!(
            "Auto-inserted {} segment(s) around nearest uncovered speech, merged {merge_count} adjacent row(s).",
            created_segment_ids.len(),
        );
        scroll_target
    }

    pub(super) fn commit_cut_preview_overlay(&mut self, smart_split: bool) -> Option<f32> {
        if self.block_read_only_edit("Cut preview") {
            return None;
        }

        let Some(overlay_range) = self.editor.as_ref().and_then(|editor| editor.overlay_range)
        else {
            self.status = "Cut preview unavailable: no active overlay range.".to_string();
            return None;
        };
        if overlay_range.duration_seconds() < CUT_PREVIEW_MIN_SECONDS {
            self.status = "Cut preview must be at least 1.00s.".to_string();
            return None;
        }

        let Some(project) = self.project().cloned() else {
            self.status = "Cut preview unavailable: no project loaded.".to_string();
            return None;
        };
        let Some(lane_id) = cut_preview_lane_id(&project, self.editor.as_ref(), overlay_range)
        else {
            self.status = "Cut preview unavailable: no selected or overlapping lane.".to_string();
            return None;
        };
        if !project.segments.iter().any(|segment| {
            segment.lane_id == lane_id
                && segment.end_seconds > overlay_range.start_seconds
                && segment.start_seconds < overlay_range.end_seconds
        }) {
            self.status =
                "Cut preview found no overlapping segment on the target lane.".to_string();
            return None;
        }

        let applied = self.with_editable_workflow_mut("Cut preview", |workflow| {
            workflow
                .cut_lane_range(
                    &lane_id,
                    overlay_range.start_seconds,
                    overlay_range.end_seconds,
                    smart_split,
                )
                .map_err(|error| error.to_string())?;
            Ok(format!(
                "Cut {:.3}s-{:.3}s on `{lane_id}`{}.",
                overlay_range.start_seconds,
                overlay_range.end_seconds,
                if smart_split { " with smart split" } else { "" }
            ))
        });
        if !applied {
            return None;
        }

        self.mark_dirty();
        self.reconcile_editor_with_project_preserving_viewport();
        self.with_editor_mut(|editor| {
            editor.overlay_range = None;
        });

        let scroll_target_segment_id = self
            .project()
            .and_then(|project| cut_preview_scroll_target(project, &lane_id, overlay_range));
        scroll_target_segment_id
            .as_deref()
            .and_then(|segment_id| self.select_segment_without_seek(segment_id))
    }

    pub(super) fn loop_cut_preview_overlay(&mut self) {
        let Some(overlay_range) = self.editor.as_ref().and_then(|editor| editor.overlay_range)
        else {
            self.status = "Selection loop unavailable: no active overlay range.".to_string();
            return;
        };
        let Some(project) = self.project().cloned() else {
            self.status = "Selection loop unavailable: no project loaded.".to_string();
            return;
        };

        self.with_editor_mut(|editor| {
            editor.set_loop_range_from_drag(
                &project,
                overlay_range.start_seconds,
                overlay_range.end_seconds,
            );
        });
        self.status = format!(
            "Looping cut preview {:.3}s-{:.3}s.",
            overlay_range.start_seconds, overlay_range.end_seconds
        );
        self.restart_audio_playback_if_playing();
    }

    pub(super) fn trim_current_segment_to_audio(&mut self) {
        if self.block_read_only_edit("Audio trim") {
            return;
        }

        let Some(segment_id) = self
            .editor
            .as_ref()
            .and_then(|editor| editor.selected_segment_id.clone())
        else {
            self.status = "Audio trim unavailable: no selected segment.".to_string();
            return;
        };

        let Some(plan) = self.audio_trim_plan_for_segment(&segment_id) else {
            return;
        };

        let updated_id = plan.segment_id.clone();
        if self.apply_audio_trim_plans(vec![plan], "Trim current segment") {
            self.status = format!("Trimmed `{updated_id}` to nearby decoded audio.");
        }
    }

    pub(super) fn trim_all_visible_segments_to_audio(&mut self) {
        if self.block_read_only_edit("Audio trim") {
            return;
        }

        let Some((project, editor)) = self.project().cloned().zip(self.editor.clone()) else {
            self.status = "Audio trim unavailable: no project loaded.".to_string();
            return;
        };
        let plans = visible_transcription_segments(
            &project,
            &editor.track_filter,
            &editor.collapsed_lane_ids,
        )
        .into_iter()
        .filter_map(|segment| {
            audio_trim_plan_for_segment(&project, &editor, segment).and_then(|plan| plan.ok())
        })
        .collect::<Vec<_>>();

        if plans.is_empty() {
            self.status = "Audio trim found no visible segments with decoded speech.".to_string();
            return;
        }

        let count = plans.len();
        if self.apply_audio_trim_plans(plans, "Trim visible segments") {
            self.status = format!("Trimmed {count} visible segments to nearby decoded audio.");
        }
    }

    fn audio_trim_plan_for_segment(&mut self, segment_id: &str) -> Option<AudioTrimPlan> {
        let Some((project, editor)) = self.project().cloned().zip(self.editor.clone()) else {
            self.status = "Audio trim unavailable: no project loaded.".to_string();
            return None;
        };
        let Some(segment) = project.segment(segment_id) else {
            self.status = format!("Audio trim unavailable: segment `{segment_id}` not found.");
            return None;
        };

        match audio_trim_plan_for_segment(&project, &editor, segment) {
            Some(Ok(plan)) => Some(plan),
            Some(Err(reason)) => {
                self.status = format!("Audio trim skipped `{segment_id}`: {reason}.");
                None
            }
            None => {
                self.status = format!("Audio trim made no timestamp changes for `{segment_id}`.");
                None
            }
        }
    }

    fn apply_audio_trim_plans(&mut self, plans: Vec<AudioTrimPlan>, action_name: &str) -> bool {
        if plans.is_empty() {
            return false;
        }
        let segment_ids = plans
            .iter()
            .map(|plan| plan.segment_id.clone())
            .collect::<Vec<_>>();
        let updates = plans
            .into_iter()
            .map(AudioTrimPlan::update)
            .collect::<Vec<_>>();

        if self.with_editable_workflow_mut(action_name, |workflow| {
            workflow
                .apply_segment_edits(updates)
                .map_err(|error| error.to_string())?;
            Ok(format!("{action_name} applied."))
        }) {
            for segment_id in segment_ids {
                self.refresh_segment_draft(&segment_id);
            }
            self.mark_dirty();
            self.reconcile_editor_with_project_preserving_viewport();
            return true;
        }
        false
    }
}

fn merge_auto_insert_adjacent_segments(
    workflow: &mut StudioWorkflow,
    segment_id: &str,
) -> Result<usize, String> {
    let mut merge_count = 0;
    if auto_insert_previous_gap_seconds(workflow.project(), segment_id)
        .is_some_and(|gap| gap <= AUTO_INSERT_SEGMENT_MERGE_WINDOW_SECONDS)
    {
        workflow
            .merge_with_previous(segment_id)
            .map_err(|error| error.to_string())?;
        merge_count += 1;
    }

    if auto_insert_next_gap_seconds(workflow.project(), segment_id)
        .is_some_and(|gap| gap <= AUTO_INSERT_SEGMENT_MERGE_WINDOW_SECONDS)
    {
        workflow
            .merge_with_next(segment_id)
            .map_err(|error| error.to_string())?;
        merge_count += 1;
    }

    Ok(merge_count)
}

fn auto_insert_previous_gap_seconds(project: &Project, segment_id: &str) -> Option<f64> {
    let current = project.segment(segment_id)?;
    project
        .segments
        .iter()
        .filter(|segment| segment.id != current.id)
        .filter(|segment| segment.lane_id == current.lane_id)
        .filter(|segment| segment.speaker_id == current.speaker_id)
        .filter(|segment| segment.end_seconds <= current.start_seconds)
        .max_by(|left, right| left.end_seconds.total_cmp(&right.end_seconds))
        .map(|previous| current.start_seconds - previous.end_seconds)
}

fn auto_insert_next_gap_seconds(project: &Project, segment_id: &str) -> Option<f64> {
    let current = project.segment(segment_id)?;
    project
        .segments
        .iter()
        .filter(|segment| segment.id != current.id)
        .filter(|segment| segment.lane_id == current.lane_id)
        .filter(|segment| segment.speaker_id == current.speaker_id)
        .filter(|segment| segment.start_seconds >= current.end_seconds)
        .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
        .map(|next| next.start_seconds - current.end_seconds)
}

fn cut_preview_lane_id(
    project: &Project,
    editor: Option<&studio_app::EditorSession>,
    overlay_range: studio_app::TimeRange,
) -> Option<String> {
    editor
        .and_then(|editor| editor.selected_segment_id.as_deref())
        .and_then(|segment_id| project.segment(segment_id))
        .map(|segment| segment.lane_id.clone())
        .or_else(|| {
            project
                .segments
                .iter()
                .filter(|segment| {
                    segment.end_seconds > overlay_range.start_seconds
                        && segment.start_seconds < overlay_range.end_seconds
                })
                .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
                .map(|segment| segment.lane_id.clone())
        })
}

fn cut_preview_scroll_target(
    project: &Project,
    lane_id: &str,
    overlay_range: studio_app::TimeRange,
) -> Option<String> {
    project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id)
        .filter(|segment| segment.start_seconds >= overlay_range.end_seconds - 0.001)
        .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
        .or_else(|| {
            project
                .segments
                .iter()
                .filter(|segment| segment.lane_id == lane_id)
                .filter(|segment| segment.end_seconds <= overlay_range.start_seconds + 0.001)
                .max_by(|left, right| left.end_seconds.total_cmp(&right.end_seconds))
        })
        .map(|segment| segment.id.clone())
}

fn auto_insert_plans_for_caret(
    project: &Project,
    editor: &studio_app::EditorSession,
) -> Vec<AutoInsertPlan> {
    project
        .lanes
        .iter()
        .filter(|lane| track_filter_matches_lane(&editor.track_filter, &lane.id))
        .filter(|lane| !editor.collapsed_lane_ids.contains(&lane.id))
        .filter_map(|lane| auto_insert_plan_for_lane(project, editor, &lane.id))
        .collect()
}

fn auto_insert_plan_for_lane(
    project: &Project,
    editor: &studio_app::EditorSession,
    lane_id: &str,
) -> Option<AutoInsertPlan> {
    let waveform = editor.lane_waveforms.get(lane_id)?;
    let audio = waveform.decoded_audio()?;
    let playhead_seconds = editor.playhead_seconds;
    let scan_start = (playhead_seconds - AUTO_INSERT_SEGMENT_SCAN_WINDOW_SECONDS)
        .max(0.0)
        .min(audio.duration());
    let scan_end = (playhead_seconds + AUTO_INSERT_SEGMENT_SCAN_WINDOW_SECONDS)
        .max(scan_start)
        .min(audio.duration());
    if scan_end <= scan_start {
        return None;
    }

    let (speech_start, speech_end) = decoded_audio_speech_islands(audio, scan_start, scan_end)
        .into_iter()
        .filter(|(start_seconds, end_seconds)| {
            !range_overlaps_existing_transcription_segment(
                project,
                lane_id,
                *start_seconds,
                *end_seconds,
            )
        })
        .min_by(|left, right| {
            distance_to_range(playhead_seconds, *left)
                .total_cmp(&distance_to_range(playhead_seconds, *right))
                .then_with(|| left.0.total_cmp(&right.0))
        })?;

    let (min_start, max_end) =
        same_lane_open_range_limits(project, lane_id, speech_start, speech_end);
    let start_seconds = (speech_start - AUTO_INSERT_SEGMENT_PROVISIONAL_PADDING_SECONDS)
        .max(0.0)
        .max(min_start);
    let end_seconds = (speech_end + AUTO_INSERT_SEGMENT_PROVISIONAL_PADDING_SECONDS)
        .min(audio.duration())
        .min(max_end);
    if end_seconds <= start_seconds + AUTO_INSERT_SEGMENT_MIN_DURATION_SECONDS {
        return None;
    }

    Some(AutoInsertPlan {
        lane_id: lane_id.to_string(),
        start_seconds,
        end_seconds,
    })
}

fn auto_segment_split_plans(
    project: &Project,
    editor: &studio_app::EditorSession,
) -> Vec<AutoSegmentSplitPlan> {
    visible_transcription_segments(project, &editor.track_filter, &editor.collapsed_lane_ids)
        .into_iter()
        .flat_map(|segment| auto_segment_split_plans_for_segment(editor, segment))
        .collect()
}

fn auto_segment_split_plans_for_segment(
    editor: &studio_app::EditorSession,
    segment: &Segment,
) -> Vec<AutoSegmentSplitPlan> {
    let Some(audio) = editor
        .lane_waveforms
        .get(&segment.lane_id)
        .and_then(|waveform| waveform.decoded_audio())
    else {
        return Vec::new();
    };
    let islands = decoded_audio_speech_islands(audio, segment.start_seconds, segment.end_seconds);
    islands
        .windows(2)
        .filter_map(|pair| {
            let left = pair[0];
            let right = pair[1];
            let silence_seconds = right.0 - left.1;
            if silence_seconds < AUTO_SEGMENT_MIN_SILENCE_SECONDS {
                return None;
            }
            let split_seconds = left.1 + silence_seconds / 2.0;
            if split_seconds <= segment.start_seconds || split_seconds >= segment.end_seconds {
                return None;
            }
            Some(AutoSegmentSplitPlan {
                segment_id: segment.id.clone(),
                split_seconds,
            })
        })
        .collect()
}

fn visible_transcription_segments<'a>(
    project: &'a Project,
    track_filter: &TrackFilter,
    collapsed_lane_ids: &std::collections::BTreeSet<String>,
) -> Vec<&'a Segment> {
    let mut segments = project
        .segments
        .iter()
        .filter(|segment| track_filter_matches_lane(track_filter, &segment.lane_id))
        .filter(|segment| !collapsed_lane_ids.contains(&segment.lane_id))
        .filter(|segment| TranscriptAnnotationKind::from_segment(segment).is_transcription())
        .collect::<Vec<_>>();
    segments.sort_by(|left, right| {
        left.start_seconds
            .total_cmp(&right.start_seconds)
            .then_with(|| left.id.cmp(&right.id))
    });
    segments
}

fn track_filter_matches_lane(track_filter: &TrackFilter, lane_id: &str) -> bool {
    match track_filter {
        TrackFilter::All => true,
        TrackFilter::Lane { id, .. } => id == lane_id,
    }
}

fn audio_trim_plan_for_segment(
    project: &Project,
    editor: &studio_app::EditorSession,
    segment: &Segment,
) -> Option<Result<AudioTrimPlan, String>> {
    let waveform = editor
        .lane_waveforms
        .get(&segment.lane_id)
        .ok_or_else(|| "decoded lane audio is not loaded".to_string());
    let waveform = match waveform {
        Ok(waveform) => waveform,
        Err(error) => return Some(Err(error)),
    };
    let audio = match waveform.decoded_audio() {
        Some(audio) => audio,
        None => return Some(Err("lane waveform has precomputed peaks only".to_string())),
    };
    let (neighbor_min_start, neighbor_max_end) = same_lane_neighbor_limits(project, segment);

    let inward_pass = match inward_audio_trim_pass(
        audio,
        segment.start_seconds,
        segment.end_seconds,
        neighbor_min_start,
        neighbor_max_end,
    ) {
        Ok(pass) => pass,
        Err(error) => return Some(Err(error)),
    };
    let mut start_seconds = inward_pass.start_seconds;
    let mut end_seconds = inward_pass.end_seconds;
    let mut changed = inward_pass.changed();

    if !inward_pass.left_changed || !inward_pass.right_changed {
        let outward_pass = outward_audio_trim_pass(
            audio,
            start_seconds,
            end_seconds,
            neighbor_min_start,
            neighbor_max_end,
            !inward_pass.left_changed,
            !inward_pass.right_changed,
        );
        start_seconds = outward_pass.start_seconds;
        end_seconds = outward_pass.end_seconds;
        changed |= outward_pass.changed;

        if changed {
            match inward_audio_trim_pass(
                audio,
                start_seconds,
                end_seconds,
                neighbor_min_start,
                neighbor_max_end,
            ) {
                Ok(final_pass) => {
                    start_seconds = final_pass.start_seconds;
                    end_seconds = final_pass.end_seconds;
                    changed |= final_pass.changed();
                }
                Err(error) if error == "no audio above trim threshold" => {}
                Err(error) => return Some(Err(format!("final trim failed: {error}"))),
            }
        }
    }

    if end_seconds <= start_seconds + AUDIO_TRIM_EPSILON_SECONDS {
        return Some(Err("trim would collapse the segment".to_string()));
    }
    if !changed {
        return None;
    }

    Some(Ok(AudioTrimPlan {
        segment_id: segment.id.clone(),
        start_seconds,
        end_seconds,
        text: segment.text.clone(),
    }))
}

fn inward_audio_trim_pass(
    audio: &DecodedAudio,
    current_start_seconds: f64,
    current_end_seconds: f64,
    min_start_seconds: f64,
    max_end_seconds: f64,
) -> Result<InwardAudioTrimPass, String> {
    let Some((speech_start, speech_end)) =
        decoded_audio_speech_bounds(audio, current_start_seconds, current_end_seconds)
    else {
        return Err("no audio above trim threshold".to_string());
    };

    let target_start_seconds = (speech_start - AUDIO_TRIM_PADDING_SECONDS)
        .max(current_start_seconds)
        .max(min_start_seconds);
    let target_end_seconds = (speech_end + AUDIO_TRIM_PADDING_SECONDS)
        .min(current_end_seconds)
        .min(max_end_seconds);

    if target_end_seconds <= target_start_seconds + AUDIO_TRIM_EPSILON_SECONDS {
        return Err("trim would collapse the segment".to_string());
    }

    let mut start_seconds = current_start_seconds;
    let mut end_seconds = current_end_seconds;
    let mut left_changed = false;
    let mut right_changed = false;

    if target_end_seconds < current_end_seconds - AUDIO_TRIM_EPSILON_SECONDS
        && target_end_seconds > current_start_seconds + AUDIO_TRIM_EPSILON_SECONDS
    {
        end_seconds = target_end_seconds;
        right_changed = true;
    }
    if target_start_seconds > current_start_seconds + AUDIO_TRIM_EPSILON_SECONDS
        && target_start_seconds < end_seconds - AUDIO_TRIM_EPSILON_SECONDS
    {
        start_seconds = target_start_seconds;
        left_changed = true;
    }

    Ok(InwardAudioTrimPass {
        start_seconds,
        end_seconds,
        left_changed,
        right_changed,
    })
}

fn outward_audio_trim_pass(
    audio: &DecodedAudio,
    current_start_seconds: f64,
    current_end_seconds: f64,
    min_start_seconds: f64,
    max_end_seconds: f64,
    extend_left: bool,
    extend_right: bool,
) -> OutwardAudioTrimPass {
    let mut start_seconds = current_start_seconds;
    let mut end_seconds = current_end_seconds;
    let mut changed = false;

    if extend_right
        && let Some(target_end_seconds) = decoded_audio_next_silence(audio, current_end_seconds)
            .map(|seconds| {
                seconds
                    .max(current_end_seconds)
                    .min(audio.duration())
                    .min(max_end_seconds)
            })
        && target_end_seconds > end_seconds + AUDIO_TRIM_EPSILON_SECONDS
        && target_end_seconds > start_seconds + AUDIO_TRIM_EPSILON_SECONDS
    {
        end_seconds = target_end_seconds;
        changed = true;
    }

    if extend_left
        && let Some(target_start_seconds) =
            decoded_audio_previous_silence(audio, current_start_seconds).map(|seconds| {
                seconds
                    .min(current_start_seconds)
                    .max(0.0)
                    .max(min_start_seconds)
            })
        && target_start_seconds < start_seconds - AUDIO_TRIM_EPSILON_SECONDS
        && target_start_seconds < end_seconds - AUDIO_TRIM_EPSILON_SECONDS
    {
        start_seconds = target_start_seconds;
        changed = true;
    }

    OutwardAudioTrimPass {
        start_seconds,
        end_seconds,
        changed,
    }
}

fn decoded_audio_speech_bounds(
    audio: &DecodedAudio,
    start_seconds: f64,
    end_seconds: f64,
) -> Option<(f64, f64)> {
    if !start_seconds.is_finite() || !end_seconds.is_finite() || end_seconds <= start_seconds {
        return None;
    }
    let sample_rate = audio.sample_rate() as f64;
    let start_frame = frame_index_for_seconds(audio, start_seconds, FrameBound::Start);
    let end_frame = frame_index_for_seconds(audio, end_seconds, FrameBound::End);
    let mut first = None;
    let mut last = None;
    for frame_index in start_frame..end_frame {
        let amplitude = (0..audio.number_of_channels())
            .filter_map(|channel_index| audio.channel_data(channel_index))
            .filter_map(|channel| channel.get(frame_index))
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        if amplitude >= AUDIO_TRIM_THRESHOLD {
            first.get_or_insert(frame_index);
            last = Some(frame_index);
        }
    }
    let first = first?;
    let last = last?;
    Some((first as f64 / sample_rate, last as f64 / sample_rate))
}

fn decoded_audio_speech_islands(
    audio: &DecodedAudio,
    start_seconds: f64,
    end_seconds: f64,
) -> Vec<(f64, f64)> {
    let sample_rate = audio.sample_rate() as f64;
    let start_frame = frame_index_for_seconds(audio, start_seconds, FrameBound::Start);
    let end_frame = frame_index_for_seconds(audio, end_seconds, FrameBound::End);
    let mut islands = Vec::new();
    let mut active_start = None;
    let mut active_last = None;

    for frame_index in start_frame..end_frame {
        let amplitude = max_frame_amplitude(audio, frame_index);
        if amplitude >= AUDIO_TRIM_THRESHOLD {
            active_start.get_or_insert(frame_index);
            active_last = Some(frame_index);
            continue;
        }

        if let (Some(first), Some(last)) = (active_start.take(), active_last.take()) {
            islands.push((first as f64 / sample_rate, (last + 1) as f64 / sample_rate));
        }
    }

    if let (Some(first), Some(last)) = (active_start, active_last) {
        islands.push((first as f64 / sample_rate, (last + 1) as f64 / sample_rate));
    }

    islands
}

fn max_frame_amplitude(audio: &DecodedAudio, frame_index: usize) -> f32 {
    (0..audio.number_of_channels())
        .filter_map(|channel_index| audio.channel_data(channel_index))
        .filter_map(|channel| channel.get(frame_index))
        .map(|sample| sample.abs())
        .fold(0.0_f32, f32::max)
}

fn decoded_audio_previous_silence(audio: &DecodedAudio, start_seconds: f64) -> Option<f64> {
    if audio.frame_count() == 0 {
        return None;
    }
    let sample_rate = audio.sample_rate() as f64;
    let step_frames = ((AUDIO_TRIM_OUTWARD_STEP_SECONDS * sample_rate).floor() as usize).max(1);
    let start_frame = frame_index_for_seconds(audio, start_seconds, FrameBound::Start);
    let mut index = start_frame.saturating_sub(1).min(audio.frame_count() - 1);

    loop {
        let block_start = index.saturating_sub(step_frames - 1);
        let block_end = index + 1;
        if is_decoded_audio_silent_block(audio, block_start, block_end) {
            return Some(index as f64 / sample_rate);
        }
        if block_start == 0 {
            break;
        }
        index = block_start - 1;
    }

    None
}

fn decoded_audio_next_silence(audio: &DecodedAudio, start_seconds: f64) -> Option<f64> {
    if audio.frame_count() == 0 {
        return None;
    }
    let sample_rate = audio.sample_rate() as f64;
    let step_frames = ((AUDIO_TRIM_OUTWARD_STEP_SECONDS * sample_rate).floor() as usize).max(1);
    let mut index =
        frame_index_for_seconds(audio, start_seconds, FrameBound::End).min(audio.frame_count() - 1);

    while index < audio.frame_count() {
        let block_end = (index + step_frames).min(audio.frame_count());
        if is_decoded_audio_silent_block(audio, index, block_end) {
            return Some(index as f64 / sample_rate);
        }
        if block_end == audio.frame_count() {
            break;
        }
        index = block_end;
    }

    None
}

fn is_decoded_audio_silent_block(
    audio: &DecodedAudio,
    start_frame: usize,
    end_frame_exclusive: usize,
) -> bool {
    if end_frame_exclusive <= start_frame {
        return false;
    }
    (start_frame..end_frame_exclusive)
        .all(|frame_index| max_frame_amplitude(audio, frame_index) < AUDIO_TRIM_THRESHOLD)
}

fn distance_to_range(point_seconds: f64, range: (f64, f64)) -> f64 {
    if point_seconds < range.0 {
        range.0 - point_seconds
    } else if point_seconds > range.1 {
        point_seconds - range.1
    } else {
        0.0
    }
}

fn range_overlaps_existing_transcription_segment(
    project: &Project,
    lane_id: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> bool {
    project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id)
        .filter(|segment| TranscriptAnnotationKind::from_segment(segment).is_transcription())
        .any(|segment| segment.start_seconds < end_seconds && start_seconds < segment.end_seconds)
}

fn same_lane_open_range_limits(
    project: &Project,
    lane_id: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> (f64, f64) {
    let previous_end = project
        .segments
        .iter()
        .filter(|candidate| candidate.lane_id == lane_id)
        .filter(|candidate| TranscriptAnnotationKind::from_segment(candidate).is_transcription())
        .filter(|candidate| candidate.end_seconds <= start_seconds)
        .map(|candidate| candidate.end_seconds)
        .max_by(f64::total_cmp)
        .unwrap_or(f64::NEG_INFINITY);
    let next_start = project
        .segments
        .iter()
        .filter(|candidate| candidate.lane_id == lane_id)
        .filter(|candidate| TranscriptAnnotationKind::from_segment(candidate).is_transcription())
        .filter(|candidate| candidate.start_seconds >= end_seconds)
        .map(|candidate| candidate.start_seconds)
        .min_by(f64::total_cmp)
        .unwrap_or(f64::INFINITY);
    (previous_end, next_start)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameBound {
    Start,
    End,
}

fn frame_index_for_seconds(audio: &DecodedAudio, seconds: f64, bound: FrameBound) -> usize {
    let frame = seconds.max(0.0) * audio.sample_rate() as f64;
    let index = match bound {
        FrameBound::Start => frame.floor(),
        FrameBound::End => frame.ceil(),
    };
    (index as usize).min(audio.frame_count())
}

fn same_lane_neighbor_limits(project: &Project, segment: &Segment) -> (f64, f64) {
    let previous_end = project
        .segments
        .iter()
        .filter(|candidate| candidate.id != segment.id && candidate.lane_id == segment.lane_id)
        .filter(|candidate| candidate.end_seconds <= segment.start_seconds)
        .map(|candidate| candidate.end_seconds + AUDIO_TRIM_NEIGHBOR_GUARD_SECONDS)
        .max_by(f64::total_cmp)
        .unwrap_or(f64::NEG_INFINITY);
    let next_start = project
        .segments
        .iter()
        .filter(|candidate| candidate.id != segment.id && candidate.lane_id == segment.lane_id)
        .filter(|candidate| candidate.start_seconds >= segment.end_seconds)
        .map(|candidate| candidate.start_seconds - AUDIO_TRIM_NEIGHBOR_GUARD_SECONDS)
        .min_by(f64::total_cmp)
        .unwrap_or(f64::INFINITY);
    (previous_end, next_start)
}
