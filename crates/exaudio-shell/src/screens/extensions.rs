//! Global extension management. The app-owned controller survives navigation so a
//! package transaction cannot be abandoned between durable installation and activation.
mod model;

use super::{
    accounts,
    ui::{self, localized as text},
};
use crate::{ProviderRegistry, ShellHandle, contributions::ContributionRegistry};
use exaudio_extension::ExtensionManifest;
use exaudio_marketplace::InstallProvenance;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{Disableable, IconName, Selectable, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use model::{DEFAULT_INDEX, ExtensionsEvent, ExtensionsModel, Preview, Removal};
use std::collections::BTreeMap;

/// Call after registering bundled native manifests. Restores approved managed WASM
/// without requiring a marketplace connection or opening this page.
pub fn startup(shell: ShellHandle, cx: &mut App) {
    let _ = model::shared(shell, cx);
}

pub struct ExtensionsView {
    shell: ShellHandle,
    model: Entity<ExtensionsModel>,
    search: Entity<InputState>,
    url: Entity<InputState>,
    index: Entity<InputState>,
    selected: Option<String>,
    advanced: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// After a catalog install, offers sign-in for every provider type that a newly active
/// extension's modes need and that has no configured instance yet. Providers are offered
/// one after another; "Later" skips to the next.
fn offer_provider_setup(shell: ShellHandle, owners: &[String], window: &mut Window, cx: &mut App) {
    let registry = cx.global::<ContributionRegistry>();
    let providers = cx.global::<ProviderRegistry>();
    let mut pending: Vec<(String, String)> = Vec::new();
    for manifest in registry
        .manifests()
        .iter()
        .filter(|manifest| owners.contains(&manifest.id))
    {
        for mode in &manifest.modes {
            let Some(provider) = &mode.provider else {
                continue;
            };
            if providers.provider_type(provider).is_some()
                && providers.instances(provider).is_empty()
                && !pending.iter().any(|(id, _)| id == provider)
            {
                pending.push((provider.clone(), mode.name.clone()));
            }
        }
    }
    pending.reverse();
    offer_next(shell, pending, window, cx);
}

fn offer_next(
    shell: ShellHandle,
    mut pending: Vec<(String, String)>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some((provider, mode_label)) = pending.pop() else {
        return;
    };
    let title = text(cx, "Sign in to {mode}", "Вход: {mode}").replace("{mode}", &mode_label);
    let next_shell = shell.clone();
    accounts::open_setup_dialog(
        shell,
        &provider,
        Some(title),
        accounts::SetupRequest {
            instance: None,
            cancel_label: text(cx, "Later", "Позже"),
            on_done: Box::new(move |_, window, cx| offer_next(next_shell, pending, window, cx)),
        },
        window,
        cx,
    );
}

/// Where an extension comes from, for the list row and the details card.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    BuiltIn,
    Installed,
    Developer,
    Catalog,
}

impl Source {
    fn label(self, cx: &App) -> &'static str {
        match self {
            Self::BuiltIn => text(cx, "Built in", "Встроено"),
            Self::Installed => text(cx, "Installed", "Установлено"),
            Self::Developer => text(cx, "Developer", "Разработка"),
            Self::Catalog => text(cx, "Available", "Доступно"),
        }
    }
}

