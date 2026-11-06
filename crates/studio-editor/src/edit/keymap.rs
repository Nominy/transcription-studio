//! Editor hotkeys: semantic [`EditorAction`]s, default bindings as gpui keystroke strings
//! (`"secondary-z"`, `"alt-shift-up"`, `"space"`), the focus context each binding lives in,
//! custom-binding persistence (JSON), conflict detection and keystroke → action routing.
//!
//! `secondary-` is gpui's platform modifier alias (cmd on macOS, ctrl elsewhere).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::structure::AuditionKey;
use crate::i18n::Text;

/// Preference-store key the custom bindings JSON is saved under.
pub const HOTKEY_STORAGE_KEY: &str = "studio.native.hotkeyBindings.v1";

/// Where a binding is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyContext {
    /// Everywhere, including while a text input has focus.
    Global,
    /// Only while a transcript row's text editor has focus.
    TranscriptText,
    /// Only while no text input has focus (timeline / transcript grid / workspace).
    Timeline,
    /// Like [`KeyContext::Timeline`], but only while a cut-preview overlay exists; these
    /// bindings take precedence over timeline bindings on the same keystroke.
    CutPreview,
    /// Wherever [`KeyContext::Timeline`] or [`KeyContext::TranscriptText`] applies (not in
    /// dialog inputs); those contexts take precedence on the same keystroke.
    Transcript,
    /// Only while a non-transcript input has focus (add-region dialog fields, find bar,
    /// lane zoom input).
    Dialog,
}

impl KeyContext {
    /// Whether bindings in `self` and `other` can fire for the same key press.
    pub fn overlaps(self, other: KeyContext) -> bool {
        use KeyContext::*;
        match (self, other) {
            (Global, _) | (_, Global) => true,
            (Transcript, Dialog) | (Dialog, Transcript) => false,
            (Transcript, _) | (_, Transcript) => true,
            (CutPreview, Timeline) | (Timeline, CutPreview) => false,
            (left, right) => left == right,
        }
    }
}

macro_rules! editor_actions {
    ($($variant:ident),* $(,)?) => {
        /// Every keyboard-triggerable editor action.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub enum EditorAction { $($variant),* }

        impl EditorAction {
            pub const ALL: &'static [EditorAction] = &[$(EditorAction::$variant),*];
        }
    };
}

editor_actions! {
    PlayPause,
    PreviousSegment,
    NextSegment,
    FocusPreviousSegment,
    FocusNextSegment,
    Undo,
    Redo,
    OpenFind,
    FindNext,
    FindPrevious,
    DeleteSelectedSegment,
    BackToProjects,
    ToggleOverlay,
    SwitchGhostCursorLane,
    CreateRegionSubmit,
    CreateRegionEscape,
    ResetSpeakerWorkflow,
    SwitchSpeakerWorkflowLane1,
    SwitchSpeakerWorkflowLane2,
    SwapSplitPanes,
    RewindPlaybackOneSecond,
    MoveTextBeforeCaretToPreviousSegment,
    MoveTextAfterCaretToNextSegment,
    IncreasePlaybackSpeed,
    DecreasePlaybackSpeed,
    MergeSelectedSegmentAbove,
    MergeSelectedSegmentBelow,
    TrimCurrentSegmentToAudio,
    TrimAllVisibleSegmentsToAudio,
    AutoInsertSegmentAtCaret,
    AutoSegmentVisibleSilences,
    SplitAtPlayhead,
    CommitCutPreviewSmartSplit,
    CommitCutPreviewPlainSplit,
    LoopCutPreviewOverlay,
    EscapePressed,
    ActivateDefaultSurfaceAction,
    TextUndo,
    TextRedo,
    TranscriptEscape,
    CompletionPrevious,
    CompletionNext,
    CompletionAccept,
    CompletionDismiss,
}

/// Static description of an action: hotkeys-dialog section/description, context and defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDef {
    pub action: EditorAction,
    pub section: Text,
    pub description: Text,
    pub context: KeyContext,
    pub defaults: &'static [&'static str],
    /// Whether the user may rebind it (fixed text-editing keys are not configurable).
    pub configurable: bool,
}

