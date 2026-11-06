//! Component workers exchange bounded requests with the authoritative GPUI owner.
//! No GPUI callback waits for a guest; only a guest worker waits for an owner reply.
mod development;
pub(crate) mod document;
mod io;
use crate::{ShellHandle, contributions::ContributionRegistry};
use anyhow::{Result, anyhow, bail};
pub use development::{reload_development, stop_watching, watch_development};
use document::{Publications, Reply, Request};
use exaudio_extension::{ExtensionManifest, Permission};
use exaudio_wasm::{
    CallContext, HostServices, Invocation, Limits, Output, Package, Runtime, Ticket, types as w,
};
use gpui_kit::*;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct ExtensionStatus {
    pub owner: String,
    pub version: String,
    pub state: String,
    pub last_event: String,
    pub logs: Vec<String>,
    pub contributions: Vec<String>,
    pub grants: BTreeSet<Permission>,
    pub development_directory: Option<PathBuf>,
}
/// Cross-document access is an explicit user decision, separate from service grants.
#[derive(Clone, Debug, Default)]
pub enum DocumentAccess {
    #[default]
    DeclaredModes,
    Modes(BTreeSet<String>),
    Global,
}
struct Entry {
    runtime: Runtime,
    host: Arc<Bridge>,
    manifest: ExtensionManifest,
    assets: BTreeMap<String, PathBuf>,
    publications: Arc<Mutex<Publications>>,
    status: ExtensionStatus,
    tickets: Vec<Ticket>,
    observer: Option<Subscription>,
    pending: Option<PendingActivation>,
    activation_error: Option<String>,
    access: DocumentAccess,
    ready: bool,
}
struct PendingActivation {
    manifest: ExtensionManifest,
    grants: BTreeSet<Permission>,
    assets: BTreeMap<String, PathBuf>,
}
pub struct ActivationTicket {
    receiver: mpsc::Receiver<Result<()>>,
    cancelled: Arc<AtomicBool>,
}
impl ActivationTicket {
    pub fn try_recv(&self) -> Result<Option<Result<()>>, mpsc::TryRecvError> {
        match self.receiver.try_recv() {
            Ok(value) => Ok(Some(value)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(error) => Err(error),
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Drop for ActivationTicket {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
#[derive(Default)]
pub struct WasmExtensions {
    entries: BTreeMap<String, Entry>,
    pumps: BTreeMap<String, Task<()>>,
    history: BTreeMap<String, ExtensionStatus>,
    watches: BTreeMap<String, development::Watch>,
    desired_scopes: BTreeMap<String, DocumentAccess>,
}
impl Global for WasmExtensions {}
#[derive(Clone)]
struct Scope {
    document: String,
    mode: String,
    entity: u64,
}
struct Bridge {
    owner: String,
    grants: Mutex<BTreeSet<Permission>>,
    /// Provider types declared by the current manifest; instance namespaces are scoped to them.
    providers: Mutex<BTreeSet<String>>,
    scope: Mutex<Option<Scope>>,
    sender: async_channel::Sender<Message>,
    root: PathBuf,
    /// Last answer per instance (`provider/id`) plus the requester waiting for the next one.
    status: Mutex<ProviderStatusSlots>,
    removed: AtomicBool,
    overflow: AtomicBool,
}
/// Cached answers expire so expired credentials are noticed without a sign-out; guests may
/// also publish a fresh status at any time (e.g. after a failed request) to replace one.
const STATUS_TTL: Duration = Duration::from_secs(30);
#[derive(Default)]
struct ProviderStatusSlots {
    cached: BTreeMap<String, (Instant, w::ProviderStatus)>,
    waiting: BTreeMap<String, mpsc::SyncSender<w::ProviderStatus>>,
}
fn instance_key(instance: &w::ProviderInstance) -> String {
    format!("{}/{}", instance.provider, instance.id)
}
/// Where one extension keeps an instance's secrets and storage. Removed with the instance
/// and on sign-out; never shared between extensions.
pub(crate) fn instance_data_dir(owner: &str, instance: &crate::ProviderInstance) -> PathBuf {
    crate::providers::instance_dir(instance)
        .join("wasm")
        .join(owner)
}
pub(crate) fn shell_instance(instance: &w::ProviderInstance) -> crate::ProviderInstance {
    crate::ProviderInstance {
        provider: instance.provider.clone(),
        id: instance.id.clone(),
        label: instance.label.clone(),
    }
}
pub(crate) fn guest_instance(instance: &crate::ProviderInstance) -> w::ProviderInstance {
    w::ProviderInstance {
        provider: instance.provider.clone(),
        id: instance.id.clone(),
        label: instance.label.clone(),
    }
}
struct Envelope {
    request: Request,
    permission: Permission,
    scope: Scope,
    deadline: Instant,
    cancelled: Arc<AtomicBool>,
    reply: mpsc::SyncSender<Result<Reply, String>>,
}
enum Message {
    Request(Envelope),
    Output(Output),
    Completion(w::CompletionRequest),
    Preflight {
        outputs: Vec<Output>,
        deadline: Instant,
        cancellation: Arc<AtomicBool>,
        reply: mpsc::SyncSender<Result<(), String>>,
    },
    Barrier(mpsc::SyncSender<Result<()>>),
}
impl Bridge {
    fn permitted(&self, cx: &CallContext<'_>, permission: Permission) -> Result<(), String> {
        cx.check()?;
        if cx.extension_id != self.owner || self.removed.load(Ordering::Acquire) {
            return Err("extension is not active".into());
        }
        if !self
            .grants
            .lock()
            .map_err(|_| "grant lock poisoned")?
            .contains(&permission)
        {
            return Err("permission denied".into());
        }
        Ok(())
    }
    /// Defense in depth behind the runtime's own check: an instance namespace is reachable
    /// only for providers this extension's manifest declares.
    fn instance_root(
        &self,
        cx: &CallContext<'_>,
        permission: Permission,
        instance: &w::ProviderInstance,
    ) -> Result<PathBuf, String> {
        self.permitted(cx, permission)?;
        if !self
            .providers
            .lock()
            .map_err(|_| "provider lock poisoned")?
            .contains(&instance.provider)
        {
            return Err("instance belongs to a provider this extension does not declare".into());
        }
        if instance.id.is_empty()
            || instance.id.len() > 64
            || !instance
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("invalid instance id".into());
        }
        Ok(instance_data_dir(&self.owner, &shell_instance(instance)))
    }
    fn cached_status(&self, instance: &w::ProviderInstance) -> Option<w::ProviderStatus> {
        let mut slots = self.status.lock().ok()?;
        let key = instance_key(instance);
        match slots.cached.get(&key) {
            Some((at, value)) if at.elapsed() < STATUS_TTL => Some(value.clone()),
            Some(_) => {
                slots.cached.remove(&key);
                None
            }
            None => None,
        }
    }
    fn forget_status(&self, instance: &w::ProviderInstance) {
        if let Ok(mut slots) = self.status.lock() {
            let key = instance_key(instance);
            slots.cached.remove(&key);
            slots.waiting.remove(&key);
        }
    }
    fn await_status(&self, instance: &w::ProviderInstance) -> mpsc::Receiver<w::ProviderStatus> {
        let (sender, receiver) = mpsc::sync_channel(1);
        if let Ok(mut slots) = self.status.lock() {
            slots.waiting.insert(instance_key(instance), sender);
        }
        receiver
    }
    fn cancel_wait(&self, instance: &w::ProviderInstance) {
        if let Ok(mut slots) = self.status.lock() {
            slots.waiting.remove(&instance_key(instance));
        }
    }
    fn deliver_status(&self, value: &w::ProviderStatus) -> bool {
        let declared = self
            .providers
            .lock()
            .is_ok_and(|providers| providers.contains(&value.instance.provider));
        let Ok(mut slots) = self.status.lock() else {
            return false;
        };
        if !declared {
            return false;
        }
        let key = instance_key(&value.instance);
        if slots.cached.len() >= 256 && !slots.cached.contains_key(&key) {
            return false;
        }
        slots
            .cached
            .insert(key.clone(), (Instant::now(), value.clone()));
        if let Some(waiter) = slots.waiting.remove(&key) {
            let _ = waiter.try_send(value.clone());
        }
        true
    }
    fn request(
        &self,
        cx: &CallContext<'_>,
        permission: Permission,
        id: &str,
        request: Request,
    ) -> Result<Reply, String> {
        self.permitted(cx, permission.clone())?;
        let scope = self
            .scope
            .lock()
            .map_err(|_| "scope lock poisoned")?
            .clone()
            .ok_or("no associated document")?;
        if scope.document != id {
            return Err("document is not associated with this extension".into());
        }
        let (reply, receive) = mpsc::sync_channel(1);
        let cancelled = Arc::clone(cx.cancellation);
        self.sender
            .try_send(Message::Request(Envelope {
                request,
                permission,
                scope,
                deadline: cx.deadline,
                cancelled: cancelled.clone(),
                reply,
            }))
            .map_err(|_| "owner queue is full or closed")?;
        loop {
            if let Err(error) = cx.check() {
                cancelled.store(true, Ordering::Release);
                return Err(error);
            }
            let wait = cx
                .deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(5));
            match receive.recv_timeout(wait) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("document owner closed".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}
impl HostServices for Bridge {
    fn preflight(&self, cx: &CallContext<'_>, outputs: &[Output]) -> Result<(), String> {
        cx.check()?;
        if cx.extension_id != self.owner || self.removed.load(Ordering::Acquire) {
            return Err("inactive extension generation".into());
        }
        let (reply, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(Message::Preflight {
                outputs: outputs.to_vec(),
                deadline: cx.deadline,
                cancellation: Arc::clone(cx.cancellation),
                reply,
            })
            .map_err(|_| "preflight queue unavailable")?;
        loop {
            cx.check()?;
            match receiver.recv_timeout(
                cx.deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(5)),
            ) {
                Ok(result) => return result,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("document owner closed during preflight".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    fn snapshot(&self, cx: &CallContext<'_>, id: &str, max: usize) -> Result<w::Document, String> {
        match self.request(cx, Permission::DocumentRead, id, Request::Snapshot(max))? {
            Reply::Document(v) => Ok(v),
            _ => Err("invalid owner reply".into()),
        }
    }
    fn apply_transaction(
        &self,
        cx: &CallContext<'_>,
        value: w::Transaction,
    ) -> Result<u64, String> {
        let id = value.document_id.clone();
        match self.request(
            cx,
            Permission::DocumentEdit,
            &id,
            Request::Transaction(value),
        )? {
            Reply::Revision(v) => Ok(v),
            _ => Err("invalid owner reply".into()),
        }
    }
    fn audio(
        &self,
        cx: &CallContext<'_>,
        value: w::AudioRequest,
        max: usize,
    ) -> Result<w::AudioData, String> {
        let id = value.document_id.clone();
        match self.request(cx, Permission::AudioRead, &id, Request::Audio(value, max))? {
            Reply::Audio(v) => Ok(v),
            _ => Err("invalid owner reply".into()),
        }
    }
    fn selection_get(&self, cx: &CallContext<'_>, id: &str) -> Result<w::Selection, String> {
        match self.request(cx, Permission::DocumentRead, id, Request::SelectionGet)? {
            Reply::Selection(v) => Ok(v),
            _ => Err("invalid owner reply".into()),
        }
    }
    fn selection_set(&self, cx: &CallContext<'_>, v: w::Selection) -> Result<(), String> {
        let id = v.document_id.clone();
        self.request(
            cx,
            Permission::SelectionControl,
            &id,
            Request::SelectionSet(v),
        )
        .map(|_| ())
    }
    fn transport_get(&self, cx: &CallContext<'_>, id: &str) -> Result<w::Transport, String> {
        match self.request(cx, Permission::AudioRead, id, Request::TransportGet)? {
            Reply::Transport(v) => Ok(v),
            _ => Err("invalid owner reply".into()),
        }
    }
    fn transport_set(&self, cx: &CallContext<'_>, v: w::Transport) -> Result<(), String> {
        let id = v.document_id.clone();
        self.request(
            cx,
            Permission::TransportControl,
            &id,
            Request::TransportSet(v),
        )
        .map(|_| ())
    }
    fn http_once(
        &self,
        cx: &CallContext<'_>,
        v: &w::HttpRequest,
        max: usize,
    ) -> Result<w::HttpResponse, String> {
        io::http(self, cx, v, max)
    }
    fn storage_get(
        &self,
        cx: &CallContext<'_>,
        key: &str,
        max: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        self.permitted(cx, Permission::Storage)?;
        io::get(&self.root, cx, key, max)
    }
    fn storage_set(&self, cx: &CallContext<'_>, key: &str, value: &[u8]) -> Result<(), String> {
        self.permitted(cx, Permission::Storage)?;
        io::set(&self.root, cx, key, value)
    }
    fn secret_get(
        &self,
        cx: &CallContext<'_>,
        key: &str,
        max: usize,
    ) -> Result<Option<String>, String> {
        self.permitted(cx, Permission::Secrets)?;
        io::secret(self, cx, key, max)
    }
    fn instance_secret_get(
        &self,
        cx: &CallContext<'_>,
        instance: &w::ProviderInstance,
        key: &str,
        max: usize,
    ) -> Result<Option<String>, String> {
        let root = self.instance_root(cx, Permission::Secrets, instance)?;
        io::instance_secret_get(&root, cx, key, max)
    }
    fn instance_secret_set(
        &self,
        cx: &CallContext<'_>,
        instance: &w::ProviderInstance,
        key: &str,
        value: Option<&str>,
    ) -> Result<(), String> {
        let root = self.instance_root(cx, Permission::Secrets, instance)?;
        io::instance_secret_set(&root, cx, key, value)
    }
    fn instance_storage_get(
        &self,
        cx: &CallContext<'_>,
        instance: &w::ProviderInstance,
        key: &str,
        max: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        let root = self.instance_root(cx, Permission::Storage, instance)?;
        io::get(&root, cx, key, max)
    }
    fn instance_storage_set(
        &self,
        cx: &CallContext<'_>,
        instance: &w::ProviderInstance,
        key: &str,
        value: &[u8],
    ) -> Result<(), String> {
        let root = self.instance_root(cx, Permission::Storage, instance)?;
        io::set(&root, cx, key, value)
    }
    fn emit(&self, owner: &str, output: Output) {
        if owner != self.owner || self.removed.load(Ordering::Acquire) {
            return;
        }
        if matches!(output, Output::Removed) {
            self.removed.store(true, Ordering::Release);
        }
        // Status answers go straight to their requester (possibly blocked on the UI thread
        // for one call deadline); the owner queue only learns about them for logging.
        if let Output::ProviderStatus(value) = &output
            && !self.deliver_status(value)
        {
            return;
        }
        // Full queues fail closed. The owner drains the channel and purges the extension,
        // rather than leaving stale contributions because a Removed event was lost.
        if self.sender.try_send(Message::Output(output)).is_err() {
            self.removed.store(true, Ordering::Release);
            self.overflow.store(true, Ordering::Release);
        }
    }
}

pub fn activate(
    shell: ShellHandle,
    mut package: Package,
    data_root: PathBuf,
    assets: BTreeMap<String, PathBuf>,
    cx: &mut App,
) -> Result<ActivationTicket> {
    package.manifest.validate()?;
    package.manifest.check_host_compatibility()?;
    if !cx.has_global::<WasmExtensions>() {
        cx.set_global(WasmExtensions::default());
    }
    let owner = package.manifest.id.clone();
    if package
        .manifest
        .permissions
        .iter()
        .any(|permission| !package.grants.contains(permission))
    {
        bail!("This extension's permissions haven't been approved.");
    }
    let registry = cx.global::<ContributionRegistry>();
    let mut available = registry.manifests();
    available.retain(|manifest| manifest.id != owner);
    available.push(package.manifest.clone());
    for (dependency, version) in &package.manifest.dependencies {
        if !available.iter().any(|manifest| {
            &manifest.id == dependency
                && version.matches(&manifest.version)
                && registry.is_enabled(dependency)
        }) {
            bail!("Turn on {dependency} {version} first.");
        }
    }
    if let Some(model) = shell.editor_model(cx) {
        let mode = &model.read(cx).origin().mode_id;
        if let Some(mode_owner) = available
            .iter()
            .find(|manifest| manifest.modes.iter().any(|entry| &entry.id == mode))
        {
            let references = available.iter().collect::<Vec<_>>();
            exaudio_extension::resolve_mode(mode_owner, mode, &references)?;
            for manifest in &available {
                if registry.required_by_active_mode(&manifest.id) {
                    if let Some(requirement) = manifest.dependencies.get(&owner) {
                        package.required_by_open_modes.push(requirement.clone());
                    }
                    for mode in &manifest.modes {
                        if let Some(requirement) = mode.requires.get(&owner) {
                            package.required_by_open_modes.push(requirement.clone());
                        }
                    }
                }
            }
        } else if registry.required_by_active_mode(&owner) {
            bail!("This update would close the open workspace. Close the project first.");
        }
    }
    // Runtime::load validates and activates the replacement before discarding the previous
    // instance. The worker serializes callbacks and stages its registration outputs.
    if cx
        .global::<WasmExtensions>()
        .entries
        .get(&owner)
        .is_some_and(|entry| entry.host.removed.load(Ordering::Acquire))
    {
        remove_owned(&owner, cx);
    }
    if cx.global::<WasmExtensions>().entries.contains_key(&owner) {
        let ticket = cx.update_global::<WasmExtensions, _>(|state, _cx| {
            let entry = state.entries.get_mut(&owner).expect("existing entry");
            if entry.pending.is_some() {
                bail!("An update is already in progress.");
            }
            let pending = PendingActivation {
                manifest: package.manifest.clone(),
                grants: package.grants.clone(),
                assets,
            };
            let ticket = entry.runtime.load(package)?;
            *entry.host.grants.lock().expect("grants") = entry
                .status
                .grants
                .union(&pending.grants)
                .cloned()
                .collect();
            entry.pending = Some(pending);
            entry.activation_error = None;
            entry.status.state = "Reloading…".into();
            Ok(ticket)
        })?;
        return Ok(acknowledge(owner, ticket, cx));
    }
    let manifest = package.manifest.clone();
    let grants = package.grants.clone();
    let (sender, receiver) = async_channel::bounded(256);
    let access = cx
        .global::<WasmExtensions>()
        .desired_scopes
        .get(&owner)
        .cloned()
        .unwrap_or_default();
    let initial_scope = shell.editor_model(cx).and_then(|model| {
        let mode = model.read(cx).doc().environment().origin.mode.clone();
        mode_allowed(&owner, &mode, &access, cx).then(|| Scope {
            document: document_handle(&model),
            mode,
            entity: model.entity_id().as_u64(),
        })
    });
    let host = Arc::new(Bridge {
        owner: owner.clone(),
        grants: Mutex::new(grants.clone()),
        providers: Mutex::new(declared_providers(&manifest)),
        scope: Mutex::new(initial_scope),
        sender,
        root: data_root.join(&owner),
        status: Mutex::new(ProviderStatusSlots::default()),
        removed: AtomicBool::new(false),
        overflow: AtomicBool::new(false),
    });
    let runtime = Runtime::spawn(owner.clone(), host.clone(), Limits::default())?;
    let ticket = runtime.load(package)?;
    let status = ExtensionStatus {
        owner: owner.clone(),
        version: manifest.version.to_string(),
        state: "Starting…".into(),
        last_event: "Starting…".into(),
        logs: vec![],
        contributions: vec![],
        grants: grants.clone(),
        development_directory: None,
    };
    cx.update_global::<ContributionRegistry, _>(|registry, _| {
        registry.register_manifest(manifest.clone(), grants.into_iter().collect(), true)
    })?;
    let pending = PendingActivation {
        manifest: manifest.clone(),
        grants: status.grants.clone(),
        assets: assets.clone(),
    };
    cx.update_global::<WasmExtensions, _>(|state, _| {
        state.history.remove(&owner);
        state.entries.insert(
            owner.clone(),
            Entry {
                runtime,
                host: host.clone(),
                manifest,
                assets,
                publications: Arc::new(Mutex::new(Publications::default())),
                status,
                tickets: vec![],
                observer: None,
                pending: Some(pending),
                activation_error: None,
                access,
                ready: false,
            },
        );
    });
    cx.global::<WasmExtensions>().entries[&owner]
        .publications
        .lock()
        .expect("publications")
        .sender = Some(host.sender.clone());
    let pump_owner = owner.clone();
    let pump_shell = shell.clone();
    let task = cx.spawn(async move |cx| {
        while let Ok(message) = receiver.recv().await {
            cx.update(|cx| {
                if !cx
                    .global::<WasmExtensions>()
                    .entries
                    .get(&pump_owner)
                    .is_some_and(|entry| Arc::ptr_eq(&entry.host, &host))
                {
                    return;
                }
                if host.overflow.load(Ordering::Acquire) {
                    remove_owned(&pump_owner, cx);
                    record(
                        &pump_owner,
                        "Stopped",
                        "The extension sent too much data and was stopped",
                        cx,
                    );
                    return;
                }
                match message {
                    Message::Request(envelope) => {
                        let result = if envelope.cancelled.load(Ordering::Acquire)
                            || Instant::now() >= envelope.deadline
                        {
                            Err("cancelled before document dispatch".into())
                        } else if host.removed.load(Ordering::Acquire) {
                            Err("extension unloaded".into())
                        } else if !host
                            .grants
                            .lock()
                            .is_ok_and(|grants| grants.contains(&envelope.permission))
                        {
                            Err("permission revoked before dispatch".into())
                        } else if !host.scope.lock().is_ok_and(|scope| {
                            scope.as_ref().is_some_and(|scope| {
                                scope.entity == envelope.scope.entity
                                    && scope.mode == envelope.scope.mode
                            })
                        }) {
                            Err("document association revoked before dispatch".into())
                        } else {
                            document::dispatch(&pump_shell, &envelope, cx)
                        };
                        let _ = envelope.reply.try_send(result);
                    }
                    Message::Output(output) => consume(&pump_owner, &pump_shell, output, cx),
                    Message::Completion(request) => invoke(
                        &pump_owner,
                        Invocation::Event(w::Event::Completion(request)),
                        cx,
                    ),
                    Message::Preflight {
                        outputs,
                        deadline,
                        cancellation,
                        reply,
                    } => {
                        let result =
                            if cancellation.load(Ordering::Acquire) || Instant::now() >= deadline {
                                Err("activation cancelled before preflight".into())
                            } else {
                                crate::wasm_ui::preflight(&pump_owner, &outputs, cx)
                                    .map_err(|e| e.to_string())
                            };
                        let _ = reply.try_send(result);
                    }
                    Message::Barrier(reply) => {
                        let result = cx.update_global::<WasmExtensions, _>(|extensions, _| {
                            let entry =
                                extensions.entries.get_mut(&pump_owner).ok_or_else(|| {
                                    anyhow!("The extension was removed while starting.")
                                })?;
                            if let Some(error) = entry.activation_error.take() {
                                bail!(error);
                            }
                            entry.pending = None;
                            entry.ready = true;
                            entry.status.state = "Running".into();
                            Ok(())
                        });
                        cx.update_global::<ContributionRegistry, _>(|registry, _| {
                            registry.release_mode_reservations(&pump_owner)
                        });
                        if result.is_err() {
                            remove_owned(&pump_owner, cx);
                        }
                        let _ = reply.try_send(result);
                    }
                }
            });
        }
    });
    cx.update_global::<WasmExtensions, _>(|state, _| {
        state.pumps.insert(owner.clone(), task);
    });
    document_opened(shell, cx);
    Ok(acknowledge(owner, ticket, cx))
}

fn acknowledge(owner: String, ticket: Ticket, cx: &mut App) -> ActivationTicket {
    let (reply, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel = cancelled.clone();
    let generation = cx.global::<WasmExtensions>().entries[&owner].host.clone();
    cx.spawn(async move |cx| {
        loop {
            if cancel.load(Ordering::Acquire) {
                ticket.cancel();
            }
            match ticket.try_recv() {
                Ok(None) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(5))
                        .await
                }
                Ok(Some(Ok(()))) => {
                    cx.update(|cx| {
                        let sender = cx
                            .global::<WasmExtensions>()
                            .entries
                            .get(&owner)
                            .filter(|entry| Arc::ptr_eq(&entry.host, &generation))
                            .map(|entry| entry.host.sender.clone());
                        if let Some(sender) = sender {
                            if let Err(error) = sender.try_send(Message::Barrier(reply)) {
                                if let Message::Barrier(reply) = error.into_inner() {
                                    let _ = reply.try_send(Err(anyhow!(
                                        "The extension stopped responding while starting."
                                    )));
                                }
                                remove_owned(&owner, cx);
                            }
                        } else {
                            let _ = reply.try_send(Err(anyhow!(
                                "The extension was removed while starting."
                            )));
                        }
                    });
                    break;
                }
                result => {
                    let error = match result {
                        Ok(Some(Err(error))) => error,
                        Err(error) => anyhow!(error),
                        _ => unreachable!(),
                    };
                    cx.update(|cx| {
                        cx.update_global::<WasmExtensions, _>(|extensions, _| {
                            if let Some(entry) = extensions
                                .entries
                                .get_mut(&owner)
                                .filter(|entry| Arc::ptr_eq(&entry.host, &generation))
                            {
                                entry.pending = None;
                                *entry.host.grants.lock().expect("grants") =
                                    entry.status.grants.clone();
                                entry.status.state = if entry.status.contributions.is_empty() {
                                    "Problem"
                                } else {
                                    "Running"
                                }
                                .into();
                                push_log(&mut entry.status, &error.to_string());
                            }
                        })
                    });
                    cx.update(|cx| {
                        if !cx
                            .global::<WasmExtensions>()
                            .entries
                            .get(&owner)
                            .is_some_and(|entry| Arc::ptr_eq(&entry.host, &generation))
                        {
                            return;
                        }
                        cx.update_global::<ContributionRegistry, _>(|registry, _| {
                            registry.release_mode_reservations(&owner)
                        });
                        if cx
                            .global::<WasmExtensions>()
                            .entries
                            .get(&owner)
                            .is_some_and(|entry| !entry.ready)
                        {
                            remove_owned(&owner, cx);
                        }
                    });
                    let _ = reply.try_send(Err(error));
                    break;
                }
            }
        }
    })
    .detach();
    ActivationTicket {
        receiver,
        cancelled,
    }
}

pub fn disable(owner: &str, cx: &mut App) -> Result<ActivationTicket> {
    if cx
        .global::<ContributionRegistry>()
        .required_by_active_mode(owner)
    {
        bail!("This extension is needed by the open workspace.");
    }
    if !cx.has_global::<WasmExtensions>() {
        cx.set_global(WasmExtensions::default());
    }
    stop_watching(owner, cx);
    if cx.has_global::<WasmExtensions>() {
        cx.update_global::<WasmExtensions, _>(|state, _| {
            state.desired_scopes.remove(owner);
        });
    }
    remove_owned(owner, cx);
    record(owner, "Stopped", "Stopped", cx);
    let (reply, receiver) = mpsc::sync_channel(1);
    let _ = reply.try_send(Ok(()));
    Ok(ActivationTicket {
        receiver,
        cancelled: Arc::new(AtomicBool::new(false)),
    })
}
pub fn status(cx: &App) -> Vec<ExtensionStatus> {
    if !cx.has_global::<WasmExtensions>() {
        return vec![];
    }
    let state = cx.global::<WasmExtensions>();
    state
        .entries
        .values()
        .map(|entry| entry.status.clone())
        .chain(state.history.values().cloned())
        .collect()
}
pub fn invoke(owner: &str, invocation: Invocation, cx: &mut App) {
    if !cx.has_global::<WasmExtensions>() {
        return;
    }
    let request = match &invocation {
        Invocation::Event(w::Event::ImportRequest(value)) => Some(value.request_id),
        Invocation::Event(w::Event::ExportRequest(value)) => Some(value.request_id),
        Invocation::Event(w::Event::ProviderSetup(value)) => Some(value.request_id),
        _ => None,
    };
    let Some(entry) = cx.global::<WasmExtensions>().entries.get(owner) else {
        if let Some(request) = request {
            crate::wasm_ui::request_failed(owner, request, cx);
        }
        return;
    };
    let generation = entry.host.clone();
    let ticket = match entry.runtime.invoke(invocation) {
        Ok(ticket) => ticket,
        Err(error) => {
            if let Some(request) = request {
                crate::wasm_ui::request_failed(owner, request, cx);
            }
            record(owner, "Problem", &error.to_string(), cx);
            return;
        }
    };
    let owner = owner.to_owned();
    cx.spawn(async move |cx| {
        loop {
            let result = match ticket.try_recv() {
                Ok(None) => {
                    cx.background_executor()
                        .timer(Duration::from_millis(10))
                        .await;
                    continue;
                }
                Ok(Some(result)) => result,
                Err(error) => Err(anyhow!(error)),
            };
            if let Err(error) = result {
                cx.update(|cx| {
                    // Completion of a dead generation must never alter its replacement.
                    if cx
                        .global::<WasmExtensions>()
                        .entries
                        .get(&owner)
                        .is_some_and(|entry| !Arc::ptr_eq(&entry.host, &generation))
                    {
                        return;
                    }
                    if let Some(request) = request {
                        crate::wasm_ui::request_failed(&owner, request, cx);
                    }
                    record(&owner, "Problem", &error.to_string(), cx);
                });
            }
            break;
        }
    })
    .detach();
}
fn push_log(status: &mut ExtensionStatus, message: &str) {
    let mut end = message.len().min(2048);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    let message = &message[..end];
    status.last_event = message.to_owned();
    if status.logs.len() == 64 {
        status.logs.remove(0);
    }
    status.logs.push(message.to_owned());
}
fn record(owner: &str, state: &str, message: &str, cx: &mut App) {
    cx.update_global::<WasmExtensions, _>(|extensions, _| {
        let status = if let Some(entry) = extensions.entries.get_mut(owner) {
            Some(&mut entry.status)
        } else {
            extensions.history.get_mut(owner)
        };
        if let Some(status) = status {
            status.state = state.into();
            push_log(status, message);
        }
    });
    cx.refresh_windows();
}
fn consume(owner: &str, shell: &ShellHandle, output: Output, cx: &mut App) {
    if matches!(output, Output::Removed) {
        remove_owned(owner, cx);
        return;
    }
    if matches!(output, Output::Activated) {
        let replacing = cx
            .global::<WasmExtensions>()
            .entries
            .get(owner)
            .is_some_and(|entry| entry.pending.is_some());
        if replacing {
            crate::contributions::clear_owner(owner, cx);
            crate::wasm_ui::remove(owner, cx);
            cx.update_global::<WasmExtensions, _>(|extensions, cx| {
                if let Some(entry) = extensions.entries.get_mut(owner) {
                    document::clear(&entry.publications, cx);
                    if let Some(pending) = &entry.pending {
                        entry.manifest = pending.manifest.clone();
                        entry.assets = pending.assets.clone();
                        entry.status.version = pending.manifest.version.to_string();
                        entry.status.grants = pending.grants.clone();
                        *entry.host.grants.lock().expect("grants") = pending.grants.clone();
                        *entry.host.providers.lock().expect("providers") =
                            declared_providers(&pending.manifest);
                        *entry.host.status.lock().expect("status") = ProviderStatusSlots::default();
                        entry
                            .publications
                            .lock()
                            .expect("publications")
                            .completion_registered = false;
                        entry.status.contributions.clear();
                        let result = cx.update_global::<ContributionRegistry, _>(|registry, _| {
                            registry.register_manifest(
                                entry.manifest.clone(),
                                entry.status.grants.iter().cloned().collect(),
                                true,
                            )
                        });
                        if let Err(error) = result {
                            entry.activation_error = Some(error.to_string());
                        }
                    }
                    if let Some(model) = shell.editor_model(cx)
                        && entry
                            .host
                            .scope
                            .lock()
                            .expect("scope")
                            .as_ref()
                            .is_some_and(|scope| scope.entity == model.entity_id().as_u64())
                    {
                        document::install(&entry.publications, &model, cx);
                    }
                }
            });
        } else {
            remove_owned(owner, cx);
        }
        return;
    }
    if !cx
        .global::<WasmExtensions>()
        .entries
        .get(owner)
        .is_some_and(|entry| !entry.host.removed.load(Ordering::Acquire))
    {
        return;
    }
    if let Some((id, revision)) = published_document(&output) {
        let Some(model) = shell.editor_model(cx) else {
            return;
        };
        if !document_associated(owner, &model, cx)
            || document_handle(&model) != id
            || model.read(cx).doc().session().current_edit_revision() != revision
        {
            record(
                owner,
                "Running",
                "Ignored an update for a project that is closed or off limits",
                cx,
            );
            return;
        }
    }
    let activation_critical = matches!(
        output,
        Output::Registered(_) | Output::Ui(_) | Output::Progress(_)
    );
    let result = match output {
        Output::Registered(registration) => {
            if matches!(registration.kind, w::ContributionKind::Completions)
                && let Some(entry) = cx.global::<WasmExtensions>().entries.get(owner)
            {
                entry
                    .publications
                    .lock()
                    .expect("publications")
                    .completion_registered = true;
            }
            crate::wasm_ui::register(owner, shell.clone(), registration, cx)
        }
        Output::Ui(tree) => crate::wasm_ui::publish(owner, tree, cx),
        Output::Diagnostics(value) => document::publish_diagnostics(owner, shell, value, cx),
        Output::Decorations(value) => document::publish_decorations(owner, shell, value, cx),
        Output::Completions(value) => document::publish_completions(owner, shell, value, cx),
        Output::Overlay(value) => crate::wasm_ui::publish_overlay(owner, shell, value, cx),
        Output::Progress(value) => crate::wasm_ui::progress(owner, value, cx),
        Output::ModeDocument(value) => crate::wasm_ui::mode_document(owner, shell, value, cx),
        Output::Import(value) => crate::wasm_ui::import_result(owner, value, cx),
        Output::Export(value) => crate::wasm_ui::export_result(owner, value, cx),
        Output::ProviderSetupDone(value) => crate::wasm_ui::provider_setup_done(owner, value, cx),
        Output::ProviderSetupCancelled {
            provider,
            request_id,
        } => crate::wasm_ui::provider_setup_cancelled(owner, &provider, request_id, cx),
        // Already delivered to its requester by the bridge; only logged here.
        Output::ProviderStatus(_) => Ok(()),
        Output::Removed | Output::Activated => unreachable!(),
    };
    match result {
        Ok(()) => record(owner, "Running", "Update received", cx),
        Err(error) => {
            cx.update_global::<WasmExtensions, _>(|extensions, _| {
                if let Some(entry) = extensions.entries.get_mut(owner)
                    && activation_critical
                    && entry.pending.is_some()
                {
                    entry.activation_error = Some(error.to_string());
                }
            });
            record(owner, "Problem", &error.to_string(), cx);
        }
    }
}
fn remove_owned(owner: &str, cx: &mut App) {
    crate::contributions::force_remove_owner(owner, cx);
    crate::wasm_ui::remove(owner, cx);
    cx.update_global::<WasmExtensions, _>(|extensions, cx| {
        if let Some(mut entry) = extensions.entries.remove(owner) {
            entry.host.removed.store(true, Ordering::Release);
            entry.host.sender.close();
            entry.tickets.iter().for_each(Ticket::cancel);
            entry.observer = None;
            document::clear(&entry.publications, cx);
            entry.status.contributions.clear();
            entry.status.state = "inactive".into();
            if extensions.history.len() >= 128
                && let Some(first) = extensions.history.keys().next().cloned()
            {
                extensions.history.remove(&first);
            }
            extensions.history.insert(owner.into(), entry.status);
            // Dropping Runtime cancels in-flight work, closes admission and never joins
            // the extension thread on GPUI. Worker exit is bounded by its call deadline.
        }
        extensions.pumps.remove(owner);
    });
    cx.refresh_windows();
}
pub fn document_opened(shell: ShellHandle, cx: &mut App) {
    if !cx.has_global::<WasmExtensions>() {
        return;
    }
    let Some(model) = shell.editor_model(cx) else {
        return;
    };
    let document = document_handle(&model);
    let mode = model.read(cx).doc().environment().origin.mode.clone();
    let revision = model.read(cx).doc().session().current_edit_revision();
    let entity = model.entity_id().as_u64();
    let requirements = cx
        .global::<ContributionRegistry>()
        .resolve_mode_requirements(&mode)
        .map(|resolved| resolved.extensions)
        .unwrap_or_default();
    let eligible = cx
        .global::<WasmExtensions>()
        .entries
        .iter()
        .filter(|(owner, entry)| access_allows(owner, &mode, &entry.access, &requirements))
        .map(|(owner, _)| owner.clone())
        .collect::<BTreeSet<_>>();
    cx.update_global::<WasmExtensions, _>(|extensions, cx| {
        for (owner, entry) in &mut extensions.entries {
            if entry.host.removed.load(Ordering::Acquire) && entry.pending.is_none() {
                continue;
            }
            if !eligible.contains(owner) {
                entry.observer = None;
                if let Some(scope) = entry.host.scope.lock().expect("scope").take() {
                    crate::wasm_ui::clear_document(owner, &scope.document, cx);
                    if let Ok(ticket) = entry
                        .runtime
                        .invoke(Invocation::Event(w::Event::DocumentClosed(scope.document)))
                    {
                        entry.tickets.push(ticket);
                    }
                }
                document::clear(&entry.publications, cx);
                continue;
            }
            if entry.observer.is_some()
                && entry
                    .host
                    .scope
                    .lock()
                    .expect("scope")
                    .as_ref()
                    .is_some_and(|scope| scope.entity == entity)
            {
                continue;
            }
            *entry.host.scope.lock().expect("scope") = Some(Scope {
                document: document.clone(),
                mode: mode.clone(),
                entity,
            });
            document::install(&entry.publications, &model, cx);
            let owner = owner.clone();
            let event_document = document.clone();
            let mut last_revision = revision;
            entry.observer = Some(cx.observe(&model, move |model, cx| {
                let revision = model.read(cx).doc().session().current_edit_revision();
                if revision != last_revision {
                    last_revision = revision;
                    invoke(
                        &owner,
                        Invocation::Event(w::Event::DocumentChanged(w::DocumentEvent {
                            document_id: event_document.clone(),
                            revision,
                        })),
                        cx,
                    );
                }
            }));
            if let Ok(ticket) = entry
                .runtime
                .invoke(Invocation::Event(w::Event::DocumentOpened(
                    w::DocumentEvent {
                        document_id: document.clone(),
                        revision,
                    },
                )))
            {
                entry.tickets.push(ticket);
            }
        }
    });
}
pub fn document_closed(cx: &mut App) {
    if !cx.has_global::<WasmExtensions>() {
        return;
    }
    cx.update_global::<WasmExtensions, _>(|extensions, cx| {
        for entry in extensions.entries.values_mut() {
            entry.observer = None;
            if let Some(scope) = entry.host.scope.lock().expect("scope").take() {
                crate::wasm_ui::clear_document(&entry.host.owner, &scope.document, cx);
                if let Ok(ticket) = entry
                    .runtime
                    .invoke(Invocation::Event(w::Event::DocumentClosed(scope.document)))
                {
                    entry.tickets.push(ticket);
                }
            }
            document::clear(&entry.publications, cx);
        }
    });
}
pub(crate) fn assets(owner: &str, cx: &App) -> BTreeMap<String, PathBuf> {
    cx.global::<WasmExtensions>()
        .entries
        .get(owner)
        .map(|e| e.assets.clone())
        .unwrap_or_default()
}
pub(crate) fn contribution_added(owner: &str, id: &str, cx: &mut App) {
    cx.update_global::<WasmExtensions, _>(|state, _| {
        if let Some(entry) = state.entries.get_mut(owner)
            && !entry.status.contributions.iter().any(|x| x == id)
        {
            entry.status.contributions.push(id.into());
        }
    });
}
pub(crate) fn has_permission(owner: &str, permission: Permission, cx: &App) -> bool {
    cx.try_global::<WasmExtensions>()
        .and_then(|state| state.entries.get(owner))
        .is_some_and(|entry| {
            !entry.host.removed.load(Ordering::Acquire) && entry.status.grants.contains(&permission)
        })
}
fn mode_allowed(owner: &str, mode: &str, access: &DocumentAccess, cx: &App) -> bool {
    mode_allowed_in(owner, mode, access, cx.global::<ContributionRegistry>())
}
fn mode_allowed_in(
    owner: &str,
    mode: &str,
    access: &DocumentAccess,
    registry: &ContributionRegistry,
) -> bool {
    let requirements = registry
        .resolve_mode_requirements(mode)
        .map(|resolved| resolved.extensions)
        .unwrap_or_default();
    access_allows(owner, mode, access, &requirements)
}

fn access_allows(
    owner: &str,
    mode: &str,
    access: &DocumentAccess,
    requirements: &BTreeSet<String>,
) -> bool {
    match access {
        DocumentAccess::Global => true,
        DocumentAccess::Modes(modes) => modes.contains(mode),
        DocumentAccess::DeclaredModes => requirements.contains(owner),
    }
}
pub fn set_document_access(
    owner: &str,
    access: DocumentAccess,
    shell: ShellHandle,
    cx: &mut App,
) -> Result<()> {
    if !exaudio_extension::manifest::valid_id(owner) {
        bail!("This extension ID isn't valid.");
    }
    if !cx.has_global::<WasmExtensions>() {
        cx.set_global(WasmExtensions::default());
    }
    cx.update_global::<WasmExtensions, _>(|state, _| {
        if state.desired_scopes.len() >= 256 && !state.desired_scopes.contains_key(owner) {
            bail!("Too many extensions have been given project access.");
        }
        state.desired_scopes.insert(owner.into(), access.clone());
        if let Some(entry) = state.entries.get_mut(owner) {
            entry.access = access;
            entry.observer = None;
        }
        Ok(())
    })?;
    document_opened(shell, cx);
    Ok(())
}
pub(crate) fn document_associated(
    owner: &str,
    model: &Entity<crate::editor::model::EditorModel>,
    cx: &App,
) -> bool {
    cx.try_global::<WasmExtensions>()
        .and_then(|state| state.entries.get(owner))
        .is_some_and(|entry| {
            !entry.host.removed.load(Ordering::Acquire)
                && entry.host.scope.lock().is_ok_and(|scope| {
                    scope
                        .as_ref()
                        .is_some_and(|scope| scope.entity == model.entity_id().as_u64())
                })
        })
}
fn published_document(output: &Output) -> Option<(&str, u64)> {
    match output {
        Output::Diagnostics(v) => Some((&v.document_id, v.revision)),
        Output::Decorations(v) => Some((&v.document_id, v.revision)),
        Output::Completions(v) => Some((&v.document_id, v.revision)),
        Output::Overlay(v) => Some((&v.document_id, v.revision)),
        _ => None,
    }
}
pub(crate) fn pending_manifest(owner: &str, cx: &App) -> Option<ExtensionManifest> {
    cx.global::<WasmExtensions>()
        .entries
        .get(owner)
        .map(|entry| {
            entry.pending.as_ref().map_or_else(
                || entry.manifest.clone(),
                |pending| pending.manifest.clone(),
            )
        })
}
fn declared_providers(manifest: &ExtensionManifest) -> BTreeSet<String> {
    manifest
        .providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect()
}
/// The manifest declaration behind a `provider` registration, if the owner declares it.
pub(crate) fn declared_provider(
    owner: &str,
    id: &str,
    cx: &App,
) -> Option<exaudio_extension::ProviderManifest> {
    pending_manifest(owner, cx)?
        .providers
        .into_iter()
        .find(|provider| provider.id == id)
}
fn translate_status(value: w::ProviderStatus) -> crate::ProviderStatus {
    match value.state {
        w::ProviderState::Ready => crate::ProviderStatus::Ready {
            detail: value.detail,
        },
        w::ProviderState::NeedsSignIn => crate::ProviderStatus::NeedsSignIn,
        w::ProviderState::Error => crate::ProviderStatus::Error(value.detail),
    }
}
/// Status of an instance as reported by its WASM provider type. Answers are cached per
/// instance until setup or sign-out invalidates them. A cold check sends `provider-status`
/// and waits for `publish-provider-status`, bounded by a few guest call deadlines: this is
/// the one place the UI thread waits on a guest, because the shell needs the answer
/// synchronously while opening a mode, and the guest answers from local secrets.
pub(crate) fn provider_status(
    owner: &str,
    instance: &crate::ProviderInstance,
    cx: &App,
) -> crate::ProviderStatus {
    let Some(entry) = cx
        .try_global::<WasmExtensions>()
        .and_then(|state| state.entries.get(owner))
        .filter(|entry| !entry.host.removed.load(Ordering::Acquire))
    else {
        return crate::ProviderStatus::Error("The account extension is not active".into());
    };
    let guest = guest_instance(instance);
    if let Some(cached) = entry.host.cached_status(&guest) {
        return translate_status(cached);
    }
    let receiver = entry.host.await_status(&guest);
    let ticket = match entry
        .runtime
        .invoke(Invocation::Event(w::Event::ProviderStatus(guest.clone())))
    {
        Ok(ticket) => ticket,
        Err(error) => {
            entry.host.cancel_wait(&guest);
            return crate::ProviderStatus::Error(error.to_string());
        }
    };
    let deadline = Instant::now() + Limits::default().call_timeout * 4;
    let result = wait_for_status(&receiver, deadline, || match ticket.try_recv() {
        Ok(None) => GuestCall::Running,
        Ok(Some(Ok(()))) => GuestCall::Finished,
        Ok(Some(Err(error))) => GuestCall::Failed(error.to_string()),
        Err(_) => GuestCall::Gone,
    });
    entry.host.cancel_wait(&guest);
    match result {
        Ok(value) => translate_status(value),
        Err(failure) => {
            ticket.cancel();
            crate::ProviderStatus::Error(failure)
        }
    }
}
enum GuestCall {
    Running,
    Finished,
    Failed(String),
    Gone,
}
/// Waits for the guest's answer until `deadline`; `poll` reports the guest call so a
/// finished-but-silent or failed call ends the wait early instead of at the deadline.
fn wait_for_status(
    receiver: &mpsc::Receiver<w::ProviderStatus>,
    deadline: Instant,
    mut poll: impl FnMut() -> GuestCall,
) -> Result<w::ProviderStatus, String> {
    loop {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(5));
        match receiver.recv_timeout(wait) {
            Ok(value) => return Ok(value),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("The account extension was unloaded".into());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        match poll() {
            GuestCall::Running => {}
            GuestCall::Finished => {
                return receiver
                    .try_recv()
                    .map_err(|_| "The account extension did not report a status".to_string());
            }
            GuestCall::Failed(error) => return Err(error),
            GuestCall::Gone => return Err("The account extension was unloaded".into()),
        }
        if Instant::now() >= deadline {
            return Err("The account extension did not report a status in time".into());
        }
    }
}
/// Invalidates the cached status so the next check asks the guest again.
pub(crate) fn refresh_provider_status(owner: &str, instance: &crate::ProviderInstance, cx: &App) {
    if let Some(entry) = cx
        .try_global::<WasmExtensions>()
        .and_then(|state| state.entries.get(owner))
    {
        entry.host.forget_status(&guest_instance(instance));
    }
}
/// Sign-out: tells the guest, then deletes everything this extension stored for the
/// instance. The registry removes the whole instance directory afterwards.
pub(crate) fn sign_out_instance(owner: &str, instance: &crate::ProviderInstance, cx: &mut App) {
    invoke(
        owner,
        Invocation::Event(w::Event::ProviderSignOut(guest_instance(instance))),
        cx,
    );
    refresh_provider_status(owner, instance, cx);
    let _ = std::fs::remove_dir_all(instance_data_dir(owner, instance));
}

#[cfg(test)]
mod tests;

/// A guest document identifier names an open incarnation, not a persisted project.
/// Reopening the same project therefore cannot make an old revision-zero edit valid.
pub(crate) fn document_handle(model: &Entity<crate::editor::model::EditorModel>) -> String {
    format!("document:{:x}", model.entity_id().as_u64())
}
