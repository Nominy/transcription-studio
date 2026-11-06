//! Providers: account/session owners shared by modes. A provider *type* is contributed by
//! an extension and knows how to sign in, report status and sign out. A provider *instance*
//! is one configured account of that type. Instances are shell-owned records; credentials
//! are stored by the provider type under [`instance_dir`], never in the instance record.
//!
//! Users meet providers only as a sign-in step while opening a mode whose manifest declares
//! `provider`, and as "Accounts" under advanced settings. A mode never stores sessions.
use crate::ShellHandle;
use crate::services::AppServices;
use anyhow::{Result, bail};
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, rc::Rc};
use studio_services::prefs::UiStorage;

const INSTANCES_KEY: &str = "exaudio.providers.instances";
const SELECTED_KEY: &str = "exaudio.providers.selected";

/// One configured account of a provider type. `id` is unique per provider and path-safe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInstance {
    pub provider: String,
    pub id: String,
    /// Human label chosen by the provider at setup (e.g. the phone number or display name).
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderStatus {
    /// Usable; `detail` is shown next to the instance label ("signed in", a host URL).
    Ready {
        detail: String,
    },
    /// Credentials expired or missing: opening a mode re-runs setup for this instance.
    NeedsSignIn,
    Error(String),
}

pub type ProviderSetupDone = Rc<dyn Fn(ProviderInstance, &mut Window, &mut App)>;
pub type ProviderSetupCancel = Rc<dyn Fn(&mut Window, &mut App)>;
/// One-shot completion for dialog-driven setup: `Some` once stored, `None` on cancel.
pub type ProviderSetupOutcome = Box<dyn FnOnce(Option<ProviderInstance>, &mut Window, &mut App)>;

/// Completion handle given to a provider's setup view. Exactly one of `done`/`cancel`
/// is called; the shell then resumes the mode launch that requested the setup.
#[derive(Clone)]
pub struct ProviderSetup {
    pub provider: String,
    /// Set when re-authenticating an existing instance; `None` when adding a new one.
    pub instance: Option<ProviderInstance>,
    done: ProviderSetupDone,
    cancel: ProviderSetupCancel,
}
impl ProviderSetup {
    pub fn new(
        provider: impl Into<String>,
        instance: Option<ProviderInstance>,
        done: impl Fn(ProviderInstance, &mut Window, &mut App) + 'static,
        cancel: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            provider: provider.into(),
            instance,
            done: Rc::new(done),
            cancel: Rc::new(cancel),
        }
    }
    /// Finishes setup. `id` must be stable for this account (so re-sign-in finds the same
    /// instance) and path-safe; `label` is what users see.
    pub fn done(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let instance = ProviderInstance {
            provider: self.provider.clone(),
            id: id.into(),
            label: label.into(),
        };
        (self.done)(instance, window, cx);
    }
    pub fn cancel(&self, window: &mut Window, cx: &mut App) {
        (self.cancel)(window, cx);
    }
}

pub type ProviderSetupFactory =
    Rc<dyn Fn(ShellHandle, ProviderSetup, &mut Window, &mut App) -> AnyView>;
pub type ProviderStatusFn = Rc<dyn Fn(&ProviderInstance, &App) -> ProviderStatus>;
pub type ProviderSignOutFn = Rc<dyn Fn(&ProviderInstance, &mut App)>;

#[derive(Clone)]
pub struct ProviderContribution {
    pub id: String,
    pub label: String,
    /// Several configured instances may coexist (advanced "Add another account").
    pub multiple: bool,
    pub setup: ProviderSetupFactory,
    pub status: ProviderStatusFn,
    /// Deletes the instance's credentials. The shell removes the instance record afterwards.
    pub sign_out: ProviderSignOutFn,
}