const fn def(
    action: EditorAction,
    section: Text,
    description: Text,
    context: KeyContext,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        action,
        section,
        description,
        context,
        defaults,
        configurable: true,
    }
}

const fn fixed(
    action: EditorAction,
    description: Text,
    defaults: &'static [&'static str],
) -> ActionDef {
    ActionDef {
        action,
        section: TEXT_EDITING,
        description,
        context: KeyContext::TranscriptText,
        defaults,
        configurable: false,
    }
}

const fn t(en: &'static str, ru: &'static str) -> Text {
    Text::new(en, ru)
}

use EditorAction as A;
use KeyContext::{CutPreview, Dialog, Global, Timeline, Transcript, TranscriptText};

const GLOBAL: Text = t("General", "Общие");
const TIMELINE: Text = t("Timeline", "Таймлайн");
const NAV: Text = t("Transcript navigation", "Навигация по расшифровке");
const DIALOG: Text = t("Dialogs", "Диалоги");
const HELPER: Text = t("Helper editing", "Редактирование Helper");
const TIMELINE_EDITING: Text = t("Timeline editing", "Редактирование таймлайна");
const CUT: Text = t("Cut selection", "Выделение для разреза");
const TEXT_EDITING: Text = t("Text editing", "Редактирование текста");

/// All actions in hotkeys-dialog order.
pub const ACTION_DEFS: &[ActionDef] = &[
    def(
        A::PlayPause,
        GLOBAL,
        t("Play / pause", "Воспроизвести / пауза"),
        Timeline,
        &["space"],
    ),
    def(
        A::PreviousSegment,
        GLOBAL,
        t("Go to previous segment", "К предыдущему сегменту"),
        Timeline,
        &["left"],
    ),
    def(
        A::NextSegment,
        GLOBAL,
        t("Go to next segment", "К следующему сегменту"),
        Timeline,
        &["right"],
    ),
    def(
        A::Undo,
        GLOBAL,
        t("Undo", "Отменить"),
        Timeline,
        &["secondary-z"],
    ),
    def(
        A::Redo,
        GLOBAL,
        t("Redo", "Повторить"),
        Timeline,
        &["secondary-shift-z", "secondary-y"],
    ),
    def(
        A::OpenFind,
        GLOBAL,
        t("Find in transcript", "Поиск в расшифровке"),
        Global,
        &["secondary-f"],
    ),
    def(
        A::FindNext,
        GLOBAL,
        t("Next match", "Следующее совпадение"),
        Global,
        &["f3", "secondary-g"],
    ),
    def(
        A::FindPrevious,
        GLOBAL,
        t("Previous match", "Предыдущее совпадение"),
        Global,
        &["shift-f3", "secondary-shift-g"],
    ),
    def(
        A::DeleteSelectedSegment,
        GLOBAL,
        t("Delete the selected segment", "Удалить выбранный сегмент"),
        Timeline,
        &["d"],
    ),
    def(
        A::BackToProjects,
        GLOBAL,
        t("Back to Start", "На начальный экран"),
        Timeline,
        &["alt-left"],
    ),
    def(
        A::ToggleOverlay,
        TIMELINE,
        t(
            "Show or hide the 1-second overlay",
            "Показать или скрыть секундную сетку",
        ),
        Timeline,
        &["t"],
    ),
    def(
        A::SwitchGhostCursorLane,
        NAV,
        t(
            "Switch the ghost cursor to the other track",
            "Переключить призрачный курсор на другую дорожку",
        ),
        Transcript,
        &["tab"],
    ),
    def(
        A::FocusPreviousSegment,
        NAV,
        t(
            "Edit the previous segment's text",
            "Редактировать текст предыдущего сегмента",
        ),
        Timeline,
        &["shift-left"],
    ),
    def(
        A::FocusNextSegment,
        NAV,
        t(
            "Edit the next segment's text",
            "Редактировать текст следующего сегмента",
        ),
        Timeline,
        &["shift-right"],
    ),
    def(
        A::CreateRegionSubmit,
        DIALOG,
        t("Add the new segment", "Добавить новый сегмент"),
        Dialog,
        &["secondary-enter"],
    ),
    def(
        A::CreateRegionEscape,
        DIALOG,
        t("Cancel the new segment", "Отменить новый сегмент"),
        Dialog,
        &["escape"],
    ),
    def(
        A::EscapePressed,
        DIALOG,
        t(
            "Close the dialog or clear the cut selection",
            "Закрыть диалог или снять выделение",
        ),
        Timeline,
        &["escape"],
    ),
    def(
        A::ActivateDefaultSurfaceAction,
        DIALOG,
        t("Confirm", "Подтвердить"),
        Timeline,
        &["enter"],
    ),
    def(
        A::ResetSpeakerWorkflow,
        HELPER,
        t("Reset the speaker tracks", "Сбросить дорожки спикеров"),
        Global,
        &["alt-`"],
    ),
    def(
        A::SwitchSpeakerWorkflowLane1,
        HELPER,
        t(
            "Switch to speaker track 1",
            "Переключиться на дорожку спикера 1",
        ),
        Global,
        &["alt-1"],
    ),
    def(
        A::SwitchSpeakerWorkflowLane2,
        HELPER,
        t(
            "Switch to speaker track 2",
            "Переключиться на дорожку спикера 2",
        ),
        Global,
        &["alt-2"],
    ),
    // Helper `split.swapPanes`: works from the row being edited too (Helper claims Ctrl+I
    // before the rich-text italic handler).
    def(
        A::SwapSplitPanes,
        HELPER,
        t("Swap the speaker panes", "Поменять панели спикеров местами"),
        Global,
        &["secondary-i"],
    ),
    def(
        A::RewindPlaybackOneSecond,
        HELPER,
        t("Rewind 1 second", "Назад на 1 секунду"),
        Global,
        &["alt-x"],
    ),
    def(
        A::MoveTextBeforeCaretToPreviousSegment,
        HELPER,
        t(
            "Move text before the cursor to the previous segment",
            "Перенести текст до курсора в предыдущий сегмент",
        ),
        TranscriptText,
        &["alt-["],
    ),
    def(
        A::MoveTextAfterCaretToNextSegment,
        HELPER,
        t(
            "Move text after the cursor to the next segment",
            "Перенести текст после курсора в следующий сегмент",
        ),
        TranscriptText,
        &["alt-]"],
    ),
    def(
        A::IncreasePlaybackSpeed,
        TIMELINE_EDITING,
        t("Faster", "Быстрее"),
        Timeline,
        &["shift-1"],
    ),
    def(
        A::DecreasePlaybackSpeed,
        TIMELINE_EDITING,
        t("Slower", "Медленнее"),
        Timeline,
        &["shift-2"],
    ),
    def(
        A::MergeSelectedSegmentAbove,
        TIMELINE_EDITING,
        t(
            "Merge with previous segment",
            "Объединить с предыдущим сегментом",
        ),
        Timeline,
        &["alt-shift-up"],
    ),
    def(
        A::MergeSelectedSegmentBelow,
        TIMELINE_EDITING,
        t(
            "Merge with next segment",
            "Объединить со следующим сегментом",
        ),
        Timeline,
        &["alt-shift-down"],
    ),
    def(
        A::TrimCurrentSegmentToAudio,
        TIMELINE_EDITING,
        t(
            "Trim the segment to the speech in it",
            "Обрезать сегмент по речи",
        ),
        Timeline,
        &["alt-r"],
    ),
    def(
        A::TrimAllVisibleSegmentsToAudio,
        TIMELINE_EDITING,
        t(
            "Trim all visible segments to their speech",
            "Обрезать все видимые сегменты по речи",
        ),
        Timeline,
        &["alt-shift-r"],
    ),
    def(
        A::AutoInsertSegmentAtCaret,
        TIMELINE_EDITING,
        t(
            "Add a segment around the nearest speech",
            "Добавить сегмент вокруг ближайшей речи",
        ),
        Timeline,
        &["alt-c"],
    ),
    def(
        A::AutoSegmentVisibleSilences,
        TIMELINE_EDITING,
        t(
            "Split visible segments at silences",
            "Разделить видимые сегменты по паузам",
        ),
        Timeline,
        &["alt-shift-s"],
    ),
    // Timeline-context `s` yields to the cut-preview `s` while a cut selection exists.
    def(
        A::SplitAtPlayhead,
        TIMELINE_EDITING,
        t(
            "Split the selected segment at the playhead",
            "Разделить выбранный сегмент по курсору воспроизведения",
        ),
        Timeline,
        &["s"],
    ),
    def(
        A::CommitCutPreviewSmartSplit,
        CUT,
        t(
            "Split here, splitting the text by words",
            "Разрезать здесь, разделив текст по словам",
        ),
        CutPreview,
        &["s"],
    ),
    def(
        A::CommitCutPreviewPlainSplit,
        CUT,
        t(
            "Split here, keeping the text",
            "Разрезать здесь, сохранив текст",
        ),
        CutPreview,
        &["shift-s"],
    ),
    def(
        A::LoopCutPreviewOverlay,
        CUT,
        t(
            "Loop the selection (again to stop)",
            "Повторять выделение (ещё раз — стоп)",
        ),
        CutPreview,
        &["l"],
    ),
    fixed(
        A::TextUndo,
        t("Undo text edit", "Отменить правку текста"),
        &["secondary-z"],
    ),
    fixed(
        A::TextRedo,
        t("Redo text edit", "Повторить правку текста"),
        &["secondary-shift-z", "secondary-y"],
    ),
    fixed(
        A::TranscriptEscape,
        t(
            "Leave the segment and play / pause",
            "Выйти из сегмента и воспроизвести / пауза",
        ),
        &["escape"],
    ),
    fixed(
        A::CompletionPrevious,
        t("Previous suggestion", "Предыдущая подсказка"),
        &["up"],
    ),
    fixed(
        A::CompletionNext,
        t("Next suggestion", "Следующая подсказка"),
        &["down"],
    ),
    fixed(
        A::CompletionAccept,
        t("Accept suggestion", "Принять подсказку"),
        &["enter", "tab"],
    ),
    fixed(
        A::CompletionDismiss,
        t("Hide suggestions", "Скрыть подсказки"),
        &["escape"],
    ),
];

