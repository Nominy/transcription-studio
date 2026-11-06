//! Keymap defaults, focus contexts, keystroke routing, persistence, conflicts and
//! `EditorDoc::perform` dispatch (ported from the iced app's keybinds.rs / hotkeys.rs tests).
#![allow(unused_must_use)]

mod edit_support;

use edit_support::*;
use studio_editor::edit::keymap::{
    ACTION_DEFS, EditorAction, FocusTarget, HOTKEY_STORAGE_KEY, KeyChord, KeyContext, KeyFocus,
    Keymap, KeymapError, audition_key_for,
};
use studio_editor::edit::structure::AuditionKey;
use studio_editor::edit::{EditError, Effect, PointerModifiers};
use studio_editor::track_filter::TrackFilter;

const WORKSPACE: KeyFocus = KeyFocus {
    target: FocusTarget::Workspace,
    cut_preview_active: false,
    completions_open: false,
};
const TRANSCRIPT_TEXT: KeyFocus = KeyFocus {
    target: FocusTarget::TranscriptText,
    cut_preview_active: false,
    completions_open: false,
};
const INPUT: KeyFocus = KeyFocus {
    target: FocusTarget::Input,
    cut_preview_active: false,
    completions_open: false,
};
const CUT_PREVIEW: KeyFocus = KeyFocus {
    target: FocusTarget::Workspace,
    cut_preview_active: true,
    completions_open: false,
};
const TAG_LIST_OPEN: KeyFocus = KeyFocus {
    target: FocusTarget::TranscriptText,
    cut_preview_active: false,
    completions_open: true,
};

fn resolve(keystroke: &str, focus: KeyFocus) -> Option<EditorAction> {
    Keymap::default().resolve(keystroke, focus, false)
}

fn three_row_doc() -> studio_editor::edit::EditorDoc {
    doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
        seg("c", "lane-2", 6.0, 20.0, "three"),
    ])
}

// ---- action registry ------------------------------------------------------------------

#[test]
fn every_action_has_exactly_one_definition_with_section_and_description() {
    assert_eq!(ACTION_DEFS.len(), EditorAction::ALL.len());
    for action in EditorAction::ALL.iter().copied() {
        let count = ACTION_DEFS
            .iter()
            .filter(|def| def.action == action)
            .count();
        assert_eq!(count, 1, "{action:?} must have exactly one ActionDef");
        let def = action.def();
        assert_eq!(def.action, action);
        assert!(
            !def.section.en.trim().is_empty() && !def.section.ru.trim().is_empty(),
            "{action:?} section"
        );
        assert!(
            !def.description.en.trim().is_empty() && !def.description.ru.trim().is_empty(),
            "{action:?} description"
        );
    }
}

#[test]
fn every_action_has_parsable_default_keystrokes() {
    for def in ACTION_DEFS {
        assert!(
            !def.defaults.is_empty(),
            "{:?} must have a default binding",
            def.action
        );
        for keystroke in def.defaults {
            assert!(
                KeyChord::parse(keystroke).is_some(),
                "{:?} default `{keystroke}` must parse",
                def.action
            );
        }
        assert_eq!(
            Keymap::default().keystrokes(def.action).len(),
            def.defaults.len()
        );
    }
}

#[test]
fn only_fixed_transcript_text_keys_are_not_configurable() {
    let fixed: Vec<EditorAction> = ACTION_DEFS
        .iter()
        .filter(|def| !def.configurable)
        .map(|def| def.action)
        .collect();
    assert_eq!(
        fixed,
        vec![
            EditorAction::TextUndo,
            EditorAction::TextRedo,
            EditorAction::TranscriptEscape,
            EditorAction::CompletionPrevious,
            EditorAction::CompletionNext,
            EditorAction::CompletionAccept,
            EditorAction::CompletionDismiss,
        ]
    );
    for action in fixed {
        assert_eq!(action.def().context, KeyContext::TranscriptText);
    }
}

