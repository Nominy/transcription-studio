//! WASM provider types and mode launches. A `provider` registration becomes a
//! [`ProviderContribution`] whose setup view is the guest's published UI for the provider
//! contribution, finished by `provider-setup-done`/`provider-setup-cancel`. Mode launches
//! remember their window and instance so a later `publish-mode-document` opens the editor
//! with the right origin, whether the mode has a launch window or opens directly.
use super::*;
use crate::{
    ModeLaunchContext,
    providers::{ProviderContribution, ProviderRegistry, ProviderSetup},
};
use exaudio_extension::ModeLaunch;
use std::sync::atomic::Ordering;

struct SetupPending {
    provider: String,
    setup: ProviderSetup,
    window: AnyWindowHandle,
    /// Fires when the shell drops the setup view (its own Cancel, dialog closed) so the
    /// guest learns the request is dead; a finished setup has already left this map.
    _release: Subscription,
}
struct ModeLaunchState {
    instance: Option<crate::ProviderInstance>,
    window: AnyWindowHandle,
}
#[derive(Default)]
struct ProviderRequests {
    setups: BTreeMap<(String, u64), SetupPending>,
    launches: BTreeMap<(String, String), ModeLaunchState>,
}
impl Global for ProviderRequests {}
fn init(cx: &mut App) {
    if !cx.has_global::<ProviderRequests>() {
        cx.set_global(ProviderRequests::default());
    }
}

pub(super) fn register(owner: &str, registration: &w::Registration, cx: &mut App) -> Result<()> {
    let declared = crate::wasm_host::declared_provider(owner, &registration.id, cx)
        .ok_or_else(|| anyhow!("provider was not declared in manifest"))?;
    if !cx.has_global::<ProviderRegistry>() {
        bail!("provider registry unavailable");
    }
    let provider = registration.id.clone();
    let label = if registration.title.trim().is_empty() {
        declared.name
    } else {
        registration.title.clone()
    };
    let setup_owner = owner.to_owned();
    let setup_provider = provider.clone();
    let status_owner = owner.to_owned();
    let sign_out_owner = owner.to_owned();
    let contribution = ProviderContribution {
        id: provider,
        label,
        multiple: declared.multiple,
        setup: Rc::new(move |shell, setup, window, cx| {
            open_setup(&setup_owner, &setup_provider, shell, setup, window, cx)
        }),
        status: Rc::new(move |instance, cx| {
            crate::wasm_host::provider_status(&status_owner, instance, cx)
        }),
        sign_out: Rc::new(move |instance, cx| {
            crate::wasm_host::sign_out_instance(&sign_out_owner, instance, cx)
        }),
    };
    cx.global_mut::<ProviderRegistry>()
        .register_type(owner, contribution)
}

fn open_setup(
    owner: &str,
    provider: &str,
    shell: ShellHandle,
    setup: ProviderSetup,
    window: &mut Window,
    cx: &mut App,
) -> AnyView {
    init(cx);
    let request = files::NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let panel = cx.new(|cx| {
        WasmPanel::new(
            owner.to_owned(),
            provider.to_owned(),
            shell,
            false,
            window,
            cx,
        )
    });
    let instance = setup.instance.clone();
    let released_owner = owner.to_owned();
    let release = cx.observe_release(&panel, move |_, cx| {
        let abandoned = cx.update_global::<ProviderRequests, _>(|state, _| {
            state
                .setups
                .remove(&(released_owner.clone(), request))
                .is_some()
        });
        if abandoned {
            crate::wasm_host::invoke(
                &released_owner,
                Invocation::Event(w::Event::ProviderSetupCancelled(request)),
                cx,
            );
        }
    });
    cx.update_global::<ProviderRequests, _>(|state, _| {
        state.setups.insert(
            (owner.to_owned(), request),
            SetupPending {
                provider: provider.to_owned(),
                setup,
                window: window.window_handle(),
                _release: release,
            },
        );
    });
    crate::wasm_host::invoke(
        owner,
        Invocation::Event(w::Event::ProviderSetup(w::ProviderSetup {
            provider: provider.to_owned(),
            request_id: request,
            instance: instance.as_ref().map(crate::wasm_host::guest_instance),
        })),
        cx,
    );
    panel.into()
}

