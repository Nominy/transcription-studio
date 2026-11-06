use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityId(&'static str);

impl CapabilityId {
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityPack {
    Helper,
    Reviewer,
    Drafting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityServiceFamily {
    Babel,
    Reviewer,
    Drafting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityHotkey {
    pub keys: &'static [&'static str],
    pub description: &'static str,
}

pub type CapabilityHotkeySection = (&'static str, Vec<(Vec<&'static str>, &'static str)>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub pack: CapabilityPack,
    pub label: &'static str,
    pub description: &'static str,
    pub default_enabled: bool,
    pub commands: &'static [&'static str],
    pub hotkeys: &'static [CapabilityHotkey],
    pub service_family: Option<CapabilityServiceFamily>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilitySettings {
    overrides: BTreeMap<&'static str, bool>,
}

impl CapabilitySettings {
    pub fn from_overrides(overrides: impl IntoIterator<Item = (&'static str, bool)>) -> Self {
        Self {
            overrides: overrides.into_iter().collect(),
        }
    }

    fn enabled_for(&self, descriptor: &CapabilityDescriptor) -> bool {
        self.overrides
            .get(descriptor.id.as_str())
            .copied()
            .unwrap_or(descriptor.default_enabled)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapabilityRegistry {
    settings: CapabilitySettings,
}

impl CapabilityRegistry {
    pub fn with_settings(settings: CapabilitySettings) -> Self {
        Self { settings }
    }

    pub fn descriptors(&self) -> &'static [CapabilityDescriptor] {
        CAPABILITY_DESCRIPTORS
    }

    pub fn descriptor(&self, id: CapabilityId) -> Option<&'static CapabilityDescriptor> {
        CAPABILITY_DESCRIPTORS
            .iter()
            .find(|descriptor| descriptor.id == id)
    }

    pub fn is_enabled(&self, id: CapabilityId) -> bool {
        self.descriptor(id)
            .is_some_and(|descriptor| self.settings.enabled_for(descriptor))
    }

    pub fn enabled_descriptors(&self) -> impl Iterator<Item = &'static CapabilityDescriptor> + '_ {
        CAPABILITY_DESCRIPTORS
            .iter()
            .filter(|descriptor| self.settings.enabled_for(descriptor))
    }

    pub fn hotkey_sections(&self) -> Vec<CapabilityHotkeySection> {
        let mut sections = RECOVERED_BABEL_HOTKEY_SECTIONS
            .iter()
            .map(|section| {
                (
                    section.title,
                    section
                        .rows
                        .iter()
                        .map(|row| (row.keys.to_vec(), row.description))
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        let helper_rows = self
            .enabled_descriptors()
            .filter(|descriptor| descriptor.pack == CapabilityPack::Helper)
            .flat_map(|descriptor| descriptor.hotkeys.iter())
            .map(|row| (row.keys.to_vec(), row.description))
            .collect::<Vec<_>>();
        if !helper_rows.is_empty() {
            sections.push(("Helper Extensions", helper_rows));
        }
        sections
    }

    pub fn service_family_for(&self, id: CapabilityId) -> Option<CapabilityServiceFamily> {
        self.descriptor(id)
            .and_then(|descriptor| descriptor.service_family)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StaticHotkeySection {
    title: &'static str,
    rows: &'static [CapabilityHotkey],
}

const GLOBAL_CONTROLS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["Space"],
        description: "Play/Pause (restores ghost cursor like Escape)",
    },
    CapabilityHotkey {
        keys: &["\u{2190}"],
        description: "Seek To Previous Segment",
    },
    CapabilityHotkey {
        keys: &["\u{2192}"],
        description: "Seek To Next Segment",
    },
    CapabilityHotkey {
        keys: &["Cmd/Ctrl", "Z"],
        description: "Undo",
    },
    CapabilityHotkey {
        keys: &["Cmd/Ctrl", "Shift", "Z"],
        description: "Redo",
    },
    CapabilityHotkey {
        keys: &["Cmd/Ctrl", "F"],
        description: "Find in transcript",
    },
    CapabilityHotkey {
        keys: &["F3", "Or", "Cmd/Ctrl", "G"],
        description: "Find next match",
    },
    CapabilityHotkey {
        keys: &["Shift", "F3", "Or", "Shift", "Cmd/Ctrl", "G"],
        description: "Find previous match",
    },
    CapabilityHotkey {
        keys: &["Cmd/Ctrl", "J"],
        description: "Confirm/undo warning on active segment",
    },
    CapabilityHotkey {
        keys: &["Hold Shift", "Scroll"],
        description: "Side-scroll",
    },
    CapabilityHotkey {
        keys: &["Pinch", "Or", "Cmd/Ctrl", "Scroll"],
        description: "Zoom in/out",
    },
];

const OVERLAY_CONTROLS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["T"],
        description: "Toggle 1-second overlay",
    },
    CapabilityHotkey {
        keys: &["Drag"],
        description: "Move overlay to check different positions",
    },
];

const SEGMENT_CONTROLS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["Click"],
        description: "Select segment",
    },
    CapabilityHotkey {
        keys: &["Click", "Click"],
        description: "Play segment",
    },
    CapabilityHotkey {
        keys: &["Hold Shift", "Click"],
        description: "Play segment and continue",
    },
    CapabilityHotkey {
        keys: &["Cmd/Ctrl", "Click"],
        description: "Split segment at click position",
    },
    CapabilityHotkey {
        keys: &["Hold Q", "Click"],
        description: "Play 500ms before segment start",
    },
    CapabilityHotkey {
        keys: &["Hold W", "Click"],
        description: "Play first 500ms of segment",
    },
    CapabilityHotkey {
        keys: &["Hold E", "Click"],
        description: "Play last 500ms of segment",
    },
    CapabilityHotkey {
        keys: &["Hold R", "Click"],
        description: "Play 500ms after segment end",
    },
];

const RECOVERED_BABEL_HOTKEY_SECTIONS: &[StaticHotkeySection] = &[
    StaticHotkeySection {
        title: "Global Controls",
        rows: GLOBAL_CONTROLS,
    },
    StaticHotkeySection {
        title: "1-Second Overlay",
        rows: OVERLAY_CONTROLS,
    },
    StaticHotkeySection {
        title: "Segment Controls (Click on segment)",
        rows: SEGMENT_CONTROLS,
    },
];

const TEXT_MOVE_HOTKEYS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["Alt", "["],
        description: "Move text before caret to previous segment",
    },
    CapabilityHotkey {
        keys: &["Alt", "]"],
        description: "Move text after caret to next segment",
    },
];

const ROW_ACTION_HOTKEYS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["Right Shift", "Left/Right"],
        description: "Focus previous / next segment from start",
    },
    CapabilityHotkey {
        keys: &["Alt", "1/2"],
        description: "Switch active speaker workflow lane",
    },
    CapabilityHotkey {
        keys: &["Alt", "`"],
        description: "Reset speaker workflow lanes",
    },
    CapabilityHotkey {
        keys: &["Alt", "X"],
        description: "Rewind playback 1000ms",
    },
    CapabilityHotkey {
        keys: &["Shift", "1/2"],
        description: "Increase / decrease playback speed",
    },
    CapabilityHotkey {
        keys: &["D"],
        description: "Delete current segment when not typing",
    },
    CapabilityHotkey {
        keys: &["Alt", "Shift", "Up/Down"],
        description: "Merge with adjacent segment",
    },
];

