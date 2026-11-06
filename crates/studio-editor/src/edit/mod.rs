//! UI-free editing operations. [`EditorDoc`] is the per-task facade; the submodules hold the
//! pure rules it composes (and that the UI may call directly for previews and labels).
//!

pub mod create_region;
pub mod doc;
pub mod draft;
pub mod find;
pub mod format;
pub mod keymap;
pub mod navigate;
pub mod silence;
pub mod structure;
pub mod text;
pub mod text_placement;
pub mod time;

pub use doc::{CompletionKey, CompletionList};
pub use doc::{
    DraftedRow, EditError, EditResult, EditorDoc, Effect, HelperFocus, Outcome, PointerModifiers,
    PointerState, RowSnapshot, TranscriptionRefusal, TranscriptionTarget, WaveformHover,
    remove_word_final_ellipsis,
};
pub use draft::{SegmentDraft, TextSelection};
pub use keymap::{EditorAction, KeyContext, Keymap};
