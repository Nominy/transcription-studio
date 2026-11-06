use super::*;
use crate::editor::model::EditorModel;
use exaudio_extension::document as d;
use studio_editor::edit::TextSelection;

pub(super) enum Request {
    Snapshot(usize),
    Transaction(w::Transaction),
    Audio(w::AudioRequest, usize),
    SelectionGet,
    SelectionSet(w::Selection),
    TransportGet,
    TransportSet(w::Transport),
}
pub(super) enum Reply {
    Document(w::Document),
    Revision(u64),
    Audio(w::AudioData),
    Selection(w::Selection),
    Transport(w::Transport),
    Unit,
}
pub(super) fn dispatch(
    shell: &ShellHandle,
    envelope: &Envelope,
    cx: &mut App,
) -> Result<Reply, String> {
    let model = shell.editor_model(cx).ok_or("document is closed")?;
    let current = model.read(cx).doc();
    if model.entity_id().as_u64() != envelope.scope.entity
        || document_handle(&model) != envelope.scope.document
        || current.environment().origin.mode != envelope.scope.mode
    {
        return Err("document/mode association changed".into());
    }
    if envelope.cancelled.load(Ordering::Acquire) || Instant::now() >= envelope.deadline {
        return Err("request expired".into());
    }
    model.update(cx, |model, cx| match &envelope.request {
        Request::Snapshot(max) => {
            snapshot(model, *max, &envelope.scope.document).map(Reply::Document)
        }
        Request::Transaction(value) => {
            let transaction = transaction_for_project(value, model.doc().project())?;
            // EditorDoc validates every edit and UTF-8 boundary against a staged project,
            // compares the revision and commits exactly one command history entry.
            let result = model.update_doc(cx, |doc| {
                doc.apply_transaction_guarded(transaction, || {
                    if envelope.cancelled.load(Ordering::Acquire)
                        || Instant::now() >= envelope.deadline
                    {
                        Err(studio_editor::edit::EditError::Unavailable(
                            "extension transaction cancelled before commit".into(),
                        ))
                    } else {
                        Ok(())
                    }
                })
            });
            let outcome = result.map_err(|e| e.to_string())?;
            model.apply_outcome(outcome, cx);
            Ok(Reply::Revision(
                model.doc().session().current_edit_revision(),
            ))
        }
        Request::Audio(value, max) => {
            if value.frames as usize > *max {
                return Err("audio sample limit exceeded".into());
            }
            if !model
                .doc()
                .project()
                .lanes
                .iter()
                .any(|lane| lane.id == value.lane_id)
            {
                return Err("unknown document lane".into());
            }
            let audio = model
                .audio()
                .lane_audio(&value.lane_id)
                .ok_or("lane audio has not been decoded")?;
            let samples = audio
                .channel_data(value.channel as usize)
                .ok_or("unknown channel")?;
            let start = usize::try_from(value.start_frame).map_err(|_| "audio offset overflow")?;
            let end = start
                .checked_add(value.frames as usize)
                .ok_or("audio range overflow")?;
            let slice = samples
                .get(start..end)
                .ok_or("audio range outside source")?;
            Ok(Reply::Audio(w::AudioData {
                sample_rate: audio.sample_rate(),
                samples: slice.to_vec(),
            }))
        }
        Request::SelectionGet => {
            let doc = model.doc();
            let mut ranges = vec![];
            if let Some(id) = &doc.session().selected_segment_id
                && let Some(draft) = doc.draft(id)
            {
                let range = draft.selection().range();
                let offset = |chars| {
                    draft
                        .text()
                        .char_indices()
                        .nth(chars)
                        .map(|(i, _)| i)
                        .unwrap_or(draft.text().len())
                };
                ranges.push(w::TextRange {
                    segment_id: id.clone(),
                    start_byte: u32::try_from(offset(range.start))
                        .map_err(|_| "selection too large")?,
                    end_byte: u32::try_from(offset(range.end))
                        .map_err(|_| "selection too large")?,
                });
            }
            Ok(Reply::Selection(w::Selection {
                document_id: envelope.scope.document.clone(),
                revision: doc.session().current_edit_revision(),
                ranges,
            }))
        }
        Request::SelectionSet(value) => {
            let doc = model.doc();
            if value.revision != doc.session().current_edit_revision() {
                return Err("stale selection revision".into());
            }
            if value.ranges.len() > 1 {
                return Err("the editor supports one focused text selection".into());
            }
            let prepared = value
                .ranges
                .iter()
                .map(|range| {
                    let draft = doc
                        .draft(&range.segment_id)
                        .ok_or("unknown selection segment")?;
                    validate_range(draft.text(), range.start_byte, range.end_byte)?;
                    Ok((
                        range.segment_id.clone(),
                        TextSelection {
                            anchor: draft.text()[..range.start_byte as usize].chars().count(),
                            head: draft.text()[..range.end_byte as usize].chars().count(),
                        },
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?;
            if let Some((id, selection)) = prepared.into_iter().next() {
                model.run(cx, |doc| doc.select_segment_without_seek(&id));
                model.update_doc(cx, |doc| doc.set_selection(&id, selection));
                cx.emit(crate::editor::model::EditorEvent::SelectText {
                    segment_id: id,
                    selection,
                });
            } else {
                model.update_doc(cx, |doc| doc.session_mut().selected_segment_id = None);
                cx.emit(crate::editor::model::EditorEvent::BlurText);
            }
            Ok(Reply::Unit)
        }
        Request::TransportGet => {
            let doc = model.doc();
            let session = doc.session();
            Ok(Reply::Transport(w::Transport {
                document_id: envelope.scope.document.clone(),
                position_ms: (session.playhead_seconds * 1000.).max(0.) as u64,
                playing: session.is_playing,
                rate: session.speed,
            }))
        }
        Request::TransportSet(value) => {
            if !value.rate.is_finite() || !(0.1..=8.).contains(&value.rate) {
                return Err("invalid playback rate".into());
            }
            model.run(cx, |doc| doc.set_speed(value.rate));
            model.run(cx, |doc| doc.seek(value.position_ms as f64 / 1000.));
            if model.doc().session().is_playing != value.playing {
                model.run(cx, |doc| doc.toggle_playback());
            }
            Ok(Reply::Unit)
        }
    })
}
fn scalar(value: &serde_json::Value) -> w::Scalar {
    match value {
        serde_json::Value::Bool(v) => w::Scalar::Boolean(*v),
        serde_json::Value::Number(v) => w::Scalar::Number(v.as_f64().unwrap_or(0.)),
        serde_json::Value::String(v) => w::Scalar::Text(v.clone()),
        v => w::Scalar::Text(v.to_string()),
    }
}
fn json(value: &w::Scalar) -> serde_json::Value {
    match value {
        w::Scalar::Text(v) => v.clone().into(),
        w::Scalar::Number(v) => serde_json::json!(v),
        w::Scalar::Boolean(v) => (*v).into(),
    }
}
fn segment(value: &studio_core::Segment) -> w::Segment {
    w::Segment {
        id: value.id.clone(),
        lane: value.lane_id.clone(),
        speaker_id: (!value.speaker_id.is_empty()).then(|| value.speaker_id.clone()),
        start_ms: (value.start_seconds * 1000.).max(0.) as u64,
        end_ms: (value.end_seconds * 1000.).max(0.) as u64,
        text: value.text.clone(),
        metadata: value
            .metadata
            .iter()
            .map(|(key, value)| w::Property {
                key: key.clone(),
                value: scalar(value),
            })
            .collect(),
    }
}
pub(crate) fn core_segment(value: &w::Segment) -> studio_core::Segment {
    studio_core::Segment {
        id: value.id.clone(),
        source_annotation_id: None,
        speaker_id: value.speaker_id.clone().unwrap_or_default(),
        lane_id: value.lane.clone(),
        start_seconds: value.start_ms as f64 / 1000.,
        end_seconds: value.end_ms as f64 / 1000.,
        text: value.text.clone(),
        metadata: value
            .metadata
            .iter()
            .map(|p| (p.key.clone(), json(&p.value)))
            .collect(),
    }
}
pub(crate) fn project(value: &w::Document) -> Result<studio_core::Project> {
    let project = studio_core::Project {
        id: value.id.clone(),
        name: value.name.clone(),
        speakers: value
            .speakers
            .iter()
            .map(|s| studio_core::Speaker {
                id: s.id.clone(),
                label: s.label.clone(),
            })
            .collect(),
        lanes: value
            .lanes
            .iter()
            .map(|l| studio_core::AudioLane {
                id: l.id.clone(),
                speaker_id: l.speaker_id.clone().unwrap_or_default(),
                label: l.label.clone(),
                source_path: String::new(),
            })
            .collect(),
        segments: value.segments.iter().map(core_segment).collect(),
        metadata: value
            .metadata
            .iter()
            .map(|p| {
                (
                    p.key.clone(),
                    match &p.value {
                        w::Scalar::Text(s) => s.clone(),
                        v => json(v).to_string(),
                    },
                )
            })
            .collect(),
    };
    if project.id.is_empty() {
        bail!("document identity is empty");
    }
    let mut speakers = BTreeSet::new();
    let mut lanes = BTreeSet::new();
    for speaker in &project.speakers {
        if speaker.id.is_empty() || !speakers.insert(speaker.id.as_str()) {
            bail!("invalid or duplicate speaker identity");
        }
    }
    for lane in &project.lanes {
        if lane.id.is_empty()
            || !lanes.insert(lane.id.as_str())
            || (!lane.speaker_id.is_empty() && !speakers.contains(lane.speaker_id.as_str()))
        {
            bail!("invalid lane identity or speaker");
        }
    }
    let mut ids = BTreeSet::new();
    for segment in &project.segments {
        if segment.id.is_empty()
            || !ids.insert(&segment.id)
            || !lanes.contains(segment.lane_id.as_str())
            || (!segment.speaker_id.is_empty() && !speakers.contains(segment.speaker_id.as_str()))
            || segment.end_seconds < segment.start_seconds
        {
            bail!("invalid mode/import document");
        }
    }
    Ok(project)
}
fn snapshot(model: &EditorModel, max: usize, id: &str) -> Result<w::Document, String> {
    let doc = model.doc();
    let project = doc.project();
    snapshot_budget(model, id, max).map_err(|error| error.to_string())?;
    let sources = project
        .lanes
        .iter()
        .filter_map(|lane| {
            model
                .audio()
                .lane_audio(&lane.id)
                .map(|audio| w::AudioSource {
                    id: lane.id.clone(),
                    label: lane.label.clone(),
                    sample_rate: audio.sample_rate(),
                    frames: audio.frame_count() as u64,
                    channels: audio.number_of_channels() as u32,
                })
        })
        .collect();
    Ok(w::Document {
        id: id.to_owned(),
        revision: doc.session().current_edit_revision(),
        name: project.name.clone(),
        mode: doc.environment().origin.mode.clone(),
        read_only: doc.is_read_only(),
        speakers: project
            .speakers
            .iter()
            .map(|s| w::Speaker {
                id: s.id.clone(),
                label: s.label.clone(),
            })
            .collect(),
        lanes: project
            .lanes
            .iter()
            .map(|l| w::Lane {
                id: l.id.clone(),
                speaker_id: (!l.speaker_id.is_empty()).then(|| l.speaker_id.clone()),
                label: l.label.clone(),
                source_id: model.audio().lane_audio(&l.id).map(|_| l.id.clone()),
            })
            .collect(),
        sources,
        metadata: project
            .metadata
            .iter()
            .map(|(key, value)| w::Property {
                key: key.clone(),
                value: w::Scalar::Text(value.clone()),
            })
            .collect(),
        segments: project.segments.iter().map(segment).collect(),
    })
}
fn snapshot_budget(model: &EditorModel, id: &str, max: usize) -> Result<()> {
    let project = model.doc().project();
    let mut budget = ByteQuota {
        bytes: 0,
        limit: max,
    };
    budget.add(64 + id.len() + project.name.len() + model.doc().environment().origin.mode.len())?;
    for (key, value) in &project.metadata {
        budget.add(32 + key.len() + value.len())?;
    }
    for speaker in &project.speakers {
        budget.add(32 + speaker.id.len() + speaker.label.len())?;
    }
    for lane in &project.lanes {
        budget.add(48 + lane.id.len() + lane.label.len() + lane.speaker_id.len())?;
        if model.audio().lane_audio(&lane.id).is_some() {
            budget.add(lane.id.len() + 48 + lane.id.len() + lane.label.len())?;
        }
    }
    for segment in &project.segments {
        budget.add(
            48 + segment.id.len()
                + segment.lane_id.len()
                + segment.speaker_id.len()
                + segment.text.len(),
        )?;
        for (key, value) in &segment.metadata {
            budget.add(32 + key.len())?;
            match value {
                serde_json::Value::String(text) => budget.add(text.len())?,
                serde_json::Value::Number(_) | serde_json::Value::Bool(_) => budget.add(8)?,
                value => budget.charge(value)?,
            }
        }
    }
    Ok(())
}
pub(crate) fn transaction(value: &w::Transaction) -> Result<d::DocumentTransaction, String> {
    Ok(d::DocumentTransaction {
        label: value.label.clone(),
        expected_revision: value.expected_revision,
        edits: value
            .edits
            .iter()
            .map(|edit| match edit {
                w::Edit::ReplaceText(v) => d::DocumentEdit::ReplaceText {
                    segment_id: v.segment_id.clone(),
                    range: Some(d::TextRange {
                        start: v.start_byte as usize,
                        end: v.end_byte as usize,
                    }),
                    text: v.text.clone(),
                },
                w::Edit::InsertSegment(v) => d::DocumentEdit::InsertSegment {
                    segment: core_segment(v),
                },
                w::Edit::RemoveSegment(id) => d::DocumentEdit::RemoveSegment {
                    segment_id: id.clone(),
                },
                w::Edit::ReplaceSegment(v) => d::DocumentEdit::ReplaceSegment {
                    segment: core_segment(v),
                },
            })
            .collect(),
    })
}
fn transaction_for_project(
    value: &w::Transaction,
    project: &studio_core::Project,
) -> Result<d::DocumentTransaction, String> {
    let mut transaction = transaction(value)?;
    for edit in &mut transaction.edits {
        if let d::DocumentEdit::ReplaceSegment { segment } = edit
            && let Some(previous) = project.segment(&segment.id)
        {
            // Native-only provenance is not a writable WIT field. Preserve it when a
            // guest changes the typed segment. Unchanged structured metadata retains
            // its native representation after the scalar JSON-text round trip.
            segment.source_annotation_id = previous.source_annotation_id.clone();
            for (key, value) in &mut segment.metadata {
                if let Some(old) = previous.metadata.get(key)
                    && matches!(
                        old,
                        serde_json::Value::Array(_)
                            | serde_json::Value::Object(_)
                            | serde_json::Value::Null
                    )
                    && value
                        .as_str()
                        .is_some_and(|text| json_text_matches(old, text))
                {
                    *value = old.clone();
                }
            }
        }
    }
    Ok(transaction)
}
fn json_text_matches(value: &serde_json::Value, text: &str) -> bool {
    struct Compare<'a> {
        bytes: &'a [u8],
        offset: usize,
    }
    impl std::io::Write for Compare<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let end = self.offset.saturating_add(bytes.len());
            if self.bytes.get(self.offset..end) != Some(bytes) {
                return Err(std::io::Error::other("metadata differs"));
            }
            self.offset = end;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Compare {
        bytes: text.as_bytes(),
        offset: 0,
    };
    serde_json::to_writer(&mut writer, value).is_ok() && writer.offset == writer.bytes.len()
}
fn validate_range(text: &str, start: u32, end: u32) -> Result<(), String> {
    if start > end
        || end as usize > text.len()
        || !text.is_char_boundary(start as usize)
        || !text.is_char_boundary(end as usize)
    {
        Err("invalid UTF-8 byte range".into())
    } else {
        Ok(())
    }
}
pub(crate) fn current(
    shell: &ShellHandle,
    id: &str,
    revision: u64,
    cx: &App,
) -> Result<Entity<EditorModel>> {
    let model = shell
        .editor_model(cx)
        .ok_or_else(|| anyhow!("document closed"))?;
    let doc = model.read(cx).doc();
    if document_handle(&model) != id || doc.session().current_edit_revision() != revision {
        bail!("stale published document revision");
    }
    Ok(model)
}
#[derive(Debug, Default)]
pub(super) struct Publications {
    model: Option<WeakEntity<EditorModel>>,
    providers: Option<d::DocumentProviders>,
    document: String,
    handle: String,
    revision: u64,
    diagnostics: BTreeMap<String, d::DiagnosticCollection>,
    decorations: Vec<d::Decoration>,
    completions: Vec<(String, d::Completion)>,
    pub(super) sender: Option<async_channel::Sender<Message>>,
    pub(super) completion_registered: bool,
    requested: Option<(u64, String, usize, u64)>,
}
impl Publications {
    fn revision(&mut self, document: String, revision: u64) {
        if self.document != document || self.revision != revision {
            self.diagnostics.clear();
            self.decorations.clear();
            self.completions.clear();
        }
        self.document = document;
        self.revision = revision;
    }
}
#[derive(Debug)]
struct Provider(std::sync::Weak<Mutex<Publications>>);
impl d::DecorationProvider for Provider {
    fn decorate(&self, project: &studio_core::Project, revision: u64) -> Vec<d::Decoration> {
        let Some(shared) = self.0.upgrade() else {
            return vec![];
        };
        let p = shared.lock().expect("publications");
        if p.document == project.id && p.revision == revision {
            p.decorations.clone()
        } else {
            vec![]
        }
    }
}
impl d::CompletionProvider for Provider {
    fn complete(&self, request: &d::CompletionRequest<'_>) -> Vec<d::Completion> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let Some(shared) = self.0.upgrade() else {
            return vec![];
        };
        let mut p = shared.lock().expect("publications");
        if !p.completion_registered {
            return vec![];
        }
        let fresh = p
            .requested
            .as_ref()
            .is_some_and(|(revision, id, offset, _)| {
                *revision == request.revision
                    && id == request.segment_id
                    && *offset == request.selection.end
            });
        if !fresh {
            let request_id = NEXT.fetch_add(1, Ordering::Relaxed);
            if let (Some(sender), Ok(offset)) = (&p.sender, u32::try_from(request.selection.end))
                && sender
                    .try_send(Message::Completion(w::CompletionRequest {
                        request_id,
                        document_id: p.handle.clone(),
                        revision: request.revision,
                        segment_id: request.segment_id.to_owned(),
                        byte_offset: offset,
                    }))
                    .is_ok()
            {
                p.requested = Some((
                    request.revision,
                    request.segment_id.to_owned(),
                    request.selection.end,
                    request_id,
                ));
            }
            return vec![];
        }
        if p.document != request.project.id || p.revision != request.revision {
            return vec![];
        }
        p.completions
            .iter()
            .filter(|(id, c)| {
                id == request.segment_id
                    && c.range.end <= request.text.len()
                    && request.text.is_char_boundary(c.range.start)
                    && request.text.is_char_boundary(c.range.end)
            })
            .map(|(_, c)| c.clone())
            .collect()
    }
}
pub(super) fn install(
    shared: &Arc<Mutex<Publications>>,
    model: &Entity<EditorModel>,
    cx: &mut App,
) {
    clear(shared, cx);
    let provider = Arc::new(Provider(Arc::downgrade(shared)));
    let providers = d::DocumentProviders {
        completions: vec![provider.clone()],
        decorations: vec![provider],
        ..Default::default()
    };
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| {
            let dest = &mut doc.session_mut().environment.providers;
            dest.diagnostics.extend(providers.diagnostics.clone());
            dest.completions.extend(providers.completions.clone());
            dest.decorations.extend(providers.decorations.clone());
        })
    });
    let mut p = shared.lock().expect("publications");
    p.model = Some(model.downgrade());
    p.providers = Some(providers);
    p.handle = document_handle(model);
}
pub(super) fn clear(shared: &Arc<Mutex<Publications>>, cx: &mut App) {
    let (model, providers, sources) = {
        let mut p = shared.lock().expect("publications");
        let model = p.model.take();
        let providers = p.providers.take();
        let sources = p.diagnostics.keys().cloned().collect::<Vec<_>>();
        p.diagnostics.clear();
        p.decorations.clear();
        p.completions.clear();
        p.document.clear();
        p.requested = None;
        (model, providers, sources)
    };
    if let (Some(model), Some(providers)) = (model, providers) {
        let _ = model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                doc.with_session(|project, session| {
                    for source in sources {
                        session.remove_diagnostics(project, &source);
                    }
                });
                let session = doc.session_mut();
                session
                    .environment
                    .providers
                    .diagnostics
                    .retain(|a| !providers.diagnostics.iter().any(|b| Arc::ptr_eq(a, b)));
                session
                    .environment
                    .providers
                    .decorations
                    .retain(|a| !providers.decorations.iter().any(|b| Arc::ptr_eq(a, b)));
                session
                    .environment
                    .providers
                    .completions
                    .retain(|a| !providers.completions.iter().any(|b| Arc::ptr_eq(a, b)));
            })
        });
    }
}
fn shared(owner: &str, cx: &App) -> Result<Arc<Mutex<Publications>>> {
    cx.global::<WasmExtensions>()
        .entries
        .get(owner)
        .map(|e| e.publications.clone())
        .ok_or_else(|| anyhow!("unknown output owner"))
}
fn range(model: &EditorModel, value: &w::TextRange) -> Result<d::TextRange> {
    let segment = model
        .doc()
        .project()
        .segment(&value.segment_id)
        .ok_or_else(|| anyhow!("unknown output segment"))?;
    validate_range(&segment.text, value.start_byte, value.end_byte).map_err(|e| anyhow!(e))?;
    Ok(d::TextRange {
        start: value.start_byte as usize,
        end: value.end_byte as usize,
    })
}
pub(super) fn publish_diagnostics(
    owner: &str,
    shell: &ShellHandle,
    value: w::Diagnostics,
    cx: &mut App,
) -> Result<()> {
    let model = current(shell, &value.document_id, value.revision, cx)?;
    let source = format!("{owner}:{}", value.provider);
    let diagnostics = value
        .items
        .iter()
        .map(|item| {
            Ok(d::Diagnostic {
                source: source.clone(),
                rule: item.id.clone(),
                severity: match item.severity {
                    w::Severity::Hint => d::Severity::Hint,
                    w::Severity::Info => d::Severity::Info,
                    w::Severity::Warning => d::Severity::Warning,
                    w::Severity::Error => d::Severity::Error,
                },
                segment_id: item.range.segment_id.clone(),
                range: Some(range(model.read(cx), &item.range)?),
                message: item.message.clone(),
                fixes: item
                    .fix
                    .iter()
                    .map(|fix| {
                        if fix.document_id != value.document_id
                            || fix.expected_revision != value.revision
                        {
                            bail!("stale diagnostic fix");
                        }
                        let t = transaction_for_project(fix, model.read(cx).doc().project())
                            .map_err(|e| anyhow!(e))?;
                        Ok(d::QuickFix {
                            title: t.label,
                            expected_revision: t.expected_revision,
                            edits: t.edits,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let collection = d::DiagnosticCollection {
        source: source.clone(),
        revision: value.revision,
        diagnostics,
    };
    {
        let shared = shared(owner, cx)?;
        let mut p = shared.lock().expect("publications");
        p.revision(model.read(cx).doc().project().id.clone(), value.revision);
        let mut quota = ByteQuota::default();
        for (id, old) in &p.diagnostics {
            if id != &source {
                quota.charge(old)?;
            }
        }
        quota.charge(&collection)?;
        if p.diagnostics
            .values()
            .filter(|old| old.source != source)
            .map(|old| old.diagnostics.len())
            .sum::<usize>()
            .saturating_add(collection.diagnostics.len())
            > 8192
        {
            bail!("diagnostic item quota exceeded");
        }
        p.diagnostics.insert(source, collection.clone());
    }
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| {
            doc.with_session(|project, session| session.publish_diagnostics(project, collection))
        })
    });
    Ok(())
}
pub(super) fn publish_decorations(
    owner: &str,
    shell: &ShellHandle,
    value: w::Decorations,
    cx: &mut App,
) -> Result<()> {
    let model = current(shell, &value.document_id, value.revision, cx)?;
    let source = format!("{owner}:{}", value.provider);
    let decorations = value
        .items
        .iter()
        .map(|item| {
            Ok(d::Decoration {
                source: source.clone(),
                kind: format!("wasm-text:{}", item.class),
                segment_id: item.range.segment_id.clone(),
                range: Some(range(model.read(cx), &item.range)?),
                label: item.hover.clone().unwrap_or_else(|| item.class.clone()),
                data: serde_json::Value::Null,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    {
        let shared = shared(owner, cx)?;
        let mut p = shared.lock().expect("publications");
        p.revision(model.read(cx).doc().project().id.clone(), value.revision);
        let mut quota = ByteQuota::default();
        for old in p.decorations.iter().filter(|d| d.source != source) {
            quota.charge(old)?;
        }
        quota.charge(&decorations)?;
        if p.decorations
            .iter()
            .filter(|d| d.source != source)
            .count()
            .saturating_add(decorations.len())
            > 8192
        {
            bail!("decoration item quota exceeded");
        }
        p.decorations.retain(|d| d.source != source);
        p.decorations.extend(decorations);
    }
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| {
            doc.session_mut();
        })
    });
    Ok(())
}
pub(super) fn publish_completions(
    owner: &str,
    shell: &ShellHandle,
    value: w::Completions,
    cx: &mut App,
) -> Result<()> {
    let model = current(shell, &value.document_id, value.revision, cx)?;
    let completions = value
        .items
        .iter()
        .map(|item| {
            let text_range = w::TextRange {
                segment_id: item.edit.segment_id.clone(),
                start_byte: item.edit.start_byte,
                end_byte: item.edit.end_byte,
            };
            Ok((
                item.edit.segment_id.clone(),
                d::Completion {
                    source: owner.into(),
                    label: item.label.clone(),
                    insert_text: item.edit.text.clone(),
                    range: range(model.read(cx), &text_range)?,
                    documentation: Some(item.detail.clone()),
                    caret_offset: Some(item.edit.start_byte as usize + item.edit.text.len()),
                },
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    {
        let shared = shared(owner, cx)?;
        let mut p = shared.lock().expect("publications");
        if !p
            .requested
            .as_ref()
            .is_some_and(|(_, _, _, request)| *request == value.request_id)
        {
            bail!("stale completion request");
        }
        ByteQuota::default().charge(&completions)?;
        p.revision(model.read(cx).doc().project().id.clone(), value.revision);
        p.completions = completions;
    }
    model.update(cx, |model, cx| {
        model.update_doc(cx, |doc| {
            doc.session_mut();
        })
    });
    Ok(())
}

struct ByteQuota {
    bytes: usize,
    limit: usize,
}
impl Default for ByteQuota {
    fn default() -> Self {
        Self {
            bytes: 0,
            limit: 2 * 1024 * 1024,
        }
    }
}
impl std::io::Write for ByteQuota {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.add(bytes.len())?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl ByteQuota {
    fn add(&mut self, bytes: usize) -> std::io::Result<()> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > self.limit {
            return Err(std::io::Error::other("extension byte quota exceeded"));
        }
        Ok(())
    }
    fn charge(&mut self, value: &impl serde::Serialize) -> Result<()> {
        serde_json::to_writer(self, value)?;
        Ok(())
    }
}
