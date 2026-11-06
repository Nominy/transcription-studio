use exaudio_sdk::{Guest, ProviderHandler, host, types::*};
use std::cell::RefCell;

const ACCOUNT: &str = "example.hello.account";
const NOTES: &str = "example.hello.notes";
const NAME_SECRET: &str = "name";

struct HelloPanel;

/// The sign-in currently on screen: the shell's request id and the name typed so far.
struct Setup {
    request_id: u64,
    name: String,
}
thread_local! {
    static SETUP: RefCell<Option<Setup>> = const { RefCell::new(None) };
}

fn sign_in_ui(hint: &str) -> Result<(), String> {
    let node = |id: &str, parent, widget, config, text: &str| UiNode {
        id: id.into(),
        parent,
        widget,
        config,
        text: text.into(),
        value: String::new(),
        enabled: true,
        command: None,
    };
    let mut nodes = vec![
        node(
            "root",
            None,
            Widget::Stack,
            WidgetConfig::Layout(LayoutConfig {
                gap: 8,
                padding: 12,
            }),
            "Hello account",
        ),
        node(
            "intro",
            Some(0),
            Widget::Text,
            WidgetConfig::Plain,
            "This sample account only needs a name.",
        ),
        node(
            "name",
            Some(0),
            Widget::TextInput,
            WidgetConfig::Input(InputConfig {
                placeholder: "Name".into(),
                multiline: false,
                read_only: false,
            }),
            "Name",
        ),
        node(
            "sign-in",
            Some(0),
            Widget::Button,
            WidgetConfig::Plain,
            "Sign in",
        ),
    ];
    if !hint.is_empty() {
        nodes.push(node(
            "hint",
            Some(0),
            Widget::Callout,
            WidgetConfig::Plain,
            hint,
        ));
    }
    host::publish_ui(&UiTree {
        contribution: ACCOUNT.into(),
        nodes,
    })
}

fn finish_sign_in() -> Result<(), String> {
    let Some((request_id, name)) = SETUP.with(|setup| {
        setup
            .borrow()
            .as_ref()
            .map(|setup| (setup.request_id, setup.name.trim().to_owned()))
    }) else {
        return Err("no sign-in in progress".into());
    };
    if name.is_empty() {
        return sign_in_ui("Enter a name to sign in.");
    }
    let instance = ProviderInstance {
        provider: ACCOUNT.into(),
        id: exaudio_sdk::instance_id(&name),
        label: name.clone(),
    };
    exaudio_sdk::instance_secret_set(&instance, NAME_SECRET, Some(&name))?;
    SETUP.with(|setup| setup.borrow_mut().take());
    exaudio_sdk::provider_setup_done(ACCOUNT, request_id, &instance.id, &name)
}

fn open_notes(request_id: u64, instance: Option<ProviderInstance>) -> Result<(), String> {
    let instance = instance.ok_or("Hello notes needs a Hello account")?;
    let name = exaudio_sdk::instance_secret_get(&instance, NAME_SECRET)?
        .ok_or("This account is signed out; sign in again")?;
    host::publish_mode_document(&ModeDocument {
        mode: NOTES.into(),
        request_id,
        document: Document {
            id: format!("hello-notes-{}", instance.id),
            revision: 0,
            name: format!("Hello notes for {name}"),
            mode: NOTES.into(),
            read_only: false,
            speakers: vec![],
            lanes: vec![Lane {
                id: "notes".into(),
                speaker_id: None,
                label: "Notes".into(),
                source_id: None,
            }],
            sources: vec![],
            metadata: vec![],
            segments: vec![Segment {
                id: "greeting".into(),
                lane: "notes".into(),
                speaker_id: None,
                start_ms: 0,
                end_ms: 2000,
                text: format!("Hello, {name}"),
                metadata: vec![],
            }],
        },
    })
}

impl ProviderHandler for HelloPanel {
    fn on_provider_setup(setup: ProviderSetup) -> Result<(), String> {
        let name = setup
            .instance
            .map(|instance| instance.label)
            .unwrap_or_default();
        SETUP.with(|slot| {
            *slot.borrow_mut() = Some(Setup {
                request_id: setup.request_id,
                name,
            })
        });
        sign_in_ui("")
    }
    fn on_provider_setup_cancelled(request_id: u64) -> Result<(), String> {
        SETUP.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot
                .as_ref()
                .is_some_and(|setup| setup.request_id == request_id)
            {
                slot.take();
            }
        });
        Ok(())
    }
    fn on_provider_status(instance: ProviderInstance) -> Result<(), String> {
        match exaudio_sdk::instance_secret_get(&instance, NAME_SECRET)? {
            Some(name) => exaudio_sdk::publish_status(
                &instance,
                ProviderState::Ready,
                &format!("Signed in as {name}"),
            ),
            None => exaudio_sdk::publish_status(&instance, ProviderState::NeedsSignIn, ""),
        }
    }
    fn on_provider_sign_out(instance: ProviderInstance) -> Result<(), String> {
        exaudio_sdk::instance_secret_set(&instance, NAME_SECRET, None)
    }
}