/// Configured instances and per-type selection; pure state, persisted by the registry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstanceStore {
    instances: Vec<ProviderInstance>,
    /// Instance chosen per provider when several exist.
    selected: BTreeMap<String, String>,
}
impl InstanceStore {
    pub fn from_storage(storage: &UiStorage) -> Self {
        let instances = storage
            .value(INSTANCES_KEY)
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        let selected = storage
            .value(SELECTED_KEY)
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        Self {
            instances,
            selected,
        }
    }
    pub fn write(&self, storage: &mut UiStorage) -> std::io::Result<()> {
        let instances = serde_json::to_string(&self.instances).unwrap_or_default();
        let selected = serde_json::to_string(&self.selected).unwrap_or_default();
        storage.set_value(INSTANCES_KEY, instances)?;
        storage.set_value(SELECTED_KEY, selected)
    }
    pub fn instances(&self, provider: &str) -> Vec<ProviderInstance> {
        self.instances
            .iter()
            .filter(|i| i.provider == provider)
            .cloned()
            .collect()
    }
    pub fn instance(&self, provider: &str, id: &str) -> Option<&ProviderInstance> {
        self.instances
            .iter()
            .find(|i| i.provider == provider && i.id == id)
    }
    /// The instance a mode uses: the explicitly selected one, else the only one.
    pub fn selected(&self, provider: &str) -> Option<ProviderInstance> {
        if let Some(id) = self.selected.get(provider)
            && let Some(instance) = self.instance(provider, id)
        {
            return Some(instance.clone());
        }
        let mut all = self.instances(provider);
        (all.len() == 1).then(|| all.remove(0))
    }
    pub fn select(&mut self, instance: &ProviderInstance) {
        self.selected
            .insert(instance.provider.clone(), instance.id.clone());
    }
    /// Adds or relabels an instance (same provider + id) and selects it.
    pub fn upsert(&mut self, instance: ProviderInstance) {
        match self
            .instances
            .iter_mut()
            .find(|i| i.provider == instance.provider && i.id == instance.id)
        {
            Some(existing) => existing.label = instance.label.clone(),
            None => self.instances.push(instance.clone()),
        }
        self.select(&instance);
    }
    /// Drops the record and its selection; returns whether it existed.
    pub fn remove(&mut self, instance: &ProviderInstance) -> bool {
        let before = self.instances.len();
        self.instances
            .retain(|i| !(i.provider == instance.provider && i.id == instance.id));
        if self.selected.get(&instance.provider) == Some(&instance.id) {
            self.selected.remove(&instance.provider);
        }
        before != self.instances.len()
    }
}

/// Shell-owned registry of provider types and configured instances. Persisted in
/// [`UiStorage`](studio_services::prefs::UiStorage) under `exaudio.providers.*`.
#[derive(Default)]
pub struct ProviderRegistry {
    types: BTreeMap<String, (String, ProviderContribution)>,
    store: InstanceStore,
}
impl Global for ProviderRegistry {}

impl ProviderRegistry {
    pub fn load(cx: &App) -> Self {
        Self {
            types: BTreeMap::new(),
            store: InstanceStore::from_storage(&cx.global::<AppServices>().storage),
        }
    }

    pub fn register_type(&mut self, owner: &str, provider: ProviderContribution) -> Result<()> {
        if !exaudio_extension::manifest::valid_id(&provider.id) {
            bail!("invalid provider identity: {}", provider.id);
        }
        if let Some((existing, _)) = self.types.get(&provider.id)
            && existing != owner
        {
            bail!("Provider {} is owned by {existing}", provider.id);
        }
        self.types
            .insert(provider.id.clone(), (owner.into(), provider));
        Ok(())
    }
    pub fn clear_types(&mut self, owner: &str) {
        self.types.retain(|_, (o, _)| o != owner);
    }
    pub fn provider_type(&self, id: &str) -> Option<&ProviderContribution> {
        self.types.get(id).map(|(_, p)| p)
    }
    pub fn types(&self) -> impl Iterator<Item = &ProviderContribution> {
        self.types.values().map(|(_, p)| p)
    }

