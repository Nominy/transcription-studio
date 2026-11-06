//! Dashboard: projects with daily progress and task claiming, task lookup, recovery drafts,
//! sub-account management and settings. Also applies the startup route once.

use std::collections::BTreeMap;

use exaudio_babel_domain::editor_model::task_lookup::{TaskLookupEffect, TaskLookupState};
use exaudio_babel_domain::route::{RouteLoad, StartupRoute};
use exaudio_babel_services::babel::{
    DashboardProject, META_STITCHED_QUEUE_ID, OpenedTask, ROUTE_METADATA_KEYS, RouteContext,
    TaskRequest, daily_task_progress,
};
use exaudio_babel_services::recovery::{self, RecoveryDraft};
use exaudio_babel_services::subaccounts::SubaccountSession;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::workflow::StudioWorkflow;

use crate::editor::panels::dialogs::no_work;
use crate::root::{BabelController, DashboardFilter};
use crate::screens::ShellModels;
use crate::screens::auth::{brand, brand_mark};
use crate::screens::error_task::ErrorTaskOpen;
use crate::screens::subaccounts::{LoadProjects, SubaccountsView};
use crate::screens::task_lookup;
use crate::text::{count, is_russian, localized, localized_fmt};
use exaudio_babel_services::babel::BabelClient;
use exaudio_shell::screens::settings::SettingsView;
use exaudio_shell::screens::ui::{self, Tone};
use exaudio_shell::screens::updates;
use exaudio_shell::services::spawn_service_in;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    Projects,
    Subaccounts,
    Settings,
}

/// The one task load in flight (a newer request replaces and thereby cancels it).
struct TaskLoad {
    label: SharedString,
    project_id: Option<String>,
    _task: Task<()>,
}

/// Where a lookup was opened from: the stitched queue and project it claims against.
#[derive(Clone, Default)]
struct LookupScope {
    project_id: String,
    project_slug: String,
    chunked_queue_id: String,
    stitched_queue_id: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProblemKind {
    /// The Babel session is missing or expired: offer "Sign in again" instead of a retry.
    SignedOut,
    /// Refreshing the dashboard failed: offer "Sign out" as the one fix that always exists.
    RefreshFailed,
    Other,
}

struct Problem {
    /// Plain headline.
    title: SharedString,
    /// Next step, one sentence (empty when the headline is enough).
    detail: SharedString,
    /// Technical text, shown under "Details".
    technical: Option<SharedString>,
    kind: ProblemKind,
}

impl Problem {
    fn new(title: impl Into<SharedString>, detail: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            detail: detail.into(),
            technical: None,
            kind: ProblemKind::Other,
        }
    }

    fn technical(mut self, technical: impl std::fmt::Display) -> Self {
        let technical = technical.to_string();
        self.technical = (!technical.trim().is_empty()).then(|| technical.into());
        self
    }

    fn signed_out(cx: &App) -> Self {
        Self {
            kind: ProblemKind::SignedOut,
            ..Self::new(
                localized(cx, "Signed out of Babel", "Вы вышли из Babel"),
                localized(
                    cx,
                    "Sign in again to continue.",
                    "Войдите снова, чтобы продолжить.",
                ),
            )
        }
    }

    fn refresh_failed(cx: &App, error: impl std::fmt::Display) -> Self {
        Self {
            kind: ProblemKind::RefreshFailed,
            ..Self::new(localized(cx, "Couldn't refresh", "Не удалось обновить"), "")
                .technical(error)
        }
    }

    fn is_signed_out(&self) -> bool {
        self.kind == ProblemKind::SignedOut
    }
}

pub struct DashboardScreen {
    root: WeakEntity<BabelController>,
    shell: exaudio_shell::ShellHandle,
    client: BabelClient,
    /// Which projects this mode lists; every project is kept for owner-side features.
    filter: DashboardFilter,
    /// The account this dashboard works as (shown in the sidebar; removed on sign-out).
    account_label: SharedString,
    section: Section,
    projects: Vec<DashboardProject>,
    /// Signed-in worker (decides whether a viewed error task is editable).
    worker_id: Option<String>,
    loaded: bool,
    refresh: Option<Task<()>>,
    task_load: Option<TaskLoad>,
    problem: Option<Problem>,
    recovery_drafts: Vec<RecoveryDraft>,
    /// Route keys from the startup route, carried into every task opened in this session.
    route_metadata: Vec<(String, String)>,
    subaccount: Option<SubaccountSession>,
    settings: Entity<SettingsView>,
    /// Owner-side sub-account management; `None` for sub-account (relay) logins.
    subaccounts: Option<Entity<SubaccountsView>>,
    _subscriptions: Vec<Subscription>,
}

