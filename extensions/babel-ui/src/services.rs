//! Babel transport is captured per document; credentials never select a document's route.
use exaudio_babel_domain::editor_model::BabelDocumentState;
use exaudio_babel_services::{babel::BabelClient, review::ReviewBackend};
use exaudio_shell::editor::model::EditorModel;
use gpui_kit::*;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    rc::Rc,
};

pub struct BabelServices {
    pub babel: BabelClient,
    pub review: ReviewBackend,
}
impl Global for BabelServices {}
impl BabelServices {
    pub fn activate(client: BabelClient, cx: &mut App) {
        cx.set_global(Self {
            babel: client,
            review: ReviewBackend::from_env(),
        });
    }
}

/// Resolve Babel's exported/cache filename conventions without changing remote source references.
pub(crate) fn local_audio_sources(
    project: &studio_core::Project,
    base: Option<&Path>,
) -> BTreeMap<String, Vec<PathBuf>> {
    let Some(base) = base else {
        return BTreeMap::new();
    };
    project
        .lanes
        .iter()
        .map(|lane| {
            let mut candidates = vec![base.join(&lane.source_path)];
            if let Some(number) = lane.speaker_id.strip_prefix("speaker-") {
                let name = format!("speaker_{number}.wav");
                for path in [base.join(&name), base.join("audio").join(&name)] {
                    if !candidates.contains(&path) {
                        candidates.push(path);
                    }
                }
            }
            (lane.id.clone(), candidates)
        })
        .collect()
}

struct DocumentServices {
    model: WeakEntity<EditorModel>,
    client: Option<BabelClient>,
    dialogs: Rc<RefCell<exaudio_babel_domain::edit::BabelAnnotationDialogs>>,
    state: Rc<RefCell<BabelDocumentState>>,
}
#[derive(Default)]
struct Documents(HashMap<EntityId, DocumentServices>);
impl Global for Documents {}

pub fn set_document_client(model: &Entity<EditorModel>, client: BabelClient, cx: &mut App) {
    attach_document(model, Some(client), cx);
}

pub fn set_file_document(model: &Entity<EditorModel>, cx: &mut App) {
    attach_document(model, None, cx);
}

fn attach_document(model: &Entity<EditorModel>, client: Option<BabelClient>, cx: &mut App) {
    let state = Rc::new(RefCell::new(BabelDocumentState::for_project(
        model.read(cx).doc().project(),
    )));
    if !cx.has_global::<Documents>() {
        cx.set_global(Documents::default());
    }
    let documents = cx.global_mut::<Documents>();
    documents.0.insert(
        model.entity_id(),
        DocumentServices {
            model: model.downgrade(),
            client,
            state,
            dialogs: Rc::new(RefCell::new(
                exaudio_babel_domain::edit::BabelAnnotationDialogs::default(),
            )),
        },
    );
    documents
        .0
        .retain(|_, value| value.model.upgrade().is_some());
}

pub fn document_client(model: &Entity<EditorModel>, cx: &App) -> BabelClient {
    cx.global::<Documents>()
        .0
        .get(&model.entity_id())
        .and_then(|services| services.client.clone())
        .expect("Remote Babel operations require the transport captured by a claimed task")
}

pub fn document_state(model: &Entity<EditorModel>, cx: &App) -> Rc<RefCell<BabelDocumentState>> {
    cx.global::<Documents>()
        .0
        .get(&model.entity_id())
        .expect("Babel document state must be attached before mounting its panels")
        .state
        .clone()
}
pub fn document_dialogs(
    model: &Entity<EditorModel>,
    cx: &App,
) -> Rc<RefCell<exaudio_babel_domain::edit::BabelAnnotationDialogs>> {
    cx.global::<Documents>()
        .0
        .get(&model.entity_id())
        .expect("Babel document state must be attached before mounting its dialogs")
        .dialogs
        .clone()
}

pub fn shutdown(cx: &mut App) {
    if cx.has_global::<Documents>() {
        cx.global_mut::<Documents>().0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn remote_lane_reference_loads_babel_export_audio_through_explicit_candidates() {
        let mut project = studio_editor::load_sample_project().unwrap();
        let lane = &mut project.lanes[0];
        lane.speaker_id = "speaker-1".into();
        lane.source_path = "s3://synthetic-bucket/recording.wav".into();
        let id = lane.id.clone();
        let base =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/studio-editor/assets/demo");
        let mut candidates = local_audio_sources(&project, Some(&base));
        let loaded =
            studio_audio::load_lane_audio_candidates(&id, candidates.remove(&id).unwrap()).unwrap();
        assert_eq!(loaded.path, base.join("speaker_1.wav"));
    }
}
