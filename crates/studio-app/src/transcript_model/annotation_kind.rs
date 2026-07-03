use studio_core::Segment;

const DEFAULT_BABEL_ANNOTATION_TYPE: &str = "transcription";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptAnnotationKind {
    Transcription,
    NonTranscription(String),
}

impl TranscriptAnnotationKind {
    pub fn from_segment(segment: &Segment) -> Self {
        let annotation_type = segment
            .metadata
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or(DEFAULT_BABEL_ANNOTATION_TYPE);

        if annotation_type == DEFAULT_BABEL_ANNOTATION_TYPE {
            Self::Transcription
        } else {
            Self::NonTranscription(annotation_type.to_string())
        }
    }

    pub fn is_transcription(&self) -> bool {
        matches!(self, Self::Transcription)
    }

    pub fn label(&self) -> String {
        match self {
            Self::Transcription => DEFAULT_BABEL_ANNOTATION_TYPE.to_string(),
            Self::NonTranscription(annotation_type) => annotation_type.clone(),
        }
    }
}

pub const BABEL_TRANSCRIPTION_TYPE: &str = DEFAULT_BABEL_ANNOTATION_TYPE;
pub const BABEL_NON_TRANSCRIPTION_TYPE: &str = "non-transcription";