#[test]
fn action_ids_round_trip_and_match_persisted_camel_case_ids() {
    assert_eq!(EditorAction::PlayPause.id(), "playPause");
    assert_eq!(
        EditorAction::DeleteSelectedSegment.id(),
        "deleteSelectedSegment"
    );
    assert_eq!(
        EditorAction::from_id("deleteSelectedSegment"),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(EditorAction::from_id("notAnAction"), None);
    for action in EditorAction::ALL.iter().copied() {
        assert_eq!(EditorAction::from_id(&action.id()), Some(action));
    }
}

#[test]
fn only_tag_suggestion_actions_require_open_suggestion_list() {
    for action in EditorAction::ALL.iter().copied() {
        let expected = matches!(
            action,
            EditorAction::CompletionPrevious
                | EditorAction::CompletionNext
                | EditorAction::CompletionAccept
                | EditorAction::CompletionDismiss
        );
        assert_eq!(action.requires_completions(), expected, "{action:?}");
    }
}

#[test]
fn default_delete_selected_segment_binds_unmodified_d_only() {
    assert_eq!(
        Keymap::default().keystrokes(EditorAction::DeleteSelectedSegment),
        vec!["d".to_string()]
    );
}

#[test]
fn default_redo_binds_secondary_shift_z_and_secondary_y() {
    assert_eq!(
        Keymap::default().keystrokes(EditorAction::Redo),
        vec!["secondary-shift-z".to_string(), "secondary-y".to_string()]
    );
}

#[test]
fn hotkey_storage_key_is_stable() {
    assert_eq!(HOTKEY_STORAGE_KEY, "studio.native.hotkeyBindings.v1");
}

// ---- key chords -------------------------------------------------------------------------

#[test]
fn key_chord_parse_normalizes_modifier_order_and_uppercase_shift() {
    let chord = KeyChord::parse("Shift-Alt-r").expect("chord");
    assert_eq!(chord.to_string(), "alt-shift-r");
    assert_eq!(KeyChord::parse("alt-R"), KeyChord::parse("alt-shift-r"));
    assert_eq!(
        KeyChord::parse("shift-secondary-z")
            .expect("chord")
            .to_string(),
        "secondary-shift-z"
    );
}

#[test]
fn key_chord_parse_rejects_empty_and_unknown_modifiers() {
    assert_eq!(KeyChord::parse(""), None);
    assert_eq!(KeyChord::parse("   "), None);
    assert_eq!(KeyChord::parse("hyper-k"), None);
}

#[test]
fn key_chord_parse_accepts_minus_key() {
    assert_eq!(KeyChord::parse("-").expect("chord").key, "-");
    let chord = KeyChord::parse("secondary--").expect("chord");
    assert!(chord.secondary);
    assert_eq!(chord.key, "-");
}

#[test]
fn key_chord_secondary_resolves_to_ctrl_or_cmd() {
    let chord = KeyChord::parse("secondary-z").expect("chord");
    assert_eq!(Some(chord.resolved(false)), KeyChord::parse("ctrl-z"));
    assert_eq!(Some(chord.resolved(true)), KeyChord::parse("cmd-z"));
}

#[test]
fn key_chord_labels_use_platform_modifier_names_and_arrows() {
    let redo = KeyChord::parse("secondary-shift-z").expect("chord");
    assert_eq!(redo.label(false), "Ctrl+Shift+Z");
    assert_eq!(redo.label(true), "Cmd+Shift+Z");
    assert_eq!(
        KeyChord::parse("alt-left").expect("chord").label(false),
        "Alt+←"
    );
}

#[test]
fn key_context_overlaps_follow_focus_exclusivity() {
    use KeyContext::*;
    for context in [Global, TranscriptText, Timeline, CutPreview, Dialog] {
        assert!(Global.overlaps(context));
        assert!(context.overlaps(Global));
        assert!(context.overlaps(context));
    }
    assert!(!CutPreview.overlaps(Timeline));
    assert!(!Timeline.overlaps(CutPreview));
    assert!(!TranscriptText.overlaps(Timeline));
    assert!(!Dialog.overlaps(TranscriptText));
    assert!(!Dialog.overlaps(Timeline));
}

// ---- persistence ------------------------------------------------------------------------

#[test]
fn persisted_override_round_trips_through_json() {
    let mut keymap = Keymap::default();
    keymap
        .set_override(EditorAction::DeleteSelectedSegment, "alt-k")
        .expect("configurable");

    let source = keymap.to_json();
    let value: serde_json::Value = serde_json::from_str(&source).expect("valid json");
    assert_eq!(
        value,
        serde_json::json!({ "deleteSelectedSegment": "alt-k" })
    );

    let recovered = Keymap::from_json(&source);
    assert_eq!(recovered, keymap);
    assert!(recovered.is_overridden(EditorAction::DeleteSelectedSegment));
    assert_eq!(
        recovered.resolve("alt-k", WORKSPACE, false),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(recovered.resolve("d", WORKSPACE, false), None);
}

#[test]
fn default_keymap_serializes_to_empty_object() {
    let value: serde_json::Value =
        serde_json::from_str(&Keymap::default().to_json()).expect("valid json");
    assert_eq!(value, serde_json::json!({}));
}

#[test]
fn legacy_iced_json_format_is_accepted() {
    let keymap = Keymap::from_json(
        r#"{
            "deleteSelectedSegment": {"code": "KeyK", "modifiers": ["alt"]},
            "increasePlaybackSpeed": {"code": "Digit3", "modifiers": ["shift"]},
            "openFind": {"code": "F4", "modifiers": []},
            "previousSegment": {"code": "ArrowUp", "modifiers": ["control", "shift"]},
            "resetSpeakerWorkflow": {"code": "Backquote", "modifiers": ["logo"]}
        }"#,
    );

    assert_eq!(
        keymap.keystrokes(EditorAction::DeleteSelectedSegment),
        vec!["alt-k".to_string()]
    );
    assert_eq!(
        keymap.keystrokes(EditorAction::IncreasePlaybackSpeed),
        vec!["shift-3".to_string()]
    );
    assert_eq!(
        keymap.keystrokes(EditorAction::OpenFind),
        vec!["f4".to_string()]
    );
    assert_eq!(
        keymap.keystrokes(EditorAction::PreviousSegment),
        vec!["ctrl-shift-up".to_string()]
    );
    assert_eq!(
        keymap.keystrokes(EditorAction::ResetSpeakerWorkflow),
        vec!["cmd-`".to_string()]
    );
    assert_eq!(
        keymap.resolve("alt-k", WORKSPACE, false),
        Some(EditorAction::DeleteSelectedSegment)
    );
}

#[test]
fn legacy_and_new_formats_can_be_mixed_and_resaved_in_new_format() {
    let keymap = Keymap::from_json(
        r#"{"deleteSelectedSegment": {"code": "KeyK", "modifiers": ["alt"]}, "toggleOverlay": "alt-l"}"#,
    );
    let value: serde_json::Value = serde_json::from_str(&keymap.to_json()).expect("valid json");
    assert_eq!(
        value,
        serde_json::json!({ "deleteSelectedSegment": "alt-k", "toggleOverlay": "alt-l" })
    );
}

#[test]
fn from_json_ignores_unknown_invalid_and_fixed_entries() {
    let keymap = Keymap::from_json(
        r#"{
            "textUndo": "alt-u",
            "completionAccept": "alt-y",
            "notAnAction": "alt-v",
            "deleteSelectedSegment": 42,
            "openFind": "hyper-q",
            "toggleOverlay": {"code": "Unidentified", "modifiers": []},
            "playPause": {"code": "KeyP", "modifiers": ["hyper"]}
        }"#,
    );
    assert_eq!(keymap, Keymap::default());
}

