use studio_core::Project;

const META_SUBMIT_MODE: &str = "studio.babel.submit.mode";
const META_CATEGORY_NAME: &str = "studio.babel.category.name";
const META_CATEGORY_USES_HIERARCHICAL_LABELS: &str = "studio.babel.category.usesHierarchicalLabels";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptEmptyStateView {
    pub col_span: usize,
    pub title: String,
    pub description: String,
}

pub(crate) fn build_transcript_empty_state(project: &Project) -> TranscriptEmptyStateView {
    if is_annotation_mode(project) {
        let category_name = project
            .metadata
            .get(META_CATEGORY_NAME)
            .map(|name| name.to_lowercase());
        return TranscriptEmptyStateView {
            col_span: if metadata_bool(project, META_CATEGORY_USES_HIERARCHICAL_LABELS) {
                8
            } else {
                6
            },
            title: "No annotations yet".to_string(),
            description: match category_name {
                Some(category_name) if !category_name.is_empty() => format!(
                    "Click and drag on the waveform above to annotate regions with {category_name}."
                ),
                _ => {
                    "Click and drag on the waveform above to create annotation regions.".to_string()
                }
            },
        };
    }

    TranscriptEmptyStateView {
        col_span: 7,
        title: "No transcriptions yet".to_string(),
        description:
            "Click and drag on the waveform above to create a region, then add your transcription text."
                .to_string(),
    }
}

fn is_annotation_mode(project: &Project) -> bool {
    project
        .metadata
        .get(META_SUBMIT_MODE)
        .is_some_and(|mode| mode == "annotation")
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project
        .metadata
        .get(key)
        .is_some_and(|value| value == "true")
}
