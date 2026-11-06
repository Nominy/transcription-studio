//! Sandboxed, typed Exaudio component runtime. No WASI or ambient OS imports are linked.
//! See `HostServices` for the native document owner and I/O cancellation contract.
mod compiler;
#[doc(hidden)]
pub use compiler::worker_main;
mod host;
mod lift;
mod services;
pub use services::{CallContext, HostServices, Output};

pub mod bindings {
    wasmtime::component::bindgen!({ path: "../../wit", world: "extension" });
}
use anyhow::{Context, Result, bail};
pub use bindings::exaudio::extension::types;
use exaudio_extension::{ExtensionManifest, Permission, Runtime as ExtensionRuntime};
use host::State;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
use wasmtime::{Engine, Store, UpdateDeadline, component::Linker};

#[derive(Clone, Debug)]
pub struct Limits {
    pub fuel: u64,
    pub call_timeout: Duration,
    pub memory_bytes: usize,
    pub output_bytes: usize,
    pub component_bytes: usize,
    pub compile_timeout: Duration,
    pub compiler_memory_bytes: usize,
    pub artifact_bytes: usize,
    /// Trusted installed helper override; absolute path only. Never an extension path.
    pub compiler_path: Option<std::path::PathBuf>,
    pub registrations: usize,
    pub ui_depth: usize,
    pub wasm_stack_bytes: usize,
    pub queue: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            fuel: 2_000_000,
            call_timeout: Duration::from_millis(250),
            memory_bytes: 32 * 1024 * 1024,
            output_bytes: 1024 * 1024,
            component_bytes: 16 * 1024 * 1024,
            compile_timeout: Duration::from_secs(30),
            compiler_memory_bytes: 1024 * 1024 * 1024,
            artifact_bytes: 128 * 1024 * 1024,
            compiler_path: None,
            registrations: 256,
            ui_depth: 32,
            wasm_stack_bytes: 256 * 1024,
            queue: 32,
        }
    }
}

pub struct Package {
    pub manifest: ExtensionManifest,
    pub component: Vec<u8>,
    /// Explicit user grants, NEVER copied automatically from manifest.permissions.
    pub grants: BTreeSet<Permission>,
    /// Active documents' mode constraints. The owner must supply every requirement
    /// on this extension (including transitive ones) from the resolved open modes.
    pub required_by_open_modes: Vec<semver::VersionReq>,
}
pub enum Invocation {
    Event(types::Event),
    Command(types::Command),
    Ui(types::UiEvent),
}
enum Work {
    Load(Package),
    Invoke(Invocation),
    Unload,
}
struct Request {
    work: Work,
    cancellation: Arc<AtomicBool>,
    reply: SyncSender<Result<()>>,
}

