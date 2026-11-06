//! Native and component contributions share one owner-scoped registry.
use crate::ShellHandle;
use anyhow::{Result, bail};
use exaudio_extension::document::{DiagnosticProvider, DocumentProviders};
use exaudio_extension::{ExtensionManifest, Permission};
use gpui_kit::*;
use std::sync::Arc;
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

pub type ViewFactory = Rc<dyn Fn(ShellHandle, &mut Window, &mut App) -> AnyView>;
pub type CommandHandler = Rc<dyn Fn(ShellHandle, &mut Window, &mut App)>;
pub type SaveHandler = Rc<
    dyn Fn(
        Entity<crate::editor::model::EditorModel>,
        &mut Window,
        &mut App,
        Option<crate::screens::local_files::Saved>,
    ),
>;
pub type LeaveHandler = Rc<
    dyn Fn(
        Entity<crate::editor::model::EditorModel>,
        &mut Window,
        &mut App,
        crate::screens::local_files::Saved,
    ),
>;
pub type DocumentCommandHandler = Rc<dyn Fn(ShellHandle, serde_json::Value, &mut Window, &mut App)>;
pub type DocumentProviderFactory = Rc<dyn Fn(&studio_core::Project) -> DocumentProviders>;
pub type TeardownHandler = Rc<dyn Fn(&mut App)>;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandContext {
    Workspace,
    Text,
    TextSelection,
    Any,
}
#[derive(Clone)]
pub struct KeybindingContribution {
    pub command: String,
    pub shortcut: String,
    pub context: CommandContext,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelLocation {
    Sidebar,
    Overlay,
    Inspector,
    Settings,
    Transport,
}
/// Context handed to a mode when the shell launches it: requirements are active and, when
/// the manifest declares a provider, `instance` is configured and ready.
#[derive(Clone, Debug)]
pub struct ModeLaunchContext {
    pub mode_id: String,
    pub instance: Option<crate::providers::ProviderInstance>,
}
/// Returns the mode's launch window, or `None` when the mode opened a document itself
/// (`ModeLaunch::Direct`) and the shell should stay on the editor or go back to Start.
pub type ModeLaunchFactory =
    Rc<dyn Fn(ShellHandle, ModeLaunchContext, &mut Window, &mut App) -> Option<AnyView>>;
#[derive(Clone)]
pub struct ModeContribution {
    pub id: String,
    pub label: String,
    pub description: String,
    pub launch: ModeLaunchFactory,
}
impl ModeContribution {
    /// A mode whose launch window is a plain view (the common case).
    pub fn with_window(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        window: impl Fn(ShellHandle, ModeLaunchContext, &mut Window, &mut App) -> AnyView + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            launch: Rc::new(move |shell, context, w, cx| Some(window(shell, context, w, cx))),
        }
    }
}

