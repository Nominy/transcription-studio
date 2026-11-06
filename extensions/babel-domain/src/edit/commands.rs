use exaudio_extension::document::{
    CommandOutcome, CommandProvider, CommandRequest, DocumentEdit, DocumentTransaction,
};
use serde::Deserialize;
use serde_json::Value;

use crate::i18n::{localized, with_detail};

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
        let segment_id = request.segment_id.ok_or_else(|| {
            localized("No annotation selected.", "Аннотация не выбрана.").to_owned()
        })?;
        let mut segment = request
            .project
            .segment(segment_id)
            .ok_or_else(|| {
                localized(
                    "The segment no longer exists.",
                    "Сегмент больше не существует.",
                )
                .to_owned()
            })?
            .clone();
        let label = match command {
            "babel.set-annotation-type" => {
                let kind = request
                    .arguments
                    .get("annotation_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        localized("Choose an annotation type.", "Выберите тип аннотации.")
                            .to_owned()
                    })?;
                segment
                    .metadata
                    .insert("type".into(), Value::String(kind.to_string()));
                localized("Annotation type changed", "Тип аннотации изменён")
            }
            "babel.update-annotation" => {
                let update = AnnotationUpdate::deserialize(request.arguments).map_err(|error| {
                    with_detail(
                        localized(
                            "Couldn't save the annotation.",
                            "Не удалось сохранить аннотацию.",
                        ),
                        error,
                    )
                })?;
                segment.text = update.text;
                segment
                    .metadata
                    .insert("intensity".into(), serde_json::json!(update.intensity));
                segment.metadata.insert("content".into(), update.content);
                localized("Annotation saved", "Аннотация сохранена")
            }
            _ => return Err(format!("Unknown Babel annotation command {command}.")),
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
