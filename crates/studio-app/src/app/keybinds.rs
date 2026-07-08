use std::collections::BTreeMap;

use iced::keyboard::{
    Modifiers,
    key::{Code, Physical},
};
use serde_json::{Map, Value, json};

pub(super) const HOTKEY_STORAGE_KEY: &str = "studio.native.hotkeyBindings.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum HotkeyAction {
    PlayPause,
    PreviousSegment,
    NextSegment,
    Undo,
    Redo,
    OpenFind,
    FindNext,
    FindPrevious,
    ToggleActiveWarningAssertion,
    DeleteSelectedSegment,
    BackToProjects,
    ToggleLoop,
    ToggleOverlay,
    GhostCursorPrevious,
    GhostCursorNext,
    CreateRegionSubmit,
    CreateRegionEscape,
    ResetSpeakerWorkflow,
    SwitchSpeakerWorkflowLane1,
    SwitchSpeakerWorkflowLane2,
    RewindPlaybackOneSecond,
    MoveTextBeforeCaretToPreviousSegment,
    MoveTextAfterCaretToNextSegment,
    ConvertSelectedNumberToSkaz,
    AutofixCurrentLintRow,
    AutofixAllLintRows,
    IncreasePlaybackSpeed,
    DecreasePlaybackSpeed,
    MergeSelectedSegmentAbove,
    MergeSelectedSegmentBelow,
    TrimCurrentSegmentToAudio,
    TrimAllVisibleSegmentsToAudio,
    AutoInsertSegmentAtCaret,
    AutoSegmentVisibleSilences,
    DraftTranscribeSelectedSegment,
    CommitCutPreviewSmartSplit,
    CommitCutPreviewPlainSplit,
    LoopCutPreviewOverlay,
    EscapePressed,
    ActivateDefaultSurfaceAction,
}

