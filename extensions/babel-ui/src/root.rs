//! One `BabelController` per (mode, account): it owns the account's client and keeps the
//! dashboard alive while the shell shows an editor. Credentials live in the account's
//! directory (see `exaudio_babel_services::account`); the shell decides which account a mode
//! launches with.
use std::{collections::BTreeMap, path::PathBuf, rc::Rc};

use exaudio_babel_domain::route::StartupRoute;
use exaudio_babel_services::account::{AccountStatus, AccountStore};
use exaudio_babel_services::babel::{BabelClient, LoadedTask};
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::screens::ui;
use exaudio_shell::{
    DocumentOrigin, ModeLaunchContext, ProviderEnvironment, ProviderInstance, ProviderRegistry,
    ProviderStatus, ShellHandle,
};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt as _};
use gpui_kit::*;
use studio_editor::workflow::StudioWorkflow;

use crate::screens::error_task::{ErrorTaskEvent, ErrorTaskOpen, ErrorTaskScreen};
use crate::screens::relay::RelayEvent;
use crate::screens::{ShellModels, dashboard::DashboardScreen};
use crate::services::{BabelServices, set_document_client};
use crate::text::{localized, localized_fmt, with_detail};
use crate::{ERRORS_ID, ID, PROVIDER_ID};

/// `{label} didn't work` — the headline for a failed dashboard action.
fn did_not_work(label: &str, cx: &App) -> String {
    localized_fmt(
        cx,
        "{label} didn't work",
        "{label}: не удалось",
        &[("label", label)],
    )
}

/// Which projects a mode's dashboard lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashboardFilter {
    /// Transcription projects (everything that is not error categorization).
    Transcription,
    /// Error-categorization projects only; tasks open the error review screen.
    ErrorReview,
}

impl DashboardFilter {
    pub fn for_mode(mode_id: &str) -> Self {
        if mode_id == ERRORS_ID {
            Self::ErrorReview
        } else {
            Self::Transcription
        }
    }

    pub fn accepts(self, project: &exaudio_babel_services::babel::DashboardProject) -> bool {
        match self {
            Self::Transcription => !project.is_error_categorization(),
            Self::ErrorReview => project.is_error_categorization(),
        }
    }
}

/// Controllers keyed by `(mode id, account instance id)`.
#[derive(Default)]
struct Controllers(BTreeMap<(String, String), Entity<BabelController>>);
impl Global for Controllers {}

pub fn controller(mode_id: &str, instance_id: &str, cx: &App) -> Option<Entity<BabelController>> {
    cx.try_global::<Controllers>()?
        .0
        .get(&(mode_id.to_string(), instance_id.to_string()))
        .cloned()
}

/// Any live controller of `mode_id` (relay-host notifications and project-queue sync only
/// need one direct-account dashboard).
pub fn controller_for_mode(mode_id: &str, cx: &App) -> Option<Entity<BabelController>> {
    cx.try_global::<Controllers>()?
        .0
        .iter()
        .find(|((mode, _), _)| mode == mode_id)
        .map(|(_, controller)| controller.clone())
}

/// The account directory of a provider instance.
pub fn account_store(instance: &ProviderInstance) -> AccountStore {
    AccountStore::new(exaudio_shell::providers::instance_dir(instance))
}

/// Provider status: credentials present and usable.
pub fn account_status(instance: &ProviderInstance) -> ProviderStatus {
    match account_store(instance).current_status() {
        AccountStatus::Ready { .. } => ProviderStatus::Ready {
            detail: instance.label.clone(),
        },
        AccountStatus::NeedsSignIn => ProviderStatus::NeedsSignIn,
    }
}

/// Provider sign-out: drop every controller using the account and delete its credentials.
/// The shell removes the instance record and its directory afterwards.
pub fn sign_out_instance(instance: &ProviderInstance, cx: &mut App) {
    if cx.has_global::<Controllers>() {
        let dropped: Vec<_> = {
            let controllers = &mut cx.global_mut::<Controllers>().0;
            let keys: Vec<_> = controllers
                .keys()
                .filter(|(_, id)| *id == instance.id)
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| controllers.remove(&key))
                .collect()
        };
        for controller in dropped {
            controller.update(cx, |controller, cx| controller.shutdown_requests(cx));
        }
    }
    // The shell deletes the whole account directory next; a failure here is not fatal.
    let _ = account_store(instance).clear_credentials();
}

