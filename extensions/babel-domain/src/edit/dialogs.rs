use studio_editor::edit::{EditError, EditResult, EditorDoc, Effect, Outcome};

use super::annotation::EditAnnotationDialog;
use super::create_region::{CreateRegionDialog, CreateRegionMode, CreateRegionTimeField};
use crate::i18n::localized;
use crate::workflow::BabelWorkflowExt;

/// Domain-owned annotation dialogs; the native document retains only plain-text region UI.
#[derive(Debug, Default)]
pub struct BabelAnnotationDialogs {
    pub create_region: Option<CreateRegionDialog>,
    pub edit_annotation: Option<EditAnnotationDialog>,
}

impl BabelAnnotationDialogs {
    pub fn open_create_region(&mut self, doc: &EditorDoc, lane_id: &str, start: f64, end: f64) {
        self.create_region = Some(CreateRegionDialog::new_for_project(
            doc.project(),
            lane_id.to_string(),
            start,
            end,
        ));
    }

    /// Target of the shell's `exaudio.babel.open-create-region` range command.
    pub fn open_create_region_command(
        &mut self,
        doc: &EditorDoc,
        arguments: &serde_json::Value,
    ) -> EditResult {
        if doc.environment().policy.read_only {
            return Err(EditError::ReadOnly {
                action: localized("Create", "Создание"),
            });
        }
        let lane_id = arguments
            .get("lane_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                EditError::Unavailable(localized("Choose a track.", "Выберите дорожку.").into())
            })?;
        let start = arguments
            .get("start_seconds")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                EditError::Unavailable(
                    localized("Set a start time.", "Укажите время начала.").into(),
                )
            })?;
        let end = arguments
            .get("end_seconds")
            .and_then(serde_json::Value::as_f64)
            .ok_or_else(|| {
                EditError::Unavailable(localized("Set an end time.", "Укажите время конца.").into())
            })?;
        if !doc.project().lanes.iter().any(|lane| lane.id == lane_id)
            || !start.is_finite()
            || !end.is_finite()
            || start < 0.0
            || start >= end
        {
            return Err(EditError::Unavailable(
                localized(
                    "The segment range is invalid.",
                    "Недопустимый диапазон сегмента.",
                )
                .into(),
            ));
        }
        self.open_create_region(doc, lane_id, start, end);
        Ok(Outcome::none())
    }

    pub fn create_region(&self) -> Option<&CreateRegionDialog> {
        self.create_region.as_ref()
    }
    pub fn create_region_mut(&mut self) -> Option<&mut CreateRegionDialog> {
        self.create_region.as_mut()
    }
    pub fn edit_annotation(&self) -> Option<&EditAnnotationDialog> {
        self.edit_annotation.as_ref()
    }
    pub fn edit_annotation_mut(&mut self) -> Option<&mut EditAnnotationDialog> {
        self.edit_annotation.as_mut()
    }
    pub fn cancel_create_region(&mut self) {
        self.create_region = None;
    }
    pub fn cancel_edit_annotation(&mut self) {
        self.edit_annotation = None;
    }

    pub fn set_create_region_text(&mut self, doc: &mut EditorDoc, value: String) -> EditResult {
        let Some(dialog) = self.create_region.as_mut() else {
            return Ok(Outcome::none());
        };
        dialog.set_text(value.clone());
        if let Some(segment_id) = &dialog.created_segment_id {
            let selection = doc
                .draft(segment_id)
                .map(|draft| draft.selection())
                .unwrap_or_default();
            let changed = doc
                .draft(segment_id)
                .is_some_and(|draft| draft.text() != value);
            let outcome = doc.edit_text(segment_id, value, selection)?;
            return Ok(if changed {
                outcome.with(Effect::DraftChanged {
                    segment_id: segment_id.clone(),
                })
            } else {
                outcome
            });
        }
        Ok(Outcome::none())
    }

    pub fn commit_create_region_time(&mut self, field: CreateRegionTimeField) -> EditResult {
        let dialog = self.create_region.as_mut().ok_or_else(|| {
            EditError::Unavailable(
                localized("No segment is being added.", "Нет добавляемого сегмента.").into(),
            )
        })?;
        if dialog.commit_time_field(field) {
            return Ok(Outcome::status(localized(
                "Time updated",
                "Время обновлено",
            )));
        }
        Err(EditError::Time(
            dialog
                .start_error
                .clone()
                .or_else(|| dialog.end_error.clone())
                .unwrap_or(studio_editor::edit::time::TimeEditError::InvalidFormat),
        ))
    }

    pub fn reset_create_region_time(&mut self) -> bool {
        self.create_region
            .as_mut()
            .is_some_and(CreateRegionDialog::reset_active_time_field_draft)
    }

    /// Domain dialogs have precedence over the native document's Escape handling.
    pub fn escape(&mut self) -> bool {
        if self.reset_create_region_time() {
            return true;
        }
        if self.edit_annotation.take().is_some() {
            return true;
        }
        self.create_region.take().is_some()
    }

    pub fn submit_create_region(&mut self, doc: &mut EditorDoc) -> EditResult {
        if doc.environment().policy.read_only {
            return Err(EditError::ReadOnly {
                action: localized("Create", "Создание"),
            });
        }
        let mut dialog = self.create_region.take().ok_or_else(|| {
            EditError::Unavailable(
                localized("No segment is being added.", "Нет добавляемого сегмента.").into(),
            )
        })?;
        let result = (|| {
            if dialog.mode == CreateRegionMode::Transcription
                && let Some(segment_id) = dialog.created_segment_id.as_deref()
            {
                let segment = doc
                    .project()
                    .segment(segment_id)
                    .ok_or_else(|| EditError::SegmentNotFound(segment_id.to_string()))?;
                if segment.text != dialog.text {
                    doc.edit_workflow(localized("Create", "Создание"), |workflow| {
                        workflow.apply_text_edit(segment_id, dialog.text.clone())
                    })?;
                }
                return Ok(doc
                    .select_segment_without_seek(segment_id)
                    .with_status(localized("Text saved", "Текст сохранён"))
                    .with(Effect::DraftChanged {
                        segment_id: segment_id.to_string(),
                    }));
            }
            let payload = dialog
                .validate_for_submit(doc.project())
                .map_err(|error| EditError::Unavailable(error.to_string()))?;
            let created =
                doc.edit_workflow(localized("Create", "Создание"), |workflow| {
                    workflow.create_segment_in_lane_with_text_and_metadata(
                        &payload.lane_id,
                        payload.start_seconds,
                        payload.end_seconds,
                        payload.text,
                        payload.metadata,
                    )
                })?;
            Ok(doc
                .select_segment_without_seek(&created)
                .with_status(localized("Segment added", "Сегмент добавлен")))
        })();
        if result.is_err() {
            self.create_region = Some(dialog);
        }
        result
    }

    pub fn open_edit_annotation(&mut self, doc: &mut EditorDoc, segment_id: &str) -> EditResult {
        self.edit_annotation = None;
        if doc.environment().policy.read_only {
            return Err(EditError::ReadOnly {
                action: localized("Edit", "Изменение"),
            });
        }
        let segment = doc
            .project()
            .segment(segment_id)
            .ok_or_else(|| EditError::SegmentNotFound(segment_id.to_string()))?;
        self.edit_annotation = Some(EditAnnotationDialog::for_project_segment(
            doc.project(),
            segment,
        ));
        Ok(doc.select_segment_without_seek(segment_id))
    }

    /// Clean row drafts follow the saved label; pending user text remains untouched.
    pub fn submit_edit_annotation(&mut self, doc: &mut EditorDoc) -> EditResult {
        if doc.environment().policy.read_only {
            return Err(EditError::ReadOnly {
                action: localized("Annotation edit", "Изменение аннотации"),
            });
        }
        let mut dialog = self.edit_annotation.take().ok_or_else(|| {
            EditError::Unavailable(
                localized(
                    "No annotation is open for editing.",
                    "Нет открытой аннотации.",
                )
                .into(),
            )
        })?;
        let payload = match dialog.save_payload() {
            Ok(payload) => payload,
            Err(error) => {
                dialog.error = Some(error);
                self.edit_annotation = Some(dialog);
                return Err(EditError::Unavailable(error.message().into()));
            }
        };
        let draft_will_change = doc
            .draft(&payload.segment_id)
            .zip(doc.project().segment(&payload.segment_id))
            .is_some_and(|(draft, segment)| {
                draft.text() == segment.text && draft.text() != payload.text
            });
        if let Err(error) = doc.edit_workflow(
            localized("Annotation edit", "Изменение аннотации"),
            |workflow| {
                workflow.update_annotation_metadata(
                    &payload.segment_id,
                    payload.text,
                    payload.intensity,
                    payload.content,
                )
            },
        ) {
            self.edit_annotation = Some(dialog);
            return Err(error);
        }
        let mut outcome = Outcome::status(localized("Annotation saved", "Аннотация сохранена"));
        if draft_will_change {
            outcome = outcome.with(Effect::DraftChanged {
                segment_id: payload.segment_id,
            });
        }
        Ok(outcome)
    }
}
