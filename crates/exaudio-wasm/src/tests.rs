use super::*;
use crate::{bindings::exaudio::extension::host::Host, types::*};
use std::{collections::BTreeMap, sync::Mutex};

struct TestHost {
    document: Mutex<Document>,
    undo: Mutex<Vec<Document>>,
    outputs: Mutex<Vec<Output>>,
    http_calls: Mutex<Vec<String>>,
    instance_secrets: Mutex<BTreeMap<(String, String, String), String>>,
    race: AtomicBool,
    reject_preflight: AtomicBool,
}
impl TestHost {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            document: Mutex::new(Document {
                id: "doc".into(),
                revision: 1,
                name: "Smoke".into(),
                mode: "exaudio.local".into(),
                read_only: false,
                speakers: vec![],
                lanes: vec![Lane {
                    id: "lane".into(),
                    speaker_id: None,
                    label: "Audio".into(),
                    source_id: None,
                }],
                sources: vec![],
                metadata: vec![],
                segments: vec![Segment {
                    id: "s".into(),
                    lane: "lane".into(),
                    speaker_id: None,
                    start_ms: 0,
                    end_ms: 1000,
                    text: String::new(),
                    metadata: vec![],
                }],
            }),
            undo: Mutex::new(Vec::new()),
            outputs: Mutex::new(Vec::new()),
            http_calls: Mutex::new(Vec::new()),
            instance_secrets: Mutex::new(BTreeMap::new()),
            race: AtomicBool::new(false),
            reject_preflight: AtomicBool::new(false),
        })
    }
    fn removed(&self) -> usize {
        self.outputs
            .lock()
            .unwrap()
            .iter()
            .filter(|o| matches!(o, Output::Removed))
            .count()
    }
    fn activated(&self) -> usize {
        self.outputs
            .lock()
            .unwrap()
            .iter()
            .filter(|o| matches!(o, Output::Activated))
            .count()
    }
}
impl HostServices for TestHost {
    fn preflight(&self, cx: &CallContext<'_>, outputs: &[Output]) -> Result<(), String> {
        cx.check()?;
        if self.reject_preflight.load(Ordering::Relaxed) {
            return Err("native registry collision".into());
        }
        let registered: BTreeSet<_> = outputs
            .iter()
            .filter_map(|o| {
                if let Output::Registered(r) = o {
                    Some(r.id.as_str())
                } else {
                    None
                }
            })
            .collect();
        if outputs.iter().any(
            |o| matches!(o, Output::Ui(tree) if !registered.contains(tree.contribution.as_str())),
        ) {
            return Err("unregistered staged UI".into());
        }
        Ok(())
    }
    fn snapshot(&self, cx: &CallContext<'_>, id: &str, _: usize) -> Result<Document, String> {
        cx.check()?;
        if id != "doc" {
            return Err("document out of scope".into());
        }
        Ok(self.document.lock().unwrap().clone())
    }
    fn apply_transaction(&self, cx: &CallContext<'_>, value: Transaction) -> Result<u64, String> {
        cx.check()?;
        let mut document = self.document.lock().unwrap();
        if self.race.swap(false, Ordering::Relaxed) {
            document.revision += 1;
        }
        if value.expected_revision != document.revision {
            return Err("atomic stale revision".into());
        }
        let mut next = document.clone();
        for edit in value.edits {
            match edit {
                Edit::ReplaceText(edit) => {
                    let segment = next
                        .segments
                        .iter_mut()
                        .find(|s| s.id == edit.segment_id)
                        .ok_or("missing segment")?;
                    let start = edit.start_byte as usize;
                    let end = edit.end_byte as usize;
                    if start > end
                        || !segment.text.is_char_boundary(start)
                        || !segment.text.is_char_boundary(end)
                    {
                        return Err("invalid UTF-8 range".into());
                    }
                    segment.text.replace_range(start..end, &edit.text);
                }
                Edit::InsertSegment(segment) => {
                    if next.segments.iter().any(|s| s.id == segment.id) {
                        return Err("duplicate segment".into());
                    }
                    next.segments.push(segment);
                }
                Edit::RemoveSegment(id) => {
                    let index = next
                        .segments
                        .iter()
                        .position(|s| s.id == id)
                        .ok_or("missing segment")?;
                    next.segments.remove(index);
                }
                Edit::ReplaceSegment(segment) => {
                    let target = next
                        .segments
                        .iter_mut()
                        .find(|s| s.id == segment.id)
                        .ok_or("missing segment")?;
                    *target = segment;
                }
            }
        }
        next.revision += 1;
        self.undo.lock().unwrap().push(document.clone());
        *document = next;
        Ok(document.revision)
    }
    fn audio(&self, _: &CallContext<'_>, _: AudioRequest, _: usize) -> Result<AudioData, String> {
        Err("no audio in fixture".into())
    }
    fn http_once(
        &self,
        cx: &CallContext<'_>,
        request: &HttpRequest,
        _: usize,
    ) -> Result<HttpResponse, String> {
        cx.check()?;
        self.http_calls.lock().unwrap().push(request.url.clone());
        Ok(HttpResponse {
            status: 302,
            headers: vec![Header {
                name: "location".into(),
                value: "https://other.example/data".into(),
            }],
            body: vec![],
        })
    }
    fn storage_get(
        &self,
        _: &CallContext<'_>,
        _: &str,
        _: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        panic!("denied capability reached native service")
    }
    fn storage_set(&self, _: &CallContext<'_>, _: &str, _: &[u8]) -> Result<(), String> {
        panic!("denied capability reached native service")
    }
    fn secret_get(&self, _: &CallContext<'_>, _: &str, _: usize) -> Result<Option<String>, String> {
        panic!("denied capability reached native service")
    }
    fn instance_secret_get(
        &self,
        cx: &CallContext<'_>,
        instance: &ProviderInstance,
        key: &str,
        _: usize,
    ) -> Result<Option<String>, String> {
        cx.check()?;
        Ok(self
            .instance_secrets
            .lock()
            .unwrap()
            .get(&(instance.provider.clone(), instance.id.clone(), key.into()))
            .cloned())
    }
    fn instance_secret_set(
        &self,
        cx: &CallContext<'_>,
        instance: &ProviderInstance,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), String> {
        cx.check()?;
        let scoped = (instance.provider.clone(), instance.id.clone(), key.into());
        let mut secrets = self.instance_secrets.lock().unwrap();
        match value {
            Some(value) => {
                secrets.insert(scoped, value.into());
            }
            None => {
                secrets.remove(&scoped);
            }
        }
        Ok(())
    }
    fn instance_storage_get(
        &self,
        _: &CallContext<'_>,
        _: &ProviderInstance,
        _: &str,
        _: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        panic!("denied capability reached native service")
    }
    fn instance_storage_set(
        &self,
        _: &CallContext<'_>,
        _: &ProviderInstance,
        _: &str,
        _: &[u8],
    ) -> Result<(), String> {
        panic!("denied capability reached native service")
    }
    fn selection_get(&self, _: &CallContext<'_>, _: &str) -> Result<Selection, String> {
        Err("no selection in fixture".into())
    }
    fn selection_set(&self, _: &CallContext<'_>, _: Selection) -> Result<(), String> {
        panic!("denied capability reached native service")
    }
    fn transport_get(&self, _: &CallContext<'_>, _: &str) -> Result<Transport, String> {
        Err("no transport in fixture".into())
    }
    fn transport_set(&self, _: &CallContext<'_>, _: Transport) -> Result<(), String> {
        panic!("denied capability reached native service")
    }
    fn emit(&self, id: &str, output: Output) {
        assert_eq!(id, "example.hello-panel");
        self.outputs.lock().unwrap().push(output);
    }
}
fn state(host: Arc<TestHost>, grants: BTreeSet<Permission>) -> State {
    state_declaring(host, grants, BTreeSet::new())
}
fn state_declaring(
    host: Arc<TestHost>,
    grants: BTreeSet<Permission>,
    providers: BTreeSet<String>,
) -> State {
    let mut state = State::new(
        "example.hello-panel".into(),
        host,
        grants,
        providers,
        Limits::default(),
        Arc::new(AtomicBool::new(false)),
    );
    state.staging = false;
    state.deadline = Instant::now() + Duration::from_secs(5);
    state
}
fn instance(provider: &str) -> ProviderInstance {
    ProviderInstance {
        provider: provider.into(),
        id: "alice".into(),
        label: "Alice".into(),
    }
}
fn transaction(revision: u64) -> Transaction {
    Transaction {
        document_id: "doc".into(),
        expected_revision: revision,
        label: "Insert".into(),
        edits: vec![Edit::ReplaceText(TextEdit {
            segment_id: "s".into(),
            start_byte: 0,
            end_byte: 0,
            text: "hello".into(),
        })],
    }
}
#[test]
fn queued_native_requests_share_the_exact_cancellation_token() {
    let cancellation = Arc::new(AtomicBool::new(false));
    let cx = CallContext {
        extension_id: "example.hello-panel",
        deadline: Instant::now() + Duration::from_secs(5),
        cancellation: &cancellation,
    };
    let queued_token = Arc::clone(cx.cancellation);
    assert!(Arc::ptr_eq(&queued_token, &cancellation));
    cx.check().unwrap();
    cancellation.store(true, Ordering::Relaxed);
    assert!(queued_token.load(Ordering::Relaxed));
    assert!(cx.check().is_err());
}

