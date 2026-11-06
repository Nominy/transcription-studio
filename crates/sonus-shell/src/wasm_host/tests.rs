use super::*;
use core::prelude::v1::test;
fn bridge(capacity: usize) -> (Bridge, async_channel::Receiver<Message>) {
    let (sender, receiver) = async_channel::bounded(capacity);
    (
        Bridge {
            owner: "test.extension".into(),
            grants: Mutex::new(BTreeSet::from([Permission::DocumentRead])),
            scope: Mutex::new(None),
            sender,
            root: PathBuf::new(),
            removed: AtomicBool::new(false),
            overflow: AtomicBool::new(false),
        },
        receiver,
    )
}
fn progress(task: u64) -> Output {
    Output::Progress(w::Progress {
        task_id: task,
        label: "work".into(),
        completed: 0,
        total: None,
    })
}
#[test]
fn removed_generation_cannot_publish_or_reopen_access() {
    let (host, receiver) = bridge(8);
    host.emit("test.extension", Output::Removed);
    host.emit("test.extension", progress(1));
    host.emit("test.extension", Output::Activated);
    assert!(matches!(
        receiver.try_recv(),
        Ok(Message::Output(Output::Removed))
    ));
    assert!(receiver.try_recv().is_err());
    let cancellation = Arc::new(AtomicBool::new(false));
    let cx = CallContext {
        extension_id: "test.extension",
        deadline: Instant::now() + Duration::from_secs(1),
        cancellation: &cancellation,
    };
    assert!(host.permitted(&cx, Permission::DocumentRead).is_err());
}
#[test]
fn output_overflow_poison_is_terminal() {
    let (host, receiver) = bridge(1);
    host.emit("test.extension", progress(1));
    host.emit("test.extension", progress(2));
    assert!(host.overflow.load(Ordering::Acquire));
    assert!(host.removed.load(Ordering::Acquire));
    receiver.try_recv().unwrap();
    host.emit("test.extension", Output::Activated);
    assert!(receiver.try_recv().is_err());
}
#[test]
fn exact_cancellation_token_prevents_admitting_owner_work() {
    let (host, receiver) = bridge(1);
    let cancellation = Arc::new(AtomicBool::new(true));
    let cx = CallContext {
        extension_id: "test.extension",
        deadline: Instant::now() + Duration::from_secs(1),
        cancellation: &cancellation,
    };
    assert!(host.snapshot(&cx, "document", 1024).is_err());
    assert!(receiver.try_recv().is_err());
}
fn manifest(id: &str, mode: Option<(&str, bool)>) -> ExtensionManifest {
    let modes=mode.map(|(mode,requires)|serde_json::json!({"id":mode,"name":mode,"requires":if requires {serde_json::json!({"test.extension":"*"})}else{serde_json::json!({})}})).into_iter().collect::<Vec<_>>();
    serde_json::from_value(serde_json::json!({"id":id,"name":id,"version":"1.0.0","api":"*","runtime":"wasm","description":"test","license":"MIT","dependencies":{},"permissions":[],"modes":modes})).unwrap()
}
fn registered_modes() -> ContributionRegistry {
    let mut registry = ContributionRegistry::default();
    for manifest in [
        manifest("test.extension", None),
        manifest("test.alpha", Some(("test.alpha.edit", true))),
        manifest("test.beta", Some(("test.beta.edit", false))),
    ] {
        let owner = manifest.id.clone();
        let modes = manifest.modes.clone();
        registry
            .register_manifest(manifest, Vec::new(), true)
            .unwrap();
        for mode in modes {
            registry
                .register_mode(
                    &owner,
                    crate::ModeContribution {
                        id: mode.id,
                        label: mode.name,
                        description: String::new(),
                        home: std::rc::Rc::new(|_, _, _| {
                            panic!("authorization must not render a mode")
                        }),
                    },
                )
                .unwrap();
        }
    }
    registry
}

