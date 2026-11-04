//! Dashboard: projects with daily progress and task claiming, task lookup, recovery drafts,
//! sub-account management and settings. Also applies the startup route once.

use std::collections::BTreeMap;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, IconName, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use studio_editor::edit::{RouteLoad, StartupRoute};
use studio_editor::editor_model::task_lookup::{TaskLookupEffect, TaskLookupState};
use studio_editor::workflow::StudioWorkflow;
use studio_services::auth::flow::AuthFlow;
use studio_services::babel::{
    DashboardProject, META_STITCHED_QUEUE_ID, NO_ACTIVE_PROJECTS_MESSAGE, OpenedTask,
    ROUTE_METADATA_KEYS, RouteContext, TaskRequest, daily_task_progress,
};
use studio_services::recovery::{self, RecoveryDraft};
use studio_services::subaccounts::{self, SubaccountSession};

use crate::editor::panels::dialogs::no_work;
use crate::root::{StudioRoot, demo_audio_dir};
use crate::screens::ShellModels;
use crate::screens::auth::{brand, brand_mark};
use crate::screens::error_task::ErrorTaskOpen;
use crate::screens::settings::SettingsView;
use crate::screens::subaccounts::{LoadProjects, SubaccountsView};
use crate::screens::task_lookup;
use crate::screens::ui::{self, Tone};
use crate::services::{AppServices, spawn_service_in};

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
    chunked_queue_id: String,
    stitched_queue_id: String,
}

struct Problem {
    title: SharedString,
    detail: SharedString,
    /// The Babel session is missing or expired: offer "Sign in again" instead of a retry.
    signed_out: bool,
}

impl Problem {
    fn signed_out() -> Self {
        Self {
            title: SESSION_EXPIRED.into(),
            detail: "Babel no longer accepts the saved login. Sign in again to load your \
                     projects and claim tasks."
                .into(),
            signed_out: true,
        }
    }
}

pub struct DashboardScreen {
    root: WeakEntity<StudioRoot>,
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
    subaccounts: Entity<SubaccountsView>,
    _subscriptions: Vec<Subscription>,
}