impl ExtensionsView {
    pub fn new(shell: ShellHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let model = model::shared(shell.clone(), cx);
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(text(
                cx,
                "Search extensions",
                "Поиск расширений",
            ))
        });
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("https://…/*.exaudio"));
        let source = model.read(cx).index.clone();
        let index = cx.new(|cx| InputState::new(window, cx).default_value(source));
        let subscriptions = vec![
            cx.observe(&model, |_, _, cx| cx.notify()),
            cx.subscribe_in(&model, window, |this, _, event, window, cx| {
                let ExtensionsEvent::Activated(owners) = event;
                let shell = this.shell.clone();
                let owners = owners.clone();
                window.defer(cx, move |window, cx| {
                    offer_provider_setup(shell, &owners, window, cx)
                });
            }),
            cx.subscribe(&search, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&url, window, |this, _, event, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.preview_url(cx);
                }
            }),
        ];
        let focus = cx.focus_handle();
        search.update(cx, |input, cx| input.focus(window, cx));
        Self {
            shell,
            model,
            search,
            url,
            index,
            selected: None,
            advanced: false,
            focus,
            _subscriptions: subscriptions,
        }
    }

    fn preview_url(&mut self, cx: &mut Context<Self>) {
        let url = self.url.read(cx).value().trim().to_owned();
        self.model
            .update(cx, |model, cx| model.preview_url(&url, cx));
    }

    fn choose(&mut self, directory: bool, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = if directory {
            text(cx, "Open developer folder", "Открыть папку разработчика")
        } else {
            text(cx, "Install from file", "Установить из файла")
        };
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        let model = self.model.clone();
        window
            .spawn(cx, async move |cx| match picked.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        cx.update(|_, cx| {
                            model.update(cx, |model, cx| model.preview_path(path, directory, cx))
                        })
                        .ok();
                    }
                }
                Ok(Err(error)) => {
                    cx.update(|_, cx| {
                        model.update(cx, |model, cx| {
                            model.fail(
                                text(
                                    cx,
                                    "Couldn't open the file picker.",
                                    "Не удалось открыть выбор файла.",
                                )
                                .into(),
                                Some(error.to_string()),
                                cx,
                            )
                        })
                    })
                    .ok();
                }
                _ => {}
            })
            .detach();
    }

    fn save_index(&mut self, cx: &mut Context<Self>) {
        let source = self.index.read(cx).value().trim().to_owned();
        self.model
            .update(cx, |model, cx| model.set_index(source, cx));
    }

    fn inventory(&self, cx: &App) -> BTreeMap<String, ExtensionManifest> {
        let model = self.model.read(cx);
        let mut manifests: BTreeMap<_, _> = cx
            .global::<ContributionRegistry>()
            .manifests()
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect();
        for installed in model
            .installed
            .active
            .values()
            .chain(model.installed.disabled.values())
        {
            manifests.insert(installed.manifest.id.clone(), installed.manifest.clone());
        }
        for (id, (_, manifest)) in &model.developer_folders {
            manifests
                .entry(id.clone())
                .or_insert_with(|| manifest.clone());
        }
        if let Some(catalog) = &model.catalog {
            for entry in &catalog.extensions {
                if let Some(release) = entry
                    .releases
                    .iter()
                    .filter(|r| {
                        r.withdrawn.is_none() && r.manifest.check_host_compatibility().is_ok()
                    })
                    .max_by(|a, b| a.version.cmp(&b.version))
                {
                    manifests
                        .entry(entry.id.clone())
                        .or_insert_with(|| release.manifest.clone());
                } else if let Some(release) = entry
                    .releases
                    .iter()
                    .max_by(|a, b| a.version.cmp(&b.version))
                {
                    manifests
                        .entry(entry.id.clone())
                        .or_insert_with(|| release.manifest.clone());
                }
            }
        }
        manifests
    }

    /// Where `id` comes from, and the developer folder when it is one.
    fn source(&self, id: &str, cx: &App) -> (Source, Option<std::path::PathBuf>) {
        let model = self.model.read(cx);
        let registry = cx.global::<ContributionRegistry>();
        let installed = model
            .installed
            .active
            .get(id)
            .or_else(|| model.installed.disabled.get(id));
        let dev = model
            .developer_folders
            .get(id)
            .map(|(directory, _)| directory.clone())
            .or_else(|| {
                crate::wasm_host::status(cx)
                    .into_iter()
                    .find(|status| status.owner == id)
                    .and_then(|status| status.development_directory)
            });
        if installed.is_none()
            && registry
                .manifests()
                .iter()
                .any(|m| m.id == id && m.runtime == exaudio_extension::Runtime::Native)
        {
            return (Source::BuiltIn, None);
        }
        if dev.is_some() {
            return (Source::Developer, dev);
        }
        match installed.map(|installed| &installed.provenance) {
            Some(InstallProvenance::Reviewed { .. }) => (Source::Installed, None),
            Some(InstallProvenance::Developer { source }) => {
                (Source::Developer, Some(source.clone()))
            }
            None => (Source::Catalog, None),
        }
    }

    fn details(&self, manifest: &ExtensionManifest, cx: &Context<Self>) -> Div {
        let model = self.model.read(cx);
        let registry = cx.global::<ContributionRegistry>();
        let installed = model
            .installed
            .active
            .get(&manifest.id)
            .or_else(|| model.installed.disabled.get(&manifest.id));
        let (source, dev) = self.source(&manifest.id, cx);
        let bundled = source == Source::BuiltIn;
        let entry = model.catalog.as_ref().and_then(|catalog| {
            catalog
                .extensions
                .iter()
                .find(|entry| entry.id == manifest.id)
        });
        let latest = entry
            .and_then(|entry| {
                entry
                    .releases
                    .iter()
                    .filter(|r| {
                        r.withdrawn.is_none() && r.manifest.check_host_compatibility().is_ok()
                    })
                    .max_by(|a, b| a.version.cmp(&b.version))
            })
            .cloned();
        let required = registry
            .required_by()
            .get(&manifest.id)
            .is_some_and(|reasons| !reasons.is_empty());
        let busy = model.busy;
        let active = registry.is_enabled(&manifest.id);
        let id = manifest.id.clone();
        let mut card = ui::card(manifest.name.clone(), Some(&manifest.description), cx)
            .child(
                ui::detail_row(text(cx, "From", "Откуда"), source.label(cx), cx).when_some(
                    dev.clone(),
                    |this, path| {
                        this.child(
                            ui::muted(path.display().to_string(), cx)
                                .min_w_0()
                                .truncate(),
                        )
                    },
                ),
            )
            .child(ui::detail_row(
                text(cx, "Status", "Состояние"),
                if active {
                    text(cx, "On", "Включено")
                } else {
                    text(cx, "Off", "Выключено")
                },
                cx,
            ))
            .child(ui::detail_row(
                text(cx, "Version", "Версия"),
                manifest.version.to_string(),
                cx,
            ))
            .child(ui::detail_row(
                text(cx, "License", "Лицензия"),
                manifest.license.clone(),
                cx,
            ));
        if required {
            card = card.child(ui::muted(
                text(
                    cx,
                    "Needed by the open workspace",
                    "Нужно открытому пространству",
                ),
                cx,
            ));
        }
        if !bundled {
            card = card.child(ui::detail_row(
                text(cx, "Can access", "Доступ"),
                if model.global_access_for(&manifest.id) {
                    text(
                        cx,
                        "All projects (this session)",
                        "Все проекты (в этой сессии)",
                    )
                } else {
                    text(cx, "Its own workspace only", "Только своё пространство")
                },
                cx,
            ));
        }
        if let Some(entry) = entry {
            let repo = format!(
                "https://github.com/{}/{}",
                entry.repository.owner, entry.repository.name
            );
            let author = if source == Source::Developer {
                text(cx, "{author} (not verified)", "{author} (не проверено)")
                    .replace("{author}", &entry.publisher)
            } else {
                entry.publisher.clone()
            };
            card = card
                .child(ui::detail_row(text(cx, "Author", "Автор"), author, cx))
                .child(
                    Button::new("extension-repository")
                        .label(text(cx, "Open on GitHub", "Открыть на GitHub"))
                        .tooltip(repo.clone())
                        .small()
                        .on_click(move |_, _, cx| cx.open_url(&repo)),
                );
        } else if let Some(InstallProvenance::Reviewed { release }) =
            installed.map(|installed| &installed.provenance)
        {
            let url = release.asset.url.clone();
            card = card.child(
                Button::new("extension-original-release")
                    .label(text(cx, "Release page", "Страница релиза"))
                    .tooltip(url.clone())
                    .small()
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        } else {
            card = card.child(ui::detail_row(
                text(cx, "Author", "Автор"),
                if bundled {
                    "Exaudio"
                } else {
                    text(cx, "Unknown", "Неизвестно")
                },
                cx,
            ));
        }
        if let Err(error) = manifest.check_host_compatibility() {
            card = card.child(ui::banner(
                ui::Tone::Warning,
                text(
                    cx,
                    "Not compatible with this version of Exaudio",
                    "Несовместимо с этой версией Exaudio",
                ),
                Some(error.to_string().into()),
                None,
                cx,
            ));
        }
        if latest.is_none() && source == Source::Catalog {
            card = card.child(ui::muted(
                text(
                    cx,
                    "No compatible version in the catalog.",
                    "В каталоге нет совместимой версии.",
                ),
                cx,
            ));
        }
        if manifest.permissions.is_empty() {
            card = card.child(ui::muted(
                text(
                    cx,
                    "This extension needs no permissions.",
                    "Этому расширению не нужны разрешения.",
                ),
                cx,
            ));
        } else {
            card = card.child(ui::caption(
                text(cx, "This extension can:", "Это расширение может:"),
                cx,
            ));
            for permission in &manifest.permissions {
                card = card.child(ui::muted(model::permission_label(permission, cx), cx));
            }
        }
        card = card.child(ui::caption(text(cx, "Also needs", "Также нужно"), cx));
        if manifest.dependencies.is_empty() {
            card = card.child(ui::muted(text(cx, "nothing", "ничего"), cx));
        }
        for (dependency, requirement) in &manifest.dependencies {
            let name = model::extension_name(dependency, model.catalog.as_ref(), cx);
            card = card.child(ui::muted(format!("{name} {requirement}"), cx));
        }
        let mut controls = h_flex().gap_2().flex_wrap();
        if let Some(release) = latest.filter(|release| {
            !bundled
                && dev.is_none()
                && installed.is_none_or(|old| {
                    old.manifest.version != release.version
                        || matches!(&old.provenance, InstallProvenance::Developer { .. })
                })
        }) {
            controls = controls.child(
                Button::new("extension-install")
                    .primary()
                    .label(if installed.is_some() {
                        text(cx, "Update…", "Обновить…")
                    } else {
                        text(cx, "Install…", "Установить…")
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model
                            .update(cx, |model, cx| model.preview_release(release.clone(), cx))
                    })),
            );
        }
        if installed.is_some() || bundled {
            let target = id.clone();
            let removal = if active {
                Removal::Disable
            } else {
                Removal::Enable
            };
            controls = controls.child(
                Button::new("extension-toggle")
                    .label(if active {
                        text(cx, "Turn off", "Выключить")
                    } else {
                        text(cx, "Turn on", "Включить")
                    })
                    .disabled(busy || required)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.preview_removal(target.clone(), removal, cx)
                        })
                    })),
            );
        }
        if installed.is_some() {
            let target = id.clone();
            controls = controls.child(
                Button::new("extension-uninstall")
                    .label(text(cx, "Remove", "Удалить"))
                    .disabled(busy || required)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.preview_removal(target.clone(), Removal::Uninstall, cx)
                        })
                    })),
            );
        }
        if let Some(directory) = dev.filter(|_| source == Source::Developer && installed.is_none())
        {
            controls = controls.child(
                Button::new("extension-reload")
                    .label(text(cx, "Reload", "Перезагрузить"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.preview_path(directory.clone(), true, cx)
                        })
                    })),
            );
            let target = id.clone();
            controls = controls.child(
                Button::new("extension-stop-development")
                    .label(text(cx, "Stop", "Остановить"))
                    .disabled(
                        busy || required || (!active && !model.developer_folders.contains_key(&id)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model
                            .update(cx, |model, cx| model.stop_development(&target, cx))
                    })),
            );
        }
        if bundled {
            card = card.child(ui::muted(
                text(
                    cx,
                    "Built-in extensions can be turned off but not removed.",
                    "Встроенные расширения можно выключить, но не удалить.",
                ),
                cx,
            ));
        }
        if required {
            card = card.child(ui::muted(
                text(
                    cx,
                    "Close the open project first.",
                    "Сначала закройте открытый проект.",
                ),
                cx,
            ));
        }
        card.child(controls)
    }

    fn consent(&self, preview: &Preview, cx: &Context<Self>) -> Div {
        let model = self.model.read(cx);
        let catalog = model.catalog.as_ref();
        let installed = |id: &str| {
            model.installed.active.contains_key(id) || model.installed.disabled.contains_key(id)
        };
        let (title, verb) = match preview {
            Preview::Install {
                owner,
                developer,
                plan,
                ..
            } => {
                let name = developer
                    .as_ref()
                    .map(|source| source.manifest.name.clone())
                    .or_else(|| {
                        plan.releases
                            .iter()
                            .find(|release| &release.manifest.id == owner)
                            .map(|release| release.manifest.name.clone())
                    })
                    .unwrap_or_else(|| model::extension_name(owner, catalog, cx));
                if installed(owner) {
                    (
                        text(cx, "Update {name}?", "Обновить {name}?").replace("{name}", &name),
                        text(cx, "Update", "Обновить"),
                    )
                } else {
                    (
                        text(cx, "Install {name}?", "Установить {name}?").replace("{name}", &name),
                        text(cx, "Install", "Установить"),
                    )
                }
            }
            Preview::Folder { manifest, .. } => (
                text(cx, "Install {name}?", "Установить {name}?").replace("{name}", &manifest.name),
                text(cx, "Install", "Установить"),
            ),
            Preview::Removal { name, action, .. } => match action {
                Removal::Enable => (
                    text(cx, "Turn on {name}?", "Включить {name}?").replace("{name}", name),
                    text(cx, "Turn on", "Включить"),
                ),
                Removal::Disable => (
                    text(cx, "Turn off {name}?", "Выключить {name}?").replace("{name}", name),
                    text(cx, "Turn off", "Выключить"),
                ),
                Removal::Uninstall => (
                    text(cx, "Remove {name}?", "Удалить {name}?").replace("{name}", name),
                    text(cx, "Remove", "Удалить"),
                ),
            },
            Preview::Rollback { .. } => (
                text(
                    cx,
                    "Go back to the previous set of extensions?",
                    "Вернуть предыдущий набор расширений?",
                )
                .to_owned(),
                text(cx, "Go back", "Вернуть"),
            ),
        };
        let mut card = ui::card(title, None, cx);
        match preview {
            Preview::Install {
                plan, developer, ..
            } => {
                if let Some(source) = developer {
                    let detail = format!(
                        "{}\n{} bytes\nSHA-256 {}",
                        source.source.display(),
                        source.bytes,
                        source.sha256
                    );
                    card = card.child(
                        v_flex().gap_1().child(ui::banner(
                            ui::Tone::Warning,
                            text(
                                cx,
                                "This file isn't from the catalog. Only install extensions you trust.",
                                "Этот файл не из каталога. Устанавливайте только расширения, которым доверяете.",
                            ),
                            None,
                            None,
                            cx,
                        ))
                        .child(ui::details(detail.into(), cx)),
                    );
                    card = card.child(ui::muted(
                        format!("{} {}", source.manifest.name, source.manifest.version),
                        cx,
                    ));
                }
                for release in &plan.releases {
                    card = card.child(ui::muted(
                        format!("{} {}", release.manifest.name, release.manifest.version),
                        cx,
                    ));
                }
                if plan.requested_permissions.values().any(|p| !p.is_empty()) {
                    card = card.child(ui::caption(
                        text(cx, "It will be able to:", "Оно сможет:"),
                        cx,
                    ));
                    for (id, permissions) in &plan.requested_permissions {
                        if plan.requested_permissions.len() > 1 {
                            card =
                                card.child(ui::muted(model::extension_name(id, catalog, cx), cx));
                        }
                        for permission in permissions {
                            card =
                                card.child(ui::muted(model::permission_label(permission, cx), cx));
                        }
                    }
                }
            }
            Preview::Folder {
                directory,
                manifest,
                ..
            } => {
                card = card
                    .child(ui::muted(
                        text(
                            cx,
                            "Developer folder. Changes you make in it are loaded automatically.",
                            "Папка разработчика. Изменения в ней подхватываются автоматически.",
                        ),
                        cx,
                    ))
                    .child(ui::muted(directory.display().to_string(), cx))
                    .child(ui::muted(
                        format!("{} {}", manifest.name, manifest.version),
                        cx,
                    ));
                if !manifest.permissions.is_empty() {
                    card = card.child(ui::caption(
                        text(cx, "It will be able to:", "Оно сможет:"),
                        cx,
                    ));
                    for permission in &manifest.permissions {
                        card = card.child(ui::muted(model::permission_label(permission, cx), cx));
                    }
                }
            }
            Preview::Removal {
                name,
                version,
                action,
                ..
            } => {
                card = card.child(ui::muted(format!("{name} {version}"), cx));
                if !matches!(action, Removal::Enable) {
                    card = card.child(ui::muted(
                        text(
                            cx,
                            "Its panels and commands go away.",
                            "Его панели и команды исчезнут.",
                        ),
                        cx,
                    ));
                }
            }
            Preview::Rollback { lines, .. } => {
                for line in lines {
                    card = card.child(ui::muted(line.clone(), cx));
                }
            }
        }
        for blocker in preview.blockers() {
            card = card.child(ui::error_block(
                text(
                    cx,
                    "Can't continue: {sentence}",
                    "Нельзя продолжить: {sentence}",
                )
                .replace("{sentence}", blocker),
                None,
                cx,
            ));
        }
        if model.document_scope_available() {
            let global_access = model.global_document_access;
            card = card.child(
                Switch::new("extension-global-document-consent")
                    .checked(global_access)
                    .disabled(model.busy)
                    .label(text(
                        cx,
                        "Allow access to all projects until I quit Exaudio",
                        "Разрешить доступ ко всем проектам до выхода из Exaudio",
                    ))
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        let checked = *checked;
                        this.model.update(cx, |model, cx| {
                            model.global_document_access = checked;
                            cx.notify();
                        })
                    })),
            );
        }
        card.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("approve-extension-plan")
                        .primary()
                        .label(verb)
                        .disabled(model.busy || !preview.can_apply())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.update(cx, |model, cx| model.approve(cx))
                        })),
                )
                .child(
                    Button::new("dismiss-extension-plan")
                        .label(text(cx, "Cancel", "Отмена"))
                        .disabled(model.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.update(cx, |model, cx| {
                                model.preview = None;
                                cx.notify();
                            })
                        })),
                ),
        )
    }

    fn advanced(&self, cx: &Context<Self>) -> Div {
        let model = self.model.read(cx);
        let busy = model.busy;
        let offline = model.offline;
        let can_rollback = model.installed.previous.is_some();
        let logs = model.logs.clone();
        let mut runtime_logs: Vec<String> = Vec::new();
        for status in crate::wasm_host::status(cx) {
            let name = model::extension_name(&status.owner, model.catalog.as_ref(), cx);
            runtime_logs.push(format!("{name}  {} · {}", status.state, status.last_event));
            runtime_logs.extend(status.logs.iter().map(|line| format!("{name}  {line}")));
        }
        runtime_logs.extend(cx.global::<ContributionRegistry>().errors.iter().map(
            |(owner, error)| {
                format!(
                    "{}  {error}",
                    model::extension_name(owner, model.catalog.as_ref(), cx)
                )
            },
        ));
        v_flex()
            .gap_4()
            .child(
                ui::card(text(cx, "Install", "Установка"), None, cx)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.url)))
                            .child(
                                Button::new("review-extension-url")
                                    .label(text(cx, "Install from link…", "Установить по ссылке…"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.preview_url(cx))),
                            ),
                    )
                    .child(ui::muted(
                        text(
                            cx,
                            "Only links listed in the catalog are accepted.",
                            "Принимаются только ссылки из каталога.",
                        ),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("review-extension-file")
                                    .label(text(cx, "Install from file…", "Установить из файла…"))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose(false, window, cx)
                                    })),
                            )
                            .child(
                                Button::new("review-extension-folder")
                                    .label(text(
                                        cx,
                                        "Open developer folder…",
                                        "Открыть папку разработчика…",
                                    ))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.choose(true, window, cx)
                                    })),
                            ),
                    )
                    .child(ui::muted(
                        text(
                            cx,
                            "Changes in a developer folder are reloaded automatically. If a reload fails, the last working version keeps running.",
                            "Изменения в папке разработчика подхватываются автоматически. Если перезагрузка не удалась, продолжает работать последняя рабочая версия.",
                        ),
                        cx,
                    ))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("offline-extensions")
                                    .label(text(cx, "Work offline", "Без сети"))
                                    .selected(offline)
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.model
                                            .update(cx, |model, cx| model.refresh(true, cx))
                                    })),
                            )
                            .child(
                                Button::new("rollback-extensions")
                                    .label(text(
                                        cx,
                                        "Restore previous set…",
                                        "Вернуть предыдущий набор…",
                                    ))
                                    .disabled(busy || !can_rollback)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.model
                                            .update(cx, |model, cx| model.preview_rollback(cx))
                                    })),
                            ),
                    ),
            )
            .child(
                ui::card(
                    text(cx, "Catalog source", "Источник каталога"),
                    Some(text(
                        cx,
                        "Extensions are listed and verified by this catalog. Change it only if you know who runs the new one.",
                        "Этот каталог перечисляет и проверяет расширения. Меняйте его, только если знаете, кто ведёт новый.",
                    )),
                    cx,
                )
                .child(Input::new(&self.index))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("save-extension-index")
                                .label(text(cx, "Use this source", "Использовать этот источник"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| this.save_index(cx))),
                        )
                        .child(
                            Button::new("default-extension-index")
                                .ghost()
                                .label(text(cx, "Default", "По умолчанию"))
                                .tooltip(DEFAULT_INDEX)
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.index.update(cx, |input, cx| {
                                        input.set_value(DEFAULT_INDEX, window, cx)
                                    });
                                    this.save_index(cx);
                                })),
                        ),
                ),
            )
            .child(
                ui::card(text(cx, "Log", "Журнал"), None, cx).children(
                    logs.into_iter()
                        .chain(runtime_logs)
                        .map(|line| ui::muted(line, cx).font_family("monospace")),
                ),
            )
    }
}

