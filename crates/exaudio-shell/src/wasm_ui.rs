//! Typed component UI. Nodes never receive native handles or arbitrary HTML/URLs.
mod files;
mod overlay;
mod providers;
mod waveform;
use crate::screens::ui::localized;
use crate::{
    ShellHandle,
    contributions::{CommandContribution, ContributionRegistry, ModeContribution, PanelLocation},
};
use anyhow::{Result, anyhow, bail};
use exaudio_wasm::{Invocation, types as w};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme, Disableable};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
pub use overlay::timeline_overlay;
use std::{
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::Arc,
};

#[derive(Default)]
struct UiStore {
    owners: BTreeMap<String, OwnerUi>,
}
impl Global for UiStore {}
#[derive(Default)]
struct OwnerUi {
    registrations: BTreeMap<String, w::Registration>,
    trees: BTreeMap<String, Arc<w::UiTree>>,
    overlays: BTreeMap<String, w::Overlay>,
    progress: BTreeMap<u64, w::Progress>,
}
impl OwnerUi {
    fn update_progress(&mut self, value: w::Progress) -> Result<()> {
        if value.label.len() > 4096 {
            bail!("progress label exceeds limit");
        }
        if value.total.is_some_and(|total| value.completed >= total) {
            self.progress.remove(&value.task_id);
            return Ok(());
        }
        if !self.progress.contains_key(&value.task_id) && self.progress.len() >= 64 {
            bail!("extension task quota exceeded");
        }
        self.progress.insert(value.task_id, value);
        Ok(())
    }
}
fn init(cx: &mut App) {
    if !cx.has_global::<UiStore>() {
        cx.set_global(UiStore::default());
    }
}
fn namespaced(owner: &str, id: &str) -> String {
    format!("{owner}:{id}")
}
fn active_document(shell: &ShellHandle, cx: &App) -> Option<String> {
    shell
        .editor_model(cx)
        .map(|model| crate::wasm_host::document_handle(&model))
}
fn tree_bytes(tree: &w::UiTree) -> usize {
    tree.contribution.len()
        + tree
            .nodes
            .iter()
            .map(|node| {
                let config = match &node.config {
                    w::WidgetConfig::Input(v) => v.placeholder.len(),
                    w::WidgetConfig::Choices(v) => {
                        v.options
                            .iter()
                            .map(|o| o.key.len() + o.label.len() + 32)
                            .sum::<usize>()
                            + v.selected.as_ref().map_or(0, String::len)
                    }
                    w::WidgetConfig::Table(v) => {
                        v.columns
                            .iter()
                            .map(|c| c.key.len() + c.title.len() + 32)
                            .sum::<usize>()
                            + v.rows
                                .iter()
                                .map(|row| {
                                    row.key.len()
                                        + row
                                            .cells
                                            .iter()
                                            .map(|cell| cell.len() + 24)
                                            .sum::<usize>()
                                        + 32
                                })
                                .sum::<usize>()
                            + v.selected.as_ref().map_or(0, String::len)
                    }
                    w::WidgetConfig::Asset(v) => v.asset_id.len() + v.alt.len(),
                    w::WidgetConfig::Waveform(v) => v.document_id.len() + v.lane_id.len(),
                    w::WidgetConfig::Plain
                    | w::WidgetConfig::Layout(_)
                    | w::WidgetConfig::Numeric(_) => 0,
                };
                128 + node.id.len()
                    + node.text.len()
                    + node.value.len()
                    + node.command.as_ref().map_or(0, String::len)
                    + config
            })
            .sum::<usize>()
}
fn overlay_bytes(overlay: &w::Overlay) -> usize {
    overlay.provider.len()
        + overlay.document_id.len()
        + overlay
            .draws
            .iter()
            .map(|draw| {
                64 + match draw {
                    w::Draw::Label(v) => v.text.len(),
                    w::Draw::HitRegion(v) => v.id.len(),
                    w::Draw::Curve(v) => v.points.len() * 16,
                    _ => 0,
                }
            })
            .sum::<usize>()
}
pub(crate) fn preflight(owner: &str, outputs: &[exaudio_wasm::Output], cx: &mut App) -> Result<()> {
    use exaudio_wasm::Output;
    if outputs.len() > 256 {
        bail!("activation contribution batch exceeds quota");
    }
    let manifest = crate::wasm_host::pending_manifest(owner, cx)
        .ok_or_else(|| anyhow!("missing activation manifest"))?;
    let mut registrations = BTreeMap::new();
    let mut modes = vec![];
    let mut node_count = 0usize;
    let mut state = OwnerUi::default();
    let mut retained_ui_bytes = 0usize;
    let mut registration_bytes = 0usize;
    for output in outputs {
        match output {
            Output::Registered(registration) => {
                if registrations
                    .insert(&registration.id, registration.kind)
                    .is_some()
                {
                    bail!("duplicate contribution ID");
                }
                registration_bytes = registration_bytes
                    .saturating_add(registration.id.len() + registration.title.len());
                if registration_bytes > 256 * 1024 {
                    bail!("registration label quota exceeded");
                }
                if matches!(registration.kind, w::ContributionKind::Mode) {
                    if !manifest.modes.iter().any(|mode| mode.id == registration.id) {
                        bail!("mode was not declared in manifest");
                    }
                    modes.push(registration.id.clone());
                }
                if matches!(registration.kind, w::ContributionKind::Provider)
                    && !manifest
                        .providers
                        .iter()
                        .any(|provider| provider.id == registration.id)
                {
                    bail!("provider was not declared in manifest");
                }
            }
            Output::Ui(tree) => {
                validate(tree)?;
                if !registrations.get(&tree.contribution).is_some_and(|kind| {
                    matches!(
                        kind,
                        w::ContributionKind::Panel
                            | w::ContributionKind::Inspector
                            | w::ContributionKind::Mode
                            | w::ContributionKind::Provider
                    )
                }) {
                    bail!("unregistered UI contribution");
                }
                node_count = node_count.saturating_add(tree.nodes.len());
                retained_ui_bytes = retained_ui_bytes.saturating_add(tree_bytes(tree));
                if retained_ui_bytes > 2 * 1024 * 1024 {
                    bail!("aggregate UI byte quota exceeded");
                }
                if node_count > 8192 {
                    bail!("aggregate UI node quota exceeded");
                }
            }
            Output::Progress(value) => state.update_progress(value.clone())?,
            Output::ModeDocument(value) => {
                crate::wasm_host::document::project(&value.document)?;
            }
            Output::Import(_)
            | Output::Export(_)
            | Output::ProviderSetupDone(_)
            | Output::ProviderSetupCancelled { .. }
            | Output::ProviderStatus(_) => {
                bail!("unsolicited request response during activation")
            }
            Output::Activated | Output::Removed => bail!("guest cannot publish lifecycle markers"),
            Output::Diagnostics(_)
            | Output::Decorations(_)
            | Output::Completions(_)
            | Output::Overlay(_) => {}
        }
    }
    cx.update_global::<ContributionRegistry, _>(|registry, _| registry.reserve_modes(owner, &modes))
}
pub(crate) fn register(
    owner: &str,
    shell: ShellHandle,
    registration: w::Registration,
    cx: &mut App,
) -> Result<()> {
    init(cx);
    let id = registration.id.clone();
    let title = registration.title.clone();
    let key = namespaced(owner, &id);
    if cx
        .global::<UiStore>()
        .owners
        .get(owner)
        .is_some_and(|o| o.registrations.contains_key(&id))
    {
        bail!("duplicate contribution ID");
    }
    let registration_bytes = cx.global::<UiStore>().owners.get(owner).map_or(0, |state| {
        state
            .registrations
            .values()
            .map(|value| value.id.len() + value.title.len())
            .sum::<usize>()
    });
    if registration_bytes.saturating_add(id.len() + title.len()) > 256 * 1024 {
        bail!("registration label quota exceeded");
    }
    let owner_string = owner.to_owned();
    match registration.kind {
        w::ContributionKind::Command | w::ContributionKind::Menu => {
            let guest_id = id.clone();
            let owner = owner_string.clone();
            cx.update_global::<ContributionRegistry, _>(|registry, _| {
                registry.register_command(
                    &owner_string,
                    CommandContribution {
                        id: key,
                        label: title,
                        shortcut: None,
                        toolbar: true,
                        menu: true,
                        run: Rc::new(move |shell, _, cx| {
                            let document_id = active_document(&shell, cx);
                            crate::wasm_host::invoke(
                                &owner,
                                Invocation::Command(w::Command {
                                    id: guest_id.clone(),
                                    document_id,
                                    selection: None,
                                    arguments: vec![],
                                }),
                                cx,
                            );
                        }),
                    },
                )
            })?;
        }
        w::ContributionKind::Panel | w::ContributionKind::Inspector => {
            let location = if matches!(registration.kind, w::ContributionKind::Inspector) {
                PanelLocation::Inspector
            } else {
                PanelLocation::Sidebar
            };
            let owner = owner_string.clone();
            let panel = id.clone();
            cx.update_global::<ContributionRegistry, _>(|registry, _| {
                registry.register_panel(
                    &owner_string,
                    &key,
                    &title,
                    location,
                    Rc::new(move |shell, window, cx| {
                        cx.new(|cx| {
                            WasmPanel::new(owner.clone(), panel.clone(), shell, true, window, cx)
                        })
                        .into()
                    }),
                )
            })?;
        }
        w::ContributionKind::Mode => {
            let mode = id.clone();
            let owner = owner_string.clone();
            let description = localized(cx, "Extension workspace", "Пространство расширения");
            cx.update_global::<ContributionRegistry, _>(|registry, _| {
                registry.register_mode(
                    &owner_string,
                    ModeContribution {
                        id: id.clone(),
                        label: title,
                        description: description.into(),
                        launch: Rc::new(move |shell, context, window, cx| {
                            providers::launch_mode(&owner, &mode, shell, context, window, cx)
                        }),
                    },
                )
            })?;
        }
        w::ContributionKind::Importer | w::ContributionKind::Exporter => {
            files::register(owner, &registration, cx)?
        }
        w::ContributionKind::Provider => providers::register(owner, &registration, cx)?,
        w::ContributionKind::Diagnostics
        | w::ContributionKind::Completions
        | w::ContributionKind::Decorations
        | w::ContributionKind::Overlay
        | w::ContributionKind::Event => {}
    }
    cx.update_global::<UiStore, _>(|store, _| {
        store
            .owners
            .entry(owner_string)
            .or_default()
            .registrations
            .insert(id.clone(), registration);
    });
    crate::wasm_host::contribution_added(owner, &id, cx);
    let _ = shell;
    Ok(())
}
pub(crate) fn publish(owner: &str, tree: w::UiTree, cx: &mut App) -> Result<()> {
    validate(&tree)?;
    init(cx);
    let known = cx
        .global::<UiStore>()
        .owners
        .get(owner)
        .and_then(|o| o.registrations.get(&tree.contribution))
        .ok_or_else(|| anyhow!("unregistered UI contribution"))?;
    if !matches!(
        known.kind,
        w::ContributionKind::Panel
            | w::ContributionKind::Inspector
            | w::ContributionKind::Mode
            | w::ContributionKind::Provider
    ) {
        bail!("contribution cannot own UI");
    }
    cx.update_global::<UiStore, _>(|store, _| {
        let owner = store.owners.get_mut(owner).expect("registered owner");
        let nodes = owner
            .trees
            .iter()
            .filter(|(id, _)| *id != &tree.contribution)
            .map(|(_, tree)| tree.nodes.len())
            .sum::<usize>();
        if nodes.saturating_add(tree.nodes.len()) > 8192 {
            bail!("aggregate UI node quota exceeded");
        }
        let bytes = owner
            .trees
            .iter()
            .filter(|(id, _)| *id != &tree.contribution)
            .map(|(_, tree)| tree_bytes(tree))
            .sum::<usize>();
        if bytes.saturating_add(tree_bytes(&tree)) > 2 * 1024 * 1024 {
            bail!("aggregate UI byte quota exceeded");
        }
        owner
            .trees
            .insert(tree.contribution.clone(), Arc::new(tree));
        Ok(())
    })?;
    cx.refresh_windows();
    Ok(())
}
fn validate(tree: &w::UiTree) -> Result<()> {
    if tree.nodes.len() > 4096 {
        bail!("UI tree too large");
    }
    let mut ids = BTreeSet::new();
    let mut depths = Vec::with_capacity(tree.nodes.len());
    for (index, node) in tree.nodes.iter().enumerate() {
        if node.id.is_empty() || node.id.len() > 256 || !ids.insert(&node.id) {
            bail!("empty or duplicate UI node ID");
        }
        let depth = match node.parent {
            Some(parent) if (parent as usize) < index => depths[parent as usize] + 1,
            Some(_) => bail!("parent must precede child"),
            None => 0,
        };
        if depth > 32 {
            bail!("UI nesting limit exceeded");
        }
        depths.push(depth);
        let valid = match node.widget {
            w::Widget::Stack
            | w::Widget::Row
            | w::Widget::Scroll
            | w::Widget::Split
            | w::Widget::ListView => matches!(
                node.config,
                w::WidgetConfig::Layout(_) | w::WidgetConfig::Plain
            ),
            w::Widget::TextInput => matches!(node.config, w::WidgetConfig::Input(_)),
            w::Widget::Number | w::Widget::Slider => {
                matches!(node.config, w::WidgetConfig::Numeric(_))
            }
            w::Widget::Tabs | w::Widget::Select => {
                matches!(node.config, w::WidgetConfig::Choices(_))
            }
            w::Widget::Table => matches!(node.config, w::WidgetConfig::Table(_)),
            w::Widget::Image | w::Widget::Icon => matches!(node.config, w::WidgetConfig::Asset(_)),
            w::Widget::Waveform => matches!(node.config, w::WidgetConfig::Waveform(_)),
            w::Widget::Text
            | w::Widget::Markdown
            | w::Widget::Button
            | w::Widget::Toggle
            | w::Widget::Progress
            | w::Widget::Badge
            | w::Widget::Callout => matches!(node.config, w::WidgetConfig::Plain),
        };
        if !valid {
            bail!("widget/config mismatch for {}", node.id);
        }
        if let w::WidgetConfig::Numeric(v) = &node.config
            && (!(v.min as f32).is_finite()
                || !(v.max as f32).is_finite()
                || !(v.step as f32).is_finite()
                || v.max <= v.min
                || v.step <= 0.)
        {
            bail!("invalid numeric widget bounds");
        }
        if let w::WidgetConfig::Table(v) = &node.config {
            let mut rows = BTreeSet::new();
            if v.columns.len() > 64
                || v.rows.len() > 4096
                || v.rows
                    .iter()
                    .any(|row| row.cells.len() != v.columns.len() || !rows.insert(&row.key))
            {
                bail!("invalid table dimensions or row keys");
            }
        }
        if let w::WidgetConfig::Choices(v) = &node.config {
            let mut keys = BTreeSet::new();
            if v.options.len() > 4096 || v.options.iter().any(|v| !keys.insert(&v.key)) {
                bail!("invalid choice keys");
            }
        }
        if matches!(node.widget, w::Widget::Tabs)
            && let w::WidgetConfig::Choices(choices) = &node.config
            && choices.options.iter().any(|choice| {
                !tree
                    .nodes
                    .iter()
                    .any(|child| child.parent == Some(index as u32) && child.id == choice.key)
            })
        {
            bail!("tab option does not identify a child");
        }
    }
    Ok(())
}
pub(crate) fn remove(owner: &str, cx: &mut App) {
    files::remove(owner, cx);
    providers::remove(owner, cx);
    if cx.has_global::<UiStore>() {
        cx.update_global::<UiStore, _>(|store, _| {
            store.owners.remove(owner);
        });
    }
    cx.refresh_windows();
}
pub(crate) fn clear_document(owner: &str, document: &str, cx: &mut App) {
    if cx.has_global::<UiStore>() {
        cx.update_global::<UiStore, _>(|store, _| {
            if let Some(state) = store.owners.get_mut(owner) {
                state
                    .overlays
                    .retain(|_, overlay| overlay.document_id != document);
            }
        });
    }
}
pub(crate) fn progress(owner: &str, value: w::Progress, cx: &mut App) -> Result<()> {
    init(cx);
    cx.update_global::<UiStore, _>(|store, _| {
        store
            .owners
            .entry(owner.into())
            .or_default()
            .update_progress(value)
    })?;
    cx.refresh_windows();
    Ok(())
}
pub(crate) fn publish_overlay(
    owner: &str,
    shell: &ShellHandle,
    value: w::Overlay,
    cx: &mut App,
) -> Result<()> {
    crate::wasm_host::document::current(shell, &value.document_id, value.revision, cx)?;
    init(cx);
    cx.update_global::<UiStore, _>(|store, _| {
        let state = store.owners.entry(owner.into()).or_default();
        let draws = state
            .overlays
            .iter()
            .filter(|(id, _)| *id != &value.provider)
            .map(|(_, overlay)| overlay.draws.len())
            .sum::<usize>();
        if draws.saturating_add(value.draws.len()) > 8192 {
            bail!("aggregate overlay quota exceeded");
        }
        let bytes = state
            .overlays
            .iter()
            .filter(|(id, _)| *id != &value.provider)
            .map(|(_, overlay)| overlay_bytes(overlay))
            .sum::<usize>();
        if bytes.saturating_add(overlay_bytes(&value)) > 2 * 1024 * 1024 {
            bail!("aggregate overlay byte quota exceeded");
        }
        state.overlays.insert(value.provider.clone(), value);
        Ok(())
    })?;
    cx.refresh_windows();
    Ok(())
}
pub(crate) fn mode_document(
    owner: &str,
    shell: &ShellHandle,
    value: w::ModeDocument,
    cx: &mut App,
) -> Result<()> {
    init(cx);
    let registered = cx
        .global::<UiStore>()
        .owners
        .get(owner)
        .and_then(|o| o.registrations.get(&value.mode));
    if !registered.is_some_and(|r| matches!(r.kind, w::ContributionKind::Mode)) {
        bail!("unregistered mode result");
    }
    if crate::wasm_host::document::project(&value.document)?.estimated_heap_bytes()
        > 2 * 1024 * 1024
    {
        bail!("mode document exceeds retained byte quota");
    }
    providers::open_mode_document(owner, shell, value, cx)
}
pub(crate) fn import_result(owner: &str, value: w::ImportResult, cx: &mut App) -> Result<()> {
    files::accept_import(owner, value, cx)
}
pub(crate) fn export_result(owner: &str, value: w::ExportResult, cx: &mut App) -> Result<()> {
    files::accept_export(owner, value, cx)
}
pub(crate) fn provider_setup_done(
    owner: &str,
    value: w::ProviderSetupResult,
    cx: &mut App,
) -> Result<()> {
    providers::setup_done(owner, value, cx)
}
pub(crate) fn provider_setup_cancelled(
    owner: &str,
    provider: &str,
    request_id: u64,
    cx: &mut App,
) -> Result<()> {
    providers::setup_cancelled(owner, provider, request_id, cx)
}
pub(crate) fn request_failed(owner: &str, request: u64, cx: &mut App) {
    files::request_failed(owner, request, cx);
    providers::request_failed(owner, request, cx);
}