impl DashboardScreen {
    pub fn new(
        root: WeakEntity<StudioRoot>,
        route: Option<StartupRoute>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = cx.new(|cx| SettingsView::new(root.clone(), window, cx));
        let subaccounts = cx.new(|cx| SubaccountsView::new(window, cx));
        let subscriptions = vec![cx.subscribe_in(
            &subaccounts,
            window,
            |this, _, _: &LoadProjects, window, cx| this.refresh(window, cx),
        )];
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
            root,
            section: Section::Projects,
            projects: Vec::new(),
            worker_id: None,
            loaded: false,
            refresh: None,
            task_load: None,
            problem: None,
            recovery_drafts: Vec::new(),
            route_metadata,
            subaccount: None,
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

    pub fn projects(&self) -> &[DashboardProject] {
        &self.projects
    }

    pub fn show_error(&mut self, detail: String, cx: &mut Context<Self>) {
        self.problem = Some(Problem {
            title: "The task could not be opened".into(),
            detail: detail.into(),
            signed_out: false,
        });
        cx.notify();
    }

    /// Reloads projects (newest request wins), recovery drafts and the sub-account session.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recovery_drafts = recovery::discover_drafts();
        self.subaccount = subaccounts::read_default_session().ok().flatten();
        let babel = cx.global::<AppServices>().babel.clone();
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
                        if matches!(&this.problem, Some(p) if p.title.as_ref() == REFRESH_FAILED || p.signed_out)
                        {
                            this.problem = None;
                        }
                        let projects = this.projects.clone();
                        this.subaccounts
                            .update(cx, |view, cx| view.set_projects(projects.clone(), cx));
                        ShellModels::relay(cx)
                            .update(cx, |relay, cx| relay.sync_project_queues(&projects, cx));
                    }
                    Err(error) if error.signed_out => {
                        this.problem = Some(Problem::signed_out());
                    }
                    Err(error) => {
                        this.problem = Some(Problem {
                            title: REFRESH_FAILED.into(),
                            detail: error.to_string().into(),
                            signed_out: false,
                        });
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
                self.problem = Some(Problem {
                    title: "QA claims need a stitched queue".into(),
                    detail: "Open the lookup from a project that has a stitched queue.".into(),
                    signed_out: false,
                });
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
                self.problem = Some(Problem {
                    title: "Task lookup needs a stitched queue".into(),
                    detail: "Open the lookup from a project that has a stitched queue.".into(),
                    signed_out: false,
                });
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
            chunked_queue_id: scope.chunked_queue_id.clone(),
            stitched_queue_id: scope.stitched_queue_id,
            review_action_id: state.review_action_id,
            processed_transcription_id: processed,
        };
        let project_id = (!scope.project_id.is_empty()).then_some(scope.project_id);
        self.open_task(request, context, project_id, window, cx);
    }

    fn start_project(
        &mut self,
        project: &DashboardProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = RouteContext {
            project_id: project.id.clone(),
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
            TaskRequest::ClaimNext { .. } => "Claiming the next task…",
            TaskRequest::ClaimByProcessedTranscription { .. } => {
                "Claiming the stitched transcription…"
            }
            TaskRequest::ClaimErrorQaTask { .. } => "Claiming the QA task…",
            TaskRequest::ReviewAction { .. } => "Loading the review action…",
            TaskRequest::LatestReviewAction { .. } => "Loading the processed transcription…",
        }
        .into();
        self.problem = None;
        let babel = cx.global::<AppServices>().babel.clone();
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
        result: Result<OpenedTask, studio_services::babel::BabelError>,
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
                self.problem = Some(Problem::signed_out());
            }
            Err(error) => {
                self.problem = Some(Problem {
                    title: format!("{} failed", request.failure_label()).into(),
                    detail: error.message.into(),
                    signed_out: false,
                });
                if matches!(request, TaskRequest::ClaimNext { .. }) {
                    no_work::open("review", window, cx, |_, _| {});
                }
            }
        }
    }

    fn recover_latest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.recovery_drafts = recovery::discover_drafts();
        let Some(draft) = self.recovery_drafts.first().cloned() else {
            self.problem = Some(Problem {
                title: "Nothing to recover".into(),
                detail: "No local recovery draft is available.".into(),
                signed_out: false,
            });
            cx.notify();
            return;
        };
        match recovery::load_draft(&draft) {
            Ok(workflow) => {
                let base = draft.path.parent().map(std::path::Path::to_path_buf);
                let status = format!("Recovered local draft {}.", draft.label);
                self.open_local_workflow(workflow, base, status, true, window, cx);
            }
            Err(error) => {
                self.problem = Some(Problem {
                    title: "Recovery failed".into(),
                    detail: format!("{error:#}").into(),
                    signed_out: false,
                });
                cx.notify();
            }
        }
    }

    fn open_demo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match studio_editor::load_sample_project() {
            Ok(project) => self.open_local_workflow(
                StudioWorkflow::from_project(project),
                demo_audio_dir(),
                "Loaded the bundled demo task.".to_string(),
                false,
                window,
                cx,
            ),
            Err(error) => {
                self.problem = Some(Problem {
                    title: "Demo failed to load".into(),
                    detail: format!("{error:#}").into(),
                    signed_out: false,
                });
                cx.notify();
            }
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
                let model = root.open_task(workflow, audio_base_dir, window, cx);
                model.update(cx, |model, cx| {
                    if dirty {
                        model.update_doc(cx, |doc| doc.session_mut().mark_dirty());
                    }
                    model.set_status(status, cx);
                });
            })
            .ok();
    }

    fn select_section(&mut self, section: Section, window: &mut Window, cx: &mut Context<Self>) {
        self.section = section;
        if section == Section::Subaccounts && self.subaccount.is_none() {
            self.subaccounts
                .update(cx, |view, cx| view.ensure_loaded(window, cx));
        }
        cx.notify();
    }

    fn use_subaccount_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root
            .update(cx, |root, cx| {
                root.show_auth(AuthFlow::subaccount_login_prompt(), window, cx)
            })
            .ok();
    }

    fn use_babel_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = subaccounts::clear_default_session() {
            self.problem = Some(Problem {
                title: "Sub-account sign-out failed".into(),
                detail: format!("{error:#}").into(),
                signed_out: false,
            });
            cx.notify();
            return;
        }
        // The gate may show this dashboard again (refresh), which must not happen while it is
        // being updated.
        let root = self.root.clone();
        window.defer(cx, move |window, cx| {
            root.update(cx, |root, cx| root.open_auth_gate(window, cx))
                .ok();
        });
    }

    /// Clears the main Babel login and returns to sign-in.
    fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        window.defer(cx, move |window, cx| {
            root.update(cx, |root, cx| root.sign_out(window, cx)).ok();
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

        let account = match &self.subaccount {
            Some(session) => {
                let name = if session.display_name.trim().is_empty() {
                    session.subaccount_id.clone()
                } else {
                    session.display_name.clone()
                };
                v_flex()
                    .items_start()
                    .gap_1()
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(name))
                    .child(ui::muted("Sub-account workspace", cx))
                    .child(
                        Button::new("use-babel-account")
                            .ghost()
                            .small()
                            .label("Use Babel account")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.use_babel_account(window, cx)
                            })),
                    )
            }
            None => v_flex()
                .items_start()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Main Babel account"),
                )
                .child(ui::muted("Full project access", cx))
                .child(
                    Button::new("use-subaccount-code")
                        .ghost()
                        .small()
                        .label("Use sub-account code")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.use_subaccount_code(window, cx)),
                        ),
                )
                .child(
                    Button::new("sign-out")
                        .ghost()
                        .small()
                        .icon(sign_out_icon())
                        .label("Sign out")
                        .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx))),
                ),
        };

        let version = ShellModels::updater(cx)
            .read(cx)
            .state()
            .current_version
            .clone();
        let nav_items = v_flex()
            .gap_1()
            .when(!compact, |this| this.child(ui::caption("WORKSPACE", cx)))
            .child(nav(
                "nav-projects",
                IconName::LayoutDashboard,
                "Projects",
                Section::Projects,
            ))
            .child(nav(
                "nav-subaccounts",
                IconName::Network,
                "Sub-accounts",
                Section::Subaccounts,
            ))
            .child(nav(
                "nav-settings",
                IconName::Settings,
                "Settings",
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
            // Narrow window (or large zoom): icons only; the account actions stay reachable.
            let account_action = if self.subaccount.is_some() {
                Button::new("use-babel-account")
                    .ghost()
                    .small()
                    .icon(IconName::User)
                    .tooltip("Use Babel account")
                    .on_click(cx.listener(|this, _, window, cx| this.use_babel_account(window, cx)))
            } else {
                Button::new("sign-out")
                    .ghost()
                    .small()
                    .icon(sign_out_icon())
                    .tooltip("Sign out")
                    .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx)))
            };
            return sidebar
                .px_2()
                .items_center()
                .child(brand_mark(cx))
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
                    .child(ui::caption("ACCOUNT", cx))
                    .child(div().px_2().child(account)),
            )
            .child(
                div()
                    .px_2()
                    .child(ui::muted(format!("Version {version}"), cx)),
            )
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, subtitle) = match self.section {
            Section::Projects => (
                "Projects",
                if self.subaccount.is_some() {
                    "Choose an assigned project and continue where you left off."
                } else {
                    "Choose a Babel project and claim the next available task."
                },
            ),
            Section::Subaccounts => (
                "Sub-accounts",
                "Create focused workspaces and control which projects they can access.",
            ),
            Section::Settings => (
                "Settings",
                "This installation, integrations, updates and local files.",
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
                                    .label("Open demo")
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
                                .label("Find task")
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
                                    "Refreshing…"
                                } else {
                                    "Refresh"
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
        if self.projects.is_empty() {
            return None;
        }
        let (completed, limit, limited) = daily_task_progress(&self.projects);
        let theme = cx.theme();
        let summary = if limited > 0 {
            format!("{completed} of {limit} tasks today")
        } else {
            format!("{} projects available", self.projects.len())
        };
        let detail = if limited > 0 {
            format!(
                "Across {limited} project{} with a daily limit.",
                if limited == 1 { "" } else { "s" }
            )
        } else {
            "Babel reports no daily limit for these projects.".to_string()
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
                            v_flex().gap_0p5().child(ui::muted("Today", cx)).child(
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
                                "Ready to claim the next task"
                            } else {
                                "No claimable queue"
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
                            format!("{} of {limit} today", project.tasks_completed_today),
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
                            .tooltip("Find a task in this project")
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
                            .label("Start task")
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
                format!(
                    "Restore {} or one of {others} other drafts before starting another task.",
                    draft.label
                )
            } else {
                format!("Restore {} before starting another task.", draft.label)
            };
            content = content.child(ui::banner(
                Tone::Warning,
                "A local recovery draft is available",
                Some(body.into()),
                Some(
                    Button::new("recover-latest")
                        .warning()
                        .small()
                        .label("Recover latest")
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

        let list = if self.projects.is_empty() {
            let (title, detail): (&str, &str) = if !self.loaded && self.refresh.is_some() {
                (
                    "Loading your projects…",
                    "This usually takes only a moment.",
                )
            } else if self.loaded {
                ("No active projects", NO_ACTIVE_PROJECTS_MESSAGE)
            } else if self
                .problem
                .as_ref()
                .is_some_and(|problem| problem.signed_out)
            {
                (
                    "Projects unavailable",
                    "Sign in again to see your projects.",
                )
            } else {
                (
                    "Projects unavailable",
                    "Refresh once the connection to Babel is back.",
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
                                .child("Available projects"),
                        )
                        .child(ui::muted(format!("{} total", self.projects.len()), cx)),
                )
                .children(
                    self.projects
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
        if self.subaccount.is_some() {
            return v_flex()
                .w_full()
                .child(ui::banner(
                    Tone::Info,
                    "Sub-account device",
                    Some("Sub-accounts are managed from the main Babel account.".into()),
                    Some(
                        Button::new("subaccount-use-babel")
                            .outline()
                            .small()
                            .label("Use Babel account")
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.use_babel_account(window, cx)
                                }),
                            )
                            .into_any_element(),
                    ),
                    cx,
                ))
                .into_any_element();
        }
        self.subaccounts.clone().into_any_element()
    }
}

const REFRESH_FAILED: &str = "We couldn't refresh this workspace";
const SESSION_EXPIRED: &str = "Your Babel session has expired";
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
            let actions = if problem.signed_out {
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new("sign-in-again")
                            .primary()
                            .small()
                            .icon(sign_out_icon())
                            .label("Sign in again")
                            .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx))),
                    )
                    .child(dismiss)
                    .into_any_element()
            } else if problem.title.as_ref() == REFRESH_FAILED {
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
                            .label("Sign out")
                            .tooltip("Session broken? Sign out and sign in to Babel again.")
                            .on_click(cx.listener(|this, _, window, cx| this.sign_out(window, cx))),
                    )
                    .child(dismiss)
                    .into_any_element()
            } else {
                dismiss.into_any_element()
            };
            ui::banner(
                if problem.signed_out {
                    Tone::Warning
                } else {
                    Tone::Danger
                },
                problem.title.clone(),
                Some(problem.detail.clone()),
                Some(actions),
                cx,
            )
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
