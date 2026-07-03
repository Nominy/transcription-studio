use studio_app::{TranscriptRowKind, TranscriptRowView};

use super::drafts::SegmentDraft;

pub(super) const DEFAULT_TRANSCRIPT_VIEWPORT_HEIGHT: f32 = 640.0;
pub(super) const DEFAULT_TRANSCRIPT_ROW_HEIGHT: f32 = 30.0;
pub(super) const MODIFIED_TRANSCRIPT_ROW_HEIGHT: f32 = 28.0;
pub(super) const TRANSCRIPT_VIRTUAL_RENDER_ALL_THRESHOLD: usize = 40;
pub(super) const TRANSCRIPT_VIRTUAL_OVERSCAN_VIEWPORTS: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TranscriptVirtualizationInput {
    pub(super) scroll_offset_y: f32,
    pub(super) viewport_height: f32,
    pub(super) overscan_height: f32,
    pub(super) render_all_threshold: usize,
    /// Row that must stay mounted even when scrolled out of the window, so
    /// widget operations (focus) can always reach its text editor.
    pub(super) pinned_row_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TranscriptVirtualWindow {
    pub(super) start_index: usize,
    pub(super) end_index: usize,
    pub(super) before_height: f32,
    pub(super) after_height: f32,
    /// Out-of-window row kept mounted as a detached island (with exact spacer
    /// splits) so widget operations such as focus can always reach it.
    pub(super) pinned_row: Option<PinnedVirtualRow>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PinnedVirtualRow {
    pub(super) index: usize,
    /// Spacer height between the surrounding gap edge and the pinned row.
    pub(super) leading_height: f32,
    /// Spacer height between the pinned row and the rest of the gap.
    pub(super) trailing_height: f32,
}

impl TranscriptVirtualWindow {
    pub(super) fn rendered_len(self) -> usize {
        self.end_index.saturating_sub(self.start_index)
    }

    pub(super) fn pinned_row_before_window(self) -> Option<PinnedVirtualRow> {
        self.pinned_row
            .filter(|pinned| pinned.index < self.start_index)
    }

    pub(super) fn pinned_row_after_window(self) -> Option<PinnedVirtualRow> {
        self.pinned_row
            .filter(|pinned| pinned.index >= self.end_index)
    }
}

pub(super) fn transcript_virtual_window(
    row_heights: &[f32],
    input: TranscriptVirtualizationInput,
) -> TranscriptVirtualWindow {
    if row_heights.is_empty() {
        return TranscriptVirtualWindow {
            start_index: 0,
            end_index: 0,
            before_height: 0.0,
            after_height: 0.0,
            pinned_row: None,
        };
    }

    let total_height = transcript_total_height(row_heights);
    if row_heights.len() <= input.render_all_threshold {
        return TranscriptVirtualWindow {
            start_index: 0,
            end_index: row_heights.len(),
            before_height: 0.0,
            after_height: 0.0,
            pinned_row: None,
        };
    }

    let viewport_height = sanitized_viewport_height(input.viewport_height);
    let max_offset = (total_height - viewport_height).max(0.0);
    let scroll_offset_y = sanitized_dimension(input.scroll_offset_y)
        .unwrap_or(0.0)
        .min(max_offset);
    let overscan_height = sanitized_dimension(input.overscan_height)
        .unwrap_or(viewport_height)
        .max(0.0);
    let start_y = (scroll_offset_y - overscan_height).max(0.0);
    let end_y = (scroll_offset_y + viewport_height + overscan_height).max(start_y);

    let mut start_index = 0;
    let mut before_height = 0.0;
    while start_index < row_heights.len()
        && before_height + sanitized_row_height(row_heights[start_index]) <= start_y
    {
        before_height += sanitized_row_height(row_heights[start_index]);
        start_index += 1;
    }

    let mut end_index = start_index;
    let mut cursor_y = before_height;
    while end_index < row_heights.len() && cursor_y < end_y {
        cursor_y += sanitized_row_height(row_heights[end_index]);
        end_index += 1;
    }
    if end_index == start_index && end_index < row_heights.len() {
        cursor_y += sanitized_row_height(row_heights[end_index]);
        end_index += 1;
    }

    let after_height = (total_height - cursor_y).max(0.0);
    let pinned_row = input
        .pinned_row_index
        .filter(|pinned_index| {
            *pinned_index < row_heights.len()
                && (*pinned_index < start_index || *pinned_index >= end_index)
        })
        .map(|pinned_index| {
            let pinned_top = row_heights
                .iter()
                .take(pinned_index)
                .map(|height| sanitized_row_height(*height))
                .sum::<f32>();
            let pinned_height = sanitized_row_height(row_heights[pinned_index]);
            if pinned_index < start_index {
                PinnedVirtualRow {
                    index: pinned_index,
                    leading_height: pinned_top,
                    trailing_height: (before_height - pinned_top - pinned_height).max(0.0),
                }
            } else {
                PinnedVirtualRow {
                    index: pinned_index,
                    leading_height: (pinned_top - cursor_y).max(0.0),
                    trailing_height: (total_height - pinned_top - pinned_height).max(0.0),
                }
            }
        });

    TranscriptVirtualWindow {
        start_index,
        end_index,
        before_height,
        after_height,
        pinned_row,
    }
}

pub(super) fn estimate_transcript_row_height(
    row_view: &TranscriptRowView,
    draft: Option<&SegmentDraft>,
) -> f32 {
    if row_view.row_kind != TranscriptRowKind::Annotation {
        return MODIFIED_TRANSCRIPT_ROW_HEIGHT;
    }

    let mut height = draft
        .map(|draft| draft.text_editor_content_height() + 4.0)
        .unwrap_or(DEFAULT_TRANSCRIPT_ROW_HEIGHT)
        .max(DEFAULT_TRANSCRIPT_ROW_HEIGHT);
    if row_view.diff.is_some() {
        height += 24.0;
    }
    if row_view.annotation_diff.is_some() {
        height = height.max(DEFAULT_TRANSCRIPT_ROW_HEIGHT + 16.0);
    }
    height
}

pub(super) fn centered_row_scroll_offset(
    row_heights: &[f32],
    row_index: usize,
    viewport_height: f32,
) -> Option<f32> {
    if row_index >= row_heights.len() {
        return None;
    }

    let viewport_height = sanitized_viewport_height(viewport_height);
    let total_height = transcript_total_height(row_heights);
    let row_top = row_heights
        .iter()
        .take(row_index)
        .map(|height| sanitized_row_height(*height))
        .sum::<f32>();
    let row_height = sanitized_row_height(row_heights[row_index]);
    let max_offset = (total_height - viewport_height).max(0.0);
    let target = row_top + (row_height / 2.0) - (viewport_height / 2.0);
    Some(target.clamp(0.0, max_offset))
}

pub(super) fn transcript_total_height(row_heights: &[f32]) -> f32 {
    row_heights
        .iter()
        .map(|height| sanitized_row_height(*height))
        .sum()
}

pub(super) fn sanitized_viewport_height(viewport_height: f32) -> f32 {
    if viewport_height.is_finite() && viewport_height > 0.0 {
        viewport_height
    } else {
        DEFAULT_TRANSCRIPT_VIEWPORT_HEIGHT
    }
}

pub(super) fn sanitized_scroll_offset(offset_y: f32) -> f32 {
    sanitized_dimension(offset_y).unwrap_or(0.0)
}

fn sanitized_dimension(value: f32) -> Option<f32> {
    value.is_finite().then_some(value.max(0.0))
}

fn sanitized_row_height(value: f32) -> f32 {
    sanitized_dimension(value)
        .filter(|height| *height > 0.0)
        .unwrap_or(DEFAULT_TRANSCRIPT_ROW_HEIGHT)
}