impl HotkeyAction {
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::PlayPause => "playPause",
            Self::PreviousSegment => "previousSegment",
            Self::NextSegment => "nextSegment",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::OpenFind => "openFind",
            Self::FindNext => "findNext",
            Self::FindPrevious => "findPrevious",
            Self::ToggleActiveWarningAssertion => "toggleActiveWarningAssertion",
            Self::DeleteSelectedSegment => "deleteSelectedSegment",
            Self::BackToProjects => "backToProjects",
            Self::ToggleLoop => "toggleLoop",
            Self::ToggleOverlay => "toggleOverlay",
            Self::GhostCursorPrevious => "ghostCursorPrevious",
            Self::GhostCursorNext => "ghostCursorNext",
            Self::CreateRegionSubmit => "createRegionSubmit",
            Self::CreateRegionEscape => "createRegionEscape",
            Self::ResetSpeakerWorkflow => "resetSpeakerWorkflow",
            Self::SwitchSpeakerWorkflowLane1 => "switchSpeakerWorkflowLane1",
            Self::SwitchSpeakerWorkflowLane2 => "switchSpeakerWorkflowLane2",
            Self::RewindPlaybackOneSecond => "rewindPlaybackOneSecond",
            Self::MoveTextBeforeCaretToPreviousSegment => "moveTextBeforeCaretToPreviousSegment",
            Self::MoveTextAfterCaretToNextSegment => "moveTextAfterCaretToNextSegment",
            Self::ConvertSelectedNumberToSkaz => "convertSelectedNumberToSkaz",
            Self::AutofixCurrentLintRow => "autofixCurrentLintRow",
            Self::AutofixAllLintRows => "autofixAllLintRows",
            Self::IncreasePlaybackSpeed => "increasePlaybackSpeed",
            Self::DecreasePlaybackSpeed => "decreasePlaybackSpeed",
            Self::MergeSelectedSegmentAbove => "mergeSelectedSegmentAbove",
            Self::MergeSelectedSegmentBelow => "mergeSelectedSegmentBelow",
            Self::TrimCurrentSegmentToAudio => "trimCurrentSegmentToAudio",
            Self::TrimAllVisibleSegmentsToAudio => "trimAllVisibleSegmentsToAudio",
            Self::AutoInsertSegmentAtCaret => "autoInsertSegmentAtCaret",
            Self::AutoSegmentVisibleSilences => "autoSegmentVisibleSilences",
            Self::DraftTranscribeSelectedSegment => "draftTranscribeSelectedSegment",
            Self::CommitCutPreviewSmartSplit => "commitCutPreviewSmartSplit",
            Self::CommitCutPreviewPlainSplit => "commitCutPreviewPlainSplit",
            Self::LoopCutPreviewOverlay => "loopCutPreviewOverlay",
            Self::EscapePressed => "escapePressed",
            Self::ActivateDefaultSurfaceAction => "activateDefaultSurfaceAction",
        }
    }

    fn from_id(value: &str) -> Option<Self> {
        Some(match value {
            "playPause" => Self::PlayPause,
            "previousSegment" => Self::PreviousSegment,
            "nextSegment" => Self::NextSegment,
            "undo" => Self::Undo,
            "redo" => Self::Redo,
            "openFind" => Self::OpenFind,
            "findNext" => Self::FindNext,
            "findPrevious" => Self::FindPrevious,
            "toggleActiveWarningAssertion" => Self::ToggleActiveWarningAssertion,
            "deleteSelectedSegment" => Self::DeleteSelectedSegment,
            "backToProjects" => Self::BackToProjects,
            "toggleLoop" => Self::ToggleLoop,
            "toggleOverlay" => Self::ToggleOverlay,
            "ghostCursorPrevious" => Self::GhostCursorPrevious,
            "ghostCursorNext" => Self::GhostCursorNext,
            "createRegionSubmit" => Self::CreateRegionSubmit,
            "createRegionEscape" => Self::CreateRegionEscape,
            "resetSpeakerWorkflow" => Self::ResetSpeakerWorkflow,
            "switchSpeakerWorkflowLane1" => Self::SwitchSpeakerWorkflowLane1,
            "switchSpeakerWorkflowLane2" => Self::SwitchSpeakerWorkflowLane2,
            "rewindPlaybackOneSecond" => Self::RewindPlaybackOneSecond,
            "moveTextBeforeCaretToPreviousSegment" => Self::MoveTextBeforeCaretToPreviousSegment,
            "moveTextAfterCaretToNextSegment" => Self::MoveTextAfterCaretToNextSegment,
            "convertSelectedNumberToSkaz" => Self::ConvertSelectedNumberToSkaz,
            "autofixCurrentLintRow" => Self::AutofixCurrentLintRow,
            "autofixAllLintRows" => Self::AutofixAllLintRows,
            "increasePlaybackSpeed" => Self::IncreasePlaybackSpeed,
            "decreasePlaybackSpeed" => Self::DecreasePlaybackSpeed,
            "mergeSelectedSegmentAbove" => Self::MergeSelectedSegmentAbove,
            "mergeSelectedSegmentBelow" => Self::MergeSelectedSegmentBelow,
            "trimCurrentSegmentToAudio" => Self::TrimCurrentSegmentToAudio,
            "trimAllVisibleSegmentsToAudio" => Self::TrimAllVisibleSegmentsToAudio,
            "autoInsertSegmentAtCaret" => Self::AutoInsertSegmentAtCaret,
            "autoSegmentVisibleSilences" => Self::AutoSegmentVisibleSilences,
            "draftTranscribeSelectedSegment" => Self::DraftTranscribeSelectedSegment,
            "commitCutPreviewSmartSplit" => Self::CommitCutPreviewSmartSplit,
            "commitCutPreviewPlainSplit" => Self::CommitCutPreviewPlainSplit,
            "loopCutPreviewOverlay" => Self::LoopCutPreviewOverlay,
            "escapePressed" => Self::EscapePressed,
            "activateDefaultSurfaceAction" => Self::ActivateDefaultSurfaceAction,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct KeyBinding {
    pub(super) modifiers: Modifiers,
    pub(super) code: Code,
}

impl KeyBinding {
    pub(super) const fn new(modifiers: Modifiers, code: Code) -> Self {
        Self { modifiers, code }
    }

    pub(super) fn from_physical(physical_key: Physical, modifiers: Modifiers) -> Option<Self> {
        let Physical::Code(code) = physical_key else {
            return None;
        };
        if is_modifier_code(code) {
            return None;
        }
        Some(Self {
            modifiers: shortcut_modifiers(modifiers),
            code,
        })
    }

    fn matches(self, code: Code, modifiers: Modifiers) -> bool {
        self.code == code && self.modifiers == shortcut_modifiers(modifiers)
    }

    pub(super) fn label(self) -> String {
        let mut parts = Vec::new();
        let modifiers = shortcut_modifiers(self.modifiers);
        if modifiers.control() {
            parts.push("Ctrl".to_string());
        }
        if modifiers.logo() {
            parts.push(if cfg!(target_os = "macos") {
                "Cmd".to_string()
            } else {
                "Win".to_string()
            });
        }
        if modifiers.alt() {
            parts.push("Alt".to_string());
        }
        if modifiers.shift() {
            parts.push("Shift".to_string());
        }
        parts.push(code_label(self.code));
        parts.join("+")
    }

    fn to_value(self) -> Value {
        json!({
            "code": code_storage_name(self.code),
            "modifiers": modifier_names(self.modifiers),
        })
    }

    fn from_value(value: &Value) -> Option<Self> {
        let code = value
            .get("code")
            .and_then(Value::as_str)
            .and_then(parse_code_storage_name)?;
        let modifiers = value.get("modifiers").and_then(parse_modifier_names)?;
        Some(Self { modifiers, code })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct HotkeyBindings {
    overrides: BTreeMap<HotkeyAction, KeyBinding>,
}

impl HotkeyBindings {
    pub(super) fn from_storage(source: Option<&str>) -> Self {
        let Some(source) = source else {
            return Self::default();
        };
        let Ok(Value::Object(root)) = serde_json::from_str::<Value>(source) else {
            return Self::default();
        };
        let mut overrides = BTreeMap::new();
        for (action_id, value) in root {
            if let Some(action) = HotkeyAction::from_id(&action_id)
                && let Some(binding) = KeyBinding::from_value(&value)
            {
                overrides.insert(action, binding);
            }
        }
        Self { overrides }
    }

    pub(super) fn to_storage(&self) -> String {
        let mut root = Map::new();
        for (action, binding) in &self.overrides {
            root.insert(action.id().to_string(), binding.to_value());
        }
        Value::Object(root).to_string()
    }

    pub(super) fn set_override(&mut self, action: HotkeyAction, binding: KeyBinding) {
        self.overrides.insert(action, binding);
    }

    pub(super) fn reset_override(&mut self, action: HotkeyAction) {
        self.overrides.remove(&action);
    }

    pub(super) fn is_overridden(&self, action: HotkeyAction) -> bool {
        self.overrides.contains_key(&action)
    }

    pub(super) fn matches(
        &self,
        action: HotkeyAction,
        physical_key: Option<Physical>,
        modifiers: Modifiers,
    ) -> bool {
        let Some(Physical::Code(code)) = physical_key else {
            return false;
        };
        if let Some(binding) = self.overrides.get(&action).copied() {
            return binding.matches(code, modifiers);
        }
        default_bindings(action)
            .iter()
            .copied()
            .any(|binding| binding.matches(code, modifiers))
    }

    pub(super) fn labels_for(&self, action: HotkeyAction) -> Vec<String> {
        if let Some(binding) = self.overrides.get(&action).copied() {
            return vec![binding.label()];
        }
        default_bindings(action)
            .iter()
            .copied()
            .map(KeyBinding::label)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HotkeyDefinition {
    pub(super) action: HotkeyAction,
    pub(super) section: &'static str,
    pub(super) description: &'static str,
}

pub(super) const HOTKEY_DEFINITIONS: &[HotkeyDefinition] = &[
    HotkeyDefinition::new(HotkeyAction::PlayPause, "Global Controls", "Play/Pause"),
    HotkeyDefinition::new(
        HotkeyAction::PreviousSegment,
        "Global Controls",
        "Seek to previous segment",
    ),
    HotkeyDefinition::new(
        HotkeyAction::NextSegment,
        "Global Controls",
        "Seek to next segment",
    ),
    HotkeyDefinition::new(HotkeyAction::Undo, "Global Controls", "Undo"),
    HotkeyDefinition::new(HotkeyAction::Redo, "Global Controls", "Redo"),
    HotkeyDefinition::new(
        HotkeyAction::OpenFind,
        "Global Controls",
        "Find in transcript",
    ),
    HotkeyDefinition::new(HotkeyAction::FindNext, "Global Controls", "Find next match"),
    HotkeyDefinition::new(
        HotkeyAction::FindPrevious,
        "Global Controls",
        "Find previous match",
    ),
    HotkeyDefinition::new(
        HotkeyAction::ToggleActiveWarningAssertion,
        "Global Controls",
        "Confirm/undo warning on active segment",
    ),
    HotkeyDefinition::new(
        HotkeyAction::DeleteSelectedSegment,
        "Global Controls",
        "Delete current segment when not typing",
    ),
    HotkeyDefinition::new(
        HotkeyAction::BackToProjects,
        "Global Controls",
        "Back to projects",
    ),
    HotkeyDefinition::new(HotkeyAction::ToggleLoop, "Timeline", "Toggle loop on/off"),
    HotkeyDefinition::new(
        HotkeyAction::ToggleOverlay,
        "Timeline",
        "Toggle 1-second overlay",
    ),
    HotkeyDefinition::new(
        HotkeyAction::GhostCursorPrevious,
        "Transcript Navigation",
        "Move ghost cursor to previous row",
    ),
    HotkeyDefinition::new(
        HotkeyAction::GhostCursorNext,
        "Transcript Navigation",
        "Move ghost cursor to next row",
    ),
    HotkeyDefinition::new(
        HotkeyAction::CreateRegionSubmit,
        "Dialog Controls",
        "Submit Add Region dialog",
    ),
    HotkeyDefinition::new(
        HotkeyAction::CreateRegionEscape,
        "Dialog Controls",
        "Cancel Add Region input",
    ),
    HotkeyDefinition::new(
        HotkeyAction::EscapePressed,
        "Dialog Controls",
        "Close active dialog or clear loop",
    ),
    HotkeyDefinition::new(
        HotkeyAction::ActivateDefaultSurfaceAction,
        "Dialog Controls",
        "Activate default action",
    ),
    HotkeyDefinition::new(
        HotkeyAction::ResetSpeakerWorkflow,
        "Helper Editing",
        "Reset speaker workflow lanes",
    ),
    HotkeyDefinition::new(
        HotkeyAction::SwitchSpeakerWorkflowLane1,
        "Helper Editing",
        "Switch active speaker workflow lane 1",
    ),
    HotkeyDefinition::new(
        HotkeyAction::SwitchSpeakerWorkflowLane2,
        "Helper Editing",
        "Switch active speaker workflow lane 2",
    ),
    HotkeyDefinition::new(
        HotkeyAction::RewindPlaybackOneSecond,
        "Helper Editing",
        "Rewind playback 1000ms",
    ),
    HotkeyDefinition::new(
        HotkeyAction::MoveTextBeforeCaretToPreviousSegment,
        "Helper Editing",
        "Move text before caret to previous segment",
    ),
    HotkeyDefinition::new(
        HotkeyAction::MoveTextAfterCaretToNextSegment,
        "Helper Editing",
        "Move text after caret to next segment",
    ),
    HotkeyDefinition::new(
        HotkeyAction::ConvertSelectedNumberToSkaz,
        "Helper Editing",
        "Convert selected number to skaz",
    ),
    HotkeyDefinition::new(
        HotkeyAction::AutofixCurrentLintRow,
        "Helper Editing",
        "Auto-fix lint issues in current row",
    ),
    HotkeyDefinition::new(
        HotkeyAction::AutofixAllLintRows,
        "Helper Editing",
        "Auto-fix lint issues in all rows",
    ),
    HotkeyDefinition::new(
        HotkeyAction::IncreasePlaybackSpeed,
        "Timeline Editing",
        "Increase playback speed",
    ),
    HotkeyDefinition::new(
        HotkeyAction::DecreasePlaybackSpeed,
        "Timeline Editing",
        "Decrease playback speed",
    ),
    HotkeyDefinition::new(
        HotkeyAction::MergeSelectedSegmentAbove,
        "Timeline Editing",
        "Merge with previous segment",
    ),
    HotkeyDefinition::new(
        HotkeyAction::MergeSelectedSegmentBelow,
        "Timeline Editing",
        "Merge with next segment",
    ),
    HotkeyDefinition::new(
        HotkeyAction::TrimCurrentSegmentToAudio,
        "Timeline Editing",
        "Trim current segment to nearby visible audio",
    ),
    HotkeyDefinition::new(
        HotkeyAction::TrimAllVisibleSegmentsToAudio,
        "Timeline Editing",
        "Trim all visible segments to nearby visible audio",
    ),
    HotkeyDefinition::new(
        HotkeyAction::AutoInsertSegmentAtCaret,
        "Timeline Editing",
        "Create empty segment around nearest uncovered speech near caret",
    ),
    HotkeyDefinition::new(
        HotkeyAction::AutoSegmentVisibleSilences,
        "Timeline Editing",
        "Split visible segments on silence runs",
    ),
    HotkeyDefinition::new(
        HotkeyAction::DraftTranscribeSelectedSegment,
        "Timeline Editing",
        "Transcribe current empty segment with native drafting audio",
    ),
    HotkeyDefinition::new(
        HotkeyAction::CommitCutPreviewSmartSplit,
        "Cut Preview",
        "Commit active cut preview with smart text split",
    ),
    HotkeyDefinition::new(
        HotkeyAction::CommitCutPreviewPlainSplit,
        "Cut Preview",
        "Commit active cut preview without smart text split",
    ),
    HotkeyDefinition::new(
        HotkeyAction::LoopCutPreviewOverlay,
        "Cut Preview",
        "Loop active cut preview range",
    ),
];

impl HotkeyDefinition {
    const fn new(action: HotkeyAction, section: &'static str, description: &'static str) -> Self {
        Self {
            action,
            section,
            description,
        }
    }
}

const EMPTY: Modifiers = Modifiers::empty();
const COMMAND: Modifiers = Modifiers::COMMAND;
const ALT: Modifiers = Modifiers::ALT;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT_SHIFT: Modifiers = Modifiers::ALT.union(Modifiers::SHIFT);
const COMMAND_SHIFT: Modifiers = Modifiers::COMMAND.union(Modifiers::SHIFT);
const PLAY_PAUSE_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::Space)];
const PREVIOUS_SEGMENT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::ArrowLeft)];
const NEXT_SEGMENT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::ArrowRight)];
const UNDO_BINDINGS: &[KeyBinding] = &[
    KeyBinding::new(COMMAND, Code::KeyZ),
    KeyBinding::new(EMPTY, Code::Undo),
];
const REDO_BINDINGS: &[KeyBinding] = &[
    KeyBinding::new(COMMAND_SHIFT, Code::KeyZ),
    KeyBinding::new(COMMAND, Code::KeyY),
    KeyBinding::new(EMPTY, Code::Again),
];
const OPEN_FIND_BINDINGS: &[KeyBinding] = &[
    KeyBinding::new(COMMAND, Code::KeyF),
    KeyBinding::new(EMPTY, Code::Find),
];
const FIND_NEXT_BINDINGS: &[KeyBinding] = &[
    KeyBinding::new(EMPTY, Code::F3),
    KeyBinding::new(COMMAND, Code::KeyG),
];
const FIND_PREVIOUS_BINDINGS: &[KeyBinding] = &[
    KeyBinding::new(SHIFT, Code::F3),
    KeyBinding::new(COMMAND_SHIFT, Code::KeyG),
];
const TOGGLE_WARNING_BINDINGS: &[KeyBinding] = &[KeyBinding::new(COMMAND, Code::KeyJ)];
const DELETE_SELECTED_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::KeyD)];
const BACK_TO_PROJECTS_BINDINGS: &[KeyBinding] = &[
    KeyBinding::new(ALT, Code::ArrowLeft),
    KeyBinding::new(EMPTY, Code::BrowserBack),
];
const TOGGLE_LOOP_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::KeyL)];
const TOGGLE_OVERLAY_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::KeyT)];
const GHOST_CURSOR_PREVIOUS_BINDINGS: &[KeyBinding] = &[KeyBinding::new(SHIFT, Code::Tab)];
const GHOST_CURSOR_NEXT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::Tab)];
const CREATE_REGION_SUBMIT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(COMMAND, Code::Enter)];
const CREATE_REGION_ESCAPE_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::Escape)];
const RESET_SPEAKER_WORKFLOW_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::Backquote)];
const SWITCH_SPEAKER_1_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::Digit1)];
const SWITCH_SPEAKER_2_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::Digit2)];
const REWIND_PLAYBACK_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::KeyX)];
const MOVE_TEXT_BEFORE_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::BracketLeft)];
const MOVE_TEXT_AFTER_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::BracketRight)];
const CONVERT_SKAZ_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::KeyA)];
const AUTOFIX_CURRENT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::KeyF)];
const AUTOFIX_ALL_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT_SHIFT, Code::KeyF)];
const INCREASE_SPEED_BINDINGS: &[KeyBinding] = &[KeyBinding::new(SHIFT, Code::Digit1)];
const DECREASE_SPEED_BINDINGS: &[KeyBinding] = &[KeyBinding::new(SHIFT, Code::Digit2)];
const MERGE_ABOVE_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT_SHIFT, Code::ArrowUp)];
const MERGE_BELOW_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT_SHIFT, Code::ArrowDown)];
const TRIM_CURRENT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::KeyR)];
const TRIM_ALL_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT_SHIFT, Code::KeyR)];
const AUTO_INSERT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT, Code::KeyC)];
const AUTO_SEGMENT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT_SHIFT, Code::KeyS)];
const DRAFT_TRANSCRIBE_BINDINGS: &[KeyBinding] = &[KeyBinding::new(ALT_SHIFT, Code::KeyG)];
const CUT_SMART_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::KeyS)];
const CUT_PLAIN_BINDINGS: &[KeyBinding] = &[KeyBinding::new(SHIFT, Code::KeyS)];
const LOOP_CUT_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::KeyL)];
const ESCAPE_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::Escape)];
const DEFAULT_ACTION_BINDINGS: &[KeyBinding] = &[KeyBinding::new(EMPTY, Code::Enter)];

