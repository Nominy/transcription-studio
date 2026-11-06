use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub speakers: Vec<Speaker>,
    pub lanes: Vec<AudioLane>,
    pub segments: Vec<Segment>,
    pub metadata: BTreeMap<String, String>,
}

impl Project {
    pub fn segment(&self, segment_id: &str) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|segment| segment.id == segment_id)
    }

    pub fn segment_mut(&mut self, segment_id: &str) -> Option<&mut Segment> {
        self.segments
            .iter_mut()
            .find(|segment| segment.id == segment_id)
    }

    pub fn has_segment(&self, segment_id: &str) -> bool {
        self.segment(segment_id).is_some()
    }

    pub fn speaker(&self, speaker_id: &str) -> Option<&Speaker> {
        self.speakers
            .iter()
            .find(|speaker| speaker.id == speaker_id)
    }

    pub fn speaker_mut(&mut self, speaker_id: &str) -> Option<&mut Speaker> {
        self.speakers
            .iter_mut()
            .find(|speaker| speaker.id == speaker_id)
    }

    /// First unused `speaker-{n}` id, `n` counting from 1.
    pub fn next_speaker_id(&self) -> String {
        (1..)
            .map(|index| format!("speaker-{index}"))
            .find(|candidate| self.speaker(candidate).is_none())
            .expect("unbounded speaker id search")
    }

    /// `template` with `{n}` replaced by the smallest `n >= 1` that no speaker is labelled
    /// with (`Speaker {n}` → `Speaker 3` when speakers 1 and 2 exist).
    pub fn next_speaker_label(&self, template: &str) -> String {
        (1..)
            .map(|index| template.replace("{n}", &index.to_string()))
            .find(|candidate| {
                self.speakers
                    .iter()
                    .all(|speaker| speaker.label != *candidate)
            })
            .expect("unbounded speaker label search")
    }

    /// Rough estimate of the heap memory this project retains, in bytes.
    ///
    /// Used by [`crate::CommandHistory`] to cap the undo/redo stacks by
    /// retained bytes in addition to snapshot count. The estimate is
    /// deliberately cheap — it sums the owned string payloads (ids, labels,
    /// segment text), charges a flat per-item constant for struct and
    /// collection overhead, and walks `serde_json` metadata structurally
    /// instead of serializing it — so it is safe to run on every history
    /// push, but it must never be treated as an exact measurement.
    pub fn estimated_heap_bytes(&self) -> usize {
        // Flat per-item charges for struct fields, discriminant, and the
        // collection bookkeeping (length/capacity) of each entry.
        const SPEAKER_OVERHEAD_BYTES: usize = 32;
        const LANE_OVERHEAD_BYTES: usize = 48;
        const SEGMENT_OVERHEAD_BYTES: usize = 120;
        const METADATA_ENTRY_OVERHEAD_BYTES: usize = 48;

        let mut total = self.id.len() + self.name.len();
        for speaker in &self.speakers {
            total += SPEAKER_OVERHEAD_BYTES + speaker.id.len() + speaker.label.len();
        }
        for lane in &self.lanes {
            total += LANE_OVERHEAD_BYTES
                + lane.id.len()
                + lane.speaker_id.len()
                + lane.label.len()
                + lane.source_path.len();
        }
        for segment in &self.segments {
            total += SEGMENT_OVERHEAD_BYTES
                + segment.id.len()
                + segment.source_annotation_id.as_deref().map_or(0, str::len)
                + segment.speaker_id.len()
                + segment.lane_id.len()
                + segment.text.len();
            for (key, value) in &segment.metadata {
                total +=
                    METADATA_ENTRY_OVERHEAD_BYTES + key.len() + estimated_json_heap_bytes(value);
            }
        }
        for (key, value) in &self.metadata {
            total += METADATA_ENTRY_OVERHEAD_BYTES + key.len() + value.len();
        }
        total
    }
}

/// Rough heap estimate for a [`serde_json::Value`], walking the structure
/// instead of serializing it. The flat constants approximate the enum
/// discriminant plus inline storage of each node.
fn estimated_json_heap_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 8,
        serde_json::Value::Bool(_) => 16,
        serde_json::Value::Number(_) => 24,
        serde_json::Value::String(text) => 24 + text.len(),
        serde_json::Value::Array(items) => {
            24 + items.iter().map(estimated_json_heap_bytes).sum::<usize>()
        }
        serde_json::Value::Object(map) => {
            24 + map
                .iter()
                .map(|(key, value)| key.len() + estimated_json_heap_bytes(value))
                .sum::<usize>()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Speaker {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioLane {
    pub id: String,
    pub speaker_id: String,
    pub label: String,
    pub source_path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub id: String,
    pub source_annotation_id: Option<String>,
    pub speaker_id: String,
    pub lane_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, serde_json::Value>,
}