/// Registers instances whose directories exist under the providers root but which the
/// registry does not know (a portable install moved to another machine). Never changes
/// the selection of accounts the registry already has.
pub fn rediscover_accounts(cx: &mut App) {
    let provider_dir = exaudio_shell::providers::providers_root().join(PROVIDER_ID);
    let records = exaudio_babel_services::account::discover_records(&provider_dir);
    if records.is_empty() {
        return;
    }
    let selected = cx.global::<ProviderRegistry>().selected(PROVIDER_ID);
    for record in records {
        if cx
            .global::<ProviderRegistry>()
            .instance(PROVIDER_ID, &record.id)
            .is_some()
        {
            continue;
        }
        cx.update_global::<ProviderRegistry, _>(|registry, cx| {
            registry.upsert(
                ProviderInstance {
                    provider: PROVIDER_ID.into(),
                    id: record.id,
                    label: record.label,
                },
                cx,
            );
        });
    }
    if let Some(selected) = selected {
        cx.update_global::<ProviderRegistry, _>(|registry, cx| registry.select(&selected, cx));
    }
}

/// Removes the account (credentials, record, directory) and goes back to Start.
pub fn sign_out(instance: ProviderInstance, shell: ShellHandle, window: &mut Window, cx: &mut App) {
    cx.update_global::<ProviderRegistry, _>(|registry, cx| registry.remove(&instance, cx));
    shell.show_home(window, cx);
}

/// Removes the account and reopens the mode, which runs the sign-in step again.
pub fn sign_in_again(
    mode_id: String,
    instance: ProviderInstance,
    shell: ShellHandle,
    window: &mut Window,
    cx: &mut App,
) {
    cx.update_global::<ProviderRegistry, _>(|registry, cx| registry.remove(&instance, cx));
    shell.show_mode(&mode_id, window, cx);
}

/// Reloads a remote task using the client captured when this document was opened.
pub fn reload_task(model: &Entity<EditorModel>, window: &mut Window, cx: &mut App) {
    use exaudio_babel_services::babel::{
        OpenedTask, TaskRequest, review_action_id, route_metadata_from_project,
    };
    let origin = model.read(cx).origin().clone();
    let Some(instance) = origin.instance.as_ref() else {
        return;
    };
    let project = model.read(cx).doc().project();
    let review_action_id = review_action_id(project);
    let metadata = route_metadata_from_project(project);
    if review_action_id.is_empty() {
        window.push_notification(
            Notification::error(localized(
                cx,
                "This task can't be reloaded.",
                "Эту задачу нельзя перезагрузить.",
            )),
            cx,
        );
        return;
    }
    let Some(controller) = controller(&origin.mode_id, &instance.id, cx) else {
        window.push_notification(
            Notification::error(localized(
                cx,
                "The task's workspace is no longer open.",
                "Пространство этой задачи уже закрыто.",
            )),
            cx,
        );
        return;
    };
    let client = crate::services::document_client(model, cx);
    controller.update(cx, |controller, cx| {
        let request = TaskRequest::ReviewAction { review_action_id };
        let transport = client.clone();
        controller.reload = Some(exaudio_shell::services::spawn_service_in(
            window,
            cx,
            async move { transport.open_task(&request, metadata).await },
            move |controller, result, window, cx| {
                controller.reload = None;
                match result {
                    Ok(OpenedTask::Transcript(task)) => {
                        controller.open_loaded_task(
                            *task,
                            localized(cx, "Task reloaded", "Задача перезагружена"),
                            localized(cx, "Reloading the task", "Перезагрузка задачи"),
                            window,
                            cx,
                        );
                    }
                    Ok(OpenedTask::ErrorTask(_)) => window.push_notification(
                        Notification::error(localized(
                            cx,
                            "This task no longer has a transcript.",
                            "У этой задачи больше нет расшифровки.",
                        )),
                        cx,
                    ),
                    Err(error) => window.push_notification(
                        ui::error_notification(
                            localized(
                                cx,
                                "Couldn't reload the task.",
                                "Не удалось перезагрузить задачу.",
                            ),
                            Some(format!("{error:#}").into()),
                        ),
                        cx,
                    ),
                }
            },
        ));
    });
}