const TIMELINE_SELECTION_HOTKEYS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["Alt", "R"],
        description: "Trim current segment to nearby visible audio",
    },
    CapabilityHotkey {
        keys: &["Alt", "Shift", "R"],
        description: "Trim all visible segments to nearby visible audio",
    },
    CapabilityHotkey {
        keys: &["Alt", "Shift", "G"],
        description: "Transcribe current empty segment with native drafting audio",
    },
    CapabilityHotkey {
        keys: &["Alt", "C"],
        description: "Create empty segment around nearest uncovered speech near caret",
    },
    CapabilityHotkey {
        keys: &["Alt", "Shift", "S"],
        description: "Split visible segments on silence runs over 1000ms, then trim all",
    },
    CapabilityHotkey {
        keys: &["S"],
        description: "Commit active cut preview with smart text split",
    },
    CapabilityHotkey {
        keys: &["Shift", "S"],
        description: "Commit active cut preview without smart text split",
    },
    CapabilityHotkey {
        keys: &["L"],
        description: "Loop playback over the cut selection (again: stop; Escape clears)",
    },
];

const CUSTOM_LINTER_HOTKEYS: &[CapabilityHotkey] = &[
    CapabilityHotkey {
        keys: &["Alt", "F"],
        description: "Auto-fix lint issues in current row",
    },
    CapabilityHotkey {
        keys: &["Alt", "Shift", "F"],
        description: "Auto-fix lint issues in all rows",
    },
];