    pub fn instances(&self, provider: &str) -> Vec<ProviderInstance> {
        self.store.instances(provider)
    }
    pub fn instance(&self, provider: &str, id: &str) -> Option<&ProviderInstance> {
        self.store.instance(provider, id)
    }
    /// The instance a mode uses: the explicitly selected one, else the only one.
    pub fn selected(&self, provider: &str) -> Option<ProviderInstance> {
        self.store.selected(provider)
    }
    pub fn select(&mut self, instance: &ProviderInstance, cx: &mut App) {
        self.store.select(instance);
        self.persist(cx);
    }
    /// Adds or relabels an instance (same provider + id) and selects it.
    pub fn upsert(&mut self, instance: ProviderInstance, cx: &mut App) {
        self.store.upsert(instance);
        self.persist(cx);
    }
    /// Runs the type's sign-out, deletes the instance record and its data directory.
    /// Call through `cx.update_global::<ProviderRegistry, _>`; `sign_out` must not read
    /// the registry.
    pub fn remove(&mut self, instance: &ProviderInstance, cx: &mut App) {
        if let Some(provider) = self.provider_type(&instance.provider).cloned() {
            (provider.sign_out)(instance, cx);
        }
        self.store.remove(instance);
        let _ = std::fs::remove_dir_all(instance_dir(instance));
        self.persist(cx);
    }
    fn persist(&self, cx: &mut App) {
        let store = self.store.clone();
        cx.update_global::<AppServices, _>(|services, _| {
            let _ = store.write(&mut services.storage);
        });
    }
}

/// Root of all provider instance directories. Resolution: `STUDIO_PROVIDER_DIR`, else
/// `<executable dir>/providers` when that directory exists (portable install), else
/// `<app support>/providers`.
pub fn providers_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("STUDIO_PROVIDER_DIR") {
        let trimmed = dir.to_string_lossy().trim().to_owned();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let portable = parent.join("providers");
        if portable.is_dir() {
            return portable;
        }
    }
    studio_services::paths::app_support_dir().join("providers")
}

/// Private directory for an instance's credentials and caches, owned by the provider type.
/// Created on demand; deleted when the instance is removed.
pub fn instance_dir(instance: &ProviderInstance) -> PathBuf {
    providers_root().join(&instance.provider).join(&instance.id)
}

#[cfg(test)]
mod tests {
    use super::{InstanceStore, ProviderInstance};
    use studio_services::prefs::UiStorage;

    fn instance(id: &str, label: &str) -> ProviderInstance {
        ProviderInstance {
            provider: "test.account".into(),
            id: id.into(),
            label: label.into(),
        }
    }

    #[test]
    fn selected_is_the_only_instance_or_the_explicit_choice() {
        let mut store = InstanceStore::default();
        assert_eq!(store.selected("test.account"), None);
        store.upsert(instance("a", "A"));
        assert_eq!(store.selected("test.account"), Some(instance("a", "A")));
        store.upsert(instance("b", "B"));
        assert_eq!(store.selected("test.account"), Some(instance("b", "B")));
        store.select(&instance("a", "A"));
        assert_eq!(store.selected("test.account"), Some(instance("a", "A")));
        assert_eq!(store.selected("other.account"), None);
    }

    #[test]
    fn upsert_relabels_without_duplicating_and_selects() {
        let mut store = InstanceStore::default();
        store.upsert(instance("a", "A"));
        store.upsert(instance("b", "B"));
        store.upsert(instance("a", "A renamed"));
        assert_eq!(
            store.instances("test.account"),
            vec![instance("a", "A renamed"), instance("b", "B")]
        );
        assert_eq!(
            store.selected("test.account"),
            Some(instance("a", "A renamed"))
        );
    }

    #[test]
    fn remove_clears_selection_and_falls_back_to_the_remaining_instance() {
        let mut store = InstanceStore::default();
        store.upsert(instance("a", "A"));
        store.upsert(instance("b", "B"));
        store.select(&instance("a", "A"));
        assert!(store.remove(&instance("a", "A")));
        assert!(!store.remove(&instance("a", "A")));
        assert_eq!(store.selected("test.account"), Some(instance("b", "B")));
        assert!(store.remove(&instance("b", "B")));
        assert_eq!(store.selected("test.account"), None);
        assert!(store.instances("test.account").is_empty());
    }

    #[test]
    fn store_round_trips_through_ui_storage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ui.json");
        let mut storage = UiStorage::load(&path).unwrap();
        let mut store = InstanceStore::default();
        store.upsert(instance("a", "A"));
        store.upsert(instance("b", "B"));
        store.select(&instance("a", "A"));
        store.write(&mut storage).unwrap();
        let reloaded = InstanceStore::from_storage(&UiStorage::load(&path).unwrap());
        assert_eq!(reloaded, store);
        assert_eq!(reloaded.selected("test.account"), Some(instance("a", "A")));
    }
}
