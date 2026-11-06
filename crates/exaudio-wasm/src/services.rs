use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use crate::types::*;

/// Every call executes on the extension worker, never on the UI thread. Implementations
/// must respect deadline/cancellation, bound responses before allocation, and must not
/// re-enter the runtime. Marshal editor mutations to its authoritative document owner.
pub struct CallContext<'a> {
    pub extension_id: &'a str,
    pub deadline: Instant,
    /// Clone this exact token into queued native requests; never mirror its value.
    pub cancellation: &'a Arc<AtomicBool>,
}
impl CallContext<'_> {
    pub fn check(&self) -> Result<(), String> {
        if self.cancellation.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        if Instant::now() >= self.deadline {
            return Err("deadline exceeded".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum Output {
    /// Successful initial activation/replacement boundary. Atomically reset owned
    /// contributions without terminating this runtime generation.
    Activated,
    Registered(Registration),
    Diagnostics(Diagnostics),
    Completions(Completions),
    Decorations(Decorations),
    Overlay(Overlay),
    Ui(UiTree),
    Progress(Progress),
    ModeDocument(ModeDocument),
    Import(ImportResult),
    Export(ExportResult),
    /// The guest finished the sign-in requested by `provider-setup{request-id}`.
    ProviderSetupDone(ProviderSetupResult),
    ProviderSetupCancelled {
        provider: String,
        request_id: u64,
    },
    /// Answer to a `provider-status` event. The owner may deliver it to a waiting
    /// requester synchronously; it is never queued behind editor outputs.
    ProviderStatus(ProviderStatus),
    /// Removes ALL contributions, diagnostics, UI and queued work owned by the ID.
    Removed,
}

pub trait HostServices: Send + Sync + 'static {
    /// Validate the entire staged activation batch on the authoritative registry,
    /// without changing visible contributions. Reject collisions/unsupported UI/
    /// stale outputs here, BEFORE the prior working guest is deactivated.
    /// Retain any necessary internal reservation until Activated or cancellation.
    fn preflight(&self, cx: &CallContext<'_>, outputs: &[Output]) -> Result<(), String>;
    fn snapshot(
        &self,
        cx: &CallContext<'_>,
        id: &str,
        max_bytes: usize,
    ) -> Result<Document, String>;
    /// REQUIRED: atomically compare expected_revision to the authoritative revision,
    /// validate all segment IDs/UTF-8 byte boundaries, and apply all edits as ONE undo
    /// transaction or apply NONE. Return the new revision. Runtime's preliminary read
    /// is defense in depth, not a replacement for this compare-and-commit requirement.
    fn apply_transaction(&self, cx: &CallContext<'_>, value: Transaction) -> Result<u64, String>;
    fn audio(
        &self,
        cx: &CallContext<'_>,
        value: AudioRequest,
        max_samples: usize,
    ) -> Result<AudioData, String>;
    /// One HTTP exchange ONLY. Redirect following MUST be disabled. Enforce deadline,
    /// cancellation and max_bytes while streaming, not after buffering the response.
    /// No ambient cookies, credentials, proxies or automatic authentication injection.
    fn http_once(
        &self,
        cx: &CallContext<'_>,
        value: &HttpRequest,
        max_bytes: usize,
    ) -> Result<HttpResponse, String>;
    /// Keys are scoped to cx.extension_id, including quotas and persistent storage.
    fn storage_get(
        &self,
        cx: &CallContext<'_>,
        key: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String>;
    fn storage_set(&self, cx: &CallContext<'_>, key: &str, value: &[u8]) -> Result<(), String>;
    fn secret_get(
        &self,
        cx: &CallContext<'_>,
        key: &str,
        max_bytes: usize,
    ) -> Result<Option<String>, String>;
    /// Per-instance namespaces for provider types. The runtime has already verified that
    /// the instance's provider is declared by this extension and that the matching grant
    /// exists; implementations isolate storage per (extension, provider, instance).
    fn instance_secret_get(
        &self,
        cx: &CallContext<'_>,
        instance: &ProviderInstance,
        key: &str,
        max_bytes: usize,
    ) -> Result<Option<String>, String>;
    /// `None` removes the secret.
    fn instance_secret_set(
        &self,
        cx: &CallContext<'_>,
        instance: &ProviderInstance,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), String>;
    fn instance_storage_get(
        &self,
        cx: &CallContext<'_>,
        instance: &ProviderInstance,
        key: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String>;
    fn instance_storage_set(
        &self,
        cx: &CallContext<'_>,
        instance: &ProviderInstance,
        key: &str,
        value: &[u8],
    ) -> Result<(), String>;
    fn selection_get(&self, cx: &CallContext<'_>, document_id: &str) -> Result<Selection, String>;
    /// Atomically validate revision and UTF-8 ranges before changing selection.
    fn selection_set(&self, cx: &CallContext<'_>, value: Selection) -> Result<(), String>;
    fn transport_get(&self, cx: &CallContext<'_>, document_id: &str) -> Result<Transport, String>;
    fn transport_set(&self, cx: &CallContext<'_>, value: Transport) -> Result<(), String>;
    /// Nonblocking delivery to the editor. Never call guest code from this callback.
    /// Outputs are already bounded, validated and namespaced by extension ID.
    fn emit(&self, extension_id: &str, output: Output);
}
