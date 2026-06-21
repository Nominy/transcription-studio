use std::collections::BTreeSet;

use studio_core::{Project, Segment};

const WAVEFORM_PEAK_COUNT: usize = 28;
const DEFAULT_SPEED: f32 = 1.0;
const DEFAULT_ZOOM: f32 = 1.0;

#[derive(Clone, Debug, PartialEq)]
pub struct EditorSession {
    pub speed: f32,
    pub zoom: f32,
    pub is_playing: bool,
    pub playhead_seconds: f64,
    pub diff_view: bool,
    pub loop_enabled: bool,
    pub loop_range: TimeRange,
    pub viewport: TimeRange,
    pub track_filter: TrackFilter,
    pub selected_segment_id: Option<String>,
    pub muted_lane_ids: BTreeSet<String>,
    pub open_actions_segment_id: Option<String>,
    pub is_dirty: bool,
    pub review_state: ReviewState,
    pub support_id: String,
}

impl EditorSession {
    pub fn for_project(project: &Project) -> Self {
        let bounds = TimeRange::for_project(project);
        Self {
            speed: DEFAULT_SPEED,
            zoom: DEFAULT_ZOOM,
            is_playing: false,
            playhead_seconds: bounds.start_seconds,
            diff_view: false,
            loop_enabled: false,
            loop_range: bounds,
            viewport: bounds,
            track_filter: TrackFilter::All,
            selected_segment_id: first_segment_id(project),
            muted_lane_ids: BTreeSet::new(),
            open_actions_segment_id: None,
            is_dirty: false,
            review_state: ReviewState::Draft,
            support_id: format!("{}:{}", project.id, project.segments.len()),
        }
    }

    pub fn track_filter_options(project: &Project) -> Vec<TrackFilter> {
        let mut options = vec![TrackFilter::All];
        options.extend(
            project
                .lanes
                .iter()
                .map(|lane| TrackFilter::Lane(lane.id.clone())),
        );
        options
    }

    pub fn timeline_lanes(&self, project: &Project) -> Vec<TimelineLaneView> {
        project
            .lanes
            .iter()
            .enumerate()
            .filter(|(_, lane)| self.track_filter.matches_lane(&lane.id))
            .map(|(index, lane)| {
                let mut segments: Vec<_> = project
                    .segments
                    .iter()
                    .filter(|segment| segment.lane_id == lane.id)
                    .collect();
                segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

                TimelineLaneView {
                    lane_id: lane.id.clone(),
                    label: lane.label.clone(),
                    speaker_id: lane.speaker_id.clone(),
                    color_token: lane_color_token(index).to_string(),
                    is_muted: self.muted_lane_ids.contains(&lane.id),
                    blocks: segments
                        .into_iter()
                        .map(|segment| WaveformBlock::for_segment(segment, self.viewport))
                        .collect(),
                }
            })
            .collect()
    }

    pub fn transcript_rows(&self, project: &Project) -> Vec<TranscriptRowView> {
        let mut segments: Vec<_> = project.segments.iter().collect();
        segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

        segments
            .into_iter()
            .enumerate()
            .filter(|(_, segment)| self.track_filter.matches_lane(&segment.lane_id))
            .map(|(index, segment)| TranscriptRowView {
                segment_id: segment.id.clone(),
                display_id: format!("{:02}", index + 1),
                speaker_label: speaker_label(project, &segment.speaker_id),
                lane_id: segment.lane_id.clone(),
                start_time: format_mm_ss(segment.start_seconds),
                end_time: format_mm_ss(segment.end_seconds),
                text: segment.text.clone(),
                is_selected: self.selected_segment_id.as_deref() == Some(segment.id.as_str()),
                lint: lint_segment(project, segment),
            })
            .collect()
    }

    pub fn blocking_lint_count(&self, project: &Project) -> usize {
        self.transcript_rows(project)
            .iter()
            .filter(|row| row.lint.severity == LintSeverity::Error)
            .count()
    }

    pub fn can_submit_review(&self, project: &Project) -> bool {
        self.blocking_lint_count(project) == 0
    }

    pub fn submit_review(&mut self, project: &Project) -> Result<(), String> {
        let blocking = self.blocking_lint_count(project);
        if blocking > 0 {
            return Err(format!("{blocking} blocking lint issue(s) remain"));
        }
        self.review_state = ReviewState::Submitted;
        self.is_dirty = false;
        Ok(())
    }

    pub fn mark_dirty(&mut self) {
        self.is_dirty = true;
        if self.review_state == ReviewState::Submitted {
            self.review_state = ReviewState::Draft;
        }
    }

    pub fn mark_saved(&mut self) {
        self.is_dirty = false;
    }

    pub fn select_segment(&mut self, project: &Project, segment_id: &str) {
        if let Some(segment) = project.segment(segment_id) {
            self.selected_segment_id = Some(segment_id.to_string());
            self.playhead_seconds = segment.start_seconds;
        }
    }

    pub fn toggle_playback(&mut self) {
        self.is_playing = !self.is_playing;
    }

    pub fn set_speed(&mut self, speed: f32) {
        if speed.is_finite() && speed > 0.0 {
            self.speed = speed;
        }
    }

    pub fn set_zoom(&mut self, zoom: f32) {
        if zoom.is_finite() {
            self.zoom = zoom.clamp(0.25, 8.0);
        }
    }

    pub fn toggle_lane_mute(&mut self, lane_id: &str) {
        if !self.muted_lane_ids.insert(lane_id.to_string()) {
            self.muted_lane_ids.remove(lane_id);
        }
    }

