use std::time::Duration;

use iced::keyboard::{
    Key, Location, Modifiers,
    key::{Code, Named, Physical},
};
use iced::{Event, Subscription, event, keyboard, mouse, time, window};
use studio_app::SUBMISSION_ID_COPY_RESET_DELAY_MS;

use super::browser_behavior_smoke::browser_behavior_smoke_enabled;
use super::diff_smoke::diff_workbench_smoke_mode;
use super::large_project_smoke::large_project_smoke_enabled;
use super::workbench_shell_smoke::workbench_shell_smoke_enabled;
use super::{AuditionKey, Message, StudioState};
use crate::app::state::SUPPORT_ID_COPIED_RESET_DELAY_MS;

pub(crate) const TRANSPORT_TICK_MILLIS: u64 = 16;
pub(crate) const TRANSPORT_TICK_SECONDS: f64 = TRANSPORT_TICK_MILLIS as f64 / 1000.0;
pub(crate) const AUTO_SAVE_TICK_MILLIS: u64 = 60_000;
pub(crate) const REVIEWER_COMMENT_SAVE_TICK_MILLIS: u64 = 500;
const AUTO_SAVE_TICK_OVERRIDE_ENV: &str = "STUDIO_AUTOSAVE_TICK_MILLIS";
const RENDERER_SMOKE_SCREENSHOT_ENV: &str = "STUDIO_SMOKE_RENDERER_SCREENSHOT";

const CTRL_F: &str = "\u{6}";
const CTRL_G: &str = "\u{7}";
const CTRL_J: &str = "\n";
const CTRL_Y: &str = "\u{19}";
const CTRL_Z: &str = "\u{1a}";
pub(crate) fn subscription(state: &StudioState) -> Subscription<Message> {
    let cut_preview_overlay_active = state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.overlay_range.is_some());
    let hotkey_handler = if cut_preview_overlay_active {
        hotkey_event_message_cut_preview
    } else {
        hotkey_event_message
    };
    let mut subscriptions = vec![event::listen_with(hotkey_handler)];
    if state.pointer_capture_subscription_active() {
        subscriptions.push(event::listen_with(pointer_capture_event_message));
    }
    if state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.is_playing)
        || state.startup_playback_smoke_delay_pending()
        || state.timeline_autoscroll_active()
    {
        subscriptions.push(
            time::every(Duration::from_millis(TRANSPORT_TICK_MILLIS)).map(Message::TransportTick),
        );
    }
    if state.should_run_editor_auto_save_tick() {
        subscriptions.push(
            time::every(Duration::from_millis(auto_save_tick_millis()))
                .map(|_| Message::AutoSaveTick),
        );
    }
    if state.should_run_reviewer_comment_save_tick() {
        subscriptions.push(
            time::every(Duration::from_millis(REVIEWER_COMMENT_SAVE_TICK_MILLIS))
                .map(|_| Message::ReviewerCommentSaveTick),
        );
    }
    if !state.renderer_smoke_screenshot_captured
        && renderer_smoke_screenshot_enabled()
        && (diff_workbench_smoke_mode().is_some()
            || large_project_smoke_enabled()
            || browser_behavior_smoke_enabled()
            || workbench_shell_smoke_enabled())
    {
        subscriptions.push(
            time::every(Duration::from_millis(1_200))
                .map(|_| Message::CaptureRendererSmokeScreenshot),
        );
    }
    if state.support_id_copied {
        subscriptions.push(
            time::every(Duration::from_millis(SUPPORT_ID_COPIED_RESET_DELAY_MS))
                .map(|_| Message::ResetSupportIdCopied),
        );
    }
    if state.copied_submission_id {
        subscriptions.push(
            time::every(Duration::from_millis(SUBMISSION_ID_COPY_RESET_DELAY_MS))
                .map(|_| Message::ResetSubmissionIdCopied),
        );
    }

    Subscription::batch(subscriptions)
}