// Trees are validated parent-before-child. Hide inactive tab subtrees before
// constructing widgets, so hidden content consumes neither layout nor render work.
fn visible_nodes(tree: &w::UiTree, values: &BTreeMap<String, String>) -> Vec<bool> {
    let mut visible: Vec<bool> = Vec::with_capacity(tree.nodes.len());
    for node in &tree.nodes {
        let shown = node.parent.is_none_or(|parent| {
            let parent = parent as usize;
            if !visible[parent] {
                return false;
            }
            let parent = &tree.nodes[parent];
            match parent.widget {
                w::Widget::Stack
                | w::Widget::Row
                | w::Widget::Scroll
                | w::Widget::Split
                | w::Widget::ListView => return true,
                w::Widget::Tabs => {}
                _ => return false,
            }
            let w::WidgetConfig::Choices(config) = &parent.config else {
                return false;
            };
            values
                .get(&parent.id)
                .filter(|value| !value.is_empty())
                .or(config.selected.as_ref())
                .is_some_and(|selected| selected == &node.id)
        });
        visible.push(shown);
    }
    visible
}

struct WasmPanel {
    owner: String,
    contribution: String,
    shell: ShellHandle,
    docked: bool,
    inputs: BTreeMap<String, Entity<InputState>>,
    textareas: BTreeMap<String, Entity<TextareaState>>,
    sliders: BTreeMap<String, Entity<SliderState>>,
    waveforms: BTreeMap<String, Entity<waveform::WaveformWidget>>,
    subscriptions: Vec<Subscription>,
    values: BTreeMap<String, String>,
    tree_signature: Vec<(String, String)>,
    synced_tree: Option<Arc<w::UiTree>>,
}
impl WasmPanel {
    fn new(
        owner: String,
        contribution: String,
        shell: ShellHandle,
        docked: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            owner,
            contribution,
            shell,
            docked,
            inputs: BTreeMap::new(),
            textareas: BTreeMap::new(),
            sliders: BTreeMap::new(),
            waveforms: BTreeMap::new(),
            subscriptions: vec![],
            values: BTreeMap::new(),
            tree_signature: vec![],
            synced_tree: None,
        }
    }
    fn event(&self, node: &w::UiNode, kind: w::UiEventKind, value: String, cx: &mut App) {
        if !node.enabled {
            return;
        }
        let document_id = active_document(&self.shell, cx);
        if matches!(kind, w::UiEventKind::Click | w::UiEventKind::Submit)
            && let Some(command) = &node.command
        {
            let arguments = vec![
                w::Property {
                    key: "contribution".into(),
                    value: w::Scalar::Text(self.contribution.clone()),
                },
                w::Property {
                    key: "node".into(),
                    value: w::Scalar::Text(node.id.clone()),
                },
                w::Property {
                    key: "value".into(),
                    value: w::Scalar::Text(value),
                },
            ];
            crate::wasm_host::invoke(
                &self.owner,
                Invocation::Command(w::Command {
                    id: command.clone(),
                    document_id,
                    selection: None,
                    arguments,
                }),
                cx,
            );
            return;
        }
        crate::wasm_host::invoke(
            &self.owner,
            Invocation::Ui(w::UiEvent {
                contribution: self.contribution.clone(),
                node: node.id.clone(),
                kind,
                value,
                document_id,
            }),
            cx,
        );
    }
    fn sync(&mut self, tree: &Arc<w::UiTree>, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .synced_tree
            .as_ref()
            .is_some_and(|previous| Arc::ptr_eq(previous, tree))
        {
            return;
        }
        self.synced_tree = Some(tree.clone());
        let signature = tree
            .nodes
            .iter()
            .map(|n| {
                (
                    n.id.clone(),
                    format!(
                        "{:?}:{:?}:{}:{:?}",
                        n.widget, n.config, n.enabled, n.command
                    ),
                )
            })
            .collect::<Vec<_>>();
        if signature == self.tree_signature {
            for node in &tree.nodes {
                if let Some(input) = self.inputs.get(&node.id)
                    && input.read(cx).value().as_ref() != node.value
                {
                    input.update(cx, |input, cx| {
                        input.set_value(node.value.clone(), window, cx)
                    });
                }
                if let Some(input) = self.textareas.get(&node.id)
                    && input.read(cx).value().as_ref() != node.value
                {
                    input.update(cx, |input, cx| {
                        input.set_value(node.value.clone(), window, cx)
                    });
                }
                if self.values.get(&node.id) != Some(&node.value)
                    && let Some(slider) = self.sliders.get(&node.id)
                    && let Ok(value) = node.value.parse::<f32>()
                {
                    slider.update(cx, |slider, cx| slider.set_value(value, window, cx));
                }
                self.values.insert(node.id.clone(), node.value.clone());
            }
            return;
        }
        self.inputs.clear();
        self.textareas.clear();
        self.sliders.clear();
        self.waveforms.clear();
        self.subscriptions.clear();
        self.values.clear();
        self.tree_signature = signature;
        for node in &tree.nodes {
            self.values.insert(node.id.clone(), node.value.clone());
            if let w::WidgetConfig::Waveform(config) = &node.config {
                let waveform = cx.new(|cx| {
                    waveform::WaveformWidget::new(
                        self.owner.clone(),
                        self.shell.clone(),
                        config.clone(),
                        cx,
                    )
                });
                self.waveforms.insert(node.id.clone(), waveform);
            }
            if matches!(node.widget, w::Widget::TextInput | w::Widget::Number) {
                let placeholder = match &node.config {
                    w::WidgetConfig::Input(v) => v.placeholder.clone(),
                    _ => String::new(),
                };
                if let w::WidgetConfig::Input(config) = &node.config
                    && config.multiline
                {
                    let input = cx.new(|cx| {
                        TextareaState::new(window, cx)
                            .placeholder(placeholder)
                            .rows(4)
                            .default_value(node.value.clone())
                    });
                    let node_id = node.id.clone();
                    let node = node.clone();
                    self.subscriptions.push(cx.subscribe(
                        &input,
                        move |this, input, event: &InputEvent, cx| {
                            if matches!(event, InputEvent::Change)
                                && !matches!(&node.config,w::WidgetConfig::Input(v) if v.read_only)
                            {
                                this.event(
                                    &node,
                                    w::UiEventKind::Change,
                                    input.read(cx).value().to_string(),
                                    cx,
                                );
                            }
                        },
                    ));
                    self.textareas.insert(node_id, input);
                    continue;
                }
                let input = cx.new(|cx| {
                    let mut state = InputState::new(window, cx).placeholder(placeholder);
                    state.set_value(node.value.clone(), window, cx);
                    state
                });
                let node_id = node.id.clone();
                let node = node.clone();
                self.subscriptions.push(cx.subscribe(
                    &input,
                    move |this, input, event: &InputEvent, cx| {
                        let kind = match event {
                            InputEvent::Change => w::UiEventKind::Change,
                            InputEvent::PressEnter { .. } => w::UiEventKind::Submit,
                            _ => return,
                        };
                        let value = input.read(cx).value().to_string();
                        if let w::WidgetConfig::Input(v) = &node.config
                            && v.read_only
                        {
                            return;
                        }
                        if let w::WidgetConfig::Numeric(v) = &node.config
                            && !value
                                .parse::<f64>()
                                .is_ok_and(|n| n.is_finite() && n >= v.min && n <= v.max)
                        {
                            return;
                        }
                        this.event(&node, kind, value, cx);
                    },
                ));
                self.inputs.insert(node_id, input);
            } else if let w::WidgetConfig::Numeric(config) = &node.config {
                let state = cx.new(|_| {
                    SliderState::new()
                        .min(config.min as f32)
                        .max(config.max as f32)
                        .step(config.step as f32)
                        .default_value(node.value.parse::<f32>().unwrap_or(config.min as f32))
                });
                let node_id = node.id.clone();
                let node = node.clone();
                self.subscriptions.push(cx.subscribe(
                    &state,
                    move |this, _, event: &SliderEvent, cx| {
                        if let SliderEvent::Change(value) = event {
                            this.event(&node, w::UiEventKind::Change, value.end().to_string(), cx);
                        }
                    },
                ));
                self.sliders.insert(node_id, state);
            }
        }
    }
    fn node(
        &self,
        node: &w::UiNode,
        children: Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id: SharedString =
            namespaced(&self.owner, &format!("{}:{}", self.contribution, node.id)).into();
        let text: SharedString = node.text.clone().into();
        match node.widget {
            w::Widget::Stack
            | w::Widget::Row
            | w::Widget::Scroll
            | w::Widget::Split
            | w::Widget::ListView => {
                let mut element = div().id(id).flex().min_w_0();
                if matches!(node.widget, w::Widget::Row | w::Widget::Split) {
                    element = element.flex_row();
                } else {
                    element = element.flex_col();
                }
                if let w::WidgetConfig::Layout(config) = &node.config {
                    element = element
                        .gap(px(config.gap as f32))
                        .p(px(config.padding as f32));
                }
                if matches!(node.widget, w::Widget::Scroll | w::Widget::ListView) {
                    element = element.overflow_y_scroll().max_h(rems(36.));
                }
                for child in children.into_iter().rev() {
                    element = element.child(div().flex_1().min_w_0().child(child));
                }
                element.into_any_element()
            }
            w::Widget::Text => div().child(text).into_any_element(),
            w::Widget::Markdown => {
                let assets = crate::wasm_host::assets(&self.owner, cx);
                let owner = self.owner.clone();
                let contribution = self.contribution.clone();
                let node = node.clone();
                TextView::markdown(id, text)
                    .image_source(move |uri| {
                        assets
                            .get(&uri.to_string())
                            .map(|path| ImageSource::from(path.clone()))
                            .unwrap_or_else(|| {
                                ImageSource::Resource(Resource::Embedded(
                                    "wasm-unregistered-package-asset".into(),
                                ))
                            })
                    })
                    .on_link_click(move |url, _, _, cx| {
                        if node.enabled {
                            crate::wasm_host::invoke(
                                &owner,
                                Invocation::Ui(w::UiEvent {
                                    contribution: contribution.clone(),
                                    node: node.id.clone(),
                                    kind: w::UiEventKind::Click,
                                    value: url.to_string(),
                                    document_id: cx
                                        .try_global::<ShellHandle>()
                                        .and_then(|shell| active_document(shell, cx)),
                                }),
                                cx,
                            );
                        }
                    })
                    .into_any_element()
            }
            w::Widget::Button => {
                let node = node.clone();
                Button::new(id)
                    .label(text)
                    .disabled(!node.enabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.event(&node, w::UiEventKind::Click, String::new(), cx)
                    }))
                    .into_any_element()
            }
            w::Widget::Toggle => {
                let checked = self.values.get(&node.id).is_some_and(|v| v == "true");
                let node = node.clone();
                Button::new(id)
                    .label(format!("{} {}", if checked { "☑" } else { "☐" }, text))
                    .disabled(!node.enabled)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let value = (!checked).to_string();
                        this.values.insert(node.id.clone(), value.clone());
                        this.event(&node, w::UiEventKind::Change, value, cx);
                        cx.notify();
                    }))
                    .into_any_element()
            }
            w::Widget::TextInput | w::Widget::Number => {
                let disabled = !node.enabled
                    || matches!(&node.config,w::WidgetConfig::Input(v) if v.read_only);
                if let Some(input) = self.textareas.get(&node.id) {
                    Textarea::new(input).disabled(disabled).into_any_element()
                } else {
                    self.inputs
                        .get(&node.id)
                        .map(|input| Input::new(input).disabled(disabled).into_any_element())
                        .unwrap_or_else(|| div().into_any_element())
                }
            }
            w::Widget::Slider => self
                .sliders
                .get(&node.id)
                .map(|slider| {
                    Slider::new(slider)
                        .horizontal()
                        .disabled(!node.enabled)
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
            w::Widget::Select | w::Widget::Tabs => {
                let w::WidgetConfig::Choices(config) = &node.config else {
                    return div().into_any_element();
                };
                let selected = self
                    .values
                    .get(&node.id)
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .or(config.selected.clone());
                let mut choices = div().flex().flex_row().flex_wrap().gap_1();
                for option in &config.options {
                    let key = option.key.clone();
                    let n = node.clone();
                    choices = choices.child(
                        Button::new(SharedString::from(format!("{id}:{key}")))
                            .label(option.label.clone())
                            .disabled(!node.enabled)
                            .when(selected.as_ref() == Some(&key), |button| button.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.values.insert(n.id.clone(), key.clone());
                                this.event(&n, w::UiEventKind::Change, key.clone(), cx);
                                cx.notify();
                            })),
                    );
                }
                let mut element = div().flex().flex_col().gap_2().child(choices);
                if matches!(node.widget, w::Widget::Tabs) {
                    element = element.children(children.into_iter().rev());
                }
                element.into_any_element()
            }
            w::Widget::Table => {
                let w::WidgetConfig::Table(config) = &node.config else {
                    return div().into_any_element();
                };
                let selected = self
                    .values
                    .get(&node.id)
                    .filter(|value| !value.is_empty())
                    .or(config.selected.as_ref());
                let mut table = div()
                    .id(id.clone())
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .max_h(rems(32.));
                let mut header = div().flex().font_weight(FontWeight::BOLD);
                for column in &config.columns {
                    header = header.child(div().flex_1().p_1().child(column.title.clone()));
                }
                table = table.child(header);
                for row in &config.rows {
                    let key = row.key.clone();
                    let node = node.clone();
                    let is_selected = selected == Some(&key);
                    let mut rendered = div()
                        .id(SharedString::from(format!("{id}:{key}")))
                        .flex()
                        .when(is_selected, |row| row.bg(cx.theme().accent))
                        .when(node.enabled, |row| row.cursor_pointer())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if node.enabled {
                                this.values.insert(node.id.clone(), key.clone());
                                this.event(&node, w::UiEventKind::RowSelect, key.clone(), cx);
                                cx.notify();
                            }
                        }));
                    for cell in &row.cells {
                        rendered = rendered.child(div().flex_1().p_1().child(cell.clone()));
                    }
                    table = table.child(rendered);
                }
                table.into_any_element()
            }
            w::Widget::Image | w::Widget::Icon => {
                let w::WidgetConfig::Asset(config) = &node.config else {
                    return div().into_any_element();
                };
                let assets = crate::wasm_host::assets(&self.owner, cx);
                match assets.get(&config.asset_id) {
                    Some(path) => img(path.clone())
                        .w(px(config.width.min(2048) as f32))
                        .h(px(config.height.min(2048) as f32))
                        .into_any_element(),
                    None => div().child(config.alt.clone()).into_any_element(),
                }
            }
            w::Widget::Waveform => self
                .waveforms
                .get(&node.id)
                .map(|waveform| waveform.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
            w::Widget::Progress => {
                let fraction = node.value.parse::<f32>().unwrap_or(0.).clamp(0., 1.);
                div()
                    .flex()
                    .flex_col()
                    .child(text)
                    .child(
                        div()
                            .h(px(6.))
                            .bg(cx.theme().muted)
                            .child(div().h_full().w(relative(fraction)).bg(cx.theme().primary)),
                    )
                    .into_any_element()
            }
            w::Widget::Badge => div()
                .rounded_md()
                .px_2()
                .py_1()
                .bg(cx.theme().muted)
                .child(text)
                .into_any_element(),
            w::Widget::Callout => div()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .p_3()
                .child(text)
                .into_any_element(),
        }
    }
}
impl Render for WasmPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tree = if cx.has_global::<UiStore>() {
            cx.global::<UiStore>()
                .owners
                .get(&self.owner)
                .and_then(|o| o.trees.get(&self.contribution))
                .cloned()
        } else {
            None
        };
        let Some(tree) = tree else {
            return div()
                .p_3()
                .child(localized(
                    cx,
                    "Waiting for the extension…",
                    "Ожидание расширения…",
                ))
                .into_any_element();
        };
        self.sync(&tree, window, cx);
        let mut result = div()
            .id(SharedString::from(namespaced(
                &self.owner,
                &self.contribution,
            )))
            .flex()
            .flex_col()
            .size_full()
            .when(self.docked, |element| {
                element
                    .w(rems(20.))
                    .max_w(window.viewport_size().width * 0.4)
                    .flex_none()
            })
            .overflow_y_scroll()
            .gap_2()
            .p_2();
        let visible = visible_nodes(&tree, &self.values);
        let mut children: Vec<Vec<AnyElement>> =
            (0..tree.nodes.len()).map(|_| Vec::new()).collect();
        let mut roots = Vec::new();
        // Children follow their parent in the wire format. Build in reverse,
        // then restore sibling order without recursively entering widget render.
        for (index, node) in tree.nodes.iter().enumerate().rev() {
            if !visible[index] {
                continue;
            }
            let element = self.node(node, std::mem::take(&mut children[index]), cx);
            if let Some(parent) = node.parent {
                children[parent as usize].push(element);
            } else {
                roots.push(element);
            }
        }
        result = result.children(roots.into_iter().rev());
        if let Some(owner) = cx.global::<UiStore>().owners.get(&self.owner) {
            let counted = localized(
                cx,
                "{label}: {done} of {total}",
                "{label}: {done} из {total}",
            );
            for progress in owner.progress.values() {
                result = result.child(
                    div().text_sm().child(match progress.total {
                        Some(total) => counted
                            .replace("{label}", &progress.label)
                            .replace("{done}", &progress.completed.to_string())
                            .replace("{total}", &total.to_string()),
                        None => format!("{}: {}", progress.label, progress.completed),
                    }),
                );
            }
        }
        result.into_any_element()
    }
}

