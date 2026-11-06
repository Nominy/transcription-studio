//! WebView2 host for Gold's inference page.
//!
//! One dedicated STA thread owns a hidden top-level window, a WebView2 environment with browser
//! extensions enabled (isolated user-data folder), Gold's unpacked host build added via
//! `AddBrowserExtension`, and the `chrome-extension://<id>/inference.html` page. Callers talk to
//! it through [`PageHost`]: commands go over a channel and wake the thread's message loop with
//! `PostThreadMessageW`; CDP completions reply over oneshot channels; page events arrive through
//! the `Runtime.bindingCalled` CDP event and are routed to per-request sinks.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

use serde_json::{Value, json};
use tokio::sync::oneshot;
use webview2_com::{
    BrowserExtensionRemoveCompletedHandler, CallDevToolsProtocolMethodCompletedHandler,
    CoTaskMemPWSTR, CoreWebView2EnvironmentOptions, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler, ProcessFailedEventHandler,
    ProfileAddBrowserExtensionCompletedHandler, ProfileGetBrowserExtensionsCompletedHandler,
};
use windows::{
    Win32::{
        Foundation::{E_POINTER, HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            LibraryLoader::GetModuleHandleW,
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
            PostQuitMessage, PostThreadMessageW, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE,
            WM_APP, WNDCLASSW, WS_OVERLAPPEDWINDOW,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

use crate::{
    AUDIO_HOST,
    rpc::{self, EMIT_BINDING, FrameError, PageError},
};

/// Chromium switches that keep a never-shown host page running at full speed.
const BROWSER_ARGUMENTS: &str = "--disable-background-timer-throttling \
     --disable-renderer-backgrounding --disable-backgrounding-occluded-windows";

/// How long the page may take to expose `babelInference` after navigation completes.
const PAGE_READY_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct HostConfig {
    /// Gold's unpacked host build (folder holding `manifest.json` and `inference.html`).
    pub extension_dir: PathBuf,
    /// WebView2 user-data folder; the model cache lives in the extension's storage here.
    pub user_data_dir: PathBuf,
    /// Folder served at `https://`[`AUDIO_HOST`](crate::AUDIO_HOST)`/`.
    pub audio_dir: PathBuf,
}

/// Facts gathered while starting the host.
#[derive(Debug, Clone)]
pub struct PageInfo {
    pub runtime_version: String,
    pub extension_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HostError {
    #[error(
        "Running AI on this computer needs Microsoft Edge WebView2. Install it from \
         https://go.microsoft.com/fwlink/p/?LinkId=2124703 and try again."
    )]
    RuntimeMissing,
    #[error("The AI engine isn't installed. ({0})")]
    ExtensionMissing(String),
    #[error("The AI engine couldn't start. (WebView2 failed while {context}: {message})")]
    WebView {
        context: &'static str,
        message: String,
    },
    #[error("The AI engine stopped. ({0})")]
    Crashed(String),
    #[error("The AI engine is shut down.")]
    Stopped,
    #[error("{}", .0.message)]
    Page(PageError),
    #[error("The AI engine sent an unreadable reply. ({0})")]
    Malformed(String),
}

impl From<FrameError> for HostError {
    fn from(error: FrameError) -> Self {
        match error {
            FrameError::Page(error) => Self::Page(error),
            FrameError::Malformed(message) => Self::Malformed(message),
        }
    }
}

fn webview_error(context: &'static str) -> impl FnOnce(windows::core::Error) -> HostError {
    move |error| HostError::WebView {
        context,
        message: error.message(),
    }
}

fn webview2_error(context: &'static str) -> impl FnOnce(webview2_com::Error) -> HostError {
    move |error| HostError::WebView {
        context,
        message: match error {
            webview2_com::Error::WindowsError(error) => error.message(),
            other => format!("{other:?}"),
        },
    }
}

/// Receives the events the page emits for one request; runs on the host thread, must not block.
pub type EventSink = Arc<dyn Fn(Value) + Send + Sync>;

struct Shared {
    alive: AtomicBool,
    failure: Mutex<Option<String>>,
    sinks: Mutex<HashMap<String, EventSink>>,
}

impl Shared {
    fn fail(&self, reason: String) {
        self.alive.store(false, Ordering::SeqCst);
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        failure.get_or_insert(reason);
    }

    fn failure(&self) -> HostError {
        match self
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            Some(reason) => HostError::Crashed(reason),
            None => HostError::Stopped,
        }
    }
}

type CdpReply = oneshot::Sender<Result<String, HostError>>;

enum Command {
    Cdp {
        method: &'static str,
        params: String,
        reply: CdpReply,
    },
    Shutdown,
}

/// Handle to the running host thread. Dropping it shuts the page down and joins the thread.
pub struct PageHost {
    commands: mpsc::Sender<Command>,
    thread_id: u32,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
    info: PageInfo,
}

impl PageHost {
    /// Starts the host thread and blocks until the page is loaded and exposes host API v1.
    pub fn start(config: HostConfig) -> Result<Self, HostError> {
        if !config.extension_dir.join("inference.html").is_file() {
            return Err(HostError::ExtensionMissing(
                config.extension_dir.display().to_string(),
            ));
        }
        let shared = Arc::new(Shared {
            alive: AtomicBool::new(true),
            failure: Mutex::new(None),
            sinks: Mutex::new(HashMap::new()),
        });
        let (commands, command_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("studio-inference-webview".to_string())
            .spawn(move || host_thread(config, thread_shared, command_rx, ready_tx))
            .map_err(|error| HostError::Crashed(format!("could not spawn host thread: {error}")))?;
        match ready_rx.recv() {
            Ok(Ok((thread_id, info))) => Ok(Self {
                commands,
                thread_id,
                shared,
                thread: Some(thread),
                info,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(HostError::Crashed(
                    "host thread exited during start".to_string(),
                ))
            }
        }
    }

    pub fn info(&self) -> &PageInfo {
        &self.info
    }

    /// False once the browser or page process died or the host was shut down.
    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// Calls `babelInference[method](...args, emit)` and returns the envelope's `result`.
    /// `sink` receives every event emitted under `request_id` until the call settles.
    pub async fn call(
        &self,
        request_id: &str,
        method: &str,
        args: &[Value],
        sink: Option<EventSink>,
    ) -> Result<Value, HostError> {
        let registration = sink.map(|sink| SinkRegistration::new(&self.shared, request_id, sink));
        let raw = self
            .cdp(
                "Runtime.evaluate",
                evaluate_params(&rpc::call_expression(request_id, method, args)),
            )
            .await?;
        drop(registration);
        Ok(rpc::parse_call_response(&raw)?)
    }

    /// Asks the page to abort request `request_id` (`babelInference.cancel`); fire-and-forget.
    pub fn cancel(&self, request_id: &str) {
        let expression = format!(
            "globalThis.babelInference?.cancel?.({})",
            Value::from(request_id)
        );
        let (reply, _) = oneshot::channel();
        let _ = self.send(Command::Cdp {
            method: "Runtime.evaluate",
            params: evaluate_params(&expression).to_string(),
            reply,
        });
    }

    /// Evaluates a plain expression (awaiting promises) and returns its by-value result.
    pub async fn evaluate(&self, expression: &str) -> Result<Value, HostError> {
        let raw = self
            .cdp("Runtime.evaluate", evaluate_params(expression))
            .await?;
        Ok(rpc::parse_evaluate_value(&raw)?)
    }

    async fn cdp(&self, method: &'static str, params: Value) -> Result<String, HostError> {
        if !self.is_alive() {
            return Err(self.shared.failure());
        }
        let (reply, response) = oneshot::channel();
        self.send(Command::Cdp {
            method,
            params: params.to_string(),
            reply,
        })?;
        match response.await {
            Ok(result) => result,
            Err(_) => Err(self.shared.failure()),
        }
    }

    fn send(&self, command: Command) -> Result<(), HostError> {
        self.commands
            .send(command)
            .map_err(|_| self.shared.failure())?;
        // SAFETY: plain Win32 call; WM_APP only wakes the host thread's message loop.
        unsafe { PostThreadMessageW(self.thread_id, WM_APP, WPARAM(0), LPARAM(0)) }
            .map_err(|_| self.shared.failure())
    }
}

impl Drop for PageHost {
    fn drop(&mut self) {
        if self.send(Command::Shutdown).is_ok()
            && let Some(thread) = self.thread.take()
        {
            let _ = thread.join();
        }
    }
}

fn evaluate_params(expression: &str) -> Value {
    json!({ "expression": expression, "awaitPromise": true, "returnByValue": true })
}

/// Routes events for one request while the call is in flight.
struct SinkRegistration<'a> {
    shared: &'a Shared,
    request_id: String,
}

impl<'a> SinkRegistration<'a> {
    fn new(shared: &'a Shared, request_id: &str, sink: EventSink) -> Self {
        shared
            .sinks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request_id.to_string(), sink);
        Self {
            shared,
            request_id: request_id.to_string(),
        }
    }
}

impl Drop for SinkRegistration<'_> {
    fn drop(&mut self) {
        self.shared
            .sinks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.request_id);
    }
}

