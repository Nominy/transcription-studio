use anyhow::{Context, Result};
use studio_babel::{import_package, BabelTaskPackage};
use studio_core::{AudioLane, Project, Segment};

const SAMPLE_TASK_JSON: &str = include_str!("../assets/sample-task.json");

#[derive(Clone, Debug)]
pub struct TimelineLaneRow<'a> {
    pub lane: &'a AudioLane,
    pub segments: Vec<&'a Segment>,
}

pub fn load_sample_project() -> Result<Project> {
    let package: BabelTaskPackage =
        serde_json::from_str(SAMPLE_TASK_JSON).context("sample Babel fixture should parse")?;
    import_package(package).context("sample Babel fixture should import")
}

pub fn timeline_lane_rows(project: &Project) -> Vec<TimelineLaneRow<'_>> {
    project
        .lanes
        .iter()
        .map(|lane| {
            let mut segments: Vec<_> = project
                .segments
                .iter()
                .filter(|segment| segment.lane_id == lane.id)
                .collect();
            segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

            TimelineLaneRow { lane, segments }
        })
        .collect()
}

pub fn transport_status(project: &Project) -> String {
    format!("{} | {} segments", project.name, project.segments.len())
}