pub(crate) fn auto_save_tick_millis() -> u64 {
    std::env::var(AUTO_SAVE_TICK_OVERRIDE_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(AUTO_SAVE_TICK_MILLIS)
}

fn renderer_smoke_screenshot_enabled() -> bool {
    std::env::var(RENDERER_SMOKE_SCREENSHOT_ENV).map_or(true, |value| value != "0")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowserBehaviorHotkeyReport {
    pub(super) captured_text_editing_preserved: bool,
    pub(super) repeated_keydown_ignored: bool,
    pub(super) tab_focus_traversal: bool,
    pub(super) browser_find_from_captured_input: bool,
    pub(super) find_navigation_from_captured_input: bool,
    pub(super) warning_assertion_from_captured_input: bool,
    pub(super) browser_back_outside_captured_input: bool,
    pub(super) browser_back_preserved_inside_input: bool,
    pub(super) captured_escape_routes_to_input_reset: bool,
    pub(super) captured_command_enter_submits_create_region: bool,
    pub(super) physical_audition_key_uses_keyboard_code: bool,
    pub(super) physical_global_shortcuts_use_keyboard_code: bool,
}

impl BrowserBehaviorHotkeyReport {
    pub(super) fn all_passed(&self) -> bool {
        self.captured_text_editing_preserved
            && self.repeated_keydown_ignored
            && self.tab_focus_traversal
            && self.browser_find_from_captured_input
            && self.find_navigation_from_captured_input
            && self.warning_assertion_from_captured_input
            && self.browser_back_outside_captured_input
            && self.browser_back_preserved_inside_input
            && self.captured_escape_routes_to_input_reset
            && self.captured_command_enter_submits_create_region
            && self.physical_audition_key_uses_keyboard_code
            && self.physical_global_shortcuts_use_keyboard_code
    }
}

pub(super) fn browser_behavior_hotkey_report() -> BrowserBehaviorHotkeyReport {
    let captured_text_editing_preserved = hotkey_message_for_event(
        Key::Named(Named::Space),
        None,
        Modifiers::empty(),
        event::Status::Captured,
        false,
    )
    .is_none()
        && hotkey_message_for_event(
            Key::Named(Named::ArrowRight),
            None,
            Modifiers::empty(),
            event::Status::Captured,
            false,
        )
        .is_none()
        && hotkey_message_for_event(
            Key::Character("z".into()),
            None,
            Modifiers::COMMAND,
            event::Status::Captured,
            false,
        )
        .is_none()
        && hotkey_message_for_event(
            Key::Named(Named::Undo),
            None,
            Modifiers::empty(),
            event::Status::Captured,
            false,
        )
        .is_none();

    let repeated_keydown_ignored = hotkey_message_for_event(
        Key::Named(Named::Space),
        None,
        Modifiers::empty(),
        event::Status::Ignored,
        true,
    )
    .is_none();

    let tab_focus_traversal = hotkey_message_for_event(
        Key::Named(Named::Tab),
        None,
        Modifiers::empty(),
        event::Status::Ignored,
        false,
    ) == Some(Message::FocusNext)
        && hotkey_message_for_event(
            Key::Named(Named::Tab),
            None,
            Modifiers::SHIFT,
            event::Status::Ignored,
            false,
        ) == Some(Message::FocusPrevious)
        && hotkey_message_for_event(
            Key::Named(Named::Tab),
            None,
            Modifiers::COMMAND,
            event::Status::Ignored,
            false,
        )
        .is_none()
        && hotkey_message_for_event(
            Key::Named(Named::Tab),
            None,
            Modifiers::empty(),
            event::Status::Captured,
            false,
        )
        .is_none();

    let browser_find_from_captured_input = hotkey_message_for_event(
        Key::Character("f".into()),
        None,
        Modifiers::COMMAND,
        event::Status::Captured,
        false,
    ) == Some(Message::OpenFind)
        && hotkey_message_for_event(
            Key::Character(CTRL_F.into()),
            None,
            Modifiers::COMMAND,
            event::Status::Captured,
            false,
        ) == Some(Message::OpenFind)
        && hotkey_message_for_event(
            Key::Named(Named::Find),
            None,
            Modifiers::empty(),
            event::Status::Captured,
            false,
        ) == Some(Message::OpenFind);

    let find_navigation_from_captured_input = hotkey_message_for_event(
        Key::Named(Named::F3),
        None,
        Modifiers::empty(),
        event::Status::Captured,
        false,
    ) == Some(Message::FindNext)
        && hotkey_message_for_event(
            Key::Named(Named::F3),
            None,
            Modifiers::SHIFT,
            event::Status::Captured,
            false,
        ) == Some(Message::FindPrevious)
        && hotkey_message_for_event(
            Key::Character("g".into()),
            None,
            Modifiers::COMMAND,
            event::Status::Captured,
            false,
        ) == Some(Message::FindNext)
        && hotkey_message_for_event(
            Key::Character(CTRL_G.into()),
            None,
            Modifiers::COMMAND | Modifiers::SHIFT,
            event::Status::Captured,
            false,
        ) == Some(Message::FindPrevious);

    let warning_assertion_from_captured_input = hotkey_message_for_event(
        Key::Character("j".into()),
        None,
        Modifiers::COMMAND,
        event::Status::Captured,
        false,
    ) == Some(Message::ToggleActiveWarningAssertion)
        && hotkey_message_for_event(
            Key::Character(CTRL_J.into()),
            None,
            Modifiers::COMMAND,
            event::Status::Captured,
            false,
        ) == Some(Message::ToggleActiveWarningAssertion);

    let browser_back_outside_captured_input = hotkey_message_for_event(
        Key::Named(Named::BrowserBack),
        None,
        Modifiers::empty(),
        event::Status::Ignored,
        false,
    ) == Some(Message::BackToProjects)
        && hotkey_message_for_event(
            Key::Named(Named::GoBack),
            None,
            Modifiers::empty(),
            event::Status::Ignored,
            false,
        ) == Some(Message::BackToProjects)
        && hotkey_message_for_event(
            Key::Named(Named::ArrowLeft),
            None,
            Modifiers::ALT,
            event::Status::Ignored,
            false,
        ) == Some(Message::BackToProjects);

    let browser_back_preserved_inside_input = hotkey_message_for_event(
        Key::Named(Named::BrowserBack),
        None,
        Modifiers::empty(),
        event::Status::Captured,
        false,
    )
    .is_none();

    let captured_escape_routes_to_input_reset = hotkey_message_for_event(
        Key::Named(Named::Escape),
        None,
        Modifiers::empty(),
        event::Status::Captured,
        false,
    ) == Some(Message::CreateRegionInputEscapePressed);

    let captured_command_enter_submits_create_region = hotkey_message_for_event(
        Key::Named(Named::Enter),
        None,
        Modifiers::COMMAND,
        event::Status::Captured,
        false,
    ) == Some(Message::SubmitCreateRegion);

    let physical_audition_key_uses_keyboard_code = matches!(
        hotkey_message_for_event(
            Key::Character("'".into()),
            Some(Physical::Code(Code::KeyQ)),
            Modifiers::empty(),
            event::Status::Ignored,
            false,
        ),
        Some(Message::AuditionKeyPressed {
            key: AuditionKey::BeforeStart,
            release_token
        }) if release_token == "Character(\"'\")"
    );

    let physical_global_shortcuts_use_keyboard_code = hotkey_message_for_event(
        Key::Character("w".into()),
        Some(Physical::Code(Code::KeyZ)),
        Modifiers::COMMAND,
        event::Status::Ignored,
        false,
    ) == Some(Message::Undo)
        && hotkey_message_for_event(
            Key::Character("x".into()),
            Some(Physical::Code(Code::KeyT)),
            Modifiers::empty(),
            event::Status::Ignored,
            false,
        ) == Some(Message::ToggleOverlay);

    BrowserBehaviorHotkeyReport {
        captured_text_editing_preserved,
        repeated_keydown_ignored,
        tab_focus_traversal,
        browser_find_from_captured_input,
        find_navigation_from_captured_input,
        warning_assertion_from_captured_input,
        browser_back_outside_captured_input,
        browser_back_preserved_inside_input,
        captured_escape_routes_to_input_reset,
        captured_command_enter_submits_create_region,
        physical_audition_key_uses_keyboard_code,
        physical_global_shortcuts_use_keyboard_code,
    }
}

fn hotkey_event_message(
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    hotkey_event_message_with_cut_preview(event, status, window, false)
}

fn hotkey_event_message_cut_preview(
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    hotkey_event_message_with_cut_preview(event, status, window, true)
}

fn hotkey_event_message_with_cut_preview(
    event: Event,
    status: event::Status,
    _window: window::Id,
    cut_preview_overlay_active: bool,
) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            physical_key,
            location,
            modifiers,
            ..
        }) => {
            if physical_key_is(Some(physical_key), Code::ShiftRight)
                || (matches!(key.as_ref(), Key::Named(Named::Shift)) && location == Location::Right)
            {
                return Some(Message::RightShiftChanged(true));
            }
            hotkey_message_for_event_with_cut_preview(
                key,
                Some(physical_key),
                modifiers,
                status,
                false,
                cut_preview_overlay_active,
            )
        }
        Event::Keyboard(keyboard::Event::KeyReleased { key, location, .. }) => {
            if matches!(key.as_ref(), Key::Named(Named::Shift)) && location == Location::Right {
                return Some(Message::RightShiftChanged(false));
            }
            hotkey_message_for_key_release(key)
        }
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::KeyboardModifiersChanged(modifiers))
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
            Some(Message::GlobalPointerReleased)
        }
        Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Middle)) => {
            Some(Message::TimelineAutoscrollReleased)
        }
        Event::Window(window::Event::Unfocused) => Some(Message::WindowUnfocused),
        Event::Window(window::Event::CloseRequested) => Some(Message::WindowCloseRequested),
        _ => None,
    }
}

