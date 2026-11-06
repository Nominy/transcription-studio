#![allow(unused_must_use)]
use exaudio_extension::document::{Completion, DocumentEnvironment};
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_editor::edit::{CompletionKey, EditError, EditorDoc, Effect, Outcome, TextSelection};

fn project(segments: Vec<Segment>) -> Project {
    Project {
        id: "project".into(),
        name: "Project".into(),
        speakers: (1..=2)
            .map(|i| Speaker {
                id: format!("speaker-{i}"),
                label: format!("Speaker {i}"),
            })
            .collect(),
        lanes: (1..=2)
            .map(|i| AudioLane {
                id: format!("lane-{i}"),
                speaker_id: format!("speaker-{i}"),
                label: format!("Speaker {i}"),
                source_path: format!("speaker-{i}.wav"),
            })
            .collect(),
        segments,
        metadata: Default::default(),
    }
}
fn seg(id: &str, lane: &str, start: f64, end: f64, text: &str) -> Segment {
    Segment {
        id: id.into(),
        source_annotation_id: None,
        speaker_id: lane.replace("lane", "speaker"),
        lane_id: lane.into(),
        start_seconds: start,
        end_seconds: end,
        text: text.into(),
        metadata: Default::default(),
    }
}
fn registered_doc(project: Project) -> EditorDoc {
    let mut environment = DocumentEnvironment::local(project.id.clone());
    exaudio_babel_tags::register(&mut environment.providers);
    EditorDoc::from_project(project, environment)
}
fn doc(segments: Vec<Segment>) -> EditorDoc {
    registered_doc(project(segments))
}
fn stable_sample_doc() -> EditorDoc {
    doc(vec![
        seg("annotation-1", "lane-1", 1.2, 3.4, "Здравствуйте."),
        seg("annotation-2", "lane-2", 3.5, 4.5, "Да."),
        seg("annotation-3", "lane-1", 4.6, 6.0, "Продолжим."),
    ])
}
fn ok(result: Result<Outcome, EditError>) -> Outcome {
    result.expect("edit succeeds")
}
fn set_text_with_caret(doc: &mut EditorDoc, id: &str, text: &str, caret: usize) {
    ok(doc.edit_text(id, text.into(), TextSelection::caret(caret)));
}
fn set_text_with_caret_at_end(doc: &mut EditorDoc, id: &str, text: &str) {
    set_text_with_caret(doc, id, text, text.chars().count());
}
fn set_text_selected_all(doc: &mut EditorDoc, id: &str, text: &str) {
    ok(doc.edit_text(id, text.into(), TextSelection::new(0, text.chars().count())));
}
fn is_dirty(doc: &EditorDoc) -> bool {
    doc.session().is_dirty
}
fn clear_dirty(doc: &mut EditorDoc) {
    doc.session_mut().is_dirty = false;
}
fn caret(doc: &EditorDoc, id: &str) -> usize {
    doc.draft(id).unwrap().selection().head
}
fn draft_text(doc: &EditorDoc, id: &str) -> String {
    doc.draft(id).unwrap().text().to_string()
}
fn has_draft_changed(effects: &[Effect], id: &str) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::DraftChanged { segment_id } if segment_id == id))
}
fn doc_suggestion(doc: &EditorDoc, id: &str, label: &str) -> Completion {
    doc.completions(id)
        .expect("completions open")
        .items
        .into_iter()
        .find(|item| item.label == label)
        .expect("completion offered")
}
fn labels(items: &[Completion]) -> Vec<&str> {
    items.iter().map(|item| item.label.as_str()).collect()
}
#[test]
fn doc_tag_suggestions_follow_project_language() {
    let mut project = project(vec![seg("a", "lane-1", 0.0, 2.0, "")]);
    project
        .metadata
        .insert("language".to_string(), "EN".to_string());
    let mut doc = registered_doc(project);

    set_text_with_caret_at_end(&mut doc, "a", "[lau");
    let list = doc.completions("a").expect("bracket suggestions");
    assert_eq!(labels(&list.items), vec!["laugh"]);

    set_text_with_caret_at_end(&mut doc, "a", "[сме");
    assert!(doc.completions("a").is_none());
}