fn default_bindings(action: HotkeyAction) -> &'static [KeyBinding] {
    match action {
        HotkeyAction::PlayPause => PLAY_PAUSE_BINDINGS,
        HotkeyAction::PreviousSegment => PREVIOUS_SEGMENT_BINDINGS,
        HotkeyAction::NextSegment => NEXT_SEGMENT_BINDINGS,
        HotkeyAction::Undo => UNDO_BINDINGS,
        HotkeyAction::Redo => REDO_BINDINGS,
        HotkeyAction::OpenFind => OPEN_FIND_BINDINGS,
        HotkeyAction::FindNext => FIND_NEXT_BINDINGS,
        HotkeyAction::FindPrevious => FIND_PREVIOUS_BINDINGS,
        HotkeyAction::ToggleActiveWarningAssertion => TOGGLE_WARNING_BINDINGS,
        HotkeyAction::DeleteSelectedSegment => DELETE_SELECTED_BINDINGS,
        HotkeyAction::BackToProjects => BACK_TO_PROJECTS_BINDINGS,
        HotkeyAction::ToggleLoop => TOGGLE_LOOP_BINDINGS,
        HotkeyAction::ToggleOverlay => TOGGLE_OVERLAY_BINDINGS,
        HotkeyAction::GhostCursorPrevious => GHOST_CURSOR_PREVIOUS_BINDINGS,
        HotkeyAction::GhostCursorNext => GHOST_CURSOR_NEXT_BINDINGS,
        HotkeyAction::CreateRegionSubmit => CREATE_REGION_SUBMIT_BINDINGS,
        HotkeyAction::CreateRegionEscape => CREATE_REGION_ESCAPE_BINDINGS,
        HotkeyAction::ResetSpeakerWorkflow => RESET_SPEAKER_WORKFLOW_BINDINGS,
        HotkeyAction::SwitchSpeakerWorkflowLane1 => SWITCH_SPEAKER_1_BINDINGS,
        HotkeyAction::SwitchSpeakerWorkflowLane2 => SWITCH_SPEAKER_2_BINDINGS,
        HotkeyAction::RewindPlaybackOneSecond => REWIND_PLAYBACK_BINDINGS,
        HotkeyAction::MoveTextBeforeCaretToPreviousSegment => MOVE_TEXT_BEFORE_BINDINGS,
        HotkeyAction::MoveTextAfterCaretToNextSegment => MOVE_TEXT_AFTER_BINDINGS,
        HotkeyAction::ConvertSelectedNumberToSkaz => CONVERT_SKAZ_BINDINGS,
        HotkeyAction::AutofixCurrentLintRow => AUTOFIX_CURRENT_BINDINGS,
        HotkeyAction::AutofixAllLintRows => AUTOFIX_ALL_BINDINGS,
        HotkeyAction::IncreasePlaybackSpeed => INCREASE_SPEED_BINDINGS,
        HotkeyAction::DecreasePlaybackSpeed => DECREASE_SPEED_BINDINGS,
        HotkeyAction::MergeSelectedSegmentAbove => MERGE_ABOVE_BINDINGS,
        HotkeyAction::MergeSelectedSegmentBelow => MERGE_BELOW_BINDINGS,
        HotkeyAction::TrimCurrentSegmentToAudio => TRIM_CURRENT_BINDINGS,
        HotkeyAction::TrimAllVisibleSegmentsToAudio => TRIM_ALL_BINDINGS,
        HotkeyAction::AutoInsertSegmentAtCaret => AUTO_INSERT_BINDINGS,
        HotkeyAction::AutoSegmentVisibleSilences => AUTO_SEGMENT_BINDINGS,
        HotkeyAction::DraftTranscribeSelectedSegment => DRAFT_TRANSCRIBE_BINDINGS,
        HotkeyAction::CommitCutPreviewSmartSplit => CUT_SMART_BINDINGS,
        HotkeyAction::CommitCutPreviewPlainSplit => CUT_PLAIN_BINDINGS,
        HotkeyAction::LoopCutPreviewOverlay => LOOP_CUT_BINDINGS,
        HotkeyAction::EscapePressed => ESCAPE_BINDINGS,
        HotkeyAction::ActivateDefaultSurfaceAction => DEFAULT_ACTION_BINDINGS,
    }
}

