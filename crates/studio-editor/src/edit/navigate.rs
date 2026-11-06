//! Transcript navigation helpers: visible-row ordering, the "helper" focus/ghost-cursor model
//! (where the caret returns when playback pauses), and speaker-workflow lane switching.

use studio_core::{Project, Segment};

use crate::editor_model::EditorSession;
use crate::track_filter::TrackFilter;

/// Pending/last helper focus restore: which row gets the caret back, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct HelperFocusRestore {
    pub segment_id: String,
    /// Playhead when the editor lost focus (playback started).
    pub blur_playhead_seconds: f64,
    /// Playhead when this restore target was computed.
    pub restore_playhead_seconds: f64,
    /// Caret `char` offset to restore.
    pub restore_offset: usize,
    /// Created without a focused editor (space/escape from the timeline).
    pub synthetic: bool,
    /// Tab switched the ghost cursor to this row's lane: playback keeps it on that lane
    /// (Helper's manual ghost projection) instead of following whichever lane speaks.
    pub lane_locked: bool,
}

/// Ghost caret shown in an unfocused row while a restore is pending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhostCursor {
    pub segment_id: String,
    pub char_offset: usize,
}

/// Whether `segment` is a transcription row visible under the filter/collapsed lanes.
pub fn segment_is_visible(session: &EditorSession, segment: &Segment) -> bool {
    lane_is_visible(session, &segment.lane_id) && session.environment().is_text_segment(segment)
}

/// Whether a lane passes the track filter and is not collapsed.
pub fn lane_is_visible(session: &EditorSession, lane_id: &str) -> bool {
    let matches_filter = match &session.track_filter {
        TrackFilter::All => true,
        TrackFilter::Lane { id, .. } => id == lane_id,
    };
    matches_filter && !session.collapsed_lane_ids.contains(lane_id)
}

/// Visible transcription rows as `(segment_id, start_seconds)`, sorted by start.
pub fn visible_transcription_rows(
    project: &Project,
    session: &EditorSession,
) -> Vec<(String, f64)> {
    let mut rows: Vec<_> = project
        .segments
        .iter()
        .filter(|segment| segment_is_visible(session, segment))
        .map(|segment| (segment.id.clone(), segment.start_seconds))
        .collect();
    rows.sort_by(|left, right| left.1.total_cmp(&right.1));
    rows
}

/// Visible transcription segment under the playhead (earliest start, then id).
pub fn segment_id_at_playhead(project: &Project, session: &EditorSession) -> Option<String> {
    let playhead = session.playhead_seconds;
    project
        .segments
        .iter()
        .filter(|segment| segment_is_visible(session, segment))
        .filter(|segment| segment.start_seconds <= playhead && playhead < segment.end_seconds)
        .min_by(|l, r| {
            l.start_seconds
                .total_cmp(&r.start_seconds)
                .then_with(|| l.id.cmp(&r.id))
        })
        .map(|segment| segment.id.clone())
}

/// Caret offset proportional to the playhead's position inside `segment`, snapped to a word
/// boundary and never before `baseline_offset`.
pub fn proportional_restore_offset(
    text: &str,
    segment: &Segment,
    playhead_seconds: f64,
    baseline_offset: usize,
) -> usize {
    let text_len = text.chars().count();
    if text_len == 0 {
        return 0;
    }
    let duration = segment.end_seconds - segment.start_seconds;
    if !duration.is_finite() || duration <= 0.0 || !playhead_seconds.is_finite() {
        return baseline_offset.min(text_len);
    }
    let ratio = ((playhead_seconds - segment.start_seconds) / duration).clamp(0.0, 1.0);
    let offset = snap_to_word_boundary(text, (text_len as f64 * ratio).round() as usize);
    offset.max(baseline_offset).min(text_len)
}

/// Snaps a `char` offset to the nearest word start (ties go backward).
pub fn snap_to_word_boundary(text: &str, offset: usize) -> usize {
    let characters: Vec<char> = text.chars().collect();
    let len = characters.len();
    if len == 0 || offset == 0 {
        return 0;
    }
    if offset >= len {
        return len;
    }
    if characters[offset - 1].is_whitespace() {
        return offset;
    }
    let mut backward = offset;
    while backward > 0 && !characters[backward - 1].is_whitespace() {
        backward -= 1;
    }
    let mut forward = offset;
    while forward < len && !characters[forward].is_whitespace() {
        forward += 1;
    }
    if forward < len {
        forward += 1;
    }
    if offset - backward <= forward.saturating_sub(offset) {
        backward
    } else {
        forward
    }
}

