//! UI-independent, document-scoped extension contracts. Ranges are UTF-8 byte offsets.
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use studio_core::{Project, Segment};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentOrigin {
    pub mode: String,
    pub key: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentPolicy {
    pub read_only: bool,
    /// A mode may require a larger gap when cutting a segment.
    pub minimum_cut_milliseconds: u64,
    /// Optional registered command that owns region creation for this document.
    pub region_creation_command: Option<String>,
    /// Registered editor for non-text segments; text segments use the built-in editor.
    pub segment_edit_command: Option<String>,
    /// Segment text may carry Babel's number markup (`4 {СКАЗ: четыре}`); other documents get
    /// the spoken words only.
    #[serde(default)]
    pub number_markup: bool,
    /// Each speaker owns one audio lane (Babel speaker tracks): assigning a speaker moves the
    /// segment to that speaker's lane and speakers cannot be added.
    #[serde(default)]
    pub speakers_bound_to_lanes: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DocumentEdit {
    ReplaceText {
        segment_id: String,
        range: Option<TextRange>,
        text: String,
    },
    InsertSegment {
        segment: Segment,
    },
    RemoveSegment {
        segment_id: String,
    },
    ReplaceSegment {
        segment: Segment,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentTransaction {
    pub label: String,
    pub expected_revision: u64,
    pub edits: Vec<DocumentEdit>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QuickFix {
    pub title: String,
    /// Snapshot revision; the host rejects the entire fix after any intervening edit.
    pub expected_revision: u64,
    pub edits: Vec<DocumentEdit>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub source: String,
    pub rule: String,
    pub severity: Severity,
    pub segment_id: String,
    pub range: Option<TextRange>,
    pub message: String,
    pub fixes: Vec<QuickFix>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticCollection {
    pub source: String,
    pub revision: u64,
    pub diagnostics: Vec<Diagnostic>,
}

pub trait DiagnosticProvider: std::fmt::Debug + Send + Sync {
    fn diagnose(&self, project: &Project, revision: u64) -> DiagnosticCollection;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    pub source: String,
    pub label: String,
    pub insert_text: String,
    pub range: TextRange,
    pub documentation: Option<String>,
    /// Absolute UTF-8 byte caret offset in the text after applying this completion.
    pub caret_offset: Option<usize>,
}

pub struct CompletionRequest<'a> {
    pub project: &'a Project,
    pub revision: u64,
    pub segment_id: &'a str,
    pub text: &'a str,
    pub selection: TextRange,
}

pub trait CompletionProvider: std::fmt::Debug + Send + Sync {
    fn complete(&self, request: &CompletionRequest<'_>) -> Vec<Completion>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decoration {
    pub source: String,
    pub kind: String,
    pub segment_id: String,
    pub range: Option<TextRange>,
    pub label: String,
    pub data: serde_json::Value,
}

pub trait DecorationProvider: std::fmt::Debug + Send + Sync {
    fn decorate(&self, project: &Project, revision: u64) -> Vec<Decoration>;
}

pub struct CommandRequest<'a> {
    pub project: &'a Project,
    pub revision: u64,
    pub segment_id: Option<&'a str>,
    pub text: Option<&'a str>,
    pub selection: Option<TextRange>,
    /// Actual selection head, as an absolute UTF-8 byte offset.
    pub caret_offset: Option<usize>,
    pub arguments: &'a serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandOutcome {
    pub transaction: Option<DocumentTransaction>,
    /// Replace drafts without committing them to project history.
    pub drafts: Vec<DraftReplacement>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftReplacement {
    pub segment_id: String,
    pub text: String,
    /// None preserves the existing character selection, clamped to the replacement text.
    pub selection: Option<TextRange>,
}

pub trait CommandProvider: std::fmt::Debug + Send + Sync {
    fn command_ids(&self) -> &[&str];
    fn execute(
        &self,
        command: &str,
        request: &CommandRequest<'_>,
    ) -> Result<CommandOutcome, String>;
}

/// Classifies opaque segment payloads for generic text-editing operations.
pub trait SegmentKindProvider: std::fmt::Debug + Send + Sync {
    fn is_text(&self, segment: &Segment) -> Option<bool>;
}

/// Composes a display view of raw provider diagnostics without changing their retained results.
pub trait DiagnosticPostprocessor: std::fmt::Debug + Send + Sync {
    fn process(
        &self,
        project: &Project,
        revision: u64,
        collections: &mut Vec<DiagnosticCollection>,
    );
}

#[derive(Clone, Debug, Default)]
pub struct DocumentProviders {
    pub diagnostics: Vec<Arc<dyn DiagnosticProvider>>,
    pub diagnostic_postprocessors: Vec<Arc<dyn DiagnosticPostprocessor>>,
    pub completions: Vec<Arc<dyn CompletionProvider>>,
    pub decorations: Vec<Arc<dyn DecorationProvider>>,
    pub commands: Vec<Arc<dyn CommandProvider>>,
    pub segment_kinds: Vec<Arc<dyn SegmentKindProvider>>,
    /// Domain mutation side effects; absence selects native, metadata-opaque edits.
    pub mutation_policy: Option<Arc<dyn studio_core::SegmentMutationPolicy>>,
}

impl PartialEq for DocumentProviders {
    fn eq(&self, other: &Self) -> bool {
        fn same<T: ?Sized>(a: &[Arc<T>], b: &[Arc<T>]) -> bool {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
        }
        same(&self.diagnostics, &other.diagnostics)
            && same(
                &self.diagnostic_postprocessors,
                &other.diagnostic_postprocessors,
            )
            && same(&self.completions, &other.completions)
            && same(&self.decorations, &other.decorations)
            && same(&self.commands, &other.commands)
            && same(&self.segment_kinds, &other.segment_kinds)
            && match (&self.mutation_policy, &other.mutation_policy) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentEnvironment {
    pub origin: DocumentOrigin,
    pub policy: DocumentPolicy,
    pub providers: DocumentProviders,
}

impl DocumentEnvironment {
    pub fn is_text_segment(&self, segment: &Segment) -> bool {
        self.providers
            .segment_kinds
            .iter()
            .find_map(|provider| provider.is_text(segment))
            .unwrap_or(true)
    }

    /// A local document has no implicit lint, authentication, review, or AI providers.
    pub fn local(key: impl Into<String>) -> Self {
        Self {
            origin: DocumentOrigin {
                mode: "exaudio.local".into(),
                key: key.into(),
            },
            policy: DocumentPolicy::default(),
            providers: DocumentProviders::default(),
        }
    }
}