fn shortcut_modifiers(modifiers: Modifiers) -> Modifiers {
    modifiers & (Modifiers::SHIFT | Modifiers::CTRL | Modifiers::ALT | Modifiers::LOGO)
}

fn is_modifier_code(code: Code) -> bool {
    matches!(
        code,
        Code::AltLeft
            | Code::AltRight
            | Code::ControlLeft
            | Code::ControlRight
            | Code::ShiftLeft
            | Code::ShiftRight
            | Code::SuperLeft
            | Code::SuperRight
            | Code::CapsLock
            | Code::Fn
            | Code::FnLock
            | Code::NumLock
            | Code::ScrollLock
    )
}

fn modifier_names(modifiers: Modifiers) -> Vec<&'static str> {
    let modifiers = shortcut_modifiers(modifiers);
    let mut names = Vec::new();
    if modifiers.shift() {
        names.push("shift");
    }
    if modifiers.control() {
        names.push("control");
    }
    if modifiers.alt() {
        names.push("alt");
    }
    if modifiers.logo() {
        names.push("logo");
    }
    names
}

fn parse_modifier_names(value: &Value) -> Option<Modifiers> {
    let mut modifiers = Modifiers::empty();
    for name in value.as_array()? {
        match name.as_str()? {
            "shift" => modifiers |= Modifiers::SHIFT,
            "control" => modifiers |= Modifiers::CTRL,
            "alt" => modifiers |= Modifiers::ALT,
            "logo" => modifiers |= Modifiers::LOGO,
            _ => return None,
        }
    }
    Some(modifiers)
}

