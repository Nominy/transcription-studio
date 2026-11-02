//! Root view: switches between the auth, dashboard and editor screens, and owns the
//! window-level flows (close / hide to tray / exit with the unsaved-changes confirm).
//!
//! Screens navigate by calling the `show_*`/`open_*` methods through the
//! `WeakEntity<StudioRoot>` they receive at construction.

use std::path::PathBuf;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt as _};
use gpui_kit::*;
use studio_editor::edit::StartupRoute;
use studio_editor::workflow::StudioWorkflow;
use studio_services::auth::flow::{AuthFlow, AuthStartup};
use studio_services::babel::LoadedTask;
use studio_services::subaccounts;

use crate::editor::model::EditorModel;
use crate::editor::{EditorScreen, EditorScreenEvent, panels};
use crate::screens::ShellModels;
use crate::screens::auth::AuthScreen;
use crate::screens::dashboard::DashboardScreen;
use crate::screens::relay::RelayEvent;
use crate::tray;

enum Screen {
    Auth(Entity<AuthScreen>),
    Dashboard,
    Editor {
        screen: Entity<EditorScreen>,
        _events: Subscription,
    },
}

pub struct StudioRoot {
    screen: Screen,
    /// Kept while an editor is open so projects, section and in-flight state survive the task.
    dashboard: Option<Entity<DashboardScreen>>,
    /// Startup route (`--review-action-id` …); consumed by the first dashboard.
    route: Option<StartupRoute>,
    /// Watches lane audio of a task whose audio was downloaded; fails the load when none decodes.
    audio_guard: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl StudioRoot {
    pub fn new(route: StartupRoute, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let relay = ShellModels::relay(cx);
        let subscriptions = vec![cx.subscribe_in(&relay, window, |_, _, event, window, cx| {
            let note = match event {
                RelayEvent::Info(message) => Notification::info(message.clone()),
                RelayEvent::Error(message) => Notification::error(message.clone()),
            };
            window.push_notification(note, cx);
        })];
        let root = cx.entity().downgrade();
        let mut this = Self {
            screen: Screen::Auth(
                cx.new(|cx| AuthScreen::new(root.clone(), AuthFlow::login_prompt(), window, cx)),
            ),
            dashboard: None,
            route: (!route.is_empty()).then_some(route),
            audio_guard: None,
            _subscriptions: subscriptions,
        };
        if std::env::var_os("STUDIO_OPEN_DEMO_ON_START").is_some() {
            match studio_editor::load_sample_project() {
                Ok(project) => {
                    let workflow = StudioWorkflow::from_project(project);
                    this.open_task(workflow, demo_audio_dir(), window, cx);
                    return this;
                }
                Err(error) => eprintln!("demo project failed to load: {error:#}"),
            }
        }
        this.open_auth_gate(window, cx);
        this
    }

    /// Shows the dashboard when a sub-account session or Babel login exists, else sign-in.
    pub fn open_auth_gate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match subaccounts::read_default_session() {
            Ok(Some(_)) => return self.show_dashboard(window, cx),
            Ok(None) => {}
            Err(error) => {
                // The window root (notification layer) may not exist yet during startup.
                let note = Notification::warning(format!("Sub-account session ignored: {error:#}"));
                window.defer(cx, move |window, cx| window.push_notification(note, cx));
            }
        }
        match AuthFlow::startup() {
            AuthStartup::Authenticated => self.show_dashboard(window, cx),
            AuthStartup::Flow(flow) => self.show_auth(flow, window, cx),
        }
    }

    pub fn show_auth(&mut self, flow: AuthFlow, window: &mut Window, cx: &mut Context<Self>) {
        let root = cx.entity().downgrade();
        self.leave_editor(cx);
        self.dashboard = None;
        self.screen = Screen::Auth(cx.new(|cx| AuthScreen::new(root, flow, window, cx)));
        cx.notify();
    }

    /// Forgets the main Babel login (cookies, pending sign-in, token cache) and shows sign-in.
    /// Used for "Sign out" and to recover from an expired or broken session.
    pub fn sign_out(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = studio_services::auth::reset_store() {
            window.push_notification(
                Notification::error(format!("Could not clear the saved Babel login: {error:#}")),
                cx,
            );
            return;
        }
        self.show_auth(AuthFlow::login_prompt(), window, cx);
    }

