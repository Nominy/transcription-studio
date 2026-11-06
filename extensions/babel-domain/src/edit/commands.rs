use serde::Deserialize;
use serde_json::Value;
use sonus_extension::document::{
    CommandOutcome, CommandProvider, CommandRequest, DocumentEdit, DocumentTransaction,
};

/// Babel annotation operations expressed as neutral, revision-bound document edits.
#[derive(Debug, Default)]
pub struct BabelAnnotationCommandProvider;

#[derive(Deserialize)]
struct AnnotationUpdate {
    text: String,
    intensity: Option<u8>,
    content: Value,
}

impl CommandProvider for BabelAnnotationCommandProvider {
    fn command_ids(&self) -> &[&str] {
        &["babel.set-annotation-type", "babel.update-annotation"]
    }

    fn execute(
        &self,
        command: &str,
        request: &CommandRequest<'_>,
    ) -> Result<CommandOutcome, String> {
        let segment_id = request.segment_id.ok_or("No annotation selected.")?;
        let mut segment = request
            .project
            .segment(segment_id)
            .ok_or_else(|| format!("Segment `{segment_id}` was not found."))?
            .clone();
        let label = match command {
            "babel.set-annotation-type" => {
                let kind = request
                    .arguments
                    .get("annotation_type")
                    .and_then(Value::as_str)
                    .ok_or("An annotation type is required.")?;
                segment
                    .metadata
                    .insert("type".into(), Value::String(kind.to_string()));
                "Annotation type change"
            }
            "babel.update-annotation" => {
                let update = AnnotationUpdate::deserialize(request.arguments)
                    .map_err(|error| format!("Invalid annotation update: {error}"))?;
                segment.text = update.text;
                segment
                    .metadata
                    .insert("intensity".into(), serde_json::json!(update.intensity));
                segment.metadata.insert("content".into(), update.content);
                "Annotation edit"
            }
            _ => return Err(format!("Unknown Babel annotation command `{command}`.")),
        };
        Ok(CommandOutcome {
            transaction: Some(DocumentTransaction {
                label: label.into(),
                expected_revision: request.revision,
                edits: vec![DocumentEdit::ReplaceSegment { segment }],
            }),
            ..Default::default()
        })
    }
}