#[test]
fn from_json_with_malformed_source_falls_back_to_defaults() {
    assert_eq!(Keymap::from_json("not json"), Keymap::default());
    assert_eq!(Keymap::from_json("[]"), Keymap::default());
    assert_eq!(Keymap::from_json(""), Keymap::default());
}

#[test]
fn non_configurable_override_is_rejected() {
    let mut keymap = Keymap::default();
    assert_eq!(
        keymap.set_override(EditorAction::TextUndo, "alt-u"),
        Err(KeymapError::NotConfigurable)
    );
    assert_eq!(
        keymap.set_override(EditorAction::CompletionDismiss, "alt-u"),
        Err(KeymapError::NotConfigurable)
    );
    assert!(!keymap.is_overridden(EditorAction::TextUndo));
    assert_eq!(keymap, Keymap::default());
}

#[test]
fn invalid_override_keystroke_is_rejected() {
    let mut keymap = Keymap::default();
    assert!(matches!(
        keymap.set_override(EditorAction::ToggleOverlay, "hyper-q"),
        Err(KeymapError::InvalidKeystroke(_))
    ));
    assert!(matches!(
        keymap.set_override(EditorAction::ToggleOverlay, ""),
        Err(KeymapError::InvalidKeystroke(_))
    ));
    assert_eq!(keymap, Keymap::default());
}

#[test]
fn reset_override_restores_default_bindings() {
    let mut keymap = Keymap::default();
    keymap
        .set_override(EditorAction::Redo, "alt-y")
        .expect("configurable");
    assert_eq!(
        keymap.keystrokes(EditorAction::Redo),
        vec!["alt-y".to_string()]
    );

    keymap.reset_override(EditorAction::Redo);

    assert!(!keymap.is_overridden(EditorAction::Redo));
    assert_eq!(keymap, Keymap::default());
    assert_eq!(
        keymap.keystrokes(EditorAction::Redo),
        vec!["secondary-shift-z".to_string(), "secondary-y".to_string()]
    );
}