fn code_label(code: Code) -> String {
    match code {
        Code::Backquote => "`".to_string(),
        Code::Backslash => "\\".to_string(),
        Code::BracketLeft => "[".to_string(),
        Code::BracketRight => "]".to_string(),
        Code::Comma => ",".to_string(),
        Code::Digit0 => "0".to_string(),
        Code::Digit1 => "1".to_string(),
        Code::Digit2 => "2".to_string(),
        Code::Digit3 => "3".to_string(),
        Code::Digit4 => "4".to_string(),
        Code::Digit5 => "5".to_string(),
        Code::Digit6 => "6".to_string(),
        Code::Digit7 => "7".to_string(),
        Code::Digit8 => "8".to_string(),
        Code::Digit9 => "9".to_string(),
        Code::Equal => "=".to_string(),
        Code::KeyA => "A".to_string(),
        Code::KeyB => "B".to_string(),
        Code::KeyC => "C".to_string(),
        Code::KeyD => "D".to_string(),
        Code::KeyE => "E".to_string(),
        Code::KeyF => "F".to_string(),
        Code::KeyG => "G".to_string(),
        Code::KeyH => "H".to_string(),
        Code::KeyI => "I".to_string(),
        Code::KeyJ => "J".to_string(),
        Code::KeyK => "K".to_string(),
        Code::KeyL => "L".to_string(),
        Code::KeyM => "M".to_string(),
        Code::KeyN => "N".to_string(),
        Code::KeyO => "O".to_string(),
        Code::KeyP => "P".to_string(),
        Code::KeyQ => "Q".to_string(),
        Code::KeyR => "R".to_string(),
        Code::KeyS => "S".to_string(),
        Code::KeyT => "T".to_string(),
        Code::KeyU => "U".to_string(),
        Code::KeyV => "V".to_string(),
        Code::KeyW => "W".to_string(),
        Code::KeyX => "X".to_string(),
        Code::KeyY => "Y".to_string(),
        Code::KeyZ => "Z".to_string(),
        Code::Minus => "-".to_string(),
        Code::Period => ".".to_string(),
        Code::Quote => "'".to_string(),
        Code::Semicolon => ";".to_string(),
        Code::Slash => "/".to_string(),
        Code::Space => "Space".to_string(),
        Code::Tab => "Tab".to_string(),
        Code::Enter => "Enter".to_string(),
        Code::Backspace => "Backspace".to_string(),
        Code::Delete => "Delete".to_string(),
        Code::Escape => "Escape".to_string(),
        Code::ArrowLeft => "←".to_string(),
        Code::ArrowRight => "→".to_string(),
        Code::ArrowUp => "↑".to_string(),
        Code::ArrowDown => "↓".to_string(),
        Code::F1 => "F1".to_string(),
        Code::F2 => "F2".to_string(),
        Code::F3 => "F3".to_string(),
        Code::F4 => "F4".to_string(),
        Code::F5 => "F5".to_string(),
        Code::F6 => "F6".to_string(),
        Code::F7 => "F7".to_string(),
        Code::F8 => "F8".to_string(),
        Code::F9 => "F9".to_string(),
        Code::F10 => "F10".to_string(),
        Code::F11 => "F11".to_string(),
        Code::F12 => "F12".to_string(),
        _ => code_storage_name(code),
    }
}

