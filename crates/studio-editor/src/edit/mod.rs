//! UI-free editing operations. [`EditorDoc`] is the per-task facade; the submodules hold the
//! pure rules it composes (and that the UI may call directly for previews and labels).
//!
//! Script-context / original-script metadata extraction lives in
//! [`crate::editor_model::context_panels`].

pub mod annotation;
pub mod autofix;
pub mod create_region;
pub mod doc;
pub mod draft;
pub mod find;
pub mod format;
pub mod keymap;
pub mod navigate;
pub mod route;
pub mod silence;
pub mod structure;
pub mod tags;
pub mod text;
pub mod time;

pub use doc::{
    EditError, EditResult, EditorDoc, Effect, HelperFocus, Outcome, PointerModifiers, PointerState,
    WaveformHover,
};
pub use draft::{SegmentDraft, TextSelection};
pub use keymap::{EditorAction, KeyContext, Keymap};
pub use route::{RouteLoad, StartupRoute};