fn pointer_capture_event_message(
    event: Event,
    _status: event::Status,
    _window: window::Id,
) -> Option<Message> {
    match event {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Message::GlobalPointerMoved(position))
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn hotkey_message(key: Key, modifiers: Modifiers) -> Option<Message> {
    hotkey_message_for_status(key, modifiers, event::Status::Ignored)
}

#[cfg(test)]
pub(crate) fn hotkey_message_for_repeat(
    key: Key,
    modifiers: Modifiers,
    status: event::Status,
    is_repeat: bool,
) -> Option<Message> {
    hotkey_message_for_event(key, None, modifiers, status, is_repeat)
}

#[cfg(test)]
pub(crate) fn hotkey_message_for_status(
    key: Key,
    modifiers: Modifiers,
    status: event::Status,
) -> Option<Message> {
    hotkey_message_for_event(key, None, modifiers, status, false)
}

#[cfg(test)]
pub(crate) fn hotkey_message_for_physical_status(
    key: Key,
    physical_key: Physical,
    modifiers: Modifiers,
    status: event::Status,
) -> Option<Message> {
    hotkey_message_for_event(key, Some(physical_key), modifiers, status, false)
}

#[cfg(test)]
pub(crate) fn hotkey_message_for_cut_preview_overlay(
    key: Key,
    physical_key: Physical,
    modifiers: Modifiers,
    status: event::Status,
) -> Option<Message> {
    hotkey_message_for_event_with_cut_preview(
        key,
        Some(physical_key),
        modifiers,
        status,
        false,
        true,
    )
}

fn hotkey_message_for_event(
    key: Key,
    physical_key: Option<Physical>,
    modifiers: Modifiers,
    status: event::Status,
    is_repeat: bool,
) -> Option<Message> {
    hotkey_message_for_event_with_cut_preview(
        key,
        physical_key,
        modifiers,
        status,
        is_repeat,
        false,
    )
}

fn hotkey_message_for_event_with_cut_preview(
    key: Key,
    physical_key: Option<Physical>,
    modifiers: Modifiers,
    status: event::Status,
    is_repeat: bool,
    cut_preview_overlay_active: bool,
) -> Option<Message> {
    if is_repeat {
        return None;
    }

    if status != event::Status::Captured
        && !modifiers.control()
        && !modifiers.alt()
        && !modifiers.logo()
        && !modifiers.command()
    {
        if let Some(physical_key) = physical_key {
            if let Some(audition_key) = audition_key_for_physical_key(physical_key) {
                return Some(Message::AuditionKeyPressed {
                    key: audition_key,
                    release_token: release_token_for_key(&key),
                });
            }
        } else if let Some(audition_key) = audition_key_for_key(key.clone()) {
            return Some(Message::AuditionKeyChanged(audition_key, true));
        }
    }

    let key = key.as_ref();

    if status == event::Status::Captured {
        if !modifiers.control()
            && !modifiers.alt()
            && !modifiers.logo()
            && !modifiers.command()
            && matches!(key, Key::Named(Named::F3))
        {
            return Some(if modifiers.shift() {
                Message::FindPrevious
            } else {
                Message::FindNext
            });
        }

        if modifiers.command()
            && !modifiers.shift()
            && !modifiers.alt()
            && key_character_is(&key, "f", CTRL_F)
        {
            return Some(Message::OpenFind);
        }

        if !modifiers.shift() && !modifiers.alt() && matches!(key, Key::Named(Named::Find)) {
            return Some(Message::OpenFind);
        }

        if modifiers.command() && !modifiers.alt() && key_character_is(&key, "g", CTRL_G) {
            return Some(if modifiers.shift() {
                Message::FindPrevious
            } else {
                Message::FindNext
            });
        }

        if modifiers.command()
            && !modifiers.shift()
            && !modifiers.alt()
            && key_character_or_code_is(&key, physical_key, "j", CTRL_J, Code::KeyJ)
        {
            return Some(Message::ToggleActiveWarningAssertion);
        }

        if modifiers.command() && !modifiers.alt() && matches!(key, Key::Named(Named::Enter)) {
            return Some(Message::SubmitCreateRegion);
        }

        if modifiers.alt()
            && !modifiers.control()
            && !modifiers.logo()
            && !modifiers.command()
            && let Some(message) = helper_text_hotkey_message(&key, physical_key, modifiers.shift())
        {
            return Some(message);
        }

        if !modifiers.control()
            && !modifiers.alt()
            && !modifiers.logo()
            && !modifiers.command()
            && matches!(key, Key::Named(Named::Escape))
        {
            return Some(Message::CreateRegionInputEscapePressed);
        }

        return None;
    }

    if cut_preview_overlay_active
        && !modifiers.alt()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
    {
        if key_character_or_code_is(&key, physical_key, "s", "", Code::KeyS) {
            return Some(Message::CommitCutPreviewOverlay {
                smart_split: !modifiers.shift(),
            });
        }
        if !modifiers.shift() && key_character_or_code_is(&key, physical_key, "l", "", Code::KeyL) {
            return Some(Message::LoopCutPreviewOverlay);
        }
    }

    if modifiers.shift()
        && !modifiers.command()
        && !modifiers.control()
        && !modifiers.alt()
        && !modifiers.logo()
    {
        if physical_key_is(physical_key, Code::Digit1) {
            return Some(Message::AdjustPlaybackSpeed(1));
        }
        if physical_key_is(physical_key, Code::Digit2) {
            return Some(Message::AdjustPlaybackSpeed(-1));
        }
        match key {
            Key::Named(Named::ArrowLeft) => return Some(Message::RightShiftSegmentNavigation(-1)),
            Key::Named(Named::ArrowRight) => return Some(Message::RightShiftSegmentNavigation(1)),
            _ => {}
        }
    }

    if matches!(key, Key::Named(Named::Tab))
        && !modifiers.command()
        && !modifiers.control()
        && !modifiers.alt()
        && !modifiers.logo()
    {
        return Some(if modifiers.shift() {
            Message::FocusPrevious
        } else {
            Message::FocusNext
        });
    }

    if !modifiers.shift() && !modifiers.alt() && matches!(key, Key::Named(Named::Find)) {
        return Some(Message::OpenFind);
    }

    if modifiers.alt()
        && modifiers.shift()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
    {
        match key {
            Key::Named(Named::ArrowUp) => return Some(Message::MergeSelectedSegmentAbove),
            Key::Named(Named::ArrowDown) => return Some(Message::MergeSelectedSegmentBelow),
            _ => {}
        }
        if key_character_or_code_is(&key, physical_key, "s", "", Code::KeyS) {
            return Some(Message::AutoSegmentVisibleSilences);
        }
        if key_character_or_code_is(&key, physical_key, "g", "", Code::KeyG) {
            return Some(Message::DraftTranscribeSelectedSegment);
        }
    }

    if modifiers.alt()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
        && key_character_or_code_is(&key, physical_key, "r", "", Code::KeyR)
    {
        return Some(if modifiers.shift() {
            Message::TrimAllVisibleSegmentsToAudio
        } else {
            Message::TrimCurrentSegmentToAudio
        });
    }

    if modifiers.alt()
        && !modifiers.shift()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
        && key_character_or_code_is(&key, physical_key, "c", "", Code::KeyC)
    {
        return Some(Message::AutoInsertSegmentAtCaret);
    }

    if modifiers.command() {
        if key_character_is(&key, "f", CTRL_F) && !modifiers.shift() {
            return Some(Message::OpenFind);
        }
        if key_character_is(&key, "g", CTRL_G) {
            return Some(if modifiers.shift() {
                Message::FindPrevious
            } else {
                Message::FindNext
            });
        }
        if key_character_or_code_is(&key, physical_key, "j", CTRL_J, Code::KeyJ) {
            return Some(Message::ToggleActiveWarningAssertion);
        }
        if key_character_or_code_is(&key, physical_key, "y", CTRL_Y, Code::KeyY) {
            return Some(Message::Redo);
        }
        if key_character_or_code_is(&key, physical_key, "z", CTRL_Z, Code::KeyZ) {
            return Some(if modifiers.shift() {
                Message::Redo
            } else {
                Message::Undo
            });
        }
        return None;
    }

    if !modifiers.shift()
        && !modifiers.alt()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
        && key_character_or_code_is(&key, physical_key, "d", "", Code::KeyD)
    {
        return Some(Message::DeleteSelectedSegment);
    }

    if modifiers.alt()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
        && let Some(message) = helper_text_hotkey_message(&key, physical_key, modifiers.shift())
    {
        return Some(message);
    }

    if modifiers.alt()
        && !modifiers.shift()
        && !modifiers.control()
        && !modifiers.logo()
        && !modifiers.command()
        && matches!(key, Key::Named(Named::ArrowLeft))
    {
        return Some(Message::BackToProjects);
    }

    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }

    match key {
        Key::Named(Named::BrowserBack | Named::GoBack) => Some(Message::BackToProjects),
        Key::Named(Named::Undo) => Some(Message::Undo),
        Key::Named(Named::Redo) => Some(Message::Redo),
        Key::Named(Named::Space) => Some(Message::TogglePlayback),
        Key::Named(Named::F3) => Some(if modifiers.shift() {
            Message::FindPrevious
        } else {
            Message::FindNext
        }),
        Key::Named(Named::ArrowLeft) => Some(Message::PreviousSegment),
        Key::Named(Named::ArrowRight) => Some(Message::NextSegment),
        Key::Named(Named::Escape) => Some(Message::EscapePressed),
        Key::Named(Named::Enter) => Some(Message::ActivateDefaultSurfaceAction),
        Key::Character("l") | Key::Character("L")
            if !physical_key
                .is_some_and(|physical| !matches!(physical, Physical::Code(Code::KeyL))) =>
        {
            Some(Message::ToggleLoop)
        }
        Key::Character("t") | Key::Character("T")
            if !physical_key
                .is_some_and(|physical| !matches!(physical, Physical::Code(Code::KeyT))) =>
        {
            Some(Message::ToggleOverlay)
        }
        _ if physical_key_is(physical_key, Code::KeyL) => Some(Message::ToggleLoop),
        _ if physical_key_is(physical_key, Code::KeyT) => Some(Message::ToggleOverlay),
        _ => None,
    }
}

