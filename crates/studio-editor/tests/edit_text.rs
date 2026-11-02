//! Draft text, local/project undo, pending-draft commit, time cells, helper text moves/SKAZ,
//! lint autofix and tag suggestions.
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_core::Project;
use studio_editor::edit::draft::{
    SegmentDraft, TEXT_HISTORY_MAX_ENTRIES, TEXT_HISTORY_MAX_RETAINED_BYTES,
};
use studio_editor::edit::tags::{
    TagSuggestion, TagSuggestionKey, TagSuggestionKind, suggestions_for_text,
};
use studio_editor::edit::text::{
    CROSS_ROW_HISTORY_MAX_ENTRIES, CROSS_ROW_HISTORY_MAX_RETAINED_BYTES, CrossRowTextEdit,
    TextMoveError, push_cross_row_history,
};
use studio_editor::edit::time::{
    CommitTimeCellEdit, CommitTimeCellResult, TimeEditError, TimeEditState, TimeField,
    TimeRangeEdit, commit_time_cell_edit,
};
use studio_editor::edit::{EditError, EditorDoc, Effect, TextSelection};
use studio_editor::project_lint_tag_config::{
    BabelProjectLintTagConfig, META_BABEL_PROJECT_ID, resolve_babel_project_lint_tag_config,
};

// ---- helpers ------------------------------------------------------------------------------

/// Types `text` into the row with the caret after `caret_chars` chars.
fn set_text_with_caret(doc: &mut EditorDoc, segment_id: &str, text: &str, caret_chars: usize) {
    ok(doc.edit_text(
        segment_id,
        text.to_string(),
        TextSelection::caret(caret_chars),
    ));
}

fn set_text_with_caret_at_end(doc: &mut EditorDoc, segment_id: &str, text: &str) {
    set_text_with_caret(doc, segment_id, text, text.chars().count());
}

fn set_text_selected_all(doc: &mut EditorDoc, segment_id: &str, text: &str) {
    ok(doc.edit_text(
        segment_id,
        text.to_string(),
        TextSelection::new(0, text.chars().count()),
    ));
}

fn is_dirty(doc: &EditorDoc) -> bool {
    doc.session().is_dirty
}

fn clear_dirty(doc: &mut EditorDoc) {
    doc.session_mut().is_dirty = false;
}

fn caret(doc: &EditorDoc, segment_id: &str) -> usize {
    doc.draft(segment_id).unwrap().selection().head
}

fn has_draft_changed(effects: &[Effect], segment_id: &str) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::DraftChanged { segment_id: id } if id == segment_id))
}

fn default_config() -> BabelProjectLintTagConfig {
    BabelProjectLintTagConfig::default()
}

fn find_suggestion(suggestions: Vec<TagSuggestion>, label: &str) -> TagSuggestion {
    suggestions
        .into_iter()
        .find(|suggestion| suggestion.label == label)
        .unwrap_or_else(|| panic!("suggestion `{label}` should be offered"))
}

fn doc_suggestion(doc: &EditorDoc, segment_id: &str, label: &str) -> TagSuggestion {
    let list = doc
        .tag_suggestions(segment_id)
        .expect("tag suggestions should be open");
    find_suggestion(list.items, label)
}

fn labels(suggestions: &[TagSuggestion]) -> Vec<&str> {
    suggestions
        .iter()
        .map(|suggestion| suggestion.label.as_str())
        .collect()
}

fn config_for_project_id(project_id: &str) -> BabelProjectLintTagConfig {
    let mut project: Project = project(Vec::new());
    project
        .metadata
        .insert(META_BABEL_PROJECT_ID.to_string(), project_id.to_string());
    resolve_babel_project_lint_tag_config(&project)
}

fn draft_with_text(text: &str) -> SegmentDraft {
    SegmentDraft::from_segment(&seg("segment", "lane-1", 0.0, 1.0, text))
}

fn cross_row_entry(index: usize, snapshot: String) -> CrossRowTextEdit {
    CrossRowTextEdit {
        source_segment_id: format!("source-{index}"),
        target_segment_id: format!("target-{index}"),
        source_before: snapshot.clone(),
        source_after: snapshot.clone(),
        target_before: snapshot.clone(),
        target_after: snapshot,
        source_before_caret: index,
        source_after_caret: index,
    }
}

fn cross_row_bytes(entries: &[CrossRowTextEdit]) -> usize {
    entries
        .iter()
        .map(CrossRowTextEdit::retained_utf8_bytes)
        .sum()
}

// ---- drafts -------------------------------------------------------------------------------

#[test]
fn segment_drafts_use_babel_timestamp_format() {
    let doc = stable_sample_doc();
    let first = doc.draft("annotation-1").unwrap();
    let long = doc.draft("annotation-5").unwrap();

    assert_eq!(first.time_value(TimeField::Start), "00:01.20");
    assert_eq!(first.time_value(TimeField::End), "00:03.40");
    assert_eq!(long.time_value(TimeField::Start), "01:05.08");
}