impl DashboardScreen {
    pub fn new(
        root: WeakEntity<BabelController>,
        shell: exaudio_shell::ShellHandle,
        client: BabelClient,
        filter: DashboardFilter,
        route: Option<StartupRoute>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let account_label: SharedString = root
            .upgrade()
            .map(|controller| controller.read(cx).instance().label.clone())
            .unwrap_or_default()
            .into();
        let settings = cx.new(|cx| SettingsView::new(shell.clone(), window, cx));
        let subaccounts = client
            .config()
            .auth_store
            .clone()
            .filter(|_| client.config().relay_session.is_none())
            .map(|auth_store| cx.new(|cx| SubaccountsView::new(auth_store, window, cx)));
        let mut subscriptions = vec![
            // The sidebar shows the update notice.
            cx.observe(
                &exaudio_shell::screens::ShellModels::updater(cx),
                |_, _, cx| cx.notify(),
            ),
        ];
        if let Some(subaccounts) = &subaccounts {
            subscriptions.push(cx.subscribe_in(
                subaccounts,
                window,
                |this, _, _: &LoadProjects, window, cx| this.refresh(window, cx),
            ));
        }
        let route_metadata = route
            .as_ref()
            .map(|route| {
                route
                    .metadata_entries()
                    .into_iter()
                    .filter(|(key, _)| ROUTE_METADATA_KEYS.contains(&key.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        let mut this = Self {
            subaccount: client.config().relay_session.clone(),
            client,
            root,
            shell,
            filter,
            account_label,
            section: Section::Projects,
            projects: Vec::new(),
            worker_id: None,
            loaded: false,
            refresh: None,
            task_load: None,
            problem: None,
            recovery_drafts: Vec::new(),
            route_metadata,
            settings,
            subaccounts,
            _subscriptions: subscriptions,
        };
        this.refresh(window, cx);
        if let Some(load) = route.as_ref().and_then(StartupRoute::load) {
            let route = route.unwrap_or_default();
            // Dialogs need the window root, which exists only after this view is built.
            cx.defer_in(window, move |this, window, cx| {
                this.apply_route(load, &route, window, cx)
            });
        }
        this
    }

    pub fn cancel_requests(&mut self, cx: &mut Context<Self>) {
        self.refresh = None;
        self.task_load = None;
        if let Some(subaccounts) = &self.subaccounts {
            subaccounts.update(cx, |view, cx| view.cancel_request(cx));
        }
    }

    /// Every project the account can see (owner-side features need the unfiltered list).
    pub fn projects(&self) -> &[DashboardProject] {
        &self.projects
    }

    /// The projects this mode shows.
    fn visible_projects(&self) -> impl Iterator<Item = &DashboardProject> {
        let filter = self.filter;
        self.projects
            .iter()
            .filter(move |project| filter.accepts(project))
    }

    pub fn show_error(&mut self, detail: String, cx: &mut Context<Self>) {
        self.problem = Some(
            Problem::new(
                localized(cx, "Couldn't open the task", "Не удалось открыть задачу"),
                "",
            )
            .technical(detail),
        );
        cx.notify();
    }

    /// Reloads projects (newest request wins) and recovery drafts using this mode's client.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recovery_drafts = recovery::discover_drafts();
        let babel = self.client.clone();
        self.refresh = Some(spawn_service_in(
            window,
            cx,
            async move { babel.load_dashboard().await },
            |this, result, _, cx| {
                this.refresh = None;
                match result {
                    Ok(dashboard) => {
                        this.loaded = true;
                        this.projects = dashboard.projects;
                        this.worker_id = dashboard.worker_id;
                        if matches!(&this.problem, Some(p) if p.kind != ProblemKind::Other) {
                            this.problem = None;
                        }
                        let projects = this.projects.clone();
                        if let Some(subaccounts) = &this.subaccounts {
                            subaccounts
                                .update(cx, |view, cx| view.set_projects(projects.clone(), cx));
                        }
                        if let Some(auth_store) = this.client.config().auth_store.clone()
                            && this.subaccount.is_none()
                        {
                            ShellModels::relay(cx).update(cx, |relay, cx| {
                                relay.sync_project_queues(&projects, auth_store, cx)
                            });
                        }
                    }
                    Err(error) if error.signed_out => {
                        this.problem = Some(Problem::signed_out(cx));
                    }
                    Err(error) => {
                        this.problem = Some(Problem::refresh_failed(cx, error));
                    }
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn apply_route(
        &mut self,
        load: RouteLoad,
        route: &StartupRoute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = RouteContext::default();
        match load {
            RouteLoad::OpenTaskLookup => {
                let scope = self.route_scope();
                let state = TaskLookupState::new(
                    route.processed_transcription_id.clone().unwrap_or_default(),
                    route.review_action_id.clone().unwrap_or_default(),
                    !scope.stitched_queue_id.is_empty(),
                );
                self.open_lookup(state, scope, window, cx);
            }
            RouteLoad::ReviewAction { review_action_id } => self.open_task(
                TaskRequest::ReviewAction { review_action_id },
                context,
                None,
                window,
                cx,
            ),
            RouteLoad::LatestReviewAction {
                processed_transcription_id,
                queue_id,
            } => self.open_task(
                TaskRequest::LatestReviewAction {
                    processed_transcription_id,
                    queue_id,
                },
                context,
                None,
                window,
                cx,
            ),
            RouteLoad::ClaimNextFromQueue { queue_id } => self.open_task(
                TaskRequest::ClaimNext { queue_id },
                context,
                None,
                window,
                cx,
            ),
        }
    }

    /// Lookup scope from the startup route (no project chosen).
    fn route_scope(&self) -> LookupScope {
        let value = |key: &str| {
            self.route_metadata
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap_or_default()
        };
        LookupScope {
            stitched_queue_id: value(META_STITCHED_QUEUE_ID),
            ..LookupScope::default()
        }
    }

    fn open_lookup(
        &mut self,
        state: TaskLookupState,
        scope: LookupScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let this = cx.entity().downgrade();
        task_lookup::open(state, window, cx, move |effect, state, window, cx| {
            let scope = scope.clone();
            let state = state.clone();
            this.update(cx, |this, cx| {
                this.apply_lookup(effect, state, scope, window, cx)
            })
            .ok();
        });
    }

    fn apply_lookup(
        &mut self,
        effect: TaskLookupEffect,
        state: TaskLookupState,
        scope: LookupScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let processed = state.processed_transcription_id.clone();
        let queue_id = scope.stitched_queue_id.clone();
        let request = match effect {
            TaskLookupEffect::ClaimQaTask if queue_id.is_empty() => {
                self.problem = Some(Self::lookup_needs_project(cx));
                cx.notify();
                return;
            }
            TaskLookupEffect::ClaimQaTask => TaskRequest::ClaimErrorQaTask {
                transcription_error_diff_id: state.qa_diff_id.clone(),
                queue_id: queue_id.clone(),
            },
            TaskLookupEffect::ViewReadOnlyByReviewActionId => TaskRequest::ReviewAction {
                review_action_id: state.review_action_id.clone(),
            },
            TaskLookupEffect::ClaimById
            | TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId
                if queue_id.is_empty() =>
            {
                self.problem = Some(Self::lookup_needs_project(cx));
                cx.notify();
                return;
            }
            TaskLookupEffect::ClaimById => TaskRequest::ClaimByProcessedTranscription {
                processed_transcription_id: processed.clone(),
                queue_id: queue_id.clone(),
            },
            TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId => {
                TaskRequest::LatestReviewAction {
                    processed_transcription_id: processed.clone(),
                    queue_id: queue_id.clone(),
                }
            }
        };
        let context = RouteContext {
            project_id: scope.project_id.clone(),
            project_slug: scope.project_slug,
            chunked_queue_id: scope.chunked_queue_id.clone(),
            stitched_queue_id: scope.stitched_queue_id,
            review_action_id: state.review_action_id,
            processed_transcription_id: processed,
        };
        let project_id = (!scope.project_id.is_empty()).then_some(scope.project_id);
        self.open_task(request, context, project_id, window, cx);
    }

    /// A lookup opened from the startup route without a project cannot take tasks.
    fn lookup_needs_project(cx: &App) -> Problem {
        Problem::new(
            localized(cx, "Couldn't open the task", "Не удалось открыть задачу"),
            localized(cx, "Search from a project.", "Откройте поиск из проекта."),
        )
    }

    fn start_project(
        &mut self,
        project: &DashboardProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = RouteContext {
            project_id: project.id.clone(),
            project_slug: project.name.clone(),
            chunked_queue_id: project.chunked_queue_id.clone(),
            stitched_queue_id: project.stitched_queue_id.clone(),
            ..RouteContext::default()
        };
        self.open_task(
            TaskRequest::ClaimNext {
                queue_id: project.chunked_queue_id.clone(),
            },
            context,
            Some(project.id.clone()),
            window,
            cx,
        );
    }

    /// Claims the next task of `project_id` (error tasks' "Next task").
    pub fn claim_next_in_project(
        &mut self,
        project_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(project) = self
            .projects
            .iter()
            .find(|project| project.id == project_id)
        {
            let project = project.clone();
            self.start_project(&project, window, cx);
        }
    }

    /// Fetches and opens a task; replaces (cancels) any load in flight.
    fn open_task(
        &mut self,
        request: TaskRequest,
        context: RouteContext,
        project_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut metadata: BTreeMap<String, String> = self.route_metadata.iter().cloned().collect();
        metadata.extend(context.metadata());
        let metadata: Vec<(String, String)> = metadata.into_iter().collect();
        let label: SharedString = match &request {
            TaskRequest::ClaimNext { .. }
            | TaskRequest::ClaimByProcessedTranscription { .. }
            | TaskRequest::ClaimErrorQaTask { .. } => {
                localized(cx, "Taking the task…", "Берём задачу…")
            }
            TaskRequest::ReviewAction { .. } | TaskRequest::LatestReviewAction { .. } => {
                localized(cx, "Loading the task…", "Загружаем задачу…")
            }
        }
        .into();
        self.problem = None;
        let babel = self.client.clone();
        let fetch = request.clone();
        let opened_project = project_id.clone();
        let task = spawn_service_in(
            window,
            cx,
            async move { babel.open_task(&fetch, metadata).await },
            move |this, result, window, cx| {
                this.task_opened(request, opened_project, result, window, cx)
            },
        );
        self.task_load = Some(TaskLoad {
            label,
            project_id,
            _task: task,
        });
        cx.notify();
    }

    fn task_opened(
        &mut self,
        request: TaskRequest,
        project_id: Option<String>,
        result: Result<OpenedTask, exaudio_babel_services::babel::BabelError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task_load = None;
        cx.notify();
        match result {
            Ok(OpenedTask::Transcript(task)) => {
                self.root
                    .update(cx, |root, cx| {
                        root.open_loaded_task(
                            *task,
                            request.success_status(),
                            request.failure_label(),
                            window,
                            cx,
                        )
                    })
                    .ok();
            }
            Ok(OpenedTask::ErrorTask(loaded)) => {
                let read_only =
                    request.error_task_read_only(&loaded.task, self.worker_id.as_deref());
                // Babel offers "Next task" only for claims from the project's queue.
                let next_project = matches!(request, TaskRequest::ClaimNext { .. })
                    .then(|| project_id.clone())
                    .flatten();
                let open = ErrorTaskOpen {
                    client: self.client.clone(),
                    loaded: *loaded,
                    project_id,
                    read_only,
                    next_project,
                    status: request.success_status(),
                };
                self.root
                    .update(cx, |root, cx| root.open_error_task(open, window, cx))
                    .ok();
            }
            Err(error) if error.signed_out => {
                self.problem = Some(Problem::signed_out(cx));
            }
            Err(error) => {
                self.problem = Some(
                    Problem::new(
                        localized_fmt(
                            cx,
                            "{x} didn't work",
                            "{x}: не удалось",
                            &[("x", request.failure_label())],
                        ),
                        "",
                    )
                    .technical(error.message),
                );
                if matches!(request, TaskRequest::ClaimNext { .. }) {
                    no_work::open("review", window, cx, |_, _| {});
                }
            }
        }
    }

    fn recover_latest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recovery_drafts = recovery::discover_drafts();
        let Some(draft) = self.recovery_drafts.first().cloned() else {
            self.problem = Some(Problem::new(
                localized(cx, "Nothing to recover", "Нечего восстанавливать"),
                "",
            ));
            cx.notify();
            return;
        };
        match recovery::load_draft(&draft) {
            Ok(workflow) => {
                let base = draft.path.parent().map(std::path::Path::to_path_buf);
                let status = localized_fmt(
                    cx,
                    "Restored {draft}.",
                    "Восстановлено: {draft}.",
                    &[("draft", &draft.label)],
                );
                self.open_local_workflow(workflow, base, status, true, window, cx);
            }
            Err(error) => {
                self.problem = Some(
                    Problem::new(
                        localized(
                            cx,
                            "Couldn't restore the work",
                            "Не удалось восстановить работу",
                        ),
                        "",
                    )
                    .technical(format!("{error:#}")),
                );
                cx.notify();
            }
        }
    }

    fn open_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = exaudio_shell::root::open_demo(&self.shell, window, cx) {
            self.problem = Some(
                Problem::new(
                    localized(cx, "Couldn't open the demo", "Не удалось открыть демо"),
                    "",
                )
                .technical(format!("{error:#}")),
            );
            cx.notify();
        }
    }

    fn open_local_workflow(
        &mut self,
        workflow: StudioWorkflow,
        audio_base_dir: Option<std::path::PathBuf>,
        status: String,
        dirty: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task_load = None;
        self.root
            .update(cx, |root, cx| {
                match root.open_task(workflow, audio_base_dir, window, cx) {
                    Ok(model) => model.update(cx, |model, cx| {
                        if dirty {
                            model.update_doc(cx, |doc| doc.session_mut().mark_dirty());
                        }
                        model.set_status(status, cx);
                    }),
                    Err(error) => root.show_dashboard_error(format!("{error:#}"), window, cx),
                }
            })
            .ok();
    }

    fn select_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.section = section;
        if section == Section::Subaccounts
            && let Some(subaccounts) = &self.subaccounts
        {
            subaccounts.update(cx, |view, cx| view.ensure_loaded(window, cx));
        }
        cx.notify();
    }

    /// Removes the account (credentials and all) and returns to Start. Deferred because
    /// the controller owning this dashboard is dropped on the way.
    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(controller) = self.root.upgrade() else {
            return;
        };
        let instance = controller.read(cx).instance().clone();
        let shell = self.shell.clone();
        window.defer(cx, move |window, cx| {
            crate::root::sign_out(instance, shell, window, cx);
        });
    }

    /// Expired session: remove the account and reopen the mode, which signs in again.
    fn sign_in_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(controller) = self.root.upgrade() else {
            return;
        };
        let (mode_id, instance) = {
            let controller = controller.read(cx);
            (
                controller.mode_id().to_string(),
                controller.instance().clone(),
            )
        };
        let shell = self.shell.clone();
        window.defer(cx, move |window, cx| {
            crate::root::sign_in_again(mode_id, instance, shell, window, cx);
        });
    }

    // ── rendering ──────────────────────────────────────────────────────────────────────────

    fn render_sidebar(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let nav =
            |id: &'static str, icon: IconName, label: &'static str, section: Section| {
                let selected = self.section == section;
                h_flex()
                    .id(id)
                    .gap_2p5()
                    .px_2p5()
                    .py_2()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(compact, |this| this.justify_center())
                    .when(selected, |this| {
                        this.bg(theme.sidebar_accent)
                            .text_color(theme.sidebar_accent_foreground)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!selected, |this| {
                        this.text_color(theme.sidebar_foreground.opacity(0.8))
                            .hover(|this| this.bg(theme.sidebar_accent.opacity(0.5)))
                    })
                    .child(Icon::new(icon).small())
                    .when(!compact, |this| this.child(label))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_section(section, window, cx)
                    }))
            };

        let account_kind = if self.subaccount.is_some() {
            localized(cx, "Sub-account", "Под-аккаунт")
        } else {
            localized(cx, "Babel account", "Аккаунт Babel")
        };
        let account = v_flex()
            .items_start()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.account_label.clone()),
            )
            .child(ui::muted(account_kind, cx))
            .child(
                Button::new("sign-out")
                    .ghost()
                    .small()
                    .icon(sign_out_icon())
                    .label(localized(cx, "Sign out", "Выйти"))
                    .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx))),
            );

        let version = exaudio_shell::screens::ShellModels::updater(cx)
            .read(cx)
            .state()
            .current_version
            .clone();
        let nav_items = v_flex()
            .gap_1()
            .when(!compact, |this| {
                this.child(ui::caption(localized(cx, "WORKSPACE", "ПРОСТРАНСТВО"), cx))
            })
            .child(nav(
                "nav-projects",
                IconName::LayoutDashboard,
                localized(cx, "Projects", "Проекты"),
                Section::Projects,
            ))
            .when(self.subaccount.is_none(), |this| {
                this.child(nav(
                    "nav-subaccounts",
                    IconName::Network,
                    localized(cx, "Sub-accounts", "Под-аккаунты"),
                    Section::Subaccounts,
                ))
            })
            .child(nav(
                "nav-settings",
                IconName::Settings,
                localized(cx, "Settings", "Настройки"),
                Section::Settings,
            ));
        let sidebar = v_flex()
            .flex_none()
            .h_full()
            .py_5()
            .gap_6()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border);
        if compact {
            // Narrow window (or large zoom): icons only; sign-out stays reachable.
            let account_action = Button::new("sign-out")
                .ghost()
                .small()
                .icon(sign_out_icon())
                .tooltip(localized(cx, "Sign out", "Выйти"))
                .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx)));
            return sidebar
                .px_2()
                .items_center()
                .child(brand_mark())
                .child(nav_items)
                .child(div().flex_1())
                .child(account_action);
        }
        sidebar
            .w(rems(15.))
            .px_3()
            .child(div().px_1().child(brand(cx)))
            .child(nav_items)
            .child(div().flex_1())
            .child(
                v_flex()
                    .gap_1()
                    .child(ui::caption(localized(cx, "ACCOUNT", "АККАУНТ"), cx))
                    .child(div().px_2().child(account)),
            )
            .children(updates::update_notice(cx))
            .child(div().px_2().child(ui::muted(
                localized_fmt(
                    cx,
                    "Version {version}",
                    "Версия {version}",
                    &[("version", &version)],
                ),
                cx,
            )))
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, subtitle) = match self.section {
            Section::Projects => (
                localized(cx, "Projects", "Проекты"),
                if self.subaccount.is_some() {
                    localized(
                        cx,
                        "Pick a project and continue where you left off.",
                        "Выберите проект и продолжите с того места, где остановились.",
                    )
                } else {
                    localized(
                        cx,
                        "Pick a project and take the next task.",
                        "Выберите проект и возьмите следующую задачу.",
                    )
                },
            ),
            Section::Subaccounts => (
                localized(cx, "Sub-accounts", "Под-аккаунты"),
                localized(
                    cx,
                    "Separate logins with their own project access.",
                    "Отдельные входы со своим доступом к проектам.",
                ),
            ),
            Section::Settings => (
                localized(cx, "Settings", "Настройки"),
                localized(
                    cx,
                    "This installation, integrations, updates and local files.",
                    "Эта установка, интеграции, обновления и локальные файлы.",
                ),
            ),
        };
        let busy = self.task_load.is_some();
        let refreshing = self.refresh.is_some();
        let theme = cx.theme();
        let row = h_flex()
            .w_full()
            .max_w(content_max_w())
            .mx_auto()
            .flex_wrap()
            .gap_4()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(rems(16.))
                    .gap_0p5()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(ui::muted(subtitle, cx)),
            )
            .when(self.section == Section::Projects, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .when(cfg!(debug_assertions), |this| {
                            this.child(
                                Button::new("open-demo")
                                    .ghost()
                                    .label(localized(cx, "Open demo", "Открыть демо"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_demo(window, cx)
                                    })),
                            )
                        })
                        .child(
                            Button::new("find-task")
                                .outline()
                                .icon(IconName::Search)
                                .label(localized(cx, "Find a task…", "Найти задачу…"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    let scope = this.route_scope();
                                    let state = TaskLookupState::new(
                                        "",
                                        "",
                                        !scope.stitched_queue_id.is_empty(),
                                    );
                                    this.open_lookup(state, scope, window, cx);
                                })),
                        )
                        .child(
                            Button::new("refresh-projects")
                                .primary()
                                .icon(IconName::RefreshCw)
                                .label(if refreshing {
                                    localized(cx, "Refreshing…", "Обновление…")
                                } else {
                                    localized(cx, "Refresh", "Обновить")
                                })
                                .loading(refreshing)
                                .disabled(refreshing)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.refresh(window, cx)),
                                ),
                        ),
                )
            });
        div()
            .w_full()
            .flex_none()
            .px_8()
            .py_5()
            .border_b_1()
            .border_color(theme.border)
            .child(row)
    }

    fn render_progress(&self, cx: &mut Context<Self>) -> Option<Div> {
        let visible: Vec<DashboardProject> = self.visible_projects().cloned().collect();
        if visible.is_empty() {
            return None;
        }
        let (completed, limit, limited) = daily_task_progress(&visible);
        let theme = cx.theme();
        let summary = if limited > 0 {
            localized_fmt(
                cx,
                "{a} of {b} today",
                "{a} из {b} сегодня",
                &[("a", &completed.to_string()), ("b", &limit.to_string())],
            )
        } else {
            is_russian(cx);
            count(
                visible.len() as u64,
                ("project", "projects"),
                ("проект", "проекта", "проектов"),
            )
        };
        let detail = if limited > 0 {
            is_russian(cx);
            localized_fmt(
                cx,
                "Across {projects} with a daily limit.",
                "По {projects} с дневным лимитом.",
                &[(
                    "projects",
                    &count(
                        limited as u64,
                        ("project", "projects"),
                        ("проекту", "проектам", "проектам"),
                    ),
                )],
            )
        } else {
            localized(cx, "Without a daily limit.", "Без дневного лимита.").to_string()
        };
        Some(
            v_flex()
                .w_full()
                .gap_2()
                .p_4()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.group_box)
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_x_4()
                        .gap_y_1()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(ui::muted(localized(cx, "Today", "Сегодня"), cx))
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(summary),
                                ),
                        )
                        .child(ui::muted(detail, cx)),
                )
                .when(limited > 0 && limit > 0, |this| {
                    this.child(Progress::new("daily-progress").value(percent(completed, limit)))
                }),
        )
    }

    fn render_project(
        &self,
        ix: usize,
        project: &DashboardProject,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let claimable = !project.chunked_queue_id.trim().is_empty();
        let busy = self.task_load.is_some();
        let loading_here = self
            .task_load
            .as_ref()
            .is_some_and(|load| load.project_id.as_deref() == Some(project.id.as_str()));
        let start = project.clone();
        let claims_qa = project.is_error_categorization();
        let lookup_scope = LookupScope {
            project_id: project.id.clone(),
            project_slug: project.name.clone(),
            chunked_queue_id: project.chunked_queue_id.clone(),
            stitched_queue_id: project.stitched_queue_id.clone(),
        };
        h_flex()
            .w_full()
            .flex_wrap()
            .gap_x_4()
            .gap_y_2()
            .px_4()
            .py_3p5()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(if loading_here {
                theme.primary
            } else {
                theme.border
            })
            .bg(theme.background)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(rems(14.))
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(project.name.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(div().size_1p5().rounded_full().bg(if claimable {
                                theme.success
                            } else {
                                theme.muted_foreground
                            }))
                            .child(if claimable {
                                localized(cx, "Tasks available", "Есть задачи")
                            } else {
                                localized(cx, "No tasks available", "Нет задач")
                            }),
                    ),
            )
            .when_some(project.daily_task_limit, |this, limit| {
                this.child(
                    v_flex()
                        .w(rems(11.25))
                        .flex_none()
                        .gap_1()
                        .child(ui::muted(
                            localized_fmt(
                                cx,
                                "{a} of {b} today",
                                "{a} из {b} сегодня",
                                &[
                                    ("a", &project.tasks_completed_today.to_string()),
                                    ("b", &limit.to_string()),
                                ],
                            ),
                            cx,
                        ))
                        .child(
                            Progress::new(("project-progress", ix))
                                .value(percent(project.tasks_completed_today, limit)),
                        ),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .flex_none()
                    .child(
                        Button::new(("lookup", ix))
                            .ghost()
                            .small()
                            .icon(IconName::Search)
                            .tooltip(localized(
                                cx,
                                "Find a task in this project",
                                "Найти задачу в этом проекте",
                            ))
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let state = TaskLookupState::new(
                                    "",
                                    "",
                                    !lookup_scope.stitched_queue_id.is_empty(),
                                )
                                .claiming_qa_tasks(claims_qa);
                                this.open_lookup(state, lookup_scope.clone(), window, cx);
                            })),
                    )
                    .child(
                        Button::new(("start", ix))
                            .primary()
                            .small()
                            .label(localized(cx, "Take next task", "Взять задачу"))
                            .loading(loading_here)
                            .disabled(busy || !claimable)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.start_project(&start, window, cx)
                            })),
                    ),
            )
    }

    fn render_projects(&self, cx: &mut Context<Self>) -> Div {
        let mut content = v_flex().w_full().gap_4();
        if let Some(load) = &self.task_load {
            content = content.child(ui::banner(
                Tone::Info,
                load.label.clone(),
                None,
                Some(Spinner::new().small().into_any_element()),
                cx,
            ));
        }
        if let Some(draft) = self.recovery_drafts.first() {
            let others = self.recovery_drafts.len() - 1;
            let body = if others > 0 {
                is_russian(cx);
                localized_fmt(
                    cx,
                    "Restore {draft} or one of {others} before starting another task.",
                    "Восстановите {draft} или {others}, прежде чем брать другую задачу.",
                    &[
                        ("draft", &draft.label),
                        (
                            "others",
                            &count(
                                others as u64,
                                ("other draft", "other drafts"),
                                ("другой черновик", "других черновика", "других черновиков"),
                            ),
                        ),
                    ],
                )
            } else {
                localized_fmt(
                    cx,
                    "Restore {draft} before starting another task.",
                    "Восстановите {draft}, прежде чем брать другую задачу.",
                    &[("draft", &draft.label)],
                )
            };
            content = content.child(ui::banner(
                Tone::Warning,
                localized(cx, "Unsaved work found", "Есть несохранённая работа"),
                Some(body.into()),
                Some(
                    Button::new("recover-latest")
                        .warning()
                        .small()
                        .label(localized(cx, "Restore latest", "Восстановить последнее"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.recover_latest(window, cx)),
                        )
                        .into_any_element(),
                ),
                cx,
            ));
        }
        if let Some(progress) = self.render_progress(cx) {
            content = content.child(progress);
        }

        let visible: Vec<&DashboardProject> = self.visible_projects().collect();
        let list = if visible.is_empty() {
            let (title, detail): (&str, &str) = if !self.loaded && self.refresh.is_some() {
                (
                    localized(cx, "Loading your projects…", "Загрузка проектов…"),
                    localized(
                        cx,
                        "This usually takes a moment.",
                        "Обычно это занимает несколько секунд.",
                    ),
                )
            } else if self.loaded {
                (
                    localized(cx, "No active projects", "Нет активных проектов"),
                    localized(
                        cx,
                        "Refresh to check again.",
                        "Обновите, чтобы проверить снова.",
                    ),
                )
            } else if self.problem.as_ref().is_some_and(Problem::is_signed_out) {
                (
                    localized(cx, "Projects unavailable", "Проекты недоступны"),
                    localized(
                        cx,
                        "Sign in again to see your projects.",
                        "Войдите снова, чтобы увидеть проекты.",
                    ),
                )
            } else {
                (
                    localized(cx, "Projects unavailable", "Проекты недоступны"),
                    localized(
                        cx,
                        "Refresh once Babel is reachable.",
                        "Обновите, когда Babel снова будет доступен.",
                    ),
                )
            };
            v_flex()
                .w_full()
                .items_center()
                .gap_2()
                .py_16()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_dashed()
                .border_color(cx.theme().border)
                .when(self.refresh.is_some(), |this| this.child(Spinner::new()))
                .when(self.refresh.is_none(), |this| {
                    this.child(
                        Icon::new(IconName::Inbox)
                            .large()
                            .text_color(cx.theme().muted_foreground),
                    )
                })
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(ui::muted(detail, cx))
        } else {
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(localized(cx, "Projects", "Проекты")),
                        )
                        .child(ui::muted(
                            localized_fmt(
                                cx,
                                "{n} total",
                                "Всего: {n}",
                                &[("n", &visible.len().to_string())],
                            ),
                            cx,
                        )),
                )
                .children(
                    visible
                        .iter()
                        .enumerate()
                        .map(|(ix, project)| {
                            self.render_project(ix, project, cx).into_any_element()
                        })
                        .collect::<Vec<_>>(),
                )
        };
        content.child(list)
    }

    fn render_subaccounts(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.subaccounts {
            Some(subaccounts) => subaccounts.clone().into_any_element(),
            None => v_flex()
                .w_full()
                .child(ui::banner(
                    Tone::Info,
                    localized(cx, "Sub-account", "Под-аккаунт"),
                    Some(
                        localized(
                            cx,
                            "Sub-accounts are managed from the main Babel account.",
                            "Под-аккаунтами управляют из основного аккаунта Babel.",
                        )
                        .into(),
                    ),
                    None,
                    cx,
                ))
                .into_any_element(),
        }
    }
}