/// Byte ranges remain byte ranges through shaping; core character selections are
/// translated only at the document boundary, never by indexing a UTF-8 string.
pub fn transcript_decorations(
    doc: &studio_editor::edit::EditorDoc,
    segment_id: &str,
    cx: &App,
) -> Vec<(std::ops::Range<usize>, HighlightStyle, String)> {
    let Some(segment) = doc.project().segment(segment_id) else {
        return vec![];
    };
    if doc
        .draft(segment_id)
        .is_some_and(|draft| draft.text() != segment.text)
    {
        return vec![];
    }
    doc.session()
        .decorations(doc.project())
        .into_iter()
        .filter_map(|decoration| {
            if decoration.segment_id != segment_id {
                return None;
            }
            let class = decoration.kind.strip_prefix("wasm-text:")?;
            let range = decoration.range?;
            if range.start > range.end
                || range.end > segment.text.len()
                || !segment.text.is_char_boundary(range.start)
                || !segment.text.is_char_boundary(range.end)
            {
                return None;
            }
            let color = match class {
                "error" => cx.theme().danger,
                "warning" => cx.theme().warning,
                "info" | "hint" => cx.theme().muted_foreground,
                _ => cx.theme().primary,
            };
            Some((
                range.start..range.end,
                HighlightStyle {
                    background_color: Some(color.opacity(0.14)),
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: Some(color),
                        wavy: class == "error" || class == "warning",
                    }),
                    ..Default::default()
                },
                decoration.label,
            ))
        })
        .collect()
}

/// GPUI's run shaper expects disjoint ranges. Sweep arbitrary overlapping provider
/// highlights into disjoint spans; later providers win without duplicating text runs.
pub fn disjoint_highlights(
    highlights: Vec<(std::ops::Range<usize>, HighlightStyle)>,
) -> Vec<(std::ops::Range<usize>, HighlightStyle)> {
    let mut events = Vec::with_capacity(highlights.len() * 2);
    for (index, (range, _)) in highlights.iter().enumerate() {
        if range.start < range.end {
            events.push((range.start, true, index));
            events.push((range.end, false, index));
        }
    }
    events.sort_unstable();
    let mut active = BTreeSet::<usize>::new();
    let mut previous = 0;
    let mut result = Vec::new();
    for (position, start, index) in events {
        if previous < position
            && let Some(index) = active.last()
        {
            result.push((previous..position, highlights[*index].1));
        }
        if start {
            active.insert(index);
        } else {
            active.remove(&index);
        }
        previous = position;
    }
    result
}

#[cfg(test)]
mod tests;