// ---------------------------------------------------------------------------------------------
// Host thread
// ---------------------------------------------------------------------------------------------

struct Page {
    window: HWND,
    controller: ICoreWebView2Controller,
    webview: ICoreWebView2,
}

type Pending = Rc<RefCell<HashMap<u64, CdpReply>>>;

fn host_thread(
    config: HostConfig,
    shared: Arc<Shared>,
    commands: mpsc::Receiver<Command>,
    ready: mpsc::Sender<Result<(u32, PageInfo), HostError>>,
) {
    // SAFETY: initializes COM for this thread only; balanced by CoUninitialize below.
    if let Err(error) = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.ok() {
        let _ = ready.send(Err(webview_error("initializing COM")(error)));
        return;
    }
    let pending: Pending = Rc::new(RefCell::new(HashMap::new()));
    match open_page(&config, &shared, &pending) {
        Ok((page, info)) => {
            // SAFETY: plain Win32 call.
            let thread_id = unsafe { GetCurrentThreadId() };
            if ready.send(Ok((thread_id, info))).is_ok() {
                run_message_loop(&page, &shared, &commands, &pending);
            }
            close_page(page);
        }
        Err(error) => {
            let _ = ready.send(Err(error));
        }
    }
    shared.fail("the inference host shut down".to_string());
    for (_, reply) in pending.borrow_mut().drain() {
        let _ = reply.send(Err(shared.failure()));
    }
    // SAFETY: balances the successful CoInitializeEx above.
    unsafe { CoUninitialize() };
}