#[test]
fn configurable_shortcuts_replace_action_defaults() {
    let mut keymap = Keymap::default();
    keymap
        .set_override(EditorAction::DeleteSelectedSegment, "alt-k")
        .expect("configurable");

    assert_eq!(
        keymap.resolve("alt-k", WORKSPACE, false),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(keymap.resolve("alt-b", WORKSPACE, false), None);
    assert_eq!(
        keymap.resolve("d", WORKSPACE, false),
        None,
        "overriding an action disables its unmodified D default"
    );
    assert_eq!(
        keymap.resolve("ctrl-backspace", WORKSPACE, false),
        None,
        "overriding must not introduce Ctrl+Backspace as a hidden fallback"
    );
}

#[test]
fn secondary_override_resolves_per_platform() {
    let mut keymap = Keymap::default();
    keymap
        .set_override(EditorAction::DeleteSelectedSegment, "secondary-k")
        .expect("configurable");
    assert_eq!(
        keymap.resolve("ctrl-k", WORKSPACE, false),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(
        keymap.resolve("cmd-k", WORKSPACE, true),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(keymap.resolve("ctrl-k", WORKSPACE, true), None);
}

// ---- conflicts --------------------------------------------------------------------------

#[test]
fn default_keymap_has_no_conflicts() {
    assert_eq!(Keymap::default().conflicts(false), Vec::new());
    assert_eq!(Keymap::default().conflicts(true), Vec::new());
}

#[test]
fn conflicts_with_detects_clash_in_overlapping_context() {
    let keymap = Keymap::default();
    assert_eq!(
        keymap.conflicts_with(EditorAction::ToggleOverlay, "d", false),
        vec![EditorAction::DeleteSelectedSegment]
    );
    assert!(
        keymap
            .conflicts_with(EditorAction::ToggleOverlay, "l", false)
            .is_empty(),
        "cut-preview `l` lives in a context exclusive with the timeline"
    );
    assert_eq!(
        keymap.conflicts_with(EditorAction::DeleteSelectedSegment, "space", false),
        vec![EditorAction::PlayPause]
    );
}

#[test]
fn conflicts_with_global_action_reports_every_context() {
    let keymap = Keymap::default();
    assert_eq!(
        keymap.conflicts_with(EditorAction::OpenFind, "l", false),
        vec![EditorAction::LoopCutPreviewOverlay]
    );
    assert_eq!(
        keymap.conflicts_with(EditorAction::OpenFind, "t", false),
        vec![EditorAction::ToggleOverlay]
    );
}

#[test]
fn conflicts_with_ignores_the_action_itself_and_exclusive_contexts() {
    let keymap = Keymap::default();
    assert!(
        keymap
            .conflicts_with(EditorAction::LoopCutPreviewOverlay, "l", false)
            .is_empty()
    );
    assert!(
        keymap
            .conflicts_with(EditorAction::Undo, "secondary-z", false)
            .is_empty(),
        "row-local TextUndo shares the key but never fires outside the row editor"
    );
    assert!(
        keymap
            .conflicts_with(EditorAction::ToggleOverlay, "hyper-q", false)
            .is_empty()
    );
}

#[test]
fn conflicts_with_resolves_secondary_per_platform() {
    let keymap = Keymap::default();
    assert_eq!(
        keymap.conflicts_with(EditorAction::DeleteSelectedSegment, "ctrl-z", false),
        vec![EditorAction::Undo]
    );
    assert!(
        keymap
            .conflicts_with(EditorAction::DeleteSelectedSegment, "ctrl-z", true)
            .is_empty()
    );
    assert_eq!(
        keymap.conflicts_with(EditorAction::DeleteSelectedSegment, "cmd-z", true),
        vec![EditorAction::Undo]
    );
}

#[test]
fn override_creating_a_clash_is_reported_by_conflicts() {
    let mut keymap = Keymap::default();
    keymap
        .set_override(EditorAction::ToggleOverlay, "d")
        .expect("configurable");

    let conflicts = keymap.conflicts(false);
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].first, EditorAction::DeleteSelectedSegment);
    assert_eq!(conflicts[0].second, EditorAction::ToggleOverlay);
    assert_eq!(Some(conflicts[0].chord.clone()), KeyChord::parse("d"));
}

// ---- routing: transport / navigation ----------------------------------------------------

#[test]
fn hotkey_mapping_covers_transport_navigation_and_workflow_actions() {
    assert_eq!(resolve("space", WORKSPACE), Some(EditorAction::PlayPause));
    assert_eq!(resolve("right", WORKSPACE), Some(EditorAction::NextSegment));
    assert_eq!(
        resolve("left", WORKSPACE),
        Some(EditorAction::PreviousSegment)
    );
    assert_eq!(resolve("secondary-s", WORKSPACE), None);
    assert_eq!(resolve("secondary-e", WORKSPACE), None);
    assert_eq!(resolve("secondary-z", WORKSPACE), Some(EditorAction::Undo));
    assert_eq!(
        resolve("secondary-shift-z", WORKSPACE),
        Some(EditorAction::Redo)
    );
    assert_eq!(resolve("secondary-y", WORKSPACE), Some(EditorAction::Redo));
    assert_eq!(resolve("l", WORKSPACE), None);
    assert_eq!(resolve("t", WORKSPACE), Some(EditorAction::ToggleOverlay));
    assert_eq!(resolve("secondary-j", WORKSPACE), None);
    assert_eq!(
        resolve("escape", WORKSPACE),
        Some(EditorAction::EscapePressed)
    );
    assert_eq!(
        resolve("enter", WORKSPACE),
        Some(EditorAction::ActivateDefaultSurfaceAction)
    );
}

#[test]
fn secondary_bindings_resolve_to_ctrl_off_mac_and_cmd_on_mac() {
    let keymap = Keymap::default();
    assert_eq!(
        keymap.resolve("ctrl-z", WORKSPACE, false),
        Some(EditorAction::Undo)
    );
    assert_eq!(
        keymap.resolve("cmd-z", WORKSPACE, true),
        Some(EditorAction::Undo)
    );
    assert_eq!(keymap.resolve("ctrl-z", WORKSPACE, true), None);
    assert_eq!(keymap.resolve("cmd-z", WORKSPACE, false), None);
    assert_eq!(
        keymap.resolve("secondary-z", WORKSPACE, true),
        Some(EditorAction::Undo)
    );
}

#[test]
fn unparsable_keystroke_resolves_to_nothing() {
    assert_eq!(resolve("", WORKSPACE), None);
    assert_eq!(resolve("hyper-z", WORKSPACE), None);
}

#[test]
fn hotkey_mapping_preserves_text_input_editing() {
    for focus in [INPUT, TRANSCRIPT_TEXT] {
        for keystroke in ["space", "right", "left", "q", "d", "shift-1", "enter"] {
            assert_eq!(
                resolve(keystroke, focus),
                None,
                "`{keystroke}` must stay with the focused input ({:?})",
                focus.target
            );
        }
    }
    assert_eq!(resolve("secondary-z", INPUT), None);
    assert_eq!(resolve("secondary-shift-z", INPUT), None);
}

#[test]
fn secondary_z_is_row_local_undo_inside_transcript_text_and_project_undo_outside() {
    assert_eq!(
        resolve("secondary-z", TRANSCRIPT_TEXT),
        Some(EditorAction::TextUndo)
    );
    assert_eq!(
        resolve("secondary-shift-z", TRANSCRIPT_TEXT),
        Some(EditorAction::TextRedo)
    );
    assert_eq!(
        resolve("secondary-y", TRANSCRIPT_TEXT),
        Some(EditorAction::TextRedo)
    );
    assert_eq!(resolve("secondary-z", WORKSPACE), Some(EditorAction::Undo));
}

#[test]
fn hotkey_mapping_routes_plain_tab_to_ghost_cursor_lane_switch() {
    assert_eq!(
        resolve("tab", WORKSPACE),
        Some(EditorAction::SwitchGhostCursorLane)
    );
    assert_eq!(
        resolve("tab", TRANSCRIPT_TEXT),
        Some(EditorAction::SwitchGhostCursorLane),
        "Tab also moves the writing cursor while a row is being edited"
    );
    assert_eq!(resolve("shift-tab", WORKSPACE), None);
    assert_eq!(resolve("secondary-tab", WORKSPACE), None);
    assert_eq!(resolve("tab", INPUT), None);
}

#[test]
fn hotkey_mapping_routes_back_to_projects_outside_text_fields() {
    assert_eq!(
        resolve("alt-left", WORKSPACE),
        Some(EditorAction::BackToProjects)
    );
    assert_eq!(resolve("alt-left", INPUT), None);
    assert_eq!(resolve("alt-left", TRANSCRIPT_TEXT), None);
}

#[test]
fn hotkey_mapping_opens_find_even_from_text_input() {
    for focus in [WORKSPACE, TRANSCRIPT_TEXT, INPUT, CUT_PREVIEW] {
        assert_eq!(resolve("secondary-f", focus), Some(EditorAction::OpenFind));
    }
}

#[test]
fn hotkey_mapping_routes_find_navigation_shortcuts_everywhere() {
    for focus in [WORKSPACE, TRANSCRIPT_TEXT, INPUT] {
        assert_eq!(resolve("f3", focus), Some(EditorAction::FindNext));
        assert_eq!(resolve("secondary-g", focus), Some(EditorAction::FindNext));
        assert_eq!(resolve("shift-f3", focus), Some(EditorAction::FindPrevious));
        assert_eq!(
            resolve("secondary-shift-g", focus),
            Some(EditorAction::FindPrevious)
        );
    }
}

#[test]
fn local_keymap_has_no_implicit_warning_assertion_command() {
    for focus in [WORKSPACE, TRANSCRIPT_TEXT, INPUT] {
        assert_eq!(resolve("secondary-j", focus), None);
    }
}

// ---- routing: helper hotkeys ------------------------------------------------------------

#[test]
fn local_keymap_has_no_implicit_tag_or_lint_commands() {
    for focus in [TRANSCRIPT_TEXT, INPUT, WORKSPACE] {
        assert_eq!(resolve("alt-a", focus), None);
        assert_eq!(resolve("alt-f", focus), None);
        assert_eq!(resolve("alt-shift-f", focus), None);
    }
}

#[test]
fn cross_row_text_move_hotkeys_route_only_inside_transcript_text() {
    assert_eq!(
        resolve("alt-[", TRANSCRIPT_TEXT),
        Some(EditorAction::MoveTextBeforeCaretToPreviousSegment)
    );
    assert_eq!(
        resolve("alt-]", TRANSCRIPT_TEXT),
        Some(EditorAction::MoveTextAfterCaretToNextSegment)
    );
    for focus in [WORKSPACE, INPUT] {
        assert_eq!(resolve("alt-[", focus), None);
        assert_eq!(resolve("alt-]", focus), None);
    }
}

#[test]
fn helper_row_action_hotkeys_route_only_when_not_typing() {
    assert_eq!(
        resolve("d", WORKSPACE),
        Some(EditorAction::DeleteSelectedSegment)
    );
    assert_eq!(resolve("d", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("d", INPUT), None);
    for focus in [WORKSPACE, TRANSCRIPT_TEXT, INPUT] {
        assert_eq!(
            resolve("ctrl-backspace", focus),
            None,
            "Ctrl+Backspace must not be a delete-segment shortcut"
        );
    }
    assert_eq!(
        resolve("alt-shift-up", WORKSPACE),
        Some(EditorAction::MergeSelectedSegmentAbove)
    );
    assert_eq!(
        resolve("alt-shift-down", WORKSPACE),
        Some(EditorAction::MergeSelectedSegmentBelow)
    );
    assert_eq!(resolve("alt-shift-up", TRANSCRIPT_TEXT), None);
}

#[test]
fn shift_arrow_segment_focus_navigation_routes_only_outside_inputs() {
    assert_eq!(
        resolve("shift-right", WORKSPACE),
        Some(EditorAction::FocusNextSegment)
    );
    assert_eq!(
        resolve("shift-left", WORKSPACE),
        Some(EditorAction::FocusPreviousSegment)
    );
    assert_eq!(
        resolve("shift-right", TRANSCRIPT_TEXT),
        None,
        "Shift+Arrow must not steal text selection inside inputs"
    );
    assert_eq!(resolve("shift-left", INPUT), None);
}

#[test]
fn helper_timeline_trim_hotkeys_route_outside_inputs() {
    assert_eq!(
        resolve("alt-r", WORKSPACE),
        Some(EditorAction::TrimCurrentSegmentToAudio)
    );
    assert_eq!(
        resolve("alt-shift-r", WORKSPACE),
        Some(EditorAction::TrimAllVisibleSegmentsToAudio)
    );
    assert_eq!(
        resolve("alt-R", WORKSPACE),
        Some(EditorAction::TrimAllVisibleSegmentsToAudio)
    );
    assert_eq!(resolve("alt-r", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("alt-r", INPUT), None);
}

#[test]
fn helper_transcription_hotkey_routes_from_rows_and_the_workspace() {
    for focus in [WORKSPACE, TRANSCRIPT_TEXT] {
        assert_eq!(resolve("alt-shift-g", focus), None);
    }
}

#[test]
fn helper_timeline_auto_insert_hotkey_routes_outside_inputs() {
    assert_eq!(
        resolve("alt-c", WORKSPACE),
        Some(EditorAction::AutoInsertSegmentAtCaret)
    );
    assert_eq!(resolve("alt-c", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("alt-c", INPUT), None);
}

#[test]
fn helper_timeline_auto_segmentation_hotkey_routes_outside_inputs() {
    assert_eq!(
        resolve("alt-shift-s", WORKSPACE),
        Some(EditorAction::AutoSegmentVisibleSilences)
    );
    assert_eq!(
        resolve("alt-S", WORKSPACE),
        Some(EditorAction::AutoSegmentVisibleSilences)
    );
    assert_eq!(resolve("alt-shift-s", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("alt-shift-s", INPUT), None);
}

#[test]
fn helper_cut_preview_overlay_hotkeys_route_only_with_overlay() {
    assert_eq!(
        resolve("s", WORKSPACE),
        Some(EditorAction::SplitAtPlayhead),
        "plain S splits at the playhead unless a cut preview overlay is active"
    );
    assert_eq!(resolve("s", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("s", INPUT), None);
    assert_eq!(
        resolve("s", CUT_PREVIEW),
        Some(EditorAction::CommitCutPreviewSmartSplit)
    );
    assert_eq!(
        resolve("shift-s", CUT_PREVIEW),
        Some(EditorAction::CommitCutPreviewPlainSplit)
    );
    assert_eq!(
        resolve("S", CUT_PREVIEW),
        Some(EditorAction::CommitCutPreviewPlainSplit)
    );
    assert_eq!(
        resolve("l", CUT_PREVIEW),
        Some(EditorAction::LoopCutPreviewOverlay)
    );
    for target in [FocusTarget::TranscriptText, FocusTarget::Input] {
        let focus = KeyFocus {
            target,
            cut_preview_active: true,
            completions_open: false,
        };
        assert_eq!(
            resolve("s", focus),
            None,
            "cut preview commit must not steal S inside inputs"
        );
    }
}

#[test]
fn cut_preview_l_loops_only_while_overlay_exists() {
    assert_eq!(resolve("l", WORKSPACE), None);
    assert_eq!(
        resolve("l", CUT_PREVIEW),
        Some(EditorAction::LoopCutPreviewOverlay)
    );
    assert_eq!(
        resolve("t", CUT_PREVIEW),
        Some(EditorAction::ToggleOverlay),
        "timeline bindings without a cut-preview counterpart still fire"
    );
    assert_eq!(
        resolve("secondary-f", CUT_PREVIEW),
        Some(EditorAction::OpenFind)
    );
}

#[test]
fn helper_playback_workflow_hotkeys_route_to_speed_rewind_and_speaker_lanes() {
    assert_eq!(
        resolve("shift-1", WORKSPACE),
        Some(EditorAction::IncreasePlaybackSpeed)
    );
    assert_eq!(
        resolve("shift-2", WORKSPACE),
        Some(EditorAction::DecreasePlaybackSpeed)
    );
    assert_eq!(
        resolve("shift-1", TRANSCRIPT_TEXT),
        None,
        "Shift+1 must not steal shifted punctuation while typing"
    );
    assert_eq!(
        resolve("alt-x", WORKSPACE),
        Some(EditorAction::RewindPlaybackOneSecond)
    );
    assert_eq!(
        resolve("alt-1", WORKSPACE),
        Some(EditorAction::SwitchSpeakerWorkflowLane1)
    );
    assert_eq!(
        resolve("alt-2", WORKSPACE),
        Some(EditorAction::SwitchSpeakerWorkflowLane2)
    );
    assert_eq!(
        resolve("alt-`", WORKSPACE),
        Some(EditorAction::ResetSpeakerWorkflow)
    );
}

#[test]
fn global_helper_playback_hotkeys_also_route_while_typing() {
    for focus in [TRANSCRIPT_TEXT, INPUT] {
        assert_eq!(
            resolve("alt-x", focus),
            Some(EditorAction::RewindPlaybackOneSecond)
        );
        assert_eq!(
            resolve("alt-1", focus),
            Some(EditorAction::SwitchSpeakerWorkflowLane1)
        );
        assert_eq!(
            resolve("alt-`", focus),
            Some(EditorAction::ResetSpeakerWorkflow)
        );
    }
}

// ---- routing: escape / enter / tag suggestions -----------------------------------------

#[test]
fn escape_routes_by_focus_target() {
    assert_eq!(
        resolve("escape", WORKSPACE),
        Some(EditorAction::EscapePressed)
    );
    assert_eq!(
        resolve("escape", INPUT),
        Some(EditorAction::CreateRegionEscape),
        "escape inside a dialog input resets the add-region field / closes find"
    );
    assert_eq!(
        resolve("escape", TRANSCRIPT_TEXT),
        Some(EditorAction::TranscriptEscape)
    );
}

#[test]
fn captured_command_enter_submits_create_region_dialog() {
    assert_eq!(
        resolve("secondary-enter", INPUT),
        Some(EditorAction::CreateRegionSubmit)
    );
    assert_eq!(resolve("enter", INPUT), None);
    assert_eq!(resolve("secondary-enter", WORKSPACE), None);
}

#[test]
fn tag_suggestion_keys_route_only_while_list_is_open() {
    assert_eq!(
        resolve("up", TAG_LIST_OPEN),
        Some(EditorAction::CompletionPrevious)
    );
    assert_eq!(
        resolve("down", TAG_LIST_OPEN),
        Some(EditorAction::CompletionNext)
    );
    assert_eq!(
        resolve("enter", TAG_LIST_OPEN),
        Some(EditorAction::CompletionAccept)
    );
    assert_eq!(
        resolve("tab", TAG_LIST_OPEN),
        Some(EditorAction::CompletionAccept)
    );
    assert_eq!(
        resolve("escape", TAG_LIST_OPEN),
        Some(EditorAction::CompletionDismiss),
        "tag dismiss wins over transcript escape while the list is open"
    );

    assert_eq!(resolve("up", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("down", TRANSCRIPT_TEXT), None);
    assert_eq!(resolve("enter", TRANSCRIPT_TEXT), None);
    assert_eq!(
        resolve("escape", TRANSCRIPT_TEXT),
        Some(EditorAction::TranscriptEscape)
    );
}

#[test]
fn tag_suggestion_flag_is_ignored_outside_transcript_text() {
    let workspace_with_flag = KeyFocus {
        target: FocusTarget::Workspace,
        cut_preview_active: false,
        completions_open: true,
    };
    assert_eq!(resolve("up", workspace_with_flag), None);
    assert_eq!(
        resolve("escape", workspace_with_flag),
        Some(EditorAction::EscapePressed)
    );
    assert_eq!(
        resolve("tab", workspace_with_flag),
        Some(EditorAction::SwitchGhostCursorLane)
    );
}

#[test]
fn non_tag_text_bindings_still_route_while_tag_list_is_open() {
    assert_eq!(
        resolve("secondary-z", TAG_LIST_OPEN),
        Some(EditorAction::TextUndo)
    );
    assert_eq!(
        resolve("alt-[", TAG_LIST_OPEN),
        Some(EditorAction::MoveTextBeforeCaretToPreviousSegment)
    );
    assert_eq!(
        resolve("secondary-f", TAG_LIST_OPEN),
        Some(EditorAction::OpenFind)
    );
}

// ---- audition keys ----------------------------------------------------------------------

#[test]
fn audition_keys_map_unmodified_q_w_e_r() {
    assert_eq!(audition_key_for("q"), Some(AuditionKey::BeforeStart));
    assert_eq!(audition_key_for("w"), Some(AuditionKey::AfterStart));
    assert_eq!(audition_key_for("e"), Some(AuditionKey::BeforeEnd));
    assert_eq!(audition_key_for("r"), Some(AuditionKey::AfterEnd));
}

#[test]
fn audition_keys_ignore_command_modifiers_and_other_keys() {
    for keystroke in [
        "alt-r",
        "ctrl-q",
        "secondary-w",
        "cmd-e",
        "a",
        "t",
        "space",
        "",
    ] {
        assert_eq!(audition_key_for(keystroke), None, "`{keystroke}`");
    }
}

#[test]
fn audition_keys_do_not_route_to_keymap_actions() {
    for keystroke in ["q", "w", "e"] {
        assert_eq!(resolve(keystroke, WORKSPACE), None);
    }
}

#[test]
fn audition_key_release_removes_only_that_key() {
    let mut doc = three_row_doc();
    doc.set_audition_key(AuditionKey::BeforeStart, true);
    doc.set_audition_key(AuditionKey::AfterEnd, true);

    doc.set_audition_key(AuditionKey::AfterStart, false);
    assert_eq!(doc.pointer().held_audition_keys.len(), 2);

    doc.set_audition_key(AuditionKey::BeforeStart, false);
    assert!(
        !doc.pointer()
            .held_audition_keys
            .contains(&AuditionKey::BeforeStart)
    );
    assert!(
        doc.pointer()
            .held_audition_keys
            .contains(&AuditionKey::AfterEnd)
    );
}

#[test]
fn window_unfocus_clears_audition_key_holds_and_modifiers() {
    let mut doc = three_row_doc();
    doc.set_audition_key(AuditionKey::BeforeStart, true);
    doc.set_audition_key(AuditionKey::AfterEnd, true);
    doc.set_modifiers(PointerModifiers {
        shift: true,
        alt: true,
        secondary: true,
    });

    ok(doc.window_unfocused());

    assert!(doc.pointer().held_audition_keys.is_empty());
    assert_eq!(doc.pointer().modifiers, PointerModifiers::default());
}

// ---- EditorDoc::perform dispatch -------------------------------------------------------

#[test]
fn perform_returns_none_for_app_level_actions() {
    let mut doc = three_row_doc();
    assert!(doc.perform(EditorAction::BackToProjects, None).is_none());
    assert!(
        doc.perform(EditorAction::ActivateDefaultSurfaceAction, None)
            .is_none(),
        "no doc dialog or find bar open: the UI owns the default action"
    );
    assert!(
        doc.perform(EditorAction::CreateRegionSubmit, None)
            .is_none(),
        "no add-region dialog open"
    );
}

#[test]
fn perform_text_actions_without_focused_row_return_none() {
    let mut doc = three_row_doc();
    for action in [
        EditorAction::TextUndo,
        EditorAction::TextRedo,
        EditorAction::TranscriptEscape,
        EditorAction::MoveTextBeforeCaretToPreviousSegment,
        EditorAction::MoveTextAfterCaretToNextSegment,
        EditorAction::CompletionPrevious,
        EditorAction::CompletionNext,
        EditorAction::CompletionAccept,
        EditorAction::CompletionDismiss,
    ] {
        assert!(doc.perform(action, None).is_none(), "{action:?}");
    }
}

#[test]
fn perform_resolved_d_deletes_selected_segment_and_undo_restores_it() {
    let mut doc = three_row_doc();
    let _ = doc.select_segment("a");

    let action = Keymap::default()
        .resolve("d", WORKSPACE, false)
        .expect("d routes outside inputs");
    ok(doc.perform(action, None).expect("doc action"));
    assert_eq!(ids(&doc), vec!["b", "c"]);

    let undo = Keymap::default()
        .resolve("secondary-z", WORKSPACE, false)
        .expect("undo routes outside inputs");
    ok(doc.perform(undo, None).expect("doc action"));
    assert_eq!(ids(&doc), vec!["a", "b", "c"]);
}

#[test]
fn perform_delete_selected_on_read_only_task_is_refused() {
    let mut doc = read_only_doc(vec![
        seg("a", "lane-1", 0.0, 2.0, "one"),
        seg("b", "lane-1", 3.0, 5.0, "two"),
    ]);
    let _ = doc.select_segment("a");

    let result = doc
        .perform(EditorAction::DeleteSelectedSegment, None)
        .expect("doc action");

    assert!(matches!(result, Err(EditError::ReadOnly { .. })));
    assert_eq!(ids(&doc), vec!["a", "b"]);
}

#[test]
fn perform_play_pause_toggles_playback() {
    let mut doc = three_row_doc();
    assert!(!doc.session().is_playing);

    let outcome = ok(doc
        .perform(EditorAction::PlayPause, None)
        .expect("doc action"));
    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StartPlayback));

    let outcome = ok(doc
        .perform(EditorAction::PlayPause, None)
        .expect("doc action"));
    assert!(!doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::StopPlayback));
}

#[test]
fn perform_speed_steps_change_speed_without_moving_playhead() {
    let mut doc = three_row_doc();
    let _ = doc.set_speed(1.0);
    doc.session_mut().playhead_seconds = 3.75;

    let outcome = ok(doc
        .perform(EditorAction::IncreasePlaybackSpeed, None)
        .expect("doc action"));
    assert_eq!(doc.session().speed, 1.5);
    assert_eq!(doc.session().playhead_seconds, 3.75);
    assert!(outcome.has_effect(&Effect::SpeedChanged(1.5)));

    let outcome = ok(doc
        .perform(EditorAction::DecreasePlaybackSpeed, None)
        .expect("doc action"));
    assert_eq!(doc.session().speed, 1.0);
    assert_eq!(doc.session().playhead_seconds, 3.75);
    assert!(outcome.has_effect(&Effect::SpeedChanged(1.0)));
}

#[test]
fn perform_rewind_moves_playhead_back_one_second() {
    let mut doc = three_row_doc();
    doc.session_mut().playhead_seconds = 3.75;

    ok(doc
        .perform(EditorAction::RewindPlaybackOneSecond, None)
        .expect("doc action"));

    assert!((doc.session().playhead_seconds - 2.75).abs() < 1e-9);
}

#[test]
fn perform_speaker_workflow_switch_and_reset() {
    let mut doc = three_row_doc();

    ok(doc
        .perform(EditorAction::SwitchSpeakerWorkflowLane1, None)
        .expect("doc action"));
    let session = doc.session();
    assert!(matches!(
        &session.track_filter,
        TrackFilter::Lane { label, .. } if label == "Speaker 1"
    ));
    assert!(!session.muted_lane_ids.contains("lane-1"));
    assert!(session.muted_lane_ids.contains("lane-2"));
    assert!(!session.collapsed_lane_ids.contains("lane-1"));
    assert!(session.collapsed_lane_ids.contains("lane-2"));

    ok(doc
        .perform(EditorAction::SwitchSpeakerWorkflowLane2, None)
        .expect("doc action"));
    let session = doc.session();
    assert!(matches!(
        &session.track_filter,
        TrackFilter::Lane { label, .. } if label == "Speaker 2"
    ));
    assert!(session.muted_lane_ids.contains("lane-1"));
    assert!(!session.muted_lane_ids.contains("lane-2"));

    ok(doc
        .perform(EditorAction::ResetSpeakerWorkflow, None)
        .expect("doc action"));
    let session = doc.session();
    assert_eq!(session.track_filter, TrackFilter::All);
    assert!(session.muted_lane_ids.is_empty());
    assert!(session.collapsed_lane_ids.is_empty());
}

#[test]
fn perform_focus_next_segment_focuses_adjacent_row_from_start_without_seeking() {
    let mut doc = three_row_doc();
    let _ = doc.select_segment("a");
    doc.session_mut().playhead_seconds = 1.0;

    let outcome = ok(doc
        .perform(EditorAction::FocusNextSegment, None)
        .expect("doc action"));

    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("b"));
    assert_eq!(doc.session().playhead_seconds, 1.0);
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "b".to_string(),
        caret: 0,
    }));

    let outcome = ok(doc
        .perform(EditorAction::FocusPreviousSegment, None)
        .expect("doc action"));
    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("a"));
    assert_eq!(doc.session().playhead_seconds, 1.0);
    assert!(outcome.has_effect(&Effect::FocusText {
        segment_id: "a".to_string(),
        caret: 0,
    }));
}

