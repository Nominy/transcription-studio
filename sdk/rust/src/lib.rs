//! Build with `cargo build --target wasm32-unknown-unknown --release`, then
//! `wasm-tools component new <module.wasm> -o extension.wasm`.
//! The SDK uses only Exaudio host imports. Do not use std filesystem, network, clock or
//! random APIs: the host deliberately does not link WASI capabilities.
pub mod bindings {
    wit_bindgen::generate!({ path: "../../wit", world: "extension", pub_export_macro: true });
}
pub use bindings::exports::exaudio::extension::guest::Guest;
pub use bindings::exaudio::extension::{host, types};

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

/// Callbacks of a provider type (an account the host signs into while opening a mode).
/// Route `Guest::on_event` through [`provider_event`] to dispatch them.
pub trait ProviderHandler {
    /// The shell asked for a sign-in: publish UI for `contribution = setup.provider` and
    /// finish with [`provider_setup_done`] or [`provider_setup_cancel`] using
    /// `setup.request_id`. `setup.instance` is set when re-authenticating an account.
    fn on_provider_setup(setup: types::ProviderSetup) -> Result<(), String>;
    /// Answer with [`publish_status`] before returning; an unanswered request is an error.
    fn on_provider_status(instance: types::ProviderInstance) -> Result<(), String>;
    /// Delete the instance's credentials; the host removes its namespace afterwards.
    fn on_provider_sign_out(instance: types::ProviderInstance) -> Result<(), String>;
    /// The shell abandoned a setup (dialog closed, timeout). Drop any pending state.
    fn on_provider_setup_cancelled(_request_id: u64) -> Result<(), String> {
        Ok(())
    }
}
/// Dispatches provider events to `T`; returns `None` for every other event so the caller
/// keeps handling documents, modes and files itself.
pub fn provider_event<T: ProviderHandler>(event: &types::Event) -> Option<Result<(), String>> {
    Some(match event {
        types::Event::ProviderSetup(setup) => T::on_provider_setup(setup.clone()),
        types::Event::ProviderSetupCancelled(request) => T::on_provider_setup_cancelled(*request),
        types::Event::ProviderStatus(instance) => T::on_provider_status(instance.clone()),
        types::Event::ProviderSignOut(instance) => T::on_provider_sign_out(instance.clone()),
        _ => return None,
    })
}
/// Registers a provider type declared in the manifest's `providers`.
pub fn register_provider(id: &str, title: &str) -> Result<(), String> {
    host::register(&types::Registration {
        id: id.into(),
        kind: types::ContributionKind::Provider,
        title: title.into(),
    })
}
/// Registers a mode declared in the manifest's `modes`.
pub fn register_mode(id: &str, title: &str) -> Result<(), String> {
    host::register(&types::Registration {
        id: id.into(),
        kind: types::ContributionKind::Mode,
        title: title.into(),
    })
}
/// Reads one secret of an account this extension's provider owns (requires `secrets`).
pub fn instance_secret_get(
    instance: &types::ProviderInstance,
    key: &str,
) -> Result<Option<String>, String> {
    host::instance_secret_get(instance, key)
}
/// Writes (`Some`) or deletes (`None`) one secret of an account (requires `secrets`).
pub fn instance_secret_set(
    instance: &types::ProviderInstance,
    key: &str,
    value: Option<&str>,
) -> Result<(), String> {
    host::instance_secret_set(instance, key, value)
}
/// Finishes a setup. `id` must be stable per account and consist of `[a-z0-9-]`; `label`
/// is what users see. See [`instance_id`] for deriving one from a display name.
pub fn provider_setup_done(
    provider: &str,
    request_id: u64,
    id: &str,
    label: &str,
) -> Result<(), String> {
    host::provider_setup_done(&types::ProviderSetupResult {
        provider: provider.into(),
        request_id,
        id: id.into(),
        label: label.into(),
    })
}
pub fn provider_setup_cancel(provider: &str, request_id: u64) -> Result<(), String> {
    host::provider_setup_cancel(provider, request_id)
}
/// Answers a `provider-status` event.
pub fn publish_status(
    instance: &types::ProviderInstance,
    state: types::ProviderState,
    detail: &str,
) -> Result<(), String> {
    host::publish_provider_status(&types::ProviderStatus {
        instance: instance.clone(),
        state,
        detail: detail.into(),
    })
}
/// A path-safe instance id from a human identifier: lowercase `[a-z0-9-]`, at most 64
/// bytes, `"account"` when nothing survives.
pub fn instance_id(source: &str) -> String {
    let mut id = String::with_capacity(source.len().min(64));
    let mut dash = true;
    for c in source.chars().flat_map(char::to_lowercase) {
        if id.len() >= 64 {
            break;
        }
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            id.push(c);
            dash = false;
        } else if !dash {
            id.push('-');
            dash = true;
        }
    }
    while id.ends_with('-') {
        id.pop();
    }
    if id.is_empty() {
        id.push_str("account");
    }
    id
}
