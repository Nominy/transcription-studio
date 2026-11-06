//! Document-scoped navigation; no authentication or platform policy lives in the shell.
//! Opening a mode is a journey: resolve requirements → make sure the mode's provider has a
//! ready instance (sign-in step, chooser) → launch. Providers render their own sign-in.
use crate::{
    ContributionRegistry, DocumentOrigin, ModeContribution, ModeLaunchContext,
    ProviderContribution, ProviderEnvironment, ProviderInstance, ProviderRegistry, ProviderStatus,
    editor::{EditorScreen, EditorScreenEvent, model::EditorModel},
    screens::{self, accounts, ui},
    tray,
};
use exaudio_extension::ModeLaunch;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme, WindowExt as _, h_flex, v_flex};
use gpui_kit::*;
use std::{path::PathBuf, rc::Rc};

pub type LeaveGuard = Rc<dyn Fn(&mut Window, &mut App, Box<dyn FnOnce(&mut Window, &mut App)>)>;
#[derive(Clone)]
pub struct ShellHandle(pub WeakEntity<StudioRoot>);
impl Global for ShellHandle {}
impl ShellHandle {
    pub fn from_root(root: WeakEntity<StudioRoot>) -> Self {
        Self(root)
    }
    pub fn editor_model(&self, cx: &App) -> Option<Entity<EditorModel>> {
        self.0.upgrade()?.read(cx).editor_model(cx)
    }
    pub fn focused_segment(&self, cx: &App) -> Option<String> {
        self.editor_model(cx)?
            .read(cx)
            .doc()
            .session()
            .selected_segment_id
            .clone()
    }
    pub fn set_leave_guard(&self, guard: Option<LeaveGuard>, cx: &mut App) {
        self.0.update(cx, |root, _| root.leave_guard = guard).ok();
    }
    pub fn show_home(&self, window: &mut Window, cx: &mut App) {
        self.0
            .update(cx, |root, cx| root.show_home(window, cx))
            .ok();
    }
    pub fn show_mode(&self, id: &str, window: &mut Window, cx: &mut App) {
        self.0
            .update(cx, |root, cx| root.show_mode(id, window, cx))
            .ok();
    }
    pub fn show_settings(&self, window: &mut Window, cx: &mut App) {
        self.0
            .update(cx, |root, cx| root.show_settings(window, cx))
            .ok();
    }
    pub fn show_extensions(&self, window: &mut Window, cx: &mut App) {
        self.0
            .update(cx, |root, cx| root.show_extensions(window, cx))
            .ok();
    }
    pub fn show_view(&self, view: AnyView, _window: &mut Window, cx: &mut App) {
        self.0
            .update(cx, |root, cx| {
                root.release_editor(cx);
                root.screen = Screen::Page(view);
                cx.notify();
            })
            .ok();
    }
    /// Runs a provider type's setup in a dialog over the current screen. `on_done` gets
    /// `Some(instance)` once the instance is stored in the [`ProviderRegistry`], `None`
    /// when the user cancels or the provider type is not installed. Called exactly once.
    pub fn request_provider_setup(
        &self,
        provider_id: &str,
        instance: Option<ProviderInstance>,
        on_done: crate::providers::ProviderSetupOutcome,
        window: &mut Window,
        cx: &mut App,
    ) {
        accounts::open_setup_dialog(
            self.clone(),
            provider_id,
            None,
            accounts::SetupRequest {
                instance,
                cancel_label: screens::ui::localized(cx, "Cancel", "Отмена"),
                on_done,
            },
            window,
            cx,
        );
    }
    /// Status of a configured instance as reported by its provider type.
    pub fn provider_status(&self, instance: &ProviderInstance, cx: &App) -> ProviderStatus {
        match cx
            .global::<ProviderRegistry>()
            .provider_type(&instance.provider)
        {
            Some(provider) => (provider.status)(instance, cx),
            None => ProviderStatus::Error(
                ui::localized(
                    cx,
                    "This account type isn't installed.",
                    "Этот тип аккаунта не установлен.",
                )
                .into(),
            ),
        }
    }
    pub fn open_document(
        &self,
        project: studio_core::Project,
        origin: DocumentOrigin,
        environment: ProviderEnvironment,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<Entity<EditorModel>> {
        let model = self.0.update(cx, |root, cx| {
            root.open_document(project, origin, environment, None, window, cx)
        })??;
        crate::wasm_host::document_opened(self.clone(), cx);
        let recovery = screens::recovery::start(&model, cx);
        model.update(cx, |model, _| model.own_recovery(recovery));
        Ok(model)
    }
    pub fn open_document_with_audio(
        &self,
        project: studio_core::Project,
        origin: DocumentOrigin,
        environment: ProviderEnvironment,
        audio: Vec<(String, studio_audio::LaneAudioFile)>,
        window: &mut Window,
        cx: &mut App,
    ) -> anyhow::Result<Entity<EditorModel>> {
        let model = self.0.update(cx, |root, cx| {
            root.open_document(project, origin, environment, Some(audio), window, cx)
        })??;
        crate::wasm_host::document_opened(self.clone(), cx);
        let recovery = screens::recovery::start(&model, cx);
        model.update(cx, |model, _| model.own_recovery(recovery));
        Ok(model)
    }
    pub fn dispatch_document_command(
        &self,
        command: &str,
        arguments: serde_json::Value,
        window: &mut Window,
        cx: &mut App,
    ) {
        let handler = cx
            .global::<ContributionRegistry>()
            .document_command(command);
        if let Some(handler) = handler {
            handler(self.clone(), arguments, window, cx);
        } else {
            window.push_notification(
                ui::error_notification(
                    ui::localized(
                        cx,
                        "This command isn't available.",
                        "Эта команда недоступна.",
                    ),
                    Some(command.to_string().into()),
                ),
                cx,
            );
        }
    }
    pub fn dispatch(&self, command: &str, window: &mut Window, cx: &mut App) {
        let handler = cx.global::<ContributionRegistry>().command(command);
        if let Some(handler) = handler {
            handler(self.clone(), window, cx);
        } else {
            window.push_notification(
                ui::error_notification(
                    ui::localized(
                        cx,
                        "This command isn't available.",
                        "Эта команда недоступна.",
                    ),
                    Some(command.to_string().into()),
                ),
                cx,
            );
        }
    }
}
enum Screen {
    Home,
    Page(AnyView),
    Editor {
        screen: Entity<EditorScreen>,
        model: Entity<EditorModel>,
        _events: Subscription,
    },
}
pub struct StudioRoot {
    screen: Screen,
    leave_guard: Option<LeaveGuard>,
    /// Mode whose launch window (or launched document) is current; the editor returns here.
    launched: Option<String>,
    _updater: Subscription,
}
impl StudioRoot {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let updater = cx.observe(&screens::ShellModels::updater(cx), |_, _, cx| cx.notify());
        Self {
            screen: Screen::Home,
            leave_guard: None,
            launched: None,
            _updater: updater,
        }
    }
    fn handle(&self, cx: &Context<Self>) -> ShellHandle {
        ShellHandle(cx.entity().downgrade())
    }
    pub fn editor_model(&self, _cx: &App) -> Option<Entity<EditorModel>> {
        match &self.screen {
            Screen::Editor { model, .. } => Some(model.clone()),
            _ => None,
        }
    }
    pub fn open_document(
        &mut self,
        project: studio_core::Project,
        origin: DocumentOrigin,
        mut environment: ProviderEnvironment,
        audio: Option<Vec<(String, studio_audio::LaneAudioFile)>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<Entity<EditorModel>> {
        if self.editor_model(cx).is_some_and(|m| {
            m.read(cx).doc().session().is_dirty || m.read(cx).doc().has_pending_drafts()
        }) {
            anyhow::bail!(
                "{}",
                ui::localized(
                    cx,
                    "Save or close the open project first.",
                    "Сначала сохраните или закройте открытый проект."
                )
            );
        }
        if !cx
            .global::<ContributionRegistry>()
            .modes()
            .iter()
            .any(|m| m.id == origin.mode_id)
        {
            return Err(
                anyhow::anyhow!("mode {} is not installed", origin.mode_id).context(ui::localized(
                    cx,
                    "This project's workspace isn't installed.",
                    "Рабочее пространство этого проекта не установлено.",
                )),
            );
        }
        cx.global_mut::<ContributionRegistry>()
            .enter_mode(&origin.mode_id)?;
        self.launched = Some(origin.mode_id.clone());
        environment.document.origin.mode = origin.mode_id.clone();
        self.release_editor(cx);
        crate::contributions::merge_providers(
            &mut environment.document.providers,
            &cx.global_mut::<ContributionRegistry>()
                .providers_for_document(&project, &origin.mode_id),
        );
        self.leave_guard = None;
        let model =
            cx.new(|cx| EditorModel::new_with_audio(project, origin, environment, audio, cx));
        let handle = self.handle(cx);
        let screen = cx.new(|cx| EditorScreen::new(model.clone(), handle, window, cx));
        let events = cx.subscribe_in(
            &screen,
            window,
            |root, _, _: &EditorScreenEvent, window, cx| {
                let mode = root
                    .editor_model(cx)
                    .map(|model| model.read(cx).origin().mode_id.clone())
                    .or_else(|| root.launched.clone());
                let registry = cx.global::<ContributionRegistry>();
                let returns_to_window = mode.as_deref().is_some_and(|id| {
                    registry.modes().iter().any(|m| m.id == id)
                        && registry
                            .mode_manifest(id)
                            .is_some_and(|manifest| manifest.launch == ModeLaunch::Window)
                });
                match mode.filter(|_| returns_to_window) {
                    Some(mode) => root.show_mode(&mode, window, cx),
                    None => root.show_home(window, cx),
                }
            },
        );
        self.screen = Screen::Editor {
            screen,
            model: model.clone(),
            _events: events,
        };
        cx.notify();
        Ok(model)
    }
    pub fn show_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.handle(cx);
        self.confirm_leave(window, cx, move |_, cx| {
            handle
                .0
                .update(cx, |root, cx| {
                    root.release_editor(cx);
                    root.leave_guard = None;
                    root.launched = None;
                    cx.global_mut::<ContributionRegistry>().leave_mode();
                    root.screen = Screen::Home;
                    cx.notify();
                })
                .ok();
        });
    }
    /// Opens a mode: requirements, then the provider step when the manifest declares one,
    /// then `launch`.
    pub fn show_mode(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_owned();
        let handle = self.handle(cx);
        self.confirm_leave(window, cx, move |window, cx| {
            let entered = cx.global_mut::<ContributionRegistry>().enter_mode(&id);
            let mode = match entered {
                Ok(mode) => mode,
                Err(error) => {
                    window.push_notification(ui::workspace_open_failed(&error, cx), cx);
                    return;
                }
            };
            handle
                .0
                .update(cx, |root, cx| {
                    root.release_editor(cx);
                    root.leave_guard = None;
                    root.launched = Some(id.clone());
                    root.screen = Screen::Home;
                    cx.notify();
                })
                .ok();
            let provider = cx
                .global::<ContributionRegistry>()
                .mode_manifest(&id)
                .and_then(|manifest| manifest.provider.clone());
            match provider {
                Some(provider) => resolve_instance(handle, mode, provider, window, cx),
                None => launch(handle, mode, None, window, cx),
            }
        });
    }
    pub fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.handle(cx);
        let view = cx.new(|cx| screens::settings::SettingsView::new(handle, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let width = (window.viewport_size().width - px(64.)).min(px(1120.));
            let height = (window.viewport_size().height - px(160.)).max(px(240.));
            dialog
                .title(screens::ui::localized(cx, "Settings", "Настройки"))
                .w(width)
                .child(div().h(height).min_h_0().child(view.clone()))
        });
    }
    pub fn show_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = self.handle(cx);
        let view = cx.new(|cx| screens::extensions::ExtensionsView::new(handle, window, cx));
        window.open_dialog(cx, move |dialog, window, cx| {
            let width = (window.viewport_size().width - px(64.)).min(px(1200.));
            let height = (window.viewport_size().height - px(160.)).max(px(240.));
            dialog
                .title(ui::localized(cx, "Extensions", "Расширения"))
                .w(width)
                .child(div().h(height).min_h_0().child(view.clone()))
        });
    }
    pub fn request_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !tray::is_active(cx) {
            return self.request_exit(window, cx);
        }
        let handle = self.handle(cx);
        self.confirm_leave(window, cx, move |window, cx| {
            handle
                .0
                .update(cx, |root, cx| {
                    root.release_editor(cx);
                    root.leave_guard = None;
                    root.launched = None;
                    root.screen = Screen::Home;
                    cx.global_mut::<ContributionRegistry>().leave_mode();
                    cx.notify();
                })
                .ok();
            tray::hide_window(window);
        });
    }
    pub fn request_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_leave(window, cx, |_, cx| cx.quit());
    }
    fn confirm_leave(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        if let Some(guard) = self.leave_guard.clone() {
            guard(
                window,
                cx,
                Box::new(move |window, cx| window.defer(cx, done)),
            );
        } else if let Some(model) = self.editor_model(cx) {
            let mode_id = model.read(cx).origin().mode_id.clone();
            let handler = cx.global::<ContributionRegistry>().leave_handler(&mode_id);
            if let Some(handler) = handler {
                handler(
                    model,
                    window,
                    cx,
                    Box::new(move |window, cx| window.defer(cx, done)),
                );
            } else {
                screens::local_files::confirm_leave(&model, window, cx, done);
            }
        } else {
            window.defer(cx, done);
        }
    }
    fn release_editor(&mut self, cx: &mut App) {
        if let Some(model) = self.editor_model(cx) {
            crate::wasm_host::document_closed(cx);
            cx.global_mut::<ContributionRegistry>()
                .clear_document_providers();
            model.update(cx, |m, _| m.unload());
        }
    }
    /// Leaves a page-less state after a launch that produced no window and no document.
    fn settle_after_launch(&mut self, cx: &mut Context<Self>) {
        if self.editor_model(cx).is_some() {
            return;
        }
        if matches!(self.screen, Screen::Page(_)) {
            self.screen = Screen::Home;
        }
        if matches!(self.screen, Screen::Home) {
            self.launched = None;
            cx.global_mut::<ContributionRegistry>().leave_mode();
        }
        cx.notify();
    }
}