impl EditorAction {
    /// Persisted id (`"playPause"`, `"deleteSelectedSegment"`, ...).
    pub fn id(self) -> String {
        match serde_json::to_value(self) {
            Ok(Value::String(id)) => id,
            _ => format!("{self:?}"),
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        serde_json::from_value(Value::String(id.to_string())).ok()
    }

    pub fn def(self) -> &'static ActionDef {
        ACTION_DEFS
            .iter()
            .find(|def| def.action == self)
            .expect("every EditorAction has an ActionDef")
    }

    /// Completion keys only apply while the completion list is open.
    pub fn requires_completions(self) -> bool {
        matches!(
            self,
            A::CompletionPrevious | A::CompletionNext | A::CompletionAccept | A::CompletionDismiss
        )
    }
}

/// A normalized keystroke (`secondary` kept symbolic until [`KeyChord::resolved`]).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyChord {
    pub secondary: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub platform: bool,
    /// Lowercase gpui key name (`"a"`, `"space"`, `"left"`, `"f3"`, `"["`).
    pub key: String,
}

impl KeyChord {
    /// Parses a gpui keystroke string (`ctrl-`, `alt-`, `shift-`, `cmd-`/`super-`/`win-`,
    /// `secondary-` modifiers in any order). A trailing uppercase ASCII letter implies shift.
    pub fn parse(source: &str) -> Option<Self> {
        let source = source.trim();
        if source.is_empty() {
            return None;
        }
        let (modifier_part, key) = if source == "-" {
            ("", "-")
        } else if let Some(prefix) = source.strip_suffix("--") {
            (prefix, "-")
        } else {
            match source.rsplit_once('-') {
                Some((prefix, key)) => (prefix, key),
                None => ("", source),
            }
        };
        if key.is_empty() {
            return None;
        }
        let mut chord = KeyChord {
            secondary: false,
            ctrl: false,
            alt: false,
            shift: false,
            platform: false,
            key: String::new(),
        };
        for modifier in modifier_part.split('-').filter(|m| !m.is_empty()) {
            match modifier.to_ascii_lowercase().as_str() {
                "secondary" => chord.secondary = true,
                "ctrl" | "control" => chord.ctrl = true,
                "alt" => chord.alt = true,
                "shift" => chord.shift = true,
                "cmd" | "super" | "win" | "platform" => chord.platform = true,
                _ => return None,
            }
        }
        if key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase() {
            chord.shift = true;
        }
        chord.key = key.to_ascii_lowercase();
        Some(chord)
    }