#[test]
fn segment_drafts_initialize_from_multiline_babel_text() {
    let doc = stable_sample_doc();
    assert_eq!(
        draft_text(&doc, "annotation-4"),
        text_of(&doc, "annotation-4")
    );
}

#[test]
fn transcript_text_edit_updates_draft_text_and_dirty_state() {
    let mut doc = stable_sample_doc();
    let edited = format!("{} edited", draft_text(&doc, "annotation-1"));
    set_text_with_caret_at_end(&mut doc, "annotation-1", &edited);

    assert_eq!(draft_text(&doc, "annotation-1"), edited);
    assert!(is_dirty(&doc));
    assert_eq!(text_of(&doc, "annotation-1"), "Здравствуйте.");
}

#[test]
fn transcript_text_undo_redo_stays_in_draft_without_project_history() {
    let mut doc = stable_sample_doc();
    let original = draft_text(&doc, "annotation-1");
    assert!(!doc.can_undo());

    let edited = format!("{original} edited");
    set_text_with_caret_at_end(&mut doc, "annotation-1", &edited);
    assert!(!doc.can_undo());

    let undo = ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), original);
    assert!(has_draft_changed(&undo.effects, "annotation-1"));
    assert!(!doc.can_undo());

    let redo = ok(doc.redo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), edited);
    assert!(has_draft_changed(&redo.effects, "annotation-1"));
    assert!(!doc.can_undo());
}

#[test]
fn read_only_project_blocks_transcript_text_and_structure_edits() {
    let mut doc = read_only_doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "first"),
        seg("b", "lane-1", 3.0, 5.0, "second"),
    ]);
    assert!(doc.is_read_only());

    let err = doc
        .edit_text("a", "first blocked".to_string(), TextSelection::caret(13))
        .unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    assert_eq!(draft_text(&doc, "a"), "first");
    assert!(!is_dirty(&doc));

    let err = doc.delete_segment("a").unwrap_err();
    assert!(matches!(err, EditError::ReadOnly { .. }));
    assert_eq!(ids(&doc), vec!["a", "b"]);
    assert!(!is_dirty(&doc));
}

#[test]
fn draft_sync_preserves_unapplied_edits() {
    let mut doc = stable_sample_doc();
    let pending = format!("{} pending", draft_text(&doc, "annotation-1"));
    set_text_with_caret_at_end(&mut doc, "annotation-1", &pending);

    doc.with_workflow(|_| ());

    assert_eq!(draft_text(&doc, "annotation-1"), pending);
}

#[test]
fn unrelated_actions_preserve_unapplied_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "draft still being typed");

    ok(doc.split_segment_at("annotation-2", 4.3));

    assert_eq!(draft_text(&doc, "annotation-1"), "draft still being typed");
}

#[test]
fn find_matches_track_live_draft_text_edits() {
    let mut doc = stable_sample_doc();
    doc.open_find();
    doc.set_find_query("draft-only-token".to_string());
    assert!(doc.find_matches().is_empty());

    set_text_with_caret_at_end(&mut doc, "annotation-1", "draft-only-token");

    assert_eq!(doc.find_matches(), vec!["annotation-1".to_string()]);
    assert_eq!(doc.find_summary().map(|summary| summary.total), Some(1));
}

// ---- local text history bounds ------------------------------------------------------------

#[test]
fn thousands_of_edits_keep_only_the_latest_undo_chain_and_redo_it() {
    let edit_count = TEXT_HISTORY_MAX_ENTRIES + 2_000;
    let mut draft = draft_with_text("edit-0");
    for index in 1..=edit_count {
        draft.replace_text_from_user_edit(format!("edit-{index}"));
    }

    assert_eq!(draft.undo_depth(), TEXT_HISTORY_MAX_ENTRIES);

    for expected in (edit_count - TEXT_HISTORY_MAX_ENTRIES..edit_count).rev() {
        assert!(draft.undo());
        assert_eq!(draft.text(), format!("edit-{expected}"));
    }
    assert!(!draft.undo());

    for expected in edit_count - TEXT_HISTORY_MAX_ENTRIES + 1..=edit_count {
        assert!(draft.redo());
        assert_eq!(draft.text(), format!("edit-{expected}"));
    }
    assert!(!draft.redo());
}

#[test]
fn text_history_is_bounded_by_utf8_bytes() {
    let mut draft = draft_with_text("initial");
    for index in 0..80 {
        draft.replace_text_from_user_edit(format!("{index}:{}", "я".repeat(32 * 1024)));
    }

    let depth = draft.undo_depth();
    assert!(depth < TEXT_HISTORY_MAX_ENTRIES);

    let mut retained = 0;
    let mut expected_index = 78;
    while draft.undo() {
        retained += draft.text().len();
        assert!(draft.text().starts_with(&format!("{expected_index}:")));
        expected_index -= 1;
    }
    assert_eq!(78 - expected_index, depth);
    assert!(retained <= TEXT_HISTORY_MAX_RETAINED_BYTES);
}

