mod edit_support;

use edit_support::{doc, seg};
use exaudio_extension::document::{DocumentEdit, DocumentTransaction, QuickFix, TextRange};

#[test]
fn extension_transaction_is_one_undo_unit() {
    let mut doc = doc(vec![
        seg("a", "lane-1", 0.0, 1.0, "one"),
        seg("b", "lane-1", 2.0, 3.0, "two"),
    ]);
    let revision = doc.session().current_edit_revision();
    let _ = doc
        .apply_transaction(DocumentTransaction {
            label: "Two edits".into(),
            expected_revision: revision,
            edits: vec![
                DocumentEdit::ReplaceText {
                    segment_id: "a".into(),
                    range: None,
                    text: "first".into(),
                },
                DocumentEdit::ReplaceText {
                    segment_id: "b".into(),
                    range: None,
                    text: "second".into(),
                },
            ],
        })
        .unwrap();
    assert_eq!(doc.project().segment("a").unwrap().text, "first");
    assert_eq!(doc.project().segment("b").unwrap().text, "second");
    let _ = doc.undo().unwrap();
    assert_eq!(doc.project().segment("a").unwrap().text, "one");
    assert_eq!(doc.project().segment("b").unwrap().text, "two");
    assert!(!doc.can_undo());
}

#[test]
fn invalid_later_edit_rolls_back_project_history_and_revision() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);
    let before = doc.project().clone();
    let revision = doc.session().current_edit_revision();
    let result = doc.apply_transaction(DocumentTransaction {
        label: "Rejected batch".into(),
        expected_revision: revision,
        edits: vec![
            DocumentEdit::ReplaceText {
                segment_id: "a".into(),
                range: None,
                text: "changed".into(),
            },
            DocumentEdit::RemoveSegment {
                segment_id: "absent".into(),
            },
        ],
    });
    assert!(result.is_err());
    assert_eq!(doc.project(), &before);
    assert_eq!(doc.session().current_edit_revision(), revision);
    assert!(!doc.can_undo());
}

#[test]
fn stale_quick_fix_cannot_overwrite_a_newer_revision() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "one")]);
    let fix = QuickFix {
        title: "Old fix".into(),
        expected_revision: doc.session().current_edit_revision(),
        edits: vec![DocumentEdit::ReplaceText {
            segment_id: "a".into(),
            range: None,
            text: "stale".into(),
        }],
    };
    let _ = doc
        .apply_transaction(DocumentTransaction {
            label: "Newer edit".into(),
            expected_revision: fix.expected_revision,
            edits: vec![DocumentEdit::ReplaceText {
                segment_id: "a".into(),
                range: None,
                text: "new".into(),
            }],
        })
        .unwrap();
    assert!(doc.apply_quick_fix(&fix).is_err());
    assert_eq!(doc.project().segment("a").unwrap().text, "new");
}

#[test]
fn text_edits_use_validated_utf8_byte_ranges() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "ёж")]);
    assert!(
        doc.apply_transaction(DocumentTransaction {
            label: "Invalid byte boundary".into(),
            expected_revision: 0,
            edits: vec![DocumentEdit::ReplaceText {
                segment_id: "a".into(),
                range: Some(TextRange { start: 1, end: 2 }),
                text: "x".into()
            }],
        })
        .is_err()
    );
    assert_eq!(doc.project().segment("a").unwrap().text, "ёж");
    let _ = doc
        .apply_transaction(DocumentTransaction {
            label: "Valid Unicode replacement".into(),
            expected_revision: 0,
            edits: vec![DocumentEdit::ReplaceText {
                segment_id: "a".into(),
                range: Some(TextRange { start: 0, end: 2 }),
                text: "е".into(),
            }],
        })
        .unwrap();
    assert_eq!(doc.project().segment("a").unwrap().text, "еж");
}

#[test]
fn local_environment_has_no_implicit_commands() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "text")]);
    assert!(
        doc.execute_command("babel.autofix-row", &serde_json::Value::Null)
            .is_err()
    );
    assert_eq!(doc.session().current_edit_revision(), 0);
    assert_eq!(doc.project().segment("a").unwrap().text, "text");
    assert!(doc.completions("a").is_none());
}

#[test]
fn recovery_snapshot_includes_pending_drafts_without_committing() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "before")]);
    let _ = doc
        .edit_text(
            "a",
            "pending".into(),
            studio_editor::edit::TextSelection::caret(7),
        )
        .unwrap();
    let revision = doc.session().current_edit_revision();
    let snapshot = doc.recovery_project().unwrap();
    assert_eq!(snapshot.segment("a").unwrap().text, "pending");
    assert_eq!(doc.project().segment("a").unwrap().text, "before");
    assert_eq!(doc.session().current_edit_revision(), revision);
    assert!(!doc.can_undo());
    assert!(doc.has_pending_drafts());
}

#[test]
fn cancelled_staged_transaction_preserves_project_history_drafts_and_revision() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "original")]);
    let before = doc.project().clone();
    let revision = doc.session().current_edit_revision();
    let called = std::cell::Cell::new(false);
    let result = doc.apply_transaction_guarded(
        DocumentTransaction {
            label: "Cancelled edit".into(),
            expected_revision: revision,
            edits: vec![DocumentEdit::ReplaceText {
                segment_id: "a".into(),
                range: None,
                text: "staged replacement".into(),
            }],
        },
        || {
            called.set(true);
            Err(studio_editor::edit::EditError::Unavailable(
                "Request cancelled".into(),
            ))
        },
    );
    assert!(called.get());
    assert!(result.is_err());
    assert_eq!(doc.project(), &before);
    assert_eq!(doc.session().current_edit_revision(), revision);
    assert_eq!(doc.draft("a").unwrap().text(), "original");
    assert!(!doc.can_undo());
    assert!(!doc.can_redo());
}

#[test]
fn invalid_staged_transaction_never_reaches_commit_guard() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "original")]);
    let result = doc.apply_transaction_guarded(
        DocumentTransaction {
            label: "Invalid edit".into(),
            expected_revision: 0,
            edits: vec![DocumentEdit::RemoveSegment {
                segment_id: "absent".into(),
            }],
        },
        || panic!("invalid transactions must not reach the commit boundary"),
    );
    assert!(result.is_err());
    assert!(!doc.can_undo());
}
