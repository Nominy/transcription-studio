use std::{collections::BTreeMap, path::PathBuf};

use iced::{Task, keyboard::Modifiers};
use serde_json::json;
use studio_app::{EditorSession, ReviewActionStripView, StudioWorkflow};
use studio_core::{AudioLane, Project, Segment, Speaker};

use super::actions::update;
use super::browser_input::OutsideInteractionContext;
use super::create_region::BACKGROUND_NOISE_ANNOTATION_PROJECT_ID;
use super::hotkeys::{BrowserBehaviorHotkeyReport, browser_behavior_hotkey_report};
use super::renderer_capture::renderer_screenshot_task;
use super::{AuditionKey, CreateRegionDialog, Message, StudioState};

const BROWSER_BEHAVIOR_SMOKE_ENV: &str = "STUDIO_SMOKE_BROWSER_BEHAVIOR_ON_START";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowserBehaviorSmokeReport {
    pub(super) hotkeys: BrowserBehaviorHotkeyReport,
    pub(super) release_token_unrelated_release_preserved: bool,
    pub(super) focus_loss_cleared_keys_and_modifiers: bool,
    pub(super) escape_order: Vec<String>,
    pub(super) pending_escape_preserved_dialog: bool,
    pub(super) pending_escape_preserved_loop: bool,
    pub(super) default_hotkeys_before_submit: bool,
    pub(super) default_submit_started_pending_request: bool,
    pub(super) default_create_region_submitted: bool,
    pub(super) default_edit_annotation_submitted: bool,
    pub(super) default_find_routes_next: bool,
    pub(super) browser_back_action_status: bool,
    pub(super) create_region_select_preserved: bool,
    pub(super) create_region_portal_preserved: bool,
    pub(super) create_region_plain_outside_closed: bool,
    pub(super) edit_annotation_select_preserved: bool,
    pub(super) edit_annotation_inside_preserved: bool,
    pub(super) edit_annotation_portal_preserved: bool,
    pub(super) edit_annotation_plain_outside_closed: bool,
    pub(super) undo_redo_disabled: bool,
    pub(super) saved_button_not_clickable: bool,
    pub(super) submit_disabled: bool,
    pub(super) skip_disabled: bool,
    pub(super) admin_pending_controls_disabled: bool,
    pub(super) drop_claim_disabled: bool,
}

impl BrowserBehaviorSmokeReport {
    pub(super) fn all_passed(&self) -> bool {
        self.hotkeys.all_passed()
            && self.release_token_unrelated_release_preserved
            && self.focus_loss_cleared_keys_and_modifiers
            && self.escape_order.iter().map(String::as_str).eq([
                "find", "edit", "create", "hotkeys", "admin", "skip", "submit", "loop",
            ])
            && self.pending_escape_preserved_dialog
            && self.pending_escape_preserved_loop
            && self.default_hotkeys_before_submit
            && self.default_submit_started_pending_request
            && self.default_create_region_submitted
            && self.default_edit_annotation_submitted
            && self.default_find_routes_next
            && self.browser_back_action_status
            && self.create_region_select_preserved
            && self.create_region_portal_preserved
            && self.create_region_plain_outside_closed
            && self.edit_annotation_select_preserved
            && self.edit_annotation_inside_preserved
            && self.edit_annotation_portal_preserved
            && self.edit_annotation_plain_outside_closed
            && self.undo_redo_disabled
            && self.saved_button_not_clickable
            && self.submit_disabled
            && self.skip_disabled
            && self.admin_pending_controls_disabled
            && self.drop_claim_disabled
    }