#[test]
fn newest_oversized_snapshot_remains_immediately_undoable() {
    let mut draft = draft_with_text("initial");
    draft.replace_text_from_user_edit("small edit".to_string());
    let oversized = "x".repeat(TEXT_HISTORY_MAX_RETAINED_BYTES + 1);
    draft.replace_text_from_user_edit(oversized.clone());
    draft.replace_text_from_user_edit("current".to_string());

    assert_eq!(draft.undo_depth(), 1);
    assert!(draft.undo());
    assert_eq!(draft.text(), oversized);
}

#[test]
fn cross_row_history_keeps_the_newest_fixed_count() {
    let mut entries = Vec::new();
    for index in 0..5_000 {
        push_cross_row_history(&mut entries, cross_row_entry(index, String::new()));
    }

    assert_eq!(entries.len(), CROSS_ROW_HISTORY_MAX_ENTRIES);
    assert_eq!(
        entries.first().unwrap().source_segment_id,
        format!("source-{}", 5_000 - CROSS_ROW_HISTORY_MAX_ENTRIES)
    );
    assert_eq!(entries.last().unwrap().source_segment_id, "source-4999");
    assert!(cross_row_bytes(&entries) <= CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
}

#[test]
fn cross_row_history_is_bounded_by_all_retained_utf8_snapshots() {
    let mut entries = Vec::new();
    for index in 0..20 {
        push_cross_row_history(&mut entries, cross_row_entry(index, "я".repeat(32 * 1024)));
    }

    assert!(entries.len() < CROSS_ROW_HISTORY_MAX_ENTRIES);
    assert!(cross_row_bytes(&entries) <= CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
    assert_eq!(entries.last().unwrap().source_segment_id, "source-19");
}

#[test]
fn cross_row_history_keeps_one_newest_oversized_move() {
    let mut entries = Vec::new();
    push_cross_row_history(&mut entries, cross_row_entry(0, String::new()));
    push_cross_row_history(
        &mut entries,
        cross_row_entry(1, "x".repeat(CROSS_ROW_HISTORY_MAX_RETAINED_BYTES)),
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source_segment_id, "source-1");
    assert!(cross_row_bytes(&entries) > CROSS_ROW_HISTORY_MAX_RETAINED_BYTES);
}

// ---- tag suggestions ----------------------------------------------------------------------

#[test]
fn tag_suggestions_match_babel_square_bracket_trigger() {
    let text = "это [не";
    let suggestion = find_suggestion(
        suggestions_for_text(text, text.len(), &default_config()),
        "неразборчиво",
    );

    assert_eq!(suggestion.kind, TagSuggestionKind::Bracket);
    assert_eq!(suggestion.insert_text, "[неразборчиво]");
    assert_eq!(suggestion.replacement_start, "это ".len());
    assert_eq!(suggestion.replacement_end, text.len());
}

#[test]
fn tag_suggestions_match_babel_style_open_and_close_triggers() {
    let open_text = "говорит <нара";
    let open = find_suggestion(
        suggestions_for_text(open_text, open_text.len(), &default_config()),
        "<речь-нараспев>",
    );
    assert_eq!(open.kind, TagSuggestionKind::StyleOpen);
    assert_eq!(open.insert_text, "<речь-нараспев></речь-нараспев>");
    assert_eq!(open.cursor_back_count, "речь-нараспев>".chars().count() + 2);

    let close_text = "говорит </смех";
    let close = find_suggestion(
        suggestions_for_text(close_text, close_text.len(), &default_config()),
        "</смех-в-речи>",
    );
    assert_eq!(close.kind, TagSuggestionKind::StyleClose);
    assert_eq!(close.insert_text, "</смех-в-речи>");
    assert_eq!(close.cursor_back_count, 0);
}

#[test]
fn tag_suggestions_match_babel_curly_mis_and_pro_triggers() {
    let mis_text = "слово {M";
    let mis = find_suggestion(
        suggestions_for_text(mis_text, mis_text.len(), &default_config()),
        "MIS:",
    );
    assert_eq!(mis.kind, TagSuggestionKind::Curly);
    assert_eq!(mis.insert_text, "{MIS: }");
    assert_eq!(mis.replacement_start, "слово ".len());
    assert_eq!(mis.replacement_end, mis_text.len());
    assert_eq!(mis.cursor_back_count, 1);

    let pro_text = "слово {pro";
    let pro = find_suggestion(
        suggestions_for_text(pro_text, pro_text.len(), &default_config()),
        "PRO:",
    );
    assert_eq!(pro.kind, TagSuggestionKind::Curly);
    assert_eq!(pro.insert_text, "{PRO: }");
}

#[test]
fn tag_suggestions_ignore_invalid_non_suffix_partial() {
    let text = "hello [не word";
    assert!(suggestions_for_text(text, text.len(), &default_config()).is_empty());
}

#[test]
fn tag_suggestions_use_contains_matching_like_babel() {
    let suggestions = suggestions_for_text("[раз", "[раз".len(), &default_config());
    assert!(suggestions.iter().any(|suggestion| {
        suggestion.kind == TagSuggestionKind::Bracket && suggestion.label == "неразборчиво"
    }));
}

#[test]
fn tag_suggestions_use_project_config_for_russian_gold_only_tags() {
    let config = config_for_project_id("0bf29f0c-371a-506a-8b77-5fffd14e5762");

    let bracket = suggestions_for_text("[муз", "[муз".len(), &config);
    let style = suggestions_for_text("<под", "<под".len(), &config);

    assert_eq!(labels(&bracket), vec!["музыка"]);
    assert_eq!(bracket[0].kind, TagSuggestionKind::Bracket);
    assert_eq!(labels(&style), vec!["<подражание>"]);
    assert_eq!(style[0].kind, TagSuggestionKind::StyleOpen);
}

#[test]
fn russian_silver_tag_suggestions_only_offer_unintelligible_bracket_tag() {
    let config = config_for_project_id("73f8e130-9fb4-5c17-b142-5a2e75d89b6d");

    let bracket = suggestions_for_text("[", 1, &config);
    let style = suggestions_for_text("<", 1, &config);
    let curly = suggestions_for_text("{", 1, &config);

    assert_eq!(labels(&bracket), vec!["неразборчиво"]);
    assert_eq!(bracket[0].kind, TagSuggestionKind::Bracket);
    assert!(style.is_empty());
    assert!(curly.is_empty());
}

#[test]
fn doc_tag_suggestions_follow_project_tag_config() {
    let mut project = project(vec![seg("a", "lane-1", 0.0, 2.0, "")]);
    project.metadata.insert(
        META_BABEL_PROJECT_ID.to_string(),
        "73f8e130-9fb4-5c17-b142-5a2e75d89b6d".to_string(),
    );
    let mut doc = EditorDoc::from_project(project);

    set_text_with_caret_at_end(&mut doc, "a", "[");
    let list = doc.tag_suggestions("a").expect("bracket suggestions");
    assert_eq!(labels(&list.items), vec!["неразборчиво"]);

    set_text_with_caret_at_end(&mut doc, "a", "<");
    assert!(doc.tag_suggestions("a").is_none());
}

#[test]
fn applying_tag_suggestion_replaces_partial_trigger_and_marks_dirty() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "слово [не");
    clear_dirty(&mut doc);
    let suggestion = doc_suggestion(&doc, "annotation-1", "неразборчиво");

    let outcome = ok(doc.apply_tag_suggestion("annotation-1", &suggestion));

    assert_eq!(draft_text(&doc, "annotation-1"), "слово [неразборчиво]");
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(is_dirty(&doc));
}

