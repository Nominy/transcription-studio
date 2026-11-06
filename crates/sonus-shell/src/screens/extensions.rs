//! Global extension management. The app-owned controller survives navigation so a
//! package transaction cannot be abandoned between durable installation and activation.
mod model;

use super::{
    accounts,
    ui::{self, localized as text},
};
use crate::{ProviderRegistry, ShellHandle, contributions::ContributionRegistry};
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{Disableable, Selectable, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use model::{DEFAULT_INDEX, ExtensionsEvent, ExtensionsModel, Preview, Removal};
use sonus_extension::ExtensionManifest;
use sonus_marketplace::InstallProvenance;
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
    let title = format!("{} {mode_label}", text(cx, "Sign in to", "Вход в"));
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
        let url = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://github.com/…/releases/download/…/*.sonusext")
        });
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
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: !directory,
            directories: directory,
            multiple: false,
            prompt: Some(text(cx, "Review extension", "Проверить расширение").into()),
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
                            model.fail(format!("File picker: {error}"), cx)
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

    fn details(&self, manifest: &ExtensionManifest, cx: &Context<Self>) -> Div {
        let model = self.model.read(cx);
        let registry = cx.global::<ContributionRegistry>();
        let installed = model
            .installed
            .active
            .get(&manifest.id)
            .or_else(|| model.installed.disabled.get(&manifest.id));
        let bundled = installed.is_none()
            && registry
                .manifests()
                .iter()
                .any(|m| m.id == manifest.id && m.runtime == sonus_extension::Runtime::Native);
        let dev = model
            .developer_folders
            .get(&manifest.id)
            .map(|(directory, _)| directory.clone())
            .or_else(|| {
                crate::wasm_host::status(cx)
                    .into_iter()
                    .find(|status| status.owner == manifest.id)
                    .and_then(|status| status.development_directory)
            });
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
            .map(|reasons| reasons.iter().cloned().collect::<Vec<_>>().join(", "))
            .unwrap_or_default();
        let busy = model.busy;
        let source = if bundled {
            text(cx, "Bundled · native", "Встроенное · native").to_owned()
        } else if let Some(directory) = &dev {
            format!(
                "{} · {}",
                text(cx, "Developer folder", "Папка разработчика"),
                directory.display()
            )
        } else if let Some(installed) = installed {
            match &installed.provenance {
                InstallProvenance::Reviewed { .. } => text(
                    cx,
                    "Installed · reviewed index",
                    "Установлено · проверенный индекс",
                )
                .to_owned(),
                InstallProvenance::Developer { source } => format!(
                    "{} · {}",
                    text(cx, "Developer package", "Пакет разработчика"),
                    source.display()
                ),
            }
        } else {
            text(cx, "Catalog", "Каталог").to_owned()
        };
        let active = registry.is_enabled(&manifest.id);
        let id = manifest.id.clone();
        let mut card = ui::card(manifest.name.clone(), Some(&manifest.description), cx)
            .child(ui::detail_row("ID", manifest.id.clone(), cx))
            .child(ui::detail_row(text(cx, "Source", "Источник"), source, cx))
            .child(ui::detail_row(
                text(cx, "State", "Состояние"),
                if active {
                    text(cx, "Enabled", "Включено")
                } else {
                    text(cx, "Not active", "Не активно")
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
            ))
            .child(ui::detail_row(
                text(cx, "Required host API", "Требуемый API"),
                manifest.api.to_string(),
                cx,
            ))
            .child(ui::detail_row(
                text(cx, "Required by active mode", "Требуется активному режиму"),
                if required.is_empty() {
                    "—".to_owned()
                } else {
                    required.clone()
                },
                cx,
            ));
        if !bundled {
            card = card.child(ui::detail_row(
                text(cx, "Document scope", "Доступ к документам"),
                if model.global_access_for(&manifest.id) {
                    text(
                        cx,
                        "All documents · explicit session approval",
                        "Все документы · явное согласие на сессию",
                    )
                } else {
                    text(
                        cx,
                        "Only modes requiring this extension",
                        "Только режимы, которым нужно расширение",
                    )
                },
                cx,
            ));
        }
        if let Some(entry) = entry {
            let repo = format!(
                "https://github.com/{}/{}",
                entry.repository.owner, entry.repository.name
            );
            let publisher_label = if dev.is_some()
                || installed.is_some_and(|installed| {
                    matches!(&installed.provenance, InstallProvenance::Developer { .. })
                }) {
                text(
                    cx,
                    "Catalog author (local source is unreviewed)",
                    "Автор каталога (локальный источник не проверен)",
                )
            } else {
                text(cx, "Author", "Автор")
            };
            card = card
                .child(ui::detail_row(publisher_label, entry.publisher.clone(), cx))
                .child(
                    Button::new("extension-repository")
                        .label(repo.clone())
                        .small()
                        .on_click(move |_, _, cx| cx.open_url(&repo)),
                );
        } else if let Some(InstallProvenance::Reviewed { release }) =
            installed.map(|installed| &installed.provenance)
        {
            let url = release.asset.url.clone();
            card = card
                .child(ui::detail_row(
                    text(cx, "Original release", "Исходный релиз"),
                    url.clone(),
                    cx,
                ))
                .child(
                    Button::new("extension-original-release")
                        .label(text(
                            cx,
                            "Open original GitHub release",
                            "Открыть исходный релиз GitHub",
                        ))
                        .small()
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
        } else {
            card = card.child(ui::detail_row(
                text(cx, "Author / repo", "Автор / репо"),
                if bundled {
                    text(
                        cx,
                        "Bundled with this application; no catalog provenance declared",
                        "Поставляется с приложением; источник каталога не заявлен",
                    )
                } else {
                    text(
                        cx,
                        "Not declared in a validated catalog",
                        "Не заявлен в проверенном каталоге",
                    )
                },
                cx,
            ));
        }
        if let Err(error) = manifest.check_host_compatibility() {
            card = card.child(ui::banner(
                ui::Tone::Warning,
                text(cx, "Incompatible version", "Несовместимая версия"),
                Some(error.to_string().into()),
                None,
                cx,
            ));
        }
        if let Some(latest) = &latest {
            card = card.child(ui::detail_row(
                text(cx, "Compatible", "Совместимая"),
                latest.version.to_string(),
                cx,
            ));
        } else if !bundled && dev.is_none() {
            card = card.child(ui::muted(text(cx, "No compatible reviewed release is available. Refresh the index or inspect its withdrawal/API requirements.", "Нет совместимой проверенной версии. Обновите индекс и проверьте требования API или отзыв версии."), cx));
        }
        card = card.child(ui::caption(text(cx, "Permissions", "Разрешения"), cx));
        if manifest.permissions.is_empty() {
            card = card.child(ui::muted(text(cx, "None", "Нет"), cx));
        }
        for permission in &manifest.permissions {
            card = card.child(ui::muted(model::permission_label(permission), cx));
        }
        card = card.child(ui::caption(text(cx, "Dependencies", "Зависимости"), cx));
        if manifest.dependencies.is_empty() {
            card = card.child(ui::muted(text(cx, "None", "Нет"), cx));
        }
        for (dependency, requirement) in &manifest.dependencies {
            card = card.child(ui::muted(format!("{dependency} {requirement}"), cx));
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
                        text(cx, "Review update", "Проверить обновление")
                    } else {
                        text(cx, "Review install", "Проверить установку")
                    })
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model
                            .update(cx, |model, cx| model.preview_release(release.clone(), cx))
                    })),
            );
        }
        if installed.is_some() {
            let target = id.clone();
            let removal = if model.installed.active.contains_key(&id) {
                Removal::Disable
            } else {
                Removal::Enable
            };
            controls = controls.child(
                Button::new("extension-toggle")
                    .label(if matches!(removal, Removal::Disable) {
                        text(cx, "Disable", "Отключить")
                    } else {
                        text(cx, "Enable", "Включить")
                    })
                    .disabled(busy || !required.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.preview_removal(target.clone(), removal, cx)
                        })
                    })),
            );
            let target = id.clone();
            controls = controls.child(
                Button::new("extension-uninstall")
                    .label(text(cx, "Uninstall", "Удалить"))
                    .disabled(busy || !required.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.preview_removal(target.clone(), Removal::Uninstall, cx)
                        })
                    })),
            );
        }
        if bundled {
            let target = id.clone();
            let action = if active {
                Removal::Disable
            } else {
                Removal::Enable
            };
            controls = controls.child(
                Button::new("extension-native-toggle")
                    .label(if active {
                        text(cx, "Disable", "Отключить")
                    } else {
                        text(cx, "Enable", "Включить")
                    })
                    .disabled(busy || !required.is_empty())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.model.update(cx, |model, cx| {
                            model.preview_removal(target.clone(), action, cx)
                        })
                    })),
            );
        }
        if let Some(directory) = dev {
            controls = controls.child(
                Button::new("extension-reload")
                    .label(text(
                        cx,
                        "Review / reload folder",
                        "Проверить / перезагрузить папку",
                    ))
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
                    .label(text(
                        cx,
                        "Stop development extension",
                        "Остановить расширение",
                    ))
                    .disabled(
                        busy || !required.is_empty()
                            || (!active && !model.developer_folders.contains_key(&id)),
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
                    "Bundled files are read-only and cannot be uninstalled from this page.",
                    "Встроенные файлы доступны только для чтения и не удаляются с этой страницы.",
                ),
                cx,
            ));
        }
        if !required.is_empty() {
            card = card.child(ui::muted(
                text(
                    cx,
                    "Close the active mode/document before disabling or uninstalling.",
                    "Закройте активный режим/документ перед отключением или удалением.",
                ),
                cx,
            ));
        }
        card.child(controls)
    }

    fn consent(&self, preview: &Preview, cx: &Context<Self>) -> Div {
        let mut card = ui::card(
            text(
                cx,
                "Review changes before approval",
                "Проверьте изменения перед подтверждением",
            ),
            None,
            cx,
        );
        for line in preview.lines() {
            card = card.child(ui::muted(line, cx));
        }
        card = card.child(ui::muted(text(cx, "Approval grants exactly the permissions shown above to these versions. Dependencies are installed as one set. Cancel leaves the current installation unchanged.", "Подтверждение выдаёт перечисленные разрешения именно этим версиям. Зависимости устанавливаются единым набором. Отмена сохраняет текущую установку."), cx));
        if self.model.read(cx).document_scope_available() {
            let global_access = self.model.read(cx).global_document_access;
            card = card.child(Button::new("extension-global-document-consent")
                .label(if global_access { text(cx, "All documents: allowed for this session", "Все документы: разрешено на эту сессию") } else { text(cx, "All documents: not allowed (click to opt in)", "Все документы: не разрешено (нажмите для согласия)") })
                .selected(global_access)
                .disabled(self.model.read(cx).busy)
                .on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| { model.global_document_access = !model.global_document_access; cx.notify(); }))))
                .child(ui::muted(text(cx, "Default access is limited to modes that require the selected extension. This additional opt-in applies only to the selected extension ID, never its dependencies, and resets at restart. Manifest permissions are still enforced.", "По умолчанию доступны только режимы, которым требуется выбранное расширение. Дополнительное согласие относится только к выбранному ID, не к зависимостям, и сбрасывается при перезапуске. Разрешения манифеста всё равно проверяются."), cx));
        }
        card.child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("approve-extension-plan")
                        .primary()
                        .label(text(cx, "Approve and apply", "Разрешить и применить"))
                        .disabled(self.model.read(cx).busy || !preview.can_apply())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.update(cx, |model, cx| model.approve(cx))
                        })),
                )
                .child(
                    Button::new("dismiss-extension-plan")
                        .label(text(cx, "Cancel", "Отмена"))
                        .disabled(self.model.read(cx).busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.model.update(cx, |model, cx| {
                                model.preview = None;
                                cx.notify();
                            })
                        })),
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
        let can_rollback = model.installed.previous.is_some();
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
        let runtime = crate::wasm_host::status(cx);
        let rows: Vec<_> = visible
            .iter()
            .map(|manifest| {
                let id = manifest.id.clone();
                let installed = model
                    .installed
                    .active
                    .get(&id)
                    .or_else(|| model.installed.disabled.get(&id));
                let source = if manifest.runtime == sonus_extension::Runtime::Native {
                    text(cx, "bundled", "встроено")
                } else if model.developer_folders.contains_key(&id)
                    || runtime
                        .iter()
                        .any(|status| status.owner == id && status.development_directory.is_some())
                    || installed.is_some_and(|installed| {
                        matches!(&installed.provenance, InstallProvenance::Developer { .. })
                    })
                {
                    text(cx, "developer", "разработка")
                } else if installed.is_some() {
                    text(cx, "installed", "установлено")
                } else {
                    text(cx, "catalog", "каталог")
                };
                Button::new(SharedString::from(format!("extension-{}", id)))
                    .label(format!(
                        "{} · {} · {source}",
                        manifest.name, manifest.version
                    ))
                    .w_full()
                    .selected(self.selected.as_deref() == Some(id.as_str()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        cx.notify();
                    }))
            })
            .collect();
        let selected = self
            .selected
            .as_ref()
            .and_then(|id| visible.iter().find(|manifest| &manifest.id == id).copied())
            .or_else(|| visible.first().copied());
        let details = selected.map(|manifest| self.details(manifest, cx));
        let logs = self.model.read(cx).logs.clone();
        let mut runtime_logs: Vec<_> = runtime
            .into_iter()
            .map(|status| {
                format!(
                    "{} · {} · {}\n{}",
                    status.owner,
                    status.state,
                    status.last_event,
                    status.logs.join("\n")
                )
            })
            .collect();
        runtime_logs.extend(
            cx.global::<ContributionRegistry>()
                .errors
                .iter()
                .map(|(owner, error)| format!("{owner}: {error}")),
        );
        let mut page = v_flex().id("extensions-page").track_focus(&self.focus).size_full().overflow_y_scroll().p_6().gap_4()
            .child(h_flex().gap_3()
                .child(Button::new("extensions-close").label(text(cx, "Close", "Закрыть")).on_click(|_, window, cx| window.close_dialog(cx)))
                .child(div().text_xl().child(text(cx, "Extensions / Marketplace", "Расширения / Маркетплейс"))))
            .child(ui::muted(text(cx, "Bundled extensions work offline. Marketplace packages are sandboxed WASM; native DLL loading is not supported.", "Встроенные расширения работают без сети. Пакеты маркетплейса — изолированный WASM; загрузка сторонних DLL запрещена."), cx))
            .child(h_flex().gap_2()
                .child(div().flex_1().child(Input::new(&self.search)))
                .child(Button::new("refresh-extensions").label(text(cx, "Refresh", "Обновить")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| model.refresh(false, cx)))))
                .child(Button::new("offline-extensions").label(text(cx, "Use cache / offline", "Кэш / без сети")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| model.refresh(true, cx)))))
                .child(Button::new("retry-extension-activation").label(text(cx, "Retry activation", "Повторить активацию")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| model.restore_startup(cx)))))
                .child(Button::new("rollback-extensions").label(text(cx, "Review rollback", "Проверить откат")).disabled(busy || !can_rollback).on_click(cx.listener(|this, _, _, cx| this.model.update(cx, |model, cx| model.preview_rollback(cx))))))
            .child(ui::detail_row(text(cx, "Catalog state", "Состояние каталога"), if !catalog_available { text(cx, "Unavailable · bundled extensions remain available", "Недоступен · встроенные расширения доступны") } else if offline { text(cx, "Offline · validated cache only", "Без сети · проверенный кэш") } else if cached { text(cx, "Cached · inspect refresh status below", "Кэш · состояние обновления ниже") } else { text(cx, "Online · validated index", "Онлайн · проверенный индекс") }, cx))
            .child(ui::muted(status, cx));
        if let Some(error) = error {
            page = page.child(ui::banner(
                ui::Tone::Danger,
                text(cx, "Operation failed", "Операция не выполнена"),
                Some(error.into()),
                None,
                cx,
            ));
        }
        if busy {
            page = page.child(
                Button::new("cancel-extension-operation")
                    .label(text(cx, "Cancel pending work", "Отменить незавершённое"))
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
                .child(v_flex().w(rems(20.)).gap_2().children(rows))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when_some(details, |this, details| this.child(details)),
                ),
        );
        if inventory.is_empty() {
            page = page.child(ui::muted(text(cx, "No extensions registered and no validated catalog available. Check the index source and refresh.", "Нет зарегистрированных расширений и проверенного каталога. Проверьте источник индекса и обновите."), cx));
        } else if visible.is_empty() {
            page = page.child(ui::muted(
                text(
                    cx,
                    "No matching extensions. Clear or change the search query.",
                    "Расширения не найдены. Очистите или измените поиск.",
                ),
                cx,
            ));
        }
        page.child(ui::card(text(cx, "Install package / development", "Установить пакет / разработка"), None, cx)
            .child(h_flex().gap_2().child(div().flex_1().child(Input::new(&self.url)))
                .child(Button::new("review-extension-url").label(text(cx, "Review URL", "Проверить URL")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.preview_url(cx)))))
            .child(ui::muted(text(cx, "URLs must exactly match a release in the reviewed index. Unindexed files require separate developer-source approval.", "URL должен точно совпадать с релизом проверенного индекса. Для неизвестного файла требуется отдельное разрешение источника разработчика."), cx))
            .child(h_flex().gap_2()
                .child(Button::new("review-extension-file").label(text(cx, "Open .sonusext…", "Открыть .sonusext…")).disabled(busy).on_click(cx.listener(|this, _, window, cx| this.choose(false, window, cx))))
                .child(Button::new("review-extension-folder").label(text(cx, "Load developer folder…", "Загрузить папку разработчика…")).disabled(busy).on_click(cx.listener(|this, _, window, cx| this.choose(true, window, cx)))))
            .child(ui::muted(text(cx, "Developer folders are explicitly trusted, watched with debounce, and never loaded as native code. Invalid reloads retain the last working instance; inspect logs below.", "Папки разработчика требуют явного доверия и отслеживаются с задержкой. Нативный код не загружается. Ошибка обновления сохраняет рабочий экземпляр; журнал ниже."), cx)))
            .child(ui::card(text(cx, "Trusted index source", "Доверенный источник индекса"), None, cx)
                .child(ui::muted(text(cx, "Changing this source changes whom you trust to publish reviewed packages. A configured endpoint is not proof the repository has been published.", "Смена источника меняет издателя, которому вы доверяете. Настроенный адрес не означает, что репозиторий уже опубликован."), cx))
                .child(Input::new(&self.index))
                .child(ui::muted(format!("Default: {DEFAULT_INDEX}"), cx))
                .child(Button::new("save-extension-index").label(text(cx, "Trust source and refresh", "Доверять источнику и обновить")).disabled(busy).on_click(cx.listener(|this, _, _, cx| this.save_index(cx)))))
            .child(ui::card(text(cx, "Extension logs", "Журнал расширений"), None, cx)
                .children(logs.into_iter().chain(runtime_logs).map(|line| ui::muted(line, cx))))
    }
}