#[test]
fn perform_toggle_overlay_and_selection_loop() {
    let mut doc = three_row_doc();

    ok(doc
        .perform(EditorAction::ToggleOverlay, None)
        .expect("doc action"));
    assert!(doc.session().overlay.is_some());
    ok(doc
        .perform(EditorAction::ToggleOverlay, None)
        .expect("doc action"));
    assert!(doc.session().overlay.is_none());

    set_overlay(&mut doc, "lane-2", 3.5, 5.1);
    ok(doc
        .perform(EditorAction::LoopCutPreviewOverlay, None)
        .expect("doc action"));
    assert_eq!(doc.session().loop_range(), overlay_range(&doc));
    ok(doc
        .perform(EditorAction::LoopCutPreviewOverlay, None)
        .expect("doc action"));
    assert_eq!(doc.session().loop_range(), None);
    assert!(doc.session().overlay.is_some(), "L off keeps the selection");
}

fn looping_selection_doc() -> studio_editor::edit::EditorDoc {
    let mut doc = three_row_doc();
    set_overlay(&mut doc, "lane-2", 3.5, 5.1);
    ok(doc
        .perform(EditorAction::LoopCutPreviewOverlay, None)
        .expect("doc action"));
    assert!(doc.session().loop_range().is_some());
    doc
}