    pub fn toggle_diff_view(&mut self) {
        self.diff_view = !self.diff_view;
    }

    pub fn toggle_loop(&mut self) {
        self.loop_enabled = !self.loop_enabled;
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl TimeRange {
    pub fn for_project(project: &Project) -> Self {
        let max_end = project
            .segments
            .iter()
            .map(|segment| segment.end_seconds)
            .filter(|seconds| seconds.is_finite())
            .fold(0.0, f64::max);
        Self {
            start_seconds: 0.0,
            end_seconds: max_end,
        }
    }

    fn duration_seconds(self) -> f64 {
        (self.end_seconds - self.start_seconds).max(0.001)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackFilter {
    All,
    Lane(String),
}

impl TrackFilter {
    pub fn label(&self) -> String {
        match self {
            TrackFilter::All => "All Tracks".to_string(),
            TrackFilter::Lane(lane_id) => label_from_lane_id(lane_id),
        }
    }

    fn matches_lane(&self, lane_id: &str) -> bool {
        match self {
            TrackFilter::All => true,
            TrackFilter::Lane(selected_lane_id) => selected_lane_id == lane_id,
        }
    }
}

impl std::fmt::Display for TrackFilter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewState {
    Draft,
    Submitted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineLaneView {
    pub lane_id: String,
    pub label: String,
    pub speaker_id: String,
    pub color_token: String,
    pub is_muted: bool,
    pub blocks: Vec<WaveformBlock>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformBlock {
    pub segment_id: String,
    pub left_percent: f32,
    pub width_percent: f32,
    pub peaks: Vec<f32>,
}

impl WaveformBlock {
    fn for_segment(segment: &Segment, viewport: TimeRange) -> Self {
        let duration = viewport.duration_seconds();
        let left_percent =
            (((segment.start_seconds - viewport.start_seconds) / duration) * 100.0) as f32;
        let width_percent =
            (((segment.end_seconds - segment.start_seconds) / duration) * 100.0) as f32;

        Self {
            segment_id: segment.id.clone(),
            left_percent: left_percent.clamp(0.0, 100.0),
            width_percent: width_percent.clamp(0.5, 100.0),
            peaks: waveform_peaks(&segment.id, WAVEFORM_PEAK_COUNT),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TranscriptRowView {
    pub segment_id: String,
    pub display_id: String,
    pub speaker_label: String,
    pub lane_id: String,
    pub start_time: String,
    pub end_time: String,
    pub text: String,
    pub is_selected: bool,
    pub lint: LintStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LintStatus {
    pub severity: LintSeverity,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LintSeverity {
    Pass,
    Warning,
    Error,
}

fn lint_segment(project: &Project, segment: &Segment) -> LintStatus {
    if !segment.start_seconds.is_finite()
        || !segment.end_seconds.is_finite()
        || segment.start_seconds >= segment.end_seconds
    {
        return LintStatus::error("start before end is required");
    }
    if !project.lanes.iter().any(|lane| lane.id == segment.lane_id) {
        return LintStatus::error("unknown track");
    }
    if segment.text.trim().is_empty() {
        return LintStatus::error("text is empty");
    }
    if overlaps_same_lane(project, segment) {
        return LintStatus::error("overlaps another segment on this track");
    }
    if segment.end_seconds - segment.start_seconds > 45.0 {
        return LintStatus::warning("long segment");
    }
    LintStatus {
        severity: LintSeverity::Pass,
        message: "OK".to_string(),
    }
}

impl LintStatus {
    fn error(message: &str) -> Self {
        Self {
            severity: LintSeverity::Error,
            message: message.to_string(),
        }
    }

    fn warning(message: &str) -> Self {
        Self {
            severity: LintSeverity::Warning,
            message: message.to_string(),
        }
    }
}

fn overlaps_same_lane(project: &Project, segment: &Segment) -> bool {
    project.segments.iter().any(|other| {
        other.id != segment.id
            && other.lane_id == segment.lane_id
            && segment.start_seconds < other.end_seconds
            && other.start_seconds < segment.end_seconds
    })
}

fn first_segment_id(project: &Project) -> Option<String> {
    project
        .segments
        .iter()
        .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
        .map(|segment| segment.id.clone())
}

pub fn format_mm_ss(seconds: f64) -> String {
    let clamped = seconds.max(0.0);
    let minutes = (clamped / 60.0).floor() as u64;
    let remaining_seconds = clamped - (minutes as f64 * 60.0);
    format!("{minutes:02}:{remaining_seconds:05.2}")
}

fn speaker_label(project: &Project, speaker_id: &str) -> String {
    project
        .speakers
        .iter()
        .find(|speaker| speaker.id == speaker_id)
        .map(|speaker| speaker.label.clone())
        .unwrap_or_else(|| speaker_id.to_string())
}

fn label_from_lane_id(lane_id: &str) -> String {
    lane_id
        .strip_prefix("track-speaker-")
        .map(|suffix| format!("Speaker {suffix}"))
        .unwrap_or_else(|| lane_id.to_string())
}

fn lane_color_token(index: usize) -> &'static str {
    match index % 4 {
        0 => "purple",
        1 => "blue",
        2 => "green",
        _ => "amber",
    }
}

fn waveform_peaks(seed: &str, count: usize) -> Vec<f32> {
    let mut hash = 0u32;
    for byte in seed.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }

    (0..count)
        .map(|index| {
            let value = hash
                .wrapping_add((index as u32).wrapping_mul(17))
                .wrapping_mul(1_103_515_245)
                .wrapping_add(12_345);
            0.18 + ((value % 100) as f32 / 100.0) * 0.78
        })
        .collect()
}