#[test]
fn applying_curly_tag_suggestion_places_cursor_before_closing_brace() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "слово {M");
    clear_dirty(&mut doc);
    let suggestion = doc_suggestion(&doc, "annotation-1", "MIS:");

    ok(doc.apply_tag_suggestion("annotation-1", &suggestion));

    assert_eq!(draft_text(&doc, "annotation-1"), "слово {MIS: }");
    assert_eq!(caret(&doc, "annotation-1"), "слово {MIS: ".chars().count());
    assert!(is_dirty(&doc));
}

#[test]
fn applying_tag_suggestion_stays_in_local_text_undo_history() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "слово [не");
    let suggestion = doc_suggestion(&doc, "annotation-1", "неразборчиво");

    ok(doc.apply_tag_suggestion("annotation-1", &suggestion));
    assert_eq!(draft_text(&doc, "annotation-1"), "слово [неразборчиво]");

    ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), "слово [не");

    ok(doc.redo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), "слово [неразборчиво]");
}

#[test]
fn applying_style_open_suggestion_places_cursor_inside_babel_style_pair() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "слово <нара");
    let suggestion = doc_suggestion(&doc, "annotation-1", "<речь-нараспев>");

    ok(doc.apply_tag_suggestion("annotation-1", &suggestion));

    assert_eq!(
        draft_text(&doc, "annotation-1"),
        "слово <речь-нараспев></речь-нараспев>"
    );
    assert_eq!(
        caret(&doc, "annotation-1"),
        "слово <речь-нараспев>".chars().count()
    );
}

#[test]
fn applying_style_tag_suggestion_wraps_selected_text_and_marks_dirty() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "тихий голос");
    doc.set_selection(
        "annotation-1",
        TextSelection::new(0, "тихий голос".chars().count()),
    );
    clear_dirty(&mut doc);
    let suggestion = doc_suggestion(&doc, "annotation-1", "<речь-нараспев>");

    ok(doc.apply_tag_suggestion("annotation-1", &suggestion));

    assert_eq!(
        draft_text(&doc, "annotation-1"),
        "<речь-нараспев>тихий голос</речь-нараспев>"
    );
    assert!(is_dirty(&doc));

    ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), "тихий голос");
}

#[test]
fn tag_suggestion_keyboard_accepts_highlighted_babel_suggestion() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "[сме");
    clear_dirty(&mut doc);
    assert_eq!(doc.tag_suggestions("annotation-1").unwrap().highlighted, 0);

    ok(doc.tag_suggestion_key("annotation-1", TagSuggestionKey::Next));
    assert_eq!(doc.tag_suggestions("annotation-1").unwrap().highlighted, 1);

    ok(doc.tag_suggestion_key("annotation-1", TagSuggestionKey::Accept));

    assert_eq!(draft_text(&doc, "annotation-1"), "[смех]");
    assert!(is_dirty(&doc));
    assert!(doc.tag_suggestions("annotation-1").is_none());
}

