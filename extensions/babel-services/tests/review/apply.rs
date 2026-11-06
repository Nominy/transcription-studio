use exaudio_gold::polish::{
    DraftRow, RedistributionAllocation, RedistributionMove, RedistributionPreview,
    apply_draft_rows, apply_redistribution, apply_redistribution_moves,
};

use super::sample_workflow;

fn row(row_id: &str, text: &str, status: &str) -> DraftRow {
    DraftRow {
        row_id: row_id.to_string(),
        rewritten_text: text.to_string(),
        status: status.to_string(),
        warnings: Vec::new(),
    }
}

fn text(workflow: &studio_editor::workflow::StudioWorkflow, id: &str) -> String {
    workflow.project().segment(id).unwrap().text.clone()
}

fn allocation(segment_id: &str, text: &str) -> RedistributionAllocation {
    RedistributionAllocation {
        segment_id: segment_id.to_string(),
        text: text.to_string(),
    }
}

fn preview(
    accept_draft: bool,
    moves: Vec<RedistributionMove>,
    allocations: Vec<RedistributionAllocation>,
) -> RedistributionPreview {
    RedistributionPreview {
        group_id: "g1".to_string(),
        model: None,
        accept_draft,
        notes: None,
        moves,
        allocations,
        error: None,
        applied: false,
    }
}

#[test]
fn draft_rows_apply_rewritten_rows_as_one_undo_step() {
    let mut workflow = sample_workflow();
    let start = workflow.project().segment("s1").unwrap().start_seconds;

    let outcome = apply_draft_rows(
        &mut workflow,
        &[
            row("s1", "Drafted one.", "rewritten"),
            row("s2", "Ignored.", "unchanged"),
            row("s3", "Drafted three.", "rewritten"),
            row("gone", "Missing.", "rewritten"),
        ],
    )
    .unwrap();

    assert_eq!(outcome.applied, vec!["s1", "s3"]);
    assert_eq!(outcome.missing, vec!["gone"]);
    assert_eq!(text(&workflow, "s1"), "Drafted one.");
    assert_eq!(text(&workflow, "s2"), "Second row.");
    assert_eq!(
        workflow.project().segment("s1").unwrap().start_seconds,
        start
    );
    assert!(workflow.can_undo());

    workflow.undo().unwrap();
    assert_eq!(text(&workflow, "s1"), "First row.");
    assert_eq!(text(&workflow, "s3"), "Third row.");
}

#[test]
fn draft_rows_without_applicable_rows_fail_without_editing() {
    let mut workflow = sample_workflow();
    assert!(apply_draft_rows(&mut workflow, &[row("s1", "x", "failed")]).is_err());
    assert!(apply_draft_rows(&mut workflow, &[row("gone", "x", "rewritten")]).is_err());
    assert!(!workflow.can_undo());
}

#[test]
fn accepted_redistribution_applies_draft_allocations_with_undo() {
    let mut workflow = sample_workflow();
    let accepted = preview(
        true,
        Vec::new(),
        vec![
            allocation("s1", "First row. Third"),
            allocation("s3", "row."),
        ],
    );

    let outcome = apply_redistribution(&mut workflow, &accepted).unwrap();

    assert_eq!(outcome.applied, vec!["s1", "s3"]);
    assert_eq!(text(&workflow, "s1"), "First row. Third");
    assert_eq!(text(&workflow, "s3"), "row.");
    workflow.undo().unwrap();
    assert_eq!(text(&workflow, "s1"), "First row.");
}

#[test]
fn model_moves_shift_sentences_between_adjacent_allocations() {
    let mut workflow = sample_workflow();
    let moved = preview(
        false,
        vec![RedistributionMove {
            from_index: 1,
            to_index: 2,
            sentence_count: 1,
        }],
        vec![allocation("s1", "One. Two!"), allocation("s3", "Three?")],
    );

    apply_redistribution(&mut workflow, &moved).unwrap();

    assert_eq!(text(&workflow, "s1"), "One.");
    assert_eq!(text(&workflow, "s3"), "Two! Three?");
    assert!(workflow.can_undo());
}

#[test]
fn backward_moves_append_leading_sentences() {
    let moved = apply_redistribution_moves(
        &[allocation("a", "One."), allocation("b", "Two… Three. tail")],
        &[RedistributionMove {
            from_index: 2,
            to_index: 1,
            sentence_count: 2,
        }],
    )
    .unwrap();
    assert_eq!(moved[0].text, "One. Two… Three.");
    assert_eq!(moved[1].text, "tail");
}

#[test]
fn invalid_moves_are_rejected() {
    let allocations = [allocation("a", "One."), allocation("b", "Two.")];
    for (from_index, to_index, sentence_count) in
        [(1, 3, 1), (0, 1, 1), (1, 2, 0), (2, 3, 1), (1, 2, 2)]
    {
        assert!(
            apply_redistribution_moves(
                &allocations,
                &[RedistributionMove {
                    from_index,
                    to_index,
                    sentence_count,
                }],
            )
            .is_err(),
            "{from_index}->{to_index} x{sentence_count}"
        );
    }
}

#[test]
fn unaccepted_group_without_moves_or_with_error_is_not_applied() {
    let mut workflow = sample_workflow();
    assert!(
        apply_redistribution(
            &mut workflow,
            &preview(false, Vec::new(), vec![allocation("s1", "x")])
        )
        .is_err()
    );
    let mut failed = preview(true, Vec::new(), vec![allocation("s1", "x")]);
    failed.error = Some("upstream failed".to_string());
    assert!(apply_redistribution(&mut workflow, &failed).is_err());
    assert!(
        apply_redistribution(
            &mut workflow,
            &preview(true, Vec::new(), vec![allocation("gone", "x")])
        )
        .is_err()
    );
    assert!(!workflow.can_undo());
}
