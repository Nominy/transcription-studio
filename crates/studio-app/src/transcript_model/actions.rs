use std::fmt;

use studio_core::Project;

use super::TranscriptRowView;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptActionMenuView {
    pub segment_id: String,
    pub items: Vec<TranscriptActionMenuEntryView>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptActionMenuItemView {
    pub command: TranscriptActionCommand,
    pub label: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptActionMenuEntryView {
    Item(TranscriptActionMenuItemView),
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptActionCommand {
    Edit,
    MergeAbove,
    MergeBelow,
    CreateBefore,
    CreateAfter,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptTrackOptionView {
    pub lane_id: String,
    pub label: String,
}

impl fmt::Display for TranscriptTrackOptionView {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.label)
    }
}

pub fn build_transcript_action_menu(
    _project: &Project,
    row: &TranscriptRowView,
) -> TranscriptActionMenuView {
    TranscriptActionMenuView {
        segment_id: row.segment_id.clone(),
        items: build_babel_row_action_items(row),
    }
}

fn build_babel_row_action_items(row: &TranscriptRowView) -> Vec<TranscriptActionMenuEntryView> {
    let mut items = Vec::new();

    if !row.annotation_kind.is_transcription() {
        items.push(item(TranscriptActionCommand::Edit, "Edit", true));
        items.push(TranscriptActionMenuEntryView::Separator);
    } else if row.can_insert_above {
        items.push(item(
            TranscriptActionCommand::CreateBefore,
            "Add Segment Above",
            true,
        ));
        if row.can_insert_below {
            items.push(item(
                TranscriptActionCommand::CreateAfter,
                "Add Segment Below",
                true,
            ));
        }
        items.push(TranscriptActionMenuEntryView::Separator);
    } else if row.can_insert_below {
        items.push(item(
            TranscriptActionCommand::CreateAfter,
            "Add Segment Below",
            true,
        ));
        items.push(TranscriptActionMenuEntryView::Separator);
    }

    items.extend([
        item(
            TranscriptActionCommand::MergeAbove,
            "Merge With Above",
            row.can_merge_above,
        ),
        item(
            TranscriptActionCommand::MergeBelow,
            "Merge With Below",
            row.can_merge_below,
        ),
        TranscriptActionMenuEntryView::Separator,
        item(TranscriptActionCommand::Delete, "Delete", true),
    ]);

    items
}

pub(crate) fn build_transcript_track_options(project: &Project) -> Vec<TranscriptTrackOptionView> {
    project
        .lanes
        .iter()
        .map(|lane| TranscriptTrackOptionView {
            lane_id: lane.id.clone(),
            label: lane.label.clone(),
        })
        .collect()
}

pub(crate) fn selected_track_option(
    track_options: &[TranscriptTrackOptionView],
    lane_id: &str,
) -> Option<TranscriptTrackOptionView> {
    track_options
        .iter()
        .find(|option| option.lane_id == lane_id)
        .cloned()
}

fn item(
    command: TranscriptActionCommand,
    label: &'static str,
    enabled: bool,
) -> TranscriptActionMenuEntryView {
    TranscriptActionMenuEntryView::Item(TranscriptActionMenuItemView {
        command,
        label: label.to_string(),
        enabled,
    })
}