pub struct ModeRequirements {
    pub extensions: BTreeSet<String>,
    pub required_by: BTreeMap<String, BTreeSet<String>>,
}
#[derive(Clone)]
pub struct CommandContribution {
    pub id: String,
    pub label: String,
    pub shortcut: Option<String>,
    /// Shown as a button above the editor body.
    pub toolbar: bool,
    /// Listed in the toolbar's ⋯ menu; `false` for commands reachable only by keyboard.
    pub menu: bool,
    pub run: CommandHandler,
}
#[derive(Clone)]
pub struct PanelContribution {
    pub id: String,
    pub label: String,
    pub location: PanelLocation,
    pub factory: ViewFactory,
    pub generation: u64,
}
#[derive(Clone)]
pub struct Owned<T> {
    pub owner: String,
    pub contribution: T,
}
#[derive(Default)]
pub struct ContributionRegistry {
    modes: BTreeMap<String, Owned<ModeContribution>>,
    mode_reservations: BTreeMap<String, String>,
    commands: BTreeMap<String, Owned<CommandContribution>>,
    panels: BTreeMap<String, Owned<PanelContribution>>,
    providers: BTreeMap<String, BTreeMap<String, (Vec<String>, DocumentProviders)>>,
    native_activators: BTreeMap<String, fn(&mut App) -> Result<()>>,
    provider_factories: BTreeMap<String, BTreeMap<String, (Vec<String>, DocumentProviderFactory)>>,
    live_factory_providers: BTreeMap<String, DocumentProviders>,
    keybindings: Vec<Owned<KeybindingContribution>>,
    enabled: BTreeSet<String>,
    active_requirements: BTreeSet<String>,
    manifests: BTreeMap<String, ExtensionManifest>,
    grants: BTreeMap<String, BTreeSet<Permission>>,
    required_by: BTreeMap<String, BTreeSet<String>>,
    lifetimes: BTreeMap<String, Vec<Box<dyn std::any::Any>>>,
    contexts: BTreeMap<String, Vec<String>>,
    active_mode: Option<String>,
    teardown: BTreeMap<String, Vec<TeardownHandler>>,
    ready: Vec<Owned<CommandHandler>>,
    document_commands: BTreeMap<String, (String, DocumentCommandHandler)>,
    save_handlers: BTreeMap<String, (String, SaveHandler)>,
    leave_handlers: BTreeMap<String, (String, LeaveHandler)>,
    pub formats: exaudio_extension::format::FormatRegistry,
    pub errors: BTreeMap<String, String>,
    generation: u64,
}
impl Global for ContributionRegistry {}
impl ContributionRegistry {
    pub fn register_manifest(
        &mut self,
        manifest: ExtensionManifest,
        grants: Vec<Permission>,
        enabled: bool,
    ) -> Result<()> {
        manifest.validate()?;
        manifest.check_host_compatibility()?;
        let approved: BTreeSet<_> = grants.into_iter().collect();
        if enabled && manifest.permissions.iter().any(|p| !approved.contains(p)) {
            bail!(
                "Permissions must be approved before enabling {}",
                manifest.id
            );
        }
        let id = manifest.id.clone();
        if let Some(previous) = self.manifests.get(&id) {
            if previous.runtime == exaudio_extension::Runtime::Native && previous != &manifest {
                bail!("A downloaded package cannot replace bundled extension {id}");
            }
            if self.required_by_active_mode(&id) && previous != &manifest {
                bail!("Close the active mode before changing {id}");
            }
            if self.required_by_active_mode(&id) && !enabled {
                bail!("{id} is required by the active mode");
            }
        }
        self.manifests.insert(id.clone(), manifest);
        self.grants.insert(id.clone(), approved);
        if enabled {
            self.enabled.insert(id);
        } else {
            self.enabled.remove(&id);
        }
        self.generation += 1;
        Ok(())
    }
    pub fn manifests(&self) -> Vec<ExtensionManifest> {
        self.manifests.values().cloned().collect()
    }
    pub fn required_by(&self) -> BTreeMap<String, BTreeSet<String>> {
        self.required_by.clone()
    }
    pub fn enable(&mut self, owner: &str) -> Result<()> {
        let manifest = self
            .manifests
            .get(owner)
            .ok_or_else(|| anyhow::anyhow!("No manifest for {owner}"))?;
        let grants = self
            .grants
            .get(owner)
            .ok_or_else(|| anyhow::anyhow!("No permission approval for {owner}"))?;
        if manifest.permissions.iter().any(|p| !grants.contains(p)) {
            bail!("Permission approval required for {owner}");
        }
        self.enabled.insert(owner.into());
        self.generation += 1;
        Ok(())
    }
    pub fn is_enabled(&self, owner: &str) -> bool {
        self.enabled.contains(owner)
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn set_contexts(&mut self, owner: &str, modes: Vec<String>) {
        self.contexts.insert(owner.into(), modes);
        self.generation += 1;
    }
    pub fn set_panel_contexts(&mut self, id: &str, modes: Vec<String>) {
        self.set_contexts(id, modes);
    }
    pub fn set_command_contexts(&mut self, id: &str, modes: Vec<String>) {
        self.set_contexts(id, modes);
    }
    fn contribution_in_context(&self, owner: &str, id: &str) -> bool {
        if self.contexts.contains_key(id) {
            self.in_context(id)
        } else {
            self.in_context(owner)
        }
    }
    fn in_context(&self, owner: &str) -> bool {
        self.contexts.get(owner).is_none_or(|modes| {
            modes.is_empty()
                || self
                    .active_mode
                    .as_ref()
                    .is_some_and(|active| modes.contains(active))
        })
    }
    pub fn on_unload(&mut self, owner: &str, callback: Rc<dyn Fn(&mut App)>) {
        self.teardown
            .entry(owner.into())
            .or_default()
            .push(callback);
    }
    pub fn on_shell_ready(&mut self, owner: &str, callback: CommandHandler) {
        self.ready.push(Owned {
            owner: owner.into(),
            contribution: callback,
        });
    }
    pub fn take_ready_callbacks(&mut self) -> Vec<CommandHandler> {
        let ready = std::mem::take(&mut self.ready);
        ready
            .into_iter()
            .filter(|entry| self.is_enabled(&entry.owner))
            .map(|entry| entry.contribution)
            .collect()
    }
    pub fn register_document_command(
        &mut self,
        owner: &str,
        id: &str,
        handler: DocumentCommandHandler,
    ) {
        self.document_commands
            .insert(id.into(), (owner.into(), handler));
    }
    pub fn document_command(&self, id: &str) -> Option<DocumentCommandHandler> {
        self.document_commands
            .get(id)
            .filter(|(owner, _)| self.is_enabled(owner) && self.contribution_in_context(owner, id))
            .map(|(_, handler)| handler.clone())
    }
    pub fn register_save_handler(&mut self, owner: &str, provider: &str, handler: SaveHandler) {
        self.save_handlers
            .insert(provider.into(), (owner.into(), handler));
    }
    pub fn save_handler(&self, provider: &str) -> Option<SaveHandler> {
        self.save_handlers
            .get(provider)
            .filter(|(owner, _)| self.is_enabled(owner))
            .map(|(_, handler)| handler.clone())
    }
    pub fn register_leave_handler(&mut self, owner: &str, provider: &str, handler: LeaveHandler) {
        self.leave_handlers
            .insert(provider.into(), (owner.into(), handler));
    }
    pub fn leave_handler(&self, provider: &str) -> Option<LeaveHandler> {
        self.leave_handlers
            .get(provider)
            .filter(|(owner, _)| self.is_enabled(owner))
            .map(|(_, handler)| handler.clone())
    }
    pub fn register_diagnostics(
        &mut self,
        owner: &str,
        mode: Option<String>,
        provider: Arc<dyn DiagnosticProvider>,
    ) -> Result<()> {
        let id = format!("{owner}.diagnostics");
        self.register_document_providers(
            owner,
            &id,
            mode.into_iter().collect(),
            DocumentProviders {
                diagnostics: vec![provider],
                ..Default::default()
            },
        )
    }
    pub fn register_document_providers(
        &mut self,
        owner: &str,
        id: &str,
        modes: Vec<String>,
        providers: DocumentProviders,
    ) -> Result<()> {
        if !self.manifests.contains_key(owner) {
            bail!("Register manifest before providers: {owner}");
        }
        if !exaudio_extension::manifest::valid_id(id) || !id.starts_with(&format!("{owner}.")) {
            bail!("Provider ID must be namespaced by {owner}");
        }
        let entries = self.providers.entry(owner.into()).or_default();
        if entries.contains_key(id) {
            bail!("Duplicate provider contribution: {id}");
        }
        entries.insert(id.into(), (modes, providers));
        self.generation += 1;
        Ok(())
    }
    pub fn register_document_provider_factory(
        &mut self,
        owner: &str,
        id: &str,
        modes: Vec<String>,
        factory: DocumentProviderFactory,
    ) -> Result<()> {
        if !self.manifests.contains_key(owner)
            || !exaudio_extension::manifest::valid_id(id)
            || !id.starts_with(&format!("{owner}."))
        {
            bail!("Invalid provider factory owner or ID: {id}");
        }
        let entries = self.provider_factories.entry(owner.into()).or_default();
        if entries.contains_key(id) {
            bail!("Duplicate provider factory: {id}");
        }
        entries.insert(id.into(), (modes, factory));
        self.generation += 1;
        Ok(())
    }
    pub fn providers_for_document(
        &mut self,
        project: &studio_core::Project,
        mode: &str,
    ) -> DocumentProviders {
        let mut result = self.providers(mode);
        self.live_factory_providers.clear();
        for (owner, factories) in &self.provider_factories {
            if !self.is_enabled(owner) {
                continue;
            }
            let mut owned = DocumentProviders::default();
            for (_, factory) in factories
                .values()
                .filter(|(scopes, _)| scopes.is_empty() || scopes.iter().any(|scope| scope == mode))
            {
                merge_providers(&mut owned, &factory(project));
            }
            merge_providers(&mut result, &owned);
            self.live_factory_providers.insert(owner.clone(), owned);
        }
        result
    }
    pub fn clear_document_providers(&mut self) {
        self.live_factory_providers.clear();
    }
    fn providers_for_enabled_owner(
        &mut self,
        owner: &str,
        project: &studio_core::Project,
        mode: &str,
    ) -> DocumentProviders {
        let mut result = DocumentProviders::default();
        if let Some(entries) = self.providers.get(owner) {
            for (_, providers) in entries
                .values()
                .filter(|(scopes, _)| scopes.is_empty() || scopes.iter().any(|scope| scope == mode))
            {
                merge_providers(&mut result, providers);
            }
        }
        if let Some(entries) = self.provider_factories.get(owner) {
            let mut live = DocumentProviders::default();
            for (_, factory) in entries
                .values()
                .filter(|(scopes, _)| scopes.is_empty() || scopes.iter().any(|scope| scope == mode))
            {
                merge_providers(&mut live, &factory(project));
            }
            merge_providers(&mut result, &live);
            self.live_factory_providers.insert(owner.into(), live);
        }
        result
    }
    pub fn register_keybinding(
        &mut self,
        owner: &str,
        binding: KeybindingContribution,
    ) -> Result<()> {
        if !self.manifests.contains_key(owner)
            || studio_editor::edit::keymap::KeyChord::parse(&binding.shortcut).is_none()
        {
            bail!("Invalid keybinding owner or shortcut");
        }
        self.keybindings.push(Owned {
            owner: owner.into(),
            contribution: binding,
        });
        Ok(())
    }
    pub fn resolve_keybinding(&self, stroke: &str, text: bool, selection: bool) -> Option<String> {
        use studio_editor::edit::keymap::KeyChord;
        let actual = KeyChord::parse(stroke)?.resolved(crate::keymap::IS_MAC);
        let matches = |shortcut: &str| {
            KeyChord::parse(shortcut)
                .is_some_and(|chord| chord.resolved(crate::keymap::IS_MAC) == actual)
        };
        self.keybindings
            .iter()
            .rev()
            .find(|entry| {
                self.is_enabled(&entry.owner)
                    && self.contribution_in_context(&entry.owner, &entry.contribution.command)
                    && matches(&entry.contribution.shortcut)
                    && match entry.contribution.context {
                        CommandContext::Any => true,
                        CommandContext::Workspace => !text,
                        CommandContext::Text => text,
                        CommandContext::TextSelection => text && selection,
                    }
            })
            .map(|entry| entry.contribution.command.clone())
            .or_else(|| {
                self.commands()
                    .into_iter()
                    .find(|command| command.shortcut.as_deref().is_some_and(matches))
                    .map(|command| command.id)
            })
    }
    pub fn providers(&self, mode: &str) -> DocumentProviders {
        let mut result = DocumentProviders::default();
        for (_, scopes) in self
            .providers
            .iter()
            .filter(|(owner, _)| self.is_enabled(owner))
        {
            for (_, providers) in scopes
                .values()
                .filter(|(scope, _)| scope.is_empty() || scope.iter().any(|s| s == mode))
            {
                merge_providers(&mut result, providers);
            }
        }
        result
    }
    pub fn retain<T: 'static>(&mut self, owner: &str, value: T) {
        self.lifetimes
            .entry(owner.into())
            .or_default()
            .push(Box::new(value));
    }
    pub fn reserve_modes(&mut self, owner: &str, ids: &[String]) -> Result<()> {
        for id in ids {
            if !exaudio_extension::manifest::valid_id(id)
                || self.modes.get(id).is_some_and(|mode| mode.owner != owner)
                || self
                    .mode_reservations
                    .get(id)
                    .is_some_and(|existing| existing != owner)
            {
                bail!("Mode ID unavailable: {id}");
            }
        }
        for id in ids {
            self.mode_reservations.insert(id.clone(), owner.into());
        }
        Ok(())
    }
    pub fn release_mode_reservations(&mut self, owner: &str) {
        self.mode_reservations
            .retain(|_, existing| existing != owner);
    }
    pub fn register_mode(&mut self, owner: &str, mode: ModeContribution) -> Result<()> {
        if self
            .mode_reservations
            .get(&mode.id)
            .is_some_and(|existing| existing != owner)
        {
            bail!("Mode is reserved by another extension: {}", mode.id);
        }
        if !self
            .manifests
            .get(owner)
            .is_some_and(|manifest| manifest.modes.iter().any(|declared| declared.id == mode.id))
        {
            bail!("Mode {} is not declared by {owner}", mode.id);
        }
        if self.modes.contains_key(&mode.id) {
            bail!("Mode already registered: {}", mode.id);
        }
        self.modes.insert(
            mode.id.clone(),
            Owned {
                owner: owner.into(),
                contribution: mode,
            },
        );
        Ok(())
    }
    pub fn register_command(&mut self, owner: &str, command: CommandContribution) -> Result<()> {
        if self.commands.contains_key(&command.id) {
            bail!("Command already registered: {}", command.id);
        }
        self.commands.insert(
            command.id.clone(),
            Owned {
                owner: owner.into(),
                contribution: command,
            },
        );
        Ok(())
    }
    pub fn register_panel(
        &mut self,
        owner: &str,
        id: &str,
        label: &str,
        location: PanelLocation,
        factory: ViewFactory,
    ) -> Result<()> {
        if self.panels.contains_key(id) {
            bail!("Panel already registered: {id}");
        }
        self.generation += 1;
        self.panels.insert(
            id.into(),
            Owned {
                owner: owner.into(),
                contribution: PanelContribution {
                    id: id.into(),
                    label: label.into(),
                    location,
                    factory,
                    generation: self.generation,
                },
            },
        );
        Ok(())
    }
    pub fn modes(&self) -> Vec<ModeContribution> {
        self.modes
            .values()
            .filter(|x| self.is_enabled(&x.owner))
            .map(|x| x.contribution.clone())
            .collect()
    }
    /// Manifest declaration of a registered mode (provider, launch kind).
    pub fn mode_manifest(&self, id: &str) -> Option<&exaudio_extension::ModeManifest> {
        let owned = self.modes.get(id)?;
        self.manifests
            .get(&owned.owner)?
            .modes
            .iter()
            .find(|mode| mode.id == id)
    }
    pub fn commands(&self) -> Vec<CommandContribution> {
        self.commands
            .values()
            .filter(|x| {
                self.is_enabled(&x.owner)
                    && self.contribution_in_context(&x.owner, &x.contribution.id)
            })
            .map(|x| x.contribution.clone())
            .collect()
    }
    pub fn panels(&self, location: PanelLocation) -> Vec<PanelContribution> {
        self.panels
            .values()
            .filter(|x| {
                self.is_enabled(&x.owner)
                    && (location == PanelLocation::Settings
                        || self.contribution_in_context(&x.owner, &x.contribution.id))
                    && x.contribution.location == location
            })
            .map(|x| x.contribution.clone())
            .collect()
    }
    pub fn command(&self, id: &str) -> Option<CommandHandler> {
        self.commands
            .get(id)
            .filter(|x| self.is_enabled(&x.owner) && self.contribution_in_context(&x.owner, id))
            .map(|x| x.contribution.run.clone())
    }
    /// Resolve from the registered mode owner, never from an arbitrary manifest declaration.
    pub fn resolve_mode_requirements(&self, id: &str) -> Result<ModeRequirements> {
        let owned = self
            .modes
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("Unknown mode: {id}"))?;
        let manifest = self
            .manifests
            .get(&owned.owner)
            .ok_or_else(|| anyhow::anyhow!("Mode has no manifest: {id}"))?;
        let available: Vec<_> = self.manifests.values().collect();
        let plan = exaudio_extension::resolve_mode(manifest, id, &available)?;
        let required: BTreeSet<String> = plan.extensions.iter().map(|x| x.id.clone()).collect();
        let missing: Vec<_> = required.difference(&self.enabled).cloned().collect();
        if !missing.is_empty() {
            bail!("Required extensions are disabled: {}", missing.join(", "));
        }
        let required_by = plan
            .required_by
            .into_iter()
            .map(|(id, by)| (id.to_owned(), by.into_iter().map(str::to_owned).collect()))
            .collect();
        Ok(ModeRequirements {
            extensions: required,
            required_by,
        })
    }

    pub fn enter_mode(&mut self, id: &str) -> Result<ModeContribution> {
        let resolved = self.resolve_mode_requirements(id)?;
        let mode = self.modes[id].contribution.clone();
        self.required_by = resolved.required_by;
        self.active_requirements = resolved.extensions;
        self.active_mode = Some(id.into());
        self.generation += 1;
        Ok(mode)
    }
    pub fn leave_mode(&mut self) {
        self.active_mode = None;
        self.active_requirements.clear();
        self.required_by.clear();
        self.generation += 1;
    }
    pub fn required_by_active_mode(&self, owner: &str) -> bool {
        self.active_requirements.contains(owner)
    }
    pub fn unload(&mut self, owner: &str) -> Result<()> {
        if self.required_by_active_mode(owner) {
            bail!("{owner} is required by the active mode; close its document first");
        }
        self.force_remove(owner);
        Ok(())
    }
    pub fn clear_contributions(&mut self, owner: &str) {
        self.modes.retain(|_, x| x.owner != owner);
        self.commands.retain(|_, x| x.owner != owner);
        self.panels.retain(|_, x| x.owner != owner);
        self.providers.remove(owner);
        self.provider_factories.remove(owner);
        self.live_factory_providers.remove(owner);
        self.keybindings.retain(|entry| entry.owner != owner);
        self.document_commands.retain(|_, (id, _)| id != owner);
        self.save_handlers.retain(|_, (id, _)| id != owner);
        self.leave_handlers.retain(|_, (id, _)| id != owner);
        self.ready.retain(|entry| entry.owner != owner);
        self.formats.unregister_extension(owner);
        self.lifetimes.remove(owner);
        self.generation += 1;
    }
    pub fn force_remove(&mut self, owner: &str) {
        self.release_mode_reservations(owner);
        self.clear_contributions(owner);
        self.active_requirements.remove(owner);
        self.required_by.remove(owner);
        self.enabled.remove(owner);
        self.errors.remove(owner);
        self.generation += 1;
    }
}

