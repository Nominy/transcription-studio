use std::{collections::BTreeMap, path::PathBuf, rc::Rc};

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt as _};
use gpui_kit::*;
use sonus_babel_domain::route::StartupRoute;
use sonus_babel_services::auth::flow::{AuthFlow, AuthStartup};
use sonus_babel_services::babel::{BabelClient, BabelConfig, LoadedTask};
use sonus_babel_services::subaccounts::{self, SubaccountSession};
use sonus_shell::editor::model::EditorModel;
use sonus_shell::{DocumentOrigin, ProviderEnvironment, ShellHandle};
use studio_editor::workflow::StudioWorkflow;

use crate::screens::error_task::{ErrorTaskEvent, ErrorTaskOpen, ErrorTaskScreen};
use crate::screens::relay::RelayEvent;
use crate::screens::{ShellModels, auth::AuthScreen, dashboard::DashboardScreen};
use crate::services::{BabelServices, set_document_client};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BabelMode {
    Direct,
    Relay,
}

impl BabelMode {
    pub fn id(self) -> &'static str {
        match self {
            Self::Direct => "sonus.babel",
            Self::Relay => "sonus.babel-relay",
        }
    }
}

#[derive(Default)]
struct Controllers(BTreeMap<BabelMode, Entity<BabelController>>);
impl Global for Controllers {}

pub fn controller(mode: BabelMode, cx: &App) -> Option<Entity<BabelController>> {
    cx.try_global::<Controllers>()?.0.get(&mode).cloned()
}

/// Reloads a remote task using the client captured when this document was opened.
pub fn reload_task(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    use sonus_babel_services::babel::{
        OpenedTask, TaskRequest, review_action_id, route_metadata_from_project,
    };
    let mode = match model.read(cx).origin().mode_id.as_str() {
        "sonus.babel" => BabelMode::Direct,
        "sonus.babel-relay" => BabelMode::Relay,
        _ => return,
    };
    let project = model.read(cx).doc().project();
    let review_action_id = review_action_id(project);
    let metadata = route_metadata_from_project(project);
    if review_action_id.is_empty() {
        window.push_notification(
            Notification::error("This task has no review action to reload."),
            cx,
        );
        return;
    }
    let Some(controller) = controller(mode, cx) else {
        window.push_notification(
            Notification::error("The task's Babel workspace is no longer active."),
            cx,
        );
        return;
    };
    let client = crate::services::document_client(model, cx);
    controller.update(cx, |controller, cx| {
        let request = TaskRequest::ReviewAction { review_action_id };
        let transport = client.clone();
        controller.reload = Some(sonus_shell::services::spawn_service_in(
            window,
            cx,
            async move { transport.open_task(&request, metadata).await },
            move |controller, result, window, cx| {
                controller.reload = None;
                match result {
                    Ok(OpenedTask::Transcript(task)) => {
                        controller.client = Some(client);
                        controller.open_loaded_task(
                            *task,
                            "Task reloaded.",
                            "Task reload",
                            window,
                            cx,
                        );
                    }
                    Ok(OpenedTask::ErrorTask(_)) => window.push_notification(
                        Notification::error(
                            "The review action no longer contains a transcript task.",
                        ),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        Notification::error(format!("Task reload failed: {error}")),
                        cx,
                    ),
                }
            },
        ));
    });
}

/// Mode homes keep their dashboard alive while the shell displays an editor.
pub fn build_home(
    shell: ShellHandle,
    mode: BabelMode,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    if !cx.has_global::<Controllers>() {
        cx.set_global(Controllers::default());
    }
    if let Some(controller) = cx.global::<Controllers>().0.get(&mode).cloned() {
        let home = controller.clone();
        window.defer(cx, move |window, cx| {
            home.update(cx, |controller, cx| controller.restore_home(window, cx));
        });
        return controller.into();
    }
    let controller = cx.new(|cx| BabelController::new(shell, mode, window, cx));
    cx.global_mut::<Controllers>()
        .0
        .insert(mode, controller.clone());
    let weak = controller.downgrade();
    window.defer(cx, move |window, cx| {
        weak.update(cx, |controller, cx| controller.open_auth_gate(window, cx))
            .ok();
    });
    controller.into()
}

pub fn shutdown_mode(mode: BabelMode, cx: &mut App) {
    if !cx.has_global::<Controllers>() {
        return;
    }
    let controller = cx.global_mut::<Controllers>().0.remove(&mode);
    if let Some(controller) = controller {
        controller.update(cx, |controller, cx| controller.shutdown_requests(cx));
    }
}

pub fn shutdown(cx: &mut App) {
    if cx.has_global::<Controllers>() {
        let controllers = std::mem::take(&mut cx.global_mut::<Controllers>().0);
        for controller in controllers.into_values() {
            controller.update(cx, |controller, cx| controller.shutdown_requests(cx));
        }
    }
    if cx.has_global::<ShellModels>() {
        ShellModels::relay(cx).update(cx, |relay, _| relay.shutdown());
    }
}