fn run_message_loop(
    page: &Page,
    shared: &Arc<Shared>,
    commands: &mpsc::Receiver<Command>,
    pending: &Pending,
) {
    let next_id = AtomicU64::new(0);
    let mut message = MSG::default();
    loop {
        // SAFETY: standard Win32 message pump on the thread that owns the window.
        let status = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if status <= 0 {
            return;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Shutdown => return,
                Command::Cdp {
                    method,
                    params,
                    reply,
                } => {
                    if !shared.alive.load(Ordering::SeqCst) {
                        let _ = reply.send(Err(shared.failure()));
                        continue;
                    }
                    let id = next_id.fetch_add(1, Ordering::Relaxed);
                    pending.borrow_mut().insert(id, reply);
                    if let Err(error) = call_cdp(&page.webview, method, &params, id, pending)
                        && let Some(reply) = pending.borrow_mut().remove(&id)
                    {
                        let _ = reply.send(Err(error));
                    }
                }
            }
        }
        if !shared.alive.load(Ordering::SeqCst) {
            return;
        }
    }
}

fn call_cdp(
    webview: &ICoreWebView2,
    method: &str,
    params: &str,
    id: u64,
    pending: &Pending,
) -> Result<(), HostError> {
    let pending = pending.clone();
    let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(
        move |result: windows::core::Result<()>, json: String| {
            if let Some(reply) = pending.borrow_mut().remove(&id) {
                let _ = reply.send(
                    result
                        .map(|()| json)
                        .map_err(webview_error("calling the DevTools protocol")),
                );
            }
            Ok(())
        },
    ));
    let method = CoTaskMemPWSTR::from(method);
    let params = CoTaskMemPWSTR::from(params);
    // SAFETY: COM call on the owning STA thread with valid, NUL-terminated strings.
    unsafe {
        webview.CallDevToolsProtocolMethod(
            *method.as_ref().as_pcwstr(),
            *params.as_ref().as_pcwstr(),
            &handler,
        )
    }
    .map_err(webview_error("calling the DevTools protocol"))
}