impl Focusable for ExtensionsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ExtensionsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.model.read(cx);
        let busy = model.busy;
        let status = model.status.clone();
        let error = model.error.clone();
        let offline = model.offline;
        let cached = model.from_cache;
        let catalog_available = model.catalog.is_some();
        let has_errors = !cx.global::<ContributionRegistry>().errors.is_empty();
        let preview = model
            .preview
            .as_ref()
            .map(|preview| self.consent(preview, cx));
        let inventory = self.inventory(cx);
        let query = self.search.read(cx).value().to_lowercase();
        let matches = |manifest: &&ExtensionManifest| {
            let metadata = model
                .catalog
                .as_ref()
                .and_then(|catalog| {
                    catalog
                        .extensions
                        .iter()
                        .find(|entry| entry.id == manifest.id)
                })
                .map(|entry| {
                    format!(
                        "{} {}/{} {}",
                        entry.publisher,
                        entry.repository.owner,
                        entry.repository.name,
                        entry.keywords.join(" ")
                    )
                })
                .unwrap_or_default();
            format!(
                "{} {} {} {metadata}",
                manifest.id, manifest.name, manifest.description
            )
            .to_lowercase()
            .contains(&query)
        };
        let visible: Vec<_> = inventory.values().filter(matches).collect();
        let registry = cx.global::<ContributionRegistry>();
        let rows: Vec<_> = visible
            .iter()
            .map(|manifest| {
                let id = manifest.id.clone();
                let (source, _) = self.source(&id, cx);
                let enabled = registry.is_enabled(&id);
                let required = registry
                    .required_by()
                    .get(&id)
                    .is_some_and(|reasons| !reasons.is_empty());
                let selected = self.selected.as_deref() == Some(id.as_str());
                let select_id = id.clone();
                let toggle_id = id.clone();
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new(SharedString::from(format!("extension-{}", id)))
                            .flex_1()
                            .min_w_0()
                            .child(
                                v_flex()
                                    .w_full()
                                    .items_start()
                                    .gap_0p5()
                                    .child(manifest.name.clone())
                                    .child(ui::muted(
                                        format!("{} · {}", manifest.version, source.label(cx)),
                                        cx,
                                    )),
                            )
                            .selected(selected)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.selected = Some(select_id.clone());
                                cx.notify();
                            })),
                    )
                    .when(source != Source::Catalog, |this| {
                        this.child(
                            Switch::new(SharedString::from(format!("extension-switch-{}", id)))
                                .checked(enabled)
                                .disabled(busy || required)
                                .accessibility_label(manifest.name.clone())
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    let action = if *checked {
                                        Removal::Enable
                                    } else {
                                        Removal::Disable
                                    };
                                    let id = toggle_id.clone();
                                    this.selected = Some(id.clone());
                                    this.model.update(cx, |model, cx| {
                                        model.preview_removal(id, action, cx)
                                    })
                                })),
                        )
                    })
            })
            .collect();
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| visible.iter().find(|manifest| &manifest.id == id).copied())
            .or_else(|| visible.first().copied());
        let details = selected.map(|manifest| self.details(manifest, cx));
        let catalog_state = if !catalog_available {
            text(cx, "Catalog: unavailable", "Каталог: недоступен")
        } else if offline {
            text(cx, "Catalog: offline", "Каталог: без сети")
        } else if cached {
            text(cx, "Catalog: cached", "Каталог: из кэша")
        } else {
            text(cx, "Catalog: up to date", "Каталог: обновлён")
        };
        let advanced = self.advanced;
        let mut page = v_flex()
            .id("extensions-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .gap_4()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&self.search)))
                    .child(
                        Button::new("refresh-extensions")
                            .label(text(cx, "Refresh catalog", "Обновить каталог"))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.model.update(cx, |model, cx| model.refresh(false, cx))
                            })),
                    )
                    .when(has_errors || error.is_some(), |this| {
                        this.child(
                            Button::new("retry-extension-activation")
                                .label(text(cx, "Retry", "Повторить"))
                                .disabled(busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.model.update(cx, |model, cx| model.restore_startup(cx))
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(ui::muted(catalog_state, cx))
                    .child(ui::muted(status, cx)),
            );
        if let Some(error) = error {
            page = page.child(ui::banner(
                ui::Tone::Danger,
                text(cx, "Something went wrong", "Что-то пошло не так"),
                Some(error.headline.into()),
                error
                    .detail
                    .map(|detail| ui::details(detail.into(), cx).into_any_element()),
                cx,
            ));
        }
        if busy {
            page = page.child(
                Button::new("cancel-extension-operation")
                    .label(text(cx, "Cancel", "Отмена"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.model.update(cx, |model, cx| model.cancel(cx))
                    })),
            );
        }
        if let Some(preview) = preview {
            page = page.child(preview);
        }
        page = page.child(
            h_flex()
                .items_start()
                .gap_4()
                .child(v_flex().w(rems(22.)).gap_2().children(rows))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when_some(details, |this, details| this.child(details)),
                ),
        );
        if inventory.is_empty() {
            page = page.child(ui::muted(
                text(
                    cx,
                    "No extensions available. Check the catalog source under Advanced.",
                    "Расширения недоступны. Проверьте источник каталога в разделе «Дополнительно».",
                ),
                cx,
            ));
        } else if visible.is_empty() {
            page = page.child(ui::muted(
                text(
                    cx,
                    "Nothing matches your search.",
                    "По запросу ничего не найдено.",
                ),
                cx,
            ));
        }
        page.child(
            Button::new("extensions-advanced")
                .ghost()
                .small()
                .icon(if advanced {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .label(text(cx, "Advanced", "Дополнительно"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.advanced = !this.advanced;
                    cx.notify();
                })),
        )
        .when(advanced, |this| this.child(self.advanced(cx)))
    }
}