pub struct BabelController {
    shell: ShellHandle,
    mode: BabelMode,
    client: Option<BabelClient>,
    view: AnyView,
    auth: Option<Entity<AuthScreen>>,
    dashboard: Option<Entity<DashboardScreen>>,
    route: Option<StartupRoute>,
    error_task: Option<Entity<ErrorTaskScreen>>,
    error_events: Option<Subscription>,
    audio_guard: Option<Subscription>,
    reload: Option<Task<()>>,
    relay_events: Option<Subscription>,
}

impl BabelController {
    pub fn new(
        shell: ShellHandle,
        mode: BabelMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let relay = ShellModels::relay(cx);
        let relay_events = cx.subscribe_in(&relay, window, |this, _, event, window, cx| {
            if this.mode != BabelMode::Direct {
                return;
            }
            window.push_notification(
                match event {
                    RelayEvent::Info(message) => Notification::info(message.clone()),
                    RelayEvent::Error(message) => Notification::error(message.clone()),
                },
                cx,
            );
        });
        let root = cx.entity().downgrade();
        let flow = match mode {
            BabelMode::Direct => AuthFlow::login_prompt(),
            BabelMode::Relay => AuthFlow::subaccount_login_prompt(),
        };
        let auth = cx.new(|cx| AuthScreen::new(root, flow, window, cx));
        let view = auth.clone().into();
        Self {
            shell,
            mode,
            client: None,
            view,
            dashboard: None,
            route: None,
            auth: Some(auth),
            error_task: None,
            error_events: None,
            audio_guard: None,
            reload: None,
            relay_events: Some(relay_events),
        }
    }

    fn shutdown_requests(&mut self, cx: &mut Context<Self>) {
        self.leave_task(cx);
        self.relay_events = None;
        if let Some(auth) = &self.auth {
            auth.update(cx, |auth, cx| {
                auth.cancel_request();
                cx.notify();
            });
        }
        if let Some(dashboard) = &self.dashboard {
            dashboard.update(cx, |dashboard, cx| dashboard.cancel_requests(cx));
        }
        self.client = None;
    }

    pub fn set_route(&mut self, route: StartupRoute) {
        self.route = (!route.is_empty()).then_some(route);
    }