/// Blocking CDP call used during start-up (pumps messages until the reply arrives).
fn call_cdp_blocking(
    webview: &ICoreWebView2,
    method: &str,
    params: Value,
) -> Result<String, HostError> {
    let (tx, rx) = mpsc::channel();
    let webview = webview.clone();
    let method = method.to_string();
    CallDevToolsProtocolMethodCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            let method = CoTaskMemPWSTR::from(method.as_str());
            let params = CoTaskMemPWSTR::from(params.to_string().as_str());
            // SAFETY: COM call on the owning STA thread.
            unsafe {
                webview.CallDevToolsProtocolMethod(
                    *method.as_ref().as_pcwstr(),
                    *params.as_ref().as_pcwstr(),
                    &handler,
                )
            }
            .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, json| {
            let _ = tx.send(result.map(|()| json));
            Ok(())
        }),
    )
    .map_err(webview2_error("calling the DevTools protocol"))?;
    rx.recv()
        .map_err(|_| HostError::Crashed("DevTools reply was lost".to_string()))?
        .map_err(webview_error("calling the DevTools protocol"))
}

fn open_page(
    config: &HostConfig,
    shared: &Arc<Shared>,
    pending: &Pending,
) -> Result<(Page, PageInfo), HostError> {
    let runtime_version = runtime_version()?;
    std::fs::create_dir_all(&config.user_data_dir).map_err(|error| HostError::WebView {
        context: "creating the WebView2 profile folder",
        message: error.to_string(),
    })?;
    std::fs::create_dir_all(&config.audio_dir).map_err(|error| HostError::WebView {
        context: "creating the audio staging folder",
        message: error.to_string(),
    })?;
    let window = create_hidden_window()?;
    let page = match create_webview(window, &config.user_data_dir) {
        Ok((controller, webview)) => Page {
            window,
            controller,
            webview,
        },
        Err(error) => {
            // SAFETY: the window was created on this thread.
            unsafe {
                let _ = DestroyWindow(window);
            }
            return Err(error);
        }
    };
    match prepare_page(&page, config, shared, pending) {
        Ok(extension_id) => Ok((
            page,
            PageInfo {
                runtime_version,
                extension_id,
            },
        )),
        Err(error) => {
            close_page(page);
            Err(error)
        }
    }
}

fn prepare_page(
    page: &Page,
    config: &HostConfig,
    shared: &Arc<Shared>,
    pending: &Pending,
) -> Result<String, HostError> {
    let webview = &page.webview;
    watch_process_failures(webview, shared, pending)?;
    map_audio_folder(webview, &config.audio_dir)?;
    let extension_id = install_extension(webview, &config.extension_dir)?;
    // The binding and its event subscription persist across navigations of this webview.
    route_emitted_events(webview, shared)?;
    call_cdp_blocking(webview, "Runtime.enable", json!({}))?;
    call_cdp_blocking(
        webview,
        "Runtime.addBinding",
        json!({ "name": EMIT_BINDING }),
    )?;
    navigate(
        webview,
        &format!("chrome-extension://{extension_id}/inference.html"),
    )?;
    wait_for_host_api(webview)?;
    Ok(extension_id)
}

fn close_page(page: Page) {
    // SAFETY: COM/Win32 calls on the owning thread; errors during teardown are irrelevant.
    unsafe {
        let _ = page.controller.Close();
        let _ = DestroyWindow(page.window);
    }
    drop(page);
    // Let WebView2 deliver its teardown messages before COM goes away.
    // SAFETY: posts WM_QUIT to this thread's own queue.
    unsafe { PostQuitMessage(0) };
    let mut message = MSG::default();
    // SAFETY: drains this thread's queue until the WM_QUIT posted above.
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn runtime_version() -> Result<String, HostError> {
    let mut version = PWSTR::null();
    // SAFETY: the loader allocates `version` with CoTaskMemAlloc; CoTaskMemPWSTR frees it.
    let result =
        unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version) };
    let version = CoTaskMemPWSTR::from(version);
    match result {
        Ok(()) if !version.as_ref().as_pcwstr().is_null() => Ok(version.to_string()),
        _ => Err(HostError::RuntimeMissing),
    }
}

extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: default handling for the host's own hidden window.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn create_hidden_window() -> Result<HWND, HostError> {
    let class_name = w!("StudioInferenceHost");
    // SAFETY: registers/creates a window owned by this thread; the class is registered once
    // per process (a repeated RegisterClassW after a restart fails harmlessly).
    unsafe {
        let instance = GetModuleHandleW(None).map_err(webview_error("creating the host window"))?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("Exaudio inference"),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            1024,
            768,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .map_err(webview_error("creating the host window"))
    }
}

