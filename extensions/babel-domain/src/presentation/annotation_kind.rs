use studio_core::Segment;

const DEFAULT_BABEL_ANNOTATION_TYPE: &str = "transcription";

use studio_editor::transcript_model::TranscriptAnnotationKind;

pub fn annotation_kind(segment: &Segment) -> TranscriptAnnotationKind {
    let annotation_type = segment
        .metadata
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or(DEFAULT_BABEL_ANNOTATION_TYPE);
    if annotation_type == DEFAULT_BABEL_ANNOTATION_TYPE {
        TranscriptAnnotationKind::Transcription
    } else {
        TranscriptAnnotationKind::NonTranscription(annotation_type.to_owned())
    }
}