/// Picks the provider instance a mode launches with, running sign-in or the chooser when
/// needed, then launches.
fn resolve_instance(
    handle: ShellHandle,
    mode: ModeContribution,
    provider_id: String,
    window: &mut Window,
    cx: &mut App,
) {
    let registry = cx.global::<ProviderRegistry>();
    let Some(provider) = registry.provider_type(&provider_id).cloned() else {
        window.push_notification(
            ui::error_notification(
                ui::localized(cx, "Couldn't open {mode}.", "Не удалось открыть {mode}.")
                    .replace("{mode}", &mode.label),
                Some(format!("account type {provider_id} is not installed").into()),
            ),
            cx,
        );
        return;
    };
    let instances = registry.instances(&provider_id);
    if instances.is_empty() {
        return show_setup(handle, mode, provider, None, window, cx);
    }
    let Some(instance) = registry.selected(&provider_id) else {
        let title = ui::localized(cx, "Which account?", "Какой аккаунт?");
        let pick_handle = handle.clone();
        let pick_mode = mode.clone();
        let pick_provider = provider_id.clone();
        let add_handle = handle.clone();
        let add_mode = mode.clone();
        let add_provider = provider.clone();
        let chooser = cx.new(|cx| {
            accounts::Chooser::new(
                title,
                &provider,
                move |instance, window, cx| {
                    cx.update_global::<ProviderRegistry, _>(|registry, cx| {
                        registry.select(&instance, cx)
                    });
                    resolve_instance(
                        pick_handle.clone(),
                        pick_mode.clone(),
                        pick_provider.clone(),
                        window,
                        cx,
                    );
                },
                provider.multiple.then(|| {
                    Rc::new(move |window: &mut Window, cx: &mut App| {
                        show_setup(
                            add_handle.clone(),
                            add_mode.clone(),
                            add_provider.clone(),
                            None,
                            window,
                            cx,
                        )
                    }) as Rc<dyn Fn(&mut Window, &mut App)>
                }),
                cx,
            )
        });
        handle.show_view(chooser.into(), window, cx);
        return;
    };
    match (provider.status)(&instance, cx) {
        ProviderStatus::Ready { .. } => launch(handle, mode, Some(instance), window, cx),
        ProviderStatus::NeedsSignIn => {
            show_setup(handle, mode, provider, Some(instance), window, cx)
        }
        ProviderStatus::Error(error) => {
            window.push_notification(
                ui::error_notification(
                    ui::localized(
                        cx,
                        "Couldn't check the sign-in.",
                        "Не удалось проверить вход.",
                    ),
                    Some(error.into()),
                ),
                cx,
            );
            show_setup(handle, mode, provider, Some(instance), window, cx)
        }
    }
}