fn create_webview(
    window: HWND,
    user_data_dir: &Path,
) -> Result<(ICoreWebView2Controller, ICoreWebView2), HostError> {
    let options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: setters on our own options object.
    unsafe {
        options.set_are_browser_extensions_enabled(true);
        options.set_additional_browser_arguments(BROWSER_ARGUMENTS.to_string());
    }
    let options: ICoreWebView2EnvironmentOptions = options.into();
    let user_data_dir = CoTaskMemPWSTR::from(user_data_dir.to_string_lossy().as_ref());

    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2EnvironmentCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: loader call with valid strings/options on the STA thread.
            unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    PCWSTR::null(),
                    *user_data_dir.as_ref().as_pcwstr(),
                    &options,
                    &handler,
                )
            }
            .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, environment| {
            let _ =
                tx.send(result.and_then(|()| {
                    environment.ok_or_else(|| windows::core::Error::from(E_POINTER))
                }));
            Ok(())
        }),
    )
    .map_err(webview2_error("creating the WebView2 environment"))?;
    let environment = rx
        .recv()
        .map_err(|_| HostError::Crashed("environment reply was lost".to_string()))?
        .map_err(webview_error("creating the WebView2 environment"))?;

    let (tx, rx) = mpsc::channel();
    CreateCoreWebView2ControllerCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: COM call on the STA thread with the window it owns.
            unsafe { environment.CreateCoreWebView2Controller(window, &handler) }
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, controller| {
            let _ = tx
                .send(result.and_then(|()| {
                    controller.ok_or_else(|| windows::core::Error::from(E_POINTER))
                }));
            Ok(())
        }),
    )
    .map_err(webview2_error("creating the WebView2 controller"))?;
    let controller = rx
        .recv()
        .map_err(|_| HostError::Crashed("controller reply was lost".to_string()))?
        .map_err(webview_error("creating the WebView2 controller"))?;

    // SAFETY: COM calls on the owning thread.
    unsafe {
        controller
            .SetBounds(RECT {
                left: 0,
                top: 0,
                right: 1024,
                bottom: 768,
            })
            .map_err(webview_error("sizing the WebView2 controller"))?;
        controller
            .SetIsVisible(true)
            .map_err(webview_error("showing the WebView2 controller"))?;
        let webview = controller
            .CoreWebView2()
            .map_err(webview_error("opening the WebView2 page"))?;
        Ok((controller, webview))
    }
}