    /// Replaces `secondary` with the platform's concrete modifier.
    pub fn resolved(&self, mac: bool) -> KeyChord {
        let mut chord = self.clone();
        if chord.secondary {
            chord.secondary = false;
            if mac {
                chord.platform = true;
            } else {
                chord.ctrl = true;
            }
        }
        chord
    }

    /// Human label for the hotkeys dialog (`Ctrl+Shift+Z`, `Alt+←`).
    pub fn label(&self, mac: bool) -> String {
        let chord = self.resolved(mac);
        let mut parts: Vec<String> = Vec::new();
        if chord.ctrl {
            parts.push("Ctrl".into());
        }
        if chord.platform {
            parts.push(if mac { "Cmd" } else { "Win" }.into());
        }
        if chord.alt {
            parts.push("Alt".into());
        }
        if chord.shift {
            parts.push("Shift".into());
        }
        parts.push(key_label(&chord.key));
        parts.join("+")
    }
}

impl fmt::Display for KeyChord {
    /// Canonical gpui keystroke string.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (enabled, name) in [
            (self.secondary, "secondary-"),
            (self.ctrl, "ctrl-"),
            (self.alt, "alt-"),
            (self.shift, "shift-"),
            (self.platform, "cmd-"),
        ] {
            if enabled {
                f.write_str(name)?;
            }
        }
        f.write_str(&self.key)
    }
}