const CAPABILITY_DESCRIPTORS: &[CapabilityDescriptor] = &[
    helper_descriptor(
        "helper.hotkeys-help",
        "Hotkeys Help",
        "Extend the native hotkeys dialog with Helper workflow rows.",
        &[],
        &[],
    ),
    helper_descriptor(
        "helper.row-actions",
        "Row Actions",
        "Delete, merge, speaker-lane, rewind, and workflow reset commands.",
        &[
            "helper.row-actions.focus-previous-next",
            "helper.row-actions.delete",
            "helper.row-actions.merge-previous",
            "helper.row-actions.merge-next",
        ],
        ROW_ACTION_HOTKEYS,
    ),
    helper_descriptor(
        "helper.text-move",
        "Text Move",
        "Move text around the caret between neighboring transcript segments.",
        &[
            "helper.text-move.before-caret",
            "helper.text-move.after-caret",
        ],
        TEXT_MOVE_HOTKEYS,
    ),
    helper_descriptor(
        "helper.focus-toggle",
        "Focus Toggle",
        "Pause, blur, resume, and restore transcript focus.",
        &["helper.focus-toggle"],
        &[
            CapabilityHotkey {
                keys: &["Escape", "Or", "Space"],
                description: "Pause and blur / resume and restore cursor",
            },
            CapabilityHotkey {
                keys: &["Tab"],
                description: "Switch ghost cursor lane",
            },
        ],
    ),
    helper_descriptor(
        "helper.quick-region-autocomplete",
        "Quick Region Autocomplete",
        "Reuse tag autocomplete in quick region and row editors.",
        &["helper.quick-region-autocomplete.accept"],
        &[],
    ),
    helper_descriptor(
        "helper.selected-number-to-skaz",
        "Selected Number to SKAZ",
        "Convert selected digits into Babel SKAZ notation.",
        &[
            "helper.selected-number-to-skaz.inline",
            "helper.selected-number-to-skaz.selection",
        ],
        &[CapabilityHotkey {
            keys: &["Alt", "A"],
            description: "Auto-convert selected digits into SKAZ notation",
        }],
    ),
    helper_descriptor(
        "helper.timeline-selection",
        "Timeline Selection",
        "Cut, trim, and preview timeline range actions.",
        &[
            "helper.timeline-selection.cut-preview",
            "helper.timeline-selection.trim",
        ],
        TIMELINE_SELECTION_HOTKEYS,
    ),
    helper_descriptor(
        "helper.timeline-defaults",
        "Timeline Defaults",
        "Persist timeline zoom, lane visibility, and workflow defaults.",
        &[],
        &[],
    ),
    helper_descriptor(
        "helper.waveform-tools",
        "Waveform Tools",
        "Waveform scale unlock, magnifier, minimap, and tooltip improvements.",
        &[
            "helper.waveform-tools.magnifier",
            "helper.waveform-tools.minimap",
        ],
        &[],
    ),
    helper_descriptor(
        "helper.custom-linter",
        "Custom Linter",
        "Run Helper lint rules and fix actions through native lint state.",
        &[
            "helper.custom-linter.fix-row",
            "helper.custom-linter.fix-all",
        ],
        CUSTOM_LINTER_HOTKEYS,
    ),
    helper_descriptor(
        "helper.extended-diff",
        "Extended Diff",
        "Surface recovered diff details in native diff toolbar and rows.",
        &[],
        &[],
    ),
    reviewer_descriptor(
        "reviewer.generate",
        "Generate Review",
        "Generate review feedback for current versus original transcript state.",
        &["reviewer.generate"],
    ),
    reviewer_descriptor(
        "reviewer.session",
        "Review Session",
        "Create, load, save, and finalize structured review sessions.",
        &[
            "reviewer.session.create",
            "reviewer.session.load",
            "reviewer.session.finalize",
        ],
    ),
    reviewer_descriptor(
        "reviewer.templates",
        "Review Templates",
        "Search templates, match cards, clear matches, and handle suggestions.",
        &[
            "reviewer.templates.search",
            "reviewer.templates.match",
            "reviewer.templates.clear",
        ],
    ),
    reviewer_descriptor(
        "reviewer.comments",
        "Review Comments",
        "Persist session and card comments.",
        &["reviewer.comments.save"],
    ),
    reviewer_descriptor(
        "reviewer.analytics",
        "Review Analytics",
        "Submit review-action analytics payloads when configured.",
        &["reviewer.analytics.submit"],
    ),
    drafting_descriptor(
        "drafting.ping",
        "Drafting Status",
        "Report provider, remote configuration, and available drafting operations.",
        true,
        &["drafting.ping"],
    ),
    drafting_descriptor(
        "drafting.transcribe-segment",
        "Transcribe Segment",
        "Send selected segment audio/context and receive proposed text.",
        false,
        &["drafting.transcribe-segment"],
    ),
    drafting_descriptor(
        "drafting.redistribute-text",
        "Redistribute Text",
        "Rebalance transcript text across selected neighboring segments.",
        false,
        &["drafting.redistribute-text"],
    ),
    drafting_descriptor(
        "drafting.generate-rows",
        "Generate Rows",
        "Generate draft rows for a selected range or loaded task context.",
        false,
        &["drafting.generate-rows"],
    ),
    drafting_descriptor(
        "drafting.audio-cues",
        "Audio Cues",
        "Request audio-aware drafting cues through the drafting backend path.",
        false,
        &["drafting.audio-cues"],
    ),
];