/// Mode launch: the dashboard of `(mode, account)`, kept alive across editor sessions.
/// `None` when the account has no usable credentials (the shell re-runs sign-in next time).
pub fn launch(
    shell: ShellHandle,
    context: ModeLaunchContext,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    let Some(instance) = context.instance else {
        window.push_notification(
            Notification::error(localized(
                cx,
                "Sign in to a Babel account to open this workspace.",
                "Войдите в аккаунт Babel, чтобы открыть это пространство.",
            )),
            cx,
        );
        return None;
    };
    if !cx.has_global::<Controllers>() {
        cx.set_global(Controllers::default());
    }
    let key = (context.mode_id.clone(), instance.id.clone());
    if let Some(controller) = cx.global::<Controllers>().0.get(&key).cloned() {
        let home = controller.clone();
        window.defer(cx, move |window, cx| {
            home.update(cx, |controller, cx| controller.restore_home(window, cx));
        });
        return Some(controller.into());
    }
    let client = match account_store(&instance).client() {
        Ok(Some(client)) => client,
        Ok(None) => {
            window.push_notification(
                Notification::error(localized_fmt(
                    cx,
                    "{account} is signed out. Open the workspace again to sign in.",
                    "{account}: выполнен выход. Откройте пространство снова, чтобы войти.",
                    &[("account", &instance.label)],
                )),
                cx,
            );
            return None;
        }
        Err(error) => {
            window.push_notification(
                ui::error_notification(
                    localized(
                        cx,
                        "Couldn't read the saved sign-in.",
                        "Не удалось прочитать сохранённый вход.",
                    ),
                    Some(format!("{error:#}").into()),
                ),
                cx,
            );
            return None;
        }
    };
    let controller =
        cx.new(|cx| BabelController::new(shell, context.mode_id, instance, client, window, cx));
    cx.global_mut::<Controllers>()
        .0
        .insert(key, controller.clone());
    let weak = controller.downgrade();
    window.defer(cx, move |window, cx| {
        weak.update(cx, |controller, cx| controller.show_dashboard(window, cx))
            .ok();
    });
    Some(controller.into())
}

/// Drops every controller (their clients point at account directories that may have moved).
pub fn reset_controllers(cx: &mut App) {
    if cx.has_global::<Controllers>() {
        let controllers = std::mem::take(&mut cx.global_mut::<Controllers>().0);
        for controller in controllers.into_values() {
            controller.update(cx, |controller, cx| controller.shutdown_requests(cx));
        }
    }
}

pub fn shutdown(cx: &mut App) {
    reset_controllers(cx);
    if cx.has_global::<ShellModels>() {
        ShellModels::relay(cx).update(cx, |relay, _| relay.shutdown());
    }
}