fn helper_text_hotkey_message(
    key: &Key<&str>,
    physical_key: Option<Physical>,
    shift: bool,
) -> Option<Message> {
    if physical_key_is(physical_key, Code::Backquote) {
        return Some(Message::ResetSpeakerWorkflow);
    }
    if !shift && physical_key_is(physical_key, Code::Digit1) {
        return Some(Message::SwitchSpeakerWorkflowLane("Speaker 1".to_string()));
    }
    if !shift && physical_key_is(physical_key, Code::Digit2) {
        return Some(Message::SwitchSpeakerWorkflowLane("Speaker 2".to_string()));
    }
    if !shift && key_character_or_code_is(key, physical_key, "x", "", Code::KeyX) {
        return Some(Message::RewindPlaybackOneSecond);
    }
    if !shift && key_character_or_code_is(key, physical_key, "[", "", Code::BracketLeft) {
        return Some(Message::MoveTextBeforeCaretToPreviousSegment);
    }
    if !shift && key_character_or_code_is(key, physical_key, "]", "", Code::BracketRight) {
        return Some(Message::MoveTextAfterCaretToNextSegment);
    }
    if !shift && key_character_or_code_is(key, physical_key, "a", "", Code::KeyA) {
        return Some(Message::ConvertSelectedNumberToSkaz);
    }
    if key_character_or_code_is(key, physical_key, "f", "", Code::KeyF) {
        return Some(if shift {
            Message::AutofixAllLintRows
        } else {
            Message::AutofixCurrentLintRow
        });
    }
    None
}