    pub(super) fn runtime_trace_line(&self) -> String {
        format!(
            "Transcription Studio browser behavior smoke: ok={} hotkeys={} release_token={} focus_loss={} escape_order={} pending_escape={}/{} default=hotkeys:{},submit:{},create:{},edit:{},find:{} back={} outside=create-select:{},create-portal:{},create-close:{},edit-select:{},edit-inside:{},edit-portal:{},edit-close:{} buttons=undo-redo:{},saved:{},submit:{},skip:{},admin:{},drop:{}",
            self.all_passed(),
            self.hotkeys.all_passed(),
            self.release_token_unrelated_release_preserved,
            self.focus_loss_cleared_keys_and_modifiers,
            joined_or_dash(&self.escape_order),
            self.pending_escape_preserved_dialog,
            self.pending_escape_preserved_loop,
            self.default_hotkeys_before_submit,
            self.default_submit_started_pending_request,
            self.default_create_region_submitted,
            self.default_edit_annotation_submitted,
            self.default_find_routes_next,
            self.browser_back_action_status,
            self.create_region_select_preserved,
            self.create_region_portal_preserved,
            self.create_region_plain_outside_closed,
            self.edit_annotation_select_preserved,
            self.edit_annotation_inside_preserved,
            self.edit_annotation_portal_preserved,
            self.edit_annotation_plain_outside_closed,
            self.undo_redo_disabled,
            self.saved_button_not_clickable,
            self.submit_disabled,
            self.skip_disabled,
            self.admin_pending_controls_disabled,
            self.drop_claim_disabled,
        )
    }
}

pub(super) fn browser_behavior_smoke_enabled() -> bool {
    std::env::var(BROWSER_BEHAVIOR_SMOKE_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn browser_behavior_renderer_screenshot_task() -> Task<Message> {
    renderer_screenshot_task(
        PathBuf::from("target")
            .join("runtime-browser-behavior-smoke")
            .join("browser-behavior-renderer.bmp"),
    )
}

pub(super) fn run_browser_behavior_smoke(state: &mut StudioState) -> BrowserBehaviorSmokeReport {
    let hotkeys = browser_behavior_hotkey_report();
    let (release_token_unrelated_release_preserved, focus_loss_cleared_keys_and_modifiers) =
        run_key_release_state_smoke(state);
    let escape_order = run_escape_stack_smoke(state);
    let (pending_escape_preserved_dialog, pending_escape_preserved_loop) =
        run_pending_escape_smoke(state);
    let (
        default_hotkeys_before_submit,
        default_submit_started_pending_request,
        default_create_region_submitted,
        default_edit_annotation_submitted,
        default_find_routes_next,
    ) = run_default_action_smoke(state);
    let browser_back_action_status = run_browser_back_action_smoke(state);
    let (
        create_region_select_preserved,
        create_region_portal_preserved,
        create_region_plain_outside_closed,
        edit_annotation_select_preserved,
        edit_annotation_inside_preserved,
        edit_annotation_portal_preserved,
        edit_annotation_plain_outside_closed,
    ) = run_outside_interaction_smoke(state);
    let (
        undo_redo_disabled,
        saved_button_not_clickable,
        submit_disabled,
        skip_disabled,
        admin_pending_controls_disabled,
        drop_claim_disabled,
    ) = button_affordance_smoke_report();

    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(true)),
        "Loaded browser behavior affordance smoke project.".to_string(),
    );
    state.close_find_bar();

    let report = BrowserBehaviorSmokeReport {
        hotkeys,
        release_token_unrelated_release_preserved,
        focus_loss_cleared_keys_and_modifiers,
        escape_order,
        pending_escape_preserved_dialog,
        pending_escape_preserved_loop,
        default_hotkeys_before_submit,
        default_submit_started_pending_request,
        default_create_region_submitted,
        default_edit_annotation_submitted,
        default_find_routes_next,
        browser_back_action_status,
        create_region_select_preserved,
        create_region_portal_preserved,
        create_region_plain_outside_closed,
        edit_annotation_select_preserved,
        edit_annotation_inside_preserved,
        edit_annotation_portal_preserved,
        edit_annotation_plain_outside_closed,
        undo_redo_disabled,
        saved_button_not_clickable,
        submit_disabled,
        skip_disabled,
        admin_pending_controls_disabled,
        drop_claim_disabled,
    };
    eprintln!("{}", report.runtime_trace_line());
    report
}

