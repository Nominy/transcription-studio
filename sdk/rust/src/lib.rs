//! Build with `cargo build --target wasm32-unknown-unknown --release`, then
//! `wasm-tools component new <module.wasm> -o extension.wasm`.
//! The SDK uses only Sonus imports. Do not use std filesystem, network, clock or
//! random APIs: the host deliberately does not link WASI capabilities.
pub mod bindings {
    wit_bindgen::generate!({ path: "../../wit", world: "extension", pub_export_macro: true });
}
pub use bindings::exports::sonus::extension::guest::Guest;
pub use bindings::sonus::extension::{host, types};

/// Build a revision-pinned, single undo-unit insertion. UTF-8 byte offsets are
/// checked locally for ergonomics and authoritatively checked again by the host.
pub fn insert_text(
    document: &types::Document,
    segment_id: &str,
    byte: usize,
    text: String,
    label: String,
) -> Result<types::Transaction, String> {
    let segment = document
        .segments
        .iter()
        .find(|segment| segment.id == segment_id)
        .ok_or("segment not found")?;
    if !segment.text.is_char_boundary(byte) {
        return Err("invalid UTF-8 insertion boundary".into());
    }
    let byte = u32::try_from(byte).map_err(|_| "text offset too large")?;
    Ok(types::Transaction {
        document_id: document.id.clone(),
        expected_revision: document.revision,
        label,
        edits: vec![types::Edit::ReplaceText(types::TextEdit {
            segment_id: segment_id.into(),
            start_byte: byte,
            end_byte: byte,
            text,
        })],
    })
}

/// Register a command and one panel. The command ID remains local to this extension;
/// the host namespaces all registrations with the verified manifest ID.
pub fn command_panel(command: &str, panel: &str, title: &str, button: &str) -> Result<(), String> {
    host::register(&types::Registration {
        id: command.into(),
        kind: types::ContributionKind::Command,
        title: button.into(),
    })?;
    host::register(&types::Registration {
        id: panel.into(),
        kind: types::ContributionKind::Panel,
        title: title.into(),
    })?;
    host::publish_ui(&types::UiTree {
        contribution: panel.into(),
        nodes: vec![
            types::UiNode {
                id: "root".into(),
                parent: None,
                widget: types::Widget::Stack,
                config: types::WidgetConfig::Layout(types::LayoutConfig {
                    gap: 8,
                    padding: 12,
                }),
                text: title.into(),
                value: String::new(),
                enabled: true,
                command: None,
            },
            types::UiNode {
                id: "insert".into(),
                parent: Some(0),
                widget: types::Widget::Button,
                config: types::WidgetConfig::Plain,
                text: button.into(),
                value: String::new(),
                enabled: true,
                command: Some(command.into()),
            },
        ],
    })
}