#[test]
fn applying_tag_suggestion_replaces_partial_trigger_and_marks_dirty() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "слово [не");
    clear_dirty(&mut doc);
    let suggestion = doc_suggestion(&doc, "annotation-1", "неразборчиво");

    let outcome = ok(doc.apply_completion("annotation-1", &suggestion));

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

    ok(doc.apply_completion("annotation-1", &suggestion));

    assert_eq!(draft_text(&doc, "annotation-1"), "слово {MIS: }");
    assert_eq!(caret(&doc, "annotation-1"), "слово {MIS: ".chars().count());
    assert!(is_dirty(&doc));
}

#[test]
fn applying_tag_suggestion_stays_in_local_text_undo_history() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "слово [не");
    let suggestion = doc_suggestion(&doc, "annotation-1", "неразборчиво");

    ok(doc.apply_completion("annotation-1", &suggestion));
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

    ok(doc.apply_completion("annotation-1", &suggestion));

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

    ok(doc.apply_completion("annotation-1", &suggestion));

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
    assert_eq!(doc.completions("annotation-1").unwrap().highlighted, 0);

    ok(doc.completion_key("annotation-1", CompletionKey::Next));
    assert_eq!(doc.completions("annotation-1").unwrap().highlighted, 1);

    ok(doc.completion_key("annotation-1", CompletionKey::Accept));

    assert_eq!(draft_text(&doc, "annotation-1"), "[смех]");
    assert!(is_dirty(&doc));
    assert!(doc.completions("annotation-1").is_none());
}

#[test]
fn tag_suggestion_escape_dismisses_current_trigger_until_text_changes() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "[сме");
    assert!(doc.completions("annotation-1").is_some());

    ok(doc.completion_key("annotation-1", CompletionKey::Dismiss));
    assert!(doc.completions("annotation-1").is_none());

    set_text_with_caret_at_end(&mut doc, "annotation-1", "[смех");
    assert!(doc.completions("annotation-1").is_some());
}
#[test]
fn helper_selected_number_to_skaz_converts_selected_digits() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_selected_all(&mut doc, "annotation-1", "1234");
    clear_dirty(&mut doc);

    let outcome = ok(doc.execute_command("babel.skaz-selection", &serde_json::Value::Null));

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

    ok(doc.execute_command("babel.skaz-selection", &serde_json::Value::Null));

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

    ok(doc.execute_command_for_segment(
        "babel.skaz-digit",
        &serde_json::json!({"digit": "5"}),
        Some("annotation-1"),
    ));

    assert_eq!(draft_text(&doc, "annotation-1"), "5 {СКАЗ: семь восемь}");
    assert_eq!(caret(&doc, "annotation-1"), 1);
    assert!(is_dirty(&doc));
}
#[test]
fn helper_lint_autofix_current_row_applies_common_babel_text_hygiene() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "  привет,мир  ");
    clear_dirty(&mut doc);

    let outcome = ok(doc.execute_command("babel.autofix-row", &serde_json::Value::Null));

    assert_eq!(draft_text(&doc, "annotation-1"), "Привет, мир.");
    assert!(has_draft_changed(&outcome.effects, "annotation-1"));
    assert!(is_dirty(&doc));
}

#[test]
fn helper_lint_autofix_current_row_is_locally_undoable() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "  привет,мир  ");

    ok(doc.execute_command("babel.autofix-row", &serde_json::Value::Null));
    ok(doc.undo_text("annotation-1"));

    assert_eq!(draft_text(&doc, "annotation-1"), "  привет,мир  ");
}

