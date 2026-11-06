//! Native Babel UI contributions. The shell depends only on the public contribution contract.
pub mod editor;
pub mod polish;
pub mod root;
pub mod screens;
pub mod services;
pub mod text;

use editor::panels::{
    capabilities::CapabilityPanel,
    context::ContextPanels,
    feedback::FeedbackPanel,
    grader::GraderPanel,
    review_workspace::ReviewWorkspacePanel,
    task::{Drawer, TaskPanels},
    transport::TransportStrip,
};
use exaudio_extension::ExtensionManifest;
use exaudio_shell::contributions::{CommandContext, KeybindingContribution};
use exaudio_shell::editor::model::EditorModel;
use exaudio_shell::{
    CommandContribution, ContributionRegistry, ModeContribution, PanelLocation,
    ProviderContribution, ProviderRegistry, ShellHandle,
};
use gpui_kit::*;
use std::rc::Rc;

pub const ID: &str = "exaudio.babel";
/// Error-review mode: same account, error-categorization projects only.
pub const ERRORS_ID: &str = "exaudio.babel.errors";
pub const FILE_ID: &str = "exaudio.babel-file";
/// Provider type of the Babel sign-in (phone/e-mail or sub-account code).
pub const PROVIDER_ID: &str = "exaudio.babel.account";

pub fn manifest() -> anyhow::Result<ExtensionManifest> {
    let manifest: ExtensionManifest = serde_json::from_str(include_str!("../manifest.json"))?;
    manifest.validate()?;
    Ok(manifest)
}

fn model(shell: &ShellHandle, cx: &App) -> Entity<EditorModel> {
    shell
        .editor_model(cx)
        .expect("Babel editor contributions require an open Babel document")
}
fn with_panels(
    shell: ShellHandle,
    window: &mut Window,
    cx: &mut App,
    run: impl FnOnce(&mut TaskPanels, &mut Window, &mut Context<TaskPanels>),
) {
    if let Some(model) = shell.editor_model(cx) {
        let panels = TaskPanels::for_model(&model, window, cx);
        panels.update(cx, |panels, cx| run(panels, window, cx));
    }
}

