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
}