fn watch_process_failures(
    webview: &ICoreWebView2,
    shared: &Arc<Shared>,
    pending: &Pending,
) -> Result<(), HostError> {
    let shared = shared.clone();
    let pending = pending.clone();
    let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
        // SAFETY: reads a property of the event args on the owning thread.
        unsafe { args.ProcessFailedKind(&mut kind)? };
        let reason = match kind {
            COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED => "the browser process exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED => "the page process exited",
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE => {
                eprintln!("local inference page is unresponsive");
                return Ok(());
            }
            COREWEBVIEW2_PROCESS_FAILED_KIND_GPU_PROCESS_EXITED => {
                // Chromium restarts the GPU process; in-flight WebGPU work fails in the page.
                eprintln!("local inference GPU process exited");
                return Ok(());
            }
            _ => return Ok(()),
        };
        shared.fail(reason.to_string());
        for (_, reply) in pending.borrow_mut().drain() {
            let _ = reply.send(Err(HostError::Crashed(reason.to_string())));
        }
        // Wake the message loop so it observes the failure and tears the page down.
        // SAFETY: posts to this thread's own queue.
        unsafe {
            let _ = PostThreadMessageW(GetCurrentThreadId(), WM_APP, WPARAM(0), LPARAM(0));
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: COM call on the owning thread.
    unsafe { webview.add_ProcessFailed(&handler, &mut token) }
        .map_err(webview_error("watching WebView2 processes"))
}

fn map_audio_folder(webview: &ICoreWebView2, audio_dir: &Path) -> Result<(), HostError> {
    let webview: ICoreWebView2_3 = windows::core::Interface::cast(webview)
        .map_err(webview_error("mapping the audio folder"))?;
    let host = CoTaskMemPWSTR::from(AUDIO_HOST);
    let folder = CoTaskMemPWSTR::from(audio_dir.to_string_lossy().as_ref());
    // SAFETY: COM call on the owning thread with valid strings.
    unsafe {
        webview.SetVirtualHostNameToFolderMapping(
            *host.as_ref().as_pcwstr(),
            *folder.as_ref().as_pcwstr(),
            COREWEBVIEW2_HOST_RESOURCE_ACCESS_KIND_ALLOW,
        )
    }
    .map_err(webview_error("mapping the audio folder"))
}

/// Adds (or refreshes) Gold's unpacked build and removes stale copies of it. Returns the
/// extension id.
fn install_extension(webview: &ICoreWebView2, extension_dir: &Path) -> Result<String, HostError> {
    let profile = profile(webview)?;
    let folder = extension_dir.to_string_lossy().to_string();
    let (tx, rx) = mpsc::channel();
    let add_profile = profile.clone();
    ProfileAddBrowserExtensionCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            let folder = CoTaskMemPWSTR::from(folder.as_str());
            // SAFETY: COM call on the owning thread.
            unsafe { add_profile.AddBrowserExtension(*folder.as_ref().as_pcwstr(), &handler) }
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, extension| {
            let _ = tx.send(
                result
                    .and_then(|()| extension.ok_or_else(|| windows::core::Error::from(E_POINTER))),
            );
            Ok(())
        }),
    )
    .map_err(webview2_error("adding Gold's extension"))?;
    let extension = rx
        .recv()
        .map_err(|_| HostError::Crashed("extension reply was lost".to_string()))?
        .map_err(webview_error("adding Gold's extension"))?;
    let id = extension_id(&extension)?;
    let name = extension_name(&extension)?;
    // Copies of Gold loaded from an earlier install path; WebView2's built-in extensions stay.
    for other in installed_extensions(&profile)? {
        if extension_id(&other)? != id && extension_name(&other)? == name {
            remove_extension(other)?;
        }
    }
    Ok(id)
}

fn profile(webview: &ICoreWebView2) -> Result<ICoreWebView2Profile7, HostError> {
    let webview: ICoreWebView2_13 = windows::core::Interface::cast(webview).map_err(
        webview_error("opening the WebView2 profile (runtime too old)"),
    )?;
    // SAFETY: COM calls on the owning thread.
    let profile =
        unsafe { webview.Profile() }.map_err(webview_error("opening the WebView2 profile"))?;
    windows::core::Interface::cast(&profile).map_err(webview_error(
        "opening the WebView2 profile (runtime too old for extensions)",
    ))
}

fn extension_id(extension: &ICoreWebView2BrowserExtension) -> Result<String, HostError> {
    let mut id = PWSTR::null();
    // SAFETY: WebView2 allocates `id` with CoTaskMemAlloc; CoTaskMemPWSTR frees it.
    unsafe { extension.Id(&mut id) }.map_err(webview_error("reading the extension id"))?;
    Ok(CoTaskMemPWSTR::from(id).to_string())
}

fn extension_name(extension: &ICoreWebView2BrowserExtension) -> Result<String, HostError> {
    let mut name = PWSTR::null();
    // SAFETY: WebView2 allocates `name` with CoTaskMemAlloc; CoTaskMemPWSTR frees it.
    unsafe { extension.Name(&mut name) }.map_err(webview_error("reading the extension name"))?;
    Ok(CoTaskMemPWSTR::from(name).to_string())
}

fn installed_extensions(
    profile: &ICoreWebView2Profile7,
) -> Result<Vec<ICoreWebView2BrowserExtension>, HostError> {
    let (tx, rx) = mpsc::channel();
    let profile = profile.clone();
    ProfileGetBrowserExtensionsCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: COM call on the owning thread.
            unsafe { profile.GetBrowserExtensions(&handler) }
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result, list| {
            let _ = tx.send(
                result.and_then(|()| list.ok_or_else(|| windows::core::Error::from(E_POINTER))),
            );
            Ok(())
        }),
    )
    .map_err(webview2_error("listing extensions"))?;
    let list = rx
        .recv()
        .map_err(|_| HostError::Crashed("extension list reply was lost".to_string()))?
        .map_err(webview_error("listing extensions"))?;
    let mut count = 0;
    // SAFETY: COM calls on the owning thread.
    unsafe {
        list.Count(&mut count)
            .map_err(webview_error("listing extensions"))?;
        (0..count)
            .map(|index| {
                list.GetValueAtIndex(index)
                    .map_err(webview_error("listing extensions"))
            })
            .collect()
    }
}

