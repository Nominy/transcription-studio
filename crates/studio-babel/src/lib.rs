use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use studio_core::{AudioLane, Project, Segment, Speaker};
use thiserror::Error;

const META_PROJECT_PRESET: &str = "project_preset";
const META_EXTRA_PACKAGE: &str = "babel.extra.package";
const META_EXTRA_SPEAKER_PREFIX: &str = "babel.extra.speaker.";
const META_EXTRA_TRACK_PREFIX: &str = "babel.extra.track.";
const META_EXTRA_ANNOTATION_PREFIX: &str = "babel.extra.annotation.";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BabelTaskPackage {
    pub task_id: String,
    pub project_preset: String,
    pub speakers: Vec<BabelSpeaker>,
    pub audio_tracks: Vec<BabelAudioTrack>,
    pub annotations: Vec<BabelAnnotation>,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BabelSpeaker {
    pub speaker_key: String,
    pub label: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BabelAudioTrack {
    pub track_id: String,
    pub speaker_key: String,
    pub path: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BabelAnnotation {
    pub annotation_id: String,
    pub speaker_key: String,
    pub track_id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    #[serde(default, flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BabelImportError {
    #[error("duplicate speaker key `{0}`")]
    DuplicateSpeaker(String),
    #[error("duplicate audio track id `{0}`")]
    DuplicateTrack(String),
    #[error("duplicate annotation id `{0}`")]
    DuplicateAnnotation(String),
    #[error("annotation references unknown speaker `{0}`")]
    UnknownSpeaker(String),
    #[error("annotation references unknown audio track `{0}`")]
    UnknownTrack(String),
    #[error("audio track `{track_id}` references unknown speaker `{speaker_key}`")]
    UnknownTrackSpeaker {
        track_id: String,
        speaker_key: String,
    },
    #[error(
        "annotation `{annotation_id}` speaker `{speaker_key}` does not match track `{track_id}` speaker `{track_speaker_key}`"
    )]
    AnnotationTrackSpeakerMismatch {
        annotation_id: String,
        speaker_key: String,
        track_id: String,
        track_speaker_key: String,
    },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum BabelExportError {
    #[error("project metadata is missing project_preset")]
    MissingProjectPreset,
}

pub fn import_package(package: BabelTaskPackage) -> Result<Project, BabelImportError> {
    validate_unique_ids(&package)?;

    let speaker_labels_by_key: BTreeMap<_, _> = package
        .speakers
        .iter()
        .map(|speaker| (speaker.speaker_key.clone(), speaker.label.clone()))
        .collect();
    let track_speakers_by_id: BTreeMap<_, _> = package
        .audio_tracks
        .iter()
        .map(|track| (track.track_id.clone(), track.speaker_key.clone()))
        .collect();

    for track in &package.audio_tracks {
        if !speaker_labels_by_key.contains_key(&track.speaker_key) {
            return Err(BabelImportError::UnknownTrackSpeaker {
                track_id: track.track_id.clone(),
                speaker_key: track.speaker_key.clone(),
            });
        }
    }

    for annotation in &package.annotations {
        if !speaker_labels_by_key.contains_key(&annotation.speaker_key) {
            return Err(BabelImportError::UnknownSpeaker(
                annotation.speaker_key.clone(),
            ));
        }
        let track_speaker_key = track_speakers_by_id
            .get(&annotation.track_id)
            .ok_or_else(|| BabelImportError::UnknownTrack(annotation.track_id.clone()))?;
        if track_speaker_key != &annotation.speaker_key {
            return Err(BabelImportError::AnnotationTrackSpeakerMismatch {
                annotation_id: annotation.annotation_id.clone(),
                speaker_key: annotation.speaker_key.clone(),
                track_id: annotation.track_id.clone(),
                track_speaker_key: track_speaker_key.clone(),
            });
        }
    }

    let mut metadata = BTreeMap::new();
    metadata.insert(
        META_PROJECT_PRESET.to_string(),
        package.project_preset.clone(),
    );
    store_extra(&mut metadata, META_EXTRA_PACKAGE, &package.extra);
    for speaker in &package.speakers {
        store_extra(
            &mut metadata,
            &metadata_key(META_EXTRA_SPEAKER_PREFIX, &speaker.speaker_key),
            &speaker.extra,
        );
    }
    for track in &package.audio_tracks {
        store_extra(
            &mut metadata,
            &metadata_key(META_EXTRA_TRACK_PREFIX, &track.track_id),
            &track.extra,
        );
    }
    for annotation in &package.annotations {
        store_extra(
            &mut metadata,
            &metadata_key(META_EXTRA_ANNOTATION_PREFIX, &annotation.annotation_id),
            &annotation.extra,
        );
    }

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
                label: speaker_labels_by_key
                    .get(&track.speaker_key)
                    .cloned()
                    .unwrap_or_else(|| track.path.clone()),
                speaker_id: track.speaker_key,
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

fn validate_unique_ids(package: &BabelTaskPackage) -> Result<(), BabelImportError> {
    let mut speaker_keys = BTreeSet::new();
    for speaker in &package.speakers {
        if !speaker_keys.insert(speaker.speaker_key.as_str()) {
            return Err(BabelImportError::DuplicateSpeaker(
                speaker.speaker_key.clone(),
            ));
        }
    }

    let mut track_ids = BTreeSet::new();
    for track in &package.audio_tracks {
        if !track_ids.insert(track.track_id.as_str()) {
            return Err(BabelImportError::DuplicateTrack(track.track_id.clone()));
        }
    }

    let mut annotation_ids = BTreeSet::new();
    for annotation in &package.annotations {
        if !annotation_ids.insert(annotation.annotation_id.as_str()) {
            return Err(BabelImportError::DuplicateAnnotation(
                annotation.annotation_id.clone(),
            ));
        }
    }

    Ok(())
}

pub fn export_package(project: &Project) -> Result<BabelTaskPackage, BabelExportError> {
    let project_preset = project
        .metadata
        .get(META_PROJECT_PRESET)
        .cloned()
        .ok_or(BabelExportError::MissingProjectPreset)?;

    Ok(BabelTaskPackage {
        task_id: project.id.clone(),
        project_preset,
        extra: load_extra(&project.metadata, META_EXTRA_PACKAGE),
        speakers: project
            .speakers
            .iter()
            .map(|speaker| BabelSpeaker {
                speaker_key: speaker.id.clone(),
                label: speaker.label.clone(),
                extra: load_extra(
                    &project.metadata,
                    &metadata_key(META_EXTRA_SPEAKER_PREFIX, &speaker.id),
                ),
            })
            .collect(),
        audio_tracks: project
            .lanes
            .iter()
            .map(|lane| BabelAudioTrack {
                track_id: lane.id.clone(),
                speaker_key: lane.speaker_id.clone(),
                path: lane.source_path.clone(),
                extra: load_extra(
                    &project.metadata,
                    &metadata_key(META_EXTRA_TRACK_PREFIX, &lane.id),
                ),
            })
            .collect(),
        annotations: project
            .segments
            .iter()
            .map(|segment| {
                let annotation_id = segment
                    .source_annotation_id
                    .clone()
                    .unwrap_or_else(|| segment.id.clone());
                BabelAnnotation {
                    extra: load_extra(
                        &project.metadata,
                        &metadata_key(META_EXTRA_ANNOTATION_PREFIX, &annotation_id),
                    ),
                    annotation_id,
                    speaker_key: segment.speaker_id.clone(),
                    track_id: segment.lane_id.clone(),
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                    text: segment.text.clone(),
                }
            })
            .collect(),
    })
}

fn metadata_key(prefix: &str, id: &str) -> String {
    format!("{prefix}{id}")
}

fn store_extra(
    metadata: &mut BTreeMap<String, String>,
    key: &str,
    extra: &BTreeMap<String, Value>,
) {
    if extra.is_empty() {
        return;
    }

    let encoded = serde_json::to_string(extra).expect("Babel extra metadata should encode");
    metadata.insert(key.to_string(), encoded);
}

fn load_extra(metadata: &BTreeMap<String, String>, key: &str) -> BTreeMap<String, Value> {
    metadata
        .get(key)
        .and_then(|encoded| serde_json::from_str(encoded).ok())
        .unwrap_or_default()
}