fn run_key_release_state_smoke(state: &mut StudioState) -> (bool, bool) {
    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior key smoke project.".to_string(),
    );
    let _ = update(
        state,
        Message::AuditionKeyPressed {
            key: AuditionKey::BeforeStart,
            release_token: "Character(\"'\")".to_string(),
        },
    );
    let _ = update(
        state,
        Message::AuditionKeyReleaseToken("Character(\"a\")".to_string()),
    );
    let unrelated_release_preserved = state.held_audition_keys.contains(&AuditionKey::BeforeStart);

    state.keyboard_modifiers = Modifiers::COMMAND;
    let _ = update(state, Message::WindowUnfocused);
    let focus_loss_cleared = state.held_audition_keys.is_empty()
        && state.audition_key_release_tokens.is_empty()
        && state.keyboard_modifiers == Modifiers::empty();

    (unrelated_release_preserved, focus_loss_cleared)
}

fn run_escape_stack_smoke(state: &mut StudioState) -> Vec<String> {
    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior escape smoke project.".to_string(),
    );
    let project = state
        .project()
        .expect("browser behavior project should remain loaded")
        .clone();
    state.with_editor_mut(|editor| editor.set_loop_range_from_drag(&project, 3.5, 5.1));
    state.show_submit_dialog = true;
    state.show_skip_chunk_dialog = true;
    state.show_admin_flag_dialog = true;
    state.show_hotkeys_dialog = true;
    state.create_region_dialog = Some(CreateRegionDialog::new_for_project(
        &project,
        "track-speaker-1".to_string(),
        6.0,
        6.5,
    ));
    state.request_edit_annotation("annotation-1");
    let _ = update(state, Message::OpenFind);

    (0..8)
        .filter_map(|_| {
            let before = EscapeSurfaceSnapshot::capture(state);
            let _ = update(state, Message::EscapePressed);
            let after = EscapeSurfaceSnapshot::capture(state);
            before.closed_surface(&after).map(str::to_string)
        })
        .collect()
}

fn run_pending_escape_smoke(state: &mut StudioState) -> (bool, bool) {
    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior pending escape smoke project.".to_string(),
    );
    let project = state
        .project()
        .expect("browser behavior project should remain loaded")
        .clone();
    state.with_editor_mut(|editor| editor.set_loop_range_from_drag(&project, 3.5, 5.1));
    state.show_submit_dialog = true;
    state.submit_is_pending = true;

    let _ = update(state, Message::EscapePressed);

    (
        state.show_submit_dialog && state.submit_is_pending,
        state
            .editor
            .as_ref()
            .is_some_and(|editor| editor.stored_loop_range.is_some()),
    )
}

fn run_default_action_smoke(state: &mut StudioState) -> (bool, bool, bool, bool, bool) {
    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior default-action smoke project.".to_string(),
    );
    state.show_submit_dialog = true;
    state.show_hotkeys_dialog = true;
    let _ = update(state, Message::ActivateDefaultSurfaceAction);
    let hotkeys_before_submit =
        !state.show_hotkeys_dialog && state.show_submit_dialog && !state.submit_is_pending;
    let _ = update(state, Message::ActivateDefaultSurfaceAction);
    let submit_started_pending_request = state.submit_is_pending
        && state.babel_service_requests.iter().any(|request| {
            matches!(
                request,
                super::service_callbacks::BabelServiceRequest::LintAnnotations(_)
            )
        });

    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior create default smoke project.".to_string(),
    );
    let project = state
        .project()
        .expect("browser behavior project should remain loaded")
        .clone();
    let before_create_count = project.segments.len();
    state.create_region_dialog = Some(CreateRegionDialog::new_for_project(
        &project,
        "track-speaker-1".to_string(),
        6.0,
        6.5,
    ));
    state.change_create_region_primary_category("background-noise".to_string());
    state.change_create_region_secondary_category("rustle".to_string());
    state.change_create_region_intensity("2".to_string());
    let _ = update(state, Message::ActivateDefaultSurfaceAction);
    let create_region_submitted = state.create_region_dialog.is_none()
        && state
            .project()
            .is_some_and(|project| project.segments.len() == before_create_count + 1);

    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior edit default smoke project.".to_string(),
    );
    state.request_edit_annotation("annotation-1");
    state.change_edit_annotation_intensity("3".to_string());
    let _ = update(state, Message::ActivateDefaultSurfaceAction);
    let edit_annotation_submitted = state.edit_annotation_dialog.is_none()
        && state.editor.as_ref().is_some_and(|editor| editor.is_dirty);

    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior find default smoke project.".to_string(),
    );
    state.open_find_bar();
    let find_routes_next = state.activate_default_surface_action() == Some(Message::FindNext);

    (
        hotkeys_before_submit,
        submit_started_pending_request,
        create_region_submitted,
        edit_annotation_submitted,
        find_routes_next,
    )
}

