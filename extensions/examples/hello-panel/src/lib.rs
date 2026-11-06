use sonus_sdk::{Guest, host, types::*};

struct HelloPanel;

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
        sonus_sdk::command_panel(
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
        })
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
            sonus_sdk::insert_text(
                &document,
                &segment.id,
                0,
                "Hello from Sonus! ".into(),
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
                    text: "Hello from Sonus! ".into(),
                    metadata: Vec::new(),
                })],
            }
        };
        host::propose(&edits)?;
        diagnostics(&id)
    }
    fn on_ui_event(value: UiEvent) -> Result<(), String> {
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
        match value {
            Event::DocumentOpened(value) | Event::DocumentChanged(value) => {
                diagnostics(&value.document_id)
            }
            _ => Ok(()),
        }
    }
}

sonus_sdk::bindings::export!(HelloPanel with_types_in sonus_sdk::bindings);
