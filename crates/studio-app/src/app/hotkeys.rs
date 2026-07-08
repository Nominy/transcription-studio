use std::time::Duration;

use iced::keyboard::{
    Key, Location, Modifiers,
    key::{Code, Named, Physical},
};
use iced::{Event, Subscription, event, keyboard, mouse, time, window};
use studio_app::SUBMISSION_ID_COPY_RESET_DELAY_MS;

use super::browser_behavior_smoke::browser_behavior_smoke_enabled;
use super::diff_smoke::diff_workbench_smoke_mode;
use super::keybinds::{HotkeyAction, HotkeyBindings, KeyBinding};
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
pub(crate) fn subscription(state: &StudioState) -> Subscription<Message> {
    let mut subscriptions = vec![event::listen_with(runtime_event_message)];
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
    pub(super) tab_ghost_cursor_navigation: bool,
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
            && self.tab_ghost_cursor_navigation
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

    let tab_ghost_cursor_navigation = hotkey_message_for_event(
        Key::Named(Named::Tab),
        None,
        Modifiers::empty(),
        event::Status::Ignored,
        false,
    ) == Some(Message::GhostCursorRowNavigation(1))
        && hotkey_message_for_event(
            Key::Named(Named::Tab),
            None,
            Modifiers::SHIFT,
            event::Status::Ignored,
            false,
        ) == Some(Message::GhostCursorRowNavigation(-1))
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
        tab_ghost_cursor_navigation,
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

fn runtime_event_message(
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    let should_route = matches!(
        &event,
        Event::Keyboard(_)
            | Event::Mouse(mouse::Event::ButtonReleased(
                mouse::Button::Left | mouse::Button::Middle
            ))
            | Event::Window(window::Event::Unfocused | window::Event::CloseRequested)
    );
    should_route.then_some(Message::RuntimeEvent {
        event,
        status,
        window,
    })
}

pub(super) fn event_message_for_state(
    state: &StudioState,
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    if let Some(action) = state.hotkey_capture {
        return hotkey_capture_event_message(event, status, window, action);
    }
    let cut_preview_overlay_active = state
        .editor
        .as_ref()
        .is_some_and(|editor| editor.overlay_range.is_some());
    hotkey_event_message_with_bindings(
        event,
        status,
        window,
        cut_preview_overlay_active,
        &state.hotkey_bindings,
    )
}

#[cfg(test)]
fn hotkey_event_message(
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    hotkey_event_message_with_cut_preview(event, status, window, false)
}

#[cfg(test)]
fn hotkey_event_message_cut_preview(
    event: Event,
    status: event::Status,
    window: window::Id,
) -> Option<Message> {
    hotkey_event_message_with_cut_preview(event, status, window, true)
}

#[cfg(test)]
fn hotkey_event_message_with_cut_preview(
    event: Event,
    status: event::Status,
    window: window::Id,
    cut_preview_overlay_active: bool,
) -> Option<Message> {
    hotkey_event_message_with_bindings(
        event,
        status,
        window,
        cut_preview_overlay_active,
        &HotkeyBindings::default(),
    )
}

fn hotkey_event_message_with_bindings(
    event: Event,
    status: event::Status,
    _window: window::Id,
    cut_preview_overlay_active: bool,
    hotkey_bindings: &HotkeyBindings,
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
            hotkey_message_for_event_with_cut_preview_and_bindings(
                key,
                Some(physical_key),
                modifiers,
                status,
                false,
                cut_preview_overlay_active,
                hotkey_bindings,
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

fn hotkey_capture_event_message(
    event: Event,
    _status: event::Status,
    _window: window::Id,
    action: HotkeyAction,
) -> Option<Message> {
    match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            physical_key,
            modifiers,
            ..
        }) => KeyBinding::from_physical(physical_key, modifiers)
            .map(|binding| Message::HotkeyBindingCaptured(action, binding)),
        Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::KeyboardModifiersChanged(modifiers))
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
    hotkey_message_for_event_with_cut_preview_and_bindings(
        key,
        physical_key,
        modifiers,
        status,
        is_repeat,
        cut_preview_overlay_active,
        &HotkeyBindings::default(),
    )
}