/// Row the pending ghost cursor should follow during playback: keep the tracked lane while it
/// has active speech, else follow any active lane, else park at the latest started row
/// (tracked lane first). A [`HelperFocusRestore::lane_locked`] ghost stays on its lane (see
/// [`lane_row_at`]). `None` when the pending row is still under the playhead.
pub fn playback_follow_target(
    project: &Project,
    session: &EditorSession,
    pending: &HelperFocusRestore,
) -> Option<String> {
    let pending_segment = project.segment(&pending.segment_id)?;
    let playhead = session.playhead_seconds;
    if !playhead.is_finite()
        || (pending_segment.start_seconds <= playhead && playhead < pending_segment.end_seconds)
    {
        return None;
    }
    let target = if pending.lane_locked {
        lane_row_at(project, session, &pending_segment.lane_id, playhead)
    } else {
        let visible = || {
            project
                .segments
                .iter()
                .filter(|segment| segment_is_visible(session, segment))
        };
        let same_lane = |segment: &&Segment| segment.lane_id == pending_segment.lane_id;
        let active = |segment: &&Segment| is_active(segment, playhead);
        let started = |segment: &&Segment| segment.start_seconds <= playhead;
        visible()
            .filter(same_lane)
            .filter(active)
            .min_by(by_start)
            .or_else(|| visible().filter(active).min_by(by_start))
            .or_else(|| visible().filter(same_lane).filter(started).max_by(by_end))
            .or_else(|| visible().filter(started).max_by(by_end))
    };
    target
        .map(|segment| segment.id.clone())
        .filter(|id| *id != pending.segment_id)
}

/// Visible transcription row of `lane_id` for `seconds`: the one under it, else the latest
/// one already started (Helper `findLaneEntryByPlaybackTime`), else the lane's first row.
pub fn lane_row_at<'a>(
    project: &'a Project,
    session: &EditorSession,
    lane_id: &str,
    seconds: f64,
) -> Option<&'a Segment> {
    let lane = || {
        project.segments.iter().filter(move |segment| {
            segment.lane_id == lane_id && segment_is_visible(session, segment)
        })
    };
    lane()
        .filter(|segment| is_active(segment, seconds))
        .min_by(by_start)
        .or_else(|| {
            lane()
                .filter(|segment| segment.start_seconds <= seconds)
                .max_by(by_end)
        })
        .or_else(|| lane().min_by(by_start))
}

fn is_active(segment: &Segment, playhead: f64) -> bool {
    segment.start_seconds <= playhead && playhead < segment.end_seconds
}

fn by_start(l: &&Segment, r: &&Segment) -> std::cmp::Ordering {
    l.start_seconds
        .total_cmp(&r.start_seconds)
        .then_with(|| l.id.cmp(&r.id))
}

fn by_end(l: &&Segment, r: &&Segment) -> std::cmp::Ordering {
    l.end_seconds
        .total_cmp(&r.end_seconds)
        .then_with(|| l.id.cmp(&r.id))
}

fn normalize_speaker_workflow_label(label: &str) -> String {
    label.trim().to_ascii_lowercase().replace('-', " ")
}

/// Lanes labelled "Speaker 1"/"Speaker 2" (case/hyphen-insensitive).
pub fn speaker_workflow_lane_ids(project: &Project) -> Vec<String> {
    project
        .lanes
        .iter()
        .filter(|lane| {
            matches!(
                normalize_speaker_workflow_label(&lane.label).as_str(),
                "speaker 1" | "speaker 2"
            )
        })
        .map(|lane| lane.id.clone())
        .collect()
}

/// Lane whose label matches `label` under speaker-workflow normalization.
pub fn speaker_workflow_lane<'a>(
    project: &'a Project,
    label: &str,
) -> Option<&'a studio_core::AudioLane> {
    let wanted = normalize_speaker_workflow_label(label);
    project
        .lanes
        .iter()
        .find(|lane| normalize_speaker_workflow_label(&lane.label) == wanted)
}