fn key_character_is(key: &Key<&str>, expected: &str, control_character: &str) -> bool {
    match key {
        Key::Character(value) => {
            value.eq_ignore_ascii_case(expected) || *value == control_character
        }
        _ => false,
    }
}

fn key_character_or_code_is(
    key: &Key<&str>,
    physical_key: Option<Physical>,
    expected: &str,
    control_character: &str,
    code: Code,
) -> bool {
    physical_key_is(physical_key, code) || key_character_is(key, expected, control_character)
}

fn physical_key_is(physical_key: Option<Physical>, code: Code) -> bool {
    matches!(physical_key, Some(Physical::Code(actual)) if actual == code)
}

fn audition_key_for_key(key: Key) -> Option<AuditionKey> {
    match key.as_ref() {
        Key::Character("q") | Key::Character("Q") => Some(AuditionKey::BeforeStart),
        Key::Character("w") | Key::Character("W") => Some(AuditionKey::AfterStart),
        Key::Character("e") | Key::Character("E") => Some(AuditionKey::BeforeEnd),
        Key::Character("r") | Key::Character("R") => Some(AuditionKey::AfterEnd),
        _ => None,
    }
}

fn hotkey_message_for_key_release(key: Key) -> Option<Message> {
    if let Some(audition_key) = audition_key_for_key(key.clone()) {
        return Some(Message::AuditionKeyChanged(audition_key, false));
    }

    if is_modifier_key(key.as_ref()) {
        return None;
    }

    Some(Message::AuditionKeyReleaseToken(release_token_for_key(
        &key,
    )))
}