/// The sign-in step of opening `mode`: a full page titled "Sign in to {mode}". Done
/// continues the launch; cancel returns to Start.
fn show_setup(
    handle: ShellHandle,
    mode: ModeContribution,
    provider: ProviderContribution,
    instance: Option<ProviderInstance>,
    window: &mut Window,
    cx: &mut App,
) {
    let title =
        ui::localized(cx, "Sign in to {mode}", "Вход: {mode}").replace("{mode}", &mode.label);
    let done_handle = handle.clone();
    let on_done: accounts::SetupOutcome = Box::new(move |instance, window, cx| match instance {
        Some(instance) => launch(done_handle, mode, Some(instance), window, cx),
        None => done_handle.show_home(window, cx),
    });
    let page = accounts::SetupPage::new(
        handle.clone(),
        &provider,
        title,
        accounts::SetupRequest {
            instance,
            cancel_label: screens::ui::localized(cx, "Cancel", "Отмена"),
            on_done,
        },
        window,
        cx,
    );
    handle.show_view(page.into(), window, cx);
}

/// Invokes the mode's launch factory. A returned view becomes the launch window; `None`
/// means the mode opened a document itself (or declined), so the shell settles.
fn launch(
    handle: ShellHandle,
    mode: ModeContribution,
    instance: Option<ProviderInstance>,
    window: &mut Window,
    cx: &mut App,
) {
    let context = ModeLaunchContext {
        mode_id: mode.id.clone(),
        instance,
    };
    match (mode.launch)(handle.clone(), context, window, cx) {
        Some(view) => handle.show_view(view, window, cx),
        None => {
            handle
                .0
                .update(cx, |root, cx| root.settle_after_launch(cx))
                .ok();
        }
    }
}