fn code_storage_name(code: Code) -> String {
    match code {
        Code::Backquote => "Backquote".to_string(),
        Code::Backslash => "Backslash".to_string(),
        Code::BracketLeft => "BracketLeft".to_string(),
        Code::BracketRight => "BracketRight".to_string(),
        Code::Comma => "Comma".to_string(),
        Code::Digit0 => "Digit0".to_string(),
        Code::Digit1 => "Digit1".to_string(),
        Code::Digit2 => "Digit2".to_string(),
        Code::Digit3 => "Digit3".to_string(),
        Code::Digit4 => "Digit4".to_string(),
        Code::Digit5 => "Digit5".to_string(),
        Code::Digit6 => "Digit6".to_string(),
        Code::Digit7 => "Digit7".to_string(),
        Code::Digit8 => "Digit8".to_string(),
        Code::Digit9 => "Digit9".to_string(),
        Code::Equal => "Equal".to_string(),
        Code::IntlBackslash => "IntlBackslash".to_string(),
        Code::IntlRo => "IntlRo".to_string(),
        Code::IntlYen => "IntlYen".to_string(),
        Code::KeyA => "KeyA".to_string(),
        Code::KeyB => "KeyB".to_string(),
        Code::KeyC => "KeyC".to_string(),
        Code::KeyD => "KeyD".to_string(),
        Code::KeyE => "KeyE".to_string(),
        Code::KeyF => "KeyF".to_string(),
        Code::KeyG => "KeyG".to_string(),
        Code::KeyH => "KeyH".to_string(),
        Code::KeyI => "KeyI".to_string(),
        Code::KeyJ => "KeyJ".to_string(),
        Code::KeyK => "KeyK".to_string(),
        Code::KeyL => "KeyL".to_string(),
        Code::KeyM => "KeyM".to_string(),
        Code::KeyN => "KeyN".to_string(),
        Code::KeyO => "KeyO".to_string(),
        Code::KeyP => "KeyP".to_string(),
        Code::KeyQ => "KeyQ".to_string(),
        Code::KeyR => "KeyR".to_string(),
        Code::KeyS => "KeyS".to_string(),
        Code::KeyT => "KeyT".to_string(),
        Code::KeyU => "KeyU".to_string(),
        Code::KeyV => "KeyV".to_string(),
        Code::KeyW => "KeyW".to_string(),
        Code::KeyX => "KeyX".to_string(),
        Code::KeyY => "KeyY".to_string(),
        Code::KeyZ => "KeyZ".to_string(),
        Code::Minus => "Minus".to_string(),
        Code::Period => "Period".to_string(),
        Code::Quote => "Quote".to_string(),
        Code::Semicolon => "Semicolon".to_string(),
        Code::Slash => "Slash".to_string(),
        Code::AltLeft => "AltLeft".to_string(),
        Code::AltRight => "AltRight".to_string(),
        Code::Backspace => "Backspace".to_string(),
        Code::CapsLock => "CapsLock".to_string(),
        Code::ContextMenu => "ContextMenu".to_string(),
        Code::ControlLeft => "ControlLeft".to_string(),
        Code::ControlRight => "ControlRight".to_string(),
        Code::Enter => "Enter".to_string(),
        Code::SuperLeft => "SuperLeft".to_string(),
        Code::SuperRight => "SuperRight".to_string(),
        Code::ShiftLeft => "ShiftLeft".to_string(),
        Code::ShiftRight => "ShiftRight".to_string(),
        Code::Space => "Space".to_string(),
        Code::Tab => "Tab".to_string(),
        Code::Convert => "Convert".to_string(),
        Code::KanaMode => "KanaMode".to_string(),
        Code::Lang1 => "Lang1".to_string(),
        Code::Lang2 => "Lang2".to_string(),
        Code::Lang3 => "Lang3".to_string(),
        Code::Lang4 => "Lang4".to_string(),
        Code::Lang5 => "Lang5".to_string(),
        Code::NonConvert => "NonConvert".to_string(),
        Code::Delete => "Delete".to_string(),
        Code::End => "End".to_string(),
        Code::Help => "Help".to_string(),
        Code::Home => "Home".to_string(),
        Code::Insert => "Insert".to_string(),
        Code::PageDown => "PageDown".to_string(),
        Code::PageUp => "PageUp".to_string(),
        Code::ArrowDown => "ArrowDown".to_string(),
        Code::ArrowLeft => "ArrowLeft".to_string(),
        Code::ArrowRight => "ArrowRight".to_string(),
        Code::ArrowUp => "ArrowUp".to_string(),
        Code::NumLock => "NumLock".to_string(),
        Code::Numpad0 => "Numpad0".to_string(),
        Code::Numpad1 => "Numpad1".to_string(),
        Code::Numpad2 => "Numpad2".to_string(),
        Code::Numpad3 => "Numpad3".to_string(),
        Code::Numpad4 => "Numpad4".to_string(),
        Code::Numpad5 => "Numpad5".to_string(),
        Code::Numpad6 => "Numpad6".to_string(),
        Code::Numpad7 => "Numpad7".to_string(),
        Code::Numpad8 => "Numpad8".to_string(),
        Code::Numpad9 => "Numpad9".to_string(),
        Code::NumpadAdd => "NumpadAdd".to_string(),
        Code::NumpadBackspace => "NumpadBackspace".to_string(),
        Code::NumpadClear => "NumpadClear".to_string(),
        Code::NumpadClearEntry => "NumpadClearEntry".to_string(),
        Code::NumpadComma => "NumpadComma".to_string(),
        Code::NumpadDecimal => "NumpadDecimal".to_string(),
        Code::NumpadDivide => "NumpadDivide".to_string(),
        Code::NumpadEnter => "NumpadEnter".to_string(),
        Code::NumpadEqual => "NumpadEqual".to_string(),
        Code::NumpadHash => "NumpadHash".to_string(),
        Code::NumpadMemoryAdd => "NumpadMemoryAdd".to_string(),
        Code::NumpadMemoryClear => "NumpadMemoryClear".to_string(),
        Code::NumpadMemoryRecall => "NumpadMemoryRecall".to_string(),
        Code::NumpadMemoryStore => "NumpadMemoryStore".to_string(),
        Code::NumpadMemorySubtract => "NumpadMemorySubtract".to_string(),
        Code::NumpadMultiply => "NumpadMultiply".to_string(),
        Code::NumpadParenLeft => "NumpadParenLeft".to_string(),
        Code::NumpadParenRight => "NumpadParenRight".to_string(),
        Code::NumpadStar => "NumpadStar".to_string(),
        Code::NumpadSubtract => "NumpadSubtract".to_string(),
        Code::Escape => "Escape".to_string(),
        Code::Fn => "Fn".to_string(),
        Code::FnLock => "FnLock".to_string(),
        Code::PrintScreen => "PrintScreen".to_string(),
        Code::ScrollLock => "ScrollLock".to_string(),
        Code::Pause => "Pause".to_string(),
        Code::BrowserBack => "BrowserBack".to_string(),
        Code::BrowserFavorites => "BrowserFavorites".to_string(),
        Code::BrowserForward => "BrowserForward".to_string(),
        Code::BrowserHome => "BrowserHome".to_string(),
        Code::BrowserRefresh => "BrowserRefresh".to_string(),
        Code::BrowserSearch => "BrowserSearch".to_string(),
        Code::BrowserStop => "BrowserStop".to_string(),
        Code::Eject => "Eject".to_string(),
        Code::LaunchApp1 => "LaunchApp1".to_string(),
        Code::LaunchApp2 => "LaunchApp2".to_string(),
        Code::LaunchMail => "LaunchMail".to_string(),
        Code::MediaPlayPause => "MediaPlayPause".to_string(),
        Code::MediaSelect => "MediaSelect".to_string(),
        Code::MediaStop => "MediaStop".to_string(),
        Code::MediaTrackNext => "MediaTrackNext".to_string(),
        Code::MediaTrackPrevious => "MediaTrackPrevious".to_string(),
        Code::Power => "Power".to_string(),
        Code::Sleep => "Sleep".to_string(),
        Code::AudioVolumeDown => "AudioVolumeDown".to_string(),
        Code::AudioVolumeMute => "AudioVolumeMute".to_string(),
        Code::AudioVolumeUp => "AudioVolumeUp".to_string(),
        Code::WakeUp => "WakeUp".to_string(),
        Code::Meta => "Meta".to_string(),
        Code::Hyper => "Hyper".to_string(),
        Code::Turbo => "Turbo".to_string(),
        Code::Abort => "Abort".to_string(),
        Code::Resume => "Resume".to_string(),
        Code::Suspend => "Suspend".to_string(),
        Code::Again => "Again".to_string(),
        Code::Copy => "Copy".to_string(),
        Code::Cut => "Cut".to_string(),
        Code::Find => "Find".to_string(),
        Code::Open => "Open".to_string(),
        Code::Paste => "Paste".to_string(),
        Code::Props => "Props".to_string(),
        Code::Select => "Select".to_string(),
        Code::Undo => "Undo".to_string(),
        Code::Hiragana => "Hiragana".to_string(),
        Code::Katakana => "Katakana".to_string(),
        Code::F1 => "F1".to_string(),
        Code::F2 => "F2".to_string(),
        Code::F3 => "F3".to_string(),
        Code::F4 => "F4".to_string(),
        Code::F5 => "F5".to_string(),
        Code::F6 => "F6".to_string(),
        Code::F7 => "F7".to_string(),
        Code::F8 => "F8".to_string(),
        Code::F9 => "F9".to_string(),
        Code::F10 => "F10".to_string(),
        Code::F11 => "F11".to_string(),
        Code::F12 => "F12".to_string(),
        Code::F13 => "F13".to_string(),
        Code::F14 => "F14".to_string(),
        Code::F15 => "F15".to_string(),
        Code::F16 => "F16".to_string(),
        Code::F17 => "F17".to_string(),
        Code::F18 => "F18".to_string(),
        Code::F19 => "F19".to_string(),
        Code::F20 => "F20".to_string(),
        Code::F21 => "F21".to_string(),
        Code::F22 => "F22".to_string(),
        Code::F23 => "F23".to_string(),
        Code::F24 => "F24".to_string(),
        Code::F25 => "F25".to_string(),
        Code::F26 => "F26".to_string(),
        Code::F27 => "F27".to_string(),
        Code::F28 => "F28".to_string(),
        Code::F29 => "F29".to_string(),
        Code::F30 => "F30".to_string(),
        Code::F31 => "F31".to_string(),
        Code::F32 => "F32".to_string(),
        Code::F33 => "F33".to_string(),
        Code::F34 => "F34".to_string(),
        Code::F35 => "F35".to_string(),
        _ => format!("{code:?}"),
    }
}