/// Below this many rems of window width the sidebar collapses to icons.
const COMPACT_SIDEBAR_BELOW_REMS: f32 = 60.;

/// Width of the centered content column shared by the header and the body.
fn content_max_w() -> Rems {
    rems(72.)
}

fn sign_out_icon() -> Icon {
    Icon::empty().path("icons/log-out.svg")
}

fn percent(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32 * 100.0).min(100.0)
    }
}

impl Render for DashboardScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let problem = self.problem.as_ref().map(|problem| {
            let dismiss = Button::new("dismiss-problem")
                .ghost()
                .xsmall()
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.problem = None;
                    cx.notify();
                }));
            let actions = if problem.is_signed_out() {
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("sign-in-again")
                            .primary()
                            .small()
                            .icon(sign_out_icon())
                            .label(localized(cx, "Sign in again", "Войти снова"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.sign_in_again(window, cx)),
                            ),
                    )
                    .child(dismiss)
                    .into_any_element()
            } else if problem.kind == ProblemKind::RefreshFailed {
                // An unrecognised auth failure looks like any other refresh error; signing out
                // and back in is the one fix the user can always try.
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("sign-out-recover")
                            .outline()
                            .small()
                            .icon(sign_out_icon())
                            .label(localized(cx, "Sign out", "Выйти"))
                            .tooltip(localized(
                                cx,
                                "Sign out and sign in to Babel again.",
                                "Выйдите и войдите в Babel снова.",
                            ))
                            .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx))),
                    )
                    .child(dismiss)
                    .into_any_element()
            } else {
                dismiss.into_any_element()
            };
            let banner = ui::banner(
                if problem.is_signed_out() {
                    Tone::Warning
                } else {
                    Tone::Danger
                },
                problem.title.clone(),
                (!problem.detail.is_empty()).then(|| problem.detail.clone()),
                Some(actions),
                cx,
            );
            match &problem.technical {
                Some(technical) => v_flex()
                    .w_full()
                    .gap_1()
                    .child(banner)
                    .child(div().px_4().child(ui::details(technical.clone(), cx))),
                None => banner,
            }
        });
        let body: AnyElement = match self.section {
            Section::Projects => self.render_projects(cx).into_any_element(),
            Section::Subaccounts => self.render_subaccounts(cx),
            Section::Settings => self.settings.clone().into_any_element(),
        };
        let compact = window.viewport_size().width < window.rem_size() * COMPACT_SIDEBAR_BELOW_REMS;
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_sidebar(compact, cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.render_header(cx))
                    .child(
                        div()
                            .id("dashboard-content")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .child(
                                div().w_full().px_8().py_6().child(
                                    v_flex()
                                        .w_full()
                                        .max_w(content_max_w())
                                        .mx_auto()
                                        .gap_4()
                                        .children(problem)
                                        .child(body),
                                ),
                            ),
                    ),
            )
    }
}
