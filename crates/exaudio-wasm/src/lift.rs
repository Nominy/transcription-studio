//! Lazy canonical-ABI lifting prevents alias-amplification: a malicious guest can
//! point thousands of list/string entries at the same linear-memory bytes. Never
//! let bindgen eagerly turn that graph into owned Rust allocations before limits.
use crate::{bindings::exaudio::extension::host::Host, host::State, types};
use anyhow::{Context, Result, ensure};
use wasmtime::{
    AsContext, StoreContextMut,
    component::{ComponentType, Lift, Linker, WasmList, WasmStr},
};

trait Take: Lift {
    type Owned;
    fn take(
        self,
        store: &mut StoreContextMut<'_, State>,
        budget: &mut usize,
    ) -> Result<Self::Owned>;
}
fn charge(budget: &mut usize, amount: usize) -> Result<()> {
    *budget = budget
        .checked_sub(amount)
        .context("canonical ABI allocation budget exceeded")?;
    Ok(())
}
impl Take for WasmStr {
    type Owned = String;
    fn take(self, store: &mut StoreContextMut<'_, State>, budget: &mut usize) -> Result<String> {
        // UTF-8 is borrowed. UTF-16 may require ONE temporary transcoding buffer,
        // bounded by linear memory size; never retain another before charging it.
        let value = self.to_str(store.as_context())?;
        charge(budget, value.len())?;
        Ok(value.into_owned())
    }
}
impl<T: Take> Take for WasmList<T> {
    type Owned = Vec<T::Owned>;
    fn take(
        self,
        store: &mut StoreContextMut<'_, State>,
        budget: &mut usize,
    ) -> Result<Self::Owned> {
        let count = self.len();
        let bytes = count
            .checked_mul(std::mem::size_of::<T::Owned>().max(1))
            .context("list allocation overflow")?;
        charge(budget, bytes)?;
        let mut values = Vec::with_capacity(count);
        for index in 0..count {
            let value = self.get(&mut *store, index).context("list index")??;
            values.push(value.take(store, budget)?);
        }
        Ok(values)
    }
}
impl<T: Take> Take for Option<T> {
    type Owned = Option<T::Owned>;
    fn take(
        self,
        store: &mut StoreContextMut<'_, State>,
        budget: &mut usize,
    ) -> Result<Self::Owned> {
        self.map(|value| value.take(store, budget)).transpose()
    }
}
macro_rules! leaf {
    ($($ty:ty),* $(,)?) => { $(impl Take for $ty {
        type Owned = Self;
        fn take(self, _: &mut StoreContextMut<'_, State>, _: &mut usize) -> Result<Self> { Ok(self) }
    })* };
}
leaf!(
    u8,
    u16,
    u32,
    u64,
    f32,
    f64,
    bool,
    types::Severity,
    types::ContributionKind,
    types::Widget,
    types::Point,
    types::Color,
    types::Line,
    types::Rect,
    types::LayoutConfig,
    types::NumberConfig,
    types::ProviderState
);
macro_rules! record {
    ($name:ident { $($field:ident : $ty:ty => $wit:literal),* $(,)? }) => {
        #[derive(ComponentType, Lift)]
        #[component(record)]
        struct $name { $(#[component(name = $wit)] $field: $ty),* }
        impl Take for $name {
            type Owned = types::$name;
            fn take(self, store: &mut StoreContextMut<'_, State>, budget: &mut usize) -> Result<Self::Owned> {
                Ok(types::$name { $($field: self.$field.take(store, budget)?),* })
            }
        }
    };
}
macro_rules! variant {
    ($name:ident { $($case:ident($ty:ty) => $wit:literal),* $(,)? }) => {
        #[derive(ComponentType, Lift)]
        #[component(variant)]
        enum $name { $(#[component(name = $wit)] $case($ty)),* }
        impl Take for $name {
            type Owned = types::$name;
            fn take(self, store: &mut StoreContextMut<'_, State>, budget: &mut usize) -> Result<Self::Owned> {
                match self { $(Self::$case(value) => Ok(types::$name::$case(value.take(store, budget)?))),* }
            }
        }
    };
}
variant!(Scalar { Text(WasmStr) => "text", Number(f64) => "number", Boolean(bool) => "boolean" });
record!(Property { key: WasmStr => "key", value: Scalar => "value" });
record!(Speaker { id: WasmStr => "id", label: WasmStr => "label" });
record!(Lane { id: WasmStr => "id", speaker_id: Option<WasmStr> => "speaker-id", label: WasmStr => "label", source_id: Option<WasmStr> => "source-id" });
record!(AudioSource { id: WasmStr => "id", label: WasmStr => "label", sample_rate: u32 => "sample-rate", frames: u64 => "frames", channels: u32 => "channels" });
record!(Segment { id: WasmStr => "id", lane: WasmStr => "lane", speaker_id: Option<WasmStr> => "speaker-id", start_ms: u64 => "start-ms", end_ms: u64 => "end-ms", text: WasmStr => "text", metadata: WasmList<Property> => "metadata" });
record!(Document { id: WasmStr => "id", revision: u64 => "revision", name: WasmStr => "name", mode: WasmStr => "mode", read_only: bool => "read-only", speakers: WasmList<Speaker> => "speakers", lanes: WasmList<Lane> => "lanes", sources: WasmList<AudioSource> => "sources", metadata: WasmList<Property> => "metadata", segments: WasmList<Segment> => "segments" });
record!(TextEdit { segment_id: WasmStr => "segment-id", start_byte: u32 => "start-byte", end_byte: u32 => "end-byte", text: WasmStr => "text" });
variant!(Edit { ReplaceText(TextEdit) => "replace-text", InsertSegment(Segment) => "insert-segment", RemoveSegment(WasmStr) => "remove-segment", ReplaceSegment(Segment) => "replace-segment" });
record!(Transaction { document_id: WasmStr => "document-id", expected_revision: u64 => "expected-revision", label: WasmStr => "label", edits: WasmList<Edit> => "edits" });
record!(TextRange { segment_id: WasmStr => "segment-id", start_byte: u32 => "start-byte", end_byte: u32 => "end-byte" });
record!(Diagnostic { id: WasmStr => "id", range: TextRange => "range", severity: types::Severity => "severity", message: WasmStr => "message", fix: Option<Transaction> => "fix" });
record!(Diagnostics { provider: WasmStr => "provider", document_id: WasmStr => "document-id", revision: u64 => "revision", items: WasmList<Diagnostic> => "items" });
record!(Completion { label: WasmStr => "label", detail: WasmStr => "detail", edit: TextEdit => "edit" });
record!(Completions { request_id: u64 => "request-id", document_id: WasmStr => "document-id", revision: u64 => "revision", items: WasmList<Completion> => "items" });
record!(Decoration { range: TextRange => "range", class: WasmStr => "class", hover: Option<WasmStr> => "hover" });
record!(Decorations { provider: WasmStr => "provider", document_id: WasmStr => "document-id", revision: u64 => "revision", items: WasmList<Decoration> => "items" });
record!(Label { at: types::Point => "at", text: WasmStr => "text", color: types::Color => "color" });
record!(Curve { points: WasmList<types::Point> => "points", color: types::Color => "color", width: f32 => "width" });
record!(HitRegion { id: WasmStr => "id", start: types::Point => "start", end: types::Point => "end" });
variant!(Draw { Line(types::Line) => "line", Rect(types::Rect) => "rect", Label(Label) => "label", Curve(Curve) => "curve", HitRegion(HitRegion) => "hit-region" });
record!(Overlay { provider: WasmStr => "provider", document_id: WasmStr => "document-id", revision: u64 => "revision", draws: WasmList<Draw> => "draws" });
record!(InputConfig { placeholder: WasmStr => "placeholder", multiline: bool => "multiline", read_only: bool => "read-only" });
record!(Choice { key: WasmStr => "key", label: WasmStr => "label" });
record!(ChoiceConfig { options: WasmList<Choice> => "options", selected: Option<WasmStr> => "selected" });
record!(TableColumn { key: WasmStr => "key", title: WasmStr => "title" });
record!(TableRow { key: WasmStr => "key", cells: WasmList<WasmStr> => "cells" });
record!(TableConfig { columns: WasmList<TableColumn> => "columns", rows: WasmList<TableRow> => "rows", selected: Option<WasmStr> => "selected" });
record!(AssetConfig { asset_id: WasmStr => "asset-id", alt: WasmStr => "alt", width: u32 => "width", height: u32 => "height" });
record!(WaveformConfig { document_id: WasmStr => "document-id", lane_id: WasmStr => "lane-id", start_ms: u64 => "start-ms", end_ms: u64 => "end-ms", channel: u32 => "channel" });
#[derive(ComponentType, Lift)]
#[component(variant)]
enum WidgetConfig {
    #[component(name = "plain")]
    Plain,
    #[component(name = "layout")]
    Layout(types::LayoutConfig),
    #[component(name = "input")]
    Input(InputConfig),
    #[component(name = "choices")]
    Choices(ChoiceConfig),
    #[component(name = "numeric")]
    Numeric(types::NumberConfig),
    #[component(name = "table")]
    Table(TableConfig),
    #[component(name = "asset")]
    Asset(AssetConfig),
    #[component(name = "waveform")]
    Waveform(WaveformConfig),
}
impl Take for WidgetConfig {
    type Owned = types::WidgetConfig;
    fn take(
        self,
        store: &mut StoreContextMut<'_, State>,
        budget: &mut usize,
    ) -> Result<Self::Owned> {
        Ok(match self {
            Self::Plain => types::WidgetConfig::Plain,
            Self::Layout(v) => types::WidgetConfig::Layout(v),
            Self::Numeric(v) => types::WidgetConfig::Numeric(v),
            Self::Input(v) => types::WidgetConfig::Input(v.take(store, budget)?),
            Self::Choices(v) => types::WidgetConfig::Choices(v.take(store, budget)?),
            Self::Table(v) => types::WidgetConfig::Table(v.take(store, budget)?),
            Self::Asset(v) => types::WidgetConfig::Asset(v.take(store, budget)?),
            Self::Waveform(v) => types::WidgetConfig::Waveform(v.take(store, budget)?),
        })
    }
}
record!(UiNode { id: WasmStr => "id", parent: Option<u32> => "parent", widget: types::Widget => "widget", config: WidgetConfig => "config", text: WasmStr => "text", value: WasmStr => "value", enabled: bool => "enabled", command: Option<WasmStr> => "command" });
record!(UiTree { contribution: WasmStr => "contribution", nodes: WasmList<UiNode> => "nodes" });
record!(Registration { id: WasmStr => "id", kind: types::ContributionKind => "kind", title: WasmStr => "title" });
record!(Progress { task_id: u64 => "task-id", label: WasmStr => "label", completed: u64 => "completed", total: Option<u64> => "total" });
record!(AudioRequest { document_id: WasmStr => "document-id", lane_id: WasmStr => "lane-id", start_frame: u64 => "start-frame", frames: u32 => "frames", channel: u32 => "channel" });
record!(Header { name: WasmStr => "name", value: WasmStr => "value" });
record!(HttpRequest { url: WasmStr => "url", method: WasmStr => "method", headers: WasmList<Header> => "headers", body: WasmList<u8> => "body" });
record!(Selection { document_id: WasmStr => "document-id", revision: u64 => "revision", ranges: WasmList<TextRange> => "ranges" });
record!(Transport { document_id: WasmStr => "document-id", position_ms: u64 => "position-ms", playing: bool => "playing", rate: f32 => "rate" });
record!(ModeDocument { mode: WasmStr => "mode", request_id: u64 => "request-id", document: Document => "document" });
record!(ImportResult { provider: WasmStr => "provider", request_id: u64 => "request-id", document: Document => "document" });
record!(ExportResult { provider: WasmStr => "provider", request_id: u64 => "request-id", name: WasmStr => "name", media_type: WasmStr => "media-type", bytes: WasmList<u8> => "bytes" });
record!(ProviderInstance { provider: WasmStr => "provider", id: WasmStr => "id", label: WasmStr => "label" });
record!(ProviderSetupResult { provider: WasmStr => "provider", request_id: u64 => "request-id", id: WasmStr => "id", label: WasmStr => "label" });
record!(ProviderStatus { instance: ProviderInstance => "instance", state: types::ProviderState => "state", detail: WasmStr => "detail" });

pub(crate) fn add_to_linker(linker: &mut Linker<State>) -> Result<()> {
    // WIT's type-only dependency has no host functions or resources.
    linker.instance("exaudio:extension/types@0.1.0")?;
    let mut host = linker.instance("exaudio:extension/host@0.1.0")?;
    macro_rules! import {
        ($wit:literal, $method:ident, $ty:ty) => {
            host.func_wrap(
                $wit,
                |mut store: StoreContextMut<'_, State>, (value,): ($ty,)| {
                    ensure!(
                        !store.data().cancelled_now(),
                        "cancelled or deadline exceeded"
                    );
                    let mut budget = store.data().remaining;
                    let value = value.take(&mut store, &mut budget)?;
                    Ok((store.data_mut().$method(value),))
                },
            )?;
        };
    }
    import!("snapshot", snapshot, WasmStr);
    import!("propose", propose, Transaction);
    import!("register", register, Registration);
    import!("publish-diagnostics", publish_diagnostics, Diagnostics);
    import!("publish-completions", publish_completions, Completions);
    import!("publish-decorations", publish_decorations, Decorations);
    import!("publish-overlay", publish_overlay, Overlay);
    import!("publish-ui", publish_ui, UiTree);
    import!("report-progress", report_progress, Progress);
    import!("audio", audio, AudioRequest);
    import!("http", http, HttpRequest);
    import!("storage-get", storage_get, WasmStr);
    import!("secret-get", secret_get, WasmStr);
    import!("selection-get", selection_get, WasmStr);
    import!("selection-set", selection_set, Selection);
    import!("transport-get", transport_get, WasmStr);
    import!("transport-set", transport_set, Transport);
    import!("publish-mode-document", publish_mode_document, ModeDocument);
    import!("publish-import", publish_import, ImportResult);
    import!("publish-export", publish_export, ExportResult);
    import!(
        "provider-setup-done",
        provider_setup_done,
        ProviderSetupResult
    );
    import!(
        "publish-provider-status",
        publish_provider_status,
        ProviderStatus
    );
    macro_rules! import_args {
        ($wit:literal, $method:ident, ($($arg:ident : $ty:ty),+)) => {
            host.func_wrap(
                $wit,
                |mut store: StoreContextMut<'_, State>, ($($arg,)+): ($($ty,)+)| {
                    ensure!(
                        !store.data().cancelled_now(),
                        "cancelled or deadline exceeded"
                    );
                    let mut budget = store.data().remaining;
                    $(let $arg = $arg.take(&mut store, &mut budget)?;)+
                    Ok((store.data_mut().$method($($arg),+),))
                },
            )?;
        };
    }
    import_args!("storage-set", storage_set, (key: WasmStr, value: WasmList<u8>));
    import_args!("instance-secret-get", instance_secret_get, (instance: ProviderInstance, key: WasmStr));
    import_args!("instance-secret-set", instance_secret_set, (instance: ProviderInstance, key: WasmStr, value: Option<WasmStr>));
    import_args!("instance-storage-get", instance_storage_get, (instance: ProviderInstance, key: WasmStr));
    import_args!("instance-storage-set", instance_storage_set, (instance: ProviderInstance, key: WasmStr, value: WasmList<u8>));
    import_args!("provider-setup-cancel", provider_setup_cancel, (provider: WasmStr, request_id: u64));
    host.func_wrap(
        "cancelled",
        |mut store: StoreContextMut<'_, State>, (): ()| Ok((store.data_mut().cancelled(),)),
    )?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn alias_probe(state: State, allocation_budget: usize) -> Result<usize> {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use wasmtime::{Config, Engine, Store, component::Component};
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;
    let component = Component::new(
        &engine,
        r#"
        (component
          (import "consume" (func $consume (param "values" (list string))))
          (core module $memory (memory (export "memory") 1))
          (core instance $memory (instantiate $memory))
          (alias core export $memory "memory" (core memory $shared))
          (core func $consume (canon lower (func $consume) (memory $shared)))
          (core module $guest
            (import "" "memory" (memory 1))
            (import "" "consume" (func $consume (param i32 i32)))
            (func (export "run")
              ;; Two different list entries alias one 16 KiB string. Eager lifting
              ;; would copy twice before a host callback could check its budget.
              i32.const 0 i32.const 8192 i32.store
              i32.const 4 i32.const 16384 i32.store
              i32.const 8 i32.const 8192 i32.store
              i32.const 12 i32.const 16384 i32.store
              i32.const 0 i32.const 2 call $consume))
          (core instance $imports
            (export "memory" (memory $shared))
            (export "consume" (func $consume)))
          (core instance $guest (instantiate $guest (with "" (instance $imports))))
          (alias core export $guest "run" (core func $run))
          (func (export "run") (canon lift (core func $run))))
    "#,
    )?;
    let mut linker = Linker::new(&engine);
    let observed = Arc::new(AtomicUsize::new(0));
    let output = Arc::clone(&observed);
    linker.root().func_wrap(
        "consume",
        move |mut store: StoreContextMut<'_, State>, (values,): (WasmList<WasmStr>,)| {
            let mut budget = allocation_budget;
            let values = values.take(&mut store, &mut budget)?;
            output.store(values.iter().map(String::len).sum(), Ordering::Relaxed);
            Ok(())
        },
    )?;
    let mut store = Store::new(&engine, state);
    let instance = linker.instantiate(&mut store, &component)?;
    instance
        .get_typed_func::<(), ()>(&mut store, "run")?
        .call(&mut store, ())?;
    Ok(observed.load(Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::charge;
    #[test]
    fn allocation_budget_checks_before_reserving_and_never_wraps() {
        let mut budget = 16;
        charge(&mut budget, 12).unwrap();
        assert!(charge(&mut budget, usize::MAX).is_err());
        assert_eq!(budget, 4);
        assert!(charge(&mut budget, 5).is_err());
    }
}