fn remove_extension(extension: ICoreWebView2BrowserExtension) -> Result<(), HostError> {
    let (tx, rx) = mpsc::channel();
    BrowserExtensionRemoveCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| {
            // SAFETY: COM call on the owning thread.
            unsafe { extension.Remove(&handler) }.map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |result| {
            let _ = tx.send(result);
            Ok(())
        }),
    )
    .map_err(webview2_error("removing a stale extension"))?;
    rx.recv()
        .map_err(|_| HostError::Crashed("extension removal reply was lost".to_string()))?
        .map_err(webview_error("removing a stale extension"))
}

fn route_emitted_events(webview: &ICoreWebView2, shared: &Arc<Shared>) -> Result<(), HostError> {
    let event = CoTaskMemPWSTR::from("Runtime.bindingCalled");
    // SAFETY: COM call on the owning thread.
    let receiver = unsafe { webview.GetDevToolsProtocolEventReceiver(*event.as_ref().as_pcwstr()) }
        .map_err(webview_error("subscribing to page events"))?;
    let shared = shared.clone();
    let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut json = PWSTR::null();
        // SAFETY: WebView2 allocates `json`; CoTaskMemPWSTR frees it.
        unsafe { args.ParameterObjectAsJson(&mut json)? };
        let json = CoTaskMemPWSTR::from(json).to_string();
        if let Some(emitted) = rpc::parse_binding_called(&json) {
            let sink = shared
                .sinks
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&emitted.request_id)
                .cloned();
            if let Some(sink) = sink {
                sink(emitted.event);
            }
        }
        Ok(())
    }));
    let mut token = 0;
    // SAFETY: COM call on the owning thread.
    unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token) }
        .map_err(webview_error("subscribing to page events"))
}

fn navigate(webview: &ICoreWebView2, url: &str) -> Result<(), HostError> {
    let (tx, rx) = mpsc::channel();
    let handler = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
        let mut success = Default::default();
        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS::default();
        if let Some(args) = args {
            // SAFETY: reads properties of the event args on the owning thread.
            unsafe {
                args.IsSuccess(&mut success)?;
                args.WebErrorStatus(&mut status)?;
            }
        }
        let _ = tx.send((success.as_bool(), status.0));
        Ok(())
    }));
    let mut token = Default::default();
    let target = CoTaskMemPWSTR::from(url);
    // SAFETY: COM calls on the owning thread.
    unsafe {
        webview
            .add_NavigationCompleted(&handler, &mut token)
            .map_err(webview_error("loading the inference page"))?;
        webview
            .Navigate(*target.as_ref().as_pcwstr())
            .map_err(webview_error("loading the inference page"))?;
    }
    let outcome = webview2_com::wait_with_pump(rx);
    // SAFETY: COM call on the owning thread.
    unsafe {
        let _ = webview.remove_NavigationCompleted(token);
    }
    match outcome.map_err(webview2_error("loading the inference page"))? {
        (true, _) => Ok(()),
        (false, status) => Err(HostError::WebView {
            context: "loading the inference page",
            message: format!("navigation to {url} failed (web error status {status})"),
        }),
    }
}

fn wait_for_host_api(webview: &ICoreWebView2) -> Result<(), HostError> {
    let expression = format!(
        "new Promise(resolve => {{ const started = Date.now(); \
         const check = () => {{ const version = globalThis.babelInference?.hostApiVersion; \
         if (version !== undefined || Date.now() - started > {PAGE_READY_TIMEOUT_MS}) resolve(version ?? null); \
         else setTimeout(check, 50); }}; check(); }})"
    );
    let raw = call_cdp_blocking(webview, "Runtime.evaluate", evaluate_params(&expression))?;
    match rpc::parse_evaluate_value(&raw)? {
        Value::Number(version) if version.as_u64() == Some(rpc::HOST_API_VERSION) => Ok(()),
        Value::Null => Err(HostError::Malformed(format!(
            "the page did not expose babelInference within {} s",
            PAGE_READY_TIMEOUT_MS / 1000
        ))),
        other => Err(HostError::Malformed(format!(
            "the bundled Gold build speaks host API {other}, Studio needs {}",
            rpc::HOST_API_VERSION
        ))),
    }
}
