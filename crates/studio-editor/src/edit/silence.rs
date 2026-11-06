//! Decoded-audio timing algorithms: trim segments to nearby speech, split visible segments on
//! long silences, auto-insert a segment around uncovered speech near the playhead, and the
//! cut-preview target/scroll helpers. All functions are pure over the project + session.

use studio_core::{Project, Segment, SegmentUpdate};
use studio_waveform::DecodedAudio;

use crate::edit::text_placement::AUTO_SEGMENT_MERGE_GAP_SECONDS;
use crate::editor_model::EditorSession;
use crate::timeline_model::TimeRange;
use crate::track_filter::TrackFilter;
use crate::workflow::StudioWorkflow;
use exaudio_extension::document::DocumentEnvironment;

/// Absolute sample amplitude counted as speech (about -62 dBFS).
pub const AUDIO_TRIM_THRESHOLD: f32 = 0.000_794_328_2;
const AUDIO_TRIM_OUTWARD_STEP_SECONDS: f64 = 0.05;
/// Padding kept around detected speech when trimming.
pub const AUDIO_TRIM_PADDING_SECONDS: f64 = 0.005;
const AUDIO_TRIM_EPSILON_SECONDS: f64 = 0.0015;
/// Gap kept from same-lane neighbours when trimming outward.
pub const AUDIO_TRIM_NEIGHBOR_GUARD_SECONDS: f64 = 0.01;
/// Auto-insert scans this far either side of the playhead.
pub const AUTO_INSERT_SEGMENT_SCAN_WINDOW_SECONDS: f64 = 1.0;
const AUTO_INSERT_SEGMENT_PROVISIONAL_PADDING_SECONDS: f64 = 0.05;
const AUTO_INSERT_SEGMENT_MIN_DURATION_SECONDS: f64 = 0.0015;
/// Auto-inserted segments merge with same-speaker neighbours closer than this.
pub const AUTO_INSERT_SEGMENT_MERGE_WINDOW_SECONDS: f64 = 1.0;
/// Auto-segmentation splits on silences at least this long.
pub const AUTO_SEGMENT_MIN_SILENCE_SECONDS: f64 = 1.0;

/// New boundaries for one segment after trimming to audio.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTrimPlan {
    pub segment_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