#[test]
fn helper_lint_autofix_all_rows_uses_same_lane_previous_context() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "первая  строка");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "следующая,строка");
    clear_dirty(&mut doc);

    ok(doc.execute_command("babel.autofix-all", &serde_json::Value::Null));

    assert_eq!(draft_text(&doc, "annotation-1"), "Первая строка.");
    assert_eq!(draft_text(&doc, "annotation-3"), "Следующая, строка.");
    assert!(is_dirty(&doc));
}

#[test]
fn helper_lint_autofix_preserves_lowercase_after_same_lane_continuation_marker() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "это --");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "продолжение");

    ok(doc.execute_command("babel.autofix-all", &serde_json::Value::Null));

    assert_eq!(draft_text(&doc, "annotation-1"), "Это --");
    assert_eq!(draft_text(&doc, "annotation-3"), "продолжение.");
}

#[test]
fn helper_lint_autofix_moves_tag_trailing_punctuation_like_babel_linter() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "привет {MIS: шум}?");

    ok(doc.execute_command("babel.autofix-row", &serde_json::Value::Null));

    assert_eq!(draft_text(&doc, "annotation-1"), "Привет? {MIS: шум}.");
}

#[test]
fn helper_lint_autofix_moves_square_tag_dash_and_normalizes_free_double_dash() {
    let mut doc = stable_sample_doc();
    doc.select_segment("annotation-1");
    set_text_with_caret_at_end(&mut doc, "annotation-1", "ну, - вот [смех]- дальше -- тут");

    ok(doc.execute_command("babel.autofix-row", &serde_json::Value::Null));

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

    ok(doc.execute_command("babel.autofix-row", &serde_json::Value::Null));

    assert_eq!(draft_text(&doc, "annotation-1"), "Э вау чёрт.");
}

#[test]
fn helper_lint_autofix_lowercases_polite_pronouns_only_inside_sentence() {
    let mut doc = stable_sample_doc();
    set_text_with_caret_at_end(&mut doc, "annotation-1", "спасибо Вам за Ваш ответ");
    set_text_with_caret_at_end(&mut doc, "annotation-2", "Вам спасибо");
    set_text_with_caret_at_end(&mut doc, "annotation-3", "вопрос: Вам удобно");

    ok(doc.execute_command("babel.autofix-all", &serde_json::Value::Null));

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

    let err = doc
        .execute_command("babel.autofix-row", &serde_json::Value::Null)
        .unwrap_err();

    assert!(matches!(err, EditError::Unavailable(_)));
    assert_eq!(draft_text(&doc, "a"), "привет,мир");
}

#[test]
fn batch_autofix_keeps_each_row_in_local_undo_and_preserves_selection() {
    let mut doc = stable_sample_doc();
    ok(doc.edit_text(
        "annotation-1",
        "  привет,мир  ".into(),
        TextSelection::new(8, 3),
    ));
    ok(doc.edit_text(
        "annotation-3",
        "  пока,мир  ".into(),
        TextSelection::caret(2),
    ));
    let first_selection = doc.draft("annotation-1").unwrap().selection();
    let third_selection = doc.draft("annotation-3").unwrap().selection();

    ok(doc.execute_command("babel.autofix-all", &serde_json::Value::Null));

    assert_eq!(
        doc.project().segment("annotation-1").unwrap().text,
        "Здравствуйте."
    );
    assert_eq!(
        doc.draft("annotation-1").unwrap().selection(),
        first_selection
    );
    assert_eq!(
        doc.draft("annotation-3").unwrap().selection(),
        third_selection
    );
    ok(doc.undo_text("annotation-1"));
    assert_eq!(draft_text(&doc, "annotation-1"), "  привет,мир  ");
    assert_eq!(draft_text(&doc, "annotation-3"), "Пока, мир.");
    ok(doc.undo_text("annotation-3"));
    assert_eq!(draft_text(&doc, "annotation-3"), "  пока,мир  ");
}