#[test]
fn tag_suggestion_escape_dismisses_current_trigger_until_text_changes() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "[сме");
    assert!(doc.tag_suggestions("annotation-1").is_some());

    ok(doc.tag_suggestion_key("annotation-1", TagSuggestionKey::Dismiss));
    assert!(doc.tag_suggestions("annotation-1").is_none());

    set_text_with_caret_at_end(&mut doc, "annotation-1", "[смех");
    assert!(doc.tag_suggestions("annotation-1").is_some());
}

// ---- helper text moves / SKAZ -------------------------------------------------------------

#[test]
fn helper_text_move_before_caret_targets_previous_same_lane_segment() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-3");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "previous");
    set_text_with_caret(&mut doc, "annotation-3", "move me stay", 7);
    clear_dirty(&mut doc);

    let outcome = ok(doc.move_text_to_previous_row("annotation-3"));

    assert_eq!(draft_text(&doc, "annotation-1"), "previous move me");
    assert_eq!(draft_text(&doc, "annotation-3"), "stay");
    assert_eq!(caret(&doc, "annotation-3"), 0);
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(has_draft_changed(&outcome.effects, "annotation-3"));
    assert!(is_dirty(&doc));
}

#[test]
fn helper_text_move_after_caret_targets_next_same_lane_segment() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "next");
    set_text_with_caret(&mut doc, "annotation-1", "keep move me", 4);
    clear_dirty(&mut doc);

    ok(doc.move_text_to_next_row("annotation-1"));

    assert_eq!(draft_text(&doc, "annotation-1"), "keep");
    assert_eq!(caret(&doc, "annotation-1"), "keep".chars().count());
    assert_eq!(draft_text(&doc, "annotation-3"), "move me next");
    assert!(is_dirty(&doc));
}

#[test]
fn helper_text_move_uses_timeline_order_for_visual_first_row() {
    // `later` is first in project order but last on the timeline.
    let mut doc = doc(vec![
        seg("later", "lane-1", 5.0, 6.0, "later row"),
        seg("first", "lane-1", 1.0, 2.0, "first visible"),
    ]);
    doc.select_segment("first");
    doc.set_selection("first", TextSelection::caret(13));

    let err = doc.move_text_to_previous_row("first").unwrap_err();

    assert!(matches!(
        err,
        EditError::TextMove(TextMoveError::NoAdjacentSegment)
    ));
    assert_eq!(draft_text(&doc, "first"), "first visible");
    assert_eq!(draft_text(&doc, "later"), "later row");
    assert!(!is_dirty(&doc));
}

#[test]
fn helper_text_move_refuses_to_empty_source_in_either_direction() {
    let mut before = stable_sample_doc();
    before.select_segment("annotation-3");
    set_text_with_caret_at_end(&mut before, "annotation-1", "previous");
    set_text_with_caret(&mut before, "annotation-3", "whole row", 9);
    clear_dirty(&mut before);

    let err = before
        .move_text_to_previous_row("annotation-3")
        .unwrap_err();
    assert!(matches!(
        err,
        EditError::TextMove(TextMoveError::WouldEmptySource)
    ));
    assert_eq!(draft_text(&before, "annotation-3"), "whole row");
    assert_eq!(draft_text(&before, "annotation-1"), "previous");
    assert!(!is_dirty(&before));

    let mut after = stable_sample_doc();
    after.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut after, "annotation-3", "next");
    set_text_with_caret(&mut after, "annotation-1", "whole row", 0);
    clear_dirty(&mut after);

    let err = after.move_text_to_next_row("annotation-1").unwrap_err();
    assert!(matches!(
        err,
        EditError::TextMove(TextMoveError::WouldEmptySource)
    ));
    assert_eq!(draft_text(&after, "annotation-1"), "whole row");
    assert_eq!(draft_text(&after, "annotation-3"), "next");
    assert!(!is_dirty(&after));
}

#[test]
fn helper_text_move_undo_redo_restores_both_rows_atomically() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-3");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "previous");
    set_text_with_caret(&mut doc, "annotation-3", "move me stay", 7);

    ok(doc.move_text_to_previous_row("annotation-3"));
    let undo = ok(doc.undo_text("annotation-3"));

    assert_eq!(draft_text(&doc, "annotation-1"), "previous");
    assert_eq!(draft_text(&doc, "annotation-3"), "move me stay");
    assert_eq!(caret(&doc, "annotation-3"), 7);
    assert!(has_draft_changed(&undo.effects, "annotation-1"));
    assert!(has_draft_changed(&undo.effects, "annotation-3"));

    ok(doc.redo_text("annotation-3"));

    assert_eq!(draft_text(&doc, "annotation-1"), "previous move me");
    assert_eq!(draft_text(&doc, "annotation-3"), "stay");
    assert_eq!(caret(&doc, "annotation-3"), 0);
}