fn diagnostics(document_id: &str) -> Result<(), String> {
    let document = host::snapshot(document_id)?;
    let items = document
        .segments
        .iter()
        .filter(|segment| segment.text.trim().is_empty())
        .map(|segment| Diagnostic {
            id: format!("empty-{}", segment.id),
            range: TextRange {
                segment_id: segment.id.clone(),
                start_byte: 0,
                end_byte: segment.text.len() as u32,
            },
            severity: Severity::Hint,
            message: "This segment has no text. Try Insert greeting in the Hello panel.".into(),
            fix: None,
        })
        .collect();
    host::publish_diagnostics(&Diagnostics {
        provider: "empty-segments".into(),
        document_id: document.id,
        revision: document.revision,
        items,
    })
}

impl Guest for HelloPanel {
    fn activate() -> Result<(), String> {
        #[cfg(feature = "runtime-probes")]
        for id in ["probe-trap", "probe-fuel", "probe-memory"] {
            host::register(&Registration {
                id: id.into(),
                kind: ContributionKind::Command,
                title: id.into(),
            })?;
        }
        exaudio_sdk::command_panel(
            "insert-greeting",
            "hello-panel",
            "Hello from an extension",
            "Insert greeting",
        )?;
        host::register(&Registration {
            id: "empty-segments".into(),
            kind: ContributionKind::Diagnostics,
            title: "Empty segments".into(),
        })?;
        host::register(&Registration {
            id: "document-events".into(),
            kind: ContributionKind::Event,
            title: "Refresh empty-segment hints".into(),
        })?;
        exaudio_sdk::register_provider(ACCOUNT, "Hello account")?;
        exaudio_sdk::register_mode(NOTES, "Hello notes")
    }
    fn deactivate() {
        // The host removes every owned contribution, even if a guest traps here.
    }
    fn on_command(value: Command) -> Result<(), String> {
        #[cfg(all(feature = "runtime-probes", target_arch = "wasm32"))]
        match value.id.as_str() {
            "probe-trap" => core::arch::wasm32::unreachable(),
            "probe-fuel" => loop {
                core::hint::spin_loop();
            },
            "probe-memory" => {
                let data = vec![1u8; 64 * 1024 * 1024];
                core::hint::black_box(data);
                return Err("memory probe unexpectedly succeeded".into());
            }
            _ => {}
        }
        if value.id != "insert-greeting" {
            return Err("unknown command".into());
        }
        let id = value.document_id.ok_or("open a document first")?;
        let document = host::snapshot(&id)?;
        let edits = if let Some(segment) = document.segments.first() {
            exaudio_sdk::insert_text(
                &document,
                &segment.id,
                0,
                "Hello from Exaudio! ".into(),
                "Insert greeting".into(),
            )?
        } else {
            let lane = document
                .lanes
                .first()
                .ok_or("document has no lane to insert into")?;
            Transaction {
                document_id: document.id.clone(),
                expected_revision: document.revision,
                label: "Insert greeting".into(),
                edits: vec![Edit::InsertSegment(Segment {
                    id: format!("hello-{}", document.revision),
                    lane: lane.id.clone(),
                    speaker_id: lane.speaker_id.clone(),
                    start_ms: 0,
                    end_ms: 1000,
                    text: "Hello from Exaudio! ".into(),
                    metadata: Vec::new(),
                })],
            }
        };
        host::propose(&edits)?;
        diagnostics(&id)
    }
    fn on_ui_event(value: UiEvent) -> Result<(), String> {
        if value.contribution == ACCOUNT {
            return match (value.node.as_str(), value.kind) {
                ("name", UiEventKind::Change) => {
                    SETUP.with(|slot| {
                        if let Some(setup) = slot.borrow_mut().as_mut() {
                            setup.name = value.value;
                        }
                    });
                    Ok(())
                }
                ("name", UiEventKind::Submit) | ("sign-in", UiEventKind::Click) => finish_sign_in(),
                _ => Ok(()),
            };
        }
        if value.contribution == "hello-panel"
            && value.node == "insert"
            && value.kind == UiEventKind::Click
        {
            Self::on_command(Command {
                id: "insert-greeting".into(),
                document_id: value.document_id,
                selection: None,
                arguments: Vec::new(),
            })
        } else {
            Ok(())
        }
    }
    fn on_event(value: Event) -> Result<(), String> {
        if let Some(result) = exaudio_sdk::provider_event::<Self>(&value) {
            return result;
        }
        match value {
            Event::DocumentOpened(value) | Event::DocumentChanged(value) => {
                diagnostics(&value.document_id)
            }
            Event::ModeOpen(open) if open.mode == NOTES => {
                open_notes(open.request_id, open.instance)
            }
            _ => Ok(()),
        }
    }
}

exaudio_sdk::bindings::export!(HelloPanel with_types_in exaudio_sdk::bindings);
