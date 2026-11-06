use super::*;
use core::prelude::v1::test;
fn bridge(capacity: usize) -> (Bridge, async_channel::Receiver<Message>) {
    let (sender, receiver) = async_channel::bounded(capacity);
    (
        Bridge {
            owner: "test.extension".into(),
            grants: Mutex::new(BTreeSet::from([Permission::DocumentRead])),
            providers: Mutex::new(BTreeSet::from(["test.account".to_string()])),
            scope: Mutex::new(None),
            sender,
            root: PathBuf::new(),
            status: Mutex::new(ProviderStatusSlots::default()),
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
fn guest_instance_of(provider: &str) -> w::ProviderInstance {
    w::ProviderInstance {
        provider: provider.into(),
        id: "alice".into(),
        label: "Alice".into(),
    }
}
#[test]
fn status_answers_reach_the_waiting_requester_and_undeclared_providers_are_dropped() {
    let (host, receiver) = bridge(8);
    let instance = guest_instance_of("test.account");
    let waiter = host.await_status(&instance);
    let answer = w::ProviderStatus {
        instance: instance.clone(),
        state: w::ProviderState::Ready,
        detail: "signed in".into(),
    };
    host.emit("test.extension", Output::ProviderStatus(answer.clone()));
    assert_eq!(waiter.try_recv().unwrap().detail, "signed in");
    assert_eq!(host.cached_status(&instance).unwrap().detail, "signed in");
    assert!(matches!(
        receiver.try_recv(),
        Ok(Message::Output(Output::ProviderStatus(_)))
    ));
    // A status for a provider this extension does not declare is neither cached nor queued.
    let foreign = guest_instance_of("other.account");
    host.emit(
        "test.extension",
        Output::ProviderStatus(w::ProviderStatus {
            instance: foreign.clone(),
            ..answer
        }),
    );
    assert!(host.cached_status(&foreign).is_none());
    assert!(receiver.try_recv().is_err());
    host.forget_status(&instance);
    assert!(host.cached_status(&instance).is_none());
}
#[test]
fn instance_namespaces_require_grant_and_declaration() {
    let (host, _receiver) = bridge(8);
    let cancellation = Arc::new(AtomicBool::new(false));
    let cx = CallContext {
        extension_id: "test.extension",
        deadline: Instant::now() + Duration::from_secs(1),
        cancellation: &cancellation,
    };
    let declared = guest_instance_of("test.account");
    assert!(
        host.instance_root(&cx, Permission::Secrets, &declared)
            .unwrap_err()
            .contains("permission denied")
    );
    host.grants.lock().unwrap().insert(Permission::Secrets);
    let root = host
        .instance_root(&cx, Permission::Secrets, &declared)
        .unwrap();
    assert!(
        root.ends_with(
            PathBuf::from("providers")
                .join("test.account")
                .join("alice")
                .join("wasm")
                .join("test.extension")
        )
    );
    assert!(
        host.instance_root(
            &cx,
            Permission::Secrets,
            &guest_instance_of("other.account")
        )
        .unwrap_err()
        .contains("does not declare")
    );
    let mut traversal = declared;
    traversal.id = "../alice".into();
    assert!(
        host.instance_root(&cx, Permission::Secrets, &traversal)
            .is_err()
    );
}
#[test]
fn silent_or_finished_guest_status_checks_fail_instead_of_hanging() {
    let (sender, receiver) = mpsc::sync_channel::<w::ProviderStatus>(1);
    // Guest keeps running without answering: bounded by the deadline.
    let started = Instant::now();
    let timed_out = wait_for_status(&receiver, started + Duration::from_millis(30), || {
        GuestCall::Running
    })
    .unwrap_err();
    assert!(timed_out.contains("in time"), "{timed_out}");
    assert!(started.elapsed() < Duration::from_secs(2));
    // Guest returned without publishing: fails at once, not at the deadline.
    let started = Instant::now();
    assert!(
        wait_for_status(&receiver, started + Duration::from_secs(30), || {
            GuestCall::Finished
        })
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        wait_for_status(&receiver, Instant::now() + Duration::from_secs(30), || {
            GuestCall::Failed("trap".into())
        })
        .unwrap_err()
        .contains("trap")
    );
    // An answer that arrives while the guest call is still running wins immediately.
    sender
        .try_send(w::ProviderStatus {
            instance: guest_instance_of("test.account"),
            state: w::ProviderState::NeedsSignIn,
            detail: String::new(),
        })
        .unwrap();
    let answered = wait_for_status(&receiver, Instant::now() + Duration::from_secs(30), || {
        GuestCall::Running
    })
    .unwrap();
    assert_eq!(answered.state, w::ProviderState::NeedsSignIn);
    drop(sender);
    assert!(
        wait_for_status(&receiver, Instant::now() + Duration::from_secs(30), || {
            GuestCall::Running
        })
        .unwrap_err()
        .contains("unloaded")
    );
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
                        launch: std::rc::Rc::new(|_, _, _, _| {
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
        "exaudio.local",
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
    use exaudio_extension::document::DocumentEnvironment;
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
    use exaudio_extension::document::DocumentEnvironment;
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