fn hotkey_message_for_event_with_cut_preview_and_bindings(
    key: Key,
    physical_key: Option<Physical>,
    modifiers: Modifiers,
    status: event::Status,
    is_repeat: bool,
    cut_preview_overlay_active: bool,
    hotkey_bindings: &HotkeyBindings,
) -> Option<Message> {
    if is_repeat {
        return None;
    }

    let effective_physical_key = physical_key.or_else(|| fallback_physical_key_for_tests(&key));

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

    if status == event::Status::Captured {
        if hotkey_bindings.matches(
            HotkeyAction::FindPrevious,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::FindPrevious);
        }
        if hotkey_bindings.matches(HotkeyAction::FindNext, effective_physical_key, modifiers) {
            return Some(Message::FindNext);
        }
        if hotkey_bindings.matches(HotkeyAction::OpenFind, effective_physical_key, modifiers) {
            return Some(Message::OpenFind);
        }
        if hotkey_bindings.matches(
            HotkeyAction::ToggleActiveWarningAssertion,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::ToggleActiveWarningAssertion);
        }
        if hotkey_bindings.matches(
            HotkeyAction::CreateRegionSubmit,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::SubmitCreateRegion);
        }
        if let Some(message) =
            helper_text_hotkey_message(effective_physical_key, modifiers, hotkey_bindings)
        {
            return Some(message);
        }
        if hotkey_bindings.matches(
            HotkeyAction::CreateRegionEscape,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::CreateRegionInputEscapePressed);
        }
        return None;
    }

    if cut_preview_overlay_active {
        if hotkey_bindings.matches(
            HotkeyAction::CommitCutPreviewPlainSplit,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::CommitCutPreviewOverlay { smart_split: false });
        }
        if hotkey_bindings.matches(
            HotkeyAction::CommitCutPreviewSmartSplit,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::CommitCutPreviewOverlay { smart_split: true });
        }
        if hotkey_bindings.matches(
            HotkeyAction::LoopCutPreviewOverlay,
            effective_physical_key,
            modifiers,
        ) {
            return Some(Message::LoopCutPreviewOverlay);
        }
    }

    if modifiers.shift()
        && !modifiers.command()
        && !modifiers.control()
        && !modifiers.alt()
        && !modifiers.logo()
    {
        if physical_key_is(effective_physical_key, Code::ArrowLeft) {
            return Some(Message::RightShiftSegmentNavigation(-1));
        }
        if physical_key_is(effective_physical_key, Code::ArrowRight) {
            return Some(Message::RightShiftSegmentNavigation(1));
        }
    }

    if hotkey_bindings.matches(
        HotkeyAction::IncreasePlaybackSpeed,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::AdjustPlaybackSpeed(1));
    }
    if hotkey_bindings.matches(
        HotkeyAction::DecreasePlaybackSpeed,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::AdjustPlaybackSpeed(-1));
    }
    if hotkey_bindings.matches(
        HotkeyAction::GhostCursorPrevious,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::GhostCursorRowNavigation(-1));
    }
    if hotkey_bindings.matches(
        HotkeyAction::GhostCursorNext,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::GhostCursorRowNavigation(1));
    }
    if hotkey_bindings.matches(
        HotkeyAction::FindPrevious,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::FindPrevious);
    }
    if hotkey_bindings.matches(HotkeyAction::FindNext, effective_physical_key, modifiers) {
        return Some(Message::FindNext);
    }
    if hotkey_bindings.matches(HotkeyAction::OpenFind, effective_physical_key, modifiers) {
        return Some(Message::OpenFind);
    }
    if hotkey_bindings.matches(
        HotkeyAction::ToggleActiveWarningAssertion,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::ToggleActiveWarningAssertion);
    }
    if hotkey_bindings.matches(HotkeyAction::Undo, effective_physical_key, modifiers) {
        return Some(Message::Undo);
    }
    if hotkey_bindings.matches(HotkeyAction::Redo, effective_physical_key, modifiers) {
        return Some(Message::Redo);
    }
    if hotkey_bindings.matches(
        HotkeyAction::MergeSelectedSegmentAbove,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::MergeSelectedSegmentAbove);
    }
    if hotkey_bindings.matches(
        HotkeyAction::MergeSelectedSegmentBelow,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::MergeSelectedSegmentBelow);
    }
    if hotkey_bindings.matches(
        HotkeyAction::AutoSegmentVisibleSilences,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::AutoSegmentVisibleSilences);
    }
    if hotkey_bindings.matches(
        HotkeyAction::DraftTranscribeSelectedSegment,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::DraftTranscribeSelectedSegment);
    }
    if hotkey_bindings.matches(
        HotkeyAction::TrimAllVisibleSegmentsToAudio,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::TrimAllVisibleSegmentsToAudio);
    }
    if hotkey_bindings.matches(
        HotkeyAction::TrimCurrentSegmentToAudio,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::TrimCurrentSegmentToAudio);
    }
    if hotkey_bindings.matches(
        HotkeyAction::AutoInsertSegmentAtCaret,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::AutoInsertSegmentAtCaret);
    }
    if hotkey_bindings.matches(
        HotkeyAction::DeleteSelectedSegment,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::DeleteSelectedSegment);
    }
    if let Some(message) =
        helper_text_hotkey_message(effective_physical_key, modifiers, hotkey_bindings)
    {
        return Some(message);
    }
    if hotkey_bindings.matches(
        HotkeyAction::BackToProjects,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::BackToProjects);
    }

    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }

    if hotkey_bindings.matches(HotkeyAction::PlayPause, effective_physical_key, modifiers) {
        return Some(Message::HelperTogglePlayback);
    }
    if hotkey_bindings.matches(
        HotkeyAction::PreviousSegment,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::PreviousSegment);
    }
    if hotkey_bindings.matches(HotkeyAction::NextSegment, effective_physical_key, modifiers) {
        return Some(Message::NextSegment);
    }
    if hotkey_bindings.matches(
        HotkeyAction::EscapePressed,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::EscapePressed);
    }
    if hotkey_bindings.matches(
        HotkeyAction::ActivateDefaultSurfaceAction,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::ActivateDefaultSurfaceAction);
    }
    if hotkey_bindings.matches(HotkeyAction::ToggleLoop, effective_physical_key, modifiers) {
        return Some(Message::ToggleLoop);
    }
    if hotkey_bindings.matches(
        HotkeyAction::ToggleOverlay,
        effective_physical_key,
        modifiers,
    ) {
        return Some(Message::ToggleOverlay);
    }

    None
}

