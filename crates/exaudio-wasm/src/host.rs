use crate::{
    CallContext, HostServices, Limits, Output, bindings::exaudio::extension::host::Host, types::*,
};
use exaudio_extension::Permission;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use wasmtime::{StoreLimits, StoreLimitsBuilder};

pub(crate) struct State {
    pub id: String,
    pub services: Arc<dyn HostServices>,
    pub grants: BTreeSet<Permission>,
    pub limits: Limits,
    pub memory: StoreLimits,
    pub cancellation: Arc<AtomicBool>,
    pub deadline: Instant,
    pub staging: bool,
    pub suppress: bool,
    pub staged: Vec<Output>,
    pub registrations: BTreeMap<String, ContributionKind>,
    /// Provider types declared in the manifest: the only instances whose per-instance
    /// secrets/storage this extension may touch, and the only `provider` registrations allowed.
    pub providers: BTreeSet<String>,
    pub ui_nodes: BTreeMap<String, BTreeSet<String>>,
    pub remaining: usize,
}
impl State {
    pub fn new(
        id: String,
        services: Arc<dyn HostServices>,
        grants: BTreeSet<Permission>,
        providers: BTreeSet<String>,
        limits: Limits,
        cancellation: Arc<AtomicBool>,
    ) -> Self {
        // Component adapters can instantiate small shim modules besides the guest.
        // Only one linear memory is allowed, so its byte cap is also the total cap.
        Self {
            id,
            services,
            grants,
            memory: StoreLimitsBuilder::new()
                .memory_size(limits.memory_bytes)
                .instances(16)
                .memories(1)
                .tables(8)
                .table_elements(10_000)
                .trap_on_grow_failure(true)
                .build(),
            cancellation,
            deadline: Instant::now(),
            staging: true,
            suppress: false,
            staged: Vec::new(),
            registrations: BTreeMap::new(),
            providers,
            ui_nodes: BTreeMap::new(),
            remaining: limits.output_bytes,
            limits,
        }
    }
    pub(crate) fn cancelled_now(&self) -> bool {
        self.cancellation.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }
    fn cx(&self) -> CallContext<'_> {
        CallContext {
            extension_id: &self.id,
            deadline: self.deadline,
            cancellation: &self.cancellation,
        }
    }
    fn check(&self, permission: &Permission) -> Result<(), String> {
        self.cx().check()?;
        if !self.grants.contains(permission) {
            return Err(format!("permission denied: {permission:?}"));
        }
        Ok(())
    }
    fn budget(&mut self, bytes: usize) -> Result<(), String> {
        self.cx().check()?;
        self.remaining = self
            .remaining
            .checked_sub(bytes)
            .ok_or("call output limit exceeded")?;
        Ok(())
    }
    fn registered(&self, id: &str, kind: ContributionKind) -> Result<(), String> {
        if self.registrations.get(id) != Some(&kind) {
            return Err("unknown contribution or wrong kind".into());
        }
        Ok(())
    }
    fn publish(&mut self, output: Output) {
        if self.suppress {
            return;
        }
        if self.staging {
            self.staged.push(output);
        } else {
            self.services.emit(&self.id, output);
        }
    }
    fn current(&self, id: &str, revision: u64) -> Result<(), String> {
        self.check(&Permission::DocumentRead)?;
        let snapshot = self
            .services
            .snapshot(&self.cx(), id, self.limits.output_bytes)?;
        if snapshot.revision != revision {
            return Err("stale document revision".into());
        }
        Ok(())
    }
    fn live_mutation(&self) -> Result<(), String> {
        if self.staging || self.suppress {
            return Err("mutations unavailable during activation/deactivation".into());
        }
        Ok(())
    }
    /// Instance namespaces are reachable only through a provider this extension declares.
    fn instance_scope(&self, instance: &ProviderInstance) -> Result<(), String> {
        if !self.providers.contains(&instance.provider) {
            return Err("instance belongs to a provider this extension does not declare".into());
        }
        valid_instance_id(&instance.id)?;
        if instance.label.len() > 256 {
            return Err("instance label exceeds limit".into());
        }
        Ok(())
    }
}
/// Instance IDs become directory names under the provider's data directory.
pub(crate) fn valid_instance_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    {
        return Err("instance id must be 1..64 characters of [a-z0-9-]".into());
    }
    Ok(())
}
fn text_size(edit: &TextEdit) -> usize {
    32 + edit.segment_id.len() + edit.text.len()
}
fn properties_size(values: &[Property]) -> usize {
    values
        .iter()
        .map(|p| {
            32 + p.key.len()
                + match &p.value {
                    Scalar::Text(text) => text.len(),
                    _ => 8,
                }
        })
        .sum()
}
fn segment_size(segment: &Segment) -> usize {
    48 + segment.id.len()
        + segment.lane.len()
        + segment.speaker_id.as_ref().map_or(0, String::len)
        + segment.text.len()
        + properties_size(&segment.metadata)
}
fn document_size(document: &Document) -> usize {
    64 + document.id.len()
        + document.name.len()
        + document.mode.len()
        + properties_size(&document.metadata)
        + document.segments.iter().map(segment_size).sum::<usize>()
        + document
            .speakers
            .iter()
            .map(|s| 32 + s.id.len() + s.label.len())
            .sum::<usize>()
        + document
            .lanes
            .iter()
            .map(|l| {
                48 + l.id.len()
                    + l.label.len()
                    + l.speaker_id.as_ref().map_or(0, String::len)
                    + l.source_id.as_ref().map_or(0, String::len)
            })
            .sum::<usize>()
        + document
            .sources
            .iter()
            .map(|s| 48 + s.id.len() + s.label.len())
            .sum::<usize>()
}
fn config_size(config: &WidgetConfig) -> usize {
    48 + match config {
        WidgetConfig::Plain | WidgetConfig::Layout(_) | WidgetConfig::Numeric(_) => 0,
        WidgetConfig::Input(c) => c.placeholder.len(),
        WidgetConfig::Choices(c) => {
            c.selected.as_ref().map_or(0, String::len)
                + c.options
                    .iter()
                    .map(|o| 32 + o.key.len() + o.label.len())
                    .sum::<usize>()
        }
        WidgetConfig::Table(c) => {
            c.selected.as_ref().map_or(0, String::len)
                + c.columns
                    .iter()
                    .map(|c| 32 + c.key.len() + c.title.len())
                    .sum::<usize>()
                + c.rows
                    .iter()
                    .map(|r| 32 + r.key.len() + r.cells.iter().map(|v| 16 + v.len()).sum::<usize>())
                    .sum::<usize>()
        }
        WidgetConfig::Asset(c) => c.asset_id.len() + c.alt.len(),
        WidgetConfig::Waveform(c) => c.document_id.len() + c.lane_id.len(),
    }
}
fn transaction_size(value: &Transaction) -> usize {
    32 + value.document_id.len()
        + value.label.len()
        + value
            .edits
            .iter()
            .map(|edit| match edit {
                Edit::ReplaceText(value) => text_size(value),
                Edit::InsertSegment(value) | Edit::ReplaceSegment(value) => segment_size(value),
                Edit::RemoveSegment(id) => 16 + id.len(),
            })
            .sum::<usize>()
}
fn valid_key(key: &str) -> Result<(), String> {
    if key.is_empty()
        || key.len() > 256
        || key.contains(['/', '\\', '\0'])
        || key == "."
        || key == ".."
    {
        Err("invalid key".into())
    } else {
        Ok(())
    }
}
pub(crate) fn check_url(url: &str, grants: &BTreeSet<Permission>) -> Result<url::Url, String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    if parsed.scheme() != "https"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port_or_known_default() != Some(443)
        || parsed.fragment().is_some()
    {
        return Err("only credential-free HTTPS on port 443 is allowed".into());
    }
    let host = parsed.host_str().ok_or("missing host")?;
    if !grants.contains(&Permission::Network(host.into())) {
        return Err(format!("network permission denied: {host}"));
    }
    Ok(parsed)
}
impl Host for State {
    fn snapshot(&mut self, document_id: String) -> Result<Document, String> {
        self.check(&Permission::DocumentRead)?;
        let value = self
            .services
            .snapshot(&self.cx(), &document_id, self.remaining)?;
        self.budget(document_size(&value))?;
        Ok(value)
    }
    fn propose(&mut self, transaction: Transaction) -> Result<u64, String> {
        self.check(&Permission::DocumentEdit)?;
        self.live_mutation()?;
        self.budget(transaction_size(&transaction))?;
        // Edit permission includes a revision check, not permission to return document text.
        let snapshot = self.services.snapshot(
            &self.cx(),
            &transaction.document_id,
            self.limits.output_bytes,
        )?;
        if snapshot.revision != transaction.expected_revision {
            return Err("stale document revision".into());
        }
        if snapshot.read_only {
            return Err("document is read only".into());
        }
        if transaction.edits.is_empty() || transaction.label.trim().is_empty() {
            return Err("empty transaction".into());
        }
        self.services.apply_transaction(&self.cx(), transaction)
    }
    fn register(&mut self, descriptor: Registration) -> Result<(), String> {
        self.budget(32 + descriptor.id.len() + descriptor.title.len())?;
        valid_key(&descriptor.id)?;
        if self.registrations.len() >= self.limits.registrations
            || self.registrations.contains_key(&descriptor.id)
        {
            return Err("duplicate registration or registration limit".into());
        }
        if descriptor.kind == ContributionKind::Provider && !self.providers.contains(&descriptor.id)
        {
            return Err("provider was not declared in manifest".into());
        }
        self.registrations
            .insert(descriptor.id.clone(), descriptor.kind);
        self.publish(Output::Registered(descriptor));
        Ok(())
    }
    fn publish_diagnostics(&mut self, value: Diagnostics) -> Result<(), String> {
        self.registered(&value.provider, ContributionKind::Diagnostics)?;
        self.budget(
            32 + value.provider.len()
                + value.document_id.len()
                + value
                    .items
                    .iter()
                    .map(|d| {
                        48 + d.id.len()
                            + d.range.segment_id.len()
                            + d.message.len()
                            + d.fix.as_ref().map(transaction_size).unwrap_or(0)
                    })
                    .sum::<usize>(),
        )?;
        self.current(&value.document_id, value.revision)?;
        self.publish(Output::Diagnostics(value));
        Ok(())
    }
    fn publish_completions(&mut self, value: Completions) -> Result<(), String> {
        if !self
            .registrations
            .values()
            .any(|k| *k == ContributionKind::Completions)
        {
            return Err("no completions provider".into());
        }
        self.budget(
            32 + value.document_id.len()
                + value
                    .items
                    .iter()
                    .map(|c| 32 + c.label.len() + c.detail.len() + text_size(&c.edit))
                    .sum::<usize>(),
        )?;
        self.current(&value.document_id, value.revision)?;
        self.publish(Output::Completions(value));
        Ok(())
    }
    fn publish_decorations(&mut self, value: Decorations) -> Result<(), String> {
        self.registered(&value.provider, ContributionKind::Decorations)?;
        self.budget(
            32 + value.provider.len()
                + value.document_id.len()
                + value
                    .items
                    .iter()
                    .map(|d| {
                        32 + d.range.segment_id.len()
                            + d.class.len()
                            + d.hover.as_ref().map_or(0, String::len)
                    })
                    .sum::<usize>(),
        )?;
        self.current(&value.document_id, value.revision)?;
        self.publish(Output::Decorations(value));
        Ok(())
    }
    fn publish_overlay(&mut self, value: Overlay) -> Result<(), String> {
        self.registered(&value.provider, ContributionKind::Overlay)?;
        let size = value
            .draws
            .iter()
            .map(|d| {
                64 + match d {
                    Draw::Label(l) => l.text.len(),
                    Draw::Curve(c) => c.points.len() * 16,
                    Draw::HitRegion(h) => h.id.len(),
                    _ => 0,
                }
            })
            .sum::<usize>();
        self.budget(32 + value.provider.len() + value.document_id.len() + size)?;
        let mut hits = BTreeSet::new();
        for draw in &value.draws {
            validate_draw(draw)?;
            if let Draw::HitRegion(hit) = draw
                && !hits.insert(hit.id.clone())
            {
                return Err("duplicate overlay hit region".into());
            }
        }
        self.current(&value.document_id, value.revision)?;
        self.ui_nodes.insert(value.provider.clone(), hits);
        self.publish(Output::Overlay(value));
        Ok(())
    }
    fn publish_ui(&mut self, value: UiTree) -> Result<(), String> {
        match self.registrations.get(&value.contribution) {
            Some(
                ContributionKind::Panel
                | ContributionKind::Inspector
                | ContributionKind::Mode
                | ContributionKind::Provider,
            ) => {}
            _ => return Err("unknown UI contribution".into()),
        }
        self.budget(
            32 + value.contribution.len()
                + value
                    .nodes
                    .iter()
                    .map(|n| {
                        64 + n.id.len()
                            + n.text.len()
                            + n.value.len()
                            + n.command.as_ref().map_or(0, String::len)
                            + config_size(&n.config)
                    })
                    .sum::<usize>(),
        )?;
        let mut depths = Vec::with_capacity(value.nodes.len());
        let mut enabled = Vec::with_capacity(value.nodes.len());
        let mut ids = BTreeMap::new();
        for (index, node) in value.nodes.iter().enumerate() {
            if ids.insert(&node.id, node.parent).is_some() {
                return Err("duplicate UI node".into());
            }
            validate_config(node)?;
            if matches!(node.config, WidgetConfig::Waveform(_)) {
                self.check(&Permission::AudioRead)?;
            }
            let depth = match node.parent {
                None => 1,
                Some(parent) if (parent as usize) < index => depths[parent as usize] + 1,
                _ => return Err("UI parent must precede child".into()),
            };
            if depth > self.limits.ui_depth {
                return Err("UI depth limit exceeded".into());
            }
            if let Some(command) = &node.command {
                self.registered(command, ContributionKind::Command)?;
            }
            depths.push(depth);
            enabled.push(node.enabled && node.parent.is_none_or(|parent| enabled[parent as usize]));
        }
        for (index, node) in value.nodes.iter().enumerate() {
            if let (Widget::Tabs, WidgetConfig::Choices(config)) = (&node.widget, &node.config)
                && config
                    .options
                    .iter()
                    .any(|option| ids.get(&option.key) != Some(&Some(index as u32)))
            {
                return Err("tab option must name a direct child".into());
            }
        }
        self.ui_nodes.insert(
            value.contribution.clone(),
            value
                .nodes
                .iter()
                .zip(enabled)
                .filter(|(_, enabled)| *enabled)
                .map(|(node, _)| node.id.clone())
                .collect(),
        );
        self.publish(Output::Ui(value));
        Ok(())
    }
    fn report_progress(&mut self, value: Progress) -> Result<(), String> {
        self.budget(32 + value.label.len())?;
        if value.total.is_some_and(|total| value.completed > total) {
            return Err("invalid progress".into());
        }
        self.publish(Output::Progress(value));
        Ok(())
    }
    fn cancelled(&mut self) -> bool {
        self.cancellation.load(Ordering::Relaxed) || Instant::now() >= self.deadline
    }
    fn audio(&mut self, request: AudioRequest) -> Result<AudioData, String> {
        self.check(&Permission::AudioRead)?;
        if request.frames as usize > self.remaining / 4 {
            return Err("audio request too large".into());
        }
        let value = self
            .services
            .audio(&self.cx(), request, self.remaining / 4)?;
        self.budget(16 + value.samples.len() * 4)?;
        Ok(value)
    }
    fn http(&mut self, mut request: HttpRequest) -> Result<HttpResponse, String> {
        self.live_mutation()?;
        for _ in 0..=5 {
            self.cx().check()?;
            let url = check_url(&request.url, &self.grants)?;
            self.budget(
                32 + request.url.len()
                    + request.method.len()
                    + request.body.len()
                    + request
                        .headers
                        .iter()
                        .map(|h| 16 + h.name.len() + h.value.len())
                        .sum::<usize>(),
            )?;
            if request.headers.iter().any(|h| {
                h.name.eq_ignore_ascii_case("host")
                    || h.name.eq_ignore_ascii_case("proxy-authorization")
            }) {
                return Err("forbidden HTTP header".into());
            }
            let response = self
                .services
                .http_once(&self.cx(), &request, self.remaining)?;
            self.budget(
                32 + response.body.len()
                    + response
                        .headers
                        .iter()
                        .map(|h| 16 + h.name.len() + h.value.len())
                        .sum::<usize>(),
            )?;
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let location = response
                .headers
                .iter()
                .find(|h| h.name.eq_ignore_ascii_case("location"))
                .ok_or("redirect missing location")?;
            let next = url.join(&location.value).map_err(|e| e.to_string())?;
            check_url(next.as_str(), &self.grants)?;
            if next.host_str() != url.host_str() {
                request.headers.retain(|h| {
                    !h.name.eq_ignore_ascii_case("authorization")
                        && !h.name.eq_ignore_ascii_case("cookie")
                });
            }
            if response.status == 303 {
                request.method = "GET".into();
                request.body.clear();
                request.headers.retain(|h| {
                    !h.name.eq_ignore_ascii_case("content-type")
                        && !h.name.eq_ignore_ascii_case("content-length")
                });
            }
            request.url = next.into();
        }
        Err("redirect limit exceeded".into())
    }
    fn storage_get(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        self.check(&Permission::Storage)?;
        valid_key(&key)?;
        let value = self
            .services
            .storage_get(&self.cx(), &key, self.remaining)?;
        self.budget(16 + value.as_ref().map_or(0, Vec::len))?;
        Ok(value)
    }
    fn storage_set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        self.check(&Permission::Storage)?;
        self.live_mutation()?;
        valid_key(&key)?;
        self.budget(key.len() + value.len())?;
        self.services.storage_set(&self.cx(), &key, &value)
    }
    fn secret_get(&mut self, key: String) -> Result<Option<String>, String> {
        self.check(&Permission::Secrets)?;
        valid_key(&key)?;
        let value = self.services.secret_get(&self.cx(), &key, self.remaining)?;
        self.budget(16 + value.as_ref().map_or(0, String::len))?;
        Ok(value)
    }
    fn instance_secret_get(
        &mut self,
        instance: ProviderInstance,
        key: String,
    ) -> Result<Option<String>, String> {
        self.check(&Permission::Secrets)?;
        self.instance_scope(&instance)?;
        valid_key(&key)?;
        let value =
            self.services
                .instance_secret_get(&self.cx(), &instance, &key, self.remaining)?;
        self.budget(16 + value.as_ref().map_or(0, String::len))?;
        Ok(value)
    }
    fn instance_secret_set(
        &mut self,
        instance: ProviderInstance,
        key: String,
        value: Option<String>,
    ) -> Result<(), String> {
        self.check(&Permission::Secrets)?;
        self.live_mutation()?;
        self.instance_scope(&instance)?;
        valid_key(&key)?;
        self.budget(key.len() + value.as_ref().map_or(0, String::len))?;
        self.services
            .instance_secret_set(&self.cx(), &instance, &key, value.as_deref())
    }
    fn instance_storage_get(
        &mut self,
        instance: ProviderInstance,
        key: String,
    ) -> Result<Option<Vec<u8>>, String> {
        self.check(&Permission::Storage)?;
        self.instance_scope(&instance)?;
        valid_key(&key)?;
        let value =
            self.services
                .instance_storage_get(&self.cx(), &instance, &key, self.remaining)?;
        self.budget(16 + value.as_ref().map_or(0, Vec::len))?;
        Ok(value)
    }
    fn instance_storage_set(
        &mut self,
        instance: ProviderInstance,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), String> {
        self.check(&Permission::Storage)?;
        self.live_mutation()?;
        self.instance_scope(&instance)?;
        valid_key(&key)?;
        self.budget(key.len() + value.len())?;
        self.services
            .instance_storage_set(&self.cx(), &instance, &key, &value)
    }
    fn provider_setup_done(&mut self, value: ProviderSetupResult) -> Result<(), String> {
        self.live_mutation()?;
        self.registered(&value.provider, ContributionKind::Provider)?;
        self.budget(32 + value.provider.len() + value.id.len() + value.label.len())?;
        valid_instance_id(&value.id)?;
        if value.label.trim().is_empty() || value.label.len() > 256 {
            return Err("instance label must be 1..256 bytes".into());
        }
        self.publish(Output::ProviderSetupDone(value));
        Ok(())
    }
    fn provider_setup_cancel(&mut self, provider: String, request_id: u64) -> Result<(), String> {
        self.live_mutation()?;
        self.registered(&provider, ContributionKind::Provider)?;
        self.budget(32 + provider.len())?;
        self.publish(Output::ProviderSetupCancelled {
            provider,
            request_id,
        });
        Ok(())
    }
    fn publish_provider_status(&mut self, value: ProviderStatus) -> Result<(), String> {
        self.live_mutation()?;
        self.registered(&value.instance.provider, ContributionKind::Provider)?;
        self.instance_scope(&value.instance)?;
        self.budget(
            32 + value.instance.id.len() + value.instance.label.len() + value.detail.len(),
        )?;
        if value.detail.len() > 1024 {
            return Err("status detail exceeds limit".into());
        }
        self.publish(Output::ProviderStatus(value));
        Ok(())
    }
    fn selection_get(&mut self, document_id: String) -> Result<Selection, String> {
        self.check(&Permission::DocumentRead)?;
        let value = self.services.selection_get(&self.cx(), &document_id)?;
        self.budget(
            32 + value.document_id.len()
                + value
                    .ranges
                    .iter()
                    .map(|r| 32 + r.segment_id.len())
                    .sum::<usize>(),
        )?;
        Ok(value)
    }
    fn selection_set(&mut self, value: Selection) -> Result<(), String> {
        self.check(&Permission::SelectionControl)?;
        self.live_mutation()?;
        self.budget(
            32 + value.document_id.len()
                + value
                    .ranges
                    .iter()
                    .map(|r| 32 + r.segment_id.len())
                    .sum::<usize>(),
        )?;
        self.services.selection_set(&self.cx(), value)
    }
    fn transport_get(&mut self, document_id: String) -> Result<Transport, String> {
        self.check(&Permission::AudioRead)?;
        let value = self.services.transport_get(&self.cx(), &document_id)?;
        self.budget(32 + value.document_id.len())?;
        Ok(value)
    }
    fn transport_set(&mut self, value: Transport) -> Result<(), String> {
        self.check(&Permission::TransportControl)?;
        self.live_mutation()?;
        self.budget(32 + value.document_id.len())?;
        if !value.rate.is_finite() || value.rate <= 0.0 || value.rate > 8.0 {
            return Err("invalid playback rate".into());
        }
        self.services.transport_set(&self.cx(), value)
    }
    fn publish_mode_document(&mut self, value: ModeDocument) -> Result<(), String> {
        self.check(&Permission::DocumentEdit)?;
        self.live_mutation()?;
        self.registered(&value.mode, ContributionKind::Mode)?;
        self.budget(32 + value.mode.len() + document_size(&value.document))?;
        self.publish(Output::ModeDocument(value));
        Ok(())
    }
    fn publish_import(&mut self, value: ImportResult) -> Result<(), String> {
        self.check(&Permission::DocumentEdit)?;
        self.live_mutation()?;
        self.registered(&value.provider, ContributionKind::Importer)?;
        self.budget(32 + value.provider.len() + document_size(&value.document))?;
        self.publish(Output::Import(value));
        Ok(())
    }
    fn publish_export(&mut self, value: ExportResult) -> Result<(), String> {
        self.check(&Permission::DocumentRead)?;
        self.live_mutation()?;
        self.registered(&value.provider, ContributionKind::Exporter)?;
        self.budget(
            32 + value.provider.len()
                + value.name.len()
                + value.media_type.len()
                + value.bytes.len(),
        )?;
        self.publish(Output::Export(value));
        Ok(())
    }
}