#[test]
fn denies_undeclared_capabilities_before_native_dispatch() {
    let mut state = state(TestHost::new(), BTreeSet::new());
    assert!(state.snapshot("doc".into()).is_err());
    assert!(state.propose(transaction(1)).is_err());
    assert!(state.storage_get("key".into()).is_err());
    assert!(state.storage_set("key".into(), vec![]).is_err());
    assert!(state.secret_get("token".into()).is_err());
    assert!(
        state
            .selection_set(Selection {
                document_id: "doc".into(),
                revision: 1,
                ranges: vec![]
            })
            .is_err()
    );
    assert!(
        state
            .transport_set(Transport {
                document_id: "doc".into(),
                position_ms: 0,
                playing: true,
                rate: 1.0
            })
            .is_err()
    );
}
#[test]
fn exact_https_grants_and_redirects_are_checked_before_dispatch() {
    let host = TestHost::new();
    let grants = BTreeSet::from([Permission::Network("allowed.example".into())]);
    for url in [
        "http://allowed.example/",
        "https://allowed.example.evil/",
        "https://allowed.example:8443/",
        "https://user@allowed.example/",
        "https://allowed.example./",
    ] {
        assert!(host::check_url(url, &grants).is_err(), "{url}");
    }
    let mut state = state(host.clone(), grants);
    assert!(
        state
            .http(HttpRequest {
                url: "https://allowed.example/".into(),
                method: "GET".into(),
                headers: vec![],
                body: vec![]
            })
            .is_err()
    );
    assert_eq!(
        host.http_calls.lock().unwrap().as_slice(),
        ["https://allowed.example/"]
    );
}
#[test]
fn revision_validation_includes_atomic_commit_race() {
    let host = TestHost::new();
    let mut state = state(host.clone(), BTreeSet::from([Permission::DocumentEdit]));
    assert!(state.propose(transaction(0)).unwrap_err().contains("stale"));
    host.race.store(true, Ordering::Relaxed);
    assert!(
        state
            .propose(transaction(1))
            .unwrap_err()
            .contains("atomic stale")
    );
    assert!(host.undo.lock().unwrap().is_empty());
    assert!(host.document.lock().unwrap().segments[0].text.is_empty());
    assert_eq!(state.propose(transaction(2)).unwrap(), 3);
    assert_eq!(host.undo.lock().unwrap().len(), 1);
}
#[test]
fn activation_cannot_mutate_and_output_budget_cannot_be_reset_by_guest() {
    let host = TestHost::new();
    let mut state = state(host, BTreeSet::from([Permission::DocumentEdit]));
    state.staging = true;
    assert!(
        state
            .propose(transaction(1))
            .unwrap_err()
            .contains("activation")
    );
    state.remaining = 2;
    assert!(
        state
            .register(Registration {
                id: "large".into(),
                kind: ContributionKind::Panel,
                title: String::new()
            })
            .is_err()
    );
    assert!(state.registrations.is_empty());
}
#[test]
fn malformed_ui_parent_and_recursive_depth_are_rejected() {
    let mut state = state(TestHost::new(), BTreeSet::new());
    state
        .register(Registration {
            id: "panel".into(),
            kind: ContributionKind::Panel,
            title: "Panel".into(),
        })
        .unwrap();
    let node = |id: &str, parent| UiNode {
        id: id.into(),
        parent,
        widget: Widget::Stack,
        config: WidgetConfig::Layout(LayoutConfig { gap: 0, padding: 0 }),
        text: String::new(),
        value: String::new(),
        enabled: true,
        command: None,
    };
    assert!(
        state
            .publish_ui(UiTree {
                contribution: "panel".into(),
                nodes: vec![node("root", Some(0))]
            })
            .is_err()
    );
    state.limits.ui_depth = 1;
    assert!(
        state
            .publish_ui(UiTree {
                contribution: "panel".into(),
                nodes: vec![node("root", None), node("child", Some(0))]
            })
            .is_err()
    );
}
#[test]
fn instance_namespaces_are_scoped_to_declared_providers() {
    let host = TestHost::new();
    let grants = BTreeSet::from([Permission::Secrets, Permission::Storage]);
    // Extension A declares the provider and may use the instance namespace.
    let mut owner = state_declaring(
        host.clone(),
        grants.clone(),
        BTreeSet::from(["example.hello.account".to_string()]),
    );
    owner
        .instance_secret_set(
            instance("example.hello.account"),
            "name".into(),
            Some("Alice".into()),
        )
        .unwrap();
    assert_eq!(
        owner
            .instance_secret_get(instance("example.hello.account"), "name".into())
            .unwrap()
            .as_deref(),
        Some("Alice")
    );
    // Extension B holds the same grants but does not declare the provider: denied before
    // any native dispatch, for secrets and storage alike.
    let mut other = state_declaring(
        host.clone(),
        grants.clone(),
        BTreeSet::from(["example.other.account".to_string()]),
    );
    assert!(
        other
            .instance_secret_get(instance("example.hello.account"), "name".into())
            .unwrap_err()
            .contains("does not declare")
    );
    assert!(
        other
            .instance_secret_set(instance("example.hello.account"), "name".into(), None)
            .is_err()
    );
    assert!(
        other
            .instance_storage_get(instance("example.hello.account"), "cache".into())
            .is_err()
    );
    assert!(
        other
            .instance_storage_set(instance("example.hello.account"), "cache".into(), vec![1])
            .is_err()
    );
    // Declaring the provider is not enough without the grant.
    let mut ungranted = state_declaring(
        host,
        BTreeSet::new(),
        BTreeSet::from(["example.hello.account".to_string()]),
    );
    assert!(
        ungranted
            .instance_secret_get(instance("example.hello.account"), "name".into())
            .is_err()
    );
    // Instance ids become directory names: path-unsafe ids never reach the service.
    let mut traversal = instance("example.hello.account");
    traversal.id = "../alice".into();
    assert!(owner.instance_secret_get(traversal, "name".into()).is_err());
}
#[test]
fn provider_setup_and_status_round_trip_through_registered_contribution() {
    let host = TestHost::new();
    let mut state = state_declaring(
        host.clone(),
        BTreeSet::new(),
        BTreeSet::from(["example.hello.account".to_string()]),
    );
    // Only declared provider types can be registered.
    assert!(
        state
            .register(Registration {
                id: "example.other.account".into(),
                kind: ContributionKind::Provider,
                title: "Other".into(),
            })
            .is_err()
    );
    // Unregistered contributions cannot finish or cancel a setup.
    let done = ProviderSetupResult {
        provider: "example.hello.account".into(),
        request_id: 7,
        id: "alice".into(),
        label: "Alice".into(),
    };
    assert!(state.provider_setup_done(done.clone()).is_err());
    assert!(
        state
            .provider_setup_cancel("example.hello.account".into(), 7)
            .is_err()
    );
    state
        .register(Registration {
            id: "example.hello.account".into(),
            kind: ContributionKind::Provider,
            title: "Hello account".into(),
        })
        .unwrap();
    state.provider_setup_done(done.clone()).unwrap();
    state
        .provider_setup_cancel("example.hello.account".into(), 8)
        .unwrap();
    let mut bad = done.clone();
    bad.id = "Alice Smith".into();
    assert!(state.provider_setup_done(bad).is_err());
    state
        .publish_provider_status(ProviderStatus {
            instance: instance("example.hello.account"),
            state: ProviderState::Ready,
            detail: "signed in".into(),
        })
        .unwrap();
    let outputs = host.outputs.lock().unwrap();
    assert!(matches!(
        &outputs[1],
        Output::ProviderSetupDone(value) if value.request_id == 7 && value.id == "alice"
    ));
    assert!(matches!(
        &outputs[2],
        Output::ProviderSetupCancelled { request_id: 8, .. }
    ));
    assert!(matches!(
        &outputs[3],
        Output::ProviderStatus(value) if value.state == ProviderState::Ready
    ));
    drop(outputs);
    // Activation cannot complete setups: that would resolve a request nobody made.
    state.staging = true;
    assert!(state.provider_setup_done(done).is_err());
}
fn wait(ticket: Ticket) -> Result<()> {
    // Test-only blocking wait; production UI uses Ticket::try_recv.
    ticket
        .receiver
        .recv_timeout(Duration::from_secs(90))
        .context("worker did not complete")?
}