fn run_browser_back_action_smoke(state: &mut StudioState) -> bool {
    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior back smoke project.".to_string(),
    );
    let _ = update(state, Message::BackToProjects);
    state.status == "Back to projects requested."
}

fn run_outside_interaction_smoke(
    state: &mut StudioState,
) -> (bool, bool, bool, bool, bool, bool, bool) {
    state.replace_workflow(
        StudioWorkflow::from_project(browser_behavior_project(false)),
        "Loaded browser behavior outside-click smoke project.".to_string(),
    );
    let project = state
        .project()
        .expect("browser behavior project should remain loaded")
        .clone();

    state.create_region_dialog = Some(CreateRegionDialog::new_for_project(
        &project,
        "track-speaker-1".to_string(),
        6.0,
        6.5,
    ));
    state.change_create_region_select_open(true);
    let _ = update(
        state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::plain_outside()),
    );
    let create_region_select_preserved = state.create_region_dialog.is_some();

    state.change_create_region_select_open(false);
    let _ = update(
        state,
        Message::CreateRegionInteractOutside(select_portal_context()),
    );
    let create_region_portal_preserved = state.create_region_dialog.is_some();

    let _ = update(
        state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::plain_outside()),
    );
    let create_region_plain_outside_closed = state.create_region_dialog.is_none();

    state.request_edit_annotation("annotation-1");
    state.change_edit_annotation_select_open(true);
    let _ = update(
        state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::plain_outside()),
    );
    let edit_annotation_select_preserved = state.edit_annotation_dialog.is_some();

    state.change_edit_annotation_select_open(false);
    let _ = update(
        state,
        Message::EditAnnotationOutsideMouseDown(inside_surface_context()),
    );
    let edit_annotation_inside_preserved = state.edit_annotation_dialog.is_some();

    let _ = update(
        state,
        Message::EditAnnotationOutsideMouseDown(select_portal_context()),
    );
    let edit_annotation_portal_preserved = state.edit_annotation_dialog.is_some();

    let _ = update(
        state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::plain_outside()),
    );
    let edit_annotation_plain_outside_closed = state.edit_annotation_dialog.is_none();

    (
        create_region_select_preserved,
        create_region_portal_preserved,
        create_region_plain_outside_closed,
        edit_annotation_select_preserved,
        edit_annotation_inside_preserved,
        edit_annotation_portal_preserved,
        edit_annotation_plain_outside_closed,
    )
}

fn button_affordance_smoke_report() -> (bool, bool, bool, bool, bool, bool) {
    let project = browser_behavior_project(true);
    let editor = EditorSession::for_project(&project);
    let strip = editor.transport_strip_with_history(&project, false, false);
    let submit = editor.submit_control(&project, false, false, false);
    let skip = EditorSession::skip_chunk_control(&project, false, false);
    let admin = EditorSession::admin_flag_control(true, "runtime smoke", true);
    let drop_claim_disabled = matches!(
        strip.review_actions,
        ReviewActionStripView::Editable {
            drop_claim: Some(ref drop_claim)
        } if drop_claim.disabled
    );

    (
        strip.undo_disabled && strip.redo_disabled,
        strip.save_label == "Saved" && !strip.save_clickable && !strip.save_disabled,
        submit.trigger_disabled,
        skip.trigger_disabled,
        admin.reason_disabled && admin.cancel_disabled && admin.confirm_disabled,
        drop_claim_disabled,
    )
}