fn key_label(key: &str) -> String {
    match key {
        "left" => "←".into(),
        "right" => "→".into(),
        "up" => "↑".into(),
        "down" => "↓".into(),
        "space" => "Space".into(),
        "tab" => "Tab".into(),
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "delete" => "Delete".into(),
        other if other.len() == 1 => other.to_ascii_uppercase(),
        other if other.starts_with('f') && other[1..].bytes().all(|b| b.is_ascii_digit()) => {
            other.to_ascii_uppercase()
        }
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// Where keyboard focus is when a key is pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyFocus {
    pub target: FocusTarget,
    /// A cut-preview overlay exists (enables [`KeyContext::CutPreview`] bindings).
    pub cut_preview_active: bool,
    /// The completion list is open in the focused transcript row.
    pub completions_open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusTarget {
    /// No text input focused.
    #[default]
    Workspace,
    /// A transcript row's text editor.
    TranscriptText,
    /// Any other text input (dialog field, find bar, numeric input).
    Input,
}

/// Rejected custom binding.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeymapError {
    #[error("`{0}` is not a valid keystroke")]
    InvalidKeystroke(String),
    #[error("this shortcut cannot be changed")]
    NotConfigurable,
}

/// Two actions that fire on the same key press.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConflict {
    pub chord: KeyChord,
    pub first: EditorAction,
    pub second: EditorAction,
}

/// Default bindings plus user overrides (one chord per overridden action).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Keymap {
    overrides: BTreeMap<EditorAction, KeyChord>,
}

