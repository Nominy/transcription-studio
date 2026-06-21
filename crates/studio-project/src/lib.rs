use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use studio_core::{AudioLane, Project, Segment, Speaker};
use thiserror::Error;

const PROJECT_FILE: &str = "project.json";
const SEGMENTS_FILE: &str = "segments.json";
const SUPPORT_DIRS: [&str; 4] = ["audio", "rules", "journal", "autosave"];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ProjectManifest {
    id: String,
    name: String,
    speakers: Vec<Speaker>,
    lanes: Vec<AudioLane>,
    metadata: BTreeMap<String, String>,
}

#[derive(Debug, Error)]
pub enum ProjectIoError {
    #[error("failed to create directory `{path}`")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write `{path}`")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to read `{path}`")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to encode project JSON")]
    EncodeJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to decode `{path}`")]
    DecodeJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub fn save_project(project: &Project, root: impl AsRef<Path>) -> Result<(), ProjectIoError> {
    let root = root.as_ref();
    create_dir(root)?;
    for support_dir in SUPPORT_DIRS {
        create_dir(&root.join(support_dir))?;
    }

    let manifest = ProjectManifest {
        id: project.id.clone(),
        name: project.name.clone(),
        speakers: project.speakers.clone(),
        lanes: project.lanes.clone(),
        metadata: project.metadata.clone(),
    };

    write_json(&root.join(PROJECT_FILE), &manifest)?;
    write_json(&root.join(SEGMENTS_FILE), &project.segments)?;
    Ok(())
}

pub fn load_project(root: impl AsRef<Path>) -> Result<Project, ProjectIoError> {
    let root = root.as_ref();
    let manifest: ProjectManifest = read_json(&root.join(PROJECT_FILE))?;
    let segments: Vec<Segment> = read_json(&root.join(SEGMENTS_FILE))?;

    Ok(Project {
        id: manifest.id,
        name: manifest.name,
        speakers: manifest.speakers,
        lanes: manifest.lanes,
        segments,
        metadata: manifest.metadata,
    })
}

fn create_dir(path: &Path) -> Result<(), ProjectIoError> {
    fs::create_dir_all(path).map_err(|source| ProjectIoError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json<T>(path: &Path, value: &T) -> Result<(), ProjectIoError>
where
    T: Serialize,
{
    let json = serde_json::to_string_pretty(value)
        .map_err(|source| ProjectIoError::EncodeJson { source })?;
    fs::write(path, json).map_err(|source| ProjectIoError::WriteFile {
        path: path.to_path_buf(),
        source,
    })
}

fn read_json<T>(path: &Path) -> Result<T, ProjectIoError>
where
    T: for<'de> Deserialize<'de>,
{
    let json = fs::read_to_string(path).map_err(|source| ProjectIoError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&json).map_err(|source| ProjectIoError::DecodeJson {
        path: path.to_path_buf(),
        source,
    })
}