    /// Only the explicitly selected Relay mode reads a saved relay session.
    pub fn open_auth_gate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.mode {
            BabelMode::Direct => {
                self.client = Some(BabelClient::direct(BabelConfig::from_env()));
                match AuthFlow::startup() {
                    AuthStartup::Authenticated => self.show_dashboard(window, cx),
                    AuthStartup::Flow(flow) => self.show_auth(flow, window, cx),
                }
            }
            BabelMode::Relay => match subaccounts::read_default_session() {
                Ok(Some(session)) => self.enter_relay(session, window, cx),
                result => {
                    if let Err(error) = result {
                        window.push_notification(
                            Notification::warning(format!(
                                "Sub-account session ignored: {error:#}"
                            )),
                            cx,
                        );
                    }
                    self.show_auth(AuthFlow::subaccount_login_prompt(), window, cx);
                }
            },
        }
    }

    pub fn enter_mode(&mut self, mode: BabelMode, window: &mut Window, cx: &mut Context<Self>) {
        self.shell.show_mode(mode.id(), window, cx);
    }

    pub fn enter_relay(
        &mut self,
        session: SubaccountSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.client = Some(BabelClient::relay(session));
        self.dashboard = None;
        self.show_dashboard(window, cx);
    }

    pub fn show_auth(&mut self, flow: AuthFlow, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_task(cx);
        self.dashboard = None;
        let root = cx.entity().downgrade();
        let auth = cx.new(|cx| AuthScreen::new(root, flow, window, cx));
        self.view = auth.clone().into();
        self.auth = Some(auth);
        cx.notify();
    }

    pub fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let result = match self.mode {
            BabelMode::Direct => sonus_babel_services::auth::reset_store(),
            BabelMode::Relay => subaccounts::clear_default_session(),
        };
        if let Err(error) = result {
            window.push_notification(
                Notification::error(format!("Could not clear the saved login: {error:#}")),
                cx,
            );
            return false;
        }
        self.client = None;
        let flow = match self.mode {
            BabelMode::Direct => AuthFlow::login_prompt(),
            BabelMode::Relay => AuthFlow::subaccount_login_prompt(),
        };
        self.show_auth(flow, window, cx);
        true
    }

    pub fn show_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_task(cx);
        let client = match &self.client {
            Some(client) => client.clone(),
            None if self.mode == BabelMode::Direct => BabelClient::direct(BabelConfig::from_env()),
            None => {
                self.show_auth(AuthFlow::subaccount_login_prompt(), window, cx);
                return;
            }
        };
        BabelServices::activate(client.clone(), cx);
        self.auth = None;
        self.client = Some(client.clone());
        match &self.dashboard {
            Some(dashboard) => dashboard.update(cx, |dashboard, cx| dashboard.refresh(window, cx)),
            None => {
                let root = cx.entity().downgrade();
                let route = self.route.take();
                let shell = self.shell.clone();
                self.dashboard =
                    Some(cx.new(|cx| DashboardScreen::new(root, shell, client, route, window, cx)));
            }
        }
        self.view = self
            .dashboard
            .as_ref()
            .expect("dashboard created")
            .clone()
            .into();
        cx.notify();
    }

    fn restore_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dashboard.is_some() {
            self.show_dashboard(window, cx);
        }
    }

    pub fn show_dashboard_error(
        &mut self,
        error: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let controller = cx.entity();
        window.defer(cx, move |window, cx| {
            controller.update(cx, |controller, cx| {
                controller.show_dashboard(window, cx);
                if let Some(dashboard) = &controller.dashboard {
                    dashboard.update(cx, |dashboard, cx| dashboard.show_error(error, cx));
                }
                controller.shell.show_view(cx.entity().into(), window, cx);
            });
        });
    }

    pub fn open_task(
        &mut self,
        workflow: StudioWorkflow,
        audio_base_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Entity<EditorModel>> {
        self.leave_task(cx);
        let client = self
            .client
            .clone()
            .ok_or_else(|| anyhow::anyhow!("This Babel mode is not signed in"))?;
        let project = workflow.into_project();
        let document =
            sonus_babel_domain::document_environment(&project, project.id.clone(), self.mode.id())?;
        let origin = DocumentOrigin {
            mode_id: self.mode.id().to_string(),
            provider_id: Some(self.mode.id().to_string()),
            read_only: document.policy.read_only,
            native_path: None,
        };
        let audio_sources =
            crate::services::local_audio_sources(&project, audio_base_dir.as_deref());
        let environment = ProviderEnvironment {
            audio_base_dir,
            audio_sources,
            document,
            values: BTreeMap::new(),
        };
        let model = self
            .shell
            .open_document(project, origin, environment, window, cx)?;
        set_document_client(&model, client, cx);
        Ok(model)
    }

    pub fn open_loaded_task(
        &mut self,
        task: LoadedTask,
        status: &'static str,
        failure_label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote_audio = task.remote_audio_count > 0;
        let model = match self.open_task(task.workflow, Some(task.audio_base_dir), window, cx) {
            Ok(model) => model,
            Err(error) => {
                let detail = format!("{failure_label} failed: {error:#}");
                if self.shell.editor_model(cx).is_some() {
                    window.push_notification(Notification::error(detail), cx);
                } else {
                    self.show_dashboard_error(detail, window, cx);
                }
                return;
            }
        };
        model.update(cx, |model, cx| model.set_status(status, cx));
        if remote_audio {
            self.audio_guard = Some(cx.observe_in(
                &model,
                window,
                move |this, model, window, cx| {
                    let load = model.read(cx).audio_load();
                    if load.pending > 0 {
                        return;
                    }
                    this.audio_guard = None;
                    if load.loaded == 0 && load.failed > 0 {
                        this.show_dashboard_error(
                            format!("{failure_label} failed: the task audio could not be loaded."),
                            window,
                            cx,
                        );
                    }
                },
            ));
        }
    }

    pub fn open_error_task(
        &mut self,
        open: ErrorTaskOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_task(cx);
        let screen = cx.new(|cx| ErrorTaskScreen::new(open, window, cx));
        self.error_events = Some(
            cx.subscribe_in(&screen, window, |this, _, event, window, cx| match event {
                ErrorTaskEvent::Leave => this.show_dashboard(window, cx),
                ErrorTaskEvent::NextTask { project_id } => {
                    this.show_dashboard(window, cx);
                    if let Some(dashboard) = this.dashboard.clone() {
                        dashboard.update(cx, |dashboard, cx| {
                            dashboard.claim_next_in_project(project_id, window, cx)
                        });
                    }
                }
            }),
        );
        let guarded = screen.clone();
        self.shell.set_leave_guard(
            Some(Rc::new(move |window, cx, confirmed| {
                let unload = guarded.clone();
                guarded.update(cx, |screen, cx| {
                    screen.confirm_leave(window, cx, move |window, cx| {
                        unload.update(cx, |screen, _| screen.unload());
                        confirmed(window, cx);
                    })
                });
            })),
            cx,
        );
        window.focus(&screen.read(cx).focus_handle(cx), cx);
        self.view = screen.clone().into();
        self.error_task = Some(screen);
        cx.notify();
    }

    pub fn dashboard(&self) -> Option<&Entity<DashboardScreen>> {
        self.dashboard.as_ref()
    }

    fn leave_task(&mut self, cx: &mut Context<Self>) {
        self.audio_guard = None;
        self.reload = None;
        self.error_events = None;
        if let Some(screen) = self.error_task.take() {
            screen.update(cx, |screen, _| screen.unload());
            self.shell.set_leave_guard(None, cx);
        }
    }
}

impl Render for BabelController {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.view.clone())
    }
}