fn validate_config(node: &UiNode) -> Result<(), String> {
    let matches = matches!(
        (&node.widget, &node.config),
        (
            Widget::Stack | Widget::Row | Widget::Scroll | Widget::Split,
            WidgetConfig::Layout(_)
        ) | (
            Widget::Tabs | Widget::Select | Widget::ListView,
            WidgetConfig::Choices(_)
        ) | (Widget::TextInput, WidgetConfig::Input(_))
            | (Widget::Number | Widget::Slider, WidgetConfig::Numeric(_))
            | (Widget::Table, WidgetConfig::Table(_))
            | (Widget::Image | Widget::Icon, WidgetConfig::Asset(_))
            | (Widget::Waveform, WidgetConfig::Waveform(_))
            | (
                Widget::Text
                    | Widget::Markdown
                    | Widget::Button
                    | Widget::Toggle
                    | Widget::Progress
                    | Widget::Badge
                    | Widget::Callout,
                WidgetConfig::Plain
            )
    );
    if !matches {
        return Err("widget configuration does not match widget".into());
    }
    match &node.config {
        WidgetConfig::Numeric(c)
            if !c.min.is_finite()
                || !c.max.is_finite()
                || !c.step.is_finite()
                || c.min > c.max
                || c.step <= 0.0 =>
        {
            return Err("invalid numeric configuration".into());
        }
        WidgetConfig::Choices(c) => {
            let keys: BTreeSet<_> = c.options.iter().map(|o| &o.key).collect();
            if keys.len() != c.options.len()
                || c.selected.as_ref().is_some_and(|s| !keys.contains(s))
            {
                return Err("invalid choices".into());
            }
        }
        WidgetConfig::Table(c) => {
            let columns: BTreeSet<_> = c.columns.iter().map(|c| &c.key).collect();
            let rows: BTreeSet<_> = c.rows.iter().map(|r| &r.key).collect();
            if columns.len() != c.columns.len()
                || rows.len() != c.rows.len()
                || c.rows.iter().any(|r| r.cells.len() != c.columns.len())
                || c.selected.as_ref().is_some_and(|s| !rows.contains(s))
            {
                return Err("invalid table".into());
            }
        }
        WidgetConfig::Asset(c) => {
            valid_key(&c.asset_id)?;
            if c.width > 8192 || c.height > 8192 {
                return Err("asset dimensions too large".into());
            }
        }
        WidgetConfig::Waveform(c) if c.start_ms > c.end_ms => {
            return Err("invalid waveform range".into());
        }
        _ => {}
    }
    Ok(())
}

fn validate_draw(draw: &Draw) -> Result<(), String> {
    let point = |p: &Point| {
        p.time_ms.is_finite() && p.time_ms >= 0.0 && p.lane.is_finite() && p.lane >= 0.0
    };
    let width = |w: f32| w.is_finite() && w > 0.0 && w <= 1024.0;
    let valid = match draw {
        Draw::Line(d) => point(&d.start) && point(&d.end) && width(d.width),
        Draw::Rect(d) => {
            point(&d.start)
                && point(&d.end)
                && d.start.time_ms <= d.end.time_ms
                && d.start.lane <= d.end.lane
        }
        Draw::Label(d) => point(&d.at),
        Draw::Curve(d) => d.points.len() >= 2 && d.points.iter().all(point) && width(d.width),
        Draw::HitRegion(d) => {
            !d.id.is_empty()
                && point(&d.start)
                && point(&d.end)
                && d.start.time_ms <= d.end.time_ms
                && d.start.lane <= d.end.lane
        }
    };
    if valid {
        Ok(())
    } else {
        Err("invalid overlay geometry".into())
    }
}