pub(super) fn setup_done(owner: &str, value: w::ProviderSetupResult, cx: &mut App) -> Result<()> {
    init(cx);
    let pending = cx
        .update_global::<ProviderRequests, _>(|state, _| {
            state.setups.remove(&(owner.to_owned(), value.request_id))
        })
        .ok_or_else(|| anyhow!("unsolicited provider setup result"))?;
    if pending.provider != value.provider {
        bail!("provider setup result names another provider");
    }
    if let Some(instance) = &pending.setup.instance
        && instance.id != value.id
    {
        bail!("re-authentication must keep the instance id");
    }
    let instance = crate::ProviderInstance {
        provider: value.provider,
        id: value.id,
        label: value.label,
    };
    crate::wasm_host::refresh_provider_status(owner, &instance, cx);
    let SetupPending { setup, window, .. } = pending;
    window
        .update(cx, move |_, window, cx| {
            setup.done(instance.id, instance.label, window, cx)
        })
        .map_err(|_| anyhow!("setup window closed"))
}

pub(super) fn setup_cancelled(
    owner: &str,
    provider: &str,
    request_id: u64,
    cx: &mut App,
) -> Result<()> {
    init(cx);
    let pending = cx
        .update_global::<ProviderRequests, _>(|state, _| {
            state.setups.remove(&(owner.to_owned(), request_id))
        })
        .ok_or_else(|| anyhow!("unsolicited provider setup cancellation"))?;
    if pending.provider != provider {
        bail!("provider setup cancellation names another provider");
    }
    cancel(pending, cx);
    Ok(())
}
fn cancel(pending: SetupPending, cx: &mut App) {
    let SetupPending { setup, window, .. } = pending;
    let _ = window.update(cx, move |_, window, cx| setup.cancel(window, cx));
}

/// Remembers the launch so a published mode document opens in the launching window with
/// the launch instance; returns the launch window unless the mode opens directly.
pub(super) fn launch_mode(
    owner: &str,
    mode: &str,
    shell: ShellHandle,
    context: ModeLaunchContext,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    init(cx);
    let direct = cx
        .global::<ContributionRegistry>()
        .mode_manifest(mode)
        .is_some_and(|manifest| manifest.launch == ModeLaunch::Direct);
    cx.update_global::<ProviderRequests, _>(|state, _| {
        state.launches.insert(
            (owner.to_owned(), mode.to_owned()),
            ModeLaunchState {
                instance: context.instance.clone(),
                window: window.window_handle(),
            },
        );
    });
    let panel = (!direct).then(|| {
        cx.new(|cx| {
            WasmPanel::new(
                owner.to_owned(),
                mode.to_owned(),
                shell.clone(),
                false,
                window,
                cx,
            )
        })
        .into()
    });
    crate::wasm_host::invoke(
        owner,
        Invocation::Event(w::Event::ModeOpen(w::ModeOpen {
            mode: mode.to_owned(),
            request_id: 0,
            key: String::new(),
            instance: context
                .instance
                .as_ref()
                .map(crate::wasm_host::guest_instance),
        })),
        cx,
    );
    panel
}

pub(super) fn open_mode_document(
    owner: &str,
    shell: &ShellHandle,
    value: w::ModeDocument,
    cx: &mut App,
) -> Result<()> {
    init(cx);
    let (instance, window) = cx
        .global::<ProviderRequests>()
        .launches
        .get(&(owner.to_owned(), value.mode.clone()))
        .map(|launch| (launch.instance.clone(), launch.window))
        .ok_or_else(|| anyhow!("mode document published before the mode was launched"))?;
    let project = crate::wasm_host::document::project(&value.document)?;
    let origin = crate::DocumentOrigin {
        mode_id: value.mode.clone(),
        instance,
        read_only: value.document.read_only,
        native_path: None,
    };
    let mut environment = crate::ProviderEnvironment::local(project.id.clone(), None);
    environment.document.origin.mode = value.mode;
    let shell = shell.clone();
    window
        .update(cx, move |_, window, cx| {
            shell
                .open_document(project, origin, environment, window, cx)
                .map(|_| ())
        })
        .map_err(|_| anyhow!("launch window closed"))?
}

pub(super) fn remove(owner: &str, cx: &mut App) {
    if !cx.has_global::<ProviderRequests>() {
        return;
    }
    let cancelled = cx.update_global::<ProviderRequests, _>(|state, _| {
        state.launches.retain(|(id, _), _| id != owner);
        let (gone, kept) = std::mem::take(&mut state.setups)
            .into_iter()
            .partition::<BTreeMap<_, _>, _>(|((id, _), _)| id == owner);
        state.setups = kept;
        gone.into_values().collect::<Vec<_>>()
    });
    for pending in cancelled {
        cancel(pending, cx);
    }
}
pub(super) fn request_failed(owner: &str, request: u64, cx: &mut App) {
    if !cx.has_global::<ProviderRequests>() {
        return;
    }
    let pending = cx.update_global::<ProviderRequests, _>(|state, _| {
        state.setups.remove(&(owner.to_owned(), request))
    });
    if let Some(pending) = pending {
        cancel(pending, cx);
    }
}