/// Removes every contribution of `owner` from both registries (contributions and
/// provider types). Use instead of `ContributionRegistry::clear_contributions` when a `cx`
/// is available.
pub fn clear_owner(owner: &str, cx: &mut App) {
    cx.global_mut::<ContributionRegistry>()
        .clear_contributions(owner);
    clear_provider_types(owner, cx);
}
/// `ContributionRegistry::force_remove` plus provider-type removal.
pub fn force_remove_owner(owner: &str, cx: &mut App) {
    cx.global_mut::<ContributionRegistry>().force_remove(owner);
    clear_provider_types(owner, cx);
}
fn clear_provider_types(owner: &str, cx: &mut App) {
    if cx.has_global::<crate::ProviderRegistry>() {
        cx.global_mut::<crate::ProviderRegistry>()
            .clear_types(owner);
    }
}

pub fn unload_extension(owner: &str, cx: &mut App) -> Result<()> {
    if cx
        .global::<ContributionRegistry>()
        .required_by_active_mode(owner)
    {
        bail!("{owner} is required by the active mode");
    }
    persist_disabled(owner, true, cx)?;
    let (callbacks, providers) = {
        let registry = cx.global_mut::<ContributionRegistry>();
        if registry.required_by_active_mode(owner) {
            bail!("{owner} is required by the active mode");
        }
        let callbacks = registry.teardown.remove(owner).unwrap_or_default();
        let mut providers = registry.providers.get(owner).cloned().unwrap_or_default();
        if let Some(live) = registry.live_factory_providers.get(owner) {
            providers.insert(String::new(), (Vec::new(), live.clone()));
        }
        (callbacks, providers)
    };
    if let Some(model) = cx
        .try_global::<ShellHandle>()
        .and_then(|shell| shell.editor_model(cx))
    {
        model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                let revision = doc.session().current_edit_revision();
                let sources: BTreeSet<_> = providers
                    .values()
                    .flat_map(|(_, providers)| providers.diagnostics.iter())
                    .map(|provider| provider.diagnose(doc.project(), revision).source)
                    .collect();
                for (_, providers) in providers.values() {
                    remove_providers(&mut doc.session_mut().environment.providers, providers);
                }
                doc.with_session(|project, session| {
                    for source in &sources {
                        session.remove_diagnostics(project, source);
                    }
                    session.refresh_diagnostics(project);
                });
            })
        });
    }
    cx.global_mut::<ContributionRegistry>().unload(owner)?;
    clear_provider_types(owner, cx);
    for callback in callbacks {
        callback(cx);
    }
    cx.refresh_windows();
    Ok(())
}

