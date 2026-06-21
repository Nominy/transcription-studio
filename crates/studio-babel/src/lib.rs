use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use studio_core::{AudioLane, Project, Segment, Speaker};
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BabelTaskPackage {
    pub task_id: String,
    pub project_preset: String,
    pub speakers: Vec<BabelSpeaker>,
    pub audio_tracks: Vec<BabelAudioTrack>,
    pub annotations: Vec<BabelAnnotation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BabelSpeaker {
    pub speaker_key: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BabelAudioTrack {
    pub track_id: String,
    pub speaker_key: String,
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BabelAnnotation {
    pub annotation_id: String,
    pub speaker_key: String,
    pub track_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BabelImportError {
    #[error("annotation references unknown speaker `{0}`")]
    UnknownSpeaker(String),
    #[error("annotation references unknown audio track `{0}`")]
    UnknownTrack(String),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BabelExportError {
    #[error("project metadata is missing project_preset")]
    MissingProjectPreset,
}

pub fn import_package(package: BabelTaskPackage) -> Result<Project, BabelImportError> {
    for annotation in &package.annotations {
        if !package
            .speakers
            .iter()
            .any(|speaker| speaker.speaker_key == annotation.speaker_key)
        {
            return Err(BabelImportError::UnknownSpeaker(
                annotation.speaker_key.clone(),
            ));
        }
        if !package
            .audio_tracks
            .iter()
            .any(|track| track.track_id == annotation.track_id)
        {
            return Err(BabelImportError::UnknownTrack(annotation.track_id.clone()));
        }
    }

    let mut metadata = BTreeMap::new();
    metadata.insert("project_preset".to_string(), package.project_preset);

    Ok(Project {
        name: format!("Babel task {}", package.task_id),
        id: package.task_id,
        speakers: package
            .speakers
            .into_iter()
            .map(|speaker| Speaker {
                id: speaker.speaker_key,
                label: speaker.label,
            })
            .collect(),
        lanes: package
            .audio_tracks
            .into_iter()
            .map(|track| AudioLane {
                id: track.track_id,
                speaker_id: track.speaker_key,
                label: format!("{} audio", track.path),
                source_path: track.path,
            })
            .collect(),
        segments: package
            .annotations
            .into_iter()
            .map(|annotation| Segment {
                id: annotation.annotation_id.clone(),
                source_annotation_id: Some(annotation.annotation_id),
                speaker_id: annotation.speaker_key,
                lane_id: annotation.track_id,
                start_seconds: annotation.start_seconds,
                end_seconds: annotation.end_seconds,
                text: annotation.text,
            })
            .collect(),
        metadata,
    })
}

pub fn export_package(project: &Project) -> Result<BabelTaskPackage, BabelExportError> {
    let project_preset = project
        .metadata
        .get("project_preset")
        .cloned()
        .ok_or(BabelExportError::MissingProjectPreset)?;

    Ok(BabelTaskPackage {
        task_id: project.id.clone(),
        project_preset,
        speakers: project
            .speakers
            .iter()
            .map(|speaker| BabelSpeaker {
                speaker_key: speaker.id.clone(),
                label: speaker.label.clone(),
            })
            .collect(),
        audio_tracks: project
            .lanes
            .iter()
            .map(|lane| BabelAudioTrack {
                track_id: lane.id.clone(),
                speaker_key: lane.speaker_id.clone(),
                path: lane.source_path.clone(),
            })
            .collect(),
        annotations: project
            .segments
            .iter()
            .map(|segment| BabelAnnotation {
                annotation_id: segment
                    .source_annotation_id
                    .clone()
                    .unwrap_or_else(|| segment.id.clone()),
                speaker_key: segment.speaker_id.clone(),
                track_id: segment.lane_id.clone(),
                start_seconds: segment.start_seconds,
                end_seconds: segment.end_seconds,
                text: segment.text.clone(),
            })
            .collect(),
    })
}