fn is_modifier_key(key: Key<&str>) -> bool {
    matches!(
        key,
        Key::Named(
            Named::Alt
                | Named::AltGraph
                | Named::CapsLock
                | Named::Control
                | Named::Fn
                | Named::FnLock
                | Named::NumLock
                | Named::ScrollLock
                | Named::Shift
                | Named::Symbol
                | Named::SymbolLock
                | Named::Meta
                | Named::Hyper
                | Named::Super
        )
    )
}

fn release_token_for_key(key: &Key) -> String {
    format!("{:?}", key.as_ref())
}

fn audition_key_for_physical_key(physical_key: Physical) -> Option<AuditionKey> {
    match physical_key {
        Physical::Code(Code::KeyQ) => Some(AuditionKey::BeforeStart),
        Physical::Code(Code::KeyW) => Some(AuditionKey::AfterStart),
        Physical::Code(Code::KeyE) => Some(AuditionKey::BeforeEnd),
        Physical::Code(Code::KeyR) => Some(AuditionKey::AfterEnd),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key::{Code, Physical};
    use iced::keyboard::{Key, Location, Modifiers};
    use iced::{Event, event, keyboard, window};

    use super::*;

    fn key_press_message(
        key: Key,
        physical_key: Physical,
        status: event::Status,
    ) -> Option<Message> {
        hotkey_event_message(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key,
                physical_key,
                location: Location::Standard,
                modifiers: Modifiers::empty(),
                text: None,
            }),
            status,
            window::Id::unique(),
        )
    }

    fn key_release_message(key: Key) -> Option<Message> {
        hotkey_event_message(
            Event::Keyboard(keyboard::Event::KeyReleased {
                key,
                location: Location::Standard,
                modifiers: Modifiers::empty(),
            }),
            event::Status::Ignored,
            window::Id::unique(),
        )
    }

    fn key_press_message_with_modifiers(
        key: Key,
        physical_key: Physical,
        modifiers: Modifiers,
        status: event::Status,
    ) -> Option<Message> {
        hotkey_event_message(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key,
                physical_key,
                location: Location::Standard,
                modifiers,
                text: None,
            }),
            status,
            window::Id::unique(),
        )
    }

    fn window_message(event: window::Event) -> Option<Message> {
        hotkey_event_message(
            Event::Window(event),
            event::Status::Ignored,
            window::Id::unique(),
        )
    }

    #[test]
    fn physical_babel_audition_keys_follow_keyboard_event_code() {
        assert_eq!(
            key_press_message(
                Key::Character("'".into()),
                Physical::Code(Code::KeyQ),
                event::Status::Ignored,
            ),
            Some(Message::AuditionKeyPressed {
                key: AuditionKey::BeforeStart,
                release_token: "Character(\"'\")".to_string(),
            })
        );
        assert_eq!(
            key_press_message(
                Key::Character("z".into()),
                Physical::Code(Code::KeyW),
                event::Status::Ignored,
            ),
            Some(Message::AuditionKeyPressed {
                key: AuditionKey::AfterStart,
                release_token: "Character(\"z\")".to_string(),
            })
        );
        assert_eq!(
            key_press_message(
                Key::Character("'".into()),
                Physical::Code(Code::KeyQ),
                event::Status::Captured,
            ),
            None
        );
    }

    #[test]
    fn physical_babel_global_shortcuts_follow_keyboard_event_code() {
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Character("w".into()),
                Physical::Code(Code::KeyZ),
                Modifiers::COMMAND,
                event::Status::Ignored,
            ),
            Some(Message::Undo)
        );
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Character("w".into()),
                Physical::Code(Code::KeyZ),
                Modifiers::COMMAND | Modifiers::SHIFT,
                event::Status::Ignored,
            ),
            Some(Message::Redo)
        );
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Character(";".into()),
                Physical::Code(Code::KeyJ),
                Modifiers::COMMAND,
                event::Status::Captured,
            ),
            Some(Message::ToggleActiveWarningAssertion)
        );
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Character("m".into()),
                Physical::Code(Code::KeyL),
                Modifiers::empty(),
                event::Status::Ignored,
            ),
            Some(Message::ToggleLoop)
        );
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Character("b".into()),
                Physical::Code(Code::KeyT),
                Modifiers::empty(),
                event::Status::Ignored,
            ),
            Some(Message::ToggleOverlay)
        );
    }

    #[test]
    fn window_unfocus_clears_babel_audition_key_holds() {
        assert_eq!(
            window_message(window::Event::Unfocused),
            Some(Message::WindowUnfocused)
        );
        assert_eq!(window_message(window::Event::Focused), None);
    }

    #[test]
    fn unmapped_key_release_uses_physical_audition_release_token() {
        assert_eq!(
            key_release_message(Key::Character("q".into())),
            Some(Message::AuditionKeyChanged(AuditionKey::BeforeStart, false))
        );
        assert_eq!(
            key_release_message(Key::Character("'".into())),
            Some(Message::AuditionKeyReleaseToken(
                "Character(\"'\")".to_string()
            ))
        );
    }
}