pub fn merge_providers(target: &mut DocumentProviders, source: &DocumentProviders) {
    fn merge<T: ?Sized>(target: &mut Vec<Arc<T>>, source: &[Arc<T>]) {
        for item in source {
            if !target.iter().any(|p| Arc::ptr_eq(p, item)) {
                target.push(item.clone());
            }
        }
    }
    merge(&mut target.diagnostics, &source.diagnostics);
    merge(&mut target.completions, &source.completions);
    merge(&mut target.decorations, &source.decorations);
    merge(&mut target.commands, &source.commands);
    merge(&mut target.segment_kinds, &source.segment_kinds);
    merge(
        &mut target.diagnostic_postprocessors,
        &source.diagnostic_postprocessors,
    );
    if target.mutation_policy.is_none() {
        target.mutation_policy = source.mutation_policy.clone();
    }
}
fn remove_providers(target: &mut DocumentProviders, source: &DocumentProviders) {
    fn remove<T: ?Sized>(target: &mut Vec<Arc<T>>, source: &[Arc<T>]) {
        target.retain(|p| !source.iter().any(|item| Arc::ptr_eq(p, item)));
    }
    remove(&mut target.diagnostics, &source.diagnostics);
    remove(&mut target.completions, &source.completions);
    remove(&mut target.decorations, &source.decorations);
    remove(&mut target.commands, &source.commands);
    remove(&mut target.segment_kinds, &source.segment_kinds);
    remove(
        &mut target.diagnostic_postprocessors,
        &source.diagnostic_postprocessors,
    );
    if target
        .mutation_policy
        .as_ref()
        .zip(source.mutation_policy.as_ref())
        .is_some_and(|(left, right)| Arc::ptr_eq(left, right))
    {
        target.mutation_policy = None;
    }
}
fn disabled(cx: &App) -> BTreeSet<String> {
    cx.global::<crate::services::AppServices>()
        .storage
        .value("exaudio.extensions.disabled")
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default()
}
fn persist_disabled(owner: &str, value: bool, cx: &mut App) -> Result<()> {
    let mut disabled = disabled(cx);
    if value {
        disabled.insert(owner.into());
    } else {
        disabled.remove(owner);
    }
    cx.global_mut::<crate::services::AppServices>()
        .storage
        .set_value(
            "exaudio.extensions.disabled",
            serde_json::to_string(&disabled)?,
        )?;
    Ok(())
}

