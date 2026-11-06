//! Construct a native document from user-selected local audio. No account or platform
//! metadata participates: one speaker and lane per file, no segments (the user draws or
//! drafts them). The returned decoded lanes can be installed directly in the editor.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use studio_audio::{LaneAudioFile, decode_audio_file};
use studio_core::{AudioLane, Project, Speaker};
use uuid::Uuid;

pub struct ImportedAudioProject {
    pub project: Project,
    pub audio_base: PathBuf,
    pub audio: Vec<(String, LaneAudioFile)>,
}

/// User-visible strings for the import, chosen by the caller in the UI language.
/// `{n}` and `{file}` are substituted.
#[derive(Clone, Copy)]
pub struct ImportStrings {
    pub new_project: &'static str,
    pub speaker: &'static str,
    pub track: &'static str,
    pub no_files: &'static str,
    pub cannot_open: &'static str,
    pub not_a_file: &'static str,
    pub cannot_decode: &'static str,
    pub no_audio: &'static str,
    pub cancelled: &'static str,
}

impl ImportStrings {
    pub const EN: Self = Self {
        new_project: "New project",
        speaker: "Speaker {n}",
        track: "Track {n}",
        no_files: "Choose at least one audio file.",
        cannot_open: "Couldn't open {file}.",
        not_a_file: "{file} is not a file.",
        cannot_decode: "Couldn't read audio from {file}.",
        no_audio: "{file} has no playable audio.",
        cancelled: "Cancelled.",
    };
    pub const RU: Self = Self {
        new_project: "Новый проект",
        speaker: "Спикер {n}",
        track: "Дорожка {n}",
        no_files: "Выберите хотя бы один аудиофайл.",
        cannot_open: "Не удалось открыть {file}.",
        not_a_file: "{file} — не файл.",
        cannot_decode: "Не удалось прочитать аудио из {file}.",
        no_audio: "В {file} нет воспроизводимого звука.",
        cancelled: "Отменено.",
    };
}

fn with_file(template: &str, path: &Path) -> String {
    template.replace(
        "{file}",
        &path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string()),
    )
}

/// Called on a background executor. `progress` runs before and after every file; returning
/// false cancels without opening a partial document. A decoder call is the cancellation
/// boundary; no imports write to or modify the user's source files.
pub fn import_audio(
    paths: &[PathBuf],
    strings: ImportStrings,
    mut progress: impl FnMut(usize, usize, &Path) -> bool,
) -> Result<ImportedAudioProject> {
    if paths.is_empty() {
        bail!("{}", strings.no_files);
    }
    let mut unique = HashSet::new();
    let mut resolved = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path
            .canonicalize()
            .with_context(|| with_file(strings.cannot_open, path))?;
        if !path.is_file() {
            bail!("{}", with_file(strings.not_a_file, &path));
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
            .unwrap_or(strings.new_project)
            .to_owned()
    } else {
        strings.new_project.to_owned()
    };
    let mut project = Project {
        id: Uuid::new_v4().to_string(),
        name,
        speakers: Vec::with_capacity(resolved.len()),
        lanes: Vec::with_capacity(resolved.len()),
        segments: Vec::new(),
        metadata: Default::default(),
    };
    let mut audio = Vec::with_capacity(resolved.len());
    for (index, path) in resolved.iter().enumerate() {
        if !progress(index, resolved.len(), path) {
            bail!("{}", strings.cancelled);
        }
        let decoded =
            decode_audio_file(path).with_context(|| with_file(strings.cannot_decode, path))?;
        let duration = decoded.duration();
        if !duration.is_finite() || duration <= 0.0 {
            bail!("{}", with_file(strings.no_audio, path));
        }
        let speaker_id = Uuid::new_v4().to_string();
        let lane_id = Uuid::new_v4().to_string();
        project.speakers.push(Speaker {
            id: speaker_id.clone(),
            label: strings.speaker.replace("{n}", &(index + 1).to_string()),
        });
        project.lanes.push(AudioLane {
            id: lane_id.clone(),
            speaker_id,
            label: path
                .file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| strings.track.replace("{n}", &(index + 1).to_string())),
            source_path: path
                .to_str()
                .context("Audio path cannot be represented as UTF-8")?
                .to_owned(),
        });
        audio.push((
            lane_id,
            LaneAudioFile {
                path: path.clone(),
                audio: decoded,
            },
        ));
        if !progress(index + 1, resolved.len(), path) {
            bail!("{}", strings.cancelled);
        }
    }
    Ok(ImportedAudioProject {
        project,
        audio_base,
        audio,
    })
}