/// Activates the Babel contributions after the host registers the reviewed manifest: the
/// `exaudio.babel.account` provider type and the three modes. No authentication or project
/// request occurs until the shell launches a mode with a configured account.
pub fn register(cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<screens::ShellModels>() {
        screens::ShellModels::init(cx);
    }
    polish::install(cx);
    let russian = text::is_russian(cx);
    let t = move |en: &'static str, ru: &'static str| if russian { ru } else { en };
    cx.global_mut::<ProviderRegistry>().register_type(
        ID,
        ProviderContribution {
            id: PROVIDER_ID.into(),
            label: t("Babel account", "Аккаунт Babel").into(),
            multiple: true,
            setup: Rc::new(|_shell, setup, window, cx| {
                cx.new(|cx| screens::auth::AuthScreen::new(setup, window, cx))
                    .into()
            }),
            status: Rc::new(|instance, _cx| root::account_status(instance)),
            sign_out: Rc::new(root::sign_out_instance),
        },
    )?;
    root::rediscover_accounts(cx);
    let registry = cx.global_mut::<ContributionRegistry>();
    registry.register_document_provider_factory(
        ID,
        "exaudio.babel.domain",
        vec![ID.into(), ERRORS_ID.into(), FILE_ID.into()],
        Rc::new(exaudio_babel_domain::domain_providers),
    )?;
    registry.register_mode(
        ID,
        ModeContribution {
            id: ID.into(),
            label: "Babel".into(),
            description: t("Transcription tasks", "Задачи транскрипции").into(),
            launch: Rc::new(root::launch),
        },
    )?;
    registry.register_mode(
        ID,
        ModeContribution {
            id: ERRORS_ID.into(),
            label: t("Babel error review", "Babel: ревью ошибок").into(),
            description: t("Error review tasks", "Задачи ревью ошибок").into(),
            launch: Rc::new(root::launch),
        },
    )?;
    registry.register_mode(
        ID,
        ModeContribution {
            id: FILE_ID.into(),
            label: t("Babel file", "Файл Babel").into(),
            description: t(
                "Edit an exported task offline",
                "Редактировать экспортированную задачу офлайн",
            )
            .into(),
            launch: Rc::new(screens::local_files::launch),
        },
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.context",
        t("Task context", "Контекст задачи"),
        PanelLocation::Inspector,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| ContextPanels::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.feedback",
        t("Feedback", "Отзыв"),
        PanelLocation::Sidebar,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| FeedbackPanel::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.assistant",
        t("Babel assistant", "Помощник Babel"),
        PanelLocation::Sidebar,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| CapabilityPanel::new(model, shell, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.review",
        "Magic Review",
        PanelLocation::Overlay,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| ReviewWorkspacePanel::new(model, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.grader",
        t("Grader", "Оценка"),
        PanelLocation::Overlay,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| GraderPanel::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.actions",
        t("Task actions", "Действия с задачей"),
        PanelLocation::Transport,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| TransportStrip::new(model, shell, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.settings",
        "Babel",
        PanelLocation::Settings,
        Rc::new(|shell, window, cx| {
            cx.new(|cx| screens::settings::SettingsView::new(shell, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "exaudio.babel.review-settings",
        t("AI review", "ИИ-ревью"),
        PanelLocation::Settings,
        Rc::new(|_, window, cx| {
            cx.new(|cx| screens::review_settings::ReviewSettingsView::new(window, cx))
                .into()
        }),
    )?;
    for (id, label, shortcut, action) in [
        (
            "exaudio.babel.submit",
            t("Submit", "Отправить"),
            Some("ctrl-enter"),
            TaskPanels::request_submit
                as fn(&mut TaskPanels, &mut Window, &mut Context<TaskPanels>),
        ),
        (
            "exaudio.babel.skip",
            t("Skip", "Пропустить"),
            None,
            TaskPanels::request_skip,
        ),
        (
            "exaudio.babel.flag",
            t("Flag recording", "Пометить запись"),
            None,
            TaskPanels::request_admin_flag,
        ),
        (
            "exaudio.babel.drop-claim",
            t("Release task", "Отпустить задачу"),
            None,
            TaskPanels::drop_claim,
        ),
        (
            "exaudio.babel.magic-review",
            "Magic Review",
            None,
            TaskPanels::magic_review,
        ),
    ] {
        registry.register_command(
            ID,
            CommandContribution {
                id: id.into(),
                label: label.into(),
                shortcut: shortcut.map(str::to_owned),
                toolbar: false,
                menu: true,
                run: Rc::new(move |shell, window, cx| with_panels(shell, window, cx, action)),
            },
        )?;
    }
    registry.register_command(
        ID,
        CommandContribution {
            id: "exaudio.babel.feedback".into(),
            label: t("Feedback", "Отзыв").into(),
            shortcut: None,
            toolbar: false,
            menu: true,
            run: Rc::new(|shell, window, cx| {
                with_panels(shell, window, cx, |panels, _, cx| {
                    panels.toggle_drawer(Drawer::Feedback, cx)
                })
            }),
        },
    )?;
    registry.register_command(
        ID,
        CommandContribution {
            id: "exaudio.babel.grader".into(),
            label: t("Grade", "Оценить").into(),
            shortcut: None,
            toolbar: false,
            menu: true,
            run: Rc::new(|shell, window, cx| {
                with_panels(shell, window, cx, |panels, _, cx| panels.open_grader(cx))
            }),
        },
    )?;
    registry.register_command(
        ID,
        CommandContribution {
            id: "exaudio.babel.edit-annotation".into(),
            label: t("Edit annotation", "Изменить аннотацию").into(),
            shortcut: None,
            toolbar: false,
            menu: true,
            run: Rc::new(|shell, window, cx| {
                shell.dispatch_document_command(
                    "exaudio.babel.edit-annotation",
                    serde_json::Value::Null,
                    window,
                    cx,
                )
            }),
        },
    )?;
    registry.register_document_command(
        ID,
        "exaudio.babel.edit-annotation",
        Rc::new(|shell, arguments, window, cx| {
            let Some(model) = shell.editor_model(cx) else {
                return;
            };
            let segment = arguments
                .get("segment_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .or_else(|| model.read(cx).doc().session().selected_segment_id.clone());
            let Some(segment) = segment else {
                return;
            };
            let state = services::document_dialogs(&model, cx);
            let opened = model.update(cx, |model, cx| {
                let result = model.update_doc(cx, |doc| {
                    state.borrow_mut().open_edit_annotation(doc, &segment)
                });
                let opened = result.is_ok();
                model.apply_result(result, cx);
                opened
            });
            if opened {
                editor::panels::dialogs::edit_annotation::open(&model, window, cx);
            }
        }),
    );
    registry.register_document_command(
        ID,
        "exaudio.babel.open-create-region",
        Rc::new(|shell, arguments, window, cx| {
            let model = model(&shell, cx);
            let state = services::document_dialogs(&model, cx);
            let opened = model.update(cx, |model, cx| {
                let result = model.update_doc(cx, |doc| {
                    state
                        .borrow_mut()
                        .open_create_region_command(doc, &arguments)
                });
                let opened = result.is_ok();
                model.apply_result(result, cx);
                opened
            });
            if opened {
                editor::panels::dialogs::create_region::open(&model, window, cx);
            }
        }),
    );
    for mode in [ID, ERRORS_ID] {
        registry.register_save_handler(
            ID,
            mode,
            Rc::new(|model, window, cx, saved| {
                let panels = TaskPanels::for_model(&model, window, cx);
                panels.update(cx, |panels, cx| {
                    panels.save_with_callback(window, cx, saved)
                });
            }),
        );
        registry.register_leave_handler(
            ID,
            mode,
            Rc::new(|model, window, cx, done| {
                editor::panels::flows::request_leave(&model, window, cx, done);
            }),
        );
    }
    registry.register_save_handler(
        ID,
        FILE_ID,
        Rc::new(|model, window, cx, done| {
            screens::local_files::save_babel_file(&model, window, cx, done)
        }),
    );
    registry.register_leave_handler(
        ID,
        FILE_ID,
        Rc::new(|model, window, cx, done| {
            screens::local_files::confirm_leave(&model, window, cx, done);
        }),
    );
    for (id, label, command) in [
        (
            "exaudio.babel.autofix-row",
            t("Fix this segment", "Исправить сегмент"),
            "babel.autofix-row",
        ),
        (
            "exaudio.babel.autofix-all",
            t("Fix all segments", "Исправить все сегменты"),
            "babel.autofix-all",
        ),
        (
            "exaudio.babel.skaz-selection",
            t("Convert to СКАЗ", "Преобразовать в СКАЗ"),
            "babel.skaz-selection",
        ),
    ] {
        registry.register_command(
            ID,
            CommandContribution {
                id: id.into(),
                label: label.into(),
                shortcut: None,
                toolbar: false,
                menu: true,
                run: Rc::new(move |shell, _, cx| {
                    if let Some(model) = shell.editor_model(cx) {
                        model.update(cx, |model, cx| {
                            model.edit(cx, |doc| {
                                doc.execute_command(command, &serde_json::Value::Null)
                            })
                        });
                    }
                }),
            },
        )?;
    }
    for digit in '0'..='9' {
        let id = format!("exaudio.babel.skaz-digit-{digit}");
        registry.register_command(
            ID,
            CommandContribution {
                id: id.clone(),
                label: text::fill(
                    t("Replace with {digit}", "Заменить на {digit}"),
                    &[("digit", &digit.to_string())],
                ),
                shortcut: None,
                toolbar: false,
                menu: false,
                run: Rc::new(move |shell, _, cx| {
                    if let Some(model) = shell.editor_model(cx) {
                        let arguments = serde_json::json!({ "digit": digit.to_string() });
                        model.update(cx, |model, cx| {
                            model.edit(cx, |doc| {
                                doc.execute_command("babel.skaz-digit", &arguments)
                            })
                        });
                    }
                }),
            },
        )?;
        registry.register_keybinding(
            ID,
            KeybindingContribution {
                command: id,
                shortcut: digit.to_string(),
                context: CommandContext::TextSelection,
            },
        )?;
    }
    registry.register_command(
        ID,
        CommandContribution {
            id: "exaudio.babel.acknowledge-warning".into(),
            label: t("Dismiss warning", "Скрыть предупреждение").into(),
            shortcut: None,
            toolbar: false,
            menu: true,
            run: Rc::new(|shell, _, cx| {
                if let Some(model) = shell.editor_model(cx) {
                    let state = services::document_state(&model, cx);
                    model.update(cx, |model, cx| {
                        model.update_doc(cx, |doc| {
                            doc.with_session(|project, session| {
                                state
                                    .borrow_mut()
                                    .bind(session)
                                    .toggle_warning_assertion(project)
                            })
                        });
                    });
                }
            }),
        },
    )?;
    for id in [
        "exaudio.babel.feedback",
        "exaudio.babel.assistant",
        "exaudio.babel.review",
        "exaudio.babel.grader",
        "exaudio.babel.actions",
    ] {
        registry.set_panel_contexts(id, vec![ID.into(), ERRORS_ID.into()]);
    }
    for id in [
        "exaudio.babel.submit",
        "exaudio.babel.skip",
        "exaudio.babel.flag",
        "exaudio.babel.drop-claim",
        "exaudio.babel.magic-review",
        "exaudio.babel.feedback",
        "exaudio.babel.grader",
    ] {
        registry.set_command_contexts(id, vec![ID.into(), ERRORS_ID.into()]);
    }
    // Document commands backed by babel-lint/babel-tags providers exist in every Babel mode,
    // including local Babel files, and nowhere else.
    let babel_modes = || vec![ID.to_string(), ERRORS_ID.to_string(), FILE_ID.to_string()];
    for id in [
        "exaudio.babel.edit-annotation",
        "exaudio.babel.autofix-row",
        "exaudio.babel.autofix-all",
        "exaudio.babel.skaz-selection",
        "exaudio.babel.acknowledge-warning",
    ] {
        registry.set_command_contexts(id, babel_modes());
    }
    for digit in '0'..='9' {
        registry.set_command_contexts(&format!("exaudio.babel.skaz-digit-{digit}"), babel_modes());
    }
    for (id, label, start) in [
        (
            "exaudio.babel.relay-start",
            t("Start sub-account relay", "Запустить ретранслятор"),
            true,
        ),
        (
            "exaudio.babel.relay-stop",
            t("Stop sub-account relay", "Остановить ретранслятор"),
            false,
        ),
    ] {
        registry.register_command(
            ID,
            CommandContribution {
                id: id.into(),
                label: label.into(),
                shortcut: None,
                toolbar: false,
                menu: true,
                run: Rc::new(move |_, _, cx| {
                    let relay = screens::ShellModels::relay(cx);
                    relay.update(cx, |relay, cx| {
                        if start {
                            relay.start(cx);
                        } else {
                            relay.stop(cx);
                        }
                    });
                }),
            },
        )?;
        registry.set_command_contexts(id, babel_modes());
    }
    registry.on_unload(
        ID,
        Rc::new(|cx| {
            TaskPanels::shutdown(cx);
            root::shutdown(cx);
            polish::uninstall(cx);
            services::shutdown(cx);
            if let Some(models) = cx.try_global::<screens::ShellModels>() {
                let relay = models.relay.clone();
                relay.update(cx, |relay, _| relay.shutdown());
            }
        }),
    );
    Ok(())
}
