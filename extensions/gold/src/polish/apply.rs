//! Applying drafting results to the project as undoable workflow edits.

use anyhow::{Result, bail};
use studio_core::SegmentUpdate;
use studio_editor::workflow::StudioWorkflow;

use super::model::{DraftRow, RedistributionAllocation, RedistributionMove, RedistributionPreview};

/// Result of applying rows: updated segment ids (one undo step) and row ids not in the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppliedRows {
    pub applied: Vec<String>,
    pub missing: Vec<String>,
}

/// Applies every `rewritten` row as one undoable multi-segment edit, keeping segment timings.
/// Errors when no row is rewritten or none exists in the project.
pub fn apply_draft_rows(workflow: &mut StudioWorkflow, rows: &[DraftRow]) -> Result<AppliedRows> {
    let rewritten = rows
        .iter()
        .filter(|row| row.status == "rewritten")
        .map(|row| (row.row_id.as_str(), row.rewritten_text.as_str()))
        .collect::<Vec<_>>();
    if rewritten.is_empty() {
        bail!("The draft has no rewritten segments to apply.");
    }
    let outcome = apply_texts(workflow, rewritten)?;
    if outcome.applied.is_empty() {
        bail!("The draft's segments are no longer in this project.");
    }
    Ok(outcome)
}

/// Applies a redistribution group: accepted drafts use the request allocations as-is, otherwise
/// the model moves are applied to them. One undo step.
pub fn apply_redistribution(
    workflow: &mut StudioWorkflow,
    preview: &RedistributionPreview,
) -> Result<AppliedRows> {
    if let Some(error) = &preview.error {
        bail!("{error}");
    }
    let allocations = if preview.accept_draft {
        preview.allocations.clone()
    } else if preview.moves.is_empty() {
        bail!(
            "group `{}` was not accepted and has no model moves.",
            preview.group_id
        );
    } else {
        apply_redistribution_moves(&preview.allocations, &preview.moves)?
    };
    let outcome = apply_texts(
        workflow,
        allocations
            .iter()
            .map(|allocation| (allocation.segment_id.as_str(), allocation.text.as_str())),
    )?;
    if outcome.applied.is_empty() {
        bail!("group `{}` has no project rows.", preview.group_id);
    }
    Ok(outcome)
}

/// Moves whole sentences between adjacent allocations (1-based indexes). Moving forward takes
/// the source's trailing sentences to the target's front; moving back takes leading sentences
/// to the target's end. Sentences end at `.`, `?`, `!` or `…`.
pub fn apply_redistribution_moves(
    allocations: &[RedistributionAllocation],
    moves: &[RedistributionMove],
) -> Result<Vec<RedistributionAllocation>> {
    let mut next = allocations.to_vec();
    for item in moves {
        if item.from_index == 0
            || item.to_index == 0
            || item.sentence_count == 0
            || item.from_index.abs_diff(item.to_index) != 1
        {
            bail!(
                "invalid model move {}->{} x{}.",
                item.from_index,
                item.to_index,
                item.sentence_count
            );
        }
        let (from, to) = (item.from_index - 1, item.to_index - 1);
        if from >= next.len() || to >= next.len() {
            bail!(
                "model move {}->{} is outside {} allocations.",
                item.from_index,
                item.to_index,
                next.len()
            );
        }
        let mut source = split_sentences(&next[from].text);
        let mut target = split_sentences(&next[to].text);
        if source.len() < item.sentence_count {
            bail!(
                "allocation {} has only {} sentence(s), cannot move {}.",
                item.from_index,
                source.len(),
                item.sentence_count
            );
        }
        if to > from {
            let mut moved = source.split_off(source.len() - item.sentence_count);
            moved.append(&mut target);
            target = moved;
        } else {
            target.extend(source.drain(..item.sentence_count));
        }
        next[from].text = source.join(" ");
        next[to].text = target.join(" ");
    }
    Ok(next)
}

fn apply_texts<'a>(
    workflow: &mut StudioWorkflow,
    rows: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<AppliedRows> {
    let mut outcome = AppliedRows::default();
    let mut updates = Vec::new();
    for (row_id, text) in rows {
        let Some(segment) = workflow.project().segment(row_id) else {
            outcome.missing.push(row_id.to_string());
            continue;
        };
        updates.push(SegmentUpdate {
            segment_id: row_id.to_string(),
            start_seconds: segment.start_seconds,
            end_seconds: segment.end_seconds,
            text: text.to_string(),
        });
        outcome.applied.push(row_id.to_string());
    }
    if !updates.is_empty() {
        workflow.apply_segment_edits(updates)?;
    }
    Ok(outcome)
}

fn split_sentences(value: &str) -> Vec<String> {
    let mut sentences = Vec::new();
    let mut start = 0;
    for (index, ch) in value.char_indices() {
        if matches!(ch, '.' | '?' | '!' | '…') {
            let end = index + ch.len_utf8();
            let sentence = value[start..end].trim();
            if !sentence.is_empty() {
                sentences.push(sentence.to_string());
            }
            start = end;
        }
    }
    let tail = value[start..].trim();
    if !tail.is_empty() {
        sentences.push(tail.to_string());
    }
    sentences
}