impl AudioTrimPlan {
    pub fn update(self) -> SegmentUpdate {
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

/// Segment to create around uncovered speech.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoInsertPlan {
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Split point in the middle of a long silence.
#[derive(Debug, Clone, PartialEq)]
pub struct AutoSegmentSplitPlan {
    pub segment_id: String,
    pub split_seconds: f64,
}

pub fn merge_auto_insert_adjacent_segments(
    workflow: &mut StudioWorkflow,
    segment_id: &str,
) -> anyhow::Result<usize> {
    let mut merge_count = 0;
    if auto_insert_previous_gap_seconds(workflow.project(), segment_id)
        .is_some_and(|gap| gap <= AUTO_INSERT_SEGMENT_MERGE_WINDOW_SECONDS)
    {
        workflow.merge_with_previous(segment_id)?;
        merge_count += 1;
    }

    if auto_insert_next_gap_seconds(workflow.project(), segment_id)
        .is_some_and(|gap| gap <= AUTO_INSERT_SEGMENT_MERGE_WINDOW_SECONDS)
    {
        workflow.merge_with_next(segment_id)?;
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

pub fn cut_preview_scroll_target(
    project: &Project,
    lane_id: &str,
    overlay_range: TimeRange,
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

pub fn auto_insert_plans_for_caret(
    project: &Project,
    editor: &EditorSession,
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
    editor: &EditorSession,
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
                editor.environment(),
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

    let (min_start, max_end) = same_lane_open_range_limits(
        project,
        editor.environment(),
        lane_id,
        speech_start,
        speech_end,
    );
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

pub fn auto_segment_split_plans(
    project: &Project,
    editor: &EditorSession,
) -> Vec<AutoSegmentSplitPlan> {
    visible_transcription_segments(
        project,
        &editor.track_filter,
        &editor.collapsed_lane_ids,
        editor.environment(),
    )
    .into_iter()
    .flat_map(|segment| auto_segment_split_plans_for_segment(editor, segment))
    .collect()
}

pub fn auto_segment_split_plans_for_segment(
    editor: &EditorSession,
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

/// One auto-segmentation merge: `left` absorbs `right`, the next row on its lane. The
/// millisecond bounds tell a retried plan from a new one between the same rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoSegmentMergePlan {
    pub left_id: String,
    pub right_id: String,
    left_end_ms: i64,
    right_start_ms: i64,
}

/// Helper `collectAutoSegmentMergePlans`: lanes in order of their first row, each lane's rows
/// by start, and every neighbour pair at most [`AUTO_SEGMENT_MERGE_GAP_SECONDS`] apart, the
/// latest pair first.
pub fn auto_segment_merge_plans(rows: &[&Segment]) -> Vec<AutoSegmentMergePlan> {
    let mut lanes: Vec<(&str, Vec<&Segment>)> = Vec::new();
    for row in rows {
        match lanes.iter_mut().find(|(lane, _)| *lane == row.lane_id) {
            Some((_, lane_rows)) => lane_rows.push(row),
            None => lanes.push((&row.lane_id, vec![row])),
        }
    }
    let mut plans = Vec::new();
    for (_, mut lane_rows) in lanes {
        lane_rows.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
        for pair in lane_rows.windows(2).rev() {
            let (current, next) = (pair[0], pair[1]);
            let gap = next.start_seconds - current.end_seconds;
            if !gap.is_finite() || gap > AUTO_SEGMENT_MERGE_GAP_SECONDS {
                continue;
            }
            let plan = AutoSegmentMergePlan {
                left_id: current.id.clone(),
                right_id: next.id.clone(),
                left_end_ms: (current.end_seconds * 1000.0).round() as i64,
                right_start_ms: (next.start_seconds * 1000.0).round() as i64,
            };
            if !plans.contains(&plan) {
                plans.push(plan);
            }
        }
    }
    plans
}

/// Whether a row's decoded lane audio has nothing above the trim threshold inside it (Helper's
/// silent-row probe). Rows whose lane audio is not decoded are never silent.
pub fn segment_is_silent(editor: &EditorSession, segment: &Segment) -> bool {
    editor
        .lane_waveforms
        .get(&segment.lane_id)
        .and_then(|waveform| waveform.decoded_audio())
        .is_some_and(|audio| {
            decoded_audio_speech_bounds(audio, segment.start_seconds, segment.end_seconds).is_none()
        })
}

/// Helper `findNearestSameSpeakerAutoSegmentRow`: the closest other row of `rows` on `silent`'s
/// lane (overlapping rows are at distance 0); on a tie a row before `silent` wins over one after.
pub fn nearest_same_lane_segment<'a>(
    rows: &[&'a Segment],
    silent: &Segment,
) -> Option<&'a Segment> {
    let mut best: Option<(&Segment, f64, bool)> = None;
    for candidate in rows {
        if candidate.id == silent.id || candidate.lane_id != silent.lane_id {
            continue;
        }
        let before = candidate.end_seconds <= silent.start_seconds;
        let distance = if before {
            silent.start_seconds - candidate.end_seconds
        } else if candidate.start_seconds >= silent.end_seconds {
            candidate.start_seconds - silent.end_seconds
        } else {
            0.0
        };
        let better = best.is_none_or(|(_, best_distance, best_before)| {
            distance < best_distance || (distance == best_distance && before && !best_before)
        });
        if better {
            best = Some((candidate, distance, before));
        }
    }
    best.map(|(segment, _, _)| segment)
}

pub fn visible_transcription_segments<'a>(
    project: &'a Project,
    track_filter: &TrackFilter,
    collapsed_lane_ids: &std::collections::BTreeSet<String>,
    environment: &DocumentEnvironment,
) -> Vec<&'a Segment> {
    let mut segments = project
        .segments
        .iter()
        .filter(|segment| track_filter_matches_lane(track_filter, &segment.lane_id))
        .filter(|segment| !collapsed_lane_ids.contains(&segment.lane_id))
        .filter(|segment| environment.is_text_segment(segment))
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

pub fn audio_trim_plan_for_segment(
    project: &Project,
    editor: &EditorSession,
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

pub fn decoded_audio_speech_bounds(
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

pub fn decoded_audio_speech_islands(
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
    environment: &DocumentEnvironment,
    lane_id: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> bool {
    project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == lane_id)
        .filter(|segment| environment.is_text_segment(segment))
        .any(|segment| segment.start_seconds < end_seconds && start_seconds < segment.end_seconds)
}

fn same_lane_open_range_limits(
    project: &Project,
    environment: &DocumentEnvironment,
    lane_id: &str,
    start_seconds: f64,
    end_seconds: f64,
) -> (f64, f64) {
    let previous_end = project
        .segments
        .iter()
        .filter(|candidate| candidate.lane_id == lane_id)
        .filter(|candidate| environment.is_text_segment(candidate))
        .filter(|candidate| candidate.end_seconds <= start_seconds)
        .map(|candidate| candidate.end_seconds)
        .max_by(f64::total_cmp)
        .unwrap_or(f64::NEG_INFINITY);
    let next_start = project
        .segments
        .iter()
        .filter(|candidate| candidate.lane_id == lane_id)
        .filter(|candidate| environment.is_text_segment(candidate))
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
