//! Periodic native recovery, independent of provider-owned documents and save routes.
use crate::{
    DocumentOrigin, ProviderEnvironment, ShellHandle, editor::model::EditorModel, screens::ui,
    services::AppServices,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use std::{
    hash::{Hash, Hasher},
    path::PathBuf,
    time::Duration,
};
pub fn entries(cx: &App) -> Vec<PathBuf> {
    cx.global::<AppServices>()
        .storage
        .value("sonus.native-recovery")
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default()
}
fn remember(root: &PathBuf, cx: &mut App) -> anyhow::Result<()> {
    let mut roots = entries(cx);
    if !roots.contains(root) {
        roots.push(root.clone());
        cx.global_mut::<AppServices>()
            .storage
            .set_value("sonus.native-recovery", serde_json::to_string(&roots)?)?;
    }
    Ok(())
}
pub fn start(model: &Entity<EditorModel>, cx: &mut App) -> Task<()> {
    let model = model.downgrade();
    cx.spawn(async move |cx| {
        let mut previous = None;
        loop {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let snapshot = model.update(cx, |model, cx| {
                if model.origin().instance.is_some()
                    || model.doc().is_read_only()
                    || (!model.doc().session().is_dirty && !model.doc().has_pending_drafts())
                {
                    return None;
                }
                let project = match model.doc().recovery_project() {
                    Ok(project) => project,
                    Err(error) => {
                        backup_not_saved(model, &error.to_string(), cx);
                        return None;
                    }
                };
                let mut hasher = std::hash::DefaultHasher::new();
                project.id.hash(&mut hasher);
                model.origin().native_path.hash(&mut hasher);
                let root = studio_services::paths::app_support_dir()
                    .join("native-recovery")
                    .join(format!("{:016x}", hasher.finish()));
                let base = model
                    .audio_base_dir()
                    .cloned()
                    .unwrap_or_else(|| root.clone());
                Some((project, root, base))
            });
            let Ok(snapshot) = snapshot else { break };
            let Some((project, root, base)) = snapshot else {
                continue;
            };
            if previous.as_ref() == Some(&project) {
                continue;
            }
            let target = root.clone();
            let result = cx
                .background_spawn(async move {
                    let result =
                        studio_services::recovery::write_snapshot(&project, &target, &base);
                    (project, result)
                })
                .await;
            match result {
                (project, Ok(())) => {
                    previous = Some(project);
                    model
                        .update(cx, |model, cx| {
                            if let Err(error) = remember(&root, cx) {
                                backup_not_saved(model, &error.to_string(), cx);
                            }
                        })
                        .ok();
                }
                (_, Err(error)) => {
                    model
                        .update(cx, |model, cx| {
                            backup_not_saved(model, &error.to_string(), cx)
                        })
                        .ok();
                }
            }
        }
    })
}
pub fn open(shell: ShellHandle, root: PathBuf, window: &mut Window, cx: &mut App) {
    let target = root.clone();
    window
        .spawn(cx, async move |cx| {
            let result = cx
                .background_spawn(async move { studio_services::recovery::load_snapshot(&target) })
                .await;
            cx.update(|window, cx| match result {
                Ok(project) => {
                    let env = ProviderEnvironment::local(project.id.clone(), Some(root));
                    match shell.open_document(project, DocumentOrigin::local(None), env, window, cx)
                    {
                        Ok(model) => model.update(cx, |model, cx| {
                            model.update_doc(cx, |doc| doc.session_mut().mark_dirty())
                        }),
                        Err(error) => window.push_notification(ui::anyhow_notification(&error), cx),
                    }
                }
                Err(error) => window.push_notification(
                    ui::error_notification(
                        ui::localized(
                            cx,
                            "Couldn't restore the unsaved work.",
                            "Не удалось восстановить несохранённую работу.",
                        ),
                        Some(format!("{error:#}").into()),
                    ),
                    cx,
                ),
            })
            .ok();
        })
        .detach();
}

/// Status line for a failed backup write; the I/O error stays behind the hover.
fn backup_not_saved(model: &mut EditorModel, error: &str, cx: &mut Context<EditorModel>) {
    model.set_status_with_detail(
        studio_editor::i18n::localized(
            "Couldn't save a backup of unsaved work.",
            "Не удалось сохранить резервную копию.",
        ),
        error.to_string(),
        cx,
    );
}