/// Poll with `try_recv` from a UI task, or await on an application's worker executor.
/// `recv` is deliberately not exposed: UI callers must never block on guest work.
pub struct Ticket {
    receiver: Receiver<Result<()>>,
    cancellation: Arc<AtomicBool>,
}
impl Ticket {
    pub fn try_recv(&self) -> Result<Option<Result<()>>, mpsc::TryRecvError> {
        match self.receiver.try_recv() {
            Ok(value) => Ok(Some(value)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

/// One isolated worker per enabled extension. Compilation, instantiation and callbacks
/// all occur here, not on the UI thread. Dropping this handle requests cancellation;
/// it never joins the worker on the UI thread. A cooperative native I/O service must
/// finish within the current call deadline; pure WASM is interrupted within one tick.
pub struct Runtime {
    sender: Option<SyncSender<Request>>,
    stopping: Arc<AtomicBool>,
    active_cancel: Arc<std::sync::Mutex<Option<Arc<AtomicBool>>>>,
}
impl Runtime {
    pub fn spawn(
        extension_id: String,
        services: Arc<dyn HostServices>,
        limits: Limits,
    ) -> Result<Self> {
        anyhow::ensure!(
            exaudio_extension::manifest::valid_id(&extension_id),
            "invalid extension id"
        );
        anyhow::ensure!(
            limits.queue > 0
                && limits.fuel > 0
                && !limits.call_timeout.is_zero()
                && !limits.compile_timeout.is_zero()
                && limits.compiler_memory_bytes > 0
                && limits.artifact_bytes > 0
                && limits.memory_bytes > 0
                && limits.output_bytes > 0,
            "invalid runtime limits"
        );
        let (sender, receiver) = mpsc::sync_channel::<Request>(limits.queue);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let active_cancel = Arc::new(std::sync::Mutex::new(None::<Arc<AtomicBool>>));
        let active = active_cancel.clone();
        thread::Builder::new()
            .name(format!("extension-{extension_id}"))
            .spawn(move || {
                let engine = Engine::new(&compiler::engine_config(limits.wasm_stack_bytes));
                let engine = match engine {
                    Ok(engine) => engine,
                    Err(error) => {
                        stop.store(true, Ordering::Relaxed);
                        if let Ok(request) = receiver.try_recv() {
                            let _ = request
                                .reply
                                .send(Err(anyhow::anyhow!("engine initialization: {error}")));
                        }
                        services.emit(&extension_id, Output::Removed);
                        return;
                    }
                };
                let _ticker = Ticker::new(engine.clone());
                let mut instance: Option<Instance> = None;
                while let Ok(request) = receiver.recv() {
                    if stop.load(Ordering::Relaxed) {
                        let _ = request.reply.send(Err(anyhow::anyhow!("runtime stopped")));
                        break;
                    }
                    *active.lock().expect("cancellation mutex") =
                        Some(request.cancellation.clone());
                    let mut terminal = false;
                    let result = if request.cancellation.load(Ordering::Relaxed) {
                        Err(anyhow::anyhow!("cancelled before dispatch"))
                    } else {
                        match request.work {
                            Work::Load(package) => {
                                match Instance::load(
                                    &engine,
                                    &extension_id,
                                    services.clone(),
                                    &limits,
                                    package,
                                    request.cancellation,
                                ) {
                                    Ok(mut replacement) => {
                                        if let Some(mut old) = instance.take() {
                                            old.deactivate();
                                        }
                                        services.emit(&extension_id, Output::Activated);
                                        replacement.commit();
                                        instance = Some(replacement);
                                        Ok(())
                                    }
                                    Err(error) => {
                                        // A failed replacement keeps the prior guest. A failed
                                        // first load has nothing to keep and ends its generation.
                                        if instance.is_none() {
                                            services.emit(&extension_id, Output::Removed);
                                            terminal = true;
                                        }
                                        Err(error)
                                    }
                                }
                            }
                            Work::Invoke(invocation) => {
                                if let Some(current) = instance.as_mut() {
                                    current.prepare(request.cancellation);
                                    let result = current.invoke(invocation);
                                    if result.as_ref().is_err_and(|error| {
                                        error.downcast_ref::<GuestFailure>().is_none()
                                    }) {
                                        // Any VM/host trap invalidates this instance; never run it again.
                                        instance.take();
                                        services.emit(&extension_id, Output::Removed);
                                        terminal = true;
                                    }
                                    result
                                } else {
                                    Err(anyhow::anyhow!("extension is not active"))
                                }
                            }
                            Work::Unload => {
                                if let Some(mut old) = instance.take() {
                                    old.deactivate();
                                }
                                services.emit(&extension_id, Output::Removed);
                                terminal = true;
                                Ok(())
                            }
                        }
                    };
                    *active.lock().expect("cancellation mutex") = None;
                    if terminal {
                        stop.store(true, Ordering::Relaxed);
                    }
                    let _ = request.reply.send(result);
                    if terminal {
                        break;
                    }
                }
                if let Some(mut old) = instance {
                    old.deactivate();
                    services.emit(&extension_id, Output::Removed);
                }
                stop.store(true, Ordering::Relaxed);
            })?;
        Ok(Self {
            sender: Some(sender),
            stopping,
            active_cancel,
        })
    }
    fn submit(&self, work: Work) -> Result<Ticket> {
        anyhow::ensure!(
            !self.stopping.load(Ordering::Relaxed),
            "runtime generation stopped"
        );
        let (reply, receiver) = mpsc::sync_channel(1);
        let cancellation = Arc::new(AtomicBool::new(false));
        self.sender
            .as_ref()
            .context("runtime stopped")?
            .try_send(Request {
                work,
                cancellation: cancellation.clone(),
                reply,
            })
            .map_err(|e| anyhow::anyhow!("extension queue unavailable: {e}"))?;
        Ok(Ticket {
            receiver,
            cancellation,
        })
    }
    pub fn load(&self, package: Package) -> Result<Ticket> {
        self.submit(Work::Load(package))
    }
    pub fn invoke(&self, invocation: Invocation) -> Result<Ticket> {
        self.submit(Work::Invoke(invocation))
    }
    /// Caller must first ensure no open mode requires the extension. Reload can instead
    /// preserve it while validating replacement requirements in Package.
    /// This is terminal: worker/ticker exit, and re-enabling needs a new Runtime.
    pub fn unload(&self) -> Result<Ticket> {
        self.submit(Work::Unload)
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(cancel) = self
            .active_cancel
            .lock()
            .expect("cancellation mutex")
            .as_ref()
        {
            cancel.store(true, Ordering::Relaxed);
        }
        self.sender.take();
    }
}

#[derive(Debug)]
struct GuestFailure(String);
impl std::fmt::Display for GuestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for GuestFailure {}
fn guest_result(result: Result<Result<(), String>>) -> Result<()> {
    match result? {
        Ok(()) => Ok(()),
        Err(message) => {
            anyhow::ensure!(message.len() <= 4096, "guest error output limit exceeded");
            Err(GuestFailure(message).into())
        }
    }
}
struct Instance {
    store: Store<State>,
    guest: bindings::Extension,
}
impl Instance {
    fn load(
        engine: &Engine,
        id: &str,
        services: Arc<dyn HostServices>,
        limits: &Limits,
        package: Package,
        cancellation: Arc<AtomicBool>,
    ) -> Result<Self> {
        package.manifest.validate()?;
        package.manifest.check_host_compatibility()?;
        anyhow::ensure!(
            package.manifest.id == id && package.manifest.runtime == ExtensionRuntime::Wasm,
            "package identity/runtime mismatch"
        );
        anyhow::ensure!(
            package.component.len() <= limits.component_bytes,
            "component size limit exceeded"
        );
        anyhow::ensure!(
            package
                .grants
                .iter()
                .all(|grant| package.manifest.permissions.contains(grant)),
            "grant absent from manifest"
        );
        anyhow::ensure!(
            package
                .required_by_open_modes
                .iter()
                .all(|requirement| requirement.matches(&package.manifest.version)),
            "replacement violates an open mode dependency"
        );
        let component = compiler::compile(engine, &package.component, limits, &cancellation)?;
        let mut linker = Linker::new(engine);
        lift::add_to_linker(&mut linker)?;
        let providers = package
            .manifest
            .providers
            .iter()
            .map(|provider| provider.id.clone())
            .collect();
        let mut store = Store::new(
            engine,
            State::new(
                id.into(),
                services,
                package.grants,
                providers,
                limits.clone(),
                cancellation.clone(),
            ),
        );
        store.limiter(|state| &mut state.memory);
        store.epoch_deadline_callback(|cx| {
            if cx.data().cancellation.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            if Instant::now() >= cx.data().deadline {
                bail!("guest deadline exceeded");
            }
            Ok(UpdateDeadline::Continue(1))
        });
        prepare_store(&mut store, cancellation);
        let guest = bindings::Extension::instantiate(&mut store, &component, &linker)?;
        let mut instance = Self { store, guest };
        guest_result(
            instance
                .guest
                .exaudio_extension_guest()
                .call_activate(&mut instance.store),
        )?;
        let state = instance.store.data_mut();
        state.deadline = Instant::now() + state.limits.call_timeout;
        let cx = CallContext {
            extension_id: &state.id,
            deadline: state.deadline,
            cancellation: &state.cancellation,
        };
        state
            .services
            .preflight(&cx, &state.staged)
            .map_err(anyhow::Error::msg)?;
        cx.check().map_err(anyhow::Error::msg)?;
        Ok(instance)
    }
    fn prepare(&mut self, cancellation: Arc<AtomicBool>) {
        prepare_store(&mut self.store, cancellation);
    }
    fn commit(&mut self) {
        let state = self.store.data_mut();
        state.staging = false;
        for output in std::mem::take(&mut state.staged) {
            state.services.emit(&state.id, output);
        }
    }
    fn invoke(&mut self, invocation: Invocation) -> Result<()> {
        let guest = self.guest.exaudio_extension_guest();
        let result = match invocation {
            Invocation::Event(event) => guest.call_on_event(&mut self.store, &event),
            Invocation::Command(command) => {
                if self.store.data().registrations.get(&command.id)
                    != Some(&types::ContributionKind::Command)
                {
                    return Err(GuestFailure("unknown command".into()).into());
                }
                guest.call_on_command(&mut self.store, &command)
            }
            Invocation::Ui(event) => {
                if !self
                    .store
                    .data()
                    .ui_nodes
                    .get(&event.contribution)
                    .is_some_and(|nodes| nodes.contains(&event.node))
                {
                    return Err(
                        GuestFailure("unknown or disabled UI node/hit region".into()).into(),
                    );
                }
                if event.value.len() > self.store.data().limits.output_bytes {
                    return Err(GuestFailure("UI event value limit exceeded".into()).into());
                }
                guest.call_on_ui_event(&mut self.store, &event)
            }
        };
        guest_result(result)
    }
    fn deactivate(&mut self) {
        self.prepare(Arc::new(AtomicBool::new(false)));
        self.store.data_mut().suppress = true;
        let _ = self
            .guest
            .exaudio_extension_guest()
            .call_deactivate(&mut self.store);
    }
}
fn prepare_store(store: &mut Store<State>, cancellation: Arc<AtomicBool>) {
    let state = store.data_mut();
    state.cancellation = cancellation;
    state.deadline = Instant::now() + state.limits.call_timeout;
    state.remaining = state.limits.output_bytes;
    let fuel = state.limits.fuel;
    store.set_fuel(fuel).expect("fuel configured");
    store.set_epoch_deadline(1);
}
struct Ticker {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Ticker {
    fn new(engine: Engine) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                thread::park_timeout(Duration::from_millis(10));
                engine.increment_epoch();
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Ticker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests;