impl Keymap {
    /// Loads persisted overrides; unknown actions, unparsable keys and non-configurable actions
    /// are ignored. Also accepts the legacy `{"action":{"code":"KeyK","modifiers":["alt"]}}`
    /// format written by the iced app.
    pub fn from_json(source: &str) -> Self {
        let Ok(Value::Object(root)) = serde_json::from_str::<Value>(source) else {
            return Self::default();
        };
        let overrides = root
            .into_iter()
            .filter_map(|(id, value)| {
                let action = EditorAction::from_id(&id).filter(|a| a.def().configurable)?;
                let chord = match &value {
                    Value::String(keystroke) => KeyChord::parse(keystroke),
                    Value::Object(_) => legacy_chord(&value),
                    _ => None,
                }?;
                Some((action, chord))
            })
            .collect();
        Self { overrides }
    }

    /// Serializes overrides as `{"actionId":"keystroke"}`.
    pub fn to_json(&self) -> String {
        let root: Map<String, Value> = self
            .overrides
            .iter()
            .map(|(action, chord)| (action.id(), Value::String(chord.to_string())))
            .collect();
        Value::Object(root).to_string()
    }

    /// Effective chords for `action` (override, or defaults).
    pub fn bindings(&self, action: EditorAction) -> Vec<KeyChord> {
        if let Some(chord) = self.overrides.get(&action) {
            return vec![chord.clone()];
        }
        action
            .def()
            .defaults
            .iter()
            .filter_map(|keystroke| KeyChord::parse(keystroke))
            .collect()
    }