fn parse_code_storage_name(name: &str) -> Option<Code> {
    Some(match name {
        "Backquote" => Code::Backquote,
        "Backslash" => Code::Backslash,
        "BracketLeft" => Code::BracketLeft,
        "BracketRight" => Code::BracketRight,
        "Comma" => Code::Comma,
        "Digit0" => Code::Digit0,
        "Digit1" => Code::Digit1,
        "Digit2" => Code::Digit2,
        "Digit3" => Code::Digit3,
        "Digit4" => Code::Digit4,
        "Digit5" => Code::Digit5,
        "Digit6" => Code::Digit6,
        "Digit7" => Code::Digit7,
        "Digit8" => Code::Digit8,
        "Digit9" => Code::Digit9,
        "Equal" => Code::Equal,
        "IntlBackslash" => Code::IntlBackslash,
        "IntlRo" => Code::IntlRo,
        "IntlYen" => Code::IntlYen,
        "KeyA" => Code::KeyA,
        "KeyB" => Code::KeyB,
        "KeyC" => Code::KeyC,
        "KeyD" => Code::KeyD,
        "KeyE" => Code::KeyE,
        "KeyF" => Code::KeyF,
        "KeyG" => Code::KeyG,
        "KeyH" => Code::KeyH,
        "KeyI" => Code::KeyI,
        "KeyJ" => Code::KeyJ,
        "KeyK" => Code::KeyK,
        "KeyL" => Code::KeyL,
        "KeyM" => Code::KeyM,
        "KeyN" => Code::KeyN,
        "KeyO" => Code::KeyO,
        "KeyP" => Code::KeyP,
        "KeyQ" => Code::KeyQ,
        "KeyR" => Code::KeyR,
        "KeyS" => Code::KeyS,
        "KeyT" => Code::KeyT,
        "KeyU" => Code::KeyU,
        "KeyV" => Code::KeyV,
        "KeyW" => Code::KeyW,
        "KeyX" => Code::KeyX,
        "KeyY" => Code::KeyY,
        "KeyZ" => Code::KeyZ,
        "Minus" => Code::Minus,
        "Period" => Code::Period,
        "Quote" => Code::Quote,
        "Semicolon" => Code::Semicolon,
        "Slash" => Code::Slash,
        "AltLeft" => Code::AltLeft,
        "AltRight" => Code::AltRight,
        "Backspace" => Code::Backspace,
        "CapsLock" => Code::CapsLock,
        "ContextMenu" => Code::ContextMenu,
        "ControlLeft" => Code::ControlLeft,
        "ControlRight" => Code::ControlRight,
        "Enter" => Code::Enter,
        "SuperLeft" => Code::SuperLeft,
        "SuperRight" => Code::SuperRight,
        "ShiftLeft" => Code::ShiftLeft,
        "ShiftRight" => Code::ShiftRight,
        "Space" => Code::Space,
        "Tab" => Code::Tab,
        "Convert" => Code::Convert,
        "KanaMode" => Code::KanaMode,
        "Lang1" => Code::Lang1,
        "Lang2" => Code::Lang2,
        "Lang3" => Code::Lang3,
        "Lang4" => Code::Lang4,
        "Lang5" => Code::Lang5,
        "NonConvert" => Code::NonConvert,
        "Delete" => Code::Delete,
        "End" => Code::End,
        "Help" => Code::Help,
        "Home" => Code::Home,
        "Insert" => Code::Insert,
        "PageDown" => Code::PageDown,
        "PageUp" => Code::PageUp,
        "ArrowDown" => Code::ArrowDown,
        "ArrowLeft" => Code::ArrowLeft,
        "ArrowRight" => Code::ArrowRight,
        "ArrowUp" => Code::ArrowUp,
        "NumLock" => Code::NumLock,
        "Numpad0" => Code::Numpad0,
        "Numpad1" => Code::Numpad1,
        "Numpad2" => Code::Numpad2,
        "Numpad3" => Code::Numpad3,
        "Numpad4" => Code::Numpad4,
        "Numpad5" => Code::Numpad5,
        "Numpad6" => Code::Numpad6,
        "Numpad7" => Code::Numpad7,
        "Numpad8" => Code::Numpad8,
        "Numpad9" => Code::Numpad9,
        "NumpadAdd" => Code::NumpadAdd,
        "NumpadBackspace" => Code::NumpadBackspace,
        "NumpadClear" => Code::NumpadClear,
        "NumpadClearEntry" => Code::NumpadClearEntry,
        "NumpadComma" => Code::NumpadComma,
        "NumpadDecimal" => Code::NumpadDecimal,
        "NumpadDivide" => Code::NumpadDivide,
        "NumpadEnter" => Code::NumpadEnter,
        "NumpadEqual" => Code::NumpadEqual,
        "NumpadHash" => Code::NumpadHash,
        "NumpadMemoryAdd" => Code::NumpadMemoryAdd,
        "NumpadMemoryClear" => Code::NumpadMemoryClear,
        "NumpadMemoryRecall" => Code::NumpadMemoryRecall,
        "NumpadMemoryStore" => Code::NumpadMemoryStore,
        "NumpadMemorySubtract" => Code::NumpadMemorySubtract,
        "NumpadMultiply" => Code::NumpadMultiply,
        "NumpadParenLeft" => Code::NumpadParenLeft,
        "NumpadParenRight" => Code::NumpadParenRight,
        "NumpadStar" => Code::NumpadStar,
        "NumpadSubtract" => Code::NumpadSubtract,
        "Escape" => Code::Escape,
        "Fn" => Code::Fn,
        "FnLock" => Code::FnLock,
        "PrintScreen" => Code::PrintScreen,
        "ScrollLock" => Code::ScrollLock,
        "Pause" => Code::Pause,
        "BrowserBack" => Code::BrowserBack,
        "BrowserFavorites" => Code::BrowserFavorites,
        "BrowserForward" => Code::BrowserForward,
        "BrowserHome" => Code::BrowserHome,
        "BrowserRefresh" => Code::BrowserRefresh,
        "BrowserSearch" => Code::BrowserSearch,
        "BrowserStop" => Code::BrowserStop,
        "Eject" => Code::Eject,
        "LaunchApp1" => Code::LaunchApp1,
        "LaunchApp2" => Code::LaunchApp2,
        "LaunchMail" => Code::LaunchMail,
        "MediaPlayPause" => Code::MediaPlayPause,
        "MediaSelect" => Code::MediaSelect,
        "MediaStop" => Code::MediaStop,
        "MediaTrackNext" => Code::MediaTrackNext,
        "MediaTrackPrevious" => Code::MediaTrackPrevious,
        "Power" => Code::Power,
        "Sleep" => Code::Sleep,
        "AudioVolumeDown" => Code::AudioVolumeDown,
        "AudioVolumeMute" => Code::AudioVolumeMute,
        "AudioVolumeUp" => Code::AudioVolumeUp,
        "WakeUp" => Code::WakeUp,
        "Meta" => Code::Meta,
        "Hyper" => Code::Hyper,
        "Turbo" => Code::Turbo,
        "Abort" => Code::Abort,
        "Resume" => Code::Resume,
        "Suspend" => Code::Suspend,
        "Again" => Code::Again,
        "Copy" => Code::Copy,
        "Cut" => Code::Cut,
        "Find" => Code::Find,
        "Open" => Code::Open,
        "Paste" => Code::Paste,
        "Props" => Code::Props,
        "Select" => Code::Select,
        "Undo" => Code::Undo,
        "Hiragana" => Code::Hiragana,
        "Katakana" => Code::Katakana,
        "F1" => Code::F1,
        "F2" => Code::F2,
        "F3" => Code::F3,
        "F4" => Code::F4,
        "F5" => Code::F5,
        "F6" => Code::F6,
        "F7" => Code::F7,
        "F8" => Code::F8,
        "F9" => Code::F9,
        "F10" => Code::F10,
        "F11" => Code::F11,
        "F12" => Code::F12,
        "F13" => Code::F13,
        "F14" => Code::F14,
        "F15" => Code::F15,
        "F16" => Code::F16,
        "F17" => Code::F17,
        "F18" => Code::F18,
        "F19" => Code::F19,
        "F20" => Code::F20,
        "F21" => Code::F21,
        "F22" => Code::F22,
        "F23" => Code::F23,
        "F24" => Code::F24,
        "F25" => Code::F25,
        "F26" => Code::F26,
        "F27" => Code::F27,
        "F28" => Code::F28,
        "F29" => Code::F29,
        "F30" => Code::F30,
        "F31" => Code::F31,
        "F32" => Code::F32,
        "F33" => Code::F33,
        "F34" => Code::F34,
        "F35" => Code::F35,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_delete_selected_segment_labels_expose_unmodified_d_only() {
        assert_eq!(
            HotkeyBindings::default().labels_for(HotkeyAction::DeleteSelectedSegment),
            vec!["D".to_string()]
        );
    }

    #[test]
    fn persisted_keybinding_round_trips_code_and_modifiers() {
        let mut bindings = HotkeyBindings::default();
        bindings.set_override(
            HotkeyAction::DeleteSelectedSegment,
            KeyBinding::new(Modifiers::ALT, Code::KeyK),
        );

        let source = bindings.to_storage();
        let recovered = HotkeyBindings::from_storage(Some(&source));

        assert!(recovered.matches(
            HotkeyAction::DeleteSelectedSegment,
            Some(Physical::Code(Code::KeyK)),
            Modifiers::ALT,
        ));
        assert!(!recovered.matches(
            HotkeyAction::DeleteSelectedSegment,
            Some(Physical::Code(Code::KeyD)),
            Modifiers::empty(),
        ));
    }
}
