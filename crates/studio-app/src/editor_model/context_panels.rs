use std::collections::BTreeMap;

pub const SCRIPT_CONTEXT_STORAGE_KEY: &str = "tx:hide-script-context";
pub const ORIGINAL_SCRIPT_STORAGE_KEY: &str = "tx:hide-original-script";
pub const SUBMISSION_ID_COPY_RESET_DELAY_MS: u64 = 2_000;
pub const COLLAPSE_COPY_RESET_DELAY_MS: u64 = 0;

const SCRIPT_CONTEXT_FIELDS: [(&str, &str); 6] = [
    ("character", "Character"),
    ("physical", "Physical"),
    ("emotional", "Emotional"),
    ("speech", "Speech"),
    ("vocal", "Vocal"),
    ("preamble", "Preamble"),
];

pub type ScriptContext = BTreeMap<String, Option<String>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContextPanelTextDirection {
    Ltr,
    Rtl,
}

impl ContextPanelTextDirection {
    fn from_is_rtl(is_rtl: bool) -> Self {
        if is_rtl { Self::Rtl } else { Self::Ltr }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollapsiblePanelToggleIcon {
    Show,
    Hide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubmissionCopyIcon {
    Copy,
    Check,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollapsiblePanelInput {
    pub title: String,
    pub storage_key: String,
    pub hidden: bool,
    pub is_rtl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollapsiblePanelModel {
    pub title: String,
    pub storage_key: String,
    pub dir: ContextPanelTextDirection,
    pub hidden: bool,
    pub content_visible: bool,
    pub toggle_tooltip: String,
    pub toggle_aria_label: String,
    pub toggle_aria_expanded: bool,
    pub toggle_icon: CollapsiblePanelToggleIcon,
    pub toggle_is_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollapsiblePanelToggleResult {
    pub hidden: bool,
    pub effects: Vec<TtsPanelEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContextFieldModel {
    pub key: String,
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionIdModel {
    pub label: String,
    pub code_dir: ContextPanelTextDirection,
    pub submission_id: String,
    pub button_title: String,
    pub button_aria_label: String,
    pub icon: SubmissionCopyIcon,
    pub clipboard_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContextPanelInput {
    pub emotion: Option<String>,
    pub submission_id: Option<String>,
    pub context: Option<ScriptContext>,
    pub hidden: bool,
    pub is_rtl: bool,
    pub copied_submission_id: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptContextPanelModel {
    pub panel: CollapsiblePanelModel,
    pub emotion: Option<String>,
    pub fields: Vec<ScriptContextFieldModel>,
    pub submission: Option<SubmissionIdModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalScriptPanelInput {
    pub script: String,
    pub hidden: bool,
    pub is_rtl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalScriptPanelModel {
    pub panel: CollapsiblePanelModel,
    pub script: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmissionIdCopyResult {
    pub copied: bool,
    pub effects: Vec<TtsPanelEffect>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TtsPanelEffect {
    LocalStorageSet { key: String, value: String },
    ResetCopiedAfterMs { ms: u64 },
    ConsoleError { prefix: String, error: String },
}

pub fn read_stored_collapsed_state(value: Option<&str>) -> bool {
    value == Some("true")
}

pub fn build_collapsible_panel_model(input: CollapsiblePanelInput) -> CollapsiblePanelModel {
    let toggle_action = if input.hidden { "Show" } else { "Hide" };

    CollapsiblePanelModel {
        title: input.title.clone(),
        storage_key: input.storage_key,
        dir: ContextPanelTextDirection::from_is_rtl(input.is_rtl),
        hidden: input.hidden,
        content_visible: !input.hidden,
        toggle_tooltip: format!("{toggle_action} {}", input.title),
        toggle_aria_label: format!("{toggle_action} {}", input.title),
        toggle_aria_expanded: !input.hidden,
        toggle_icon: if input.hidden {
            CollapsiblePanelToggleIcon::Show
        } else {
            CollapsiblePanelToggleIcon::Hide
        },
        toggle_is_active: input.hidden,
    }
}

pub fn reduce_collapsible_panel_toggle(
    storage_key: impl Into<String>,
    hidden: bool,
) -> CollapsiblePanelToggleResult {
    let hidden = !hidden;
    CollapsiblePanelToggleResult {
        hidden,
        effects: vec![TtsPanelEffect::LocalStorageSet {
            key: storage_key.into(),
            value: hidden.to_string(),
        }],
    }
}

pub fn filter_script_context_fields(
    context: Option<&ScriptContext>,
) -> Vec<ScriptContextFieldModel> {
    let Some(context) = context else {
        return Vec::new();
    };

    SCRIPT_CONTEXT_FIELDS
        .iter()
        .filter_map(|(key, label)| {
            let value = context.get(*key)?.as_ref()?;
            if value.trim().is_empty() {
                return None;
            }

            Some(ScriptContextFieldModel {
                key: (*key).to_string(),
                label: (*label).to_string(),
                value: value.clone(),
            })
        })
        .collect()
}

pub fn build_submission_id_model(
    submission_id: impl Into<String>,
    copied: bool,
) -> SubmissionIdModel {
    let submission_id = submission_id.into();
    SubmissionIdModel {
        label: "Submission ID".to_string(),
        code_dir: ContextPanelTextDirection::Ltr,
        submission_id: submission_id.clone(),
        button_title: "Copy submission ID".to_string(),
        button_aria_label: "Copy submission ID".to_string(),
        icon: if copied {
            SubmissionCopyIcon::Check
        } else {
            SubmissionCopyIcon::Copy
        },
        clipboard_text: submission_id,
    }
}

pub fn resolve_submission_id_copy_success() -> SubmissionIdCopyResult {
    SubmissionIdCopyResult {
        copied: true,
        effects: vec![TtsPanelEffect::ResetCopiedAfterMs {
            ms: SUBMISSION_ID_COPY_RESET_DELAY_MS,
        }],
    }
}

pub fn resolve_submission_id_copy_failure(error: impl Into<String>) -> SubmissionIdCopyResult {
    SubmissionIdCopyResult {
        copied: false,
        effects: vec![TtsPanelEffect::ConsoleError {
            prefix: "Failed to copy submission ID:".to_string(),
            error: error.into(),
        }],
    }
}

pub fn build_script_context_panel_model(
    input: ScriptContextPanelInput,
) -> Option<ScriptContextPanelModel> {
    let has_emotion = input
        .emotion
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    let has_submission_id = input
        .submission_id
        .as_ref()
        .is_some_and(|value| !value.is_empty());

    if !has_emotion && input.context.is_none() && !has_submission_id {
        return None;
    }

    let fields = filter_script_context_fields(input.context.as_ref());
    let submission = input
        .submission_id
        .filter(|submission_id| !submission_id.is_empty())
        .map(|submission_id| build_submission_id_model(submission_id, input.copied_submission_id));

    Some(ScriptContextPanelModel {
        panel: build_collapsible_panel_model(CollapsiblePanelInput {
            title: "Script Context".to_string(),
            storage_key: SCRIPT_CONTEXT_STORAGE_KEY.to_string(),
            hidden: input.hidden,
            is_rtl: input.is_rtl,
        }),
        emotion: input.emotion,
        fields,
        submission,
    })
}

pub fn build_original_script_panel_model(
    input: OriginalScriptPanelInput,
) -> OriginalScriptPanelModel {
    OriginalScriptPanelModel {
        panel: build_collapsible_panel_model(CollapsiblePanelInput {
            title: "Original Script".to_string(),
            storage_key: ORIGINAL_SCRIPT_STORAGE_KEY.to_string(),
            hidden: input.hidden,
            is_rtl: input.is_rtl,
        }),
        script: input.script,
    }
}