#[test]
fn helper_selected_number_to_skaz_converts_selected_digits() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_selected_all(&mut doc, "annotation-1", "1234");
    clear_dirty(&mut doc);

    let outcome = ok(doc.convert_selection_to_skaz());

    assert_eq!(
        draft_text(&doc, "annotation-1"),
        "1 234 {СКАЗ: одна тысяча двести тридцать четыре}"
    );
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(is_dirty(&doc));
}

#[test]
fn helper_selected_number_to_skaz_prefers_helper_focused_row() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-2");
    set_text_selected_all(&mut doc, "annotation-1", "1234");
    doc.set_text_focus(Some("annotation-1"));

    ok(doc.convert_selection_to_skaz());

    assert_eq!(
        draft_text(&doc, "annotation-1"),
        "1 234 {СКАЗ: одна тысяча двести тридцать четыре}"
    );
}

#[test]
fn helper_digit_over_selection_preserves_babel_skaz_cursor_offset() {
    let mut doc = stable_sample_doc();
    set_text_selected_all(&mut doc, "annotation-1", "семь восемь");
    clear_dirty(&mut doc);

    ok(doc.convert_selection_with_digit("annotation-1", '5'));

    assert_eq!(draft_text(&doc, "annotation-1"), "5 {СКАЗ: семь восемь}");
    assert_eq!(caret(&doc, "annotation-1"), 1);
    assert!(is_dirty(&doc));
}

// ---- lint autofix -------------------------------------------------------------------------

#[test]
fn helper_lint_autofix_current_row_applies_common_babel_text_hygiene() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "  привет,мир  ");
    clear_dirty(&mut doc);

    let outcome = ok(doc.autofix_current_row());

    assert_eq!(draft_text(&doc, "annotation-1"), "Привет, мир.");
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(is_dirty(&doc));
}

#[test]
fn helper_lint_autofix_current_row_is_locally_undoable() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "  привет,мир  ");

    ok(doc.autofix_current_row());
    ok(doc.undo_text("annotation-1"));

    assert_eq!(draft_text(&doc, "annotation-1"), "  привет,мир  ");
}

#[test]
fn helper_lint_autofix_all_rows_uses_same_lane_previous_context() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "первая  строка");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "следующая,строка");
    clear_dirty(&mut doc);

    ok(doc.autofix_all_rows());

    assert_eq!(draft_text(&doc, "annotation-1"), "Первая строка.");
    assert_eq!(draft_text(&doc, "annotation-3"), "Следующая, строка.");
    assert!(is_dirty(&doc));
}

#[test]
fn helper_lint_autofix_preserves_lowercase_after_same_lane_continuation_marker() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "это --");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "продолжение");

    ok(doc.autofix_all_rows());

    assert_eq!(draft_text(&doc, "annotation-1"), "Это --");
    assert_eq!(draft_text(&doc, "annotation-3"), "продолжение.");
}

#[test]
fn helper_lint_autofix_moves_tag_trailing_punctuation_like_babel_linter() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "привет {MIS: шум}?");

    ok(doc.autofix_current_row());

    assert_eq!(draft_text(&doc, "annotation-1"), "Привет? {MIS: шум}.");
}

#[test]
fn helper_lint_autofix_moves_square_tag_dash_and_normalizes_free_double_dash() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "ну, - вот [смех]- дальше -- тут");

    ok(doc.autofix_current_row());

    assert_eq!(
        draft_text(&doc, "annotation-1"),
        "Ну - вот- [смех] дальше - тут."
    );
}

#[test]
fn helper_lint_autofix_normalizes_interjection_variants() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "эээ уау чорт");

    ok(doc.autofix_current_row());

    assert_eq!(draft_text(&doc, "annotation-1"), "Э вау чёрт.");
}

#[test]
fn helper_lint_autofix_lowercases_polite_pronouns_only_inside_sentence() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "спасибо Вам за Ваш ответ");
    set_text_with_caret_at_end(&mut doc, "annotation-2", "Вам спасибо");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "вопрос: Вам удобно");

    ok(doc.autofix_all_rows());

    assert_eq!(
        draft_text(&doc, "annotation-1"),
        "Спасибо вам за ваш ответ."
    );
    assert_eq!(draft_text(&doc, "annotation-2"), "Вам спасибо.");
    assert_eq!(draft_text(&doc, "annotation-3"), "Вопрос: Вам удобно.");
}

#[test]
fn helper_lint_autofix_current_row_requires_selection() {
    let mut doc = doc(vec![seg("a", "lane-1", 0.0, 1.0, "привет,мир")]);
    doc.session_mut().selected_segment_id = None;

    let err = doc.autofix_current_row().unwrap_err();

    assert!(matches!(err, EditError::NoSelection { .. }));
    assert_eq!(draft_text(&doc, "a"), "привет,мир");
}

// ---- time cells ---------------------------------------------------------------------------