#[test]
fn canonical_alias_amplification_traps_before_owned_graph_allocation() {
    let bytes = crate::lift::alias_probe(state(TestHost::new(), BTreeSet::new()), 65_536)
        .expect("the well-formed probe runs with sufficient allocation budget");
    assert_eq!(
        bytes, 32_768,
        "two entries alias the same 16 KiB guest string"
    );
    assert!(crate::lift::alias_probe(state(TestHost::new(), BTreeSet::new()), 128).is_err());
}
fn package(bytes: &[u8]) -> Package {
    let manifest = serde_json::from_str(include_str!(
        "../../../extensions/examples/hello-panel/manifest.json"
    ))
    .unwrap();
    Package {
        manifest,
        component: bytes.to_vec(),
        grants: BTreeSet::from([
            Permission::DocumentRead,
            Permission::DocumentEdit,
            Permission::Secrets,
        ]),
        required_by_open_modes: vec!["^0.1".parse().unwrap()],
    }
}
fn click() -> Invocation {
    Invocation::Ui(UiEvent {
        contribution: "hello-panel".into(),
        node: "insert".into(),
        kind: UiEventKind::Click,
        value: String::new(),
        document_id: Some("doc".into()),
    })
}

#[test]
#[ignore = "run smoke.ps1 after building the real example with runtime-probes"]
fn real_component_activation_ui_transactions_diagnostics_reload_and_isolation() {
    let bytes = std::fs::read(
        std::env::var_os("EXAUDIO_TEST_COMPONENT").expect("EXAUDIO_TEST_COMPONENT required"),
    )
    .unwrap();
    let host = TestHost::new();
    let limits = Limits {
        call_timeout: Duration::from_secs(2),
        ..Limits::default()
    };
    let runtime =
        Runtime::spawn("example.hello-panel".into(), host.clone(), limits.clone()).unwrap();
    wait(runtime.load(package(&bytes)).unwrap()).unwrap();
    assert!(
        host.outputs
            .lock()
            .unwrap()
            .iter()
            .any(|o| matches!(o, Output::Ui(tree) if tree.contribution == "hello-panel"))
    );
    wait(
        runtime
            .invoke(Invocation::Event(Event::DocumentOpened(DocumentEvent {
                document_id: "doc".into(),
                revision: 1,
            })))
            .unwrap(),
    )
    .unwrap();
    assert!(
        host.outputs
            .lock()
            .unwrap()
            .iter()
            .any(|o| matches!(o, Output::Diagnostics(d) if d.items.len() == 1))
    );
    wait(runtime.invoke(click()).unwrap()).unwrap();
    assert_eq!(
        host.document.lock().unwrap().segments[0].text,
        "Hello from Exaudio! "
    );
    assert_eq!(host.undo.lock().unwrap().len(), 1);
    assert!(
        host.outputs
            .lock()
            .unwrap()
            .iter()
            .any(|o| matches!(o, Output::Diagnostics(d) if d.revision == 2 && d.items.is_empty()))
    );
    // Provider round trip: setup UI → typed name → sign in → instance secret → status →
    // mode-open publishes the greeting document for that instance.
    wait(
        runtime
            .invoke(Invocation::Event(Event::ProviderSetup(ProviderSetup {
                provider: "example.hello.account".into(),
                request_id: 41,
                instance: None,
            })))
            .unwrap(),
    )
    .unwrap();
    let account_ui = |node: &str, kind, value: &str| {
        Invocation::Ui(UiEvent {
            contribution: "example.hello.account".into(),
            node: node.into(),
            kind,
            value: value.into(),
            document_id: None,
        })
    };
    wait(
        runtime
            .invoke(account_ui("name", UiEventKind::Change, "Alice Smith"))
            .unwrap(),
    )
    .unwrap();
    wait(
        runtime
            .invoke(account_ui("sign-in", UiEventKind::Click, ""))
            .unwrap(),
    )
    .unwrap();
    let signed_in = host
        .outputs
        .lock()
        .unwrap()
        .iter()
        .find_map(|o| match o {
            Output::ProviderSetupDone(v) if v.request_id == 41 => Some(v.clone()),
            _ => None,
        })
        .expect("setup finished");
    assert_eq!(
        (signed_in.id.as_str(), signed_in.label.as_str()),
        ("alice-smith", "Alice Smith")
    );
    let account = ProviderInstance {
        provider: signed_in.provider.clone(),
        id: signed_in.id.clone(),
        label: signed_in.label.clone(),
    };
    assert_eq!(
        host.instance_secrets.lock().unwrap().get(&(
            account.provider.clone(),
            account.id.clone(),
            "name".into()
        )),
        Some(&"Alice Smith".to_string())
    );
    wait(
        runtime
            .invoke(Invocation::Event(Event::ProviderStatus(account.clone())))
            .unwrap(),
    )
    .unwrap();
    assert!(host.outputs.lock().unwrap().iter().any(|o| matches!(
        o,
        Output::ProviderStatus(v) if v.state == ProviderState::Ready && v.instance.id == "alice-smith"
    )));
    wait(
        runtime
            .invoke(Invocation::Event(Event::ModeOpen(ModeOpen {
                mode: "example.hello.notes".into(),
                request_id: 42,
                key: String::new(),
                instance: Some(account.clone()),
            })))
            .unwrap(),
    )
    .unwrap();
    assert!(host.outputs.lock().unwrap().iter().any(|o| matches!(
        o,
        Output::ModeDocument(v) if v.request_id == 42 && v.document.segments[0].text == "Hello, Alice Smith"
    )));
    wait(
        runtime
            .invoke(Invocation::Event(Event::ProviderSignOut(account.clone())))
            .unwrap(),
    )
    .unwrap();
    assert!(host.instance_secrets.lock().unwrap().is_empty());
    wait(
        runtime
            .invoke(Invocation::Event(Event::ProviderStatus(account)))
            .unwrap(),
    )
    .unwrap();
    assert!(host.outputs.lock().unwrap().iter().any(|o| matches!(
        o,
        Output::ProviderStatus(v) if v.state == ProviderState::NeedsSignIn
    )));
    let removed = host.removed();
    assert!(wait(runtime.load(package(b"not a component")).unwrap()).is_err());
    assert_eq!(host.removed(), removed);
    let mut incompatible = package(&bytes);
    incompatible.manifest.version = "2.0.0".parse().unwrap();
    assert!(wait(runtime.load(incompatible).unwrap()).is_err());
    host.reject_preflight.store(true, Ordering::Relaxed);
    let activated = host.activated();
    assert!(
        wait(runtime.load(package(&bytes)).unwrap())
            .unwrap_err()
            .to_string()
            .contains("collision")
    );
    assert_eq!(host.activated(), activated);
    assert_eq!(host.removed(), removed);
    host.reject_preflight.store(false, Ordering::Relaxed);
    wait(runtime.invoke(click()).unwrap()).unwrap(); // Previous instance is still usable.
    wait(runtime.load(package(&bytes)).unwrap()).unwrap();
    assert_eq!(host.removed(), removed);
    assert_eq!(host.activated(), activated + 1);
    let mut denied = package(&bytes);
    denied.grants.clear();
    wait(runtime.load(denied).unwrap()).unwrap();
    assert!(
        wait(runtime.invoke(click()).unwrap())
            .unwrap_err()
            .to_string()
            .contains("permission denied")
    );
    wait(runtime.unload().unwrap()).unwrap();
    assert!(runtime.load(package(&bytes)).is_err());
    for command in ["probe-trap", "probe-fuel", "probe-memory"] {
        let runtime =
            Runtime::spawn("example.hello-panel".into(), host.clone(), limits.clone()).unwrap();
        wait(runtime.load(package(&bytes)).unwrap()).unwrap();
        let removed = host.removed();
        let activated = host.activated();
        let trapped = runtime
            .invoke(Invocation::Command(Command {
                id: command.into(),
                document_id: None,
                selection: None,
                arguments: vec![],
            }))
            .unwrap();
        let queued = runtime.load(package(&bytes));
        assert!(wait(trapped).is_err(), "{command}");
        if let Ok(ticket) = queued {
            assert!(
                wait(ticket).is_err(),
                "queued load revived a terminal generation"
            );
        }
        assert_eq!(
            host.removed(),
            removed + 1,
            "{command} did not isolate instance"
        );
        assert_eq!(host.activated(), activated);
        assert!(runtime.invoke(click()).is_err());
    }
    let runtime = Runtime::spawn("example.hello-panel".into(), host.clone(), limits).unwrap();
    wait(runtime.load(package(&bytes)).unwrap()).unwrap();
    let removed = host.removed();
    wait(runtime.unload().unwrap()).unwrap();
    assert_eq!(host.removed(), removed + 1);
    assert!(runtime.invoke(click()).is_err());
}
