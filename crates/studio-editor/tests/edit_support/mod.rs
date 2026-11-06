//! Shared fixtures for the `edit_*` integration tests.
#![allow(dead_code)]

use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_editor::edit::{EditorDoc, Outcome};
use studio_editor::editor_model::CutOverlay;
use studio_editor::timeline_model::{LaneWaveform, TimeRange};
use studio_waveform::{DecodedAudio, WaveformHit, WaveformHitTarget};

pub const STABLE_NATIVE_PROJECT_JSON: &str = include_str!("../fixtures/stable-native-project.json");

/// Lanes `lane-1`/`lane-2` labelled "Speaker 1"/"Speaker 2" (speakers `speaker-1`/`speaker-2`).
pub fn project(segments: Vec<Segment>) -> Project {
    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
        speakers: (1..=2)
            .map(|index| Speaker {
                id: format!("speaker-{index}"),
                label: format!("Speaker {index}"),
            })
            .collect(),
        lanes: (1..=2)
            .map(|index| AudioLane {
                id: format!("lane-{index}"),
                speaker_id: format!("speaker-{index}"),
                label: format!("Speaker {index}"),
                source_path: format!("audio/speaker-{index}.wav"),
            })
            .collect(),
        segments,
        metadata: Default::default(),
    }
}

/// Segment on `lane` (`"lane-1"`/`"lane-2"`), speaker matching the lane.
pub fn seg(id: &str, lane: &str, start: f64, end: f64, text: &str) -> Segment {
    Segment {
        id: id.to_string(),
        source_annotation_id: None,
        speaker_id: lane.replace("lane", "speaker"),
        lane_id: lane.to_string(),
        start_seconds: start,
        end_seconds: end,
        text: text.to_string(),
        metadata: Default::default(),
    }
}

pub fn doc(segments: Vec<Segment>) -> EditorDoc {
    EditorDoc::from_project(
        project(segments),
        exaudio_extension::document::DocumentEnvironment::local("test"),
    )
}

pub fn read_only_doc(segments: Vec<Segment>) -> EditorDoc {
    let project = project(segments);
    let mut environment =
        exaudio_extension::document::DocumentEnvironment::local(project.id.clone());
    environment.policy.read_only = true;
    EditorDoc::from_project(project, environment)
}

pub fn stable_sample_doc() -> EditorDoc {
    EditorDoc::from_project(
        serde_json::from_str(STABLE_NATIVE_PROJECT_JSON).expect("native fixture should parse"),
        exaudio_extension::document::DocumentEnvironment::local("test"),
    )
}

pub fn sample_doc() -> EditorDoc {
    EditorDoc::from_project(
        studio_editor::load_sample_project().expect("sample project"),
        exaudio_extension::document::DocumentEnvironment::local("test"),
    )
}

/// Mono audio: amplitude 0.1 inside `ranges`, silence elsewhere.
pub fn audio_with_speech(duration: f64, sample_rate: u32, ranges: &[(f64, f64)]) -> DecodedAudio {
    let frames = (duration * sample_rate as f64).ceil() as usize;
    let channel = (0..frames)
        .map(|index| {
            let time = index as f64 / sample_rate as f64;
            if ranges
                .iter()
                .any(|(start, end)| time >= *start && time < *end)
            {
                0.1
            } else {
                0.0
            }
        })
        .collect();
    DecodedAudio::new(sample_rate, vec![channel]).expect("valid test audio")
}

pub fn install_audio(doc: &mut EditorDoc, lane_id: &str, audio: DecodedAudio) {
    doc.set_lane_waveform(lane_id, LaneWaveform::from_decoded(audio));
}

pub fn hit(target: WaveformHitTarget, region: Option<&str>, time: f64) -> WaveformHit {
    WaveformHit {
        target,
        region_id: region.map(str::to_string),
        time_seconds: time,
    }
}

pub fn text_of(doc: &EditorDoc, segment_id: &str) -> String {
    doc.project()
        .segment(segment_id)
        .unwrap_or_else(|| panic!("segment {segment_id} exists"))
        .text
        .clone()
}

pub fn draft_text(doc: &EditorDoc, segment_id: &str) -> String {
    doc.draft(segment_id)
        .unwrap_or_else(|| panic!("draft {segment_id} exists"))
        .text()
        .to_string()
}

pub fn ids(doc: &EditorDoc) -> Vec<String> {
    doc.project()
        .segments
        .iter()
        .map(|s| s.id.clone())
        .collect()
}

pub fn ok(result: Result<Outcome, studio_editor::edit::EditError>) -> Outcome {
    result.unwrap_or_else(|error| panic!("operation failed: {error}"))
}

/// Places the (unlooped) cut preview on `lane_id` over `start..end`.
pub fn set_overlay(doc: &mut EditorDoc, lane_id: &str, start: f64, end: f64) {
    doc.session_mut().overlay = Some(CutOverlay {
        lane_id: lane_id.to_string(),
        range: TimeRange {
            start_seconds: start,
            end_seconds: end,
        },
        looping: false,
    });
}

/// Range of the cut preview (whatever its lane).
pub fn overlay_range(doc: &EditorDoc) -> Option<TimeRange> {
    doc.session().overlay.as_ref().map(|overlay| overlay.range)
}

/// Lane of the cut preview.
pub fn overlay_lane(doc: &EditorDoc) -> Option<&str> {
    doc.session()
        .overlay
        .as_ref()
        .map(|overlay| overlay.lane_id.as_str())
}