#[test]
fn time_cell_commit_rules_match_babel_submit_and_blur() {
    let invalid_submit = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: Some(TimeField::Start),
        field: TimeField::Start,
        draft_value: "00:03.40".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: false,
    });
    assert_eq!(
        invalid_submit,
        CommitTimeCellResult::Handled {
            active_field: Some(TimeField::Start),
            draft_value: "00:03.40".to_string(),
            error: Some(TimeEditError::StartNotBeforeEnd),
            next_times: None,
        }
    );

    let invalid_blur = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: Some(TimeField::Start),
        field: TimeField::Start,
        draft_value: "not a time".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: true,
    });
    assert_eq!(
        invalid_blur,
        CommitTimeCellResult::Handled {
            active_field: None,
            draft_value: "00:01.20".to_string(),
            error: None,
            next_times: None,
        }
    );

    let valid_submit = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: Some(TimeField::End),
        field: TimeField::End,
        draft_value: "00:03.75".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: false,
    });
    assert_eq!(
        valid_submit,
        CommitTimeCellResult::Handled {
            active_field: None,
            draft_value: "00:03.75".to_string(),
            error: None,
            next_times: Some(TimeRangeEdit {
                start_seconds: 1.2,
                end_seconds: 3.75,
            }),
        }
    );
}

#[test]
fn time_cell_commit_is_ignored_when_field_is_not_active() {
    let result = commit_time_cell_edit(CommitTimeCellEdit {
        active_field: None,
        field: TimeField::Start,
        draft_value: "00:02.00".to_string(),
        start_seconds: 1.2,
        end_seconds: 3.4,
        from_blur: false,
    });
    assert_eq!(result, CommitTimeCellResult::Ignored);
}

#[test]
fn submitting_time_cell_edit_updates_boundary_without_flushing_text_draft() {
    let mut doc = stable_sample_doc();
    let original_text = text_of(&doc, "annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "text draft still being edited");
    clear_dirty(&mut doc);

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:01.25".to_string()));
    assert!(!is_dirty(&doc));

    ok(doc.commit_time_edit("annotation-1", TimeField::Start, false));

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.25);
    assert_eq!(segment.end_seconds, 3.4);
    assert_eq!(segment.text, original_text);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:01.25");
    assert_eq!(draft.text(), "text draft still being edited");
    assert_eq!(*draft.time_edit(), TimeEditState::default());
    assert!(is_dirty(&doc));
}

#[test]
fn submitting_time_cell_edit_preserves_timeline_viewport() {
    let mut doc = stable_sample_doc();
    doc.session_mut().viewport.start_seconds = 1.0;
    doc.session_mut().viewport.end_seconds = 6.0;
    doc.session_mut().playhead_seconds = 80.0;
    let before_viewport = doc.session().viewport;

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:01.25".to_string()));
    ok(doc.commit_time_edit("annotation-1", TimeField::Start, false));

    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.25
    );
    assert_eq!(doc.session().playhead_seconds, 80.0);
    assert_eq!(doc.session().viewport, before_viewport);
}

#[test]
fn invalid_time_cell_submit_stays_local_and_reports_babel_error() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:03.40".to_string()));
    let err = doc
        .commit_time_edit("annotation-1", TimeField::Start, false)
        .unwrap_err();

    assert!(matches!(
        err,
        EditError::Time(TimeEditError::StartNotBeforeEnd)
    ));
    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.2);
    assert_eq!(segment.end_seconds, 3.4);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:03.40");
    assert_eq!(draft.time_edit().active_field, Some(TimeField::Start));
    assert_eq!(
        draft.time_edit().error,
        Some(TimeEditError::StartNotBeforeEnd)
    );
    assert!(draft.time_error(TimeField::Start).is_some());
    assert!(!is_dirty(&doc));
}

#[test]
fn invalid_time_cell_blur_reverts_silently() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::Start, "not a time".to_string()));
    ok(doc.commit_time_edit("annotation-1", TimeField::Start, true));

    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.time_value(TimeField::Start), "00:01.20");
    assert_eq!(draft.time_edit().active_field, None);
    assert_eq!(draft.time_error(TimeField::Start), None);
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.2
    );
    assert!(!is_dirty(&doc));
}

#[test]
fn time_cell_boundary_edit_is_one_project_undo_step() {
    let mut doc = stable_sample_doc();

    ok(doc.set_time_draft("annotation-1", TimeField::End, "00:03.30".to_string()));
    ok(doc.commit_time_edit("annotation-1", TimeField::End, false));
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().end_seconds,
        3.3
    );

    ok(doc.undo());

    assert_eq!(
        doc.project().segment("annotation-1").unwrap().end_seconds,
        3.4
    );
    assert_eq!(
        doc.draft("annotation-1")
            .unwrap()
            .time_value(TimeField::End),
        "00:03.40"
    );
}

// ---- commit / project undo ----------------------------------------------------------------