#[test]
fn mode_dependency_cannot_read_an_unrelated_document_mode() {
    let registry = registered_modes();
    assert!(mode_allowed_in(
        "test.extension",
        "test.alpha.edit",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
    assert!(!mode_allowed_in(
        "test.extension",
        "test.beta.edit",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
    assert!(!mode_allowed_in(
        "test.extension",
        "sonus.local",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
    assert!(mode_allowed_in(
        "test.extension",
        "test.beta.edit",
        &DocumentAccess::Global,
        &registry
    ));
    let only_alpha = DocumentAccess::Modes(BTreeSet::from(["test.alpha.edit".into()]));
    assert!(!mode_allowed_in(
        "test.extension",
        "test.beta.edit",
        &only_alpha,
        &registry
    ));
}

#[test]
fn an_unregistered_manifest_claim_cannot_impersonate_a_mode_owner() {
    let mut registry = registered_modes();
    let mut spy = manifest("aaa.spy", Some(("test.alpha.edit", false)));
    spy.permissions.push(Permission::DocumentRead);
    registry
        .register_manifest(spy, vec![Permission::DocumentRead], true)
        .unwrap();
    // The attacker deliberately emits no mode registration; manifest ordering is irrelevant.
    assert!(!mode_allowed_in(
        "aaa.spy",
        "test.alpha.edit",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
    assert!(mode_allowed_in(
        "test.extension",
        "test.alpha.edit",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
    registry.unload("test.alpha").unwrap();
    assert!(!mode_allowed_in(
        "aaa.spy",
        "test.alpha.edit",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
    assert!(!mode_allowed_in(
        "test.extension",
        "test.alpha.edit",
        &DocumentAccess::DeclaredModes,
        &registry
    ));
}
#[test]
fn utf8_transaction_is_one_undo_unit_and_stale_edits_are_rejected() {
    use sonus_extension::document::DocumentEnvironment;
    let project = studio_core::Project {
        id: "doc".into(),
        name: "test".into(),
        speakers: vec![studio_core::Speaker {
            id: "speaker".into(),
            label: "Speaker".into(),
        }],
        lanes: vec![studio_core::AudioLane {
            id: "lane".into(),
            speaker_id: "speaker".into(),
            label: "Lane".into(),
            source_path: String::new(),
        }],
        segments: vec![studio_core::Segment {
            id: "one".into(),
            source_annotation_id: None,
            speaker_id: "speaker".into(),
            lane_id: "lane".into(),
            start_seconds: 0.,
            end_seconds: 1.,
            text: "aé🙂z".into(),
            metadata: BTreeMap::new(),
        }],
        metadata: BTreeMap::new(),
    };
    let mut doc = studio_editor::edit::EditorDoc::from_project(
        project.clone(),
        DocumentEnvironment::local("doc"),
    );
    let transaction = w::Transaction {
        document_id: "doc".into(),
        expected_revision: 0,
        label: "Guest edit".into(),
        edits: vec![
            w::Edit::ReplaceText(w::TextEdit {
                segment_id: "one".into(),
                start_byte: 1,
                end_byte: 3,
                text: "e".into(),
            }),
            w::Edit::ReplaceText(w::TextEdit {
                segment_id: "one".into(),
                start_byte: 2,
                end_byte: 6,
                text: "!".into(),
            }),
        ],
    };
    let _ = doc
        .apply_transaction(document::transaction(&transaction).unwrap())
        .unwrap();
    assert_eq!(doc.project().segments[0].text, "ae!z");
    assert!(
        doc.apply_transaction(document::transaction(&transaction).unwrap())
            .is_err()
    );
    let _ = doc.undo().unwrap();
    assert_eq!(doc.project(), &project);
}
#[test]
fn invalid_utf8_edit_preserves_every_segment_and_history() {
    use sonus_extension::document::DocumentEnvironment;
    let project = studio_core::Project {
        id: "doc".into(),
        name: "test".into(),
        speakers: vec![],
        lanes: vec![studio_core::AudioLane {
            id: "lane".into(),
            speaker_id: String::new(),
            label: "Lane".into(),
            source_path: String::new(),
        }],
        segments: vec![studio_core::Segment {
            id: "one".into(),
            source_annotation_id: None,
            speaker_id: String::new(),
            lane_id: "lane".into(),
            start_seconds: 0.,
            end_seconds: 1.,
            text: "é".into(),
            metadata: BTreeMap::new(),
        }],
        metadata: BTreeMap::new(),
    };
    let mut doc = studio_editor::edit::EditorDoc::from_project(
        project.clone(),
        DocumentEnvironment::local("doc"),
    );
    let transaction = w::Transaction {
        document_id: "doc".into(),
        expected_revision: 0,
        label: "Invalid".into(),
        edits: vec![w::Edit::ReplaceText(w::TextEdit {
            segment_id: "one".into(),
            start_byte: 1,
            end_byte: 2,
            text: "x".into(),
        })],
    };
    assert!(
        doc.apply_transaction(document::transaction(&transaction).unwrap())
            .is_err()
    );
    assert_eq!(doc.project(), &project);
    assert!(!doc.can_undo());
}

#[test]
fn reopened_document_does_not_accept_prior_incarnation_handle() {
    let (host, receiver) = bridge(1);
    *host.scope.lock().unwrap() = Some(Scope {
        document: "document:2".into(),
        mode: "test.alpha.edit".into(),
        entity: 2,
    });
    let cancellation = Arc::new(AtomicBool::new(false));
    let cx = CallContext {
        extension_id: "test.extension",
        deadline: Instant::now() + Duration::from_secs(1),
        cancellation: &cancellation,
    };
    assert!(host.snapshot(&cx, "document:1", 1024).is_err());
    assert!(receiver.try_recv().is_err());
}