const fn helper_descriptor(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    commands: &'static [&'static str],
    hotkeys: &'static [CapabilityHotkey],
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id),
        pack: CapabilityPack::Helper,
        label,
        description,
        default_enabled: true,
        commands,
        hotkeys,
        service_family: None,
    }
}

const fn reviewer_descriptor(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    commands: &'static [&'static str],
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id),
        pack: CapabilityPack::Reviewer,
        label,
        description,
        default_enabled: true,
        commands,
        hotkeys: &[],
        service_family: Some(CapabilityServiceFamily::Reviewer),
    }
}

const fn drafting_descriptor(
    id: &'static str,
    label: &'static str,
    description: &'static str,
    default_enabled: bool,
    commands: &'static [&'static str],
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id: CapabilityId::new(id),
        pack: CapabilityPack::Drafting,
        label,
        description,
        default_enabled,
        commands,
        hotkeys: &[],
        service_family: Some(CapabilityServiceFamily::Drafting),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_helper_reviewer_and_drafting_packs() {
        let registry = CapabilityRegistry::default();

        assert_eq!(registry.descriptors().len(), 21);
        assert!(registry.is_enabled(CapabilityId::new("helper.custom-linter")));
        assert!(registry.is_enabled(CapabilityId::new("reviewer.generate")));
        assert!(!registry.is_enabled(CapabilityId::new("drafting.transcribe-segment")));
    }

    #[test]
    fn descriptor_lookup_returns_stable_capability_metadata() {
        let registry = CapabilityRegistry::default();
        let descriptor = registry
            .descriptor(CapabilityId::new("helper.text-move"))
            .expect("helper text move capability should exist");

        assert_eq!(descriptor.pack, CapabilityPack::Helper);
        assert_eq!(descriptor.label, "Text Move");
        assert_eq!(
            descriptor.commands,
            &[
                "helper.text-move.before-caret",
                "helper.text-move.after-caret",
            ]
        );
        assert_eq!(descriptor.hotkeys.len(), 2);
    }

    #[test]
    fn settings_can_disable_default_enabled_capabilities_and_enable_ai() {
        let settings = CapabilitySettings::from_overrides([
            ("helper.custom-linter", false),
            ("drafting.transcribe-segment", true),
        ]);
        let registry = CapabilityRegistry::with_settings(settings);

        assert!(!registry.is_enabled(CapabilityId::new("helper.custom-linter")));
        assert!(registry.is_enabled(CapabilityId::new("drafting.transcribe-segment")));
        assert!(
            registry
                .enabled_descriptors()
                .any(|descriptor| descriptor.id.as_str() == "drafting.transcribe-segment")
        );
    }

    #[test]
    fn service_family_groups_ai_and_review_capabilities_without_changing_babel_requests() {
        let registry = CapabilityRegistry::default();

        assert_eq!(
            registry.service_family_for(CapabilityId::new("reviewer.generate")),
            Some(CapabilityServiceFamily::Reviewer)
        );
        assert_eq!(
            registry.service_family_for(CapabilityId::new("drafting.redistribute-text")),
            Some(CapabilityServiceFamily::Drafting)
        );
        assert_eq!(
            registry.service_family_for(CapabilityId::new("helper.custom-linter")),
            None
        );
    }
}
