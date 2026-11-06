use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde_json::Value;
use studio_core::{EditorCommand, Segment, SegmentUpdate};

use super::{StudioWorkflow, compare_seconds};

const INSERT_DEFAULT_DURATION_SECONDS: f64 = 0.8;

impl StudioWorkflow {
    pub fn apply_segment_edit(
        &mut self,
        segment_id: &str,
        start_seconds: f64,
        end_seconds: f64,
        text: impl Into<String>,
    ) -> Result<()> {
        self.apply(EditorCommand::UpdateSegment {
            segment_id: segment_id.to_string(),
            start_seconds,
            end_seconds,
            text: text.into(),
        })
    }

    pub fn apply_segment_edits(&mut self, updates: Vec<SegmentUpdate>) -> Result<()> {
        self.apply(EditorCommand::UpdateSegments { updates })
    }

    pub fn split_segment(&mut self, segment_id: &str, at_seconds: f64) -> Result<String> {
        let new_segment_id = self.next_derived_segment_id(segment_id);
        self.apply(EditorCommand::SplitSegment {
            segment_id: segment_id.to_string(),
            at_seconds,
            new_segment_id: new_segment_id.clone(),
        })?;
        Ok(new_segment_id)
    }

    pub fn split_segment_with_text(
        &mut self,
        segment_id: &str,
        at_seconds: f64,
        left_text: impl Into<String>,
        right_text: impl Into<String>,
    ) -> Result<String> {
        let new_segment_id = self.next_derived_segment_id(segment_id);
        self.apply(EditorCommand::SplitSegmentWithText {
            segment_id: segment_id.to_string(),
            at_seconds,
            new_segment_id: new_segment_id.clone(),
            left_text: left_text.into(),
            right_text: right_text.into(),
        })?;
        Ok(new_segment_id)
    }

    pub fn split_segment_with_project_text_ratio(
        &mut self,
        segment_id: &str,
        at_seconds: f64,
    ) -> Result<String> {
        let segment = self
            .project
            .segment(segment_id)
            .with_context(|| format!("segment `{segment_id}` was not found"))?;
        let duration = segment.end_seconds - segment.start_seconds;
        let ratio = if duration.is_finite() && duration > 0.0 {
            (at_seconds - segment.start_seconds) / duration
        } else {
            0.0
        };
        let (left_text, right_text) = split_text_by_word_ratio(&segment.text, ratio);
        self.split_segment_with_text(segment_id, at_seconds, left_text, right_text)
    }

    /// Cuts `[start, end)` out of a lane; a row covering both edges gets `split_texts` (left,
    /// right) or keeps its full text on both halves.
    pub fn cut_lane_range(
        &mut self,
        lane_id: &str,
        start_seconds: f64,
        end_seconds: f64,
        split_texts: Option<(String, String)>,
    ) -> Result<()> {
        self.apply(EditorCommand::CutLaneRange {
            lane_id: lane_id.to_string(),
            start_seconds,
            end_seconds,
            split_texts,
        })
    }

    /// Merges `right_segment_id` into `left_segment_id`: two rows of one lane, the left one
    /// ending before the right one starts.
    pub fn merge_segments(&mut self, left_segment_id: &str, right_segment_id: &str) -> Result<()> {
        self.apply(EditorCommand::MergeSegments {
            left_segment_id: left_segment_id.to_string(),
            right_segment_id: right_segment_id.to_string(),
        })
    }

    pub fn merge_with_next(&mut self, left_segment_id: &str) -> Result<()> {
        let left = self
            .project
            .segment(left_segment_id)
            .with_context(|| format!("segment `{left_segment_id}` was not found"))?;
        let left_speaker_id = left.speaker_id.clone();
        let left_lane_id = left.lane_id.clone();
        let left_end_seconds = left.end_seconds;
        let right_segment_id = self
            .project
            .segments
            .iter()
            .filter(|segment| {
                segment.id != left_segment_id
                    && segment.speaker_id == left_speaker_id
                    && segment.lane_id == left_lane_id
                    && left_end_seconds <= segment.start_seconds
            })
            .min_by(|left, right| compare_seconds(left.start_seconds, right.start_seconds))
            .map(|segment| segment.id.clone())
            .with_context(|| {
                format!("segment `{left_segment_id}` has no next segment on its lane")
            })?;

        self.apply(EditorCommand::MergeSegments {
            left_segment_id: left_segment_id.to_string(),
            right_segment_id,
        })
    }

