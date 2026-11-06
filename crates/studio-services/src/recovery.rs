//! Native crash recovery for a caller-selected project root.
//! Route selection belongs to the document origin, never to account/session state.
//! Babel JSON drafts are owned separately by the Babel extension.

use std::path::Path;
use studio_project::{ProjectIoError, RecoverySnapshot};

/// Atomically save a portable native snapshot, resolving relative audio from `source_base`.
/// The caller keeps `root` with the document origin, including before its first manual save.
pub fn write_snapshot(
    snapshot: &RecoverySnapshot,
    root: &Path,
    source_base: &Path,
) -> Result<(), ProjectIoError> {
    studio_project::save_recovery(snapshot, root, source_base)
}

/// Load native recovery with audio resolved against its project root.
pub fn load_snapshot(root: &Path) -> Result<RecoverySnapshot, ProjectIoError> {
    studio_project::load_recovery(root)
}