    /// Shows the dashboard; a dashboard kept from before an editor session refreshes.
    pub fn show_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_editor(cx);
        match &self.dashboard {
            Some(dashboard) => {
                dashboard.update(cx, |dashboard, cx| dashboard.refresh(window, cx));
            }
            None => {
                let root = cx.entity().downgrade();
                let route = self.route.take();
                self.dashboard = Some(cx.new(|cx| DashboardScreen::new(root, route, window, cx)));
            }
        }
        self.screen = Screen::Dashboard;
        cx.notify();
    }

    /// Back on the dashboard with an error banner (failed task load after the editor opened).
    fn show_dashboard_error(&mut self, error: String, window: &mut Window, cx: &mut Context<Self>) {
        self.show_dashboard(window, cx);
        if let Some(dashboard) = &self.dashboard {
            dashboard.update(cx, |dashboard, cx| dashboard.show_error(error, cx));
        }
    }

    /// Opens a loaded task in the editor. `audio_base_dir` resolves lane audio files.
    pub fn open_task(
        &mut self,
        workflow: StudioWorkflow,
        audio_base_dir: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<EditorModel> {
        self.leave_editor(cx);
        let model = cx.new(|cx| EditorModel::new(workflow, audio_base_dir, cx));
        let screen = cx.new(|cx| EditorScreen::new(model.clone(), window, cx));
        let events = cx.subscribe_in(&screen, window, |this, _, event, window, cx| match event {
            EditorScreenEvent::Leave => this.show_dashboard(window, cx),
        });
        self.screen = Screen::Editor {
            screen,
            _events: events,
        };
        cx.notify();
        model
    }

    /// Opens a task fetched from Babel. When its audio had to be downloaded and no lane decodes,
    /// the load counts as failed (old behavior) and the dashboard shows `failure_label`.
    pub fn open_loaded_task(
        &mut self,
        task: LoadedTask,
        status: &'static str,
        failure_label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remote_audio = task.remote_audio_count > 0;
        let model = self.open_task(task.workflow, Some(task.audio_base_dir), window, cx);
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

    /// The dashboard, kept alive while a task is open.
    pub fn dashboard(&self) -> Option<&Entity<DashboardScreen>> {
        self.dashboard.as_ref()
    }

    /// The open editor model, if any (tray/exit handling).
    pub fn editor_model(&self, cx: &App) -> Option<Entity<EditorModel>> {
        match &self.screen {
            Screen::Editor { screen, .. } => Some(screen.read(cx).model().clone()),
            _ => None,
        }
    }

    /// Window close button: hide to the tray when there is one, otherwise exit. An open task
    /// with unsaved changes goes through the save / discard / cancel confirm first; hiding
    /// leaves the task (back to the dashboard) so its audio and PCM are released.
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !tray::is_active(cx) {
            return self.request_exit(window, cx);
        }
        let root = cx.entity().downgrade();
        self.confirm_leave(window, cx, move |window, cx| {
            root.update(cx, |root, cx| {
                if root.editor_model(cx).is_some() {
                    root.show_dashboard(window, cx);
                }
            })
            .ok();
            tray::hide_window(window);
        });
    }

    /// Tray "Exit" (or close without a tray): confirm unsaved changes, stop the relay, quit.
    pub fn request_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_leave(window, cx, |_, cx| {
            ShellModels::relay(cx).update(cx, |relay, _| relay.shutdown());
            cx.quit();
        });
    }

    fn confirm_leave(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        on_confirmed: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        match self.editor_model(cx) {
            Some(model) => panels::flows::request_leave(&model, window, cx, on_confirmed),
            // Deferred like the editor path: the callback updates this root again.
            None => window.defer(cx, on_confirmed),
        }
    }

    /// Releases audio and decoded PCM of the current editor before switching screens.
    fn leave_editor(&mut self, cx: &mut Context<Self>) {
        self.audio_guard = None;
        if let Some(model) = self.editor_model(cx) {
            model.update(cx, |model, _| model.unload());
        }
    }
}

impl Render for StudioRoot {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.screen {
            Screen::Auth(screen) => screen.clone().into_any_element(),
            Screen::Dashboard => match &self.dashboard {
                Some(dashboard) => dashboard.clone().into_any_element(),
                None => div().into_any_element(),
            },
            Screen::Editor { screen, .. } => screen.clone().into_any_element(),
        };
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(body)
    }
}

/// Two-speaker recording used by the demo task when running from a source checkout.
pub fn demo_audio_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../studio-babel/tests/fixtures/recovered/babel-fixture");
    dir.is_dir().then_some(dir)
}