pub fn register_native_extension(
    manifest: ExtensionManifest,
    activate: fn(&mut App) -> Result<()>,
    cx: &mut App,
) -> Result<()> {
    if manifest.runtime != exaudio_extension::Runtime::Native {
        bail!("Native activation requires a native manifest");
    }
    let owner = manifest.id.clone();
    let grants = manifest.permissions.clone();
    let enabled = !disabled(cx).contains(&owner);
    let registry = cx.global_mut::<ContributionRegistry>();
    registry.register_manifest(manifest, grants, false)?;
    registry.native_activators.insert(owner.clone(), activate);
    if enabled {
        enable_extension(&owner, cx)?;
    }
    Ok(())
}
pub fn enable_extension(owner: &str, cx: &mut App) -> Result<()> {
    if cx.global::<ContributionRegistry>().is_enabled(owner) {
        return Ok(());
    }
    let activate = *cx
        .global::<ContributionRegistry>()
        .native_activators
        .get(owner)
        .ok_or_else(|| anyhow::anyhow!("No native activation factory for {owner}"))?;
    cx.global_mut::<ContributionRegistry>().enable(owner)?;
    let activated = activate(cx).and_then(|()| persist_disabled(owner, false, cx));
    if let Err(error) = activated {
        let callbacks = cx
            .global_mut::<ContributionRegistry>()
            .teardown
            .remove(owner)
            .unwrap_or_default();
        force_remove_owner(owner, cx);
        for callback in callbacks {
            callback(cx);
        }
        cx.global_mut::<ContributionRegistry>()
            .errors
            .insert(owner.into(), error.to_string());
        cx.refresh_windows();
        return Err(error);
    }
    if let Some(model) = cx
        .try_global::<ShellHandle>()
        .and_then(|shell| shell.editor_model(cx))
    {
        let providers = cx.update_global::<ContributionRegistry, _>(|registry, cx| {
            let model = model.read(cx);
            registry.providers_for_enabled_owner(
                owner,
                model.doc().project(),
                &model.origin().mode_id,
            )
        });
        model.update(cx, |model, cx| {
            model.update_doc(cx, |doc| {
                merge_providers(&mut doc.session_mut().environment.providers, &providers);
                doc.with_session(|project, session| session.refresh_diagnostics(project));
            })
        });
    }
    cx.refresh_windows();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CommandContext, CommandContribution, ContributionRegistry, KeybindingContribution,
        ModeContribution,
    };
    use exaudio_extension::ExtensionManifest;
    use std::rc::Rc;
    fn manifest(id: &str) -> ExtensionManifest {
        serde_json::from_value(serde_json::json!({"id":id,"name":id,"version":"1.0.0","api":"^0.1","runtime":"native","description":"Test contribution","license":"MIT","dependencies":{},"permissions":[],"modes":[]})).unwrap()
    }
    #[test]
    fn mode_entry_resolves_versions_and_blocks_required_unload() {
        let mut registry = ContributionRegistry::default();
        let mut owner = manifest("exaudio.test");
        owner.modes=serde_json::from_value(serde_json::json!([{"id":"exaudio.test.mode","name":"Test","requires":{"exaudio.dependency":"^1"}}])).unwrap();
        registry.register_manifest(owner, Vec::new(), true).unwrap();
        registry
            .register_mode(
                "exaudio.test",
                ModeContribution {
                    id: "exaudio.test.mode".into(),
                    label: "Test".into(),
                    description: String::new(),
                    launch: Rc::new(|_, _, _, _| {
                        panic!("UI factory is not invoked by dependency resolution")
                    }),
                },
            )
            .unwrap();
        assert!(registry.enter_mode("exaudio.test.mode").is_err());
        registry
            .register_manifest(manifest("exaudio.dependency"), Vec::new(), false)
            .unwrap();
        assert!(registry.enter_mode("exaudio.test.mode").is_err());
        registry.enable("exaudio.dependency").unwrap();
        registry.enter_mode("exaudio.test.mode").unwrap();
        assert!(registry.unload("exaudio.dependency").is_err());
        registry.leave_mode();
        registry.unload("exaudio.dependency").unwrap();
    }
    #[test]
    fn registering_contributions_never_enables_disabled_extension() {
        let mut registry = ContributionRegistry::default();
        registry
            .register_manifest(manifest("exaudio.test"), Vec::new(), false)
            .unwrap();
        registry
            .register_command(
                "exaudio.test",
                CommandContribution {
                    id: "exaudio.test.command".into(),
                    label: "Test".into(),
                    shortcut: None,
                    toolbar: false,
                    menu: true,
                    run: Rc::new(|_, _, _| panic!("disabled command")),
                },
            )
            .unwrap();
        assert!(registry.commands().is_empty());
        assert!(!registry.is_enabled("exaudio.test"));
        assert!(registry.providers("exaudio.local").diagnostics.is_empty());
    }
    #[test]
    fn command_contexts_follow_the_active_mode() {
        let mut registry = ContributionRegistry::default();
        let mut owner = manifest("exaudio.test");
        owner.modes = serde_json::from_value(serde_json::json!([
            {"id":"exaudio.test.a","name":"A"},
            {"id":"exaudio.test.b","name":"B"}
        ]))
        .unwrap();
        registry.register_manifest(owner, Vec::new(), true).unwrap();
        for id in ["exaudio.test.a", "exaudio.test.b"] {
            registry
                .register_mode(
                    "exaudio.test",
                    ModeContribution {
                        id: id.into(),
                        label: id.into(),
                        description: String::new(),
                        launch: Rc::new(|_, _, _, _| {
                            panic!("launch is not part of context resolution")
                        }),
                    },
                )
                .unwrap();
        }
        registry
            .register_command(
                "exaudio.test",
                CommandContribution {
                    id: "exaudio.test.only-a".into(),
                    label: "Only in A".into(),
                    shortcut: None,
                    toolbar: false,
                    menu: true,
                    run: Rc::new(|_, _, _| {}),
                },
            )
            .unwrap();
        registry.set_command_contexts("exaudio.test.only-a", vec!["exaudio.test.a".into()]);
        let ids = |registry: &ContributionRegistry| {
            registry
                .commands()
                .into_iter()
                .map(|command| command.id)
                .collect::<Vec<_>>()
        };

        registry.enter_mode("exaudio.test.b").unwrap();
        assert!(ids(&registry).is_empty());
        assert!(registry.command("exaudio.test.only-a").is_none());
        registry.leave_mode();

        registry.enter_mode("exaudio.test.a").unwrap();
        assert_eq!(ids(&registry), vec!["exaudio.test.only-a".to_string()]);
        assert!(registry.command("exaudio.test.only-a").is_some());
    }
    #[test]
    fn mode_reservations_are_atomic_and_owner_scoped() {
        let mut registry = ContributionRegistry::default();
        registry
            .reserve_modes("exaudio.first", &["exaudio.shared".into()])
            .unwrap();
        assert!(
            registry
                .reserve_modes(
                    "exaudio.second",
                    &["exaudio.other".into(), "exaudio.shared".into()]
                )
                .is_err()
        );
        registry
            .reserve_modes("exaudio.third", &["exaudio.other".into()])
            .unwrap();
        registry.release_mode_reservations("exaudio.first");
        registry
            .reserve_modes("exaudio.second", &["exaudio.shared".into()])
            .unwrap();
    }
    #[test]
    fn selection_keybinding_does_not_intercept_plain_typing() {
        let mut registry = ContributionRegistry::default();
        registry
            .register_manifest(manifest("exaudio.test"), Vec::new(), true)
            .unwrap();
        registry
            .register_keybinding(
                "exaudio.test",
                KeybindingContribution {
                    command: "exaudio.test.convert".into(),
                    shortcut: "1".into(),
                    context: CommandContext::TextSelection,
                },
            )
            .unwrap();
        assert_eq!(registry.resolve_keybinding("1", true, false), None);
        assert_eq!(registry.resolve_keybinding("1", false, false), None);
        assert_eq!(
            registry.resolve_keybinding("1", true, true).as_deref(),
            Some("exaudio.test.convert")
        );
    }
}
