use studio_core::Project;

const META_HIDE_TIMESTAMPS: &str = "studio.babel.table.hideTimestamps";
const META_SHOW_REVIEWERS: &str = "studio.babel.diff.showReviewers";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TranscriptRowVisibleCells {
    pub id: bool,
    pub track: bool,
    pub start: bool,
    pub end: bool,
    pub reviewer: bool,
    pub text: bool,
    pub linter: bool,
    pub actions: bool,
}

pub(crate) fn derive_transcription_row_visible_cells(
    project: &Project,
) -> TranscriptRowVisibleCells {
    let hide_timestamps = metadata_bool(project, META_HIDE_TIMESTAMPS);
    TranscriptRowVisibleCells {
        id: !hide_timestamps,
        track: !hide_timestamps,
        start: !hide_timestamps,
        end: !hide_timestamps,
        reviewer: metadata_bool(project, META_SHOW_REVIEWERS),
        text: true,
        linter: true,
        actions: !hide_timestamps,
    }
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project
        .metadata
        .get(key)
        .is_some_and(|value| value == "true")
}