#[derive(Debug, Clone, Copy)]
struct EscapeSurfaceSnapshot {
    find: bool,
    edit: bool,
    create: bool,
    hotkeys: bool,
    admin: bool,
    skip: bool,
    submit: bool,
    loop_range: bool,
}

impl EscapeSurfaceSnapshot {
    fn capture(state: &StudioState) -> Self {
        Self {
            find: state.find_bar_open,
            edit: state.edit_annotation_dialog.is_some(),
            create: state.create_region_dialog.is_some(),
            hotkeys: state.show_hotkeys_dialog,
            admin: state.show_admin_flag_dialog,
            skip: state.show_skip_chunk_dialog,
            submit: state.show_submit_dialog,
            loop_range: state
                .editor
                .as_ref()
                .is_some_and(|editor| editor.stored_loop_range.is_some()),
        }
    }

    fn closed_surface(self, after: &Self) -> Option<&'static str> {
        if self.find && !after.find {
            Some("find")
        } else if self.edit && !after.edit {
            Some("edit")
        } else if self.create && !after.create {
            Some("create")
        } else if self.hotkeys && !after.hotkeys {
            Some("hotkeys")
        } else if self.admin && !after.admin {
            Some("admin")
        } else if self.skip && !after.skip {
            Some("skip")
        } else if self.submit && !after.submit {
            Some("submit")
        } else if self.loop_range && !after.loop_range {
            Some("loop")
        } else {
            None
        }
    }
}

fn browser_behavior_project(disabled_controls: bool) -> Project {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "project_preset".to_string(),
        "browser-behavior-smoke".to_string(),
    );
    metadata.insert("studio.babel.mode".to_string(), "annotation".to_string());
    metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    metadata.insert(
        "studio.babel.reviewActionId".to_string(),
        "browser-behavior-review-action".to_string(),
    );
    metadata.insert(
        "studio.babel.dropClaim.available".to_string(),
        "true".to_string(),
    );
    if disabled_controls {
        metadata.insert(
            "studio.babel.submit.hasMissingIntensity".to_string(),
            "true".to_string(),
        );
        metadata.insert("studio.babel.skip.canSkip".to_string(), "false".to_string());
        metadata.insert(
            "studio.babel.dropClaim.isDropping".to_string(),
            "true".to_string(),
        );
    }

    Project {
        id: BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string(),
        name: if disabled_controls {
            "Browser Behavior Disabled Controls Smoke".to_string()
        } else {
            "Browser Behavior Runtime Smoke".to_string()
        },
        speakers: vec![Speaker {
            id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
        }],
        lanes: vec![AudioLane {
            id: "track-speaker-1".to_string(),
            speaker_id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
            source_path: "audio/speaker_1.wav".to_string(),
        }],
        segments: vec![Segment {
            id: "annotation-1".to_string(),
            source_annotation_id: Some("annotation-1".to_string()),
            speaker_id: "speaker-1".to_string(),
            lane_id: "track-speaker-1".to_string(),
            start_seconds: 0.0,
            end_seconds: 1.0,
            text: "Ambient rustle.".to_string(),
            metadata: BTreeMap::from([
                ("type".to_string(), json!("background_noise")),
                ("intensity".to_string(), json!(2)),
                (
                    "content".to_string(),
                    json!({
                        "primaryLabel": "background-noise",
                        "secondaryLabel": "rustle"
                    }),
                ),
            ]),
        }],
        metadata,
    }
}

fn inside_surface_context() -> OutsideInteractionContext {
    OutsideInteractionContext {
        inside_surface: true,
        ..OutsideInteractionContext::plain_outside()
    }
}

fn select_portal_context() -> OutsideInteractionContext {
    OutsideInteractionContext {
        has_listbox_ancestor: true,
        has_radix_portal_ancestor: true,
        has_radix_select_content_ancestor: true,
        ..OutsideInteractionContext::plain_outside()
    }
}

fn joined_or_dash(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(">")
    }
}