#[test]
fn applying_text_only_preserves_precise_timing() {
    let mut doc = doc(vec![seg("a", "lane-1", 1.23456, 3.45678, "Original.")]);
    set_text_with_caret_at_end(&mut doc, "a", "Only text changed.");

    ok(doc.commit_draft("a"));

    let segment = doc.project().segment("a").unwrap();
    assert_eq!(segment.start_seconds, 1.23456);
    assert_eq!(segment.end_seconds, 3.45678);
    assert_eq!(segment.text, "Only text changed.");
    assert!(is_dirty(&doc));
    assert!(!doc.has_pending_drafts());
}

#[test]
fn applying_text_and_timing_undoes_as_one_step() {
    let mut doc = stable_sample_doc();
    ok(doc.set_time_draft("annotation-1", TimeField::Start, "00:01.50".to_string()));
    ok(doc.set_time_draft("annotation-1", TimeField::End, "00:03.75".to_string()));
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Updated together.");

    ok(doc.commit_draft("annotation-1"));
    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.5);
    assert_eq!(segment.end_seconds, 3.75);
    assert_eq!(segment.text, "Updated together.");

    ok(doc.undo());

    let segment = doc.project().segment("annotation-1").unwrap();
    assert_eq!(segment.start_seconds, 1.2);
    assert_eq!(segment.end_seconds, 3.4);
    assert_eq!(segment.text, "Здравствуйте.");
    assert_eq!(draft_text(&doc, "annotation-1"), "Здравствуйте.");
}

#[test]
fn committing_draft_with_invalid_typed_time_is_refused() {
    let mut doc = stable_sample_doc();
    ok(doc.set_time_draft("annotation-1", TimeField::Start, "bogus".to_string()));

    let err = doc.commit_draft("annotation-1").unwrap_err();

    assert!(matches!(err, EditError::Draft { .. }));
    assert_eq!(
        doc.project().segment("annotation-1").unwrap().start_seconds,
        1.2
    );
    assert!(doc.pending_draft_segment_ids().is_err());
    assert!(doc.has_pending_drafts());
}

#[test]
fn global_undo_is_blocked_losslessly_with_pending_transcript_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Committed history step.");
    ok(doc.commit_draft("annotation-1"));
    let committed = text_of(&doc, "annotation-1");

    set_text_with_caret(
        &mut doc,
        "annotation-1",
        "Pending local edit.",
        "Pending".chars().count(),
    );
    set_text_with_caret(
        &mut doc,
        "annotation-1",
        "Pending! local edit.",
        "Pending!".chars().count(),
    );

    let err = doc.undo().unwrap_err();

    assert!(matches!(err, EditError::PendingDrafts { action: "Undo" }));
    assert_eq!(text_of(&doc, "annotation-1"), committed);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.text(), "Pending! local edit.");
    assert_eq!(
        draft.selection(),
        TextSelection::caret("Pending!".chars().count())
    );
    assert!(draft.can_undo());

    ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), "Pending local edit.");
}

#[test]
fn global_redo_is_blocked_losslessly_with_pending_transcript_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Redo history step.");
    ok(doc.commit_draft("annotation-1"));
    ok(doc.undo());
    let undone = text_of(&doc, "annotation-1");
    assert!(doc.can_redo());

    set_text_with_caret_at_end(&mut doc, "annotation-1", "Pending redo-safe draft.");

    let err = doc.redo().unwrap_err();

    assert!(matches!(err, EditError::PendingDrafts { action: "Redo" }));
    assert_eq!(text_of(&doc, "annotation-1"), undone);
    let draft = doc.draft("annotation-1").unwrap();
    assert_eq!(draft.text(), "Pending redo-safe draft.");
    assert_eq!(
        draft.selection(),
        TextSelection::caret("Pending redo-safe draft.".chars().count())
    );
    assert!(doc.can_redo());
}

#[test]
fn apply_pending_drafts_commits_multiple_rows_as_one_undo_step() {
    let mut doc = stable_sample_doc();
    let first = doc.project().segments[0].id.clone();
    let second = doc.project().segments[1].id.clone();
    let original_first = text_of(&doc, &first);
    let original_second = text_of(&doc, &second);
    set_text_with_caret_at_end(&mut doc, &first, "Grouped draft one.");
    set_text_with_caret_at_end(&mut doc, &second, "Grouped draft two.");
    assert_eq!(
        doc.pending_draft_segment_ids().unwrap(),
        vec![first.clone(), second.clone()]
    );

    ok(doc.commit_pending_drafts("Save"));

    assert_eq!(text_of(&doc, &first), "Grouped draft one.");
    assert_eq!(text_of(&doc, &second), "Grouped draft two.");
    assert!(!doc.has_pending_drafts());

    ok(doc.undo());

    assert_eq!(text_of(&doc, &first), original_first);
    assert_eq!(text_of(&doc, &second), original_second);
}

#[test]
fn commit_pending_drafts_refuses_invalid_drafts() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "Valid text.");
    ok(doc.set_time_draft("annotation-2", TimeField::End, "nope".to_string()));

    let err = doc.commit_pending_drafts("Save").unwrap_err();

    assert!(matches!(err, EditError::Draft { action: "Save", .. }));
    assert_eq!(text_of(&doc, "annotation-1"), "Здравствуйте.");
}
