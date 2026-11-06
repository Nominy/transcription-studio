//! Native Babel UI contributions. The shell depends only on the public contribution contract.
pub mod editor;
pub mod polish;
pub mod root;
pub mod screens;
pub mod services;

use editor::panels::{
    capabilities::CapabilityPanel,
    context::ContextPanels,
    feedback::FeedbackPanel,
    grader::GraderPanel,
    review_workspace::ReviewWorkspacePanel,
    task::{Drawer, TaskPanels},
    transport::TransportStrip,
};
use gpui_kit::*;
use root::BabelMode;
use sonus_extension::ExtensionManifest;
use sonus_shell::contributions::{CommandContext, KeybindingContribution};
use sonus_shell::editor::model::EditorModel;
use sonus_shell::{
    CommandContribution, ContributionRegistry, ModeContribution, PanelLocation, ShellHandle,
};
use std::rc::Rc;

pub const ID: &str = "sonus.babel";
pub const RELAY_ID: &str = "sonus.babel-relay";
pub const FILE_ID: &str = "sonus.babel-file";

pub fn manifest() -> anyhow::Result<ExtensionManifest> {
    let manifest: ExtensionManifest = serde_json::from_str(include_str!("../manifest.json"))?;
    manifest.validate()?;
    Ok(manifest)
}
pub fn relay_manifest() -> anyhow::Result<ExtensionManifest> {
    let manifest: ExtensionManifest = serde_json::from_str(include_str!("../relay-manifest.json"))?;
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

/// Activates direct-account contributions after the host registers the reviewed manifest.
/// No authentication or project request occurs until the user selects the mode home.
pub fn register(cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<screens::ShellModels>() {
        screens::ShellModels::init(cx);
    }
    polish::install(cx);
    let registry = cx.global_mut::<ContributionRegistry>();
    registry.set_contexts(ID, vec![ID.into(), RELAY_ID.into(), FILE_ID.into()]);
    registry.register_document_provider_factory(
        ID,
        "sonus.babel.domain",
        vec![ID.into(), RELAY_ID.into(), FILE_ID.into()],
        Rc::new(sonus_babel_domain::domain_providers),
    )?;
    registry.register_mode(
        ID,
        ModeContribution {
            id: ID.into(),
            label: "Babel".into(),
            description: "Direct Babel account tasks and review".into(),
            home: Rc::new(|shell, window, cx| {
                root::build_home(shell, BabelMode::Direct, window, cx)
            }),
        },
    )?;
    registry.register_mode(
        ID,
        ModeContribution {
            id: FILE_ID.into(),
            label: "Babel JSON file".into(),
            description: "Offline Babel file editing; no account or remote mutations".into(),
            home: Rc::new(screens::local_files::build_home),
        },
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.context",
        "Task context",
        PanelLocation::Inspector,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| ContextPanels::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.feedback",
        "Feedback",
        PanelLocation::Sidebar,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| FeedbackPanel::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.assistant",
        "Babel assistant",
        PanelLocation::Sidebar,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| CapabilityPanel::new(model, shell, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.review",
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
        "sonus.babel.grader",
        "Grader",
        PanelLocation::Overlay,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| GraderPanel::new(model, window, cx)).into()
        }),
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.actions",
        "Babel task actions",
        PanelLocation::Transport,
        Rc::new(|shell, window, cx| {
            let model = model(&shell, cx);
            cx.new(|cx| TransportStrip::new(model, shell, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.settings",
        "Babel account and relay",
        PanelLocation::Settings,
        Rc::new(|shell, window, cx| {
            cx.new(|cx| screens::settings::SettingsView::new(shell, window, cx))
                .into()
        }),
    )?;
    registry.register_panel(
        ID,
        "sonus.babel.review-settings",
        "Babel AI review",
        PanelLocation::Settings,
        Rc::new(|_, window, cx| {
            cx.new(|cx| screens::review_settings::ReviewSettingsView::new(window, cx))
                .into()
        }),
    )?;
    for (id, label, shortcut, action) in [
        (
            "sonus.babel.submit",
            "Submit task",
            Some("ctrl-enter"),
            TaskPanels::request_submit
                as fn(&mut TaskPanels, &mut Window, &mut Context<TaskPanels>),
        ),
        (
            "sonus.babel.skip",
            "Skip chunk",
            None,
            TaskPanels::request_skip,
        ),
        (
            "sonus.babel.flag",
            "Flag recording",
            None,
            TaskPanels::request_admin_flag,
        ),
        (
            "sonus.babel.drop-claim",
            "Drop claim",
            None,
            TaskPanels::drop_claim,
        ),
        (
            "sonus.babel.magic-review",
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
                run: Rc::new(move |shell, window, cx| with_panels(shell, window, cx, action)),
            },
        )?;
    }
    registry.register_command(
        ID,
        CommandContribution {
            id: "sonus.babel.feedback".into(),
            label: "Feedback".into(),
            shortcut: None,
            toolbar: false,
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
            id: "sonus.babel.grader".into(),
            label: "Grade review".into(),
            shortcut: None,
            toolbar: false,
            run: Rc::new(|shell, window, cx| {
                with_panels(shell, window, cx, |panels, _, cx| panels.open_grader(cx))
            }),
        },
    )?;
    registry.register_command(
        ID,
        CommandContribution {
            id: "sonus.babel.edit-annotation".into(),
            label: "Edit annotation".into(),
            shortcut: None,
            toolbar: false,
            run: Rc::new(|shell, window, cx| {
                shell.dispatch_document_command(
                    "sonus.babel.edit-annotation",
                    serde_json::Value::Null,
                    window,
                    cx,
                )
            }),
        },
    )?;
    registry.register_document_command(
        ID,
        "sonus.babel.edit-annotation",
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
        "sonus.babel.open-create-region",
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
    for provider in [ID, RELAY_ID] {
        registry.register_save_handler(
            ID,
            provider,
            Rc::new(|model, window, cx, saved| {
                let panels = TaskPanels::for_model(&model, window, cx);
                panels.update(cx, |panels, cx| {
                    panels.save_with_callback(window, cx, saved)
                });
            }),
        );
        registry.register_leave_handler(
            ID,
            provider,
            Rc::new(|model, window, cx, done| {
                editor::panels::flows::request_leave(&model, window, cx, done);
            }),
        );
    }
    registry.register_save_handler(
        ID,
        "sonus.babel-file",
        Rc::new(|model, window, cx, done| {
            screens::local_files::save_babel_file(&model, window, cx, done)
        }),
    );
    registry.register_leave_handler(
        ID,
        "sonus.babel-file",
        Rc::new(|model, window, cx, done| {
            screens::local_files::confirm_leave(&model, window, cx, done);
        }),
    );
    for (id, label, command) in [
        (
            "sonus.babel.autofix-row",
            "Autofix selected row",
            "babel.autofix-row",
        ),
        (
            "sonus.babel.autofix-all",
            "Autofix all rows",
            "babel.autofix-all",
        ),
        (
            "sonus.babel.skaz-selection",
            "Convert selection to SKAZ",
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
        let id = format!("sonus.babel.skaz-digit-{digit}");
        registry.register_command(
            ID,
            CommandContribution {
                id: id.clone(),
                label: format!("Replace selected spoken digits with {digit}"),
                shortcut: None,
                toolbar: false,
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
            id: "sonus.babel.acknowledge-warning".into(),
            label: "Acknowledge warning".into(),
            shortcut: None,
            toolbar: false,
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
        "sonus.babel.feedback",
        "sonus.babel.assistant",
        "sonus.babel.review",
        "sonus.babel.grader",
        "sonus.babel.actions",
    ] {
        registry.set_panel_contexts(id, vec![ID.into(), RELAY_ID.into()]);
    }
    for id in [
        "sonus.babel.submit",
        "sonus.babel.skip",
        "sonus.babel.flag",
        "sonus.babel.drop-claim",
        "sonus.babel.magic-review",
        "sonus.babel.feedback",
        "sonus.babel.grader",
    ] {
        registry.set_command_contexts(id, vec![ID.into(), RELAY_ID.into()]);
    }
    for (id, label, start) in [
        ("sonus.babel.relay-start", "Start subaccount relay", true),
        ("sonus.babel.relay-stop", "Stop subaccount relay", false),
    ] {
        registry.register_command(
            ID,
            CommandContribution {
                id: id.into(),
                label: label.into(),
                shortcut: None,
                toolbar: false,
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
        registry.set_command_contexts(id, Vec::new());
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

/// Relay is separately activated and never inferred from saved global credentials.
pub fn register_relay(cx: &mut App) -> anyhow::Result<()> {
    if !cx.has_global::<screens::ShellModels>() {
        screens::ShellModels::init(cx);
    }
    let registry = cx.global_mut::<ContributionRegistry>();
    registry.register_mode(
        RELAY_ID,
        ModeContribution {
            id: RELAY_ID.into(),
            label: "Babel Relay".into(),
            description: "Tasks through an explicit subaccount relay session".into(),
            home: Rc::new(|shell, window, cx| {
                root::build_home(shell, BabelMode::Relay, window, cx)
            }),
        },
    )?;
    registry.on_unload(
        RELAY_ID,
        Rc::new(|cx| root::shutdown_mode(BabelMode::Relay, cx)),
    );
    Ok(())
}