    pub fn merge_with_previous(&mut self, right_segment_id: &str) -> Result<()> {
        let right = self
            .project
            .segment(right_segment_id)
            .with_context(|| format!("segment `{right_segment_id}` was not found"))?;
        let right_speaker_id = right.speaker_id.clone();
        let right_lane_id = right.lane_id.clone();
        let right_start_seconds = right.start_seconds;
        let left_segment_id = self
            .project
            .segments
            .iter()
            .filter(|segment| {
                segment.id != right_segment_id
                    && segment.speaker_id == right_speaker_id
                    && segment.lane_id == right_lane_id
                    && segment.end_seconds <= right_start_seconds
            })
            .max_by(|left, right| compare_seconds(left.end_seconds, right.end_seconds))
            .map(|segment| segment.id.clone())
            .with_context(|| {
                format!("segment `{right_segment_id}` has no previous segment on its lane")
            })?;

        self.apply(EditorCommand::MergeSegmentsKeepingRight {
            left_segment_id,
            right_segment_id: right_segment_id.to_string(),
        })
    }

    pub fn delete_segment(&mut self, segment_id: &str) -> Result<()> {
        self.apply(EditorCommand::DeleteSegment {
            segment_id: segment_id.to_string(),
        })
    }

    pub fn create_segment_after(&mut self, anchor_segment_id: &str) -> Result<String> {
        let anchor = self
            .project
            .segment(anchor_segment_id)
            .with_context(|| format!("segment `{anchor_segment_id}` was not found"))?
            .clone();
        let new_segment_id = self.next_derived_segment_id(anchor_segment_id);
        let next_start_seconds = self
            .project
            .segments
            .iter()
            .filter(|segment| {
                segment.id != anchor.id
                    && segment.lane_id == anchor.lane_id
                    && segment.start_seconds > anchor.start_seconds
            })
            .min_by(|left, right| compare_seconds(left.start_seconds, right.start_seconds))
            .map(|segment| segment.start_seconds);
        let boundary = next_start_seconds
            .filter(|start_seconds| uses_table_boundary(*start_seconds))
            .unwrap_or(anchor.end_seconds + INSERT_DEFAULT_DURATION_SECONDS);
        let start_seconds = anchor.end_seconds.max(0.0);
        let end_seconds = (anchor.end_seconds + 0.01).max(boundary);
        let segment = Segment {
            id: new_segment_id.clone(),
            source_annotation_id: None,
            speaker_id: anchor.speaker_id,
            lane_id: anchor.lane_id,
            start_seconds,
            end_seconds,
            text: String::new(),
            metadata: Default::default(),
        };

        self.apply(EditorCommand::CreateSegment { segment })?;
        Ok(new_segment_id)
    }

    pub fn create_segment_before(&mut self, anchor_segment_id: &str) -> Result<String> {
        let anchor = self
            .project
            .segment(anchor_segment_id)
            .with_context(|| format!("segment `{anchor_segment_id}` was not found"))?
            .clone();
        let new_segment_id = self.next_derived_segment_id(anchor_segment_id);
        let previous_end_seconds = self
            .project
            .segments
            .iter()
            .filter(|segment| {
                segment.id != anchor.id
                    && segment.lane_id == anchor.lane_id
                    && segment.end_seconds <= anchor.start_seconds
            })
            .max_by(|left, right| compare_seconds(left.end_seconds, right.end_seconds))
            .map(|segment| segment.end_seconds);
        let boundary = previous_end_seconds
            .filter(|end_seconds| uses_table_boundary(*end_seconds))
            .unwrap_or((anchor.start_seconds - INSERT_DEFAULT_DURATION_SECONDS).max(0.0));
        let start_seconds = boundary.max(0.0);
        let end_seconds = (boundary + 0.01).max(anchor.start_seconds);
        let segment = Segment {
            id: new_segment_id.clone(),
            source_annotation_id: None,
            speaker_id: anchor.speaker_id,
            lane_id: anchor.lane_id,
            start_seconds,
            end_seconds,
            text: String::new(),
            metadata: Default::default(),
        };

        self.apply(EditorCommand::CreateSegment { segment })?;
        Ok(new_segment_id)
    }

    pub fn create_segment_in_lane(
        &mut self,
        lane_id: &str,
        start_seconds: f64,
        end_seconds: f64,
    ) -> Result<String> {
        self.create_segment_in_lane_with_text(lane_id, start_seconds, end_seconds, String::new())
    }

    pub fn create_segment_in_lane_with_text(
        &mut self,
        lane_id: &str,
        start_seconds: f64,
        end_seconds: f64,
        text: impl Into<String>,
    ) -> Result<String> {
        self.create_segment_in_lane_with_text_and_metadata(
            lane_id,
            start_seconds,
            end_seconds,
            text,
            BTreeMap::new(),
        )
    }

