//! Construct a native document from user-selected local audio. No account or platform
//! metadata participates. The returned decoded lanes can be installed directly in the editor.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use studio_audio::{LaneAudioFile, decode_audio_file};
use studio_core::{AudioLane, Project, Segment, Speaker};
use uuid::Uuid;

pub struct ImportedAudioProject {
    pub project: Project,
    pub audio_base: PathBuf,
    pub audio: Vec<(String, LaneAudioFile)>,
}

/// Called on a background executor. `progress` runs before and after every file; returning
/// false cancels without opening a partial document. A decoder call is the cancellation
/// boundary; no imports write to or modify the user's source files.
pub fn import_audio(
    paths: &[PathBuf],
    mut progress: impl FnMut(usize, usize, &Path) -> bool,
) -> Result<ImportedAudioProject> {
    if paths.is_empty() {
        bail!("Choose at least one audio file.");
    }
    let mut unique = HashSet::new();
    let mut resolved = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path
            .canonicalize()
            .with_context(|| format!("Cannot open {}", path.display()))?;
        if !path.is_file() {
            bail!("Not a file: {}", path.display());
        }
        if unique.insert(path.clone()) {
            resolved.push(path);
        }
    }
    let audio_base = resolved[0]
        .parent()
        .context("Audio source has no parent directory")?
        .to_path_buf();
    let name = if resolved.len() == 1 {
        resolved[0]
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_owned()
    } else {
        audio_base
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("New transcript")
            .to_owned()
    };
    let mut project = Project {
        id: Uuid::new_v4().to_string(),
        name,
        speakers: Vec::with_capacity(resolved.len()),
        lanes: Vec::with_capacity(resolved.len()),
        segments: Vec::with_capacity(resolved.len()),
        metadata: Default::default(),
    };
    let mut audio = Vec::with_capacity(resolved.len());
    for (index, path) in resolved.iter().enumerate() {
        if !progress(index, resolved.len(), path) {
            bail!("Audio import cancelled.");
        }
        let decoded =
            decode_audio_file(path).with_context(|| format!("Cannot decode {}", path.display()))?;
        let duration = decoded.duration();
        if !duration.is_finite() || duration <= 0.0 {
            bail!("Audio contains no playable samples: {}", path.display());
        }
        let speaker_id = Uuid::new_v4().to_string();
        let lane_id = Uuid::new_v4().to_string();
        project.speakers.push(Speaker {
            id: speaker_id.clone(),
            label: format!("Speaker {}", index + 1),
        });
        project.lanes.push(AudioLane {
            id: lane_id.clone(),
            speaker_id: speaker_id.clone(),
            label: path
                .file_stem()
                .and_then(|name| name.to_str())
                .unwrap_or("Audio")
                .to_owned(),
            source_path: path
                .to_str()
                .context("Audio path cannot be represented as UTF-8")?
                .to_owned(),
        });
        project.segments.push(Segment {
            id: Uuid::new_v4().to_string(),
            source_annotation_id: None,
            speaker_id,
            lane_id: lane_id.clone(),
            start_seconds: 0.0,
            end_seconds: duration,
            text: String::new(),
            metadata: Default::default(),
        });
        audio.push((
            lane_id,
            LaneAudioFile {
                path: path.clone(),
                audio: decoded,
            },
        ));
        if !progress(index + 1, resolved.len(), path) {
            bail!("Audio import cancelled.");
        }
    }
    Ok(ImportedAudioProject {
        project,
        audio_base,
        audio,
    })
}
