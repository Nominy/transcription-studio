//! Document-scoped navigation; no authentication or platform policy lives in the shell.
use crate::{
    ContributionRegistry, DocumentOrigin, ProviderEnvironment,
    editor::{EditorScreen, EditorScreenEvent, model::EditorModel},
    screens, tray,
};
use gpui_kit::component::button::Button;
use gpui_kit::component::notification::Notification;
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
                Notification::error(format!("Document command unavailable: {command}")),
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
                Notification::error(format!("Command unavailable: {command}")),
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
    _updater: Subscription,
}
impl StudioRoot {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let updater = cx.observe(&screens::ShellModels::updater(cx), |_, _, cx| cx.notify());
        Self {
            screen: Screen::Home,
            leave_guard: None,
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
            anyhow::bail!("Save or close the current document before opening another");
        }
        if cx
            .global::<ContributionRegistry>()
            .modes()
            .iter()
            .any(|m| m.id == origin.mode_id)
        {
            cx.global_mut::<ContributionRegistry>()
                .enter_mode(&origin.mode_id)?;
        } else if origin.provider_id.is_some() {
            anyhow::bail!("The document's mode is not installed: {}", origin.mode_id);
        } else {
            cx.global_mut::<ContributionRegistry>().leave_mode();
        }
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
                    .map(|model| model.read(cx).origin().mode_id.clone());
                if let Some(mode) = mode.filter(|id| {
                    cx.global::<ContributionRegistry>()
                        .modes()
                        .iter()
                        .any(|m| &m.id == id)
                }) {
                    root.show_mode(&mode, window, cx);
                } else {
                    root.show_home(window, cx);
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
                    cx.global_mut::<ContributionRegistry>().leave_mode();
                    root.screen = Screen::Home;
                    cx.notify();
                })
                .ok();
        });
    }
    pub fn show_mode(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let id = id.to_owned();
        let handle = self.handle(cx);
        self.confirm_leave(window, cx, move |window, cx| {
            let resolved = cx.global_mut::<ContributionRegistry>().enter_mode(&id);
            match resolved {
                Ok(mode) => {
                    handle
                        .0
                        .update(cx, |root, cx| {
                            root.release_editor(cx);
                            root.leave_guard = None;
                            root.screen = Screen::Home;
                        })
                        .ok();
                    let view = (mode.home)(handle.clone(), window, cx);
                    handle.show_view(view, window, cx);
                }
                Err(error) => window.push_notification(Notification::error(error.to_string()), cx),
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
                .title(screens::ui::localized(cx, "Extensions", "Расширения"))
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
            let handler = model
                .read(cx)
                .origin()
                .provider_id
                .as_deref()
                .and_then(|provider| cx.global::<ContributionRegistry>().leave_handler(provider));
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
                                    .label(screens::ui::localized(cx, "Home", "Главная"))
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
                                    .label(screens::ui::localized(cx, "Extensions", "Расширения"))
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
    let audio = demo_audio_dir()
        .ok_or_else(|| anyhow::anyhow!("Demo audio is missing from this installation."))?;
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