impl Render for StudioRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.screen {
            Screen::Home => screens::home::render(self.handle(cx), window, cx).into_any_element(),
            Screen::Page(view) => {
                let home = self.handle(cx);
                let settings = home.clone();
                let extensions = home.clone();
                v_flex()
                    .size_full()
                    .child(
                        h_flex()
                            .px_4()
                            .py_2()
                            .gap_2()
                            .flex_none()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(
                                Button::new("mode-home")
                                    .label(screens::ui::localized(cx, "Start", "Начало"))
                                    .on_click(move |_, window, cx| home.show_home(window, cx)),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("mode-settings")
                                    .label(screens::ui::localized(cx, "Settings", "Настройки"))
                                    .on_click(move |_, window, cx| {
                                        settings.show_settings(window, cx)
                                    }),
                            )
                            .child(
                                Button::new("mode-extensions")
                                    .label(ui::localized(cx, "Extensions", "Расширения"))
                                    .on_click(move |_, window, cx| {
                                        extensions.show_extensions(window, cx)
                                    }),
                            ),
                    )
                    .child(div().flex_1().min_h_0().child(view.clone()))
                    .into_any_element()
            }
            Screen::Editor { screen, .. } => screen.clone().into_any_element(),
        };
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(body)
    }
}
fn demo_audio_dir() -> Option<PathBuf> {
    let complete = |path: &std::path::Path| {
        ["speaker_1.wav", "speaker_2.wav"]
            .iter()
            .all(|name| path.join(name).is_file())
    };
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let installed = parent.join("assets/demo");
        if complete(&installed) {
            return Some(installed);
        }
    }
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../studio-editor/assets/demo");
    complete(&source).then_some(source)
}

pub fn open_demo(shell: &ShellHandle, window: &mut Window, cx: &mut App) -> anyhow::Result<()> {
    let project = studio_editor::load_sample_project()?;
    let audio = demo_audio_dir().ok_or_else(|| {
        anyhow::anyhow!(
            "{}",
            ui::localized(
                cx,
                "The demo audio is missing from this installation.",
                "В этой установке нет демо-аудио."
            )
        )
    })?;
    let environment = ProviderEnvironment::local(project.id.clone(), Some(audio));
    shell.open_document(
        project,
        DocumentOrigin::local(None),
        environment,
        window,
        cx,
    )?;
    Ok(())
}