#[test]
fn escape_closes_create_region_dialog_before_cancelling_looping_selection() {
    let mut doc = looping_selection_doc();
    doc.open_create_region("lane-2", 12.0, 12.8);
    assert!(doc.create_region().is_some());

    ok(doc
        .perform(EditorAction::EscapePressed, None)
        .expect("doc action"));
    assert!(doc.create_region().is_none());
    assert!(doc.session().loop_range().is_some());

    ok(doc
        .perform(EditorAction::EscapePressed, None)
        .expect("doc action"));
    assert!(doc.session().overlay.is_none());
    assert!(doc.session().loop_range().is_none());
}

#[test]
fn escape_closes_find_bar_before_dialogs_and_selection() {
    let mut doc = looping_selection_doc();
    doc.open_create_region("lane-2", 12.0, 12.8);
    let _ = doc.open_find();

    ok(doc
        .perform(EditorAction::EscapePressed, None)
        .expect("doc action"));

    assert!(!doc.find_state().open);
    assert!(doc.create_region().is_some());
    assert!(doc.session().loop_range().is_some());
}

#[test]
fn captured_escape_closes_find_before_create_region_input_reset() {
    let mut doc = three_row_doc();
    doc.open_create_region("lane-2", 12.0, 12.8);
    let _ = doc.open_find();
    assert!(doc.find_state().open);

    let action = Keymap::default()
        .resolve("escape", INPUT, false)
        .expect("escape routes inside dialog inputs");
    ok(doc.perform(action, None).expect("doc action"));

    assert!(!doc.find_state().open);
    assert!(doc.create_region().is_some());

    ok(doc.perform(action, None).expect("doc action"));
    assert!(
        doc.create_region().is_some(),
        "input escape only resets the field; it never closes the dialog"
    );
}

#[test]
fn activate_default_surface_action_steps_find_when_find_bar_is_open() {
    let mut doc = three_row_doc();
    let _ = doc.open_find();

    let result = doc.perform(EditorAction::ActivateDefaultSurfaceAction, None);

    assert!(matches!(result, Some(Ok(_))));
    assert!(doc.find_state().open);
}

#[test]
fn transcript_escape_in_row_blurs_and_starts_playback() {
    let mut doc = three_row_doc();

    let outcome = ok(doc
        .perform(EditorAction::TranscriptEscape, Some("b"))
        .expect("focused row action"));

    assert!(doc.session().is_playing);
    assert!(outcome.has_effect(&Effect::BlurText));
    assert!(outcome.has_effect(&Effect::StartPlayback));
    assert_eq!(doc.session().selected_segment_id.as_deref(), Some("b"));
}
