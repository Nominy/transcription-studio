//! Behavior tests for the Reviewer/Drafting backend, reviewer workspace and feedback form.

mod apply;
mod audio;
mod backend;
mod draft_stream;
mod fake_backend;
mod feedback;
mod parsing;
mod payloads;
mod workspace;

use std::collections::BTreeMap;

use exaudio_babel_domain::workflow::BabelWorkflowExt;
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_editor::workflow::StudioWorkflow;

/// Recovered Babel review action (`studio-babel` fixture) with local audio paths.
pub fn review_action_workflow() -> StudioWorkflow {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../crates/studio-babel/tests/fixtures/review-action-final.json"
    ))
    .expect("review action fixture should parse");
    let uri_map = value
        .pointer_mut("/0/result/data/json/processedRecordingUriMap")
        .and_then(serde_json::Value::as_object_mut)
        .expect("fixture should include processed recording URIs");
    for (index, source) in uri_map.values_mut().enumerate() {
        *source = serde_json::Value::String(format!("audio/speaker_{}.wav", index + 1));
    }
    StudioWorkflow::from_babel_json(&value.to_string()).expect("review action should import")
}

/// Two speakers, three segments: `s1` (spk-1), `s2` (spk-2), `s3` (spk-1).
pub fn sample_project() -> Project {
    let segment = |id: &str, speaker: &str, start: f64, end: f64, text: &str| Segment {
        id: id.to_string(),
        source_annotation_id: None,
        speaker_id: speaker.to_string(),
        lane_id: format!("lane-{speaker}"),
        start_seconds: start,
        end_seconds: end,
        text: text.to_string(),
        metadata: BTreeMap::new(),
    };
    Project {
        id: "project-1".to_string(),
        name: "Sample".to_string(),
        speakers: ["spk-1", "spk-2"]
            .into_iter()
            .map(|id| Speaker {
                id: id.to_string(),
                label: id.to_string(),
            })
            .collect(),
        lanes: ["spk-1", "spk-2"]
            .into_iter()
            .enumerate()
            .map(|(index, speaker)| AudioLane {
                id: format!("lane-{speaker}"),
                speaker_id: speaker.to_string(),
                label: format!("Speaker {}", index + 1),
                source_path: format!("audio/{speaker}.wav"),
            })
            .collect(),
        segments: vec![
            segment("s1", "spk-1", 0.0, 2.0, "First row."),
            segment("s2", "spk-2", 2.0, 4.0, "Second row."),
            segment("s3", "spk-1", 4.0, 6.0, "Third row."),
        ],
        metadata: BTreeMap::new(),
    }
}

pub fn sample_workflow() -> StudioWorkflow {
    StudioWorkflow::from_project(sample_project())
}