fn fallback_physical_key_for_tests(key: &Key) -> Option<Physical> {
    let code = match key.as_ref() {
        Key::Named(Named::BrowserBack | Named::GoBack) => Code::BrowserBack,
        Key::Named(Named::Undo) => Code::Undo,
        Key::Named(Named::Redo) => Code::Again,
        Key::Named(Named::Space) => Code::Space,
        Key::Named(Named::F3) => Code::F3,
        Key::Named(Named::Find) => Code::Find,
        Key::Named(Named::ArrowLeft) => Code::ArrowLeft,
        Key::Named(Named::ArrowRight) => Code::ArrowRight,
        Key::Named(Named::ArrowUp) => Code::ArrowUp,
        Key::Named(Named::ArrowDown) => Code::ArrowDown,
        Key::Named(Named::Escape) => Code::Escape,
        Key::Named(Named::Enter) => Code::Enter,
        Key::Named(Named::Tab) => Code::Tab,
        Key::Character(value) => character_physical_code_for_tests(value)?,
        _ => return None,
    };
    Some(Physical::Code(code))
}

fn character_physical_code_for_tests(value: &str) -> Option<Code> {
    Some(match value {
        "`" => Code::Backquote,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "a" | "A" | "\u{1}" => Code::KeyA,
        "c" | "C" | "\u{3}" => Code::KeyC,
        "d" | "D" => Code::KeyD,
        "f" | "F" | "\u{6}" => Code::KeyF,
        "g" | "G" | "\u{7}" => Code::KeyG,
        "j" | "J" | "\n" => Code::KeyJ,
        "l" | "L" => Code::KeyL,
        "r" | "R" => Code::KeyR,
        "s" | "S" => Code::KeyS,
        "t" | "T" => Code::KeyT,
        "x" | "X" | "\u{18}" => Code::KeyX,
        "y" | "Y" | "\u{19}" => Code::KeyY,
        "z" | "Z" | "\u{1a}" => Code::KeyZ,
        "1" => Code::Digit1,
        "2" => Code::Digit2,
        _ => return None,
    })
}