    /// Effective keystroke strings for `action` (for gpui `KeyBinding::new`).
    pub fn keystrokes(&self, action: EditorAction) -> Vec<String> {
        self.bindings(action)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// Display labels for the hotkeys dialog.
    pub fn labels(&self, action: EditorAction, mac: bool) -> Vec<String> {
        self.bindings(action).iter().map(|c| c.label(mac)).collect()
    }

    pub fn is_overridden(&self, action: EditorAction) -> bool {
        self.overrides.contains_key(&action)
    }

    /// Replaces the bindings of a configurable action with one keystroke.
    pub fn set_override(
        &mut self,
        action: EditorAction,
        keystroke: &str,
    ) -> Result<(), KeymapError> {
        if !action.def().configurable {
            return Err(KeymapError::NotConfigurable);
        }
        let chord = KeyChord::parse(keystroke)
            .ok_or_else(|| KeymapError::InvalidKeystroke(keystroke.to_string()))?;
        self.overrides.insert(action, chord);
        Ok(())
    }

    /// Restores the default bindings of `action`.
    pub fn reset_override(&mut self, action: EditorAction) {
        self.overrides.remove(&action);
    }

    /// Actions that would also fire if `action` were bound to `keystroke`.
    pub fn conflicts_with(
        &self,
        action: EditorAction,
        keystroke: &str,
        mac: bool,
    ) -> Vec<EditorAction> {
        let Some(chord) = KeyChord::parse(keystroke).map(|c| c.resolved(mac)) else {
            return Vec::new();
        };
        let context = action.def().context;
        EditorAction::ALL
            .iter()
            .copied()
            .filter(|other| {
                *other != action
                    && context.overlaps(other.def().context)
                    && other.requires_completions() == action.requires_completions()
            })
            .filter(|other| {
                self.bindings(*other)
                    .iter()
                    .any(|c| c.resolved(mac) == chord)
            })
            .collect()
    }

    /// Every pair of actions sharing a key press in overlapping contexts.
    pub fn conflicts(&self, mac: bool) -> Vec<KeyConflict> {
        let mut conflicts = Vec::new();
        for (index, first) in EditorAction::ALL.iter().copied().enumerate() {
            for second in EditorAction::ALL[index + 1..].iter().copied() {
                if !first.def().context.overlaps(second.def().context)
                    || (first.requires_completions() != second.requires_completions())
                {
                    continue;
                }
                let second_chords: Vec<KeyChord> = self
                    .bindings(second)
                    .iter()
                    .map(|c| c.resolved(mac))
                    .collect();
                for chord in self.bindings(first).iter().map(|c| c.resolved(mac)) {
                    if second_chords.contains(&chord) {
                        conflicts.push(KeyConflict {
                            chord,
                            first,
                            second,
                        });
                    }
                }
            }
        }
        conflicts
    }

    /// Routes a key press (gpui keystroke string) to an action given the focus.
    /// Precedence: tag-suggestion keys > focused-input context > cut preview > timeline >
    /// transcript > global.
    pub fn resolve(&self, keystroke: &str, focus: KeyFocus, mac: bool) -> Option<EditorAction> {
        let chord = KeyChord::parse(keystroke)?.resolved(mac);
        let contexts: &[KeyContext] = match focus.target {
            FocusTarget::TranscriptText => &[
                KeyContext::TranscriptText,
                KeyContext::Transcript,
                KeyContext::Global,
            ],
            FocusTarget::Input => &[KeyContext::Dialog, KeyContext::Global],
            FocusTarget::Workspace if focus.cut_preview_active => &[
                KeyContext::CutPreview,
                KeyContext::Timeline,
                KeyContext::Transcript,
                KeyContext::Global,
            ],
            FocusTarget::Workspace => &[
                KeyContext::Timeline,
                KeyContext::Transcript,
                KeyContext::Global,
            ],
        };
        let matches = |action: &EditorAction| {
            self.bindings(*action)
                .iter()
                .any(|binding| binding.resolved(mac) == chord)
        };
        if focus.target == FocusTarget::TranscriptText
            && focus.completions_open
            && let Some(action) = EditorAction::ALL
                .iter()
                .filter(|a| a.requires_completions())
                .find(|a| matches(a))
        {
            return Some(*action);
        }
        contexts.iter().find_map(|context| {
            EditorAction::ALL
                .iter()
                .filter(|a| !a.requires_completions() && a.def().context == *context)
                .find(|a| matches(a))
                .copied()
        })
    }
}

/// Held boundary-audition key for an unmodified Q/W/E/R press outside text inputs.
pub fn audition_key_for(keystroke: &str) -> Option<AuditionKey> {
    let chord = KeyChord::parse(keystroke)?;
    if chord.secondary || chord.ctrl || chord.alt || chord.platform {
        return None;
    }
    match chord.key.as_str() {
        "q" => Some(AuditionKey::BeforeStart),
        "w" => Some(AuditionKey::AfterStart),
        "e" => Some(AuditionKey::BeforeEnd),
        "r" => Some(AuditionKey::AfterEnd),
        _ => None,
    }
}

fn legacy_chord(value: &Value) -> Option<KeyChord> {
    let code = value.get("code")?.as_str()?;
    let key = if let Some(letter) = code.strip_prefix("Key") {
        letter.to_ascii_lowercase()
    } else if let Some(digit) = code.strip_prefix("Digit") {
        digit.to_string()
    } else if let Some(arrow) = code.strip_prefix("Arrow") {
        arrow.to_ascii_lowercase()
    } else if code.len() <= 3
        && code.starts_with('F')
        && code[1..].bytes().all(|b| b.is_ascii_digit())
    {
        code.to_ascii_lowercase()
    } else {
        match code {
            "Space" => "space",
            "Tab" => "tab",
            "Enter" => "enter",
            "Escape" => "escape",
            "Backspace" => "backspace",
            "Delete" => "delete",
            "Insert" => "insert",
            "Home" => "home",
            "End" => "end",
            "PageUp" => "pageup",
            "PageDown" => "pagedown",
            "Backquote" => "`",
            "Minus" => "-",
            "Equal" => "=",
            "BracketLeft" => "[",
            "BracketRight" => "]",
            "Backslash" => "\\",
            "Semicolon" => ";",
            "Quote" => "'",
            "Comma" => ",",
            "Period" => ".",
            "Slash" => "/",
            _ => return None,
        }
        .to_string()
    };
    let mut chord = KeyChord {
        secondary: false,
        ctrl: false,
        alt: false,
        shift: false,
        platform: false,
        key,
    };
    for name in value.get("modifiers")?.as_array()? {
        match name.as_str()? {
            "shift" => chord.shift = true,
            "control" => chord.ctrl = true,
            "alt" => chord.alt = true,
            "logo" => chord.platform = true,
            _ => return None,
        }
    }
    Some(chord)
}