    pub fn create_segment_in_lane_with_text_and_metadata(
        &mut self,
        lane_id: &str,
        start_seconds: f64,
        end_seconds: f64,
        text: impl Into<String>,
        metadata: BTreeMap<String, Value>,
    ) -> Result<String> {
        let lane = self
            .project
            .lanes
            .iter()
            .find(|lane| lane.id == lane_id)
            .with_context(|| format!("audio lane `{lane_id}` was not found"))?
            .clone();
        let new_segment_id = self.next_derived_segment_id(lane_id);
        let segment = Segment {
            id: new_segment_id.clone(),
            source_annotation_id: None,
            speaker_id: lane.speaker_id,
            lane_id: lane.id,
            start_seconds,
            end_seconds,
            text: text.into(),
            metadata,
        };

        self.apply(EditorCommand::CreateSegment { segment })?;
        Ok(new_segment_id)
    }

    /// Removes `remove_segment_ids` and creates one row per `(lane_id, start, end, text)` as a
    /// single undo step; returns the created ids in input order.
    pub fn replace_segments_with_rows(
        &mut self,
        remove_segment_ids: Vec<String>,
        rows: Vec<(String, f64, f64, String)>,
    ) -> Result<Vec<String>> {
        let mut ids: Vec<String> = Vec::with_capacity(rows.len());
        let mut next_index: BTreeMap<String, usize> = BTreeMap::new();
        let mut segments = Vec::with_capacity(rows.len());
        for (lane_id, start_seconds, end_seconds, text) in rows {
            let lane = self
                .project
                .lanes
                .iter()
                .find(|lane| lane.id == lane_id)
                .with_context(|| format!("audio lane `{lane_id}` was not found"))?;
            let index = next_index.entry(lane_id.clone()).or_insert(1);
            let id = loop {
                let candidate = format!("{lane_id}__studio_{index}");
                *index += 1;
                if !self.project.has_segment(&candidate) {
                    break candidate;
                }
            };
            segments.push(Segment {
                id: id.clone(),
                source_annotation_id: None,
                speaker_id: lane.speaker_id.clone(),
                lane_id: lane.id.clone(),
                start_seconds,
                end_seconds,
                text,
                metadata: BTreeMap::new(),
            });
            ids.push(id);
        }
        self.apply(EditorCommand::ReplaceSegments {
            remove_segment_ids,
            segments,
        })?;
        Ok(ids)
    }

    pub fn create_segment_at_time_in_lane(
        &mut self,
        lane_id: &str,
        start_seconds: f64,
    ) -> Result<String> {
        let start_seconds = if start_seconds.is_finite() {
            start_seconds.max(0.0)
        } else {
            0.0
        };
        self.create_segment_in_lane(
            lane_id,
            start_seconds,
            start_seconds + INSERT_DEFAULT_DURATION_SECONDS,
        )
    }

    pub fn create_blank_segment_in_lane(&mut self, lane_id: &str) -> Result<String> {
        self.create_segment_at_time_in_lane(lane_id, 0.0)
    }

    pub fn change_segment_speaker(&mut self, segment_id: &str, speaker_id: &str) -> Result<()> {
        let lane_id = self
            .project
            .lanes
            .iter()
            .find(|lane| lane.speaker_id == speaker_id)
            .map(|lane| lane.id.clone())
            .with_context(|| format!("speaker `{speaker_id}` has no audio lane"))?;

        self.change_segment_track(segment_id, &lane_id)
    }

    pub fn change_segment_track(&mut self, segment_id: &str, lane_id: &str) -> Result<()> {
        let speaker_id = self
            .project
            .lanes
            .iter()
            .find(|lane| lane.id == lane_id)
            .map(|lane| lane.speaker_id.clone())
            .with_context(|| format!("audio lane `{lane_id}` was not found"))?;

        self.apply(EditorCommand::ChangeSegmentSpeaker {
            segment_id: segment_id.to_string(),
            speaker_id,
            lane_id: lane_id.to_string(),
        })
    }
}

fn uses_table_boundary(seconds: f64) -> bool {
    seconds != 0.0 && !seconds.is_nan()
}

fn split_text_by_word_ratio(text: &str, ratio: f64) -> (String, String) {
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return (String::new(), String::new());
    }

    let clamped_ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut split_count = (words.len() as f64 * clamped_ratio).round() as usize;
    if clamped_ratio > 0.0 && clamped_ratio < 1.0 && words.len() > 1 {
        split_count = split_count.clamp(1, words.len() - 1);
    } else {
        split_count = split_count.min(words.len());
    }

    (
        words[..split_count].join(" "),
        words[split_count..].join(" "),
    )
}