fn helper_text_hotkey_message(
    physical_key: Option<Physical>,
    modifiers: Modifiers,
    hotkey_bindings: &HotkeyBindings,
) -> Option<Message> {
    if hotkey_bindings.matches(HotkeyAction::ResetSpeakerWorkflow, physical_key, modifiers) {
        return Some(Message::ResetSpeakerWorkflow);
    }
    if hotkey_bindings.matches(
        HotkeyAction::SwitchSpeakerWorkflowLane1,
        physical_key,
        modifiers,
    ) {
        return Some(Message::SwitchSpeakerWorkflowLane("Speaker 1".to_string()));
    }
    if hotkey_bindings.matches(
        HotkeyAction::SwitchSpeakerWorkflowLane2,
        physical_key,
        modifiers,
    ) {
        return Some(Message::SwitchSpeakerWorkflowLane("Speaker 2".to_string()));
    }
    if hotkey_bindings.matches(
        HotkeyAction::RewindPlaybackOneSecond,
        physical_key,
        modifiers,
    ) {
        return Some(Message::RewindPlaybackOneSecond);
    }
    if hotkey_bindings.matches(
        HotkeyAction::MoveTextBeforeCaretToPreviousSegment,
        physical_key,
        modifiers,
    ) {
        return Some(Message::MoveTextBeforeCaretToPreviousSegment);
    }
    if hotkey_bindings.matches(
        HotkeyAction::MoveTextAfterCaretToNextSegment,
        physical_key,
        modifiers,
    ) {
        return Some(Message::MoveTextAfterCaretToNextSegment);
    }
    if hotkey_bindings.matches(
        HotkeyAction::ConvertSelectedNumberToSkaz,
        physical_key,
        modifiers,
    ) {
        return Some(Message::ConvertSelectedNumberToSkaz);
    }
    if hotkey_bindings.matches(HotkeyAction::AutofixAllLintRows, physical_key, modifiers) {
        return Some(Message::AutofixAllLintRows);
    }
    if hotkey_bindings.matches(HotkeyAction::AutofixCurrentLintRow, physical_key, modifiers) {
        return Some(Message::AutofixCurrentLintRow);
    }
    None
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
    use iced::keyboard::key::{Code, Named, Physical};
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

    fn key_press_message_with_bindings(
        key: Key,
        physical_key: Physical,
        modifiers: Modifiers,
        status: event::Status,
        hotkey_bindings: &HotkeyBindings,
    ) -> Option<Message> {
        hotkey_event_message_with_bindings(
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
            false,
            hotkey_bindings,
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
    fn delete_selected_segment_default_routes_unmodified_d_only_outside_captured_text_input() {
        assert_eq!(
            key_press_message(
                Key::Character("d".into()),
                Physical::Code(Code::KeyD),
                event::Status::Ignored,
            ),
            Some(Message::DeleteSelectedSegment)
        );
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Named(Named::Backspace),
                Physical::Code(Code::Backspace),
                Modifiers::CTRL,
                event::Status::Ignored,
            ),
            None,
            "Ctrl+Backspace must stay available for browser-style text editing"
        );
        assert_eq!(
            key_press_message_with_modifiers(
                Key::Named(Named::Backspace),
                Physical::Code(Code::Backspace),
                Modifiers::CTRL,
                event::Status::Captured,
            ),
            None
        );
    }

    #[test]
    fn configurable_shortcuts_match_physical_code_and_replace_action_defaults() {
        let mut bindings = HotkeyBindings::default();
        bindings.set_override(
            HotkeyAction::DeleteSelectedSegment,
            KeyBinding::new(Modifiers::ALT, Code::KeyK),
        );

        assert_eq!(
            key_press_message_with_bindings(
                Key::Character("x".into()),
                Physical::Code(Code::KeyK),
                Modifiers::ALT,
                event::Status::Ignored,
                &bindings,
            ),
            Some(Message::DeleteSelectedSegment),
            "the configured physical key code should win over the logical character"
        );
        assert_eq!(
            key_press_message_with_bindings(
                Key::Character("k".into()),
                Physical::Code(Code::KeyB),
                Modifiers::ALT,
                event::Status::Ignored,
                &bindings,
            ),
            None,
            "the logical character must not trigger when the physical key is different"
        );
        assert_eq!(
            key_press_message_with_bindings(
                Key::Character("d".into()),
                Physical::Code(Code::KeyD),
                Modifiers::empty(),
                event::Status::Ignored,
                &bindings,
            ),
            None,
            "overriding an action should disable its unmodified D default"
        );
        assert_eq!(
            key_press_message_with_bindings(
                Key::Named(Named::Backspace),
                Physical::Code(Code::Backspace),
                Modifiers::CTRL,
                event::Status::Ignored,
                &bindings,
            ),
            None,
            "overriding an action must not introduce Ctrl+Backspace as a hidden fallback"
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
