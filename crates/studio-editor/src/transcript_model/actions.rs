use std::fmt;

use studio_core::Project;

use super::TranscriptRowView;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptActionMenuEntry {
    Item {
        command: TranscriptActionCommand,
        enabled: bool,
    },
    Separator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TranscriptActionCommand {
    /// Edit annotation (non-transcription rows only).
    Edit,
    MergeAbove,
    MergeBelow,
    CreateBefore,
    CreateAfter,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TranscriptTrackOptionView {
    pub lane_id: String,
    pub label: String,
}

impl fmt::Display for TranscriptTrackOptionView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

/// Row overflow-menu entries, in display order.
pub(crate) fn build_transcript_action_menu(
    row: &TranscriptRowView,
) -> Vec<TranscriptActionMenuEntry> {
    let mut items = Vec::new();

    if !row.annotation_kind.is_transcription() {
        items.push(item(TranscriptActionCommand::Edit, true));
        items.push(TranscriptActionMenuEntry::Separator);
    } else if row.can_insert_above || row.can_insert_below {
        if row.can_insert_above {
            items.push(item(TranscriptActionCommand::CreateBefore, true));
        }
        if row.can_insert_below {
            items.push(item(TranscriptActionCommand::CreateAfter, true));
        }
        items.push(TranscriptActionMenuEntry::Separator);
    }

    items.extend([
        item(TranscriptActionCommand::MergeAbove, row.can_merge_above),
        item(TranscriptActionCommand::MergeBelow, row.can_merge_below),
        TranscriptActionMenuEntry::Separator,
        item(TranscriptActionCommand::Delete, true),
    ]);

    items
}

pub fn build_transcript_track_options(project: &Project) -> Vec<TranscriptTrackOptionView> {
    project
        .lanes
        .iter()
        .map(|lane| TranscriptTrackOptionView {
            lane_id: lane.id.clone(),
            label: lane.label.clone(),
        })
        .collect()
}

pub fn selected_track_option(
    track_options: &[TranscriptTrackOptionView],
    lane_id: &str,
) -> Option<TranscriptTrackOptionView> {
    track_options
        .iter()
        .find(|option| option.lane_id == lane_id)
        .cloned()
}

fn item(command: TranscriptActionCommand, enabled: bool) -> TranscriptActionMenuEntry {
    TranscriptActionMenuEntry::Item { command, enabled }
}