pub struct BabelController {
    shell: ShellHandle,
    mode_id: String,
    instance: ProviderInstance,
    client: BabelClient,
    view: Option<AnyView>,
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
        mode_id: String,
        instance: ProviderInstance,
        client: BabelClient,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Relay-host notices are shown once, by the owner's main Babel dashboard.
        let shows_relay_events = mode_id == ID && client.config().relay_session.is_none();
        let relay_events = shows_relay_events.then(|| {
            let relay = ShellModels::relay(cx);
            cx.subscribe_in(&relay, window, |_, _, event, window, cx| {
                window.push_notification(
                    match event {
                        RelayEvent::Info(message) => Notification::info(message.clone()),
                        RelayEvent::Error { headline, detail } => {
                            ui::error_notification(headline.clone(), detail.clone())
                        }
                    },
                    cx,
                );
            })
        });
        Self {
            shell,
            mode_id,
            instance,
            client,
            view: None,
            dashboard: None,
            route: None,
            error_task: None,
            error_events: None,
            audio_guard: None,
            reload: None,
            relay_events,
        }
    }

    fn shutdown_requests(&mut self, cx: &mut Context<Self>) {
        self.leave_task(cx);
        self.relay_events = None;
        if let Some(dashboard) = &self.dashboard {
            dashboard.update(cx, |dashboard, cx| dashboard.cancel_requests(cx));
        }
    }

    pub fn set_route(&mut self, route: StartupRoute) {
        self.route = (!route.is_empty()).then_some(route);
    }

    pub fn mode_id(&self) -> &str {
        &self.mode_id
    }

    pub fn instance(&self) -> &ProviderInstance {
        &self.instance
    }

    /// The account's `auth.json` for owner-side relay calls; `None` for sub-accounts.
    pub fn owner_auth_store(&self) -> Option<PathBuf> {
        self.client
            .config()
            .auth_store
            .clone()
            .filter(|_| self.client.config().relay_session.is_none())
    }

    pub fn show_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_task(cx);
        BabelServices::activate(self.client.clone(), cx);
        match &self.dashboard {
            Some(dashboard) => dashboard.update(cx, |dashboard, cx| dashboard.refresh(window, cx)),
            None => {
                let root = cx.entity().downgrade();
                let route = self.route.take();
                let shell = self.shell.clone();
                let client = self.client.clone();
                let filter = DashboardFilter::for_mode(&self.mode_id);
                self.dashboard = Some(cx.new(|cx| {
                    DashboardScreen::new(root, shell, client, filter, route, window, cx)
                }));
            }
        }
        self.view = self.dashboard.clone().map(Into::into);
        cx.notify();
    }

    fn restore_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_dashboard(window, cx);
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
        let project = workflow.into_project();
        let document = exaudio_babel_domain::document_environment(
            &project,
            project.id.clone(),
            &self.mode_id,
        )?;
        let origin = DocumentOrigin {
            mode_id: self.mode_id.clone(),
            instance: Some(self.instance.clone()),
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
        set_document_client(&model, self.client.clone(), cx);
        Ok(model)
    }

    /// `status` is shown in the editor after the task opened; `failure_label` names the
    /// action in `{label} didn't work` when it fails.
    pub fn open_loaded_task(
        &mut self,
        task: LoadedTask,
        status: impl Into<SharedString>,
        failure_label: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let failure_label = failure_label.into();
        let remote_audio = task.remote_audio_count > 0;
        let model = match self.open_task(task.workflow, Some(task.audio_base_dir), window, cx) {
            Ok(model) => model,
            Err(error) => {
                let headline = did_not_work(&failure_label, cx);
                if self.shell.editor_model(cx).is_some() {
                    window.push_notification(
                        ui::error_notification(headline, Some(format!("{error:#}").into())),
                        cx,
                    );
                } else {
                    self.show_dashboard_error(
                        with_detail(&headline, format!("{error:#}")),
                        window,
                        cx,
                    );
                }
                return;
            }
        };
        let status = status.into();
        model.update(cx, |model, cx| model.set_status(status, cx));
        if remote_audio {
            self.audio_guard =
                Some(
                    cx.observe_in(&model, window, move |this, model, window, cx| {
                        let load = model.read(cx).audio_load();
                        if load.pending > 0 {
                            return;
                        }
                        this.audio_guard = None;
                        if load.loaded == 0 && load.failed > 0 {
                            let headline = did_not_work(&failure_label, cx);
                            this.show_dashboard_error(
                                with_detail(
                                    &headline,
                                    localized(
                                        cx,
                                        "The task audio couldn't be loaded.",
                                        "Не удалось загрузить аудио задачи.",
                                    ),
                                ),
                                window,
                                cx,
                            );
                        }
                    }),
                );
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
        self.view = Some(screen.clone().into());
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
            .children(self.view.clone())
    }
}
