use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::{Duration, Instant},
};

use iced::keyboard::{
    Key, Modifiers,
    key::{Code, Named, Physical},
};
use iced::widget::text_editor::{
    Action as TextEditAction, Edit as TextEdit, Motion as TextEditMotion,
};
use iced::{Point, event::Status};
use studio_app::{
    FEEDBACK_MAX_COMMENT_LENGTH, FeedbackDraftSaveRequest, FeedbackInputResponse, LaneWaveform,
    LoopStripHit, LoopStripHitTarget, NoWorkModalInput, ORIGINAL_SCRIPT_STORAGE_KEY, ReviewState,
    SCRIPT_CONTEXT_STORAGE_KEY, ScriptContextPanelInput, StudioWorkflow, TaskLookupEffect,
    TimeRange, TranscriptActionMenuEntryView, WaveformPeakSource, build_no_work_modal_model,
    build_original_script_panel_model, build_script_context_panel_model,
    reduce_collapsible_panel_toggle,
};
use studio_babel::BabelTaskPackage;
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_waveform::{DecodedAudio, WaveformHit, WaveformHitTarget};

use super::annotation_smoke::{AnnotationMutationSmokeReport, run_annotation_mutation_smoke};
use super::browser_behavior_smoke::run_browser_behavior_smoke;
use super::browser_input::OutsideInteractionContext;
use super::create_region::{
    BACKGROUND_NOISE_ANNOTATION_PROJECT_ID, CreateRegionDialog, CreateRegionMode,
    CreateRegionTimeField, create_region_secondary_options, create_region_track_options,
};
use super::diff_smoke::{DiffWorkbenchSmokeMode, run_diff_workbench_smoke};
use super::formatting::{
    babel_slider_from_zoom_factor, babel_speed_labels, parse_draft_times, parse_seconds,
    zoom_factor_from_babel_slider,
};
use super::hotkeys::{
    AUTO_SAVE_TICK_MILLIS, hotkey_message, hotkey_message_for_cut_preview_overlay,
    hotkey_message_for_physical_status, hotkey_message_for_repeat, hotkey_message_for_status,
};
use super::io_actions::editor_auto_save_runtime_trace_line;
use super::large_project_smoke::{
    run_large_project_smoke, run_large_project_synthetic_waveform_smoke,
};
use super::message::AuditionKey;
use super::native_auth_flow::NativeAuthFlowState;
use super::native_capabilities::{
    DraftRedistributionAllocationPreview, DraftRedistributionMovePreview,
    DraftRedistributionPreview, DraftingRequestConfig, build_draft_generate_payload,
    build_draft_redistribute_payload, build_draft_transcribe_segment_payload,
    build_reviewer_payload,
};
use super::panel_persistence::PanelCollapseStorage;
use super::review_backend_client::ReviewBackendRequest;
use super::service_callbacks::{
    AdminRateReviewActionRequest, BabelServiceRequest, ClaimByProcessedTranscriptionRequest,
    ClaimNextFromQueueRequest, DropWorkerActionRequest, LoadLatestReviewActionRequest,
    LoadReviewActionRequest, SkipReviewActionRequest, SubmitReviewActionRequest,
    build_save_annotations_request,
};
use super::service_executor::{
    fixture_payload_message_for_request, is_dispatchable_babel_service_request,
    request_envelope_for_service_command,
};
use super::startup_route::{
    StartupRouteConfig, apply_startup_route_config, parse_startup_route_config,
};
use super::state::{
    AppSurface, HelperFocusRestore, NativeCapabilityModalTab, TimelineAutoscrollMode,
};
use super::tag_suggestions::{
    TagSuggestionKeyboardAction, TagSuggestionKind, tag_suggestions_for_draft,
    tag_suggestions_for_text,
};
use super::time_edit::{
    CommitTimeCellEditInput, CommitTimeCellEditResult, TimeEditState, TimeField, TimeRangeEdit,
    commit_time_cell_edit, format_recovered_time,
};
use super::updater::{DEFAULT_UPDATE_FEED_URL, UpdateCheckSummary, UpdatePhase};
use super::{
    DEFAULT_IMPORT_PATH, EditAnnotationDialog, Message, RuntimeGraphicsInfo, StudioState, update,
};

const CACHE_PROJECT_SEGMENT_COUNT: usize = 240;
const VIRTUALIZED_PROJECT_SEGMENT_COUNT: usize = 320;
const LONG_SPARSE_PROJECT_SEGMENT_COUNT: usize = 240;
const LONG_SPARSE_PROJECT_SPACING_SECONDS: f64 = 5.0;

fn large_two_lane_project(segment_count: usize) -> Project {
    large_two_lane_project_with_spacing(segment_count, 1.0)
}

fn cache_sized_two_lane_project() -> Project {
    large_two_lane_project(CACHE_PROJECT_SEGMENT_COUNT)
}

fn virtualized_two_lane_project() -> Project {
    large_two_lane_project(VIRTUALIZED_PROJECT_SEGMENT_COUNT)
}

fn long_sparse_waveform_project() -> Project {
    large_two_lane_project_with_spacing(
        LONG_SPARSE_PROJECT_SEGMENT_COUNT,
        LONG_SPARSE_PROJECT_SPACING_SECONDS,
    )
}

fn large_two_lane_project_with_spacing(segment_count: usize, spacing_seconds: f64) -> Project {
    let speakers = vec![
        Speaker {
            id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
        },
        Speaker {
            id: "speaker-2".to_string(),
            label: "Speaker 2".to_string(),
        },
    ];
    let lanes = vec![
        AudioLane {
            id: "track-1".to_string(),
            speaker_id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
            source_path: "audio/speaker-1.wav".to_string(),
        },
        AudioLane {
            id: "track-2".to_string(),
            speaker_id: "speaker-2".to_string(),
            label: "Speaker 2".to_string(),
            source_path: "audio/speaker-2.wav".to_string(),
        },
    ];

    let mut segments = Vec::with_capacity(segment_count);
    segments.push(Segment {
        id: "annotation-0000".to_string(),
        source_annotation_id: Some("annotation-0000".to_string()),
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds: 0.0,
        end_seconds: 12.0,
        text: "large project opening segment keeps playback inside one selected row".to_string(),
        metadata: Default::default(),
    });

    for index in 1..segment_count {
        let is_first_lane = index % 2 == 0;
        let lane_id = if is_first_lane { "track-1" } else { "track-2" };
        let speaker_id = if is_first_lane {
            "speaker-1"
        } else {
            "speaker-2"
        };
        let start_seconds = 20.0 + index as f64 * spacing_seconds;
        segments.push(Segment {
            id: format!("annotation-{index:04}"),
            source_annotation_id: Some(format!("annotation-{index:04}")),
            speaker_id: speaker_id.to_string(),
            lane_id: lane_id.to_string(),
            start_seconds,
            end_seconds: start_seconds + 0.72,
            text: format!("large project row {index} with unique transcript text"),
            metadata: Default::default(),
        });
    }

    Project {
        id: "large-project".to_string(),
        name: "Large Babel Project".to_string(),
        speakers,
        lanes,
        segments,
        metadata: Default::default(),
    }
}

fn decoded_audio_for_duration(duration_seconds: f64) -> DecodedAudio {
    let sample_rate = 80;
    let frame_count = (duration_seconds * sample_rate as f64).ceil() as usize;
    let channel = (0..frame_count)
        .map(|index| {
            let phase = (index % sample_rate as usize) as f64 / sample_rate as f64;
            (phase * 2.0 - 1.0) * 0.6
        })
        .collect::<Vec<_>>();
    DecodedAudio::new(sample_rate, vec![channel]).expect("large synthetic audio should be valid")
}

fn decoded_audio_with_speech_ranges(
    duration_seconds: f64,
    sample_rate: u32,
    ranges: &[(f64, f64)],
) -> DecodedAudio {
    let frame_count = (duration_seconds * sample_rate as f64).ceil() as usize;
    let channel = (0..frame_count)
        .map(|index| {
            let time = index as f64 / sample_rate as f64;
            if ranges
                .iter()
                .any(|(start, end)| time >= *start && time < *end)
            {
                0.1
            } else {
                0.0
            }
        })
        .collect::<Vec<_>>();
    DecodedAudio::new(sample_rate, vec![channel]).expect("test trim audio should be valid")
}

const STABLE_WORKFLOW_TASK_JSON: &str =
    include_str!("../../tests/fixtures/stable-workflow-task.json");
const BABEL_DAY_TO_DAY_CONTRACT_JSON: &str =
    include_str!("../../tests/fixtures/babel-day-to-day-contract.json");

fn stable_sample_state() -> StudioState {
    let mut state = StudioState::default();
    let workflow = StudioWorkflow::from_babel_json(STABLE_WORKFLOW_TASK_JSON)
        .expect("stable workflow fixture should import");
    state.replace_workflow(
        workflow,
        "Loaded native synthetic workflow test fixture.".to_string(),
    );
    state
}

fn review_action_final_with_local_audio() -> String {
    let mut value: serde_json::Value = serde_json::from_str(include_str!(
        "../../../studio-babel/tests/fixtures/review-action-final.json"
    ))
    .expect("review action fixture should parse");
    let uri_map = value
        .pointer_mut("/0/result/data/json/processedRecordingUriMap")
        .and_then(serde_json::Value::as_object_mut)
        .expect("review action fixture should include processed recording URIs");
    for (index, source) in uri_map.values_mut().enumerate() {
        *source = serde_json::Value::String(format!("audio/speaker_{}.wav", index + 1));
    }
    value.to_string()
}

fn enable_l2_reviewer_capability(state: &mut StudioState) {
    state.workflow.as_mut().unwrap().merge_project_metadata([(
        "babel.extra.package".to_string(),
        r#"{"action_level":2}"#.to_string(),
    )]);
}

fn babel_day_to_day_contract() -> serde_json::Value {
    serde_json::from_str(BABEL_DAY_TO_DAY_CONTRACT_JSON)
        .expect("Babel day-to-day contract should parse")
}

fn contract_string(contract: &serde_json::Value, path: &[&str]) -> String {
    let value = path.iter().fold(contract, |value, key| {
        value
            .get(*key)
            .unwrap_or_else(|| panic!("missing contract key `{key}`"))
    });
    value
        .as_str()
        .unwrap_or_else(|| panic!("contract path `{}` should be a string", path.join(".")))
        .to_string()
}

fn contract_usize(contract: &serde_json::Value, path: &[&str]) -> usize {
    let value = path.iter().fold(contract, |value, key| {
        value
            .get(*key)
            .unwrap_or_else(|| panic!("missing contract key `{key}`"))
    });
    value
        .as_u64()
        .unwrap_or_else(|| panic!("contract path `{}` should be an integer", path.join(".")))
        as usize
}

fn contract_string_array(contract: &serde_json::Value, path: &[&str]) -> Vec<String> {
    let value = path.iter().fold(contract, |value, key| {
        value
            .get(*key)
            .unwrap_or_else(|| panic!("missing contract key `{key}`"))
    });
    value
        .as_array()
        .unwrap_or_else(|| panic!("contract path `{}` should be an array", path.join(".")))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| {
                    panic!("contract path `{}` should contain strings", path.join("."))
                })
                .to_string()
        })
        .collect()
}

#[test]
fn default_import_path_points_to_existing_fixture() {
    assert!(Path::new(DEFAULT_IMPORT_PATH).exists());
}

#[test]
fn app_opens_on_dashboard_with_bundled_project_ready() {
    let state = StudioState::default();

    assert!(state.editor.is_some());
    assert_eq!(state.app_surface, AppSurface::Dashboard);
    assert!(state.show_dashboard());
    assert!(state.workflow.as_ref().unwrap().project().segments.len() >= 12);
}

#[test]
fn bundled_project_starts_with_recovered_babel_opening_not_stable_smoke_rows() {
    let state = StudioState::default();
    let project = state.project().expect("bundled sample project");
    let first = project
        .segments
        .iter()
        .min_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds))
        .expect("bundled sample should contain recovered rows");

    assert_eq!(first.id, "annotation-1");
    assert_eq!(first.start_seconds, 0.929);
    assert!(
        first
            .text
            .starts_with("Ты знаешь, э, есть у меня такое беспокойство")
    );
    assert!(!first.text.contains("Здравствуйте."));
    assert_eq!(
        project
            .metadata
            .get("babel.extra.annotation.annotation-1")
            .map(|extra| extra.contains("eaa5afc3-3d59-4267-b073-d390d0b48060")),
        Some(true)
    );
}

#[test]
fn native_editor_matches_babel_day_to_day_contract() {
    let contract = babel_day_to_day_contract();
    let mut state = StudioState::default();
    let project = state.project().expect("stable sample project");
    let editor = state.editor.as_ref().expect("editor session");

    for text in contract_string_array(&contract, &["identity", "firstMeaningfulTexts"]) {
        assert!(
            matches!(
                text.as_str(),
                "Copy Support ID" | "Hotkeys" | "All Tracks" | "Saved" | "Submit Review"
            ),
            "native contract should know the normal Babel editor surface text `{text}`"
        );
    }
    assert_eq!(
        project.segments.len(),
        contract_usize(&contract, &["transcript", "initialRowCount"])
    );

    let strip = editor.transport_strip(project);
    assert_eq!(
        strip.track_filter.label(),
        contract_string(&contract, &["transport", "trackFilter"])
    );
    assert_eq!(
        strip.save_label,
        contract_string(&contract, &["transport", "saveCleanLabel"])
    );
    assert_eq!(
        strip.submit_label,
        contract_string(&contract, &["transport", "submitLabel"])
    );
    assert_eq!(
        strip.play_label,
        contract_string(&contract, &["transport", "nativePlayLabel"])
    );
    assert_eq!(
        format!("{}x", editor.speed as i32),
        contract_string(&contract, &["transport", "speedLabel"])
    );
    let mut playing_editor = editor.clone();
    playing_editor.toggle_playback();
    assert_eq!(
        playing_editor.transport_strip(project).play_label,
        contract_string(&contract, &["transport", "nativePauseLabel"])
    );
    assert_eq!(
        editor.support_id_button(false).button_label,
        contract_string(&contract, &["support", "copyLabel"])
    );
    assert_eq!(
        editor.support_id_button(true).button_label,
        contract_string(&contract, &["support", "copiedLabel"])
    );
    assert_eq!(
        strip.submit.dialog_title,
        contract_string(&contract, &["submitConfirmation", "title"])
    );
    assert_eq!(
        strip.submit.dialog_description,
        contract_string(&contract, &["submitConfirmation", "body"])
    );
    let submit_buttons = contract_string_array(&contract, &["submitConfirmation", "buttons"]);
    assert_eq!(strip.submit.cancel_label, submit_buttons[0]);
    assert_eq!(strip.submit.confirm_label, submit_buttons[1]);
    assert_eq!(strip.submit.close_label, submit_buttons[2]);
    assert!(!studio_app::EditorSession::skip_chunk_control_available(
        project
    ));
    assert!(!studio_app::EditorSession::admin_flag_control_available(
        project
    ));
    assert!(state.script_context_panel(project).is_none());
    assert!(state.original_script_panel(project).is_none());

    let lanes = editor.timeline_lanes(project);
    assert_eq!(
        lanes
            .iter()
            .map(|lane| lane.label.clone())
            .collect::<Vec<_>>(),
        contract_string_array(&contract, &["timeline", "laneOrder"])
    );
    assert_eq!(
        lanes
            .iter()
            .map(|lane| lane.color_token.clone())
            .collect::<Vec<_>>(),
        contract_string_array(&contract, &["timeline", "laneColors"])
    );
    let default_region_contract = &contract["timeline"]["defaultVisibleRegion"];
    assert_eq!(
        lanes[1].label,
        default_region_contract["laneLabel"].as_str().unwrap()
    );
    assert!(lanes[1].blocks.iter().any(|block| block.is_transcription));

    let rows = editor.transcript_rows(project);
    assert_eq!(rows.len(), project.segments.len());
    for (index, expected) in contract["transcript"]["firstRows"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let row = &rows[index];
        assert_eq!(row.display_id, expected["displayId"].as_str().unwrap());
        assert_eq!(row.speaker_label, expected["speaker"].as_str().unwrap());
        assert_eq!(row.start_time, expected["start"].as_str().unwrap());
        assert_eq!(row.end_time, expected["end"].as_str().unwrap());
        if let Some(text) = expected["text"].as_str() {
            assert_eq!(row.text, text);
        }
        if let Some(prefix) = expected["textPrefix"].as_str() {
            assert!(row.text.starts_with(prefix));
        }
    }

    let menu = editor.transcript_action_menu(project, &rows[0]);
    let menu_labels = menu
        .items
        .iter()
        .filter_map(|entry| match entry {
            TranscriptActionMenuEntryView::Item(item) => Some(item.label.clone()),
            TranscriptActionMenuEntryView::Separator => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        menu_labels,
        contract_string_array(&contract, &["transcript", "rowMenuItems"])
    );
    for forbidden in contract_string_array(&contract, &["transcript", "rowMenuForbiddenItems"]) {
        assert!(!menu_labels.contains(&forbidden));
    }

    let _ = update(&mut state, Message::CopySupportId);
    assert!(state.support_id_copied);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .support_id_button(state.support_id_copied)
            .button_label,
        contract_string(&contract, &["support", "copiedLabel"])
    );
    let _ = update(&mut state, Message::ShowHotkeys);
    assert!(state.show_hotkeys_dialog);
    let _ = update(&mut state, Message::CloseHotkeys);
    assert!(!state.show_hotkeys_dialog);
    acknowledge_all_assertable_warnings(&mut state);
    let _ = update(&mut state, Message::RequestSubmitReview);
    assert!(state.show_submit_dialog);
    let submit = state.editor.as_ref().unwrap().submit_control(
        state.project().unwrap(),
        state.show_submit_dialog,
        state.submit_is_pending,
        state.submit_has_error,
    );
    assert!(submit.open);
    assert_eq!(
        submit.dialog_title,
        contract_string(&contract, &["submitConfirmation", "title"])
    );
    assert_eq!(
        submit.dialog_description,
        contract_string(&contract, &["submitConfirmation", "body"])
    );
    assert_eq!(submit.cancel_label, submit_buttons[0]);
    assert_eq!(submit.confirm_label, submit_buttons[1]);
    assert_eq!(submit.close_label, submit_buttons[2]);
    let _ = update(&mut state, Message::CancelSubmitReview);
    assert!(!state.show_submit_dialog);

    let inline_suffix = contract_string(&contract, &["transcript", "inlineEditDirtySuffix"]);
    let edited_segment_id = rows[0].segment_id.clone();
    let _ = update(
        &mut state,
        Message::SegmentTextEdited(
            edited_segment_id.clone(),
            TextEditAction::Edit(TextEdit::Paste(inline_suffix.clone().into())),
        ),
    );
    assert!(
        state
            .segment_drafts
            .get(&edited_segment_id)
            .unwrap()
            .text
            .contains(&inline_suffix)
    );
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .transport_strip(state.project().unwrap())
            .save_label,
        contract_string(&contract, &["transport", "saveDirtyLabel"])
    );

    let existing = state
        .project()
        .unwrap()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let hit_time = (existing.start_seconds + existing.end_seconds) / 2.0;
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            existing.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some(existing.id.clone()),
                time_seconds: hit_time,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(existing.lane_id));
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-speaker-2".to_string()),
    );
    assert!(state.create_region_dialog.is_none());
    assert!(state.edit_annotation_dialog.is_none());

    let original_count = state.project().unwrap().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);
    let dialog = state
        .create_region_dialog
        .as_ref()
        .expect("dragging empty waveform space should open Add Region");
    assert_eq!(dialog.mode, CreateRegionMode::Transcription);
    assert_eq!(dialog.selected_lane_id, "track-speaker-1");
    assert_eq!(dialog.text, "");
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert!(dialog.created_segment_id.is_some());
    assert_eq!(
        state.project().unwrap().segments.len(),
        original_count
            + contract_usize(
                &contract,
                &["createRegion", "createdSegmentCountDeltaOnDrag"]
            )
    );
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .transport_strip(state.project().unwrap())
            .save_label,
        contract_string(&contract, &["transport", "saveDirtyLabel"])
    );
    let _ = update(&mut state, Message::CancelCreateRegion);
    assert!(state.create_region_dialog.is_none());
    assert_eq!(state.project().unwrap().segments.len(), original_count + 1);

    open_create_region_dialog_from_waveform(&mut state, 13.0, 13.8);
    let saved_text = contract_string(&contract, &["createRegion", "savedText"]);
    let _ = update(
        &mut state,
        Message::CreateRegionTextChanged(saved_text.clone()),
    );
    let _ = update(&mut state, Message::SubmitCreateRegion);
    let selected_segment_id = state
        .editor
        .as_ref()
        .unwrap()
        .selected_segment_id
        .as_ref()
        .unwrap();
    let created = state
        .project()
        .unwrap()
        .segment(selected_segment_id)
        .expect("saved Add Region segment");
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.text, saved_text);
    assert!(state.create_region_dialog.is_none());
}

#[test]
fn runtime_bundled_sample_loads_recovered_fixture_waveforms_without_duplicate_assets() {
    let mut state = StudioState::default();

    let loaded = state.load_bundled_sample_waveforms_for_runtime();

    assert_eq!(loaded, 2);
    assert!(state.status.contains("Loaded 2 waveform tracks."));
    let project = state.project().expect("bundled sample project");
    let lanes = state.editor.as_ref().unwrap().timeline_lanes(project);
    assert_eq!(lanes.len(), 2);
    assert!(
        lanes
            .iter()
            .flat_map(|lane| lane.blocks.iter())
            .any(|block| block.peak_source == WaveformPeakSource::Audio)
    );
}

#[test]
fn runtime_bundled_sample_waveform_load_prewarms_initial_render_caches() {
    let mut state = StudioState::default();

    let loaded = state.load_bundled_sample_waveforms_for_runtime();

    assert_eq!(loaded, 2);
    let editor = state.editor.as_ref().expect("bundled sample editor");
    assert!(
        state.timeline_lanes_cache_miss_count() > 0,
        "loading recovered WAVs should prewarm the initial timeline lane snapshot before playback frames"
    );
    assert!(
        state.overview_waveform_cache_miss_count() > 0,
        "loading recovered WAVs should prewarm the initial overview waveform snapshot before playback frames"
    );
    for waveform in editor.lane_waveforms.values() {
        assert!(
            waveform.render_cache_stats().misses > 0,
            "loading recovered WAVs should build the initial decoded render model before playback"
        );
        assert!(
            waveform.cached_peak_resolution_count() > 0,
            "loading recovered WAVs should build decoded peak resolutions outside the first playback frame"
        );
    }
}

#[test]
fn runtime_bundled_sample_waveform_load_prewarns_native_audio_backend_before_first_play() {
    let mut state = StudioState::default();
    let prepare_count = state.audio_playback.prepare_count();
    let prepared_player_count = state.audio_playback.prepared_player_count();

    let loaded = state.load_bundled_sample_waveforms_for_runtime();

    assert_eq!(loaded, 2);
    assert!(
        state.audio_playback.prepare_count() > prepare_count,
        "loading recovered WAVs should open the native audio sink before the first visible play action"
    );
    assert!(
        state.audio_playback.prepared_player_count() > prepared_player_count,
        "loading recovered WAVs should connect reusable native players before the first visible play action"
    );
}

#[test]
fn transport_strip_cache_retains_paused_and_playing_snapshots_for_first_play_transition() {
    let state = stable_sample_state();
    let project = state.project().unwrap().clone();
    let paused_editor = state.editor.as_ref().unwrap().clone();
    let mut playing_editor = paused_editor.clone();
    playing_editor.is_playing = true;

    let paused = state.transport_strip_snapshot_for_render(&project, &paused_editor);
    let playing = state.transport_strip_snapshot_for_render(&project, &playing_editor);
    assert_ne!(paused.play_label, playing.play_label);
    let misses_after_variants = state.transport_strip_cache_miss_count();

    let paused_again = state.transport_strip_snapshot_for_render(&project, &paused_editor);
    let playing_again = state.transport_strip_snapshot_for_render(&project, &playing_editor);

    assert_eq!(
        state.transport_strip_cache_miss_count(),
        misses_after_variants,
        "first play should not evict the paused transport snapshot or rebuild it on the next paused render"
    );
    assert!(std::sync::Arc::ptr_eq(&paused, &paused_again));
    assert!(std::sync::Arc::ptr_eq(&playing, &playing_again));
}

#[test]
fn runtime_bundled_sample_waveform_load_prewarns_first_play_transport_snapshot() {
    let mut state = StudioState::default();

    let loaded = state.load_bundled_sample_waveforms_for_runtime();

    assert_eq!(loaded, 2);
    let project = state.project().unwrap().clone();
    let paused_editor = state.editor.as_ref().unwrap().clone();
    let mut playing_editor = paused_editor.clone();
    playing_editor.is_playing = true;
    let misses_after_load = state.transport_strip_cache_miss_count();

    let _ = state.transport_strip_snapshot_for_render(&project, &paused_editor);
    let _ = state.transport_strip_snapshot_for_render(&project, &playing_editor);

    assert_eq!(
        state.transport_strip_cache_miss_count(),
        misses_after_load,
        "loading recovered WAVs should prewarm both paused and first-playing transport snapshots"
    );
}

#[test]
fn runtime_startup_playback_smoke_starts_recovered_two_track_audio() {
    let mut state = StudioState::default();

    assert!(state.start_recovered_audio_playback_smoke_for_runtime());

    let playback = state
        .runtime_audio_playback_info
        .as_ref()
        .expect("recovered startup playback should record trace info");
    assert_eq!(playback.track_count, 2);
    assert_eq!(playback.start_seconds, 0.0);
    assert_eq!(playback.speed, 1.0);
    assert_eq!(
        playback.runtime_log_line(),
        "Transcription Studio audio playback: tracks=2 start=0.000s speed=1.00x"
    );
    assert!(state.editor.as_ref().unwrap().is_playing);

    let _ = update(&mut state, Message::TogglePlayback);
}

#[test]
fn delayed_startup_playback_smoke_waits_for_warm_window_frames_before_playing() {
    let mut state = StudioState::default();

    assert_eq!(state.load_bundled_sample_waveforms_for_runtime(), 2);
    state.schedule_startup_playback_smoke_after_frames(2);
    assert!(state.startup_playback_smoke_delay_pending());
    assert!(!state.editor.as_ref().unwrap().is_playing);

    let start = Instant::now();
    state.advance_transport_tick(start);
    assert!(state.startup_playback_smoke_delay_pending());
    assert!(!state.editor.as_ref().unwrap().is_playing);
    assert!(state.runtime_audio_playback_info.is_none());

    state.advance_transport_tick(start + Duration::from_millis(16));
    assert!(!state.startup_playback_smoke_delay_pending());
    assert!(state.editor.as_ref().unwrap().is_playing);
    assert_eq!(
        state
            .runtime_audio_playback_info
            .as_ref()
            .expect("delayed smoke should start recovered playback")
            .track_count,
        2
    );

    let _ = update(&mut state, Message::TogglePlayback);
}

#[test]
fn delayed_startup_playback_smoke_subscribes_to_frames_without_changing_default_playback() {
    let app_source = include_str!("mod.rs");
    let hotkey_source = include_str!("hotkeys.rs");
    let state_source = include_str!("state.rs");

    assert!(state_source.contains("STUDIO_SMOKE_PLAYBACK_DELAY_FRAMES"));
    assert!(state_source.contains("startup_playback_smoke_delay_frames_from_env()"));
    assert!(state_source.contains("schedule_startup_playback_smoke_after_frames("));
    assert!(state_source.contains("startup_playback_smoke_delay_pending("));
    assert!(state_source.contains("advance_startup_playback_smoke_delay("));
    assert!(app_source.contains("startup_playback_smoke_delay_frames_from_env"));
    assert!(hotkey_source.contains("state.startup_playback_smoke_delay_pending()"));
}

#[test]
fn context_panel_collapse_state_persists_across_native_app_restart() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let storage_path = temp.path().join("ui-local-storage.json");
    let mut storage =
        PanelCollapseStorage::load(&storage_path).expect("missing settings should load empty");

    assert!(!storage.is_collapsed(SCRIPT_CONTEXT_STORAGE_KEY));

    let toggle = reduce_collapsible_panel_toggle(SCRIPT_CONTEXT_STORAGE_KEY, false);
    storage
        .apply_tts_panel_effects(&toggle.effects)
        .expect("toggle effect should persist");

    let restarted =
        PanelCollapseStorage::load(&storage_path).expect("settings should reload after restart");
    let context = BTreeMap::from([("character".to_string(), Some("Alice".to_string()))]);
    let script_context = build_script_context_panel_model(ScriptContextPanelInput {
        emotion: None,
        submission_id: Some("submission-123".to_string()),
        context: Some(context),
        hidden: restarted.is_collapsed(SCRIPT_CONTEXT_STORAGE_KEY),
        is_rtl: false,
        copied_submission_id: false,
    })
    .expect("script context should render from Babel metadata");

    assert!(script_context.panel.hidden);
    assert!(!script_context.panel.content_visible);
    assert_eq!(script_context.panel.toggle_tooltip, "Show Script Context");
}

#[test]
fn context_panel_collapse_storage_uses_recovered_true_string_semantics() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let storage_path = temp.path().join("ui-local-storage.json");
    std::fs::write(
        &storage_path,
        serde_json::json!({
            SCRIPT_CONTEXT_STORAGE_KEY: "TRUE",
            ORIGINAL_SCRIPT_STORAGE_KEY: "true"
        })
        .to_string(),
    )
    .expect("seed settings should write");

    let storage = PanelCollapseStorage::load(&storage_path).expect("settings should load");
    let original_script = build_original_script_panel_model(studio_app::OriginalScriptPanelInput {
        script: "Original line".to_string(),
        hidden: storage.is_collapsed(ORIGINAL_SCRIPT_STORAGE_KEY),
        is_rtl: false,
    });

    assert!(!storage.is_collapsed(SCRIPT_CONTEXT_STORAGE_KEY));
    assert!(original_script.panel.hidden);
}

#[test]
fn context_panel_toggle_message_updates_visible_model_and_persists_storage() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let storage_path = temp.path().join("ui-local-storage.json");
    let mut state = StudioState {
        panel_collapse_storage: PanelCollapseStorage::load(&storage_path)
            .expect("missing settings should load empty"),
        script_context_panel_hidden: false,
        original_script_panel_hidden: false,
        ..stable_sample_state()
    };
    state.workflow.as_mut().unwrap().merge_project_metadata([(
        "studio.babel.scriptContext.character".to_string(),
        "Alice".to_string(),
    )]);
    let project = state.project().unwrap().clone();

    let initial = state
        .script_context_panel(&project)
        .expect("real script context metadata should render script context panel");
    assert!(!initial.panel.hidden);

    let _ = update(
        &mut state,
        Message::ToggleContextPanel(SCRIPT_CONTEXT_STORAGE_KEY.to_string()),
    );

    let toggled = state
        .script_context_panel(&project)
        .expect("script context panel should remain renderable after toggle");
    assert!(toggled.panel.hidden);
    let restarted =
        PanelCollapseStorage::load(&storage_path).expect("settings should reload after toggle");
    assert!(restarted.is_collapsed(SCRIPT_CONTEXT_STORAGE_KEY));
}

#[test]
fn context_panels_derive_native_models_from_babel_metadata() {
    let state = StudioState::default();
    let project = state.project().unwrap();

    assert!(
        state.script_context_panel(project).is_none(),
        "the bundled transcription task matches Babel: no Script Context panel without real context"
    );
    assert!(
        state.original_script_panel(project).is_none(),
        "the bundled transcription task should not fabricate Original Script from segment text"
    );
}

#[test]
fn context_panel_icons_reuse_static_svg_handles() {
    let source = include_str!("components/context_panels.rs");
    let svg_icon_fn = source
        .split("fn view_svg_icon")
        .nth(1)
        .expect("context panel icon helper should exist");

    assert!(source.contains("COPY_ICON_HANDLE"));
    assert!(source.contains("CHECK_ICON_HANDLE"));
    assert!(source.contains("SHOW_ICON_HANDLE"));
    assert!(source.contains("HIDE_ICON_HANDLE"));
    assert!(svg_icon_fn.contains("icon: SvgHandle"));
    assert!(!svg_icon_fn.contains("SvgHandle::from_memory(icon)"));
}

#[test]
fn toolbar_uses_recovered_babel_speed_and_zoom_values() {
    assert_eq!(
        babel_speed_labels(),
        vec!["0.25x", "0.5x", "0.75x", "1x", "1.5x", "2x"]
    );
    assert_eq!(babel_slider_from_zoom_factor(1.0), 200.0);
    assert_eq!(zoom_factor_from_babel_slider(10.0), 0.05);
    assert_eq!(zoom_factor_from_babel_slider(2000.0), 10.0);
}

#[test]
fn toolbar_runtime_trace_lines_report_zoom_and_speed_changes() {
    assert_eq!(
        super::actions::toolbar_zoom_runtime_trace_line(1.0, 2.5),
        "Transcription Studio toolbar: action=zoom previous=1.000 current=2.500"
    );
    assert_eq!(
        super::actions::toolbar_speed_runtime_trace_line(1.0, 1.25),
        "Transcription Studio toolbar: action=speed previous=1.00x current=1.25x"
    );
}

#[test]
fn timeline_drag_runtime_trace_lines_report_loop_scrollbar_and_vertical_zoom() {
    let range = TimeRange {
        start_seconds: 1.2,
        end_seconds: 3.4,
    };
    let viewport = TimeRange {
        start_seconds: 10.0,
        end_seconds: 15.5,
    };

    assert_eq!(
        super::state::timeline_loop_runtime_trace_line(LoopStripHitTarget::RangeBody, range, true),
        "Transcription Studio timeline drag: action=loop target=RangeBody start=1.200 end=3.400 enabled=true"
    );
    assert_eq!(
        super::state::timeline_scrollbar_runtime_trace_line(viewport),
        "Transcription Studio timeline drag: action=scrollbar viewport=10.000-15.500s"
    );
    assert_eq!(
        super::state::timeline_vertical_zoom_runtime_trace_line("track-speaker-1", 2.5),
        "Transcription Studio timeline drag: action=vertical-zoom lane=track-speaker-1 value=2.500"
    );
    assert_eq!(
        super::state::timeline_pointer_capture_release_runtime_trace_line(true, false, true, false),
        "Transcription Studio timeline drag: action=captured-release loop=true scrollbar=false vertical-zoom=true waveform=false"
    );
}

#[test]
fn timeline_wheel_runtime_trace_lines_report_zoom_pan_and_noop_routes() {
    let before = TimeRange {
        start_seconds: 10.0,
        end_seconds: 15.0,
    };
    let after = TimeRange {
        start_seconds: 12.5,
        end_seconds: 17.5,
    };

    assert_eq!(
        super::actions::timeline_wheel_runtime_trace_line(
            "command-zoom",
            0.0,
            -120.0,
            1.0,
            1.1,
            before,
            before
        ),
        "Transcription Studio timeline wheel: route=command-zoom delta_x=0.000 delta_y=-120.000 zoom=1.000->1.100 viewport=10.000-15.000s->10.000-15.000s"
    );
    assert_eq!(
        super::actions::timeline_wheel_runtime_trace_line(
            "shift-pan",
            0.0,
            120.0,
            2.0,
            2.0,
            before,
            after
        ),
        "Transcription Studio timeline wheel: route=shift-pan delta_x=0.000 delta_y=120.000 zoom=2.000->2.000 viewport=10.000-15.000s->12.500-17.500s"
    );
    assert_eq!(
        super::actions::timeline_wheel_runtime_trace_line(
            "horizontal-pan",
            120.0,
            0.0,
            2.0,
            2.0,
            before,
            after
        ),
        "Transcription Studio timeline wheel: route=horizontal-pan delta_x=120.000 delta_y=0.000 zoom=2.000->2.000 viewport=10.000-15.000s->12.500-17.500s"
    );
    assert_eq!(
        super::actions::timeline_wheel_runtime_trace_line(
            "plain-vertical-noop",
            0.0,
            -120.0,
            1.0,
            1.0,
            before,
            before
        ),
        "Transcription Studio timeline wheel: route=plain-vertical-noop delta_x=0.000 delta_y=-120.000 zoom=1.000->1.000 viewport=10.000-15.000s->10.000-15.000s"
    );
}

#[test]
fn transcript_action_runtime_trace_lines_report_toggle_state() {
    assert_eq!(
        super::actions::transcript_action_toggle_runtime_trace_line("annotation-1", false, true),
        "Transcription Studio action menu: segment=annotation-1 open=false->true"
    );
    assert_eq!(
        super::actions::transcript_action_toggle_runtime_trace_line("annotation-1", true, false),
        "Transcription Studio action menu: segment=annotation-1 open=true->false"
    );
}

#[test]
fn row_structure_action_runtime_trace_lines_report_command_effect() {
    assert_eq!(
        super::segment_actions::structure::row_structure_action_runtime_trace_line(
            "create-before",
            "annotation-3",
            Some("annotation-3__studio_1"),
            Some("annotation-3__studio_1"),
            25,
            true,
            None,
        ),
        "Transcription Studio row action: action=create-before segment=annotation-3 result=annotation-3__studio_1 selected=annotation-3__studio_1 segments=25 dirty=true menu=none"
    );
    assert_eq!(
        super::segment_actions::structure::row_structure_action_runtime_trace_line(
            "delete",
            "annotation-1",
            None,
            Some("annotation-2"),
            23,
            true,
            Some("annotation-9"),
        ),
        "Transcription Studio row action: action=delete segment=annotation-1 result=- selected=annotation-2 segments=23 dirty=true menu=annotation-9"
    );
}

#[test]
fn problem_navigation_runtime_trace_lines_report_selected_problem_row() {
    assert_eq!(
        super::actions::problem_navigation_runtime_trace_line(
            1,
            Some("annotation-1"),
            Some("annotation-4"),
            0,
            5,
        ),
        "Transcription Studio problem navigation: direction=1 previous=annotation-1 selected=annotation-4 index=0 problems=5"
    );
    assert_eq!(
        super::actions::problem_navigation_runtime_trace_line(-1, None, None, 0, 0),
        "Transcription Studio problem navigation: direction=-1 previous=- selected=- index=0 problems=0"
    );
}

#[test]
fn create_region_runtime_trace_lines_report_modal_state() {
    let mut dialog = CreateRegionDialog::new_annotation("track-speaker-1".to_string(), 12.0, 12.8);
    dialog.primary_selection = "background-noise".to_string();
    dialog.secondary_selection = "rustle".to_string();
    dialog.intensity = Some(3);

    assert_eq!(
        super::create_region::create_region_runtime_trace_line("submit", &dialog),
        "Transcription Studio create region: action=submit mode=annotation lane=track-speaker-1 start=12.000 end=12.800 enabled=true primary=background-noise secondary=rustle intensity=3 text_len=0"
    );

    let mut transcription = CreateRegionDialog::new("track-speaker-2".to_string(), 3.5, 5.1);
    transcription.text = "new transcript region".to_string();

    assert_eq!(
        super::create_region::create_region_runtime_trace_line("open", &transcription),
        "Transcription Studio create region: action=open mode=transcription lane=track-speaker-2 start=3.500 end=5.100 enabled=true primary= secondary= intensity=none text_len=21"
    );
}

#[test]
fn edit_annotation_runtime_trace_lines_report_popover_state() {
    let state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment_index = project
        .segments
        .iter()
        .position(|segment| segment.id == "annotation-2")
        .unwrap();
    project.segments[segment_index]
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    project.segments[segment_index].metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    project.segments[segment_index]
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    let segment = project.segments[segment_index].clone();
    let dialog = EditAnnotationDialog::for_project_segment(&project, &segment);

    assert_eq!(
        super::edit_annotation::edit_annotation_runtime_trace_line("open", &dialog),
        "Transcription Studio edit annotation: action=open segment=annotation-2 title=Edit Annotation hierarchical=true primary=background-noise secondary=rustle intensity=3 save_ready=true text_len=11"
    );
}

#[test]
fn copying_support_id_sets_recovered_babel_copied_state_until_reset() {
    let mut state = stable_sample_state();

    assert!(!state.support_id_copied);

    let _ = update(&mut state, Message::CopySupportId);

    assert!(state.support_id_copied);
    assert_eq!(
        state.status,
        "Support ID copied: 9edb22ab-0993-41f3-926e-90779da4c624"
    );

    let _ = update(&mut state, Message::ResetSupportIdCopied);

    assert!(!state.support_id_copied);
}

#[test]
fn toolbar_support_id_button_uses_recovered_babel_copied_label() {
    let toolbar_source = include_str!("components/toolbar.rs");

    assert!(toolbar_source.contains("editor.support_id_button(state.support_id_copied)"));
    assert!(toolbar_source.contains("support_id.button_label"));
    assert!(toolbar_source.contains("support_id.tooltip_content"));
    assert!(toolbar_source.contains("tooltip("));
}

#[test]
fn overview_toolbar_click_outside_window_seeks_like_recovered_babel_minimap() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| editor.set_zoom(&project, 4.0));
    let mut expected_editor = state.editor.as_ref().unwrap().clone();
    expected_editor.seek_overview_fraction(&project, 0.85);

    let _ = update(
        &mut state,
        Message::OverviewPointerMoved {
            fraction: 0.85,
            width: 100.0,
        },
    );
    let _ = update(&mut state, Message::OverviewPressed);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, expected_editor.playhead_seconds);
    assert_eq!(
        editor.selected_segment_id,
        expected_editor.selected_segment_id
    );
    assert_eq!(editor.viewport, expected_editor.viewport);
}

#[test]
fn overview_toolbar_viewport_drag_pans_window_without_moving_playhead() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.select_segment(&project, "annotation-17");
        editor.set_zoom(&project, 4.0);
    });
    let playhead = state.editor.as_ref().unwrap().playhead_seconds;
    let initial_overview = state.editor.as_ref().unwrap().overview_waveform(&project);
    let initial_drag_fraction = (initial_overview.viewport_left_percent
        + initial_overview.viewport_width_percent / 2.0)
        / 100.0;
    let mut expected_editor = state.editor.as_ref().unwrap().clone();
    expected_editor.set_overview_viewport_start_fraction(&project, initial_drag_fraction);
    expected_editor.set_overview_viewport_start_fraction(&project, 0.45);

    let _ = update(
        &mut state,
        Message::OverviewPointerMoved {
            fraction: initial_drag_fraction,
            width: 100.0,
        },
    );
    let _ = update(&mut state, Message::OverviewPressed);
    let _ = update(
        &mut state,
        Message::OverviewPointerMoved {
            fraction: 0.45,
            width: 100.0,
        },
    );
    let _ = update(&mut state, Message::OverviewReleased);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, playhead);
    assert_eq!(editor.viewport, expected_editor.viewport);
}

#[test]
fn overview_toolbar_viewport_drag_continues_after_pointer_leaves_minimap() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.select_segment(&project, "annotation-17");
        editor.set_zoom(&project, 4.0);
    });
    let playhead = state.editor.as_ref().unwrap().playhead_seconds;
    let initial_overview = state.editor.as_ref().unwrap().overview_waveform(&project);
    let initial_drag_fraction = (initial_overview.viewport_left_percent
        + initial_overview.viewport_width_percent / 2.0)
        / 100.0;
    let mut expected_editor = state.editor.as_ref().unwrap().clone();
    expected_editor.set_overview_viewport_start_fraction(&project, initial_drag_fraction);
    expected_editor.set_overview_viewport_start_fraction(&project, 0.45);

    let _ = update(
        &mut state,
        Message::GlobalPointerMoved(Point::new(initial_drag_fraction * 200.0, 12.0)),
    );
    let _ = update(
        &mut state,
        Message::OverviewPointerMoved {
            fraction: initial_drag_fraction,
            width: 200.0,
        },
    );
    let _ = update(&mut state, Message::OverviewPressed);
    let _ = update(
        &mut state,
        Message::GlobalPointerMoved(Point::new(90.0, -18.0)),
    );
    let _ = update(&mut state, Message::OverviewReleased);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, playhead);
    assert_eq!(editor.viewport, expected_editor.viewport);
}

#[test]
fn command_wheel_over_timeline_uses_babel_zoom_delta_and_clamps() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::COMMAND),
    );
    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 0.0,
            delta_y: -10.0,
        },
    );

    assert_eq!(state.editor.as_ref().unwrap().zoom, 1.1);

    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 0.0,
            delta_y: 10_000.0,
        },
    );

    assert_eq!(state.editor.as_ref().unwrap().zoom, 0.05);
}

#[test]
fn command_wheel_zoom_keeps_hovered_waveform_time_at_the_same_screen_fraction() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let before = state.editor.as_ref().unwrap().viewport;
    let anchor_fraction = 0.25;
    let anchor_time = before.start_seconds + before.duration_seconds() * anchor_fraction;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::COMMAND),
    );
    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 0.0,
            delta_y: -10.0,
        },
    );

    let editor = state.editor.as_ref().unwrap();
    let after_fraction =
        (anchor_time - editor.viewport.start_seconds) / editor.viewport.duration_seconds();
    assert_eq!(editor.zoom, 2.1);
    assert!((after_fraction - anchor_fraction).abs() < 0.001);
}

#[test]
fn plain_timeline_wheel_does_not_change_zoom() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 0.0,
            delta_y: -10.0,
        },
    );

    assert_eq!(state.editor.as_ref().unwrap().zoom, 1.0);
}

#[test]
fn timeline_wheel_event_preserves_touchpad_horizontal_delta() {
    let timeline_source = include_str!("components/timeline/mod.rs");
    let message_source = include_str!("message.rs");

    assert!(message_source.contains("TimelineWheelScrolled"));
    assert!(message_source.contains("delta_x: f32"));
    assert!(message_source.contains("delta_y: f32"));
    assert!(timeline_source.contains("mouse::ScrollDelta::Lines { x, y }"));
    assert!(timeline_source.contains("mouse::ScrollDelta::Pixels { x, y }"));
    assert!(timeline_source.contains("Message::TimelineWheelScrolled { delta_x, delta_y }"));
}

#[test]
fn shift_wheel_over_timeline_side_scrolls_babel_viewport_without_zooming() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let before = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::SHIFT),
    );
    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 0.0,
            delta_y: 10.0,
        },
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.zoom, 2.0);
    assert!(editor.viewport.start_seconds > before.start_seconds);
    assert!((editor.viewport.duration_seconds() - before.duration_seconds()).abs() < 0.001);
}

#[test]
fn horizontal_touchpad_wheel_over_timeline_pans_babel_viewport_without_zooming() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let before = state.editor.as_ref().unwrap().viewport;
    let playhead = state.editor.as_ref().unwrap().playhead_seconds;

    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 10.0,
            delta_y: 0.0,
        },
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.zoom, 2.0);
    assert_eq!(editor.playhead_seconds, playhead);
    assert!(editor.viewport.start_seconds > before.start_seconds);
    assert!((editor.viewport.duration_seconds() - before.duration_seconds()).abs() < 0.001);
}

#[test]
fn middle_mouse_quick_click_releases_into_browser_style_timeline_autoscroll() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let before = state.editor.as_ref().unwrap().viewport;
    let playhead = state.editor.as_ref().unwrap().playhead_seconds;
    let selected_segment_id = state.editor.as_ref().unwrap().selected_segment_id.clone();
    let anchor_time = before.start_seconds + before.duration_seconds() * 0.5;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::TimelineAutoscrollPressed("track-speaker-1".to_string()),
    );
    assert_eq!(
        state
            .active_timeline_autoscroll
            .as_ref()
            .map(|autoscroll| autoscroll.mode),
        Some(TimelineAutoscrollMode::Held)
    );

    let _ = update(&mut state, Message::TimelineAutoscrollReleased);
    assert_eq!(
        state
            .active_timeline_autoscroll
            .as_ref()
            .map(|autoscroll| autoscroll.mode),
        Some(TimelineAutoscrollMode::Click)
    );

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time + 0.75,
            },
        ),
    );
    assert_eq!(state.editor.as_ref().unwrap().viewport, before);

    let tick = Instant::now();
    let _ = update(&mut state, Message::TransportTick(tick));
    let _ = update(
        &mut state,
        Message::TransportTick(tick + Duration::from_millis(64)),
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, playhead);
    assert_eq!(editor.selected_segment_id, selected_segment_id);
    assert!(state.active_waveform_create_drag.is_none());
    assert!(state.active_waveform_drag.is_none());
    assert!(editor.viewport.start_seconds > before.start_seconds);
    assert!((editor.viewport.duration_seconds() - before.duration_seconds()).abs() < 0.001);

    let after_autoscroll = editor.viewport;
    let _ = update(&mut state, Message::EscapePressed);
    assert!(state.active_timeline_autoscroll.is_none());
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time + 1.5,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::TransportTick(tick + Duration::from_millis(128)),
    );
    assert_eq!(state.editor.as_ref().unwrap().viewport, after_autoscroll);
}

#[test]
fn middle_mouse_hold_autoscrolls_until_middle_release_after_motion() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let before = state.editor.as_ref().unwrap().viewport;
    let anchor_time = before.start_seconds + before.duration_seconds() * 0.5;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::TimelineAutoscrollPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time + 0.75,
            },
        ),
    );
    assert_eq!(
        state
            .active_timeline_autoscroll
            .as_ref()
            .map(|autoscroll| (autoscroll.mode, autoscroll.moved_while_held)),
        Some((TimelineAutoscrollMode::Held, true))
    );

    let tick = Instant::now();
    let _ = update(&mut state, Message::TransportTick(tick));
    let _ = update(
        &mut state,
        Message::TransportTick(tick + Duration::from_millis(64)),
    );
    assert!(state.editor.as_ref().unwrap().viewport.start_seconds > before.start_seconds);

    let after_hold_scroll = state.editor.as_ref().unwrap().viewport;
    let _ = update(&mut state, Message::TimelineAutoscrollReleased);
    assert!(state.active_timeline_autoscroll.is_none());
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: anchor_time + 1.5,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::TransportTick(tick + Duration::from_millis(128)),
    );
    assert_eq!(state.editor.as_ref().unwrap().viewport, after_hold_scroll);
}

#[test]
fn waveform_scrollbar_drag_pans_shared_babel_viewport_without_moving_playhead() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let before = state.editor.as_ref().unwrap().viewport;
    let playhead = state.editor.as_ref().unwrap().playhead_seconds;

    let _ = update(&mut state, Message::TimelineScrollbarHovered(0.0));
    let _ = update(&mut state, Message::TimelineScrollbarPressed);
    let _ = update(&mut state, Message::TimelineScrollbarHovered(0.45));
    let _ = update(&mut state, Message::TimelineScrollbarReleased);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, playhead);
    assert!((editor.viewport.duration_seconds() - before.duration_seconds()).abs() < 0.001);
    assert!(editor.viewport.start_seconds > before.start_seconds);
}

#[test]
fn lane_vertical_zoom_slider_drag_updates_zoom_without_header_horizontal_slider() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| editor.set_lane_vertical_zoom("track-speaker-2", 7.5));
    let before = state
        .editor
        .as_ref()
        .unwrap()
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap()
        .vertical_zoom;

    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: 0.9,
            height: 100.0,
        },
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPressed("track-speaker-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: 0.1,
            height: 100.0,
        },
    );
    let _ = update(&mut state, Message::LaneVerticalZoomSliderReleased);

    let after = state
        .editor
        .as_ref()
        .unwrap()
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap()
        .vertical_zoom;
    assert_ne!(before, after);
    assert!(after > before);
}

#[test]
fn lane_vertical_zoom_slider_drag_continues_after_pointer_leaves_slider_rail() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| editor.set_lane_vertical_zoom("track-speaker-2", 7.5));
    let before = state
        .editor
        .as_ref()
        .unwrap()
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap()
        .vertical_zoom;

    let _ = update(
        &mut state,
        Message::GlobalPointerMoved(Point::new(6.0, 90.0)),
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: 0.9,
            height: 100.0,
        },
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPressed("track-speaker-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::GlobalPointerMoved(Point::new(-44.0, 10.0)),
    );
    let _ = update(&mut state, Message::LaneVerticalZoomSliderReleased);

    let after = state
        .editor
        .as_ref()
        .unwrap()
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap()
        .vertical_zoom;
    assert_ne!(before, after);
    assert!((after - 900.03).abs() < 0.01);
}

#[test]
fn global_pointer_release_finishes_captured_timeline_control_drags() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let _ = update(&mut state, Message::TimelineScrollbarHovered(0.0));
    let _ = update(&mut state, Message::TimelineScrollbarPressed);
    assert!(state.timeline_scrollbar_drag_offset_fraction.is_some());

    let _ = update(&mut state, Message::GlobalPointerReleased);
    assert!(state.timeline_scrollbar_drag_offset_fraction.is_none());

    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: 0.5,
            height: 100.0,
        },
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPressed("track-speaker-2".to_string()),
    );
    assert_eq!(
        state.active_lane_vertical_zoom_slider_id.as_deref(),
        Some("track-speaker-2")
    );

    let _ = update(&mut state, Message::GlobalPointerReleased);
    assert!(state.active_lane_vertical_zoom_slider_id.is_none());

    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::Empty,
            time_seconds: 2.0,
        }),
    );
    let _ = update(&mut state, Message::LoopStripPressed);
    assert!(state.active_loop_drag.is_some());

    let _ = update(&mut state, Message::GlobalPointerReleased);
    assert!(state.active_loop_drag.is_none());
}

#[test]
fn window_unfocus_cancels_captured_pointer_drags_and_audition_holds() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::COMMAND),
    );
    let _ = update(&mut state, Message::TimelineScrollbarHovered(0.0));
    let _ = update(&mut state, Message::TimelineScrollbarPressed);
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: 0.5,
            height: 100.0,
        },
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPressed("track-speaker-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::Empty,
            time_seconds: 2.0,
        }),
    );
    let _ = update(&mut state, Message::LoopStripPressed);
    let _ = update(
        &mut state,
        Message::AuditionKeyPressed {
            key: AuditionKey::BeforeStart,
            release_token: "Character(\"q\")".to_string(),
        },
    );

    assert!(state.timeline_scrollbar_drag_offset_fraction.is_some());
    assert!(state.active_lane_vertical_zoom_slider_id.is_some());
    assert!(state.active_loop_drag.is_some());
    assert!(state.held_audition_keys.contains(&AuditionKey::BeforeStart));
    assert!(!state.audition_key_release_tokens.is_empty());
    assert!(state.keyboard_modifiers.command());

    let _ = update(&mut state, Message::WindowUnfocused);

    assert!(state.timeline_scrollbar_drag_offset_fraction.is_none());
    assert!(state.active_lane_vertical_zoom_slider_id.is_none());
    assert!(state.active_loop_drag.is_none());
    assert!(state.held_audition_keys.is_empty());
    assert!(state.audition_key_release_tokens.is_empty());
    assert!(state.keyboard_modifiers.is_empty());
}

#[test]
fn selecting_diff_compare_action_updates_metadata_without_dirtying_editor() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::DiffCompareActionChanged("l0".to_string()),
    );

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(
        project
            .metadata
            .get("studio.babel.diff.selectedCompareActionId")
            .map(String::as_str),
        Some("l0")
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn toggling_diff_reviewers_updates_metadata_without_dirtying_editor() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::ToggleDiffReviewers);

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(
        project
            .metadata
            .get("studio.babel.diff.showReviewers")
            .map(String::as_str),
        Some("true")
    );
    assert!(
        state
            .editor
            .as_ref()
            .unwrap()
            .diff_toolbar(project)
            .reviewers_checked
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);

    let _ = update(&mut state, Message::ToggleDiffReviewers);

    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.diff.showReviewers")
            .map(String::as_str),
        Some("false")
    );
}

#[test]
fn diff_workbench_startup_smoke_exercises_transcription_runtime_paths() {
    let mut state = stable_sample_state();

    let reports = run_diff_workbench_smoke(&mut state, DiffWorkbenchSmokeMode::Transcription);

    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.mode, "transcription");
    assert!(report.diff_checked);
    assert_eq!(report.selected_compare_action_id.as_deref(), Some("l1"));
    assert_eq!(report.reviewers_checked, Some(false));
    assert!(!report.editor_dirty);
    assert_eq!(report.row_count, 6);
    assert_eq!(
        report.changed_marker.as_deref(),
        Some("Changed: text, timing, track")
    );
    assert_eq!(report.changed_badge.as_deref(), Some("1 change"));
    assert_eq!(
        report.changed_tokens,
        vec!["removed:selected", "added:edited", "unchanged:text"]
    );
    assert!(report.modified_deleted_visible);
    assert!(report.annotation_ghost_visible);
    assert_eq!(
        report.annotation_status.as_deref(),
        Some("subcategory-mismatch")
    );
    assert_eq!(report.avg_shift_value.as_deref(), Some("42ms"));
    assert_eq!(report.word_diff_value.as_deref(), Some("12.35%"));
    assert_eq!(
        report.timestamp_tooltip_title.as_deref(),
        Some("Timestamp Accuracy")
    );
    assert_eq!(
        report.timestamp_tolerance_labels,
        vec!["+/-50ms", "+/-100ms", "+/-200ms"]
    );
    assert!(report.runtime_trace_line().contains("mode=transcription"));

    let project = state.project().unwrap();
    assert_eq!(
        project
            .metadata
            .get("studio.babel.diff.selectedCompareActionId")
            .map(String::as_str),
        Some("l1")
    );
    assert_eq!(
        project
            .metadata
            .get("studio.babel.diff.showReviewers")
            .map(String::as_str),
        Some("false")
    );
    assert!(state.editor.as_ref().unwrap().diff_view);
}

#[test]
fn diff_workbench_startup_smoke_exercises_annotation_runtime_paths() {
    let mut state = stable_sample_state();

    let reports = run_diff_workbench_smoke(&mut state, DiffWorkbenchSmokeMode::Annotation);

    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.mode, "annotation");
    assert!(report.diff_checked);
    assert_eq!(report.selected_compare_action_id.as_deref(), Some("ann-l1"));
    assert_eq!(report.reviewers_checked, None);
    assert!(!report.editor_dirty);
    assert_eq!(report.row_count, 4);
    assert!(report.modified_deleted_visible);
    assert!(report.annotation_ghost_visible);
    assert_eq!(
        report.annotation_status.as_deref(),
        Some("subcategory-mismatch")
    );
    assert_eq!(report.event_metric_value.as_deref(), Some("91.0%"));
    assert_eq!(report.label_metric_value.as_deref(), Some("64.0%"));
    assert_eq!(report.intensity_metric_value.as_deref(), Some("37.0%"));
    assert_eq!(
        report.annotation_metric_tooltip_titles,
        vec!["Event Detection", "Label Accuracy", "Intensity Calibration"]
    );
    assert!(report.runtime_trace_line().contains("mode=annotation"));

    let project = state.project().unwrap();
    assert_eq!(
        project
            .metadata
            .get("studio.babel.diff.selectedCompareActionId")
            .map(String::as_str),
        Some("ann-l1")
    );
    assert!(state.editor.as_ref().unwrap().diff_view);
}

#[test]
fn transcript_table_header_can_render_recovered_babel_reviewer_column() {
    let header_source = include_str!("components/transcript_table/header.rs");

    assert!(header_source.contains("\"Reviewer\""));
    assert!(header_source.contains("show_reviewers"));
}

#[test]
fn transcript_linter_cell_matches_recovered_babel_dot_tooltip_column() {
    let mod_source = include_str!("components/transcript_table/mod.rs");
    let header_source = include_str!("components/transcript_table/header.rs");
    let lint_source = include_str!("components/transcript_table/lint.rs");

    assert!(mod_source.contains("const LINTER_COLUMN_WIDTH: f32 = 40.0;"));
    assert!(header_source.contains("super::LINTER_COLUMN_WIDTH"));
    assert!(lint_source.contains("view_lint_dot"));
    assert!(lint_source.contains("tooltip("));
    assert!(lint_source.contains("Message::ToggleWarningAssertion"));
    assert!(lint_source.contains(".style(table_plain_button_style)"));
    assert!(lint_source.contains(".width(Length::Fixed(super::LINTER_COLUMN_WIDTH))"));
    assert!(!lint_source.contains("text(message).size(12).width(Length::Fixed(106.0))"));
    assert!(!lint_source.contains("row!["));
    assert!(!lint_source.contains(".width(Length::Fixed(124.0))"));
}

#[test]
fn transcript_row_renderer_uses_recovered_babel_visible_cells() {
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(row_source.contains("visible_cells.id"));
    assert!(row_source.contains("visible_cells.track"));
    assert!(row_source.contains("visible_cells.start"));
    assert!(row_source.contains("visible_cells.end"));
    assert!(row_source.contains("visible_cells.reviewer"));
    assert!(row_source.contains("visible_cells.actions"));
    assert!(row_source.contains("if row_view.is_read_only"));
    assert!(row_source.contains("view_read_only_time_cell(row_view.start_time.clone())"));
    assert!(row_source.contains("view_read_only_time_cell(row_view.end_time.clone())"));
    assert!(row_source.contains("view_reviewer_cell(row_view.reviewer.clone())"));
    assert!(row_source.contains("row_view.visible_cells.actions && !row_view.is_read_only"));
}

#[test]
fn transcript_table_renders_recovered_babel_empty_state_model() {
    let source = include_str!("components/transcript_table/mod.rs");
    let empty_state_source = include_str!("../transcript_model/empty_state.rs");

    assert!(source.contains("editor.transcript_empty_state(project)"));
    assert!(source.contains("view_empty_state"));
    assert!(source.contains("empty_state.col_span"));
    assert!(source.contains("text(empty_state.title)"));
    assert!(source.contains("text(empty_state.description)"));
    assert!(empty_state_source.contains("No transcriptions yet"));
    assert!(empty_state_source.contains("No annotations yet"));
}

#[test]
fn importing_babel_json_loads_relative_wav_tracks_into_editor_waveforms() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let audio_dir = temp.path().join("audio");
    std::fs::create_dir_all(&audio_dir).expect("audio dir should be created");
    std::fs::write(
        audio_dir.join("speaker_1.wav"),
        pcm16_wav_bytes(4, 1, &[16_384, 16_384, 16_384, 16_384]),
    )
    .expect("wav should be written");
    let import_path = temp.path().join("task.json");
    std::fs::write(
        &import_path,
        serde_json::json!({
            "task_id": "task-with-audio",
            "project_preset": "ru-demo",
            "speakers": [
                { "speaker_key": "speaker-1", "label": "Speaker 1" }
            ],
            "audio_tracks": [
                { "track_id": "track-1", "speaker_key": "speaker-1", "path": "audio/speaker_1.wav" }
            ],
            "annotations": [
                {
                    "annotation_id": "annotation-1",
                    "speaker_key": "speaker-1",
                    "track_id": "track-1",
                    "start_seconds": 0.0,
                    "end_seconds": 1.0,
                    "text": "hello"
                }
            ]
        })
        .to_string(),
    )
    .expect("task should be written");
    let mut state = StudioState {
        import_path: import_path.display().to_string(),
        ..stable_sample_state()
    };

    let _ = update(&mut state, Message::ImportBabelJson);

    let project = state.workflow.as_ref().unwrap().project();
    let lanes = state.editor.as_ref().unwrap().timeline_lanes(project);
    let block = &lanes[0].blocks[0];
    assert_eq!(block.peak_source, WaveformPeakSource::Audio);
    assert!(block.peaks.iter().all(|peak| (*peak - 0.5).abs() < 0.001));
}

#[test]
fn importing_babel_json_uses_local_speaker_wav_when_track_path_is_remote() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    std::fs::write(
        temp.path().join("speaker_1.wav"),
        pcm16_wav_bytes(4, 1, &[8_192, 8_192, 8_192, 8_192]),
    )
    .expect("wav should be written");
    let import_path = temp.path().join("task.json");
    std::fs::write(
        &import_path,
        serde_json::json!({
            "task_id": "task-with-remote-audio",
            "project_preset": "ru-demo",
            "speakers": [
                { "speaker_key": "speaker-1", "label": "Speaker 1" }
            ],
            "audio_tracks": [
                { "track_id": "track-1", "speaker_key": "speaker-1", "path": "s3://bucket/recording.wav" }
            ],
            "annotations": [
                {
                    "annotation_id": "annotation-1",
                    "speaker_key": "speaker-1",
                    "track_id": "track-1",
                    "start_seconds": 0.0,
                    "end_seconds": 1.0,
                    "text": "hello"
                }
            ]
        })
        .to_string(),
    )
    .expect("task should be written");
    let mut state = StudioState {
        import_path: import_path.display().to_string(),
        ..stable_sample_state()
    };

    let _ = update(&mut state, Message::ImportBabelJson);

    let project = state.workflow.as_ref().unwrap().project();
    let lanes = state.editor.as_ref().unwrap().timeline_lanes(project);
    let block = &lanes[0].blocks[0];
    assert_eq!(block.peak_source, WaveformPeakSource::Audio);
    assert!(block.peaks.iter().all(|peak| (*peak - 0.25).abs() < 0.001));
}

#[test]
fn playback_toggle_starts_and_stops_native_audio_backend_for_loaded_waveforms() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let audio_dir = temp.path().join("audio");
    std::fs::create_dir_all(&audio_dir).expect("audio dir should be created");
    std::fs::write(
        audio_dir.join("speaker_1.wav"),
        pcm16_wav_bytes(4, 1, &[16_384, 16_384, 16_384, 16_384]),
    )
    .expect("wav should be written");
    let import_path = temp.path().join("task.json");
    std::fs::write(
        &import_path,
        serde_json::json!({
            "task_id": "task-with-playback",
            "project_preset": "ru-demo",
            "speakers": [
                { "speaker_key": "speaker-1", "label": "Speaker 1" }
            ],
            "audio_tracks": [
                { "track_id": "track-1", "speaker_key": "speaker-1", "path": "audio/speaker_1.wav" }
            ],
            "annotations": [
                {
                    "annotation_id": "annotation-1",
                    "speaker_key": "speaker-1",
                    "track_id": "track-1",
                    "start_seconds": 0.0,
                    "end_seconds": 1.0,
                    "text": "hello"
                }
            ]
        })
        .to_string(),
    )
    .expect("task should be written");
    let mut state = StudioState {
        import_path: import_path.display().to_string(),
        ..stable_sample_state()
    };
    let _ = update(&mut state, Message::ImportBabelJson);

    let _ = update(&mut state, Message::TogglePlayback);

    assert!(state.editor.as_ref().unwrap().is_playing);
    assert!(state.audio_playback.is_active());
    assert_eq!(state.audio_playback.track_count(), 1);
    let audio_info = state
        .runtime_audio_playback_info
        .as_ref()
        .expect("starting decoded WAV playback should record a runtime smoke trace");
    assert_eq!(audio_info.track_count, 1);
    assert_eq!(audio_info.start_seconds, 0.0);
    assert_eq!(audio_info.speed, 1.0);
    assert_eq!(
        audio_info.runtime_log_line(),
        "Transcription Studio audio playback: tracks=1 start=0.000s speed=1.00x"
    );

    let _ = update(&mut state, Message::TogglePlayback);

    assert!(!state.editor.as_ref().unwrap().is_playing);
    assert!(!state.audio_playback.is_active());
}

#[test]
fn native_audio_playback_streams_decoded_audio_without_materializing_full_track() {
    let source = include_str!("audio_backend.rs");
    let state_source = include_str!("state.rs");

    assert!(source.contains("struct DecodedAudioPlaybackSource"));
    assert!(source.contains("DecodedAudioPlaybackSource::new("));
    assert!(source.contains("audio: Arc<DecodedAudio>"));
    assert!(source.contains("player.append(source);"));
    assert!(
        !source
            .contains("let samples = track.interleaved_samples_until(start_seconds, end_seconds);"),
        "playback startup should not allocate and interleave the entire remaining track"
    );
    assert!(
        !source.contains("SamplesBuffer::new(channels, sample_rate, samples)"),
        "rodio should receive a streaming source backed by decoded channels"
    );
    assert!(
        !state_source.contains("PlaybackTrack::new(lane.id.clone(), audio.clone())"),
        "native playback should share decoded WAV data instead of deep-cloning full tracks on play"
    );
}

#[test]
fn native_audio_playback_source_binds_channel_buffers_before_the_sample_hot_loop() {
    let source = include_str!("audio_backend.rs");
    let source_struct = source
        .split("struct DecodedAudioPlaybackSource")
        .nth(1)
        .expect("decoded playback source should exist")
        .split("impl DecodedAudioPlaybackSource")
        .next()
        .expect("source struct should appear before its impl");
    let source_impl = source
        .split("impl DecodedAudioPlaybackSource")
        .nth(1)
        .expect("decoded playback source impl should exist");
    let source_new = source_impl
        .split("fn new(")
        .nth(1)
        .expect("decoded playback source constructor should exist")
        .split("fn is_empty")
        .next()
        .expect("constructor should appear before is_empty");
    let source_next = source
        .split("fn next(&mut self) -> Option<Self::Item>")
        .nth(1)
        .expect("decoded playback iterator should define next")
        .split("fn size_hint")
        .next()
        .expect("next should appear before size_hint");

    assert!(source_struct.contains("channels: Vec<Arc<[f32]>>"));
    assert!(source_new.contains("channel_data_shared"));
    assert!(
        !source_next.contains("channel_data("),
        "audio sample playback should not perform a decoded-audio channel lookup for every emitted sample"
    );
}

#[test]
fn audio_playback_startup_does_not_clone_project_or_editor() {
    let state_source = include_str!("state.rs");
    let start_audio_playback = state_source
        .split("pub(super) fn start_audio_playback")
        .nth(1)
        .expect("state should define audio playback startup")
        .split("fn record_audio_playback_info")
        .next()
        .expect("audio playback startup should appear before playback info recording");
    let play_audio_range = state_source
        .split("pub(super) fn play_audio_range")
        .nth(1)
        .expect("state should define range audition playback")
        .split("pub(super) fn acknowledge_low_confidence_word")
        .next()
        .expect("range audition should appear before low-confidence helpers");

    assert!(!start_audio_playback.contains("self.project().cloned()"));
    assert!(!start_audio_playback.contains("self.editor.clone()"));
    assert!(start_audio_playback.contains("workflow.project()"));
    assert!(start_audio_playback.contains("self.editor.as_ref()"));
    assert!(!play_audio_range.contains("self.project().cloned()"));
    assert!(!play_audio_range.contains("self.editor.clone()"));
    assert!(play_audio_range.contains("workflow.project()"));
    assert!(play_audio_range.contains("self.editor.as_ref()"));
}

#[test]
fn audio_playback_startup_trace_reports_frontend_and_backend_boundaries() {
    let state_source = include_str!("state.rs");
    let backend_source = include_str!("audio_backend.rs");

    assert!(state_source.contains("STUDIO_TRACE_AUDIO_STARTUP"));
    assert!(state_source.contains("audio_playback_startup_trace_enabled()"));
    assert!(state_source.contains("audio_playback_startup_runtime_trace_line("));
    assert!(state_source.contains("track_collect_ms="));
    assert!(state_source.contains("backend_ms="));
    assert!(backend_source.contains("audio_backend_startup_runtime_trace_line("));
    assert!(backend_source.contains("sink_ms="));
    assert!(backend_source.contains("append_ms="));
}

#[test]
fn audio_playback_backend_prewarms_reusable_players_off_the_first_play_hot_path() {
    let backend_source = include_str!("audio_backend.rs");
    let play_tracks_until = backend_source
        .split("pub(super) fn play_tracks_until")
        .nth(1)
        .expect("native backend should define play_tracks_until")
        .split("pub(super) fn stop")
        .next()
        .expect("play_tracks_until should appear before stop");

    assert!(backend_source.contains("idle_players: Vec<rodio::Player>"));
    assert!(backend_source.contains("ensure_player_pool(AUDIO_PREWARMED_PLAYER_POOL_SIZE)"));
    assert!(play_tracks_until.contains(".idle_players"));
    assert!(play_tracks_until.contains(".pop()"));
    assert!(!play_tracks_until.contains("rodio::Player::connect_new"));
}

#[test]
fn audio_playback_backend_mixes_babel_shape_tracks_into_one_rodio_player() {
    let backend_source = include_str!("audio_backend.rs");
    let play_tracks_until = backend_source
        .split("pub(super) fn play_tracks_until")
        .nth(1)
        .expect("native backend should define play_tracks_until")
        .split("pub(super) fn stop")
        .next()
        .expect("play_tracks_until should appear before stop");

    assert!(backend_source.contains("struct MixedDecodedAudioPlaybackSource"));
    assert!(backend_source.contains("fn mixed_source_for_tracks("));
    assert!(play_tracks_until.contains("let sources = playable_sources_for_tracks("));
    assert!(play_tracks_until.contains("if let Some(source) = mixed_source_for_tracks(&sources)"));
    assert!(play_tracks_until.contains("self.ensure_player_pool(1)"));
    assert!(play_tracks_until.contains("player.append(source);"));
    assert!(play_tracks_until.contains("append_individual_sources"));
}

#[test]
fn audio_playback_startup_trace_line_formats_runtime_timings() {
    assert_eq!(
        super::state::audio_playback_startup_runtime_trace_line(
            2,
            Duration::from_millis(3),
            Duration::from_millis(5),
            Duration::from_millis(8),
            "started",
        ),
        "Transcription Studio audio startup: tracks=2 outcome=started track_collect_ms=3.000 backend_ms=5.000 total_ms=8.000"
    );
}

#[test]
fn waveform_shift_click_auditions_full_babel_region_without_starting_drag() {
    let mut state = imported_audio_state_for_audition(1.0, 3.0);
    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::SHIFT),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: 1.5,
            },
        ),
    );

    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );

    assert!(state.audio_playback.is_active());
    assert_eq!(state.audio_playback.last_start_seconds, 1.0);
    assert_eq!(state.audio_playback.last_end_seconds, Some(3.0));
    assert!(state.active_waveform_drag.is_none());
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-1")
    );
}

#[test]
fn waveform_held_audition_keys_play_babel_500ms_boundary_windows() {
    let cases = [
        (AuditionKey::BeforeStart, 0.5, 1.0),
        (AuditionKey::AfterStart, 1.0, 1.5),
        (AuditionKey::BeforeEnd, 2.5, 3.0),
        (AuditionKey::AfterEnd, 3.0, 3.5),
    ];

    for (audition_key, expected_start, expected_end) in cases {
        let mut state = imported_audio_state_for_audition(1.0, 3.0);
        let _ = update(&mut state, Message::AuditionKeyChanged(audition_key, true));
        let _ = update(
            &mut state,
            Message::WaveformHitHovered(
                "track-1".to_string(),
                WaveformHit {
                    target: WaveformHitTarget::RegionBody,
                    region_id: Some("annotation-1".to_string()),
                    time_seconds: 2.0,
                },
            ),
        );

        let _ = update(
            &mut state,
            Message::WaveformHitPressed("track-1".to_string()),
        );

        assert_eq!(state.audio_playback.last_start_seconds, expected_start);
        assert_eq!(state.audio_playback.last_end_seconds, Some(expected_end));
        assert!(state.active_waveform_drag.is_none());
    }
}

#[test]
fn segment_drafts_use_babel_timestamp_format() {
    let state = stable_sample_state();

    let first = state.segment_drafts.get("annotation-1").unwrap();
    let long = state.segment_drafts.get("annotation-5").unwrap();

    assert_eq!(first.start_seconds, "00:01.20");
    assert_eq!(first.end_seconds, "00:03.40");
    assert_eq!(long.start_seconds, "01:05.08");
}

#[test]
fn segment_drafts_initialize_multiline_editor_content_from_babel_text() {
    let state = stable_sample_state();
    let draft = state.segment_drafts.get("annotation-4").unwrap();

    assert_eq!(draft.text, draft.editor_text());
    assert!(draft.text_editor_content_height() > 40.0);
}

#[test]
fn segment_drafts_use_recovered_babel_single_row_text_editor_density() {
    let state = stable_sample_state();
    let short = state.segment_drafts.get("annotation-1").unwrap();
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(short.text_editor_content_height() <= 30.0);
    assert!(row_source.contains(".padding([2, 4])"));
    assert!(row_source.contains(".size(12)"));
    assert!(row_source.contains(".style(table_plain_text_editor_style)"));
    assert!(!row_source.contains(".padding(5)"));
}

#[test]
fn babel_day_to_day_mismatch_transcript_text_cells_expand_instead_of_internal_scroll() {
    let draft_source = include_str!("drafts.rs");
    let row_source = include_str!("components/transcript_table/row.rs");
    let table_source = include_str!("components/transcript_table/mod.rs");

    assert!(table_source.contains("scrollable(rows)"));
    assert!(row_source.contains(".height(Length::Shrink)"));
    assert!(!row_source.contains(".height(Length::Fixed(draft.text_editor_content_height()))"));
    assert!(draft_source.contains("pub(super) fn text_editor_content_height"));
    assert!(!draft_source.contains(".clamp(1, 7)"));
}

#[test]
fn babel_day_to_day_mismatch_text_editor_wheel_routes_to_transcript_scroll_container() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let message_source = include_str!("message.rs");
    let actions_source = include_str!("actions.rs");

    assert!(row_source.contains("transcript_text_wheel_overlay()"));
    assert!(row_source.contains("Message::TranscriptTextWheelScrolled"));
    assert!(row_source.contains(".height(Length::Shrink)"));
    assert!(!row_source.contains(".height(Length::Fixed(draft.text_editor_content_height()))"));
    assert!(message_source.contains("TranscriptTextWheelScrolled {"));
    assert!(actions_source.contains("Message::TranscriptTextWheelScrolled"));
    assert!(actions_source.contains("scrollable::scroll_by("));
    assert!(actions_source.contains("find::transcript_scroll_id()"));
}

#[test]
fn transcript_text_editor_actions_update_draft_text_and_dirty_state() {
    let mut state = stable_sample_state();
    let _ = update(
        &mut state,
        Message::SegmentTextEdited(
            "annotation-1".to_string(),
            TextEditAction::Edit(TextEdit::Paste(" edited".to_string().into())),
        ),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(draft.text.contains("edited"));
    assert_eq!(draft.text, draft.editor_text());
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn transcript_text_editor_undo_redo_stays_in_draft_like_browser_input() {
    let mut state = stable_sample_state();
    let original_text = state
        .segment_drafts
        .get("annotation-1")
        .unwrap()
        .text
        .clone();
    assert!(!state.can_undo());

    let _ = update(
        &mut state,
        Message::SegmentTextEdited(
            "annotation-1".to_string(),
            TextEditAction::Edit(TextEdit::Paste(" edited".to_string().into())),
        ),
    );
    let edited_text = state
        .segment_drafts
        .get("annotation-1")
        .unwrap()
        .text
        .clone();
    assert_ne!(edited_text, original_text);
    assert!(!state.can_undo());

    let _ = update(
        &mut state,
        Message::SegmentTextUndo("annotation-1".to_string()),
    );
    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        original_text
    );
    assert!(!state.can_undo());

    let _ = update(
        &mut state,
        Message::SegmentTextRedo("annotation-1".to_string()),
    );
    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        edited_text
    );
    assert!(!state.can_undo());
}

#[test]
fn read_only_project_blocks_transcript_text_and_structure_edits() {
    let mut state = stable_sample_state();
    let mut project = state.workflow.as_ref().unwrap().project().clone();
    project
        .metadata
        .insert("studio.babel.readOnly".to_string(), "true".to_string());
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded read-only fixture".to_string(),
    );

    let original_text = state
        .segment_drafts
        .get("annotation-1")
        .unwrap()
        .text
        .clone();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();

    let _ = update(
        &mut state,
        Message::SegmentTextEdited(
            "annotation-1".to_string(),
            TextEditAction::Edit(TextEdit::Paste(" blocked".to_string().into())),
        ),
    );

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        original_text
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert_eq!(state.status, "Text edit unavailable: project is read-only.");

    let _ = update(
        &mut state,
        Message::DeleteSegment("annotation-1".to_string()),
    );

    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count
    );
    assert!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .is_some()
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert_eq!(state.status, "Delete unavailable: project is read-only.");
}

#[test]
fn draft_sync_preserves_existing_text_editor_content_for_unapplied_edits() {
    let mut state = stable_sample_state();
    let _ = update(
        &mut state,
        Message::SegmentTextEdited(
            "annotation-1".to_string(),
            TextEditAction::Edit(TextEdit::Paste(" pending".to_string().into())),
        ),
    );

    state.sync_segment_drafts_preserving_existing();

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(draft.text.contains("pending"));
    assert_eq!(draft.text, draft.editor_text());
}

#[test]
fn tag_suggestions_match_babel_square_bracket_trigger() {
    let text = "это [не";
    let suggestions = tag_suggestions_for_text(text, text.len());
    let suggestion = suggestions
        .iter()
        .find(|suggestion| suggestion.label == "неразборчиво")
        .expect("Russian Babel bracket tag should be suggested");

    assert_eq!(suggestion.kind, TagSuggestionKind::Bracket);
    assert_eq!(suggestion.insert_text, "[неразборчиво]");
    assert_eq!(suggestion.replacement_start, "это ".len());
    assert_eq!(suggestion.replacement_end, text.len());
    assert_eq!(suggestion.backspace_count, "[не".chars().count());
}

#[test]
fn tag_suggestions_match_babel_style_open_and_close_triggers() {
    let open_text = "говорит <нара";
    let open_suggestions = tag_suggestions_for_text(open_text, open_text.len());
    let open = open_suggestions
        .iter()
        .find(|suggestion| suggestion.label == "<речь-нараспев>")
        .expect("Russian Babel style tag should be suggested");

    assert_eq!(open.kind, TagSuggestionKind::StyleOpen);
    assert_eq!(open.insert_text, "<речь-нараспев></речь-нараспев>");
    assert_eq!(open.cursor_back_count, "речь-нараспев>".chars().count() + 2);

    let close_text = "говорит </смех";
    let close_suggestions = tag_suggestions_for_text(close_text, close_text.len());
    let close = close_suggestions
        .iter()
        .find(|suggestion| suggestion.label == "</смех-в-речи>")
        .expect("Russian Babel style close tag should be suggested");

    assert_eq!(close.kind, TagSuggestionKind::StyleClose);
    assert_eq!(close.insert_text, "</смех-в-речи>");
    assert_eq!(close.cursor_back_count, 0);
}

#[test]
fn tag_suggestions_match_babel_curly_mis_and_pro_triggers() {
    let mis_text = "слово {M";
    let mis_suggestions = tag_suggestions_for_text(mis_text, mis_text.len());
    let mis = mis_suggestions
        .iter()
        .find(|suggestion| suggestion.label == "MIS:")
        .expect("Babel MIS curly tag should be suggested");

    assert_eq!(mis.kind, TagSuggestionKind::Curly);
    assert_eq!(mis.insert_text, "{MIS: }");
    assert_eq!(mis.replacement_start, "слово ".len());
    assert_eq!(mis.replacement_end, mis_text.len());
    assert_eq!(mis.cursor_back_count, 1);

    let pro_text = "слово {pro";
    let pro_suggestions = tag_suggestions_for_text(pro_text, pro_text.len());
    let pro = pro_suggestions
        .iter()
        .find(|suggestion| suggestion.label == "PRO:")
        .expect("Babel PRO curly tag should be suggested case-insensitively");

    assert_eq!(pro.kind, TagSuggestionKind::Curly);
    assert_eq!(pro.insert_text, "{PRO: }");
}

#[test]
fn applying_tag_suggestion_replaces_partial_trigger_and_marks_dirty() {
    let mut state = stable_sample_state();
    let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
    draft.replace_text("слово [не".to_string());
    let suggestion = tag_suggestions_for_text(&draft.text, draft.text.len())
        .into_iter()
        .find(|suggestion| suggestion.label == "неразборчиво")
        .unwrap();

    let _ = update(
        &mut state,
        Message::ApplyTagSuggestion("annotation-1".to_string(), suggestion),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "слово [неразборчиво]");
    assert_eq!(draft.text, draft.editor_text());
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn applying_curly_tag_suggestion_places_cursor_before_closing_brace() {
    let mut state = stable_sample_state();
    let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
    draft.replace_text("слово {M".to_string());
    let suggestion = tag_suggestions_for_text(&draft.text, draft.text.len())
        .into_iter()
        .find(|suggestion| suggestion.label == "MIS:")
        .unwrap();

    let _ = update(
        &mut state,
        Message::ApplyTagSuggestion("annotation-1".to_string(), suggestion),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "слово {MIS: }");
    assert_eq!(
        draft.text_content.cursor_position(),
        (0, "слово {MIS: ".len())
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn applying_tag_suggestion_stays_in_browser_like_text_undo_history() {
    let mut state = stable_sample_state();
    let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
    draft.replace_text("слово [не".to_string());
    let original_text = draft.text.clone();
    let suggestion = tag_suggestions_for_text(&draft.text, draft.text.len())
        .into_iter()
        .find(|suggestion| suggestion.label == "неразборчиво")
        .unwrap();

    let _ = update(
        &mut state,
        Message::ApplyTagSuggestion("annotation-1".to_string(), suggestion),
    );
    let suggested_text = state
        .segment_drafts
        .get("annotation-1")
        .unwrap()
        .text
        .clone();
    assert_eq!(suggested_text, "слово [неразборчиво]");

    let _ = update(
        &mut state,
        Message::SegmentTextUndo("annotation-1".to_string()),
    );
    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        original_text
    );

    let _ = update(
        &mut state,
        Message::SegmentTextRedo("annotation-1".to_string()),
    );
    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        suggested_text
    );
}

#[test]
fn applying_style_open_suggestion_places_cursor_inside_babel_style_pair() {
    let mut state = stable_sample_state();
    let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
    draft.replace_text("слово <нара".to_string());
    let suggestion = tag_suggestions_for_text(&draft.text, draft.text.len())
        .into_iter()
        .find(|suggestion| suggestion.label == "<речь-нараспев>")
        .unwrap();

    let _ = update(
        &mut state,
        Message::ApplyTagSuggestion("annotation-1".to_string(), suggestion),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "слово <речь-нараспев></речь-нараспев>");
    assert_eq!(
        draft.text_content.cursor_position(),
        (0, "слово <речь-нараспев>".len())
    );
}

#[test]
fn applying_style_tag_suggestion_wraps_selected_text_and_marks_dirty() {
    let mut state = stable_sample_state();
    let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
    draft.replace_text("тихий голос".to_string());
    draft.text_content.perform(TextEditAction::SelectAll);
    let suggestion = tag_suggestions_for_draft(draft)
        .into_iter()
        .find(|suggestion| suggestion.label == "<речь-нараспев>")
        .expect("style tag suggestions should be available for selected text");

    let _ = update(
        &mut state,
        Message::ApplyTagSuggestion("annotation-1".to_string(), suggestion),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "<речь-нараспев>тихий голос</речь-нараспев>");
    assert_eq!(draft.text, draft.editor_text());
    assert!(state.editor.as_ref().unwrap().is_dirty);

    let _ = update(
        &mut state,
        Message::SegmentTextUndo("annotation-1".to_string()),
    );

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "тихий голос"
    );
}

fn set_draft_text_with_cursor_at_end(state: &mut StudioState, segment_id: &str, text: &str) {
    let draft = state.segment_drafts.get_mut(segment_id).unwrap();
    draft.replace_text(String::new());
    draft.perform_text_edit(TextEditAction::Edit(TextEdit::Paste(
        text.to_string().into(),
    )));
}

fn set_draft_text_with_cursor_after_chars(
    state: &mut StudioState,
    segment_id: &str,
    text: &str,
    cursor_chars: usize,
) {
    let draft = state.segment_drafts.get_mut(segment_id).unwrap();
    draft.replace_text(text.to_string());
    for _ in 0..cursor_chars {
        draft
            .text_content
            .perform(TextEditAction::Move(TextEditMotion::Right));
    }
}

fn set_draft_text_selected_all(state: &mut StudioState, segment_id: &str, text: &str) {
    let draft = state.segment_drafts.get_mut(segment_id).unwrap();
    draft.replace_text(text.to_string());
    draft.text_content.perform(TextEditAction::SelectAll);
}

#[test]
fn helper_text_move_before_caret_targets_previous_same_lane_segment() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-3");
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("previous".to_string());
    set_draft_text_with_cursor_after_chars(&mut state, "annotation-3", "move me stay", 7);

    let _ = update(&mut state, Message::MoveTextBeforeCaretToPreviousSegment);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "previous move me"
    );
    let current = state.segment_drafts.get("annotation-3").unwrap();
    assert_eq!(current.text, "stay");
    assert_eq!(current.text_content.cursor_position(), (0, 0));
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_text_move_after_caret_targets_next_same_lane_segment() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    state
        .segment_drafts
        .get_mut("annotation-3")
        .unwrap()
        .replace_text("next".to_string());
    set_draft_text_with_cursor_after_chars(&mut state, "annotation-1", "keep move me", 4);

    let _ = update(&mut state, Message::MoveTextAfterCaretToNextSegment);

    let current = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(current.text, "keep");
    assert_eq!(current.text_content.cursor_position(), (0, "keep".len()));
    assert_eq!(
        state.segment_drafts.get("annotation-3").unwrap().text,
        "move me next"
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_selected_number_to_skaz_converts_selected_digits_like_extension() {
    let mut state = stable_sample_state();
    set_draft_text_selected_all(&mut state, "annotation-1", "1234");

    let _ = update(&mut state, Message::ConvertSelectedNumberToSkaz);

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(
        draft.text,
        "1 234 {СКАЗ: одна тысяча двести тридцать четыре}"
    );
    assert_eq!(draft.text, draft.editor_text());
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_digit_over_selection_preserves_babel_skaz_cursor_offset() {
    let mut state = stable_sample_state();
    set_draft_text_selected_all(&mut state, "annotation-1", "семь восемь");

    let _ = update(
        &mut state,
        Message::ConvertSelectionToSkazWithDigit("annotation-1".to_string(), '5'),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "5 {СКАЗ: семь восемь}");
    assert_eq!(draft.text_content.cursor_position(), (0, 1));
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_hotkeys_route_inside_captured_transcript_inputs() {
    assert_eq!(
        hotkey_message_for_status(Key::Character("[".into()), Modifiers::ALT, Status::Captured,),
        Some(Message::MoveTextBeforeCaretToPreviousSegment)
    );
    assert_eq!(
        hotkey_message_for_status(Key::Character("]".into()), Modifiers::ALT, Status::Captured,),
        Some(Message::MoveTextAfterCaretToNextSegment)
    );
    assert_eq!(
        hotkey_message_for_status(Key::Character("a".into()), Modifiers::ALT, Status::Captured,),
        Some(Message::ConvertSelectedNumberToSkaz)
    );
    assert_eq!(
        hotkey_message_for_status(Key::Character("f".into()), Modifiers::ALT, Status::Captured,),
        Some(Message::AutofixCurrentLintRow)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("f".into()),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Captured,
        ),
        Some(Message::AutofixAllLintRows)
    );
}

#[test]
fn helper_row_action_hotkeys_route_only_when_not_typing() {
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("d".into()),
            Modifiers::empty(),
            Status::Ignored,
        ),
        Some(Message::DeleteSelectedSegment)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("d".into()),
            Modifiers::empty(),
            Status::Captured,
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::ArrowUp),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::MergeSelectedSegmentAbove)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::ArrowDown),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::MergeSelectedSegmentBelow)
    );
}

#[test]
fn helper_right_shift_segment_navigation_uses_physical_shift_state() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Named(Named::ArrowRight),
            Physical::Code(Code::ArrowRight),
            Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::RightShiftSegmentNavigation(1))
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Named(Named::ArrowLeft),
            Physical::Code(Code::ArrowLeft),
            Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::RightShiftSegmentNavigation(-1))
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Named(Named::ArrowRight),
            Physical::Code(Code::ArrowRight),
            Modifiers::SHIFT,
            Status::Captured,
        ),
        None,
        "Right Shift navigation must not steal text selection inside inputs"
    );
}

#[test]
fn helper_right_shift_navigation_focuses_adjacent_row_from_start_without_seeking() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    state.editor.as_mut().unwrap().playhead_seconds = 42.0;

    let _ = update(&mut state, Message::RightShiftSegmentNavigation(1));
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-1"),
        "generic Shift+Arrow is ignored until the physical right Shift key is held"
    );
    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 42.0);

    let _ = update(&mut state, Message::RightShiftChanged(true));
    let _ = update(&mut state, Message::RightShiftSegmentNavigation(1));

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-2"));
    assert_eq!(
        editor.playhead_seconds, 42.0,
        "Helper focus navigation should not perform the bare-arrow seek"
    );
    assert_eq!(
        state
            .helper_last_focus_restore
            .as_ref()
            .map(|restore| (restore.segment_id.as_str(), restore.restore_offset)),
        Some(("annotation-2", 0))
    );

    let _ = update(&mut state, Message::RightShiftChanged(false));
    let _ = update(&mut state, Message::RightShiftSegmentNavigation(-1));
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-2"),
        "right-shift release disables Helper focus navigation"
    );
}

#[test]
fn helper_timeline_trim_hotkeys_match_recovered_helper_routes() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("r".into()),
            Physical::Code(Code::KeyR),
            Modifiers::ALT,
            Status::Ignored,
        ),
        Some(Message::TrimCurrentSegmentToAudio)
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("R".into()),
            Physical::Code(Code::KeyR),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::TrimAllVisibleSegmentsToAudio)
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("r".into()),
            Physical::Code(Code::KeyR),
            Modifiers::ALT,
            Status::Captured,
        ),
        None,
        "trim hotkeys must not steal Alt+R inside captured text inputs"
    );
}

#[test]
fn helper_timeline_transcription_hotkey_routes_to_native_drafting_segment_transcription() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("G".into()),
            Physical::Code(Code::KeyG),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::DraftTranscribeSelectedSegment)
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("G".into()),
            Physical::Code(Code::KeyG),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Captured,
        ),
        None,
        "segment transcription hotkey must not steal Alt+Shift+G inside captured text inputs"
    );
}

#[test]
fn helper_timeline_auto_insert_hotkey_routes_to_native_decoded_audio_insert() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("c".into()),
            Physical::Code(Code::KeyC),
            Modifiers::ALT,
            Status::Ignored,
        ),
        Some(Message::AutoInsertSegmentAtCaret)
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("c".into()),
            Physical::Code(Code::KeyC),
            Modifiers::ALT,
            Status::Captured,
        ),
        None,
        "auto-insert hotkey must not steal Alt+C inside captured text inputs"
    );
}

#[test]
fn helper_timeline_auto_segmentation_hotkey_routes_to_native_silence_split() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("S".into()),
            Physical::Code(Code::KeyS),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::AutoSegmentVisibleSilences)
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("S".into()),
            Physical::Code(Code::KeyS),
            Modifiers::ALT | Modifiers::SHIFT,
            Status::Captured,
        ),
        None,
        "auto-segmentation hotkey must not steal Alt+Shift+S inside captured text inputs"
    );
}

#[test]
fn helper_cut_preview_overlay_hotkeys_match_recovered_commit_and_loop_routes() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("s".into()),
            Physical::Code(Code::KeyS),
            Modifiers::empty(),
            Status::Ignored,
        ),
        None,
        "plain S must not become global unless a cut preview overlay is active"
    );
    assert_eq!(
        hotkey_message_for_cut_preview_overlay(
            Key::Character("s".into()),
            Physical::Code(Code::KeyS),
            Modifiers::empty(),
            Status::Ignored,
        ),
        Some(Message::CommitCutPreviewOverlay { smart_split: true })
    );
    assert_eq!(
        hotkey_message_for_cut_preview_overlay(
            Key::Character("S".into()),
            Physical::Code(Code::KeyS),
            Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::CommitCutPreviewOverlay { smart_split: false })
    );
    assert_eq!(
        hotkey_message_for_cut_preview_overlay(
            Key::Character("l".into()),
            Physical::Code(Code::KeyL),
            Modifiers::empty(),
            Status::Ignored,
        ),
        Some(Message::LoopCutPreviewOverlay)
    );
    assert_eq!(
        hotkey_message_for_cut_preview_overlay(
            Key::Character("s".into()),
            Physical::Code(Code::KeyS),
            Modifiers::empty(),
            Status::Captured,
        ),
        None,
        "cut preview commit must not steal S inside captured text inputs"
    );
}

#[test]
fn helper_cut_preview_overlay_commit_smart_splits_middle_segment_and_clears_overlay() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 1.0;
    segment.end_seconds = 6.0;
    segment.text = "one two three four five six".to_string();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded cut preview fixture".to_string(),
    );
    state.select_segment("annotation-1");
    state.with_editor_mut(|editor| {
        editor.overlay_range = Some(TimeRange {
            start_seconds: 2.5,
            end_seconds: 4.0,
        });
    });

    let _ = update(
        &mut state,
        Message::CommitCutPreviewOverlay { smart_split: true },
    );

    let project = state.project().unwrap();
    let left = project.segment("annotation-1").unwrap();
    let right = project.segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.start_seconds, 1.0);
    assert_eq!(left.end_seconds, 2.5);
    assert_eq!(left.text, "one two three");
    assert_eq!(right.start_seconds, 4.0);
    assert_eq!(right.end_seconds, 6.0);
    assert_eq!(right.text, "four five six");
    assert!(state.editor.as_ref().unwrap().overlay_range.is_none());
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-1__studio_1")
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_cut_preview_overlay_loop_sets_native_loop_range_without_editing_project() {
    let mut state = stable_sample_state();
    let before_segments = state.project().unwrap().segments.clone();
    state.with_editor_mut(|editor| {
        editor.overlay_range = Some(TimeRange {
            start_seconds: 2.0,
            end_seconds: 5.0,
        });
    });

    let _ = update(&mut state, Message::LoopCutPreviewOverlay);

    let editor = state.editor.as_ref().unwrap();
    assert!(editor.loop_enabled);
    assert_eq!(
        editor.stored_loop_range,
        Some(TimeRange {
            start_seconds: 2.0,
            end_seconds: 5.0,
        })
    );
    assert_eq!(state.project().unwrap().segments, before_segments);
}

#[test]
fn helper_auto_segment_visible_silences_splits_long_decoded_silence_then_trims_segments() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 1.0;
    segment.end_seconds = 4.0;
    segment.text = "first island second island".to_string();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded auto-segmentation fixture".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(
                6.0,
                100,
                &[(1.20, 1.60), (3.20, 3.60)],
            )),
        );
    });

    let _ = update(&mut state, Message::AutoSegmentVisibleSilences);

    let project = state.project().unwrap();
    let mut lane_segments = project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == "track-speaker-1")
        .collect::<Vec<_>>();
    lane_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
    assert_eq!(lane_segments.len(), 2);
    assert_eq!(lane_segments[0].id, "annotation-1");
    assert_eq!(lane_segments[1].id, "annotation-1__studio_1");
    assert!((lane_segments[0].start_seconds - 1.195).abs() < 0.0001);
    assert!((lane_segments[0].end_seconds - 1.595).abs() < 0.0001);
    assert!((lane_segments[1].start_seconds - 3.195).abs() < 0.0001);
    assert!((lane_segments[1].end_seconds - 3.595).abs() < 0.0001);
    assert_eq!(lane_segments[0].text, "first island");
    assert_eq!(lane_segments[1].text, "second island");
    assert!(state.segment_drafts.contains_key("annotation-1__studio_1"));
    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "first island"
    );
    assert_eq!(
        state
            .segment_drafts
            .get("annotation-1__studio_1")
            .unwrap()
            .text,
        "second island"
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
    assert!(state.status.contains("Auto-segmented 1 silence run"));
}

#[test]
fn helper_auto_insert_segment_at_caret_creates_padded_empty_segment_from_nearest_uncovered_speech()
{
    let mut state = stable_sample_state();
    let original_count = state.project().unwrap().segments.len();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 8.5;
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(12.0, 100, &[(8.4, 8.8)])),
        );
    });

    let _ = update(&mut state, Message::AutoInsertSegmentAtCaret);

    let project = state.project().unwrap();
    assert_eq!(project.segments.len(), original_count + 1);
    let created = project
        .segments
        .iter()
        .find(|segment| segment.id.starts_with("track-speaker-1__studio_"))
        .expect("auto-insert should create a derived segment in the decoded lane");
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.speaker_id, "speaker-1");
    assert_eq!(created.text, "");
    assert!((created.start_seconds - 8.35).abs() < 0.0001);
    assert!((created.end_seconds - 8.85).abs() < 0.0001);
    let editor = state.editor.as_ref().unwrap();
    assert_eq!(
        editor.selected_segment_id.as_deref(),
        Some(created.id.as_str())
    );
    assert_eq!(editor.playhead_seconds, 8.5);
    assert!(editor.is_dirty);
    assert!(state.status.contains("Auto-inserted"));
}

#[test]
fn helper_auto_insert_segment_at_caret_skips_speech_already_covered_by_visible_row() {
    let mut state = stable_sample_state();
    let original_count = state.project().unwrap().segments.len();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 2.0;
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(8.0, 100, &[(1.8, 2.2)])),
        );
    });

    let _ = update(&mut state, Message::AutoInsertSegmentAtCaret);

    assert_eq!(state.project().unwrap().segments.len(), original_count);
    assert!(state.status.contains("no uncovered speech"));
}

#[test]
fn helper_auto_insert_segment_at_caret_merges_adjacent_same_lane_rows_and_focuses_result() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let first = project.segment_mut("annotation-1").unwrap();
    first.start_seconds = 1.0;
    first.end_seconds = 1.5;
    first.text = "before".to_string();
    let mut next = first.clone();
    next.id = "annotation-next".to_string();
    next.source_annotation_id = Some("annotation-next".to_string());
    next.start_seconds = 2.5;
    next.end_seconds = 3.0;
    next.text = "after".to_string();
    project.segments.push(next);
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded auto-insert merge fixture".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 2.0;
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(4.0, 100, &[(1.55, 2.45)])),
        );
    });

    let _ = update(&mut state, Message::AutoInsertSegmentAtCaret);

    let project = state.project().unwrap();
    let lane_segments = project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == "track-speaker-1")
        .collect::<Vec<_>>();
    assert_eq!(lane_segments.len(), 1);
    let merged = lane_segments[0];
    assert_eq!(merged.id, "track-speaker-1__studio_1");
    assert_eq!(merged.start_seconds, 1.0);
    assert_eq!(merged.end_seconds, 3.0);
    assert_eq!(merged.text, "before after");
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("track-speaker-1__studio_1")
    );
    assert_eq!(
        state
            .segment_drafts
            .get("track-speaker-1__studio_1")
            .unwrap()
            .text,
        "before after"
    );
    assert!(state.status.contains("merged 2 adjacent row"));
}

#[test]
fn helper_trim_current_segment_to_decoded_audio_moves_boundaries_with_padding() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(
                10.0,
                100,
                &[(1.50, 2.90)],
            )),
        );
    });

    let _ = update(&mut state, Message::TrimCurrentSegmentToAudio);

    let segment = state.project().unwrap().segment("annotation-1").unwrap();
    assert!((segment.start_seconds - 1.495).abs() < 0.0001);
    assert!((segment.end_seconds - 2.895).abs() < 0.0001);
    let draft = state.segment_drafts.get("annotation-1").unwrap();
    let (draft_start_seconds, draft_end_seconds) = parse_draft_times(segment, draft).unwrap();
    assert!((draft_start_seconds - segment.start_seconds).abs() < 0.0001);
    assert!((draft_end_seconds - segment.end_seconds).abs() < 0.0001);
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_trim_current_segment_extends_outward_then_final_trims_coarse_silence_probe() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 2.0;
    segment.end_seconds = 3.0;
    segment.text = "continuous speech".to_string();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded outward trim fixture".to_string(),
    );
    state.select_segment("annotation-1");
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(
                5.0,
                1000,
                &[(1.397, 3.603)],
            )),
        );
    });

    let _ = update(&mut state, Message::TrimCurrentSegmentToAudio);

    let segment = state.project().unwrap().segment("annotation-1").unwrap();
    assert!((segment.start_seconds - 1.392).abs() < 0.0001);
    assert!((segment.end_seconds - 3.607).abs() < 0.0001);
    assert_eq!(segment.text, "continuous speech");
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_trim_current_segment_outward_pass_respects_same_lane_neighbor_guards() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let template = {
        let segment = project.segment_mut("annotation-1").unwrap();
        segment.start_seconds = 2.0;
        segment.end_seconds = 3.0;
        segment.text = "middle".to_string();
        segment.clone()
    };
    let mut previous = template.clone();
    previous.id = "annotation-prev".to_string();
    previous.source_annotation_id = Some("annotation-prev".to_string());
    previous.start_seconds = 1.0;
    previous.end_seconds = 1.35;
    previous.text = "previous".to_string();
    let mut next = template.clone();
    next.id = "annotation-next".to_string();
    next.source_annotation_id = Some("annotation-next".to_string());
    next.start_seconds = 3.70;
    next.end_seconds = 4.0;
    next.text = "next".to_string();
    project.segments.push(previous);
    project.segments.push(next);
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded outward trim guard fixture".to_string(),
    );
    state.select_segment("annotation-1");
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(5.0, 1000, &[(1.0, 4.2)])),
        );
    });

    let _ = update(&mut state, Message::TrimCurrentSegmentToAudio);

    let segment = state.project().unwrap().segment("annotation-1").unwrap();
    assert!((segment.start_seconds - 1.36).abs() < 0.0001);
    assert!((segment.end_seconds - 3.69).abs() < 0.0001);
}

#[test]
fn helper_trim_all_visible_segments_uses_decoded_audio_and_same_lane_guards() {
    let mut state = stable_sample_state();
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-speaker-1",
            LaneWaveform::from_decoded(decoded_audio_with_speech_ranges(
                10.0,
                100,
                &[(1.50, 2.90), (5.70, 7.05)],
            )),
        );
    });

    let _ = update(&mut state, Message::TrimAllVisibleSegmentsToAudio);

    let project = state.project().unwrap();
    let first = project.segment("annotation-1").unwrap();
    let third = project.segment("annotation-3").unwrap();
    assert!((first.start_seconds - 1.495).abs() < 0.0001);
    assert!((first.end_seconds - 2.895).abs() < 0.0001);
    assert!((third.start_seconds - 5.695).abs() < 0.0001);
    assert!((third.end_seconds - 7.045).abs() < 0.0001);
    assert_eq!(
        project.segment("annotation-2").unwrap().start_seconds,
        3.5,
        "lanes without decoded audio are skipped"
    );
    assert!(state.status.contains("Trimmed 2 visible segments"));
}

#[test]
fn helper_lint_autofix_current_row_applies_common_babel_text_hygiene() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    set_draft_text_with_cursor_at_end(&mut state, "annotation-1", "  привет,мир  ");

    let _ = update(&mut state, Message::AutofixCurrentLintRow);

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "Привет, мир.");
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_lint_autofix_all_rows_uses_same_lane_previous_context() {
    let mut state = stable_sample_state();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("первая  строка".to_string());
    state
        .segment_drafts
        .get_mut("annotation-3")
        .unwrap()
        .replace_text("следующая,строка".to_string());

    let _ = update(&mut state, Message::AutofixAllLintRows);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "Первая строка."
    );
    assert_eq!(
        state.segment_drafts.get("annotation-3").unwrap().text,
        "Следующая, строка."
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn helper_lint_autofix_preserves_lowercase_after_same_lane_continuation_marker() {
    let mut state = stable_sample_state();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("это --".to_string());
    state
        .segment_drafts
        .get_mut("annotation-3")
        .unwrap()
        .replace_text("продолжение".to_string());

    let _ = update(&mut state, Message::AutofixAllLintRows);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "Это --"
    );
    assert_eq!(
        state.segment_drafts.get("annotation-3").unwrap().text,
        "продолжение."
    );
}

#[test]
fn helper_lint_autofix_moves_tag_trailing_punctuation_like_babel_linter() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    set_draft_text_with_cursor_at_end(&mut state, "annotation-1", "привет {MIS: шум}?");

    let _ = update(&mut state, Message::AutofixCurrentLintRow);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "Привет? {MIS: шум}."
    );
}

#[test]
fn helper_lint_autofix_moves_square_tag_dash_and_normalizes_free_double_dash() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    set_draft_text_with_cursor_at_end(
        &mut state,
        "annotation-1",
        "ну, - вот [смех]- дальше -- тут",
    );

    let _ = update(&mut state, Message::AutofixCurrentLintRow);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "Ну - вот- [смех] дальше - тут."
    );
}

#[test]
fn helper_lint_autofix_normalizes_recovered_interjection_variants() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    set_draft_text_with_cursor_at_end(&mut state, "annotation-1", "эээ уау чорт");

    let _ = update(&mut state, Message::AutofixCurrentLintRow);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "Э вау чёрт."
    );
}

#[test]
fn helper_lint_autofix_lowercases_polite_pronouns_only_inside_sentence() {
    let mut state = stable_sample_state();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("спасибо Вам за Ваш ответ".to_string());
    state
        .segment_drafts
        .get_mut("annotation-2")
        .unwrap()
        .replace_text("Вам спасибо".to_string());
    state
        .segment_drafts
        .get_mut("annotation-3")
        .unwrap()
        .replace_text("вопрос: Вам удобно".to_string());

    let _ = update(&mut state, Message::AutofixAllLintRows);

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "Спасибо вам за ваш ответ."
    );
    assert_eq!(
        state.segment_drafts.get("annotation-2").unwrap().text,
        "Вам спасибо."
    );
    assert_eq!(
        state.segment_drafts.get("annotation-3").unwrap().text,
        "Вопрос: Вам удобно."
    );
}

#[test]
fn tag_suggestion_keyboard_accepts_highlighted_recovered_babel_suggestion() {
    let mut state = stable_sample_state();
    set_draft_text_with_cursor_at_end(&mut state, "annotation-1", "[сме");

    let _ = update(
        &mut state,
        Message::TranscriptTagSuggestionKeyPressed(
            "annotation-1".to_string(),
            TagSuggestionKeyboardAction::Next,
        ),
    );

    assert_eq!(
        state
            .tag_suggestion_state
            .as_ref()
            .map(|state| state.highlighted_index),
        Some(1)
    );

    let _ = update(
        &mut state,
        Message::TranscriptTagSuggestionKeyPressed(
            "annotation-1".to_string(),
            TagSuggestionKeyboardAction::Accept,
        ),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.text, "[смех]");
    assert_eq!(draft.text, draft.editor_text());
    assert!(state.editor.as_ref().unwrap().is_dirty);
    assert!(state.tag_suggestion_state.is_none());
}

#[test]
fn tag_suggestion_escape_dismisses_current_trigger_until_text_changes() {
    let mut state = stable_sample_state();
    set_draft_text_with_cursor_at_end(&mut state, "annotation-1", "[сме");

    let _ = update(
        &mut state,
        Message::TranscriptTagSuggestionKeyPressed(
            "annotation-1".to_string(),
            TagSuggestionKeyboardAction::Dismiss,
        ),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(!state.should_show_tag_suggestions("annotation-1", draft));
    assert_eq!(
        state
            .tag_suggestion_state
            .as_ref()
            .map(|state| state.dismissed),
        Some(true)
    );

    let _ = update(
        &mut state,
        Message::SegmentTextEdited(
            "annotation-1".to_string(),
            TextEditAction::Edit(TextEdit::Paste("х".to_string().into())),
        ),
    );

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(state.should_show_tag_suggestions("annotation-1", draft));
    assert!(state.tag_suggestion_state.is_none());
}

#[test]
fn timestamp_parser_matches_recovered_babel_mm_ss_centiseconds_format() {
    assert_eq!(parse_seconds("Start", "01:05.08").unwrap(), 65.08);
    assert_eq!(parse_seconds("Start", "00:03.50").unwrap(), 3.5);
    assert_eq!(format_recovered_time(59.995), "01:00.00");
    assert_eq!(
        parse_seconds("Start", "3.5"),
        Err("Invalid format".to_string())
    );
    assert_eq!(
        parse_seconds("Start", "01:05.8"),
        Err("Invalid format".to_string())
    );
    assert_eq!(
        parse_seconds("Start", "001:05.80"),
        Err("Invalid format".to_string())
    );
    assert_eq!(
        parse_seconds("Start", "01:"),
        Err("Invalid format".to_string())
    );
    assert!(parse_seconds("Start", "01:60.00").is_err());
}

#[test]
fn time_cell_edit_commit_matches_recovered_babel_submit_and_blur_rules() {
    let invalid_submit = commit_time_cell_edit(CommitTimeCellEditInput {
        active_field: Some(TimeField::Start),
        field: TimeField::Start,
        draft_value: "00:03.40".to_string(),
        value: 1.2,
        start_seconds: 1.2,
        end_seconds: 3.4,
        annotation_id: "annotation-1".to_string(),
        from_blur: false,
    });
    assert_eq!(
        invalid_submit,
        CommitTimeCellEditResult::Handled {
            active_field: Some(TimeField::Start),
            draft_value: "00:03.40".to_string(),
            error: Some("Start must be earlier than end".to_string()),
            should_call_on_time_change: false,
            annotation_id: "annotation-1".to_string(),
            next_times: None,
        }
    );

    let invalid_blur = commit_time_cell_edit(CommitTimeCellEditInput {
        active_field: Some(TimeField::Start),
        field: TimeField::Start,
        draft_value: "not a time".to_string(),
        value: 1.2,
        start_seconds: 1.2,
        end_seconds: 3.4,
        annotation_id: "annotation-1".to_string(),
        from_blur: true,
    });
    assert_eq!(
        invalid_blur,
        CommitTimeCellEditResult::Handled {
            active_field: None,
            draft_value: "00:01.20".to_string(),
            error: None,
            should_call_on_time_change: false,
            annotation_id: "annotation-1".to_string(),
            next_times: None,
        }
    );

    let valid_submit = commit_time_cell_edit(CommitTimeCellEditInput {
        active_field: Some(TimeField::End),
        field: TimeField::End,
        draft_value: "00:03.75".to_string(),
        value: 3.4,
        start_seconds: 1.2,
        end_seconds: 3.4,
        annotation_id: "annotation-1".to_string(),
        from_blur: false,
    });
    assert_eq!(
        valid_submit,
        CommitTimeCellEditResult::Handled {
            active_field: None,
            draft_value: "00:03.75".to_string(),
            error: None,
            should_call_on_time_change: true,
            annotation_id: "annotation-1".to_string(),
            next_times: Some(TimeRangeEdit {
                start_seconds: 1.2,
                end_seconds: 3.75,
            }),
        }
    );
}

#[test]
fn submitting_time_cell_edit_updates_boundary_without_flushing_text_draft() {
    let mut state = stable_sample_state();
    let original_text = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .text
        .clone();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("text draft still being edited".to_string());

    let _ = update(
        &mut state,
        Message::SegmentTimeChanged(
            "annotation-1".to_string(),
            TimeField::Start,
            "00:01.25".to_string(),
        ),
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);

    let _ = update(
        &mut state,
        Message::CommitSegmentTimeEdit {
            segment_id: "annotation-1".to_string(),
            field: TimeField::Start,
            from_blur: false,
        },
    );

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.start_seconds, 1.25);
    assert_eq!(segment.end_seconds, 3.4);
    assert_eq!(segment.text, original_text);
    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.start_seconds, "00:01.25");
    assert_eq!(draft.text, "text draft still being edited");
    assert_eq!(draft.time_edit, TimeEditState::default());
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn submitting_time_cell_edit_preserves_timeline_viewport() {
    let mut state = stable_sample_state();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 1.0,
            end_seconds: 6.0,
        };
        editor.playhead_seconds = 80.0;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::SegmentTimeChanged(
            "annotation-1".to_string(),
            TimeField::Start,
            "00:01.25".to_string(),
        ),
    );
    let _ = update(
        &mut state,
        Message::CommitSegmentTimeEdit {
            segment_id: "annotation-1".to_string(),
            field: TimeField::Start,
            from_blur: false,
        },
    );

    let editor = state.editor.as_ref().unwrap();
    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.start_seconds, 1.25);
    assert_eq!(editor.playhead_seconds, 80.0);
    assert_eq!(editor.viewport, before_viewport);
}

#[test]
fn invalid_time_cell_submit_stays_local_and_reports_babel_error() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::SegmentTimeChanged(
            "annotation-1".to_string(),
            TimeField::Start,
            "00:03.40".to_string(),
        ),
    );
    let _ = update(
        &mut state,
        Message::CommitSegmentTimeEdit {
            segment_id: "annotation-1".to_string(),
            field: TimeField::Start,
            from_blur: false,
        },
    );

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.start_seconds, 1.2);
    assert_eq!(segment.end_seconds, 3.4);
    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert_eq!(draft.start_seconds, "00:03.40");
    assert_eq!(draft.time_edit.active_field, Some(TimeField::Start));
    assert_eq!(
        draft.time_edit.error,
        Some("Start must be earlier than end".to_string())
    );
    assert_eq!(state.status, "Start must be earlier than end");
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn applying_text_only_preserves_precise_timing() {
    let mut state = stable_sample_state();
    let project = state.workflow.as_mut().unwrap().project();
    assert!(project.segment("annotation-1").is_some());
    let mut project = project.clone();
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 1.23456;
    segment.end_seconds = 3.45678;
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded precise timing fixture".to_string(),
    );

    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("Only text changed.".to_string());
    state.apply_segment("annotation-1");

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.start_seconds, 1.23456);
    assert_eq!(segment.end_seconds, 3.45678);
    assert_eq!(segment.text, "Only text changed.");
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn applying_text_and_timing_undoes_as_one_step() {
    let mut state = stable_sample_state();
    let draft = state.segment_drafts.get_mut("annotation-1").unwrap();
    draft.start_seconds = "1.500".to_string();
    draft.end_seconds = "3.750".to_string();
    draft.replace_text("Updated together.".to_string());

    state.apply_segment("annotation-1");
    state.undo();

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.start_seconds, 1.2);
    assert_eq!(segment.end_seconds, 3.4);
    assert_eq!(segment.text, "Здравствуйте.");
}

#[test]
fn unrelated_actions_preserve_unapplied_drafts() {
    let mut state = stable_sample_state();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text("draft still being typed".to_string());

    state.split_segment("annotation-2");

    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "draft still being typed"
    );
}

#[test]
fn split_uses_typed_split_time() {
    let mut state = stable_sample_state();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .split_seconds = "00:02.00".to_string();

    state.split_segment("annotation-1");

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.end_seconds, 1.95);
}

#[test]
fn row_split_uses_typed_split_time_even_when_playhead_is_inside_the_segment() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    state.editor.as_mut().unwrap().playhead_seconds = 2.75;
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .split_seconds = "00:02.00".to_string();

    state.split_segment("annotation-1");

    let project = state.workflow.as_ref().unwrap().project();
    let left = project.segment("annotation-1").unwrap();
    let right = project.segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.end_seconds, 1.95);
    assert_eq!(right.start_seconds, 2.05);
}

#[test]
fn split_smart_allocates_text_by_babel_word_ratio() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 0.0;
    segment.end_seconds = 4.0;
    segment.text = "one two three four five".to_string();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded smart split fixture".to_string(),
    );
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .split_seconds = "00:02.00".to_string();

    state.split_segment("annotation-1");

    let project = state.workflow.as_ref().unwrap().project();
    let left = project.segment("annotation-1").unwrap();
    let right = project.segment("annotation-1__studio_1").unwrap();
    assert_eq!(left.text, "one two three");
    assert_eq!(right.text, "four five");
    assert_eq!(
        state.segment_drafts.get("annotation-1").unwrap().text,
        "one two three"
    );
    assert_eq!(
        state
            .segment_drafts
            .get("annotation-1__studio_1")
            .unwrap()
            .text,
        "four five"
    );
}

#[test]
fn split_rejects_babel_tail_guard_before_workflow_mutation() {
    let mut state = stable_sample_state();
    let before = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .split_seconds = "00:03.31".to_string();

    state.split_segment("annotation-1");

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segment("annotation-1").unwrap(), &before);
    assert!(project.segment("annotation-1__studio_1").is_none());
    assert_eq!(
        state.status,
        "Split must be at least 100ms before the segment end."
    );
}

#[test]
fn waveform_command_press_splits_at_engine_hit_time_without_block_fraction() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::COMMAND),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: 2.85,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );

    let project = state.workflow.as_ref().unwrap().project();
    let left = project.segment("annotation-1").unwrap();
    let right = project.segment("annotation-1__studio_1").unwrap();
    assert!((left.end_seconds - 2.8).abs() < f64::EPSILON * 8.0);
    assert!((right.start_seconds - 2.9).abs() < f64::EPSILON * 8.0);
}

#[test]
fn waveform_segment_click_selects_without_moving_playhead() {
    let mut state = stable_sample_state();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 8.75;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: 1.5,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-1".to_string()),
    );

    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-1")
    );
    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 8.75);
    assert_eq!(state.editor.as_ref().unwrap().viewport, before_viewport);
    assert!(!state.audio_playback.is_active());
    assert!(state.active_waveform_drag.is_none());
}

#[test]
fn waveform_segment_double_click_selects_without_moving_playhead() {
    let mut state = stable_sample_state();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 8.75;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    for time_seconds in [1.5, 1.7] {
        let _ = update(
            &mut state,
            Message::WaveformHitHovered(
                "track-1".to_string(),
                WaveformHit {
                    target: WaveformHitTarget::RegionBody,
                    region_id: Some("annotation-1".to_string()),
                    time_seconds,
                },
            ),
        );
        let _ = update(
            &mut state,
            Message::WaveformHitPressed("track-1".to_string()),
        );
        let _ = update(
            &mut state,
            Message::WaveformHitReleased("track-1".to_string()),
        );

        if time_seconds == 1.5 {
            assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 8.75);
        }
    }

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-1"));
    assert_eq!(editor.playhead_seconds, 8.75);
    assert_eq!(editor.viewport, before_viewport);
    assert!(!state.audio_playback.is_active());
    assert!(state.active_waveform_drag.is_none());
}

#[test]
fn transcript_row_click_selects_without_moving_playhead() {
    let mut state = stable_sample_state();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 8.75;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::TranscriptRowBodyPressed("annotation-1".to_string()),
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-1"));
    assert_eq!(editor.playhead_seconds, 8.75);
    assert_eq!(editor.viewport, before_viewport);
    assert!(!state.audio_playback.is_active());
}

#[test]
fn transcript_row_double_click_moves_playhead_to_segment_start() {
    let mut state = stable_sample_state();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 8.75;
    });
    let segment_start = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .start_seconds;

    let _ = update(
        &mut state,
        Message::TranscriptRowBodyPressed("annotation-1".to_string()),
    );
    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 8.75);

    let _ = update(
        &mut state,
        Message::TranscriptRowBodyPressed("annotation-1".to_string()),
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-1"));
    assert_eq!(editor.playhead_seconds, segment_start);
    assert!(!state.audio_playback.is_active());
}

#[test]
fn repeated_waveform_segment_click_does_not_start_region_playback() {
    let mut state = stable_sample_state();

    for _ in 0..2 {
        let _ = update(
            &mut state,
            Message::WaveformHitHovered(
                "track-1".to_string(),
                WaveformHit {
                    target: WaveformHitTarget::RegionBody,
                    region_id: Some("annotation-1".to_string()),
                    time_seconds: 1.5,
                },
            ),
        );
        let _ = update(
            &mut state,
            Message::WaveformHitPressed("track-1".to_string()),
        );
        let _ = update(
            &mut state,
            Message::WaveformHitReleased("track-1".to_string()),
        );
    }

    assert!(!state.audio_playback.is_active());
    assert_eq!(state.audio_playback.last_end_seconds, None);
    assert!(state.active_waveform_drag.is_none());
}

#[test]
fn waveform_hover_exposes_babel_region_label_and_tooltip_text() {
    let mut state = stable_sample_state();
    let project = state.workflow.as_ref().unwrap().project();
    let segment = project.segment("annotation-1").unwrap();
    let editor = state.editor.as_ref().unwrap();
    let lane = editor
        .timeline_lanes(project)
        .into_iter()
        .find(|lane| lane.lane_id == segment.lane_id)
        .unwrap();
    let block = lane
        .blocks
        .into_iter()
        .find(|block| block.segment_id == "annotation-1")
        .unwrap();

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: 1.5,
            },
        ),
    );

    let hover = state.waveform_hit_hover.as_ref().unwrap();
    assert_eq!(hover.region_label.as_deref(), Some(block.label.as_str()));
    assert_eq!(
        hover.region_tooltip.as_deref(),
        Some(block.tooltip.as_str())
    );
}

#[test]
fn identical_waveform_hover_without_drag_keeps_cached_hover_state() {
    let mut state = stable_sample_state();
    let hit = WaveformHit {
        target: WaveformHitTarget::RegionBody,
        region_id: Some("annotation-1".to_string()),
        time_seconds: 1.5,
    };

    let _ = update(
        &mut state,
        Message::WaveformHitHovered("track-1".to_string(), hit.clone()),
    );
    state
        .waveform_hit_hover
        .as_mut()
        .expect("first hover should cache hover state")
        .region_tooltip = Some("cached tooltip".to_string());

    let _ = update(
        &mut state,
        Message::WaveformHitHovered("track-1".to_string(), hit),
    );

    assert_eq!(
        state
            .waveform_hit_hover
            .as_ref()
            .and_then(|hover| hover.region_tooltip.as_deref()),
        Some("cached tooltip")
    );
}

#[test]
fn waveform_start_handle_drag_updates_segment_start_from_engine_hit_time() {
    let mut state = stable_sample_state();
    let original_end = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .end_seconds;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some("annotation-1".to_string()),
                time_seconds: 1.0,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some("annotation-1".to_string()),
                time_seconds: 1.25,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-1".to_string()),
    );

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.start_seconds, 1.25);
    assert_eq!(segment.end_seconds, original_end);
    assert!(state.active_waveform_drag.is_none());
}

#[test]
fn waveform_drag_hover_previews_segment_without_dirtying_project_until_release() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let lane_id = original.lane_id.clone();

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some("annotation-1".to_string()),
                time_seconds: original.start_seconds,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some("annotation-1".to_string()),
                time_seconds: original.start_seconds + 0.25,
            },
        ),
    );

    let during_hover = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(during_hover.start_seconds, original.start_seconds);
    assert_eq!(during_hover.end_seconds, original.end_seconds);
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    let drag = state
        .active_waveform_drag
        .as_ref()
        .expect("drag should remain active until pointer release");
    assert_eq!(drag.preview_start_seconds, original.start_seconds + 0.25);
    assert_eq!(drag.preview_end_seconds, original.end_seconds);

    let _ = update(&mut state, Message::WaveformHitReleased(lane_id));

    let committed = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(committed.start_seconds, original.start_seconds + 0.25);
    assert_eq!(committed.end_seconds, original.end_seconds);
    assert!(state.editor.as_ref().unwrap().is_dirty);
    assert!(state.active_waveform_drag.is_none());
}

#[test]
fn waveform_drag_preview_updates_timeline_block_before_release() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let lane_id = original.lane_id.clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: (original.start_seconds - 0.2).max(0.0),
            end_seconds: original.end_seconds + 1.0,
        };
    });

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time + 0.4,
            },
        ),
    );

    let project = state.project().unwrap().clone();
    let lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let preview_block = lanes
        .iter()
        .find(|lane| lane.lane_id == lane_id)
        .unwrap()
        .blocks
        .iter()
        .find(|block| block.segment_id == "annotation-1")
        .unwrap();
    assert!((preview_block.start_seconds - (original.start_seconds + 0.4)).abs() < 0.000_001);
    assert!((preview_block.end_seconds - (original.end_seconds + 0.4)).abs() < 0.000_001);

    let project_segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(project_segment.start_seconds, original.start_seconds);
    assert_eq!(project_segment.end_seconds, original.end_seconds);
}

#[test]
fn waveform_edge_drag_exposes_recovered_magnifier_overlay_model() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let lane_id = original.lane_id.clone();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 0.0,
            end_seconds: 10.0,
        };
    });

    assert!(
        state
            .waveform_magnifier_overlay_for_lane(&lane_id)
            .is_none()
    );

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some("annotation-1".to_string()),
                time_seconds: original.start_seconds,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some("annotation-1".to_string()),
                time_seconds: original.start_seconds + 0.25,
            },
        ),
    );

    let overlay = state
        .waveform_magnifier_overlay_for_lane(&lane_id)
        .expect("edge trim drag should expose Babel's recovered magnifier overlay");
    assert_eq!(overlay.scale, 3.0);
    assert_eq!(overlay.width_px, 180.0);
    assert_eq!(overlay.max_height_px, 150.0);
    assert!((overlay.time_seconds - (original.start_seconds + 0.25)).abs() < 0.000_001);
    assert!((overlay.fraction - ((original.start_seconds + 0.25) / 10.0) as f32).abs() < 0.000_001);
    assert!(
        state
            .waveform_magnifier_overlay_for_lane("missing-lane")
            .is_none()
    );

    let _ = update(&mut state, Message::WaveformHitReleased(lane_id.clone()));
    assert!(
        state
            .waveform_magnifier_overlay_for_lane(&lane_id)
            .is_none()
    );
}

#[test]
fn waveform_body_drag_does_not_show_edge_magnifier() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let lane_id = original.lane_id.clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time + 0.2,
            },
        ),
    );

    assert!(
        state
            .waveform_magnifier_overlay_for_lane(&lane_id)
            .is_none(),
        "Babel shows the 3x magnifier for edge trims, not whole-region drags"
    );
}

#[test]
fn waveform_drag_preview_reuses_prewarmed_segment_block_peaks() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded waveform edge drag cache project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.viewport = TimeRange {
            start_seconds: 20.0,
            end_seconds: 25.0,
        };
    });

    let project = state.project().unwrap().clone();
    let original = project
        .segment("annotation-0002")
        .expect("track-1 segment should exist in the first timeline viewport")
        .clone();
    {
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }
    let stats_after_prewarm = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .segment_peak_cache_stats();

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: original.start_seconds,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed(original.lane_id.clone()),
    );

    for offset in [0.05, 0.1, 0.15, 0.2] {
        let preview_start = original.start_seconds + offset;
        let _ = update(
            &mut state,
            Message::WaveformHitHovered(
                original.lane_id.clone(),
                WaveformHit {
                    target: WaveformHitTarget::RegionStartHandle,
                    region_id: Some(original.id.clone()),
                    time_seconds: preview_start,
                },
            ),
        );
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            state.timeline_lanes_snapshot_for_render(&project, editor)
        };
        let preview_block = lanes
            .iter()
            .find(|lane| lane.lane_id == original.lane_id)
            .unwrap()
            .blocks
            .iter()
            .find(|block| block.segment_id == original.id)
            .unwrap();
        assert!((preview_block.start_seconds - preview_start).abs() < 0.000_001);
    }

    let stats_after_drag_previews = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .segment_peak_cache_stats();
    assert_eq!(
        stats_after_drag_previews.misses, stats_after_prewarm.misses,
        "edge-drag preview frames should reuse the original segment's prewarmed block data instead of recomputing peaks for each temporary boundary"
    );
}

#[test]
fn edge_drag_preview_keeps_full_region_fill_for_existing_segment() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded waveform edge drag jitter project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.viewport = TimeRange {
            start_seconds: 20.0,
            end_seconds: 25.0,
        };
    });

    let project = state.project().unwrap().clone();
    let original = project
        .segment("annotation-0002")
        .expect("track-1 segment should exist in the first timeline viewport")
        .clone();
    {
        let editor = state.editor.as_ref().unwrap();
        let lanes = state.timeline_lanes_snapshot_for_render(&project, editor);
        let lane = lanes
            .iter()
            .find(|lane| lane.lane_id == original.lane_id)
            .unwrap();
        assert!(lane.blocks.len() < lane.strip_blocks.len());
    }

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: original.start_seconds,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed(original.lane_id.clone()),
    );

    let preview_start = original.start_seconds + 0.2;
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: preview_start,
            },
        ),
    );

    let lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let lane = lanes
        .iter()
        .find(|lane| lane.lane_id == original.lane_id)
        .unwrap();
    let preview_block = lane
        .blocks
        .iter()
        .find(|block| block.segment_id == original.id)
        .unwrap();
    assert!((preview_block.start_seconds - preview_start).abs() < 0.000_001);
    assert!((preview_block.end_seconds - original.end_seconds).abs() < 0.000_001);

    let strip_block = lane
        .strip_blocks
        .iter()
        .find(|block| block.segment_id == original.id)
        .unwrap();
    assert!(
        (strip_block.start_seconds - preview_start).abs() < 0.000_001,
        "edge-drag preview should keep drawing the full preview region fill, not just a handle/border"
    );
    assert!((strip_block.end_seconds - original.end_seconds).abs() < 0.000_001);
}

#[test]
fn waveform_edge_drag_does_not_seek_playhead_to_grab_time() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 8.75;
    });

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: original.start_seconds,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed(original.lane_id.clone()),
    );

    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 8.75);

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: original.start_seconds + 0.2,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased(original.lane_id.clone()),
    );

    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 8.75);
}

#[test]
fn waveform_edge_drag_commit_preserves_viewport_instead_of_recentering_playhead() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 1.0,
            end_seconds: 6.0,
        };
        editor.playhead_seconds = 80.0;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: original.start_seconds,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed(original.lane_id.clone()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            original.lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::RegionStartHandle,
                region_id: Some(original.id.clone()),
                time_seconds: original.start_seconds + 0.2,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased(original.lane_id.clone()),
    );

    let editor = state.editor.as_ref().unwrap();
    let committed = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(committed.start_seconds, original.start_seconds + 0.2);
    assert_eq!(committed.end_seconds, original.end_seconds);
    assert_eq!(editor.playhead_seconds, 80.0);
    assert_eq!(editor.viewport, before_viewport);
}

#[test]
fn waveform_region_body_drag_moves_segment_without_changing_duration() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let duration = original.end_seconds - original.start_seconds;
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time + 0.4,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-1".to_string()),
    );

    let moved = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert!((moved.start_seconds - (original.start_seconds + 0.4)).abs() < f64::EPSILON * 8.0);
    assert!((moved.end_seconds - moved.start_seconds - duration).abs() < f64::EPSILON * 8.0);
}

#[test]
fn global_pointer_release_finishes_captured_waveform_drag() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time + 0.25,
            },
        ),
    );
    assert!(state.active_waveform_drag.is_some());

    let _ = update(&mut state, Message::GlobalPointerReleased);

    let moved = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert!(state.active_waveform_drag.is_none());
    assert!((moved.start_seconds - (original.start_seconds + 0.25)).abs() < f64::EPSILON * 8.0);
}

#[test]
fn waveform_body_drag_inside_active_cut_preview_locks_without_editing_project() {
    let mut state = stable_sample_state();
    let lane_id = state.project().unwrap().lanes[0].id.clone();
    let original_count = state.project().unwrap().segments.len();

    let _ = update(&mut state, Message::ToggleOverlay);
    let initial = state.editor.as_ref().unwrap().overlay_range.unwrap();

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: initial.start_seconds + 0.4,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: initial.start_seconds + 2.0,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitReleased(lane_id));

    let overlay = state.editor.as_ref().unwrap().overlay_range.unwrap();
    assert_eq!(state.project().unwrap().segments.len(), original_count);
    assert!(state.create_region_dialog.is_none());
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert_eq!(overlay, initial);
}

#[test]
fn command_waveform_body_drag_inside_active_overlay_moves_babel_overlay_without_editing_project() {
    let mut state = stable_sample_state();
    let lane_id = state.project().unwrap().lanes[0].id.clone();
    let original_count = state.project().unwrap().segments.len();

    let _ = update(&mut state, Message::ToggleOverlay);
    let initial = state.editor.as_ref().unwrap().overlay_range.unwrap();
    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::COMMAND),
    );

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: initial.start_seconds + 0.4,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: initial.start_seconds + 2.0,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitReleased(lane_id));

    let overlay = state.editor.as_ref().unwrap().overlay_range.unwrap();
    assert_eq!(state.project().unwrap().segments.len(), original_count);
    assert!(state.create_region_dialog.is_none());
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert!((overlay.start_seconds - (initial.start_seconds + 1.6)).abs() < 0.001);
    assert!(
        (overlay.end_seconds - overlay.start_seconds - initial.duration_seconds()).abs() < 0.001
    );
}

#[test]
fn cut_preview_overlay_edge_drag_resizes_with_recovered_one_second_floor() {
    let mut state = stable_sample_state();
    let lane_id = state.project().unwrap().lanes[0].id.clone();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 0.0,
            end_seconds: 10.0,
        };
        editor.overlay_range = Some(TimeRange {
            start_seconds: 2.0,
            end_seconds: 4.0,
        });
    });

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 2.0,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 2.6,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitReleased(lane_id.clone()));

    let overlay = state.editor.as_ref().unwrap().overlay_range.unwrap();
    assert!((overlay.start_seconds - 2.6).abs() < 0.001);
    assert!((overlay.end_seconds - 4.0).abs() < 0.001);

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 4.0,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 2.8,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitReleased(lane_id));

    let overlay = state.editor.as_ref().unwrap().overlay_range.unwrap();
    assert!((overlay.start_seconds - 2.6).abs() < 0.001);
    assert!((overlay.end_seconds - 3.6).abs() < 0.001);
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn alt_waveform_drag_creates_cut_preview_overlay_without_editing_project() {
    let mut state = stable_sample_state();
    let lane_id = state.project().unwrap().lanes[0].id.clone();
    let original_count = state.project().unwrap().segments.len();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 0.0,
            end_seconds: 10.0,
        };
        editor.overlay_range = Some(TimeRange {
            start_seconds: 1.0,
            end_seconds: 2.0,
        });
    });
    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::ALT),
    );

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 3.0,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    assert!(state.editor.as_ref().unwrap().overlay_range.is_none());
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 5.25,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitReleased(lane_id));

    let overlay = state.editor.as_ref().unwrap().overlay_range.unwrap();
    assert_eq!(state.project().unwrap().segments.len(), original_count);
    assert!(state.create_region_dialog.is_none());
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert!((overlay.start_seconds - 3.0).abs() < 0.001);
    assert!((overlay.end_seconds - 5.25).abs() < 0.001);
}

#[test]
fn alt_waveform_tiny_drag_keeps_cut_preview_draft_unmaterialized() {
    let mut state = stable_sample_state();
    let lane_id = state.project().unwrap().lanes[0].id.clone();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 0.0,
            end_seconds: 10.0,
        };
        editor.overlay_range = None;
    });
    let _ = update(
        &mut state,
        Message::KeyboardModifiersChanged(Modifiers::ALT),
    );

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 3.0,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitPressed(lane_id.clone()));
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            lane_id.clone(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 3.02,
            },
        ),
    );
    let _ = update(&mut state, Message::WaveformHitReleased(lane_id));

    assert!(state.editor.as_ref().unwrap().overlay_range.is_none());
    assert!(state.active_overlay_drag.is_none());
}

#[test]
fn empty_waveform_drag_creates_segment_and_opens_babel_add_region_card() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    state.with_editor_mut(|editor| {
        editor.viewport = TimeRange {
            start_seconds: 11.0,
            end_seconds: 14.0,
        };
        editor.playhead_seconds = 80.0;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 12.0,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 12.8,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-speaker-1".to_string()),
    );

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), original_count + 1);
    let dialog = state
        .create_region_dialog
        .as_ref()
        .expect("empty waveform drag should open Add Region");
    let created_segment_id = dialog
        .created_segment_id
        .as_ref()
        .expect("Babel creates the blank segment before opening the card");
    let created = project
        .segment(created_segment_id)
        .expect("created segment should be in the workflow immediately");
    assert_eq!(dialog.selected_lane_id, "track-speaker-1");
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.start_draft, "00:12.00");
    assert_eq!(dialog.end_draft, "00:12.80");
    assert!(dialog.text.is_empty());
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    assert!(created.text.is_empty());
    assert!(state.active_waveform_create_drag.is_none());
    assert!(state.editor.as_ref().unwrap().is_dirty);
    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 80.0);
    assert_eq!(state.editor.as_ref().unwrap().viewport, before_viewport);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some(created_segment_id.as_str())
    );
}

#[test]
fn attached_create_region_text_populates_created_row_draft_immediately() {
    let mut state = stable_sample_state();
    state.editor.as_mut().unwrap().viewport = TimeRange {
        start_seconds: 11.0,
        end_seconds: 14.0,
    };

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 12.0,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 12.8,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-speaker-1".to_string()),
    );

    let created_segment_id = state
        .create_region_dialog
        .as_ref()
        .and_then(|dialog| dialog.created_segment_id.clone())
        .expect("attached Add Region popup should track the created segment");

    let _ = update(
        &mut state,
        Message::CreateRegionTextChanged("new segment text".to_string()),
    );

    let dialog = state
        .create_region_dialog
        .as_ref()
        .expect("popup should stay open while typing");
    assert_eq!(dialog.text, "new segment text");
    let draft = state
        .segment_drafts
        .get(&created_segment_id)
        .expect("created segment row should have a live draft");
    assert_eq!(draft.text, "new segment text");
    assert_eq!(draft.editor_text(), "new segment text");
}

#[test]
fn empty_waveform_drag_previews_created_segment_before_release() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    state.editor.as_mut().unwrap().viewport = TimeRange {
        start_seconds: 11.0,
        end_seconds: 14.0,
    };

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 12.0,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 12.8,
            },
        ),
    );

    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count,
        "create preview should not commit a segment before release"
    );
    let project = state.project().unwrap().clone();
    let lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let preview_block = lanes
        .iter()
        .find(|lane| lane.lane_id == "track-speaker-1")
        .unwrap()
        .blocks
        .iter()
        .find(|block| block.segment_id == "__studio_create_preview__")
        .expect("dragging an empty waveform range should render a live create preview");
    assert!((preview_block.start_seconds - 12.0).abs() < 0.000_001);
    assert!((preview_block.end_seconds - 12.8).abs() < 0.000_001);
    assert!(state.active_waveform_create_drag.is_some());
}

#[test]
fn babel_day_to_day_mismatch_empty_waveform_click_seeks_playhead_without_short_region_error() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    let target_time = 12.35;
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 80.0;
    });

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: target_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );

    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 80.0);
    assert!(state.active_waveform_create_drag.is_some());

    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-speaker-1".to_string()),
    );

    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, target_time);
    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count
    );
    assert!(state.create_region_dialog.is_none());
    assert!(state.active_waveform_create_drag.is_none());
    assert_ne!(state.status, "Created region is too short.");
}

#[test]
fn empty_waveform_tiny_pointer_jitter_stays_click_not_create_drag() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    let target_time = 12.0;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: target_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: target_time + 0.01,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-speaker-1".to_string()),
    );

    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, target_time);
    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count
    );
    assert!(state.create_region_dialog.is_none());
    assert!(state.active_waveform_create_drag.is_none());
}

#[test]
fn empty_waveform_reverse_drag_normalizes_babel_create_region_dialog_range() {
    let mut state = stable_sample_state();
    state.editor.as_mut().unwrap().viewport = TimeRange {
        start_seconds: 13.0,
        end_seconds: 15.0,
    };

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 14.0,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: 13.2,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-speaker-1".to_string()),
    );

    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.start_seconds, 13.2);
    assert_eq!(dialog.end_seconds, 14.0);
    assert_eq!(dialog.start_draft, "00:13.20");
    assert_eq!(dialog.end_draft, "00:14.00");
}

#[test]
fn create_region_speaker_picker_uses_babel_track_labels() {
    let state = stable_sample_state();
    let options = create_region_track_options(state.project().unwrap());

    assert_eq!(
        options
            .iter()
            .map(|option| (option.lane_id.as_str(), option.label.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("track-speaker-1", "Speaker 1"),
            ("track-speaker-2", "Speaker 2"),
        ]
    );
    assert_eq!(options[0].to_string(), "Speaker 1");
}

#[test]
fn create_region_create_enablement_matches_recovered_babel_transcription_rules() {
    let mut dialog = CreateRegionDialog::new("track-speaker-1".to_string(), 12.0, 12.8);

    assert!(dialog.is_create_enabled());

    dialog.selected_lane_id = String::new();
    assert!(!dialog.is_create_enabled());

    dialog.selected_lane_id = "track-speaker-1".to_string();
    dialog.start_error = Some("Invalid format".to_string());
    assert!(!dialog.is_create_enabled());

    dialog.start_error = None;
    dialog.end_error = Some("Invalid format".to_string());
    assert!(!dialog.is_create_enabled());

    dialog.end_error = None;
    dialog.start_seconds = 12.8;
    assert!(!dialog.is_create_enabled());
}

#[test]
fn create_region_annotation_enablement_matches_recovered_babel_category_rules() {
    let mut dialog = CreateRegionDialog::new_annotation("track-speaker-1".to_string(), 12.0, 12.8);

    assert_eq!(dialog.mode, CreateRegionMode::Annotation);
    assert!(!dialog.is_create_enabled());

    dialog.intensity = Some(3);
    dialog.primary_selection = "background-noise".to_string();
    assert!(!dialog.is_create_enabled());

    dialog.secondary_selection = "rustle".to_string();
    assert!(dialog.is_create_enabled());

    dialog.secondary_selection = "other".to_string();
    assert!(!dialog.is_create_enabled());

    dialog.custom_description = "chair scrape".to_string();
    assert!(dialog.is_create_enabled());
}

#[test]
fn create_region_annotation_requires_explicit_single_secondary_choice_like_recovered_babel() {
    let mut dialog = CreateRegionDialog::new_annotation("track-speaker-1".to_string(), 12.0, 12.8);
    dialog.intensity = Some(3);
    dialog.primary_selection = "background-speech".to_string();

    assert_eq!(
        create_region_secondary_options("background-speech"),
        vec!["background speech".to_string(), "other".to_string()]
    );
    assert!(!dialog.is_create_enabled());

    let mut state = stable_sample_state();
    let mut project = state_project_with_annotation_mode();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation-mode fixture".to_string(),
    );
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(
        &mut state,
        Message::CreateRegionPrimaryCategoryChanged("background-speech".to_string()),
    );
    let _ = update(
        &mut state,
        Message::CreateRegionIntensityChanged("3".to_string()),
    );
    let _ = update(&mut state, Message::SubmitCreateRegion);

    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count
    );
    assert!(state.create_region_dialog.is_some());
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn create_region_secondary_options_include_recovered_babel_other_choice() {
    assert_eq!(
        create_region_secondary_options("background-noise")
            .last()
            .map(String::as_str),
        Some("other")
    );
}

#[test]
fn create_region_secondary_changes_preserve_description_like_recovered_babel() {
    let mut state = stable_sample_state();
    state.create_region_dialog = Some(CreateRegionDialog::new_annotation(
        "track-speaker-1".to_string(),
        12.0,
        12.8,
    ));
    {
        let dialog = state.create_region_dialog.as_mut().unwrap();
        dialog.primary_selection = "background-noise".to_string();
        dialog.secondary_selection = "other".to_string();
        dialog.custom_description = "custom wind".to_string();
    }

    let _ = update(
        &mut state,
        Message::CreateRegionSecondaryCategoryChanged("wind".to_string()),
    );

    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.secondary_selection, "wind");
    assert_eq!(dialog.custom_description, "custom wind");

    let _ = update(
        &mut state,
        Message::CreateRegionSecondaryCategoryChanged("other".to_string()),
    );

    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.secondary_selection, "other");
    assert_eq!(dialog.custom_description, "");
}

#[test]
fn create_region_intensity_change_accepts_recovered_numeric_zero() {
    let mut state = stable_sample_state();
    state.create_region_dialog = Some(CreateRegionDialog::new_annotation(
        "track-speaker-1".to_string(),
        12.0,
        12.8,
    ));

    let _ = update(
        &mut state,
        Message::CreateRegionIntensityChanged("0".to_string()),
    );
    assert_eq!(
        state.create_region_dialog.as_ref().unwrap().intensity,
        Some(0)
    );

    let _ = update(
        &mut state,
        Message::CreateRegionIntensityChanged(String::new()),
    );
    assert_eq!(state.create_region_dialog.as_ref().unwrap().intensity, None);
}

#[test]
fn create_region_annotation_category_rules_are_project_id_gated_like_babel() {
    let mut fallback_project = state_project_with_annotation_mode();
    fallback_project.id = "ordinary-project".to_string();
    let mut fallback = CreateRegionDialog::new_for_project(
        &fallback_project,
        "track-speaker-1".to_string(),
        12.0,
        12.8,
    );

    assert_eq!(fallback.mode, CreateRegionMode::Annotation);
    assert!(!fallback.uses_hierarchical_labels);
    fallback.intensity = Some(2);
    assert!(fallback.is_create_enabled());

    let mut background_noise_project = fallback_project;
    background_noise_project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let mut hierarchical = CreateRegionDialog::new_for_project(
        &background_noise_project,
        "track-speaker-1".to_string(),
        12.0,
        12.8,
    );

    assert!(hierarchical.uses_hierarchical_labels);
    hierarchical.intensity = Some(2);
    hierarchical.primary_selection = "background-noise".to_string();
    assert!(!hierarchical.is_create_enabled());
    hierarchical.secondary_selection = "rustle".to_string();
    assert!(hierarchical.is_create_enabled());
}

#[test]
fn submitting_annotation_create_region_dialog_creates_babel_annotation_metadata() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    project.segments[0]
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation-mode fixture".to_string(),
    );
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();

    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.mode, CreateRegionMode::Annotation);

    let _ = update(
        &mut state,
        Message::CreateRegionPrimaryCategoryChanged("background-noise".to_string()),
    );
    let _ = update(
        &mut state,
        Message::CreateRegionSecondaryCategoryChanged("rustle".to_string()),
    );
    let _ = update(
        &mut state,
        Message::CreateRegionIntensityChanged("3".to_string()),
    );
    let _ = update(&mut state, Message::SubmitCreateRegion);

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), original_count + 1);
    let created_id = state
        .editor
        .as_ref()
        .unwrap()
        .selected_segment_id
        .as_ref()
        .unwrap();
    let created = project.segment(created_id).unwrap();
    assert_eq!(
        created.metadata.get("type"),
        Some(&serde_json::json!("background_noise"))
    );
    assert_eq!(
        created.metadata.get("intensity"),
        Some(&serde_json::json!(3))
    );
    assert_eq!(
        created.metadata.get("content"),
        Some(&serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
    assert!(state.create_region_dialog.is_none());
}

#[test]
fn submitting_babel_create_region_dialog_creates_segment_with_text_and_closes() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(
        &mut state,
        Message::CreateRegionTextChanged("hello new region".to_string()),
    );
    let _ = update(&mut state, Message::SubmitCreateRegion);

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), original_count + 1);
    let created_id = state
        .editor
        .as_ref()
        .unwrap()
        .selected_segment_id
        .as_ref()
        .unwrap();
    let created = project.segment(created_id).unwrap();
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.speaker_id, "speaker-1");
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    assert_eq!(created.text, "hello new region");
    assert!(state.create_region_dialog.is_none());
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn create_region_attached_card_keeps_drag_created_segment_lane_like_browser() {
    let mut state = stable_sample_state();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(
        &mut state,
        Message::CreateRegionTrackChanged("track-speaker-2".to_string()),
    );
    let _ = update(&mut state, Message::SubmitCreateRegion);

    let project = state.workflow.as_ref().unwrap().project();
    let created_id = state
        .editor
        .as_ref()
        .unwrap()
        .selected_segment_id
        .as_ref()
        .unwrap();
    let created = project.segment(created_id).unwrap();
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.speaker_id, "speaker-1");
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
}

#[test]
fn create_region_start_enter_updates_attached_dialog_without_creating_another_segment() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);
    let created_segment_id = state
        .create_region_dialog
        .as_ref()
        .unwrap()
        .created_segment_id
        .as_ref()
        .unwrap()
        .clone();
    let count_after_open = state.workflow.as_ref().unwrap().project().segments.len();
    assert_eq!(count_after_open, original_count + 1);

    let _ = update(
        &mut state,
        Message::CreateRegionStartChanged("00:12.20".to_string()),
    );
    let _ = update(&mut state, Message::CommitCreateRegionStart);

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), count_after_open);
    let created = project.segment(&created_segment_id).unwrap();
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.start_seconds, 12.2);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.start_draft, "00:12.20");
    assert_eq!(dialog.end_draft, "00:12.80");
    assert_eq!(dialog.start_error, None);
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn create_region_end_enter_reports_invalid_range_without_creating_another_segment() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);
    let created_segment_id = state
        .create_region_dialog
        .as_ref()
        .unwrap()
        .created_segment_id
        .as_ref()
        .unwrap()
        .clone();
    let count_after_open = state.workflow.as_ref().unwrap().project().segments.len();
    assert_eq!(count_after_open, original_count + 1);

    let _ = update(
        &mut state,
        Message::CreateRegionEndChanged("00:11.90".to_string()),
    );
    let _ = update(&mut state, Message::CommitCreateRegionEnd);

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), count_after_open);
    let created = project.segment(&created_segment_id).unwrap();
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.end_draft, "00:11.90");
    assert_eq!(
        dialog.end_error,
        Some("End must be later than start".to_string())
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn create_region_start_blur_resets_invalid_draft_like_recovered_babel() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);
    let created_segment_id = state
        .create_region_dialog
        .as_ref()
        .unwrap()
        .created_segment_id
        .as_ref()
        .unwrap()
        .clone();
    let count_after_open = state.workflow.as_ref().unwrap().project().segments.len();
    assert_eq!(count_after_open, original_count + 1);

    let _ = update(
        &mut state,
        Message::CreateRegionStartChanged("00:12.90".to_string()),
    );
    let ok = state
        .create_region_dialog
        .as_mut()
        .unwrap()
        .blur_time_field(CreateRegionTimeField::Start);

    assert!(!ok);
    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), count_after_open);
    let created = project.segment(&created_segment_id).unwrap();
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.end_seconds, 12.8);
    assert_eq!(dialog.start_draft, "00:12.00");
    assert_eq!(
        dialog.start_error,
        Some("Start must be earlier than end".to_string())
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn create_region_captured_escape_resets_active_time_draft_without_closing_dialog() {
    let mut state = stable_sample_state();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(
        &mut state,
        Message::CreateRegionStartChanged("00:12.90".to_string()),
    );
    let _ = update(&mut state, Message::CommitCreateRegionStart);
    let _ = update(&mut state, Message::CreateRegionInputEscapePressed);

    let dialog = state.create_region_dialog.as_ref().unwrap();
    assert_eq!(dialog.start_seconds, 12.0);
    assert_eq!(dialog.start_draft, "00:12.00");
    assert_eq!(dialog.start_error, None);
}

#[test]
fn create_region_outside_close_respects_recovered_babel_select_guards() {
    let mut state = stable_sample_state();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(&mut state, Message::CreateRegionSelectOpenChanged(true));
    let _ = update(
        &mut state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::plain_outside()),
    );

    assert!(state.create_region_dialog.is_some());
    assert!(state.create_region_dialog.as_ref().unwrap().select_open);

    let _ = update(&mut state, Message::CreateRegionSelectOpenChanged(false));
    let _ = update(
        &mut state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::listbox()),
    );
    let _ = update(
        &mut state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::radix_portal()),
    );
    let _ = update(
        &mut state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::radix_select_content()),
    );

    assert!(state.create_region_dialog.is_some());
    assert!(!state.create_region_dialog.as_ref().unwrap().select_open);

    let _ = update(
        &mut state,
        Message::CreateRegionInteractOutside(OutsideInteractionContext::plain_outside()),
    );

    assert!(state.create_region_dialog.is_none());
}

#[test]
fn attached_babel_add_region_submit_ignores_hidden_time_drafts() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);
    let created_segment_id = state
        .create_region_dialog
        .as_ref()
        .unwrap()
        .created_segment_id
        .as_ref()
        .unwrap()
        .clone();
    let count_after_open = state.workflow.as_ref().unwrap().project().segments.len();
    assert_eq!(count_after_open, original_count + 1);

    let _ = update(
        &mut state,
        Message::CreateRegionStartChanged("00:12.90".to_string()),
    );
    let _ = update(&mut state, Message::SubmitCreateRegion);

    let project = state.workflow.as_ref().unwrap().project();
    assert_eq!(project.segments.len(), count_after_open);
    let created = project.segment(&created_segment_id).unwrap();
    assert_eq!(created.start_seconds, 12.0);
    assert_eq!(created.end_seconds, 12.8);
    assert!(state.create_region_dialog.is_none());
    assert!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref()
            != Some("")
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn babel_day_to_day_mismatch_attached_add_region_uses_waveform_popup_overlay() {
    let components_source = include_str!("components/mod.rs");
    let dialog_source = include_str!("components/create_region_dialog.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let popup_overlay = waveform_source
        .split("fn view_attached_create_region_popup")
        .nth(1)
        .and_then(|source| source.split("fn view_hover_cursor").next())
        .expect("attached transcription Add Region popup should render in waveform layers");

    assert!(dialog_source.contains("pub(super) fn is_attached_transcription_card"));
    assert!(components_source.contains("layout = layout.push(waveform_card);"));
    assert!(!components_source.contains("layout = layout.push(create_region_dialog::view_create_region_dialog(\n            project, editor, dialog,\n        ));"));
    assert!(timeline_source.contains("AttachedCreateRegionPopup"));
    assert!(timeline_source.contains("attached_create_region_popup"));
    assert!(waveform_source.contains("view_attached_create_region_popup"));
    assert!(waveform_source.contains("layers.push(view_attached_create_region_popup"));
    assert!(popup_overlay.contains("opaque("));
    assert!(popup_overlay.contains("text_input(\"Enter text for this region...\""));
    assert!(popup_overlay.contains("Message::CreateRegionTextChanged"));
    assert!(popup_overlay.contains("Message::SubmitCreateRegion"));
    assert!(!popup_overlay.contains("\"Speaker\""));
    assert!(!popup_overlay.contains("\"Start Time"));
    assert!(!popup_overlay.contains("\"End Time"));
    assert!(!popup_overlay.contains("view_annotation_fields"));
}

#[test]
fn separate_waveform_drag_gestures_are_separate_undo_steps() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;

    drag_region_body(&mut state, grab_time, grab_time + 0.4);
    let after_first_drag = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();

    drag_region_body(&mut state, grab_time + 0.4, grab_time + 0.7);
    let after_second_drag = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();

    assert_ne!(
        after_first_drag.start_seconds,
        after_second_drag.start_seconds
    );

    let _ = update(&mut state, Message::Undo);

    let restored = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(restored.start_seconds, after_first_drag.start_seconds);
    assert_eq!(restored.end_seconds, after_first_drag.end_seconds);
}

#[test]
fn repeated_hover_updates_within_one_waveform_drag_are_one_undo_step() {
    let mut state = stable_sample_state();
    let original = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap()
        .clone();
    let grab_time = (original.start_seconds + original.end_seconds) / 2.0;

    let _ = update(
        &mut state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time,
            },
        ),
    );
    let _ = update(
        &mut state,
        Message::WaveformHitPressed("track-1".to_string()),
    );
    for move_time in [grab_time + 0.2, grab_time + 0.4, grab_time + 0.6] {
        let _ = update(
            &mut state,
            Message::WaveformHitHovered(
                "track-1".to_string(),
                WaveformHit {
                    target: WaveformHitTarget::RegionBody,
                    region_id: Some("annotation-1".to_string()),
                    time_seconds: move_time,
                },
            ),
        );
    }
    let _ = update(
        &mut state,
        Message::WaveformHitReleased("track-1".to_string()),
    );

    let _ = update(&mut state, Message::Undo);

    let restored = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(restored.start_seconds, original.start_seconds);
    assert_eq!(restored.end_seconds, original.end_seconds);
}

#[test]
fn create_segment_before_uses_full_recovered_previous_lane_gap_and_selects_new_segment() {
    let mut state = stable_sample_state();

    state.create_segment_before("annotation-3");

    let project = state.workflow.as_ref().unwrap().project();
    let created = project.segment("annotation-3__studio_1").unwrap();
    assert_eq!(created.lane_id, "track-speaker-1");
    assert_eq!(created.speaker_id, "speaker-1");
    assert!((created.start_seconds - 3.4).abs() < f64::EPSILON * 8.0);
    assert_eq!(created.end_seconds, 5.4);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-3__studio_1")
    );
}

#[test]
fn track_change_updates_segment_lane_and_speaker() {
    let mut state = stable_sample_state();

    state.change_segment_track("annotation-1", "track-speaker-2");

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.lane_id, "track-speaker-2");
    assert_eq!(segment.speaker_id, "speaker-2");
}

#[test]
fn track_cell_edit_state_matches_recovered_babel_open_change_and_select() {
    let mut state = stable_sample_state();

    state.begin_segment_track_edit("annotation-1");
    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(draft.track_edit.is_editing);
    assert!(draft.track_edit.select_open);
    assert_eq!(draft.track_edit.draft_lane_id, "track-speaker-1");
    assert!(draft.track_edit.should_focus);

    let _ = update(
        &mut state,
        Message::SegmentTrackSelectOpenChanged("annotation-1".to_string(), false),
    );
    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(!draft.track_edit.is_editing);
    assert!(!draft.track_edit.select_open);
    assert_eq!(draft.track_edit.draft_lane_id, "track-speaker-1");

    state.begin_segment_track_edit("annotation-1");
    let _ = update(
        &mut state,
        Message::SegmentTrackChanged("annotation-1".to_string(), "track-speaker-2".to_string()),
    );

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.lane_id, "track-speaker-2");
    assert_eq!(segment.speaker_id, "speaker-2");
    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(!draft.track_edit.is_editing);
    assert!(!draft.track_edit.select_open);
    assert_eq!(draft.track_edit.draft_lane_id, "track-speaker-2");
}

#[test]
fn track_cell_double_click_begins_recovered_babel_edit_without_single_click_editing() {
    let mut state = stable_sample_state();
    let first_click = Instant::now();

    state.press_segment_track_cell_at("annotation-2", first_click);

    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-2")
    );
    let draft = state.segment_drafts.get("annotation-2").unwrap();
    assert!(!draft.track_edit.is_editing);
    assert!(!draft.track_edit.select_open);

    state.press_segment_track_cell_at("annotation-2", first_click + Duration::from_millis(120));

    let draft = state.segment_drafts.get("annotation-2").unwrap();
    assert!(draft.track_edit.is_editing);
    assert!(draft.track_edit.select_open);
    assert_eq!(draft.track_edit.draft_lane_id, "track-speaker-2");
    assert!(draft.track_edit.should_focus);
    assert!(state.last_track_cell_press.is_none());
}

#[test]
fn track_cell_double_click_window_expires_like_browser_edit_trigger() {
    let mut state = stable_sample_state();
    let first_click = Instant::now();

    state.press_segment_track_cell_at("annotation-1", first_click);
    state.press_segment_track_cell_at("annotation-1", first_click + Duration::from_millis(700));

    let draft = state.segment_drafts.get("annotation-1").unwrap();
    assert!(!draft.track_edit.is_editing);
    assert!(!draft.track_edit.select_open);
}

#[test]
fn annotation_type_change_updates_state_and_babel_export() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };

    state.set_segment_annotation_type("annotation-2", "non-transcription");

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-2")
        .unwrap();
    assert_eq!(
        segment.metadata.get("type"),
        Some(&serde_json::json!("non-transcription"))
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);

    state.export_babel_json();

    let exported = std::fs::read_to_string(export_path).unwrap();
    let package: BabelTaskPackage = serde_json::from_str(&exported).unwrap();
    let annotation = package
        .annotations
        .iter()
        .find(|annotation| annotation.annotation_id == "annotation-2")
        .unwrap();
    assert_eq!(
        annotation.extra.get("type"),
        Some(&serde_json::json!("non-transcription"))
    );
}

#[test]
fn edit_annotation_dialog_hydrates_recovered_babel_metadata() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = project.segment_mut("annotation-2").unwrap();
    segment.text = "background-noise / rustle".to_string();
    segment
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    segment
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    segment.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation edit fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-2".to_string()),
    );

    let dialog = state.edit_annotation_dialog.as_ref().unwrap();
    assert_eq!(dialog.segment_id, "annotation-2");
    assert_eq!(dialog.title(), "Edit Annotation");
    assert_eq!(dialog.primary_selection, "background-noise");
    assert_eq!(dialog.secondary_selection, "rustle");
    assert_eq!(dialog.custom_description, "");
    assert_eq!(dialog.intensity, Some(3));
    assert!(dialog.error.is_none());
    assert!(dialog.is_save_enabled());
}

#[test]
fn edit_annotation_dialog_hydrates_recovered_babel_other_secondary_description() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = project.segment_mut("annotation-2").unwrap();
    segment.text = "background-noise / chair scrape".to_string();
    segment
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    segment
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    segment.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "chair scrape"
        }),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation edit fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-2".to_string()),
    );

    let dialog = state.edit_annotation_dialog.as_ref().unwrap();
    assert_eq!(dialog.primary_selection, "background-noise");
    assert_eq!(dialog.secondary_selection, "other");
    assert_eq!(dialog.custom_description, "chair scrape");
    assert!(dialog.is_save_enabled());
}

#[test]
fn edit_annotation_dialog_requires_recovered_babel_secondary_and_description_rules() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = project.segment_mut("annotation-2").unwrap();
    segment.text = "background-noise / rustle".to_string();
    segment
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    segment
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    segment.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation edit fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationSecondaryCategoryChanged("other".to_string()),
    );
    let _ = update(&mut state, Message::SubmitEditAnnotation);

    let dialog = state.edit_annotation_dialog.as_ref().unwrap();
    assert_eq!(
        dialog.error.as_deref(),
        Some("Primary and secondary categories are required")
    );
    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-2")
        .unwrap();
    assert_eq!(
        segment.metadata.get("content"),
        Some(&serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
}

#[test]
fn edit_annotation_requires_explicit_single_secondary_choice_like_recovered_babel() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = project.segment_mut("annotation-2").unwrap();
    segment.text = "background-noise / rustle".to_string();
    segment
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    segment
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    segment.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation edit fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationPrimaryCategoryChanged("background-speech".to_string()),
    );

    let dialog = state.edit_annotation_dialog.as_ref().unwrap();
    assert_eq!(dialog.primary_selection, "background-speech");
    assert_eq!(dialog.secondary_selection, "");
    assert!(!dialog.is_save_enabled());

    let _ = update(&mut state, Message::SubmitEditAnnotation);

    let dialog = state.edit_annotation_dialog.as_ref().unwrap();
    assert_eq!(
        dialog.error.as_deref(),
        Some("Primary and secondary categories are required")
    );
    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-2")
        .unwrap();
    assert_eq!(
        segment.metadata.get("content"),
        Some(&serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }))
    );
}

#[test]
fn edit_annotation_outside_mouse_down_matches_recovered_babel_popover_guards() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = project.segment_mut("annotation-2").unwrap();
    segment.text = "background-noise / rustle".to_string();
    segment
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    segment
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    segment.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation edit fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-2".to_string()),
    );
    let _ = update(&mut state, Message::EditAnnotationSelectOpenChanged(true));
    let _ = update(
        &mut state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::plain_outside()),
    );

    assert!(state.edit_annotation_dialog.is_some());
    assert!(state.edit_annotation_dialog.as_ref().unwrap().select_open);

    let _ = update(&mut state, Message::EditAnnotationSelectOpenChanged(false));
    let _ = update(
        &mut state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::inside_surface()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::listbox()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::radix_portal()),
    );

    assert!(state.edit_annotation_dialog.is_some());

    let _ = update(
        &mut state,
        Message::EditAnnotationOutsideMouseDown(OutsideInteractionContext::plain_outside()),
    );

    assert!(state.edit_annotation_dialog.is_none());
}

#[test]
fn saving_edit_annotation_dialog_updates_babel_metadata_text_and_export() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };
    let mut project = state.project().unwrap().clone();
    project.id = BACKGROUND_NOISE_ANNOTATION_PROJECT_ID.to_string();
    let segment = project.segment_mut("annotation-2").unwrap();
    segment.text = "background-noise / rustle".to_string();
    segment
        .metadata
        .insert("type".to_string(), serde_json::json!("background_noise"));
    segment
        .metadata
        .insert("intensity".to_string(), serde_json::json!(3));
    segment.metadata.insert(
        "content".to_string(),
        serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "rustle"
        }),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation edit fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationSecondaryCategoryChanged("other".to_string()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationCustomDescriptionChanged("chair scrape".to_string()),
    );
    let _ = update(
        &mut state,
        Message::EditAnnotationIntensityChanged("4".to_string()),
    );
    let _ = update(&mut state, Message::SubmitEditAnnotation);

    assert!(state.edit_annotation_dialog.is_none());
    assert!(state.editor.as_ref().unwrap().is_dirty);
    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-2")
        .unwrap();
    assert_eq!(segment.text, "background-noise / chair scrape");
    assert_eq!(
        segment.metadata.get("intensity"),
        Some(&serde_json::json!(4))
    );
    assert_eq!(
        segment.metadata.get("content"),
        Some(&serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "chair scrape"
        }))
    );

    state.export_babel_json();
    let exported = std::fs::read_to_string(export_path).unwrap();
    let package: BabelTaskPackage = serde_json::from_str(&exported).unwrap();
    let annotation = package
        .annotations
        .iter()
        .find(|annotation| annotation.annotation_id == "annotation-2")
        .unwrap();
    assert_eq!(annotation.text, "background-noise / chair scrape");
    assert_eq!(
        annotation.extra.get("intensity"),
        Some(&serde_json::json!(4))
    );
    assert_eq!(
        annotation.extra.get("content"),
        Some(&serde_json::json!({
            "primaryLabel": "background-noise",
            "secondaryLabel": "chair scrape"
        }))
    );
}

#[test]
fn annotation_mutation_startup_smoke_exercises_runtime_command_paths_and_export() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };

    let report = run_annotation_mutation_smoke(&mut state);

    assert_eq!(
        report,
        AnnotationMutationSmokeReport {
            edit_text: "background-noise / chair scrape".to_string(),
            edit_intensity: Some(4),
            timed_range: (1.3, 2.3),
            track_lane_id: "track-speaker-2".to_string(),
            split_created_id: Some("ann-split__studio_1".to_string()),
            merge_removed: true,
            delete_removed: true,
            created_id: Some("track-speaker-2__studio_1".to_string()),
            saved: true,
            segment_count: 6,
        }
    );
    assert_eq!(
        report.runtime_trace_line(),
        "Transcription Studio annotation smoke: edit_text=background-noise / chair scrape edit_intensity=4 time=1.300-2.300s track=track-speaker-2 split=ann-split__studio_1 merge_removed=true delete_removed=true created=track-speaker-2__studio_1 saved=true segments=6"
    );

    let exported = std::fs::read_to_string(export_path).unwrap();
    let package: BabelTaskPackage = serde_json::from_str(&exported).unwrap();
    assert!(
        package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == "ann-edit"
                && annotation.text == "background-noise / chair scrape"
                && annotation.extra.get("intensity") == Some(&serde_json::json!(4)))
    );
    assert!(
        package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == "ann-time-track"
                && annotation.track_id == "track-speaker-2"
                && (annotation.start_seconds - 1.3).abs() < f64::EPSILON * 8.0
                && (annotation.end_seconds - 2.3).abs() < f64::EPSILON * 8.0)
    );
    assert!(
        package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == "ann-split__studio_1")
    );
    assert!(
        !package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == "ann-merge-b")
    );
    assert!(
        !package
            .annotations
            .iter()
            .any(|annotation| annotation.annotation_id == "ann-delete")
    );
    assert!(
        package
            .annotations
            .iter()
            .any(
                |annotation| annotation.annotation_id == "track-speaker-2__studio_1"
                    && annotation.text == "background-noise / annotation smoke create"
                    && annotation.extra.get("intensity") == Some(&serde_json::json!(2))
            )
    );
}

#[test]
fn lane_control_messages_update_babel_track_state() {
    let mut state = stable_sample_state();
    let zoom_fraction = (1000.0 - 7.5) / (1000.0 - 0.3);

    let _ = update(
        &mut state,
        Message::ToggleLaneCollapse("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::ToggleLaneMute("track-speaker-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: zoom_fraction,
            height: 100.0,
        },
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPressed("track-speaker-2".to_string()),
    );
    let _ = update(&mut state, Message::LaneVerticalZoomSliderReleased);

    let project = state.workflow.as_ref().unwrap().project();
    let lanes = state.editor.as_ref().unwrap().timeline_lanes(project);
    assert!(
        lanes
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-1")
            .unwrap()
            .is_collapsed
    );
    assert!(
        lanes
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-2")
            .unwrap()
            .is_muted
    );
    assert!(
        (lanes
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-2")
            .unwrap()
            .vertical_zoom
            - 7.5)
            .abs()
            < 0.001
    );
    assert_ne!(
        lanes
            .iter()
            .find(|lane| lane.lane_id == "track-speaker-2")
            .unwrap()
            .vertical_zoom,
        20.0
    );
}

#[test]
fn lane_vertical_zoom_accepts_recovered_waveform_scale_unlock_ceiling() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();

    state.with_editor_mut(|editor| editor.set_lane_vertical_zoom("track-speaker-2", 750.0));

    let lane = state
        .editor
        .as_ref()
        .unwrap()
        .timeline_lanes(&project)
        .into_iter()
        .find(|lane| lane.lane_id == "track-speaker-2")
        .unwrap();
    assert_eq!(lane.vertical_zoom, 750.0);
}

#[test]
fn adjacent_segment_navigation_skips_babel_collapsed_tracks() {
    let mut state = stable_sample_state();
    let _ = update(
        &mut state,
        Message::ToggleLaneCollapse("track-speaker-2".to_string()),
    );
    state.select_segment("annotation-1");

    let _ = update(&mut state, Message::NextSegment);

    let editor = state.editor.as_ref().unwrap();
    let project = state.workflow.as_ref().unwrap().project();
    let selected = project
        .segment(editor.selected_segment_id.as_deref().unwrap())
        .unwrap();
    assert_eq!(selected.lane_id, "track-speaker-1");
}

#[test]
fn adjacent_segment_navigation_uses_babel_boundary_seek_semantics() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");

    let _ = update(&mut state, Message::PreviousSegment);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, 0.0);
    assert_eq!(editor.selected_segment_id, None);

    let last_segment_id = {
        let project = state.workflow.as_ref().unwrap().project();
        let editor = state.editor.as_ref().unwrap();
        editor
            .transcript_rows(project)
            .last()
            .expect("sample should contain rows")
            .segment_id
            .clone()
    };
    state.select_segment(&last_segment_id);
    let project_duration =
        studio_app::TimeRange::for_project(state.workflow.as_ref().unwrap().project()).end_seconds;

    let _ = update(&mut state, Message::NextSegment);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, project_duration);
    assert_eq!(editor.selected_segment_id, None);
}

#[test]
fn playback_tick_advances_the_open_editor_transport() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-2");
    state.editor.as_mut().unwrap().set_speed(2.0);
    let start_tick = Instant::now();

    let _ = update(&mut state, Message::TransportTick(start_tick));
    let before_play = state.editor.as_ref().unwrap().playhead_seconds;

    let _ = update(&mut state, Message::TogglePlayback);
    let _ = update(
        &mut state,
        Message::TransportTick(start_tick + Duration::from_millis(16)),
    );

    let after_play = state.editor.as_ref().unwrap().playhead_seconds;
    assert_eq!(before_play, 3.5);
    assert!((after_play - 3.532).abs() < f64::EPSILON * 16.0);
}

#[test]
fn playback_tick_uses_elapsed_frame_time_for_smoother_native_playhead() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-2");
    let start_tick = Instant::now();

    let _ = update(&mut state, Message::TogglePlayback);
    let _ = update(&mut state, Message::TransportTick(start_tick));
    let first_playhead = state.editor.as_ref().unwrap().playhead_seconds;

    let _ = update(
        &mut state,
        Message::TransportTick(start_tick + Duration::from_millis(48)),
    );
    let second_playhead = state.editor.as_ref().unwrap().playhead_seconds;

    assert!((first_playhead - 3.516).abs() < f64::EPSILON * 16.0);
    assert!((second_playhead - 3.564).abs() < f64::EPSILON * 16.0);
    assert!(second_playhead - first_playhead < 0.05);
}

#[test]
fn transcript_row_render_cache_reuses_rows_across_playhead_ticks_inside_same_segment() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.select_segment(&project, "annotation-1");
        editor.toggle_playback();
    });

    let first_rows = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor)
    };
    let misses_after_first_render = state.transcript_rows_cache_miss_count();

    let _ = update(&mut state, Message::TransportTick(Instant::now()));

    let second_rows = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor)
    };

    assert_eq!(first_rows, second_rows);
    assert_eq!(
        state.transcript_rows_cache_miss_count(),
        misses_after_first_render
    );
}

#[test]
fn transcript_row_render_cache_trace_marks_playback_frames_as_hits() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.select_segment(&project, "annotation-1");
        editor.toggle_playback();
    });

    let first_row_count = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor).len()
    };
    let first_trace = state
        .transcript_rows_cache_last_trace()
        .expect("first render should record a cache trace");
    assert_eq!(
        first_trace.event,
        super::state::TranscriptRowsCacheTraceEvent::Miss
    );
    assert_eq!(first_trace.row_count, first_row_count);
    assert!(first_trace.playing);

    let _ = update(&mut state, Message::TransportTick(Instant::now()));

    let second_row_count = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor).len()
    };
    let second_trace = state
        .transcript_rows_cache_last_trace()
        .expect("playback render should record a cache trace");

    assert_eq!(
        second_trace.event,
        super::state::TranscriptRowsCacheTraceEvent::Hit
    );
    assert_eq!(second_trace.row_count, second_row_count);
    assert!(second_trace.playing);
}

#[test]
fn transcript_row_render_cache_recomputes_when_table_filter_changes() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();

    let initial_count = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor).len()
    };
    let misses_after_first_render = state.transcript_rows_cache_miss_count();

    let _ = update(
        &mut state,
        Message::TrackFilterChanged(studio_app::TrackFilter::lane(
            "track-speaker-2",
            "Speaker 2",
        )),
    );

    let filtered_count = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor).len()
    };
    let misses_after_filter_render = state.transcript_rows_cache_miss_count();
    let cached_filtered_count = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor).len()
    };

    assert!(filtered_count < initial_count);
    assert_eq!(filtered_count, cached_filtered_count);
    assert_eq!(misses_after_filter_render, misses_after_first_render + 1);
    assert_eq!(
        state.transcript_rows_cache_miss_count(),
        misses_after_filter_render
    );
}

#[test]
fn helper_timeline_defaults_persist_and_hydrate_on_reload() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ZoomChanged(2.0));
    let _ = update(&mut state, Message::SpeedChanged("1.5x".to_string()));
    let _ = update(
        &mut state,
        Message::TrackFilterChanged(studio_app::TrackFilter::lane(
            "track-speaker-2",
            "Speaker 2",
        )),
    );
    let _ = update(
        &mut state,
        Message::ToggleLaneCollapse("track-speaker-2".to_string()),
    );
    let _ = update(
        &mut state,
        Message::ToggleLaneMute("track-speaker-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPointerMoved {
            lane_id: "track-speaker-2".to_string(),
            fraction: 0.25,
            height: 100.0,
        },
    );
    let _ = update(
        &mut state,
        Message::LaneVerticalZoomSliderPressed("track-speaker-2".to_string()),
    );
    let _ = update(&mut state, Message::LaneVerticalZoomSliderReleased);

    let expected_vertical_zoom = *state
        .editor
        .as_ref()
        .and_then(|editor| editor.lane_vertical_zoom.get("track-speaker-2"))
        .expect("vertical zoom should be set before reload");
    let project = state.project().expect("project should be loaded").clone();
    let mut reloaded = stable_sample_state();
    reloaded.replace_workflow(
        StudioWorkflow::from_project(project),
        "reloaded helper timeline defaults".to_string(),
    );

    let editor = reloaded.editor.as_ref().expect("editor should reload");
    assert!((editor.zoom - 2.0).abs() < f32::EPSILON);
    assert!((editor.speed - 1.5).abs() < f32::EPSILON);
    assert_eq!(
        editor.track_filter,
        studio_app::TrackFilter::lane("track-speaker-2", "Speaker 2")
    );
    assert!(editor.collapsed_lane_ids.contains("track-speaker-2"));
    assert!(editor.muted_lane_ids.contains("track-speaker-1"));
    assert_eq!(
        editor.lane_vertical_zoom.get("track-speaker-2").copied(),
        Some(expected_vertical_zoom)
    );
}

#[test]
fn transcript_row_render_cache_invalidates_after_project_structure_mutation() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    {
        let editor = state.editor.as_ref().unwrap();
        assert!(
            state
                .transcript_rows_for_render(&project, editor)
                .iter()
                .any(|row| row.segment_id == "annotation-1")
        );
    }
    let misses_after_first_render = state.transcript_rows_cache_miss_count();

    let _ = update(
        &mut state,
        Message::DeleteSegment("annotation-1".to_string()),
    );

    let project = state.project().unwrap().clone();
    let rows_after_delete = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_for_render(&project, editor)
    };

    assert!(
        !rows_after_delete
            .iter()
            .any(|row| row.segment_id == "annotation-1")
    );
    assert_eq!(
        state.transcript_rows_cache_miss_count(),
        misses_after_first_render + 1
    );
}

#[test]
fn large_project_playback_reuses_transcript_rows_and_lane_peak_resolutions() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.set_lane_waveform(
            "track-2",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.toggle_playback();
    });

    let project = state.project().unwrap().clone();
    let (first_rows, first_lanes, first_overview, first_transport, first_submit) = {
        let editor = state.editor.as_ref().unwrap();
        let rows = state.transcript_rows_for_render(&project, editor);
        let lanes = state.timeline_lanes_snapshot_for_render(&project, editor);
        let first_overview = state.overview_waveform_snapshot_for_render(&project, editor);
        let first_transport = state.transport_strip_snapshot_for_render(&project, editor);
        let first_submit = state.submit_control_snapshot_for_render(&project, editor);
        assert_eq!(rows.len(), CACHE_PROJECT_SEGMENT_COUNT);
        assert_eq!(lanes.len(), 2);
        (rows, lanes, first_overview, first_transport, first_submit)
    };
    let misses_after_first_render = state.transcript_rows_cache_miss_count();
    let timeline_misses_after_first_render = state.timeline_lanes_cache_miss_count();
    let overview_misses_after_first_render = state.overview_waveform_cache_miss_count();
    let transport_misses_after_first_render = state.transport_strip_cache_miss_count();
    let submit_misses_after_first_render = state.submit_control_cache_miss_count();

    let start_tick = Instant::now();
    for frame in 0..20 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let editor = state.editor.as_ref().unwrap();
        assert_eq!(
            state.transcript_rows_for_render(&project, editor),
            first_rows
        );
        let overview = state.overview_waveform_snapshot_for_render(&project, editor);
        let transport = state.transport_strip_snapshot_for_render(&project, editor);
        let submit = state.submit_control_snapshot_for_render(&project, editor);
        assert!(
            std::sync::Arc::ptr_eq(&first_transport, &transport),
            "playback ticks should reuse the shared transport control snapshot instead of cloning the model"
        );
        assert!(
            std::sync::Arc::ptr_eq(&first_submit, &submit),
            "playback ticks should reuse the shared submit control snapshot instead of cloning the model"
        );
        assert!(
            std::sync::Arc::ptr_eq(&first_overview.overview, &overview.overview),
            "playback ticks should reuse the shared overview waveform snapshot instead of cloning the model"
        );
        let lanes = state.timeline_lanes_snapshot_for_render(&project, editor);
        assert_eq!(lanes.len(), 2);
        assert!(
            std::sync::Arc::ptr_eq(&first_lanes, &lanes),
            "playback ticks should reuse the shared timeline lane snapshot instead of cloning lane views"
        );
    }

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(
        state.transcript_rows_cache_miss_count(),
        misses_after_first_render
    );
    assert_eq!(
        state.timeline_lanes_cache_miss_count(),
        timeline_misses_after_first_render
    );
    assert_eq!(
        state.overview_waveform_cache_miss_count(),
        overview_misses_after_first_render
    );
    assert_eq!(
        state.transport_strip_cache_miss_count(),
        transport_misses_after_first_render
    );
    assert_eq!(
        state.submit_control_cache_miss_count(),
        submit_misses_after_first_render
    );
    for lane_id in ["track-1", "track-2"] {
        let waveform = editor
            .lane_waveforms
            .get(lane_id)
            .expect("large project should have decoded lane waveform");
        assert_eq!(
            waveform.cached_peak_resolution_count(),
            2,
            "moving playback viewport should reuse the lane's decoded peak cache for {lane_id}"
        );
    }
}

#[test]
fn transport_tick_advances_playback_without_cloning_project() {
    let source = include_str!("state.rs");
    let advance_transport_tick = source
        .split("pub(super) fn advance_transport_tick")
        .nth(1)
        .expect("state should define the transport tick hot path")
        .split("fn advance_large_project_live_pan_smoke")
        .next()
        .expect("transport tick should appear before live-pan smoke helper");

    assert!(!advance_transport_tick.contains("self.project().cloned()"));
    assert!(advance_transport_tick.contains("workflow.project()"));
    assert!(advance_transport_tick.contains("self.editor.as_mut()"));
    assert!(advance_transport_tick.contains("timeline_project_bounds_for_render(project)"));
    assert!(
        advance_transport_tick
            .contains("editor.advance_transport_with_bounds(project, bounds, elapsed_seconds)")
    );
}

#[test]
fn transport_tick_reuses_cached_project_bounds_while_playing() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(cache_sized_two_lane_project()),
        "Loaded large transport-bounds project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 30.0;
        editor.set_zoom(&project, 1.0);
        editor.toggle_playback();
    });

    let start_tick = Instant::now();
    for frame in 0..20 {
        state.advance_transport_tick(start_tick + Duration::from_millis(16 * frame));
    }

    assert_eq!(
        state.timeline_project_bounds_cache_miss_count(),
        1,
        "transport ticks should reuse cached project bounds instead of scanning every segment per frame"
    );
}

#[test]
fn frame_cadence_probe_reports_real_window_frame_intervals() {
    let start = Instant::now();
    let mut probe = super::state::FrameCadenceProbe::new(4);

    assert_eq!(probe.record(start), None);
    assert_eq!(probe.record(start + Duration::from_millis(16)), None);
    assert_eq!(probe.record(start + Duration::from_millis(33)), None);
    let report = probe
        .record(start + Duration::from_millis(50))
        .expect("four recorded window frames should emit one cadence report");

    assert_eq!(report.frames, 4);
    assert_eq!(report.intervals, 3);
    assert_eq!(report.elapsed, Duration::from_millis(50));
    assert_eq!(report.total_interval, Duration::from_millis(50));
    assert_eq!(report.min_interval, Duration::from_millis(16));
    assert_eq!(report.max_interval, Duration::from_millis(17));
    assert_eq!(
        report.runtime_trace_line(),
        "Transcription Studio frame cadence: frames=4 intervals=3 elapsed_ms=50.000 avg_ms=16.667 min_ms=16.000 max_ms=17.000"
    );
    assert_eq!(probe.record(start + Duration::from_millis(66)), None);
}

#[test]
fn transport_tick_update_trace_line_reports_playback_start_boundary() {
    assert_eq!(
        super::state::transport_tick_update_runtime_trace_line(
            Duration::from_micros(2750),
            true,
            true,
            super::state::TransportTickUpdatePhaseTimings {
                advance_transport: Duration::from_micros(1200),
                live_pan_smoke: Duration::from_micros(100),
                live_transcript_scroll_smoke: Duration::from_micros(150),
                startup_playback_smoke: Duration::from_micros(900),
                cadence_probe: Duration::from_micros(50),
            },
        ),
        "Transcription Studio transport tick update: elapsed_ms=2.750 playing=true started_playback=true advance_ms=1.200 live_pan_ms=0.100 live_scroll_ms=0.150 startup_ms=0.900 cadence_ms=0.050"
    );
}

#[test]
fn runtime_transport_tick_update_trace_is_env_gated() {
    let state_source = include_str!("state.rs");

    assert!(state_source.contains("STUDIO_TRACE_TRANSPORT_TICK_UPDATE"));
    assert!(state_source.contains("STUDIO_TRACE_TRANSPORT_TICK_UPDATE_SLOW_MS"));
    assert!(state_source.contains("transport_tick_update_trace_enabled()"));
    assert!(state_source.contains("transport_tick_update_should_log("));
    assert!(state_source.contains("struct TransportTickUpdatePhaseTimings"));
    assert!(state_source.contains("phase_timings.advance_transport"));
    assert!(state_source.contains("phase_timings.startup_playback_smoke"));
    assert!(state_source.contains("transport_tick_update_runtime_trace_line("));
    assert!(
        state_source.contains("let started_playback = self.advance_startup_playback_smoke_delay()")
    );
}

#[test]
fn runtime_frame_cadence_slow_interval_trace_is_env_gated() {
    let state_source = include_str!("state.rs");

    assert!(state_source.contains("STUDIO_TRACE_FRAME_CADENCE_SLOW_MS"));
    assert!(state_source.contains("slow_interval_threshold"));
    assert!(state_source.contains("with_slow_interval_threshold("));
    assert!(state_source.contains("frame_cadence_slow_interval_runtime_trace_line("));
    assert!(state_source.contains("Transcription Studio frame interval:"));
}

#[test]
fn runtime_frame_cadence_trace_is_env_gated_and_large_smoke_can_keep_playing() {
    let state_source = include_str!("state.rs");
    let smoke_source = include_str!("large_project_smoke.rs");
    let hotkey_source = include_str!("hotkeys.rs");

    assert!(state_source.contains("STUDIO_TRACE_FRAME_CADENCE"));
    assert!(state_source.contains("STUDIO_TRACE_FRAME_CADENCE_FRAMES"));
    assert!(state_source.contains("frame_cadence_probe_from_env()"));
    assert!(state_source.contains("self.frame_cadence_probe.as_mut()"));
    assert!(state_source.contains("report.runtime_trace_line()"));
    assert!(smoke_source.contains("STUDIO_SMOKE_LARGE_PROJECT_KEEP_PLAYING"));
    assert!(smoke_source.contains("large_project_keep_playing_enabled()"));
    assert!(smoke_source.contains("if !large_project_keep_playing_enabled()"));
    assert!(hotkey_source.contains("STUDIO_SMOKE_RENDERER_SCREENSHOT"));
    assert!(hotkey_source.contains("renderer_smoke_screenshot_enabled()"));
    assert!(
        hotkey_source.contains("&& renderer_smoke_screenshot_enabled()"),
        "performance cadence probes must be able to disable smoke screenshots so the screenshot task does not create a fake playback hitch"
    );
}

#[test]
fn large_project_runtime_can_probe_live_pan_cadence_without_user_visible_changes() {
    let state_source = include_str!("state.rs");
    let smoke_source = include_str!("large_project_smoke.rs");

    assert!(smoke_source.contains("STUDIO_SMOKE_LARGE_PROJECT_LIVE_PAN"));
    assert!(smoke_source.contains("large_project_live_pan_enabled()"));
    assert!(smoke_source.contains("state.enable_large_project_live_pan_smoke()"));
    assert!(state_source.contains("advance_large_project_live_pan_smoke("));
    assert!(state_source.contains("large_project_live_pan_enabled()"));
    assert!(state_source.contains("large_project_live_pan_active"));
    assert!(state_source.contains("self.with_project_and_editor_mut(|project, editor|"));
    assert!(state_source.contains("editor.pan_viewport_by_scroll_delta(project,"));
    assert!(
        state_source.contains("self.large_project_live_pan_frame"),
        "live pan smoke should alternate real viewport movement during window-frame ticks"
    );
}

#[test]
fn large_project_live_pan_and_timeline_wheel_pan_borrow_project_without_frame_clone() {
    let state_source = include_str!("state.rs");
    let actions_source = include_str!("actions.rs");
    let live_pan_source = state_source
        .split("fn advance_large_project_live_pan_smoke")
        .nth(1)
        .and_then(|source| {
            source
                .split("pub(super) fn enable_large_project_live_pan_smoke")
                .next()
        })
        .expect("live pan helper should exist");
    let wheel_source = actions_source
        .split("Message::TimelineWheelScrolled")
        .nth(1)
        .and_then(|source| source.split("Message::SeekToTime").next())
        .expect("timeline wheel handler should exist");

    assert!(state_source.contains("fn with_project_and_editor_mut"));
    assert!(live_pan_source.contains("self.with_project_and_editor_mut"));
    assert!(
        !live_pan_source.contains("self.project().cloned()"),
        "smooth live pan must not clone the whole project every frame"
    );
    assert!(wheel_source.contains("state.with_project_and_editor_mut"));
    assert!(
        !wheel_source.contains("state.project().cloned()"),
        "real timeline wheel pan/zoom must borrow the project instead of cloning the task per wheel event"
    );
}

#[test]
fn large_project_runtime_can_probe_live_transcript_scroll_cadence_without_user_visible_changes() {
    let state_source = include_str!("state.rs");
    let smoke_source = include_str!("large_project_smoke.rs");

    assert!(smoke_source.contains("STUDIO_SMOKE_LARGE_PROJECT_LIVE_TRANSCRIPT_SCROLL"));
    assert!(smoke_source.contains("large_project_live_transcript_scroll_enabled()"));
    assert!(smoke_source.contains("state.enable_large_project_live_transcript_scroll_smoke()"));
    assert!(state_source.contains("advance_large_project_live_transcript_scroll_smoke("));
    assert!(state_source.contains("large_project_live_transcript_scroll_enabled()"));
    assert!(state_source.contains("large_project_live_transcript_scroll_active"));
    assert!(state_source.contains("self.update_transcript_scroll_viewport("));
    assert!(
        state_source.contains("self.large_project_live_transcript_scroll_frame"),
        "live transcript scroll smoke should move the real virtualized table viewport during window-frame ticks"
    );
}

#[test]
fn large_project_playback_reuses_shared_transcript_row_snapshot() {
    let project = cache_sized_two_lane_project();
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.toggle_playback();
    });

    let project = state.project().unwrap().clone();
    let first_rows = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_snapshot_for_render(&project, editor)
    };
    let misses_after_first_render = state.transcript_rows_cache_miss_count();
    assert_eq!(first_rows.len(), CACHE_PROJECT_SEGMENT_COUNT);

    let start_tick = Instant::now();
    for frame in 0..20 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let editor = state.editor.as_ref().unwrap();
        let next_rows = state.transcript_rows_snapshot_for_render(&project, editor);
        assert!(
            std::sync::Arc::ptr_eq(&first_rows, &next_rows),
            "playback ticks should reuse the immutable transcript row snapshot instead of cloning every row"
        );
    }

    assert_eq!(
        state.transcript_rows_cache_miss_count(),
        misses_after_first_render
    );
}

#[test]
fn large_project_playback_reuses_transcript_rows_across_selected_segment_changes() {
    let project = cache_sized_two_lane_project();
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );

    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 21.0;
        editor.select_segment(&project, "annotation-0001");
        editor.toggle_playback();
    });

    let first_rows = {
        let editor = state.editor.as_ref().unwrap();
        state.transcript_rows_snapshot_for_render(&project, editor)
    };
    let misses_after_first_render = state.transcript_rows_cache_miss_count();
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0001")
    );

    let start_tick = Instant::now();
    for frame in 1..80 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let editor = state.editor.as_ref().unwrap();
        let next_rows = state.transcript_rows_snapshot_for_render(&project, editor);
        assert!(
            std::sync::Arc::ptr_eq(&first_rows, &next_rows),
            "selected-row playback movement should not rebuild the full transcript row snapshot"
        );
    }

    assert_ne!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0001"),
        "test setup should cross at least one short large-project segment"
    );
    assert_eq!(
        state.transcript_rows_cache_miss_count(),
        misses_after_first_render
    );
}

#[test]
fn transport_and_submit_controls_use_shared_snapshots_across_playback_ticks() {
    let state_source = include_str!("state.rs");
    let transport_source = include_str!("components/transport_strip.rs");
    let shell_source = include_str!("components/mod.rs");
    let smoke_source = include_str!("large_project_smoke.rs");

    assert!(state_source.contains("strip: Arc<TransportStripView>"));
    assert!(state_source.contains("submit: Arc<SubmitControlView>"));
    assert!(state_source.contains("transport_strip_snapshot_for_render"));
    assert!(state_source.contains("submit_control_snapshot_for_render"));
    assert!(state_source.contains("return Arc::clone(&entry.strip);"));
    assert!(state_source.contains("return Arc::clone(&entry.submit);"));
    assert!(!state_source.contains("return entry.strip.clone();"));
    assert!(!state_source.contains("return entry.submit.clone();"));
    assert!(
        transport_source.contains("state.transport_strip_snapshot_for_render(project, editor)")
    );
    assert!(transport_source.contains("state.submit_control_snapshot_for_render(project, editor)"));
    assert!(shell_source.contains("state.submit_control_snapshot_for_render(project, editor)"));
    assert!(smoke_source.contains("stable_transport"));
    assert!(smoke_source.contains("stable_submit"));
    assert!(smoke_source.contains("Arc::ptr_eq(&first_transport, &transport)"));
    assert!(smoke_source.contains("Arc::ptr_eq(&first_submit, &submit)"));
}

#[test]
fn transport_strip_view_is_lazy_cached_across_playback_ticks() {
    let transport_source = include_str!("components/transport_strip.rs");

    assert!(transport_source.contains("lazy,"));
    assert!(transport_source.contains("struct TransportStripDependency"));
    assert!(transport_source.contains("strip: Arc<TransportStripView>"));
    assert!(transport_source.contains("submit: Arc<SubmitControlView>"));
    assert!(transport_source.contains("impl std::hash::Hash for TransportStripDependency"));
    assert!(transport_source.contains("fn view_lazy_transport_strip("));
    assert!(transport_source.contains("lazy(dependency, view_transport_strip_content)"));
    assert!(transport_source.contains("Arc::as_ptr(&self.strip)"));
    assert!(transport_source.contains("Arc::as_ptr(&self.submit)"));
    assert!(transport_source.contains("drop_claim_is_pending: state.drop_claim_is_pending"));
    assert!(
        transport_source
            .contains("show_track_filter: editor.transcript_visible_cells(project).track")
    );
}

#[test]
fn large_project_playback_reuses_transcript_layout_metrics() {
    let project = cache_sized_two_lane_project();
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.toggle_playback();
    });

    let project = state.project().unwrap().clone();
    let (first_rows, first_layout) = {
        let editor = state.editor.as_ref().unwrap();
        let rows = state.transcript_rows_snapshot_for_render(&project, editor);
        let layout = state.transcript_layout_for_render(&rows, editor);
        (rows, layout)
    };
    let misses_after_first_render = state.transcript_layout_cache_miss_count();
    assert!(first_layout.virtual_window.rendered_len() <= 28);

    let start_tick = Instant::now();
    for frame in 0..20 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let editor = state.editor.as_ref().unwrap();
        let next_rows = state.transcript_rows_snapshot_for_render(&project, editor);
        let next_layout = state.transcript_layout_for_render(&next_rows, editor);
        assert!(std::sync::Arc::ptr_eq(&first_rows, &next_rows));
        assert!(
            std::sync::Arc::ptr_eq(&first_layout.row_heights, &next_layout.row_heights),
            "playback ticks should reuse transcript row-height metrics instead of scanning every row"
        );
        assert_eq!(first_layout.virtual_window, next_layout.virtual_window);
    }

    assert_eq!(
        state.transcript_layout_cache_miss_count(),
        misses_after_first_render
    );
}

#[test]
fn large_project_transcript_scroll_reuses_transcript_layout_metrics() {
    let project = cache_sized_two_lane_project();
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );
    state.transcript_scroll_viewport_height = 360.0;

    let project = state.project().unwrap().clone();
    let (first_rows, first_layout) = {
        let editor = state.editor.as_ref().unwrap();
        let rows = state.transcript_rows_snapshot_for_render(&project, editor);
        let layout = state.transcript_layout_for_render(&rows, editor);
        (rows, layout)
    };
    let misses_after_first_render = state.transcript_layout_cache_miss_count();

    for scroll_offset in [120.0, 480.0, 960.0, 1440.0] {
        let _ = update(
            &mut state,
            Message::TranscriptScrolled {
                offset_y: scroll_offset,
                viewport_height: 360.0,
            },
        );
        let editor = state.editor.as_ref().unwrap();
        let next_rows = state.transcript_rows_snapshot_for_render(&project, editor);
        let next_layout = state.transcript_layout_for_render(&next_rows, editor);

        assert!(std::sync::Arc::ptr_eq(&first_rows, &next_rows));
        assert!(
            std::sync::Arc::ptr_eq(&first_layout.row_heights, &next_layout.row_heights),
            "transcript scroll should reuse row-height metrics and update only the virtual window"
        );
        assert_ne!(first_layout.virtual_window, next_layout.virtual_window);
    }

    assert_eq!(
        state.transcript_layout_cache_miss_count(),
        misses_after_first_render
    );
}

#[test]
fn timeline_lane_render_cache_moves_playhead_without_rebuilding_static_lanes() {
    let project = large_two_lane_project(24);
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.playhead_seconds = 0.25;
    });

    let project = state.project().unwrap().clone();
    let first_lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let first_misses = state.timeline_lanes_cache_miss_count();
    let first_lane = first_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("track-1 should render");
    let first_model = first_lane
        .waveform_model
        .as_ref()
        .expect("audio-backed lane should render a waveform model");

    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 1.75;
    });
    let second_lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let second_lane = second_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("track-1 should still render");
    let second_model = second_lane
        .waveform_model
        .as_ref()
        .expect("cached audio-backed lane should still render a waveform model");

    assert_eq!(state.timeline_lanes_cache_miss_count(), first_misses);
    assert!(
        std::sync::Arc::ptr_eq(&first_lanes, &second_lanes),
        "moving playback should reuse the immutable timeline lane snapshot"
    );
    assert_eq!(second_lane.blocks, first_lane.blocks);
    assert!(std::sync::Arc::ptr_eq(
        &first_lane.blocks,
        &second_lane.blocks
    ));
    assert!(std::sync::Arc::ptr_eq(
        &first_model.channels[0].peaks,
        &second_model.channels[0].peaks
    ));
    assert_eq!(
        first_model.cursor.progress_percent,
        second_model.cursor.progress_percent
    );
}

#[test]
fn decoded_waveform_render_cache_reuses_recent_timeline_viewports_during_scroll_jitter() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
    });

    let project = state.project().unwrap().clone();
    let first_viewport = TimeRange {
        start_seconds: 20.0,
        end_seconds: 25.0,
    };
    let second_viewport = TimeRange {
        start_seconds: 20.35,
        end_seconds: 25.35,
    };
    for viewport in [
        first_viewport,
        second_viewport,
        first_viewport,
        second_viewport,
    ] {
        state.with_editor_mut(|editor| {
            editor.viewport = viewport;
        });
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            editor.timeline_lanes(&project)
        };
        let lane = lanes
            .iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 should render");
        assert!(
            lane.waveform_model.is_some(),
            "decoded waveform lane should render for viewport {viewport:?}"
        );
    }

    let stats = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .expect("track-1 should keep decoded audio")
        .render_cache_stats();
    assert_eq!(
        stats.misses, 1,
        "scroll jitter with unchanged zoom and visible segments should build the decoded waveform render model once"
    );
    assert_eq!(
        stats.hits, 3,
        "adjacent decoded waveform viewports should retarget the cached render model"
    );
}

#[test]
fn timeline_lane_snapshot_cache_keeps_only_current_viewport_during_scroll() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large timeline snapshot jitter project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
    });

    let project = state.project().unwrap().clone();
    let first_viewport = TimeRange {
        start_seconds: 20.0,
        end_seconds: 25.0,
    };
    let second_viewport = TimeRange {
        start_seconds: 20.35,
        end_seconds: 25.35,
    };

    let mut snapshots = Vec::new();
    for viewport in [
        first_viewport,
        second_viewport,
        first_viewport,
        second_viewport,
    ] {
        state.with_editor_mut(|editor| {
            editor.viewport = viewport;
        });
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            state.timeline_lanes_snapshot_for_render(&project, editor)
        };
        snapshots.push(lanes);
    }

    assert_eq!(
        state.timeline_lanes_cache_miss_count(),
        4,
        "scrolling should not retain old final viewport snapshots; only the current render frame is reusable"
    );
    assert!(
        !std::sync::Arc::ptr_eq(&snapshots[0], &snapshots[2]),
        "returning to a previous viewport should rebuild final geometry instead of resurrecting an old snapshot"
    );
    assert!(
        !std::sync::Arc::ptr_eq(&snapshots[1], &snapshots[3]),
        "the timeline lane cache should not behave like a viewport LRU"
    );
}

#[test]
fn timeline_scroll_across_new_segments_uses_prewarmed_segment_blocks() {
    let project = long_sparse_waveform_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded prewarmed segment block project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.viewport = TimeRange {
            start_seconds: 30.0,
            end_seconds: 35.0,
        };
    });

    let project = state.project().unwrap().clone();
    {
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }
    let initial_stats = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .segment_peak_cache_stats();

    for viewport in [
        TimeRange {
            start_seconds: 260.0,
            end_seconds: 265.0,
        },
        TimeRange {
            start_seconds: 620.0,
            end_seconds: 625.0,
        },
        TimeRange {
            start_seconds: 1_000.0,
            end_seconds: 1_005.0,
        },
    ] {
        state.with_editor_mut(|editor| {
            editor.viewport = viewport;
        });
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }

    let final_stats = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .segment_peak_cache_stats();
    assert_eq!(
        final_stats.misses, initial_stats.misses,
        "scrolling into new visible segments should hit prewarmed in-memory static blocks instead of computing segment peaks late"
    );
}

#[test]
fn decoded_waveform_far_scroll_reuses_full_chunk_strip() {
    let project = long_sparse_waveform_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded full chunk waveform scroll project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
    });

    let project = state.project().unwrap().clone();
    let mut initial_scroll_strip = None;
    let mut initial_strip_range = None;
    for viewport in [
        TimeRange {
            start_seconds: 30.0,
            end_seconds: 35.0,
        },
        TimeRange {
            start_seconds: 620.0,
            end_seconds: 625.0,
        },
        TimeRange {
            start_seconds: 1_000.0,
            end_seconds: 1_005.0,
        },
    ] {
        state.with_editor_mut(|editor| {
            editor.viewport = viewport;
        });
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            state.timeline_lanes_snapshot_for_render(&project, editor)
        };
        let lane = lanes
            .iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 should render");
        let channel = lane
            .waveform_model
            .as_ref()
            .expect("decoded track should have a waveform model")
            .channels
            .first()
            .expect("decoded model should have a channel");

        if let Some(initial_scroll_strip) = &initial_scroll_strip {
            assert!(
                std::sync::Arc::ptr_eq(initial_scroll_strip, &channel.scroll_strip_peaks),
                "far timeline scrolling should reuse one full in-memory waveform strip instead of replacing strip peaks"
            );
        } else {
            initial_scroll_strip = Some(std::sync::Arc::clone(&channel.scroll_strip_peaks));
            initial_strip_range = Some((
                channel.scroll_strip_start_seconds,
                channel.scroll_strip_end_seconds,
            ));
        }
    }

    let (strip_start_seconds, strip_end_seconds) =
        initial_strip_range.expect("first render should record strip range");
    assert_eq!(strip_start_seconds, 0.0);
    assert!(
        strip_end_seconds >= duration_seconds,
        "full in-memory strip should cover the whole project audio chunk"
    );
}

#[test]
fn decoded_waveform_far_scroll_keeps_full_lane_region_fill_blocks_in_memory() {
    let project = long_sparse_waveform_project();
    let track_1_segment_count = project
        .segments
        .iter()
        .filter(|segment| segment.lane_id == "track-1")
        .count();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded full lane region-fill block project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
    });

    let project = state.project().unwrap().clone();
    let mut initial_strip_blocks = None;
    for viewport in [
        TimeRange {
            start_seconds: 30.0,
            end_seconds: 35.0,
        },
        TimeRange {
            start_seconds: 620.0,
            end_seconds: 625.0,
        },
        TimeRange {
            start_seconds: 1_000.0,
            end_seconds: 1_005.0,
        },
    ] {
        state.with_editor_mut(|editor| {
            editor.viewport = viewport;
        });
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            state.timeline_lanes_snapshot_for_render(&project, editor)
        };
        let lane = lanes
            .iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 should render");
        assert!(
            lane.blocks.len() < lane.strip_blocks.len(),
            "visible hit-test blocks should stay viewport-sized while region-fill strip blocks cover the full lane"
        );
        assert_eq!(lane.strip_blocks.len(), track_1_segment_count);

        if let Some(initial_strip_blocks) = &initial_strip_blocks {
            assert!(
                std::sync::Arc::ptr_eq(initial_strip_blocks, &lane.strip_blocks),
                "far scrolling should reuse one full-lane region-fill block Arc instead of rebuilding viewport-shaped region geometry"
            );
        } else {
            initial_strip_blocks = Some(std::sync::Arc::clone(&lane.strip_blocks));
        }
    }
}

#[test]
fn timeline_pan_rebuilds_exact_geometry_without_rebuilding_audio_strip() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded subpixel wheel-pan project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.viewport = TimeRange {
            start_seconds: 20.0,
            end_seconds: 25.0,
        };
    });

    let project = state.project().unwrap().clone();
    {
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }
    let misses_after_first_render = state.timeline_lanes_cache_miss_count();
    let waveform_misses_after_first_render = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .render_cache_stats()
        .misses;

    for _ in 0..12 {
        let _ = update(
            &mut state,
            Message::TimelineWheelScrolled {
                delta_x: 0.002,
                delta_y: 0.0,
            },
        );
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }

    let editor = state.editor.as_ref().unwrap();
    assert!(
        editor.viewport.start_seconds > 20.0,
        "subpixel pan deltas should still move the real editor viewport"
    );
    assert!(
        state.timeline_lanes_cache_miss_count() > misses_after_first_render,
        "subpixel wheel/trackpad panning should rebuild exact lane geometry instead of reusing stale snapshots"
    );
    let waveform_misses_after_pan = editor
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .render_cache_stats()
        .misses;
    assert_eq!(
        waveform_misses_after_pan, waveform_misses_after_first_render,
        "exact geometry updates should retarget the cached decoded audio strip instead of rebuilding waveform data"
    );
}

#[test]
fn timeline_scroll_updates_block_geometry_on_every_subpixel_pan_step() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded smooth wheel-pan geometry project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.viewport = TimeRange {
            start_seconds: 20.0,
            end_seconds: 25.0,
        };
    });

    let project = state.project().unwrap().clone();
    let initial_left_percent = {
        let editor = state.editor.as_ref().unwrap();
        state
            .timeline_lanes_snapshot_for_render(&project, editor)
            .iter()
            .find(|lane| lane.lane_id == "track-1")
            .unwrap()
            .blocks
            .iter()
            .find(|block| block.segment_id == "annotation-0002")
            .unwrap()
            .left_percent
    };

    let _ = update(
        &mut state,
        Message::TimelineWheelScrolled {
            delta_x: 0.002,
            delta_y: 0.0,
        },
    );

    let panned_left_percent = {
        let editor = state.editor.as_ref().unwrap();
        assert!(
            editor.viewport.start_seconds > 20.0,
            "the real editor viewport should move for subpixel wheel deltas"
        );
        state
            .timeline_lanes_snapshot_for_render(&project, editor)
            .iter()
            .find(|lane| lane.lane_id == "track-1")
            .unwrap()
            .blocks
            .iter()
            .find(|block| block.segment_id == "annotation-0002")
            .unwrap()
            .left_percent
    };

    assert_ne!(
        panned_left_percent, initial_left_percent,
        "timeline block geometry should update every pan frame instead of sticking until a cache bucket changes"
    );
}

#[test]
fn decoded_waveform_scroll_reuses_audio_strip_when_visible_segments_change() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded smooth decoded waveform pan project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
    });

    let project = state.project().unwrap().clone();
    for viewport in [
        TimeRange {
            start_seconds: 20.0,
            end_seconds: 25.0,
        },
        TimeRange {
            start_seconds: 21.1,
            end_seconds: 26.1,
        },
        TimeRange {
            start_seconds: 22.2,
            end_seconds: 27.2,
        },
        TimeRange {
            start_seconds: 23.3,
            end_seconds: 28.3,
        },
    ] {
        state.with_editor_mut(|editor| {
            editor.viewport = viewport;
        });
        let editor = state.editor.as_ref().unwrap();
        let lane = editor
            .timeline_lanes(&project)
            .into_iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 lane should render while panning");
        assert!(
            lane.waveform_model.is_some(),
            "decoded waveform lane should render at viewport {viewport:?}"
        );
    }

    let stats = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .expect("track-1 should keep decoded audio")
        .render_cache_stats();
    assert_eq!(
        stats.misses, 1,
        "scrolling across segment boundaries should reuse the decoded audio strip instead of rebuilding waveform geometry"
    );
}

#[test]
fn overview_waveform_render_cache_moves_playhead_without_rebuilding_blocks() {
    let project = large_two_lane_project(24);
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.playhead_seconds = 0.25;
    });

    let project = state.project().unwrap().clone();
    let first_overview = {
        let editor = state.editor.as_ref().unwrap();
        state.overview_waveform_snapshot_for_render(&project, editor)
    };
    let first_misses = state.overview_waveform_cache_miss_count();
    let first_model = first_overview
        .overview
        .waveform_model
        .as_ref()
        .expect("audio-backed overview should render a waveform model");

    state.with_editor_mut(|editor| {
        editor.playhead_seconds = 1.75;
    });
    let second_overview = {
        let editor = state.editor.as_ref().unwrap();
        state.overview_waveform_snapshot_for_render(&project, editor)
    };
    let second_model = second_overview
        .overview
        .waveform_model
        .as_ref()
        .expect("cached audio-backed overview should still render a waveform model");

    assert_eq!(state.overview_waveform_cache_miss_count(), first_misses);
    assert!(
        std::sync::Arc::ptr_eq(&first_overview.overview, &second_overview.overview),
        "moving playback should reuse the immutable overview snapshot"
    );
    assert_eq!(
        second_overview.overview.blocks,
        first_overview.overview.blocks
    );
    assert!(std::sync::Arc::ptr_eq(
        &first_overview.overview.blocks,
        &second_overview.overview.blocks
    ));
    assert!(std::sync::Arc::ptr_eq(
        &first_model.channels[0].peaks,
        &second_model.channels[0].peaks
    ));
    assert_eq!(
        first_model.cursor.progress_percent,
        second_model.cursor.progress_percent
    );
}

#[test]
fn overview_waveform_static_blocks_survive_timeline_viewport_pans() {
    let project = cache_sized_two_lane_project();
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );

    let project = state.project().unwrap().clone();
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.seek_to_time(&project, 22.0);
        editor.set_zoom(&project, 1.0);
    });

    let first_overview = {
        let editor = state.editor.as_ref().unwrap();
        state.overview_waveform_snapshot_for_render(&project, editor)
    };
    let misses_after_first_render = state.overview_waveform_cache_miss_count();

    state.with_editor_mut(|editor| {
        editor.pan_viewport_by_scroll_delta(&project, 1.0);
    });
    let second_overview = {
        let editor = state.editor.as_ref().unwrap();
        state.overview_waveform_snapshot_for_render(&project, editor)
    };

    assert_eq!(
        state.overview_waveform_cache_miss_count(),
        misses_after_first_render,
        "viewport-only pans should not rebuild static overview waveform data"
    );
    assert_ne!(
        first_overview.overview.viewport_left_percent,
        second_overview.overview.viewport_left_percent
    );
    assert!(std::sync::Arc::ptr_eq(
        &first_overview.overview.blocks,
        &second_overview.overview.blocks
    ));
    let first_model = first_overview
        .overview
        .waveform_model
        .as_ref()
        .expect("decoded overview should render waveform peaks");
    let second_model = second_overview
        .overview
        .waveform_model
        .as_ref()
        .expect("decoded overview should reuse waveform peaks");
    assert!(std::sync::Arc::ptr_eq(
        &first_model.channels[0].peaks,
        &second_model.channels[0].peaks
    ));
}

#[test]
fn large_project_startup_smoke_reports_runtime_cache_and_waveform_reuse() {
    let mut state = stable_sample_state();

    let report = run_large_project_smoke(&mut state);

    assert!(report.decoded_waveforms);
    assert_eq!(report.rows, 1_200);
    assert!(report.rendered_rows <= 36);
    assert_eq!(report.lanes, 2);
    assert_eq!(report.frames, 4);
    assert_eq!(report.cache_misses, 1);
    assert_eq!(report.layout_cache_misses, 1);
    assert_eq!(report.timeline_cache_misses, 1);
    assert_eq!(report.overview_cache_misses, 1);
    assert_eq!(report.transport_cache_misses, 1);
    assert_eq!(report.submit_cache_misses, 1);
    assert_eq!(report.interaction_frames, 4);
    assert!(report.interaction_view_build_micros_total > 0);
    assert!(report.interaction_view_build_micros_max > 0);
    assert_eq!(
        report.interaction_timeline_cache_misses, report.interaction_frames,
        "timeline interaction should rebuild exact final geometry every moving frame instead of caching old viewport snapshots"
    );
    assert_eq!(
        report.interaction_overview_cache_misses, 0,
        "timeline panning should update overview viewport chrome without rebuilding static overview blocks"
    );
    assert!(
        report.interaction_waveform_cache_misses <= 4,
        "back-and-forth timeline interaction should keep decoded waveform data cached while final geometry rebuilds"
    );
    assert_eq!(
        report.peak_cache_counts,
        vec![("track-1".to_string(), 2), ("track-2".to_string(), 2)]
    );
    assert!(report.stable_rows);
    assert!(report.stable_lanes);
    assert!(report.stable_overview);
    assert!(report.stable_transport);
    assert!(report.stable_submit);
    assert!(report.runtime_trace_line().contains("rows=1200"));
    assert!(
        report
            .runtime_trace_line()
            .contains("decoded_waveforms=true")
    );
    assert!(report.runtime_trace_line().contains("rendered_rows="));
    assert!(
        report
            .runtime_trace_line()
            .contains("layout_cache_misses=1")
    );
    assert!(
        report
            .runtime_trace_line()
            .contains("timeline_cache_misses=1")
    );
    assert!(
        report
            .runtime_trace_line()
            .contains("overview_cache_misses=1")
    );
    assert!(
        report
            .runtime_trace_line()
            .contains("transport_cache_misses=1")
    );
    assert!(
        report
            .runtime_trace_line()
            .contains("submit_cache_misses=1")
    );
    assert!(report.runtime_trace_line().contains("stable_overview=true"));
    assert!(
        report
            .runtime_trace_line()
            .contains("stable_transport=true")
    );
    assert!(report.runtime_trace_line().contains("stable_submit=true"));
    assert!(report.runtime_trace_line().contains("interaction_frames=4"));
    assert!(
        report
            .runtime_trace_line()
            .contains("interaction_view_build_micros_total=")
    );
    assert!(
        report
            .runtime_trace_line()
            .contains("interaction_waveform_cache_hits=")
    );
}

#[test]
fn large_project_synthetic_waveform_smoke_reports_no_audio_overlay_path() {
    let mut state = stable_sample_state();

    let report = run_large_project_synthetic_waveform_smoke(&mut state);

    assert!(!report.decoded_waveforms);
    assert_eq!(report.rows, 1_200);
    assert!(report.rendered_rows <= 36);
    assert_eq!(report.lanes, 2);
    assert_eq!(report.frames, 4);
    assert_eq!(report.cache_misses, 1);
    assert_eq!(report.layout_cache_misses, 1);
    assert_eq!(report.timeline_cache_misses, 1);
    assert_eq!(report.overview_cache_misses, 1);
    assert_eq!(report.transport_cache_misses, 1);
    assert_eq!(report.submit_cache_misses, 1);
    assert!(report.peak_cache_counts.is_empty());
    assert!(report.stable_rows);
    assert!(report.stable_lanes);
    assert!(report.stable_overview);
    assert!(report.stable_transport);
    assert!(report.stable_submit);
    assert!(
        report
            .runtime_trace_line()
            .contains("decoded_waveforms=false")
    );
    assert!(report.runtime_trace_line().contains("peak_caches="));
    let trace = report.runtime_trace_line();

    assert!(trace.contains("view_build_frames=4"));
    assert!(trace.contains("view_build_micros_total="));
    assert!(trace.contains("view_build_micros_max="));
}

#[test]
fn transcript_virtual_window_bounds_large_project_render_work() {
    let row_heights = vec![30.0; 1_200];

    let window = super::transcript_virtualization::transcript_virtual_window(
        &row_heights,
        super::transcript_virtualization::TranscriptVirtualizationInput {
            scroll_offset_y: 900.0,
            viewport_height: 600.0,
            overscan_height: 600.0,
            render_all_threshold: 100,
            pinned_row_index: None,
        },
    );

    assert_eq!(window.start_index, 10);
    assert_eq!(window.end_index, 70);
    assert_eq!(window.rendered_len(), 60);
    assert_eq!(window.before_height, 300.0);
    assert_eq!(window.after_height, 33_900.0);
}

#[test]
fn transcript_virtual_window_virtualizes_demo_sized_projects() {
    let row_heights = vec![30.0; 120];

    let window = super::transcript_virtualization::transcript_virtual_window(
        &row_heights,
        super::transcript_virtualization::TranscriptVirtualizationInput {
            scroll_offset_y: 0.0,
            viewport_height: 600.0,
            overscan_height: 300.0,
            render_all_threshold:
                super::transcript_virtualization::TRANSCRIPT_VIRTUAL_RENDER_ALL_THRESHOLD,
            pinned_row_index: None,
        },
    );

    assert!(window.rendered_len() < row_heights.len());
    assert!(window.after_height > 0.0);
    assert!(window.rendered_len() <= 36);
}

#[test]
fn transcript_virtual_window_renders_small_projects_without_spacers() {
    let row_heights = vec![30.0; 80];

    let window = super::transcript_virtualization::transcript_virtual_window(
        &row_heights,
        super::transcript_virtualization::TranscriptVirtualizationInput {
            scroll_offset_y: 900.0,
            viewport_height: 600.0,
            overscan_height: 600.0,
            render_all_threshold: 100,
            pinned_row_index: None,
        },
    );

    assert_eq!(window.start_index, 0);
    assert_eq!(window.end_index, 80);
    assert_eq!(window.rendered_len(), 80);
    assert_eq!(window.before_height, 0.0);
    assert_eq!(window.after_height, 0.0);
}

#[test]
fn transcript_virtual_window_respects_variable_row_heights() {
    let row_heights = vec![10.0, 20.0, 30.0, 40.0, 50.0, 60.0];

    let window = super::transcript_virtualization::transcript_virtual_window(
        &row_heights,
        super::transcript_virtualization::TranscriptVirtualizationInput {
            scroll_offset_y: 55.0,
            viewport_height: 35.0,
            overscan_height: 0.0,
            render_all_threshold: 0,
            pinned_row_index: None,
        },
    );

    assert_eq!(window.start_index, 2);
    assert_eq!(window.end_index, 4);
    assert_eq!(window.before_height, 30.0);
    assert_eq!(window.after_height, 110.0);
}

#[test]
fn transcript_scroll_message_tracks_absolute_viewport_for_virtual_rows() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::TranscriptScrolled {
            offset_y: 450.4,
            viewport_height: 312.0,
        },
    );

    assert_eq!(state.transcript_scroll_offset_y, 450.4);
    assert_eq!(state.transcript_scroll_viewport_height, 312.0);

    let _ = update(
        &mut state,
        Message::TranscriptScrolled {
            offset_y: f32::NAN,
            viewport_height: -1.0,
        },
    );

    assert_eq!(state.transcript_scroll_offset_y, 0.0);
    assert_eq!(
        state.transcript_scroll_viewport_height,
        super::transcript_virtualization::DEFAULT_TRANSCRIPT_VIEWPORT_HEIGHT
    );
}

#[test]
fn selecting_far_transcript_row_updates_virtual_scroll_target() {
    let project = virtualized_two_lane_project();
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like project.".to_string(),
    );

    state.transcript_scroll_viewport_height = 300.0;
    state.select_segment("annotation-0260");

    assert!(state.transcript_scroll_offset_y > 7_000.0);
    assert!(state.transcript_scroll_offset_y < 8_000.0);
}

#[test]
fn browser_behavior_startup_smoke_exercises_native_browser_contracts() {
    let mut state = stable_sample_state();

    let report = run_browser_behavior_smoke(&mut state);

    assert!(report.all_passed());
    assert!(report.hotkeys.all_passed());
    assert_eq!(
        report.escape_order,
        vec![
            "find".to_string(),
            "edit".to_string(),
            "create".to_string(),
            "hotkeys".to_string(),
            "admin".to_string(),
            "skip".to_string(),
            "submit".to_string(),
            "loop".to_string(),
        ]
    );
    assert!(report.pending_escape_preserved_dialog);
    assert!(report.pending_escape_preserved_loop);
    assert!(report.default_hotkeys_before_submit);
    assert!(report.default_submit_started_pending_request);
    assert!(report.default_create_region_submitted);
    assert!(report.default_edit_annotation_submitted);
    assert!(report.default_find_routes_next);
    assert!(report.create_region_select_preserved);
    assert!(report.create_region_portal_preserved);
    assert!(report.create_region_plain_outside_closed);
    assert!(report.edit_annotation_select_preserved);
    assert!(report.edit_annotation_inside_preserved);
    assert!(report.edit_annotation_portal_preserved);
    assert!(report.edit_annotation_plain_outside_closed);
    assert!(report.undo_redo_disabled);
    assert!(report.saved_button_not_clickable);
    assert!(report.submit_disabled);
    assert!(report.skip_disabled);
    assert!(report.admin_pending_controls_disabled);
    assert!(report.drop_claim_disabled);
    assert!(report.runtime_trace_line().contains("ok=true"));
    assert_eq!(
        state.project().unwrap().name,
        "Browser Behavior Disabled Controls Smoke"
    );
}

#[test]
fn waveform_hover_reuses_timeline_geometry_cache_without_new_misses() {
    let project = large_two_lane_project(20);
    let duration_seconds = studio_app::TimeRange::for_project(&project).end_seconds + 1.0;
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded Babel-like project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
    });

    let project = state.project().unwrap().clone();
    {
        let editor = state.editor.as_ref().unwrap();
        let lane = editor
            .timeline_lanes(&project)
            .into_iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 lane should render");
        assert!(lane.waveform_model.is_some());
    }
    let stats_after_first_render = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .expect("track-1 should have decoded audio")
        .render_cache_stats();
    assert_eq!(stats_after_first_render.misses, 1);

    for (index, time_seconds) in [1.5, 2.0, 2.5, 3.0].into_iter().enumerate() {
        let _ = update(
            &mut state,
            Message::WaveformHitHovered(
                "track-1".to_string(),
                WaveformHit {
                    target: WaveformHitTarget::RegionBody,
                    region_id: Some("annotation-0000".to_string()),
                    time_seconds,
                },
            ),
        );

        let editor = state.editor.as_ref().unwrap();
        let lane = editor
            .timeline_lanes(&project)
            .into_iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 lane should keep rendering during hover");
        assert!(lane.waveform_model.is_some());
        let stats = editor
            .lane_waveforms
            .get("track-1")
            .expect("track-1 should keep decoded audio")
            .render_cache_stats();
        assert_eq!(
            stats.misses, stats_after_first_render.misses,
            "hover-only timeline refresh at {time_seconds}s should not rebuild waveform geometry"
        );
        assert_eq!(stats.hits, stats_after_first_render.hits + index + 1);
    }
}

#[test]
fn next_segment_does_not_rewrite_recovered_babel_loop_range() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-2");
    let project = state.workflow.as_ref().unwrap().project().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);

    let _ = update(&mut state, Message::NextSegment);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-3"));
    assert_eq!(editor.loop_range.start_seconds, 3.5);
    assert_eq!(editor.loop_range.end_seconds, 5.1);
    assert_eq!(editor.stored_loop_range, Some(editor.loop_range));
}

#[test]
fn loop_strip_body_drag_moves_loop_range_without_editing_project() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-2");
    let project = state.workflow.as_ref().unwrap().project().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);
    let before_segments = state.workflow.as_ref().unwrap().project().segments.clone();

    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::RangeBody,
            time_seconds: 4.3,
        }),
    );
    let _ = update(&mut state, Message::LoopStripPressed);
    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::RangeBody,
            time_seconds: 7.0,
        }),
    );
    let _ = update(&mut state, Message::LoopStripReleased);

    let editor = state.editor.as_ref().unwrap();
    assert!(editor.loop_enabled);
    assert_eq!(editor.stored_loop_range, Some(editor.loop_range));
    assert!((editor.loop_range.start_seconds - 6.2).abs() < 0.001);
    assert!((editor.loop_range.end_seconds - 7.8).abs() < 0.001);
    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments,
        before_segments
    );
}

#[test]
fn loop_strip_empty_drag_creates_babel_loop_range_without_editing_project() {
    let mut state = stable_sample_state();
    let before_segments = state.workflow.as_ref().unwrap().project().segments.clone();

    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::Empty,
            time_seconds: 2.0,
        }),
    );
    let _ = update(&mut state, Message::LoopStripPressed);
    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::Empty,
            time_seconds: 5.0,
        }),
    );
    let _ = update(&mut state, Message::LoopStripReleased);

    let editor = state.editor.as_ref().unwrap();
    assert!(editor.loop_enabled);
    assert_eq!(editor.stored_loop_range, Some(editor.loop_range));
    assert_eq!(editor.loop_range.start_seconds, 2.0);
    assert_eq!(editor.loop_range.end_seconds, 5.0);
    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments,
        before_segments
    );
}

#[test]
fn loop_strip_handle_drags_clamp_to_valid_loop_duration() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-2");
    let project = state.workflow.as_ref().unwrap().project().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);

    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::StartHandle,
            time_seconds: 3.5,
        }),
    );
    let _ = update(&mut state, Message::LoopStripPressed);
    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::StartHandle,
            time_seconds: 5.5,
        }),
    );
    let _ = update(&mut state, Message::LoopStripReleased);

    let editor = state.editor.as_ref().unwrap();
    assert!(editor.loop_range.start_seconds < editor.loop_range.end_seconds);
    assert!((editor.loop_range.end_seconds - editor.loop_range.start_seconds) >= 0.01 - 0.000_001);
    assert!((editor.loop_range.end_seconds - 5.1).abs() < 0.001);

    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::EndHandle,
            time_seconds: 5.1,
        }),
    );
    let _ = update(&mut state, Message::LoopStripPressed);
    let _ = update(
        &mut state,
        Message::LoopStripHovered(LoopStripHit {
            target: LoopStripHitTarget::EndHandle,
            time_seconds: 0.0,
        }),
    );
    let _ = update(&mut state, Message::LoopStripReleased);

    let editor = state.editor.as_ref().unwrap();
    assert!(editor.loop_range.end_seconds > editor.loop_range.start_seconds);
    assert!((editor.loop_range.end_seconds - editor.loop_range.start_seconds) >= 0.01 - 0.000_001);
}

#[test]
fn submit_review_marks_clean_project_submitted_and_blocks_linted_project() {
    let mut state = stable_sample_state();
    let warning_segment_ids: Vec<_> = {
        let project = state.workflow.as_ref().unwrap().project();
        let editor = state.editor.as_ref().unwrap();
        editor
            .transcript_rows(project)
            .into_iter()
            .filter(|row| row.lint.is_assertable_warning())
            .map(|row| row.segment_id)
            .collect()
    };
    assert!(!warning_segment_ids.is_empty());
    for segment_id in warning_segment_ids {
        let project = state.workflow.as_ref().unwrap().project().clone();
        let editor = state.editor.as_mut().unwrap();
        editor.select_segment(&project, &segment_id);
        assert!(editor.toggle_warning_assertion(&project));
    }

    state.submit_review();
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Submitted
    );

    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .text
        .clear();
    state.apply_segment("annotation-1");
    state.submit_review();
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );
    assert!(state.status.contains("blocked"));
}

#[test]
fn request_submit_review_opens_babel_confirmation_without_submitting() {
    let mut state = stable_sample_state();
    acknowledge_all_assertable_warnings(&mut state);

    let _ = update(&mut state, Message::RequestSubmitReview);

    assert!(state.show_submit_dialog);
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );
}

#[test]
fn request_submit_review_uses_recovered_babel_disabled_state() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::RequestSubmitReview);

    assert!(!state.show_submit_dialog);
    assert_eq!(state.status, "Please acknowledge all warnings");
}

#[test]
fn submit_review_blocks_recovered_metadata_only_blockers() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    project.segment_mut("annotation-1").unwrap().text = "clean".to_string();
    project.metadata.insert(
        "studio.babel.submit.mode".to_string(),
        "annotation".to_string(),
    );
    project.metadata.insert(
        "studio.babel.submit.hasMissingIntensity".to_string(),
        "true".to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded annotation blocker fixture".to_string(),
    );

    state.submit_review();

    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );
    assert!(state.show_submit_dialog);
    assert_eq!(
        state.status,
        "Submit blocked: Please set intensity for all annotations"
    );
}

#[test]
fn cancel_submit_review_closes_babel_confirmation() {
    let mut state = stable_sample_state();
    acknowledge_all_assertable_warnings(&mut state);
    let _ = update(&mut state, Message::RequestSubmitReview);

    let _ = update(&mut state, Message::CancelSubmitReview);

    assert!(!state.show_submit_dialog);
}

#[test]
fn default_action_submits_babel_confirmation_and_respects_pending_guard() {
    let mut state = stable_sample_state();
    acknowledge_all_assertable_warnings(&mut state);
    let _ = update(&mut state, Message::RequestSubmitReview);

    let _ = update(&mut state, Message::ActivateDefaultSurfaceAction);

    assert!(state.show_submit_dialog);
    assert!(state.submit_is_pending);
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );

    let _ = update(&mut state, Message::SubmitReviewSucceeded);

    assert!(!state.show_submit_dialog);
    assert!(!state.submit_is_pending);
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Submitted
    );

    let mut pending_state = stable_sample_state();
    acknowledge_all_assertable_warnings(&mut pending_state);
    let _ = update(&mut pending_state, Message::RequestSubmitReview);
    pending_state.submit_is_pending = true;

    let _ = update(&mut pending_state, Message::ActivateDefaultSurfaceAction);

    assert!(pending_state.show_submit_dialog);
    assert!(pending_state.submit_is_pending);
    assert_eq!(
        pending_state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );
}

#[test]
fn default_action_uses_top_surface_before_underlying_confirmation() {
    let mut state = stable_sample_state();
    acknowledge_all_assertable_warnings(&mut state);
    let _ = update(&mut state, Message::RequestSubmitReview);
    let _ = update(&mut state, Message::ShowHotkeys);

    let _ = update(&mut state, Message::ActivateDefaultSurfaceAction);

    assert!(!state.show_hotkeys_dialog);
    assert!(state.show_submit_dialog);
    assert!(!state.submit_is_pending);
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );

    let _ = update(&mut state, Message::ActivateDefaultSurfaceAction);

    assert!(state.show_submit_dialog);
    assert!(state.submit_is_pending);
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );

    let _ = update(&mut state, Message::SubmitReviewSucceeded);

    assert!(!state.show_submit_dialog);
    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::SubmitReviewAction(_)))
    );
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Submitted
    );
}

#[test]
fn show_hotkeys_opens_babel_shortcuts_dialog() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::ShowHotkeys);

    assert!(state.show_hotkeys_dialog);
    assert!(!state.status.starts_with("Hotkeys:"));
}

#[test]
fn close_hotkeys_closes_babel_shortcuts_dialog() {
    let mut state = stable_sample_state();
    let _ = update(&mut state, Message::ShowHotkeys);

    let _ = update(&mut state, Message::CloseHotkeys);

    assert!(!state.show_hotkeys_dialog);
}

#[test]
fn active_warning_hotkey_toggles_selected_babel_warning() {
    let mut state = stable_sample_state();
    let warning_segment_id = {
        let project = state.workflow.as_ref().unwrap().project();
        state
            .editor
            .as_ref()
            .unwrap()
            .transcript_rows(project)
            .into_iter()
            .find(|row| row.lint.is_assertable_warning())
            .expect("sample should include an assertable warning")
            .segment_id
    };
    state.select_segment(&warning_segment_id);

    let _ = update(&mut state, Message::ToggleActiveWarningAssertion);

    let project = state.workflow.as_ref().unwrap().project();
    let row = state
        .editor
        .as_ref()
        .unwrap()
        .transcript_rows(project)
        .into_iter()
        .find(|row| row.segment_id == warning_segment_id)
        .expect("selected warning row should still exist");
    assert!(row.has_asserted_warning);
}

#[test]
fn navigate_problem_rows_message_delegates_to_editor_navigation() {
    let mut state = stable_sample_state();
    let project = state.workflow.as_ref().unwrap().project().clone();
    let mut expected = state.editor.as_ref().unwrap().clone();
    expected.navigate_problem_rows(&project, 1);

    let _ = update(&mut state, Message::NavigateProblemRows(1));

    assert_eq!(
        state.editor.as_ref().unwrap().selected_segment_id,
        expected.selected_segment_id
    );
}

#[test]
fn browser_find_query_selects_transcript_matches_and_wraps() {
    let project = large_two_lane_project(6);
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded Babel-like project.".to_string(),
    );

    let _ = update(&mut state, Message::OpenFind);
    assert!(state.find_bar_open);

    let _ = update(
        &mut state,
        Message::FindQueryChanged("large project row".to_string()),
    );
    assert_eq!(state.find_query, "large project row");
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0001")
    );
    let project = state.project().unwrap().clone();
    assert_eq!(
        state.find_match_summary(&project, state.editor.as_ref().unwrap()),
        Some("1 of 5".to_string())
    );

    let _ = update(&mut state, Message::FindPrevious);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0005")
    );
    assert_eq!(
        state.find_match_summary(&project, state.editor.as_ref().unwrap()),
        Some("5 of 5".to_string())
    );

    let _ = update(&mut state, Message::FindNext);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0001")
    );

    let _ = update(&mut state, Message::FindQueryChanged("row 3".to_string()));
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0003")
    );
    assert_eq!(
        state.find_match_summary(&project, state.editor.as_ref().unwrap()),
        Some("1 of 1".to_string())
    );

    let _ = update(
        &mut state,
        Message::FindQueryChanged("not in transcript".to_string()),
    );
    assert_eq!(
        state.find_match_summary(&project, state.editor.as_ref().unwrap()),
        Some("No matches".to_string())
    );
}

#[test]
fn browser_find_navigation_shortcut_messages_move_between_matches() {
    let project = large_two_lane_project(6);
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded Babel-like project.".to_string(),
    );

    let _ = update(&mut state, Message::OpenFind);
    let _ = update(
        &mut state,
        Message::FindQueryChanged("large project row".to_string()),
    );

    let _ = update(&mut state, Message::FindNext);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0002")
    );

    let _ = update(&mut state, Message::FindPrevious);
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("annotation-0001")
    );
}

#[test]
fn browser_find_scroll_fraction_tracks_selected_match_row_position() {
    let project = large_two_lane_project(6);
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded Babel-like project.".to_string(),
    );

    let _ = update(
        &mut state,
        Message::FindQueryChanged("large project row".to_string()),
    );
    let project = state.project().unwrap().clone();
    assert_eq!(
        state.find_scroll_fraction_for_selected_match(&project, state.editor.as_ref().unwrap()),
        Some(0.2)
    );

    let _ = update(&mut state, Message::FindPrevious);
    assert_eq!(
        state.find_scroll_fraction_for_selected_match(&project, state.editor.as_ref().unwrap()),
        Some(1.0)
    );

    let _ = update(
        &mut state,
        Message::FindQueryChanged("not in transcript".to_string()),
    );
    assert_eq!(
        state.find_scroll_fraction_for_selected_match(&project, state.editor.as_ref().unwrap()),
        None
    );
}

#[test]
fn browser_find_scrolls_transcript_pane_to_selected_match() {
    let transcript_source = include_str!("components/transcript_table/mod.rs");
    let actions_source = include_str!("actions.rs");

    assert!(transcript_source.contains("scrollable(rows)"));
    assert!(transcript_source.contains(".id(find::transcript_scroll_id())"));
    assert!(actions_source.contains("scrollable::scroll_to"));
    assert!(actions_source.contains("find::transcript_scroll_id()"));
    assert!(actions_source.contains("find_scroll_offset_for_selected_match"));
}

#[test]
fn toggle_warning_assertion_message_asserts_selected_babel_warning() {
    let mut state = stable_sample_state();
    let warning_segment_id = {
        let project = state.workflow.as_ref().unwrap().project();
        let editor = state.editor.as_ref().unwrap();
        editor
            .transcript_rows(project)
            .into_iter()
            .find(|row| row.lint.is_assertable_warning())
            .expect("sample should include an assertable warning")
            .segment_id
    };
    let before = {
        let project = state.workflow.as_ref().unwrap().project();
        let editor = state.editor.as_ref().unwrap();
        editor.unacknowledged_warning_count(project)
    };

    let _ = update(
        &mut state,
        Message::ToggleWarningAssertion(warning_segment_id.clone()),
    );

    let project = state.workflow.as_ref().unwrap().project();
    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.unacknowledged_warning_count(project) + 1, before);
    assert!(
        editor
            .transcript_rows(project)
            .into_iter()
            .find(|row| row.segment_id == warning_segment_id)
            .unwrap()
            .has_asserted_warning
    );
}

#[test]
fn submit_review_marks_recovered_review_action_export_as_pass() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(include_str!(
            "../../../studio-babel/tests/fixtures/review-action-final.json"
        ))
        .expect("review action fixture should import"),
        "loaded review action fixture".to_string(),
    );
    let warning_segment_ids: Vec<_> = {
        let project = state.workflow.as_ref().unwrap().project();
        let editor = state.editor.as_ref().unwrap();
        editor
            .transcript_rows(project)
            .into_iter()
            .filter(|row| row.lint.is_assertable_warning())
            .map(|row| row.segment_id)
            .collect()
    };
    for segment_id in warning_segment_ids {
        let project = state.workflow.as_ref().unwrap().project().clone();
        let editor = state.editor.as_mut().unwrap();
        editor.select_segment(&project, &segment_id);
        assert!(editor.toggle_warning_assertion(&project));
    }

    state.submit_review();
    let exported = state
        .workflow
        .as_ref()
        .unwrap()
        .export_babel_json()
        .expect("submitted review action should export");
    let value: serde_json::Value =
        serde_json::from_str(&exported).expect("exported review action should be JSON");

    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Submitted
    );
    assert_eq!(
        value.pointer("/0/result/data/json/actionDecision"),
        Some(&serde_json::json!("pass"))
    );
}

#[test]
fn resolving_low_confidence_metadata_unblocks_submit_without_editing_text() {
    let mut state = stable_sample_state();
    let mut project = state.workflow.as_ref().unwrap().project().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 1.0;
    segment.end_seconds = 2.0;
    segment.text = "alpha beta".to_string();
    segment.metadata.clear();
    segment.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "word": "beta", "start": 1.2, "end": 1.4, "isYellow": true }
        ]),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded low-confidence fixture".to_string(),
    );

    state.submit_review();
    assert!(state.status.contains("low-confidence"));

    state.resolve_low_confidence("annotation-1");
    state.submit_review();

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(segment.text, "alpha beta");
    assert_eq!(
        segment.metadata.get("lowConfidenceResolved"),
        Some(&serde_json::json!(true))
    );
    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Submitted
    );
}

#[test]
fn acknowledging_each_low_confidence_word_resolves_babel_segment_metadata() {
    let mut state = stable_sample_state();
    let mut project = state.workflow.as_ref().unwrap().project().clone();
    project
        .segments
        .retain(|segment| segment.id == "annotation-1");
    let segment = project.segment_mut("annotation-1").unwrap();
    segment.start_seconds = 1.0;
    segment.end_seconds = 2.0;
    segment.text = "alpha beta".to_string();
    segment.metadata.clear();
    segment.metadata.insert(
        "babelLowConfidenceWords".to_string(),
        serde_json::json!([
            { "index": 0, "finalWord": "alpha", "start": 1.05, "end": 1.10, "isYellow": true },
            { "index": 1, "finalWord": "beta", "start": 1.20, "end": 1.30, "isYellow": true }
        ]),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded low-confidence fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::AcknowledgeLowConfidenceWord("annotation-1".to_string(), 0),
    );
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .unwrap()
            .metadata
            .get("lowConfidenceResolved"),
        None
    );

    let _ = update(
        &mut state,
        Message::AcknowledgeLowConfidenceWord("annotation-1".to_string(), 1),
    );

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-1")
        .unwrap();
    assert_eq!(
        segment.metadata.get("lowConfidenceResolved"),
        Some(&serde_json::json!(true))
    );
    assert!(
        !state
            .acknowledged_low_confidence_words
            .contains_key("annotation-1")
    );
}

#[test]
fn deleting_segment_clears_recovered_babel_low_confidence_state_first() {
    let mut state = stable_sample_state();
    state
        .acknowledged_low_confidence_words
        .insert("annotation-1".to_string(), BTreeSet::from([0]));

    state.delete_segment("annotation-1");

    assert!(
        !state
            .acknowledged_low_confidence_words
            .contains_key("annotation-1")
    );
}

#[test]
fn merging_segment_clears_recovered_babel_low_confidence_state_first() {
    let mut state = stable_sample_state();
    state
        .acknowledged_low_confidence_words
        .insert("annotation-1".to_string(), BTreeSet::from([0]));

    state.merge_segment_below("annotation-1");

    assert!(
        !state
            .acknowledged_low_confidence_words
            .contains_key("annotation-1")
    );
}

#[test]
fn export_flushes_pending_draft_and_marks_saved_after_write() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };

    state
        .segment_drafts
        .get_mut("annotation-2")
        .unwrap()
        .replace_text("exported draft".to_string());
    state.mark_dirty();
    state.export_babel_json();

    let exported = std::fs::read_to_string(export_path).unwrap();
    let package: BabelTaskPackage = serde_json::from_str(&exported).unwrap();
    let annotation = package
        .annotations
        .iter()
        .find(|annotation| annotation.annotation_id == "annotation-2")
        .unwrap();
    assert_eq!(annotation.text, "exported draft");
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn save_progress_flushes_pending_draft_writes_babel_json_and_marks_saved() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };
    state
        .segment_drafts
        .get_mut("annotation-2")
        .unwrap()
        .replace_text("saved in place".to_string());
    state.mark_dirty();

    let _ = update(&mut state, Message::SaveProgress);

    let segment = state
        .workflow
        .as_ref()
        .unwrap()
        .project()
        .segment("annotation-2")
        .unwrap();
    assert_eq!(segment.text, "saved in place");
    assert_eq!(state.status, "Progress saved.");
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert!(state.editor.as_ref().unwrap().last_saved_at.is_some());
    let exported = std::fs::read_to_string(export_path).unwrap();
    let package: BabelTaskPackage = serde_json::from_str(&exported).unwrap();
    let annotation = package
        .annotations
        .iter()
        .find(|annotation| annotation.annotation_id == "annotation-2")
        .unwrap();
    assert_eq!(annotation.text, "saved in place");
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .transport_strip(state.workflow.as_ref().unwrap().project())
            .save_label,
        "Saved"
    );
}

#[test]
fn save_progress_failure_surfaces_backend_error_on_retry_tooltip() {
    let mut state = stable_sample_state();
    state.mark_dirty();

    state.complete_save_progress_failure("Babel HTTP 400: invalid annotation".to_string());

    let strip = state
        .editor
        .as_ref()
        .unwrap()
        .transport_strip(state.workflow.as_ref().unwrap().project());
    assert_eq!(strip.save_label, "Error - Retry");
    assert!(strip.save_clickable);
    assert_eq!(
        strip.save_tooltip_content,
        "Save failed: Babel HTTP 400: invalid annotation"
    );
}

#[test]
fn save_progress_builds_recovered_babel_annotation_save_request() {
    let mut state = StudioState::default();
    let workflow = StudioWorkflow::from_babel_json(include_str!(
        "../../../studio-babel/tests/fixtures/review-action-final.json"
    ))
    .expect("recovered review action fixture should import");
    state.replace_workflow(workflow, "loaded recovered review action".to_string());
    let edited_segment_id = state.project().unwrap().segments[0].id.clone();
    let edited_annotation_id = state.project().unwrap().segments[0]
        .source_annotation_id
        .clone()
        .unwrap_or_else(|| edited_segment_id.clone());
    state
        .segment_drafts
        .get_mut(&edited_segment_id)
        .unwrap()
        .replace_text("saved remotely".to_string());
    state.mark_dirty();

    assert!(state.apply_pending_drafts("Save"));
    let request = build_save_annotations_request(state.workflow.as_ref().unwrap())
        .expect("save annotations request");

    assert_eq!(
        request.review_action_id,
        "9edb22ab-0993-41f3-926e-90779da4c624"
    );
    assert!(
        request.annotations.iter().any(|annotation| {
            annotation.get("id").and_then(serde_json::Value::as_str)
                == Some(edited_annotation_id.as_str())
                && annotation
                    .get("content")
                    .and_then(serde_json::Value::as_str)
                    == Some("saved remotely")
        }),
        "edited annotation should be included in Babel save payload: {:?}",
        request.annotations
    );
    let envelope =
        request_envelope_for_service_command(&BabelServiceRequest::SaveAnnotations(request));
    assert_eq!(
        envelope["operation"],
        "transcriptions.saveAnnotationsByReviewActionId"
    );
    assert_eq!(
        envelope["payload"]["reviewActionId"],
        "9edb22ab-0993-41f3-926e-90779da4c624"
    );
    assert!(
        envelope["payload"]["annotations"]
            .as_array()
            .is_some_and(|annotations| {
                annotations.iter().any(|annotation| {
                    annotation.get("id").and_then(serde_json::Value::as_str)
                        == Some(edited_annotation_id.as_str())
                        && annotation
                            .get("content")
                            .and_then(serde_json::Value::as_str)
                            == Some("saved remotely")
                })
            })
    );
}

#[test]
fn apply_pending_drafts_commits_multiple_rows_as_one_undo_step() {
    let mut state = stable_sample_state();
    let first_segment_id = state.project().unwrap().segments[0].id.clone();
    let second_segment_id = state.project().unwrap().segments[1].id.clone();
    let original_first = state
        .project()
        .unwrap()
        .segment(&first_segment_id)
        .unwrap()
        .text
        .clone();
    let original_second = state
        .project()
        .unwrap()
        .segment(&second_segment_id)
        .unwrap()
        .text
        .clone();
    state
        .segment_drafts
        .get_mut(&first_segment_id)
        .unwrap()
        .replace_text("Grouped draft one.".to_string());
    state
        .segment_drafts
        .get_mut(&second_segment_id)
        .unwrap()
        .replace_text("Grouped draft two.".to_string());

    assert!(state.apply_pending_drafts("Save"));

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        "Grouped draft one."
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&second_segment_id)
            .unwrap()
            .text,
        "Grouped draft two."
    );

    let _ = update(&mut state, Message::Undo);

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        original_first
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&second_segment_id)
            .unwrap()
            .text,
        original_second
    );
}

#[test]
fn auto_save_tick_flushes_dirty_pending_draft_like_recovered_interval_save() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };
    state
        .segment_drafts
        .get_mut("annotation-2")
        .unwrap()
        .replace_text("autosaved in place".to_string());
    state.mark_dirty();

    let _ = update(&mut state, Message::AutoSaveTick);

    assert_eq!(state.status, "Progress saved.");
    assert!(!state.editor.as_ref().unwrap().is_dirty);
    assert!(state.editor.as_ref().unwrap().last_saved_at.is_some());
    let exported = std::fs::read_to_string(export_path).unwrap();
    let package: BabelTaskPackage = serde_json::from_str(&exported).unwrap();
    let annotation = package
        .annotations
        .iter()
        .find(|annotation| annotation.annotation_id == "annotation-2")
        .unwrap();
    assert_eq!(annotation.text, "autosaved in place");
}

#[test]
fn auto_save_tick_skips_while_submit_pauses_recovered_autosave() {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let export_path = temp.path().join("babel-task.json");
    let mut state = StudioState {
        export_path: export_path.display().to_string(),
        ..stable_sample_state()
    };
    state
        .segment_drafts
        .get_mut("annotation-2")
        .unwrap()
        .replace_text("pending submit autosave pause".to_string());
    state.mark_dirty();
    state.submit_is_pending = true;
    let previous_status = state.status.clone();

    let _ = update(&mut state, Message::AutoSaveTick);

    assert!(state.editor.as_ref().unwrap().is_dirty);
    assert!(!export_path.exists());
    assert_eq!(state.status, previous_status);
}

#[test]
fn editor_autosave_predicate_matches_recovered_dirty_saving_and_paused_gates() {
    let mut state = stable_sample_state();

    assert!(!state.should_run_editor_auto_save_tick());

    state.mark_dirty();
    assert!(state.should_run_editor_auto_save_tick());

    state.submit_is_pending = true;
    assert!(!state.should_run_editor_auto_save_tick());

    state.submit_is_pending = false;
    state.editor.as_mut().unwrap().begin_save();
    assert!(!state.should_run_editor_auto_save_tick());
}

#[test]
fn editor_autosave_runtime_trace_line_reports_save_result() {
    assert_eq!(
        editor_auto_save_runtime_trace_line(
            ".studio-dev/export/babel-task.json",
            true,
            false,
            false
        ),
        "Transcription Studio editor autosave: action=save path=.studio-dev/export/babel-task.json dirty=true->false error=false"
    );
}

#[test]
fn read_only_action_messages_surface_recovered_assessment_intents() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::RetryPractice);
    assert_eq!(state.status, "Practice retry requested.");

    let _ = update(&mut state, Message::TakeAssessment);
    assert_eq!(state.status, "Assessment transition requested.");

    let _ = update(&mut state, Message::RetryAssessment);
    assert_eq!(state.status, "Assessment retry requested.");

    let _ = update(&mut state, Message::BackToProjects);
    assert_eq!(state.status, "Back to projects requested.");
}

fn enable_feedback_panel_for_word_accuracy(state: &mut StudioState) {
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.feedback.showButton".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.rubricFlags".to_string(),
        "tx-word-accuracy,tx-other-feedback".to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded feedback fixture".to_string(),
    );
}

#[test]
fn feedback_panel_opens_from_recovered_toolbar_button_and_closes_like_drawer() {
    let mut state = stable_sample_state();
    enable_feedback_panel_for_word_accuracy(&mut state);

    assert!(
        state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| !panel.open)
    );

    let _ = update(&mut state, Message::OpenFeedbackPanel);

    let panel = state.feedback_panel.as_ref().unwrap();
    assert!(panel.open);
    assert_eq!(panel.title(), "L1 Feedback Form");
    assert_eq!(
        panel.description(),
        "Provide feedback for the L1 transcriber."
    );
    assert_eq!(panel.footer_label(), "Complete all fields");

    let _ = update(&mut state, Message::CloseFeedbackPanel);

    assert!(
        state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| !panel.open)
    );
}

#[test]
fn feedback_panel_hydrates_recovered_initial_feedback_metadata() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.feedback.showButton".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.rubricFlags".to_string(),
        "tx-word-accuracy,tx-other-feedback".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.initialFeedback".to_string(),
        serde_json::json!({
            "wordAccuracy": {
                "rating": 3,
                "comment": "Recovered initial draft"
            },
            "otherFeedback": "Recovered general note"
        })
        .to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded recovered feedback fixture".to_string(),
    );

    let panel = state.feedback_panel.as_ref().unwrap();
    let word_accuracy = panel.draft.entries.get("wordAccuracy").unwrap();
    assert_eq!(word_accuracy.rating, 3);
    assert_eq!(word_accuracy.comment, "Recovered initial draft");
    assert_eq!(panel.draft.other_feedback, "Recovered general note");
    assert_eq!(panel.footer_label(), "Ready");
    assert!(!panel.is_saving);
}

#[test]
fn feedback_panel_hydrates_recovered_form_data_service_responses() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.feedback.showButton".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.readOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.rubricFlags".to_string(),
        "tx-word-accuracy,tx-other-feedback".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.inputs".to_string(),
        serde_json::json!([
            { "id": "rating", "label": "Word Accuracy" },
            { "id": "comment", "label": "Word Accuracy Comment" },
            { "id": "other", "label": "Other Feedback" }
        ])
        .to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.inputResponses".to_string(),
        serde_json::json!([
            { "formInputId": "rating", "value": "2abc" },
            { "formInputId": "comment", "value": "Recovered service comment" },
            { "formInputId": "other", "value": "Recovered service note" }
        ])
        .to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded recovered feedback service fixture".to_string(),
    );

    let panel = state.feedback_panel.as_ref().unwrap();
    assert!(panel.read_only);
    assert_eq!(panel.title(), "Feedback Received");
    let word_accuracy = panel.draft.entries.get("wordAccuracy").unwrap();
    assert_eq!(word_accuracy.rating, 2);
    assert_eq!(word_accuracy.comment, "Recovered service comment");
    assert_eq!(panel.draft.other_feedback, "Recovered service note");
}

#[test]
fn feedback_panel_read_only_without_recovered_initial_feedback_uses_empty_state() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.feedback.showButton".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.readOnly".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.rubricFlags".to_string(),
        "tx-word-accuracy,tx-other-feedback".to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded empty read-only feedback fixture".to_string(),
    );

    let panel = state.feedback_panel.as_ref().unwrap();
    assert!(panel.read_only);
    assert_eq!(panel.title(), "Feedback Received");
    assert!(panel.draft.entries.is_empty());
    assert!(panel.draft.other_feedback.is_empty());
}

#[test]
fn feedback_panel_updates_rubric_draft_with_recovered_comment_limit() {
    let mut state = stable_sample_state();
    enable_feedback_panel_for_word_accuracy(&mut state);
    let _ = update(&mut state, Message::OpenFeedbackPanel);

    let _ = update(
        &mut state,
        Message::FeedbackRatingChanged("wordAccuracy".to_string(), 3),
    );
    let _ = update(
        &mut state,
        Message::FeedbackCommentChanged("wordAccuracy".to_string(), "Specific fix".to_string()),
    );
    let _ = update(
        &mut state,
        Message::FeedbackOtherFeedbackChanged("General note".to_string()),
    );

    let panel = state.feedback_panel.as_ref().unwrap();
    let entry = panel.draft.entries.get("wordAccuracy").unwrap();
    assert_eq!(entry.rating, 3);
    assert_eq!(entry.comment, "Specific fix");
    assert_eq!(panel.draft.other_feedback, "General note");
    assert!(panel.is_saving);

    let too_long = "x".repeat(FEEDBACK_MAX_COMMENT_LENGTH + 1);
    let _ = update(
        &mut state,
        Message::FeedbackCommentChanged("wordAccuracy".to_string(), too_long.clone()),
    );
    let _ = update(&mut state, Message::FeedbackOtherFeedbackChanged(too_long));

    let panel = state.feedback_panel.as_ref().unwrap();
    assert_eq!(
        panel.draft.entries.get("wordAccuracy").unwrap().comment,
        "Specific fix"
    );
    assert_eq!(panel.draft.other_feedback, "General note");

    let _ = update(
        &mut state,
        Message::FeedbackDraftSaved("2026-06-23T10:00:00Z".to_string()),
    );

    let panel = state.feedback_panel.as_ref().unwrap();
    assert!(!panel.is_saving);
    assert_eq!(panel.last_saved.as_deref(), Some("2026-06-23T10:00:00Z"));
    assert_eq!(panel.footer_label(), "Ready");
}

#[test]
fn feedback_panel_edits_enqueue_recovered_save_draft_request() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.feedback.showButton".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.rubricFlags".to_string(),
        "tx-word-accuracy,tx-other-feedback".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.formStepResponseId".to_string(),
        "form-step-response-1".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.inputs".to_string(),
        serde_json::json!([
            { "id": "rating", "label": "Word Accuracy" },
            { "id": "comment", "label": "Word Accuracy Comment" },
            { "id": "other", "label": "Other Feedback" }
        ])
        .to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded feedback save fixture".to_string(),
    );

    let _ = update(
        &mut state,
        Message::FeedbackCommentChanged("wordAccuracy".to_string(), "Queued save".to_string()),
    );

    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::SaveFeedbackDraft(
            FeedbackDraftSaveRequest {
                form_step_response_id: "form-step-response-1".to_string(),
                input_responses: vec![
                    FeedbackInputResponse {
                        form_input_id: "rating".to_string(),
                        value: "1".to_string(),
                    },
                    FeedbackInputResponse {
                        form_input_id: "comment".to_string(),
                        value: "Queued save".to_string(),
                    },
                    FeedbackInputResponse {
                        form_input_id: "other".to_string(),
                        value: String::new(),
                    },
                ],
            }
        ))
    );

    let _ = update(
        &mut state,
        Message::FeedbackDraftSaved("2026-06-23T10:00:00Z".to_string()),
    );

    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::SaveFeedbackDraft(_)))
    );
}

#[test]
fn feedback_panel_imports_recovered_service_metadata_and_enqueues_save_draft() {
    let source = serde_json::json!({
        "json": {
            "actionId": "action-feedback-runtime",
            "actionLevel": 1,
            "studio.babel.feedback.showButton": true,
            "studio.babel.feedback.initialOpen": true,
            "studio.babel.feedback.rubricFlags": "tx-word-accuracy,tx-other-feedback",
            "studio.babel.feedback.formStepResponseId": "runtime-form-step-response",
            "studio.babel.feedback.inputs": [
                { "id": "rating", "label": "Word Accuracy" },
                { "id": "comment", "label": "Word Accuracy Comment" },
                { "id": "other", "label": "Other Feedback" }
            ],
            "processedRecordingUriMap": {
                "chunked-recording-1": "audio/recording.wav"
            },
            "transcriptionChunkProcessedRecordings": [
                {
                    "transcriptionChunkId": "chunk-1",
                    "chunkedProcessedRecordingId": "chunked-recording-1",
                    "processedRecordingId": "processed-recording-1",
                    "speaker": 1
                }
            ],
            "annotations": [
                {
                    "id": "annotation-feedback-runtime",
                    "reviewActionId": "action-feedback-runtime",
                    "type": "transcription",
                    "content": "Runtime feedback fixture",
                    "processedRecordingId": "processed-recording-1",
                    "startTimeInSeconds": 0.0,
                    "endTimeInSeconds": 1.0,
                    "metadata": null
                }
            ],
            "lintErrors": []
        }
    });
    let workflow = StudioWorkflow::from_babel_json(&source.to_string())
        .expect("feedback service metadata should import");
    let mut state = stable_sample_state();
    state.replace_workflow(workflow, "loaded feedback runtime fixture".to_string());

    let panel = state
        .feedback_panel
        .as_ref()
        .expect("feedback panel should be created from imported metadata");
    assert!(panel.open);
    assert_eq!(panel.footer_label(), "Complete all fields");

    let _ = update(
        &mut state,
        Message::FeedbackCommentChanged(
            "wordAccuracy".to_string(),
            "Runtime feedback save".to_string(),
        ),
    );

    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::SaveFeedbackDraft(
            FeedbackDraftSaveRequest {
                form_step_response_id: "runtime-form-step-response".to_string(),
                input_responses: vec![
                    FeedbackInputResponse {
                        form_input_id: "rating".to_string(),
                        value: "1".to_string(),
                    },
                    FeedbackInputResponse {
                        form_input_id: "comment".to_string(),
                        value: "Runtime feedback save".to_string(),
                    },
                    FeedbackInputResponse {
                        form_input_id: "other".to_string(),
                        value: String::new(),
                    },
                ],
            }
        ))
    );
}

#[test]
fn feedback_panel_submit_blocking_tracks_recovered_required_comment_state() {
    let mut state = stable_sample_state();
    enable_feedback_panel_for_word_accuracy(&mut state);
    acknowledge_all_assertable_warnings(&mut state);

    let project = state.project().unwrap();
    let submit = state.editor.as_ref().unwrap().submit_control(
        project,
        state.show_submit_dialog,
        state.submit_is_pending,
        state.submit_has_error,
    );
    assert!(submit.trigger_disabled);
    assert_eq!(
        submit.trigger_tooltip_content,
        "Please complete the feedback form"
    );

    let _ = update(
        &mut state,
        Message::FeedbackCommentChanged("wordAccuracy".to_string(), "Good correction".to_string()),
    );

    let saving_project = state.project().unwrap();
    let saving_submit = state.editor.as_ref().unwrap().submit_control(
        saving_project,
        state.show_submit_dialog,
        state.submit_is_pending,
        state.submit_has_error,
    );
    assert!(saving_submit.trigger_disabled);
    assert_eq!(saving_submit.trigger_tooltip_content, "Saving feedback...");

    let _ = update(
        &mut state,
        Message::FeedbackDraftSaved("2026-06-23T10:00:00Z".to_string()),
    );

    let saved_project = state.project().unwrap();
    let saved_submit = state.editor.as_ref().unwrap().submit_control(
        saved_project,
        state.show_submit_dialog,
        state.submit_is_pending,
        state.submit_has_error,
    );
    assert!(!saved_submit.trigger_disabled);
}

#[test]
fn feedback_panel_component_wires_recovered_drawer_copy_and_actions() {
    let state_source = include_str!("feedback_panel.rs");
    let toolbar_source = include_str!("components/toolbar.rs");
    let panel_source = include_str!("components/feedback_panel.rs");
    let shell_source = include_str!("components/mod.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");

    assert!(toolbar_source.contains("Message::OpenFeedbackPanel"));
    assert!(toolbar_source.contains("Feedback"));
    assert!(state_source.contains("L1 Feedback Form"));
    assert!(state_source.contains("Provide feedback for the L1 transcriber."));
    assert!(state_source.contains("Complete all fields"));
    assert!(state_source.contains("Ready"));
    assert!(panel_source.contains("Message::CloseFeedbackPanel"));
    assert!(shell_source.contains("view_feedback_overlay(panel)"));
    assert!(shell_source.contains("stack![content, view_feedback_overlay(panel)]"));
    assert!(
        !shell_source.contains("layout = layout.push(feedback_panel::view_feedback_panel(panel));")
    );
    assert!(timeline_source.contains("suppress_region_affordances"));
    assert!(waveform_source.contains("if suppress_region_affordances {"));
    assert!(waveform_source.contains("if is_selected || is_hovered {"));
}

fn open_task_lookup_fixture(state: &mut StudioState) {
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.taskLookup.open".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.taskLookup.showStitchedOption".to_string(),
        "true".to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded task lookup fixture".to_string(),
    );
}

fn install_babel_route_metadata(state: &mut StudioState) {
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.reviewActionId".to_string(),
        " review-action-42 ".to_string(),
    );
    project.metadata.insert(
        "studio.babel.activeQueueId".to_string(),
        " chunked-queue-7 ".to_string(),
    );
    project.metadata.insert(
        "studio.babel.stitchedQueueId".to_string(),
        " stitched-queue-9 ".to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded route metadata fixture".to_string(),
    );
}

fn enable_optional_babel_review_controls(state: &mut StudioState) {
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.skip.available".to_string(),
        "true".to_string(),
    );
    project
        .metadata
        .insert("studio.babel.skip.canSkip".to_string(), "true".to_string());
    project.metadata.insert(
        "studio.babel.adminFlag.available".to_string(),
        "true".to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "enabled optional Babel review controls".to_string(),
    );
}

#[test]
fn startup_route_args_enqueue_recovered_review_action_load_before_default_claim() {
    let config = parse_startup_route_config([
        "studio-app",
        "--reviewActionId= review-action-url ",
        "--processedTranscriptionId= processed-url ",
        "--displayFeedback=true",
        "--chunkedQueueId= chunked-queue-url ",
        "--stitchedQueueId= stitched-queue-url ",
    ]);
    assert_eq!(
        config,
        StartupRouteConfig {
            review_action_id: Some("review-action-url".to_string()),
            processed_transcription_id: Some("processed-url".to_string()),
            display_feedback: true,
            show_task_lookup_modal: false,
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            stitched_queue_id: Some("stitched-queue-url".to_string()),
        }
    );

    let mut state = stable_sample_state();
    apply_startup_route_config(&mut state, config);

    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::LoadReviewAction(
            LoadReviewActionRequest {
                review_action_id: "review-action-url".to_string(),
            }
        ))
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .metadata
            .get("studio.babel.reviewActionId"),
        Some(&"review-action-url".to_string())
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .metadata
            .get("studio.babel.displayFeedback"),
        Some(&"true".to_string())
    );
    assert!(
        state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.open)
    );
    assert_eq!(state.status, "Review action route load requested.");
}

#[test]
fn review_action_route_success_imports_recovered_payload_and_drains_request() {
    let mut state = stable_sample_state();
    apply_startup_route_config(
        &mut state,
        StartupRouteConfig {
            review_action_id: Some("review-action-url".to_string()),
            display_feedback: true,
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );
    assert!(
        state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadReviewAction(_)))
    );

    let _ = update(
        &mut state,
        Message::LoadReviewActionSucceeded(review_action_final_with_local_audio()),
    );

    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadReviewAction(_)))
    );
    assert_eq!(state.status, "Review action loaded.");
    let project = state.project().unwrap();
    assert_eq!(project.id, "62077784-367e-4709-85d7-1b2e403944df");
    assert_eq!(project.segments.len(), 24);
    assert_eq!(
        project.metadata.get("babel.format").map(String::as_str),
        Some("review-action")
    );
    assert_eq!(
        project
            .metadata
            .get("studio.babel.displayFeedback")
            .map(String::as_str),
        Some("true")
    );
    assert_eq!(
        project
            .metadata
            .get("studio.babel.activeQueueId")
            .map(String::as_str),
        Some("chunked-queue-url")
    );
    assert!(
        state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.open)
    );
    assert_eq!(
        state.editor.as_ref().unwrap().support_id,
        "9edb22ab-0993-41f3-926e-90779da4c624"
    );
    assert_eq!(
        state
            .editor
            .as_ref()
            .unwrap()
            .selected_segment_id
            .as_deref(),
        Some("eaa5afc3-3d59-4267-b073-d390d0b48060")
    );
    assert!(
        state
            .segment_drafts
            .contains_key("eaa5afc3-3d59-4267-b073-d390d0b48060")
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn remote_babel_payload_success_waits_for_audio_preparation_before_editor() {
    let source = include_str!("../../../studio-babel/tests/fixtures/review-action-final.json");
    let mut state = stable_sample_state();
    let previous_project_id = state.project().unwrap().id.clone();
    apply_startup_route_config(
        &mut state,
        StartupRouteConfig {
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );

    let _ = update(
        &mut state,
        Message::ClaimNextFromQueueSucceeded(source.to_string()),
    );

    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::ClaimNextFromQueue(_)))
    );
    assert_eq!(state.app_surface, AppSurface::Dashboard);
    assert!(state.dashboard.is_pending);
    assert_eq!(state.dashboard.error, None);
    assert_eq!(state.status, "Downloading Babel audio...");
    assert_eq!(state.project().unwrap().id, previous_project_id);
}

#[test]
fn babel_payload_success_callbacks_import_and_drain_matching_service_request() {
    let source = review_action_final_with_local_audio();

    let mut latest_state = stable_sample_state();
    apply_startup_route_config(
        &mut latest_state,
        StartupRouteConfig {
            processed_transcription_id: Some("processed-url".to_string()),
            stitched_queue_id: Some("stitched-queue-url".to_string()),
            display_feedback: true,
            ..StartupRouteConfig::default()
        },
    );
    assert!(
        latest_state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadLatestReviewAction(_)))
    );

    let _ = update(
        &mut latest_state,
        Message::LoadLatestReviewActionSucceeded(source.to_string()),
    );

    assert!(
        !latest_state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadLatestReviewAction(_)))
    );
    assert_eq!(latest_state.status, "Processed transcription loaded.");
    assert_eq!(latest_state.project().unwrap().segments.len(), 24);
    assert_eq!(
        latest_state
            .project()
            .unwrap()
            .metadata
            .get("studio.babel.stitchedQueueId")
            .map(String::as_str),
        Some("stitched-queue-url")
    );
    assert!(
        latest_state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.open)
    );

    let mut lookup_claim_state = stable_sample_state();
    open_task_lookup_fixture(&mut lookup_claim_state);
    install_babel_route_metadata(&mut lookup_claim_state);
    let _ = update(
        &mut lookup_claim_state,
        Message::TaskLookupProcessedTranscriptionChanged("processed-1".to_string()),
    );
    let _ = update(
        &mut lookup_claim_state,
        Message::TaskLookupClickClaimStitched,
    );
    let _ = update(&mut lookup_claim_state, Message::TaskLookupConfirmEdit);
    assert!(
        lookup_claim_state
            .babel_service_requests
            .iter()
            .any(|request| matches!(
                request,
                BabelServiceRequest::ClaimByProcessedTranscription(_)
            ))
    );

    let _ = update(
        &mut lookup_claim_state,
        Message::ClaimByProcessedTranscriptionSucceeded(source.to_string()),
    );

    assert!(
        !lookup_claim_state
            .babel_service_requests
            .iter()
            .any(|request| matches!(
                request,
                BabelServiceRequest::ClaimByProcessedTranscription(_)
            ))
    );
    assert_eq!(lookup_claim_state.status, "Stitched transcription claimed.");
    assert_eq!(lookup_claim_state.project().unwrap().segments.len(), 24);
    assert!(lookup_claim_state.task_lookup.is_none());

    let mut queue_claim_state = stable_sample_state();
    apply_startup_route_config(
        &mut queue_claim_state,
        StartupRouteConfig {
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );
    assert!(
        queue_claim_state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::ClaimNextFromQueue(_)))
    );

    let _ = update(
        &mut queue_claim_state,
        Message::ClaimNextFromQueueSucceeded(source.to_string()),
    );

    assert!(
        !queue_claim_state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::ClaimNextFromQueue(_)))
    );
    assert_eq!(queue_claim_state.status, "Queue claim loaded.");
    assert_eq!(queue_claim_state.project().unwrap().segments.len(), 24);
    assert_eq!(
        queue_claim_state
            .project()
            .unwrap()
            .metadata
            .get("studio.babel.activeQueueId")
            .map(String::as_str),
        Some("chunked-queue-url")
    );
}

#[test]
fn load_and_claim_service_requests_dispatch_once_until_callback_settles() {
    let mut state = stable_sample_state();
    apply_startup_route_config(
        &mut state,
        StartupRouteConfig {
            review_action_id: Some("review-action-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );

    let _ = state.dispatch_new_babel_service_requests();
    let _ = state.dispatch_new_babel_service_requests();

    assert_eq!(
        state.dispatched_babel_service_requests,
        vec![BabelServiceRequest::LoadReviewAction(
            LoadReviewActionRequest {
                review_action_id: "review-action-url".to_string(),
            }
        )]
    );
    assert!(
        state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadReviewAction(_)))
    );

    let _ = update(
        &mut state,
        Message::LoadReviewActionSucceeded(review_action_final_with_local_audio()),
    );

    assert!(state.babel_service_requests.is_empty());
    assert!(state.dispatched_babel_service_requests.is_empty());
}

#[test]
fn review_backend_service_requests_dispatch_once_until_callback_settles() {
    let mut state = stable_sample_state();
    let request = ReviewBackendRequest::ReviewGenerate(serde_json::json!({
        "reviewActionId": "review-action-1",
        "original": [],
        "current": []
    }));
    state.review_backend_requests.push(request.clone());

    let _ = state.dispatch_new_review_backend_requests();
    let _ = state.dispatch_new_review_backend_requests();

    assert_eq!(
        state.dispatched_review_backend_requests,
        vec![request.clone()]
    );
    assert_eq!(state.review_backend_requests, vec![request.clone()]);

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(request, "{\"ok\":true}".to_string()),
    );

    assert!(state.review_backend_requests.is_empty());
    assert!(state.dispatched_review_backend_requests.is_empty());
}

#[test]
fn reviewer_payload_uses_preserved_original_and_current_native_transcript() {
    let workflow = StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
        .expect("recovered review action should import");
    let mut project = workflow.project().clone();
    let first = project.segments.first_mut().unwrap();
    let first_annotation_id = first
        .source_annotation_id
        .clone()
        .unwrap_or_else(|| first.id.clone());
    let original_text = first
        .metadata
        .get("studio.babel.original.text")
        .and_then(serde_json::Value::as_str)
        .unwrap()
        .to_string();
    first.text = "Corrected native transcript row".to_string();

    let payload = build_reviewer_payload(&project).expect("reviewer payload should build");

    assert_eq!(
        payload["reviewActionId"],
        payload["current"]["annotations"][0]["reviewActionId"]
    );
    let original_annotation = payload["original"]["annotations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|annotation| annotation["id"] == first_annotation_id)
        .unwrap();
    let current_annotation = payload["current"]["annotations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|annotation| annotation["id"] == first_annotation_id)
        .unwrap();
    assert_eq!(
        original_annotation["content"],
        serde_json::json!(original_text)
    );
    assert_eq!(
        current_annotation["content"],
        serde_json::json!("Corrected native transcript row")
    );
    assert_eq!(payload["babelDiff"], serde_json::Value::Null);
    assert!(
        payload["current"]["recordings"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn drafting_payload_builders_emit_gold_extension_contracts_from_native_rows() {
    let workflow = StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
        .expect("recovered review action should import");
    let project = workflow.project();
    let selected_segment_id = project.segments.first().unwrap().id.as_str();
    let config = DraftingRequestConfig::from_inputs(
        "sk-test",
        "openai/gpt-4o-mini",
        "flex",
        "low",
        "draft-1",
    );

    let generate = build_draft_generate_payload(project, &config)
        .expect("draft generate payload should build");
    assert_eq!(generate["projectPreset"], "ru-gold-2sp-v1");
    assert_eq!(generate["openRouterApiKey"], "sk-test");
    assert_eq!(generate["model"], "openai/gpt-4o-mini");
    assert_eq!(generate["serviceTier"], "flex");
    assert_eq!(generate["reasoningEffort"], "low");
    assert_eq!(generate["draftSessionId"], "draft-1");
    assert!(
        generate["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["rowId"] == selected_segment_id)
    );

    let transcribe =
        build_draft_transcribe_segment_payload(project, Some(selected_segment_id), &config)
            .expect("draft transcribe payload should build");
    assert_eq!(transcribe["segment"]["rowId"], selected_segment_id);
    assert_eq!(transcribe["openRouterApiKey"], "sk-test");

    let redistribute =
        build_draft_redistribute_payload(project, Some(selected_segment_id), &config)
            .expect("draft redistribution payload should build");
    assert_eq!(
        redistribute["groups"][0]["speakerKey"],
        project.segments[0].speaker_id
    );
    assert!(
        redistribute["groups"][0]["draftAllocations"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
    );
}

#[test]
fn native_capability_messages_enqueue_reviewer_workspace_requests() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    enable_l2_reviewer_capability(&mut state);

    let _ = update(&mut state, Message::ToggleNativeCapabilityPanel);
    let _ = update(&mut state, Message::ReviewerGenerateReview);
    let _ = update(&mut state, Message::ReviewerCreateSession);
    let _ = update(
        &mut state,
        Message::ReviewerSessionIdChanged("session-1".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerLoadSession);
    let _ = update(
        &mut state,
        Message::ReviewerCardIdChanged("card-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::ReviewerTemplateIdChanged("template-1".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerSetTemplateMatch);
    let _ = update(&mut state, Message::ReviewerClearTemplateMatch);
    let _ = update(&mut state, Message::ReviewerGenerateTemplateSuggestions);
    let _ = update(
        &mut state,
        Message::ReviewerProposalIdChanged("proposal-1".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerApproveTemplateSuggestion);
    let _ = update(&mut state, Message::ReviewerRejectTemplateSuggestion);
    let _ = update(&mut state, Message::ReviewerFinalizeApply);
    let _ = update(&mut state, Message::ReviewerFinalizeSkip);
    let _ = update(&mut state, Message::ReviewerSubmitAnalytics);
    let _ = update(
        &mut state,
        Message::ReviewerTemplateQueryChanged("timestamp".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerSearchTemplates);
    let _ = update(
        &mut state,
        Message::ReviewerHistoryQueryChanged("reviewActionId=action-1".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerLoadHistory);

    assert!(state.native_capability_panel_open);
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewGenerate(payload) if payload["original"].is_object())
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewCreateSession(payload) if payload["current"].is_object())
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewGetSession { session_id } if session_id == "session-1")
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewSetTemplateMatch { session_id, card_id, body }
            if session_id == "session-1" && card_id == "card-1" && body["templateId"] == "template-1")
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewGenerateTemplateSuggestions { session_id } if session_id == "session-1")
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewDecideTemplateSuggestion { proposal_id, body, .. }
            if proposal_id == "proposal-1" && body["decision"] == "approved")
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewFinalizeSession { body, .. } if body["mode"] == "apply")
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewSubmitAnalytics(payload)
            if payload["reviewActionId"].is_string()
                && payload["inputBoxes"]["categories"].is_object()
                && payload["metadata"]["client"] == "transcription-studio-native")
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewSearchTemplates { query, limit } if query == "timestamp" && *limit == Some(20))
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewHistory { query } if query == &vec![("reviewActionId".to_string(), "action-1".to_string())])
    }));
}

#[test]
fn native_capability_messages_enqueue_gold_drafting_requests() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );

    let _ = update(
        &mut state,
        Message::DraftOpenRouterApiKeyChanged("sk-test".to_string()),
    );
    let _ = update(
        &mut state,
        Message::DraftModelChanged("openai/gpt-4o-mini".to_string()),
    );
    let _ = update(
        &mut state,
        Message::DraftServiceTierChanged("flex".to_string()),
    );
    let _ = update(
        &mut state,
        Message::DraftReasoningEffortChanged("low".to_string()),
    );
    let _ = update(
        &mut state,
        Message::DraftSessionIdChanged("draft-session-1".to_string()),
    );
    let _ = update(&mut state, Message::DraftGenerateRows);
    let _ = update(&mut state, Message::DraftContinueTextOnly);
    let selected_segment = state.project().unwrap().segments.first().unwrap().clone();
    state.editor.as_mut().unwrap().selected_segment_id = Some(selected_segment.id.clone());
    state.editor.as_mut().unwrap().set_lane_waveform(
        selected_segment.lane_id.clone(),
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.0, 0.25, -0.25, 0.5]])
                .expect("test audio should be valid"),
        ),
    );
    let _ = update(&mut state, Message::DraftTranscribeSelectedSegment);
    let _ = update(&mut state, Message::DraftRedistributeText);

    assert!(state.review_backend_requests.iter().any(|request| {
        match request {
            ReviewBackendRequest::DraftGenerateStream(payload) => {
                payload["projectPreset"] == "ru-gold-2sp-v1"
                    && payload["openRouterApiKey"] == "sk-test"
                    && payload["model"] == "openai/gpt-4o-mini"
            }
            ReviewBackendRequest::DraftGenerateStreamWithAudio { payload, .. } => {
                payload["projectPreset"] == "ru-gold-2sp-v1"
                    && payload["openRouterApiKey"] == "sk-test"
                    && payload["model"] == "openai/gpt-4o-mini"
            }
            _ => false,
        }
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::DraftBrokerTranscribeSegmentWithAudio { payload, audio_tracks }
            if payload["segment"]["rowId"].is_string() && !audio_tracks.is_empty())
    }));
    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::DraftBrokerRedistributeText(payload)
            if payload["groups"].as_array().is_some_and(|items| !items.is_empty()))
    }));
}

#[test]
fn drafting_generate_rows_attaches_native_audio_cues_when_available() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    let lane = state.project().unwrap().lanes.first().unwrap().clone();
    state.editor.as_mut().unwrap().set_lane_waveform(
        lane.id.clone(),
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.0, 0.25, -0.25, 0.5]])
                .expect("test audio should be valid"),
        ),
    );
    let _ = update(
        &mut state,
        Message::DraftOpenRouterApiKeyChanged("sk-test".to_string()),
    );

    let _ = update(&mut state, Message::DraftGenerateRows);
    let _ = update(&mut state, Message::DraftUseCapturedAudio);

    let request = state
        .review_backend_requests
        .iter()
        .find_map(|request| match request {
            ReviewBackendRequest::DraftGenerateStreamWithAudio {
                payload,
                audio_tracks,
            } => Some((payload, audio_tracks)),
            _ => None,
        })
        .expect("draft generation should attach native lane audio cues");
    assert_eq!(request.0["projectPreset"], "ru-gold-2sp-v1");
    assert_eq!(request.1.len(), 1);
    assert_eq!(request.1[0].track_id, lane.id);
    assert_eq!(request.1[0].metadata["speakerKey"], lane.speaker_id);
    studio_waveform::decode_pcm16_wav(&request.1[0].bytes)
        .expect("attached draft audio should be PCM16 WAV");
}

#[test]
fn drafting_generate_rows_uses_recovered_stream_route_with_audio_cues() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    let lanes = state.project().unwrap().lanes.clone();
    for lane in &lanes {
        state.editor.as_mut().unwrap().set_lane_waveform(
            lane.id.clone(),
            LaneWaveform::from_decoded(
                DecodedAudio::new(8, vec![vec![0.0, 0.25, -0.25, 0.5, 0.0]])
                    .expect("test audio should be valid"),
            ),
        );
    }

    let _ = update(
        &mut state,
        Message::DraftOpenRouterApiKeyChanged("sk-test".to_string()),
    );
    let _ = update(&mut state, Message::DraftGenerateRows);

    let request = state
        .review_backend_requests
        .iter()
        .find_map(|request| match request {
            ReviewBackendRequest::DraftGenerateStreamWithAudio {
                payload,
                audio_tracks,
            } => Some((payload, audio_tracks)),
            _ => None,
        })
        .expect("draft generation should use the recovered stream route with native audio cues");

    assert_eq!(request.0["openRouterApiKey"], "sk-test");
    assert_eq!(request.1.len(), lanes.len());
}

#[test]
fn drafting_generate_rows_shows_recovered_audio_guard_before_text_only_continue() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::DraftOpenRouterApiKeyChanged("sk-test".to_string()),
    );
    let _ = update(&mut state, Message::DraftGenerateRows);

    let guard = state
        .draft_audio_guard
        .as_ref()
        .expect("missing native lane audio should open the drafting audio guard");
    assert_eq!(guard.captured_speaker_lanes, 0);
    assert!(!guard.can_use_partial_audio);
    assert!(state.review_backend_requests.is_empty());

    let _ = update(&mut state, Message::DraftContinueTextOnly);

    assert!(state.draft_audio_guard.is_none());
    let stream_payload = state
        .review_backend_requests
        .iter()
        .find_map(|request| match request {
            ReviewBackendRequest::DraftGenerateStream(payload) => Some(payload),
            _ => None,
        })
        .expect("text-only continue should queue a recovered stream request");
    assert_eq!(stream_payload["openRouterApiKey"], "sk-test");
    assert!(
        stream_payload["draftSessionId"]
            .as_str()
            .is_some_and(|session_id| session_id
                .starts_with(stream_payload["jobId"].as_str().expect("job id")))
    );
}

#[test]
fn drafting_stream_failure_queues_recovered_reconcile_request() {
    let mut state = stable_sample_state();
    let request = ReviewBackendRequest::DraftGenerateStream(serde_json::json!({
        "jobId": "draft-job",
        "draftSessionId": "draft-job:session-1",
        "rows": []
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestFailed(request, "failed to fetch".to_string()),
    );

    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(
            request,
            ReviewBackendRequest::DraftGenerate(payload)
                if payload["draftSessionId"] == "draft-job:session-1"
        )
    }));
    assert_eq!(
        state.status,
        "Stream connection lost. Reconciling final draft response..."
    );
}

#[test]
fn drafting_audio_cue_status_reports_loaded_and_missing_lanes() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    let lanes = state.project().unwrap().lanes.clone();

    let status = state
        .draft_audio_cue_status()
        .expect("loaded project should report draft audio cue status");
    assert_eq!(status.total_lanes, lanes.len());
    assert_eq!(status.loaded_tracks, 0);
    assert_eq!(
        status.missing_lane_labels,
        lanes
            .iter()
            .map(|lane| lane.label.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        status.missing_lane_sources,
        lanes
            .iter()
            .map(|lane| lane.source_path.clone())
            .collect::<Vec<_>>()
    );

    state.editor.as_mut().unwrap().set_lane_waveform(
        lanes[0].id.clone(),
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.0, 0.25, -0.25, 0.5]])
                .expect("test audio should be valid"),
        ),
    );

    let status = state
        .draft_audio_cue_status()
        .expect("loaded project should report draft audio cue status");
    assert_eq!(status.total_lanes, lanes.len());
    assert_eq!(status.loaded_tracks, 1);
    assert_eq!(
        status.missing_lane_labels,
        lanes
            .iter()
            .skip(1)
            .map(|lane| lane.label.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        status.missing_lane_sources,
        lanes
            .iter()
            .skip(1)
            .map(|lane| lane.source_path.clone())
            .collect::<Vec<_>>()
    );
}

#[test]
fn drafting_transcribe_segment_blocks_until_native_lane_audio_is_loaded() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    let _ = update(
        &mut state,
        Message::DraftOpenRouterApiKeyChanged("sk-test".to_string()),
    );

    let _ = update(&mut state, Message::DraftTranscribeSelectedSegment);

    assert!(state.review_backend_requests.iter().all(|request| {
        !matches!(
            request,
            ReviewBackendRequest::DraftBrokerTranscribeSegment(_)
                | ReviewBackendRequest::DraftBrokerTranscribeSegmentWithAudio { .. }
        )
    }));
    assert_eq!(
        state.status,
        "Drafting transcription blocked: load native lane audio first."
    );
}

#[test]
fn drafting_transcribe_segment_attaches_native_lane_audio_when_available() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    let selected_segment = state.project().unwrap().segments.first().unwrap().clone();
    state.editor.as_mut().unwrap().selected_segment_id = Some(selected_segment.id.clone());
    state.editor.as_mut().unwrap().set_lane_waveform(
        selected_segment.lane_id.clone(),
        LaneWaveform::from_decoded(
            DecodedAudio::new(4, vec![vec![0.0, 0.25, -0.25, 0.5]])
                .expect("test audio should be valid"),
        ),
    );
    let _ = update(
        &mut state,
        Message::DraftOpenRouterApiKeyChanged("sk-test".to_string()),
    );

    let _ = update(&mut state, Message::DraftTranscribeSelectedSegment);

    let request = state
        .review_backend_requests
        .iter()
        .find_map(|request| match request {
            ReviewBackendRequest::DraftBrokerTranscribeSegmentWithAudio {
                payload,
                audio_tracks,
            } => Some((payload, audio_tracks)),
            _ => None,
        })
        .expect("transcribe request should attach native lane audio");
    assert_eq!(request.0["segment"]["rowId"], selected_segment.id);
    assert_eq!(request.1.len(), 1);
    assert_eq!(request.1[0].track_id, selected_segment.lane_id);
    assert_eq!(
        request.1[0].metadata["speakerKey"],
        selected_segment.speaker_id
    );
    assert_eq!(&request.1[0].bytes[0..4], b"RIFF");
    studio_waveform::decode_pcm16_wav(&request.1[0].bytes)
        .expect("attached broker audio should be PCM16 WAV");
}

#[test]
fn drafting_transcription_success_populates_diff_preview_for_request_segment() {
    let mut state = stable_sample_state();
    let segment = state.project().unwrap().segments.first().unwrap().clone();
    let request = ReviewBackendRequest::DraftBrokerTranscribeSegment(serde_json::json!({
        "segment": {
            "rowId": segment.id.clone()
        }
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "text": "Fresh broker transcription.",
                "model": "openai/gpt-4o-mini"
            })
            .to_string(),
        ),
    );

    let preview = state
        .draft_transcription_preview
        .as_ref()
        .expect("transcription response should populate a native preview");
    assert_eq!(preview.segment_id.as_deref(), Some(segment.id.as_str()));
    assert_eq!(preview.original_text, segment.text);
    assert_eq!(preview.text, "Fresh broker transcription.");
    assert_eq!(preview.model.as_deref(), Some("openai/gpt-4o-mini"));
}

#[test]
fn drafting_apply_last_rows_uses_undoable_segment_edits() {
    let mut state = stable_sample_state();
    let segment_id = state
        .project()
        .unwrap()
        .segments
        .first()
        .unwrap()
        .id
        .clone();
    let original_text = state
        .project()
        .unwrap()
        .segment(&segment_id)
        .unwrap()
        .text
        .clone();
    state.native_capability_last_response = Some(
        serde_json::json!({
            "draftRows": [
                {
                    "rowId": segment_id,
                    "rewrittenText": "Drafted replacement text.",
                    "status": "rewritten",
                    "warnings": []
                }
            ],
            "summary": {
                "totalRows": 1,
                "rewrittenRows": 1,
                "unchangedRows": 0,
                "failedRows": 0,
                "anomalyCounts": {}
            }
        })
        .to_string(),
    );

    let _ = update(&mut state, Message::DraftApplyLastRows);

    assert_eq!(
        state.project().unwrap().segment(&segment_id).unwrap().text,
        "Drafted replacement text."
    );
    assert!(state.editor.as_ref().unwrap().is_dirty);
    assert!(state.can_undo());

    let _ = update(&mut state, Message::Undo);

    assert_eq!(
        state.project().unwrap().segment(&segment_id).unwrap().text,
        original_text
    );
}

#[test]
fn drafting_apply_segment_text_uses_selected_segment_and_undo() {
    let mut state = stable_sample_state();
    let segment_id = state
        .editor
        .as_ref()
        .unwrap()
        .selected_segment_id
        .clone()
        .unwrap();
    let original_text = state
        .project()
        .unwrap()
        .segment(&segment_id)
        .unwrap()
        .text
        .clone();
    state.native_capability_last_response = Some(
        serde_json::json!({
            "text": "Fresh broker transcription.",
            "model": "openai/gpt-4o-mini"
        })
        .to_string(),
    );

    let _ = update(&mut state, Message::DraftApplySegmentText);

    assert_eq!(
        state.project().unwrap().segment(&segment_id).unwrap().text,
        "Fresh broker transcription."
    );
    assert!(state.can_undo());

    let _ = update(&mut state, Message::Undo);

    assert_eq!(
        state.project().unwrap().segment(&segment_id).unwrap().text,
        original_text
    );
}

#[test]
fn drafting_apply_segment_text_uses_preview_segment_when_selection_changes() {
    let mut state = stable_sample_state();
    let segments = state
        .project()
        .unwrap()
        .segments
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(segments.len(), 2);
    let original_first = segments[0].text.clone();
    let original_second = segments[1].text.clone();
    let request = ReviewBackendRequest::DraftBrokerTranscribeSegment(serde_json::json!({
        "segment": {
            "rowId": segments[0].id.clone()
        }
    }));
    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "text": "Fresh broker transcription.",
                "model": "openai/gpt-4o-mini"
            })
            .to_string(),
        ),
    );
    state.editor.as_mut().unwrap().selected_segment_id = Some(segments[1].id.clone());

    let _ = update(&mut state, Message::DraftApplySegmentText);

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&segments[0].id)
            .unwrap()
            .text,
        "Fresh broker transcription."
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&segments[1].id)
            .unwrap()
            .text,
        original_second
    );
    assert!(state.can_undo());

    let _ = update(&mut state, Message::Undo);

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&segments[0].id)
            .unwrap()
            .text,
        original_first
    );
}

#[test]
fn reviewer_success_responses_populate_native_session_and_template_state() {
    let mut state = stable_sample_state();
    let session_request = ReviewBackendRequest::ReviewCreateSession(serde_json::json!({
        "reviewActionId": "review-action-1"
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            session_request,
            serde_json::json!({
                "sessionId": "session-1",
                "reviewActionId": "review-action-1",
                "backendVersion": {
                    "service": "babel-review-backend",
                    "release": "0.3.9",
                    "apiSchema": 1,
                    "evidenceSchema": 1
                },
                "cards": [
                    {
                        "id": "card-1",
                        "changeIndex": 4,
                        "type": "TEXT CHANGE",
                        "summary": "Text changed",
                        "evidence": "[-old+new]",
                        "categories": ["Word Accuracy"],
                        "matchedTemplateId": "tpl-1",
                        "templateTitle": "Wrong word",
                        "matchSource": "model",
                        "opinionText": "Use the template.",
                        "rationale": "The diff is a word substitution."
                    }
                ],
                "comments": {
                    "sessionComment": "session note",
                    "cardComments": {
                        "card-1": "card note"
                    }
                },
                "suggestions": [
                    {
                        "proposalId": "proposal-1",
                        "operation": "create_template",
                        "category": "Word Accuracy",
                        "title": "New template",
                        "description": "Reusable review note",
                        "reportTexts": ["Use this wording"],
                        "reason": "Repeated pattern",
                        "sourceCardIds": ["card-1"],
                        "decision": "pending"
                    }
                ],
                "categoryFeedback": []
            })
            .to_string(),
        ),
    );

    let session = state
        .reviewer_session_preview
        .as_ref()
        .expect("reviewer session should be parsed");
    assert_eq!(session.session_id, "session-1");
    assert_eq!(
        session.backend_service.as_deref(),
        Some("babel-review-backend")
    );
    assert_eq!(session.session_comment, "session note");
    assert_eq!(session.cards[0].id, "card-1");
    assert_eq!(
        session.cards[0].template_title.as_deref(),
        Some("Wrong word")
    );
    assert_eq!(session.cards[0].comment, "card note");
    assert_eq!(session.suggestions[0].proposal_id, "proposal-1");
    assert_eq!(state.reviewer_session_id, "session-1");
    assert_eq!(state.reviewer_card_id, "card-1");

    let search_request = ReviewBackendRequest::ReviewSearchTemplates {
        query: "wrong word".to_string(),
        limit: Some(20),
    };
    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            search_request,
            serde_json::json!({
                "query": "wrong word",
                "results": [
                    {
                        "id": "tpl-1",
                        "title": "Wrong word",
                        "description": "Use for word substitutions",
                        "category": "Word Accuracy",
                        "reportTexts": ["Wrong word"],
                        "score": 0.91
                    }
                ]
            })
            .to_string(),
        ),
    );

    assert_eq!(state.reviewer_template_results.len(), 1);
    assert_eq!(state.reviewer_template_results[0].id, "tpl-1");
    assert_eq!(state.reviewer_template_results[0].score, "0.91");
}

#[test]
fn reviewer_card_selection_syncs_template_and_comment_from_session_preview() {
    let mut state = stable_sample_state();
    let session_request = ReviewBackendRequest::ReviewCreateSession(serde_json::json!({
        "reviewActionId": "review-action-1"
    }));
    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            session_request,
            serde_json::json!({
                "sessionId": "session-1",
                "reviewActionId": "review-action-1",
                "cards": [
                    {
                        "id": "card-1",
                        "changeIndex": 1,
                        "type": "TEXT CHANGE",
                        "summary": "First card",
                        "evidence": "first",
                        "matchedTemplateId": "tpl-1",
                        "templateTitle": "First template"
                    },
                    {
                        "id": "card-2",
                        "changeIndex": 2,
                        "type": "TEXT CHANGE",
                        "summary": "Second card",
                        "evidence": "second",
                        "matchedTemplateId": "tpl-2",
                        "templateTitle": "Second template"
                    },
                    {
                        "id": "card-3",
                        "changeIndex": 3,
                        "type": "TEXT CHANGE",
                        "summary": "Third card",
                        "evidence": "third"
                    }
                ],
                "comments": {
                    "cardComments": {
                        "card-1": "note one",
                        "card-2": "note two",
                        "card-3": "note three"
                    }
                },
                "suggestions": [],
                "categoryFeedback": []
            })
            .to_string(),
        ),
    );

    let _ = update(
        &mut state,
        Message::ReviewerCardIdChanged("card-2".to_string()),
    );

    assert_eq!(state.reviewer_card_id, "card-2");
    assert_eq!(state.reviewer_template_id, "tpl-2");
    assert_eq!(state.reviewer_card_comment, "note two");

    let _ = update(
        &mut state,
        Message::ReviewerCardIdChanged("card-3".to_string()),
    );

    assert_eq!(state.reviewer_card_id, "card-3");
    assert_eq!(state.reviewer_template_id, "");
    assert_eq!(state.reviewer_card_comment, "note three");
}

#[test]
fn reviewer_session_workspace_expansion_matches_recovered_review_dialog() {
    let mut state = stable_sample_state();
    let session_request = ReviewBackendRequest::ReviewCreateSession(serde_json::json!({
        "reviewActionId": "review-action-1"
    }));
    let cards = (1..=7)
        .map(|index| {
            serde_json::json!({
                "id": format!("card-{index}"),
                "changeIndex": index,
                "type": "TEXT CHANGE",
                "summary": format!("Card {index}"),
                "evidence": format!("Evidence {index}")
            })
        })
        .collect::<Vec<_>>();
    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            session_request,
            serde_json::json!({
                "sessionId": "session-expanded",
                "reviewActionId": "review-action-1",
                "cards": cards,
                "comments": {},
                "suggestions": [
                    {
                        "proposalId": "proposal-1",
                        "operation": "create_template",
                        "category": "Word Accuracy",
                        "title": "Reusable template",
                        "reason": "Repeated reviewer change",
                        "decision": "pending"
                    }
                ],
                "categoryFeedback": []
            })
            .to_string(),
        ),
    );

    assert!(!state.reviewer_cards_expanded);
    assert!(state.reviewer_suggestions_expanded);

    let _ = update(&mut state, Message::ReviewerExpandAllCards);
    assert!(state.reviewer_cards_expanded);

    let _ = update(&mut state, Message::ReviewerCollapseAllCards);
    assert!(!state.reviewer_cards_expanded);

    let _ = update(&mut state, Message::ReviewerToggleSuggestionsExpanded);
    assert!(!state.reviewer_suggestions_expanded);

    let _ = update(&mut state, Message::ReviewerToggleSuggestionsExpanded);
    assert!(state.reviewer_suggestions_expanded);
}

#[test]
fn reviewer_session_id_persists_to_project_metadata_and_hydrates_on_reload() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::ReviewerSessionIdChanged("session-persisted".to_string()),
    );

    let project = state.project().expect("project should be loaded");
    assert_eq!(
        project
            .metadata
            .get("studio.native.reviewer.sessionId")
            .map(String::as_str),
        Some("session-persisted")
    );

    let mut reloaded = stable_sample_state();
    reloaded.replace_workflow(
        StudioWorkflow::from_project(project.clone()),
        "reloaded persisted reviewer session".to_string(),
    );

    assert_eq!(reloaded.reviewer_session_id, "session-persisted");
}

#[test]
fn reviewer_comment_changes_autosave_on_tick_without_duplicate_requests() {
    let mut state = stable_sample_state();
    enable_l2_reviewer_capability(&mut state);
    state.reviewer_session_id = "session-1".to_string();
    state.reviewer_card_id = "card-1".to_string();

    let _ = update(
        &mut state,
        Message::ReviewerSessionCommentChanged("session autosave note".to_string()),
    );
    let _ = update(
        &mut state,
        Message::ReviewerCardCommentChanged("card autosave note".to_string()),
    );

    assert!(state.reviewer_comments_dirty);

    let _ = update(&mut state, Message::AutoSaveTick);

    assert!(!state.reviewer_comments_dirty);
    let update_requests = state
        .review_backend_requests
        .iter()
        .filter_map(|request| match request {
            ReviewBackendRequest::ReviewUpdateComments { session_id, body } => {
                Some((session_id, body))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(update_requests.len(), 1);
    assert_eq!(update_requests[0].0, "session-1");
    assert_eq!(
        update_requests[0].1["sessionComment"],
        "session autosave note"
    );
    assert_eq!(
        update_requests[0].1["cardComments"]["card-1"],
        "card autosave note"
    );

    let _ = update(&mut state, Message::AutoSaveTick);

    assert_eq!(
        state
            .review_backend_requests
            .iter()
            .filter(|request| matches!(request, ReviewBackendRequest::ReviewUpdateComments { .. }))
            .count(),
        1
    );
}

#[test]
fn reviewer_comment_save_tick_matches_recovered_500ms_debounce() {
    let hotkey_source = include_str!("hotkeys.rs");
    let action_source = include_str!("actions.rs");

    assert!(hotkey_source.contains("REVIEWER_COMMENT_SAVE_TICK_MILLIS: u64 = 500"));
    assert!(hotkey_source.contains("Message::ReviewerCommentSaveTick"));
    assert!(action_source.contains("Message::ReviewerCommentSaveTick"));

    let mut state = stable_sample_state();
    enable_l2_reviewer_capability(&mut state);
    state.reviewer_session_id = "session-1".to_string();
    state.reviewer_card_id = "card-1".to_string();
    let _ = update(
        &mut state,
        Message::ReviewerSessionCommentChanged("debounced session note".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerCommentSaveTick);

    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(
            request,
            ReviewBackendRequest::ReviewUpdateComments { session_id, body }
                if session_id == "session-1"
                    && body["sessionComment"] == "debounced session note"
        )
    }));
}

#[test]
fn reviewer_generate_success_populates_preview_and_analytics_payload() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_babel_json(&review_action_final_with_local_audio())
            .expect("recovered review action should import"),
        "loaded recovered review action".to_string(),
    );
    enable_l2_reviewer_capability(&mut state);
    let request = ReviewBackendRequest::ReviewGenerate(serde_json::json!({
        "reviewActionId": "review-action-1"
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "prepared": {
                    "promptVersion": "review-v1",
                    "metricsVersion": "metrics-v1"
                },
                "llm": {
                    "feedback": [
                        {
                            "category": "Word Accuracy",
                            "score": 1,
                            "note": "Use the wrong-word template text."
                        }
                    ],
                    "rawContent": "{\"classifications\":[{\"change\":4,\"templateId\":\"tpl-1\"}]}",
                    "model": "openai/gpt-4o-mini",
                    "latencyMs": 1200,
                    "receivedAt": "2026-06-29T10:00:00Z",
                    "matchedTemplateIds": ["tpl-1"],
                    "classifications": [
                        {
                            "change": 4,
                            "templateId": "tpl-1"
                        }
                    ],
                    "templateRegistryVersion": "registry-v1"
                }
            })
            .to_string(),
        ),
    );

    let generated = state
        .reviewer_generated_review_preview
        .as_ref()
        .expect("generated review should be parsed");
    assert_eq!(generated.model.as_deref(), Some("openai/gpt-4o-mini"));
    assert_eq!(
        generated.template_registry_version.as_deref(),
        Some("registry-v1")
    );
    assert_eq!(generated.matched_template_ids, vec!["tpl-1".to_string()]);
    assert_eq!(generated.feedback[0].category, "Word Accuracy");
    assert_eq!(generated.cards[0].id, "change-4");
    assert_eq!(
        generated.cards[0].matched_template_id.as_deref(),
        Some("tpl-1")
    );

    let _ = update(&mut state, Message::ReviewerSubmitAnalytics);

    assert!(state.review_backend_requests.iter().any(|request| {
        matches!(request, ReviewBackendRequest::ReviewSubmitAnalytics(payload)
            if payload["prepared"]["promptVersion"] == "review-v1"
                && payload["aiReview"]["feedback"][0]["note"] == "Use the wrong-word template text."
                && payload["inputBoxes"]["categories"]["Word Accuracy"]["score"] == "1"
                && payload["inputBoxes"]["categories"]["Word Accuracy"]["note"] == "Use the wrong-word template text."
                && payload["metadata"]["client"] == "transcription-studio-native")
    }));
}

#[test]
fn reviewer_apply_selected_card_updates_feedback_draft_and_queues_save() {
    let mut state = stable_sample_state();
    let mut project = state.project().unwrap().clone();
    project.metadata.insert(
        "studio.babel.feedback.showButton".to_string(),
        "true".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.rubricFlags".to_string(),
        "tx-word-accuracy,tx-other-feedback".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.formStepResponseId".to_string(),
        "feedback-step-1".to_string(),
    );
    project.metadata.insert(
        "studio.babel.feedback.inputs".to_string(),
        serde_json::json!([
            { "id": "word-rating", "label": "Word Accuracy" },
            { "id": "word-comment", "label": "Word Accuracy Comment" },
            { "id": "other", "label": "Other Feedback" }
        ])
        .to_string(),
    );
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded feedback-enabled review action".to_string(),
    );
    let request = ReviewBackendRequest::ReviewGenerate(serde_json::json!({
        "reviewActionId": "review-action-1"
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "llm": {
                    "feedback": [
                        {
                            "category": "Word Accuracy",
                            "score": 2,
                            "note": "Wrong-word correction should be reviewed."
                        }
                    ]
                }
            })
            .to_string(),
        ),
    );

    let _ = update(
        &mut state,
        Message::ReviewerCardIdChanged("feedback-0".to_string()),
    );
    let _ = update(&mut state, Message::ReviewerApplySelectedCardFeedback);

    let panel = state
        .feedback_panel
        .as_ref()
        .expect("feedback panel should remain available");
    let entry = panel
        .draft
        .entries
        .get("wordAccuracy")
        .expect("word accuracy feedback should be updated");
    assert_eq!(entry.rating, 2);
    assert_eq!(entry.comment, "Wrong-word correction should be reviewed.");
    assert!(panel.open);
    assert!(panel.is_saving);
    assert!(state.status.contains("Applied reviewer card feedback-0"));
    assert!(state.babel_service_requests.iter().any(|request| {
        matches!(request, BabelServiceRequest::SaveFeedbackDraft(save)
            if save.form_step_response_id == "feedback-step-1"
                && save.input_responses.iter().any(|response| response.form_input_id == "word-rating" && response.value == "2")
                && save.input_responses.iter().any(|response| response.form_input_id == "word-comment" && response.value == "Wrong-word correction should be reviewed."))
    }));
}

#[test]
fn reviewer_finalize_success_populates_feedback_preview_without_session_cards() {
    let mut state = stable_sample_state();
    let request = ReviewBackendRequest::ReviewFinalizeSession {
        session_id: "session-1".to_string(),
        body: serde_json::json!({ "mode": "skip" }),
    };

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "sessionId": "session-1",
                "reviewActionId": "review-action-1",
                "backendVersion": {
                    "service": "babel-review-backend",
                    "release": "0.3.9",
                    "apiSchema": 1,
                    "evidenceSchema": 1
                },
                "categoryFeedback": [
                    {
                        "category": "Timestamp Accuracy",
                        "score": 1,
                        "note": "Timing issue note."
                    }
                ],
                "appliedAt": "2026-06-29T10:00:00Z",
                "mode": "skip",
                "aiReview": null
            })
            .to_string(),
        ),
    );

    let preview = state
        .reviewer_generated_review_preview
        .as_ref()
        .expect("finalize response should populate generated review preview");
    assert_eq!(preview.feedback[0].category, "Timestamp Accuracy");
    assert_eq!(preview.cards[0].kind, "CATEGORY FEEDBACK");
    assert_eq!(preview.cards[0].evidence, "Timing issue note.");
    let finalize = state
        .reviewer_finalize_preview
        .as_ref()
        .expect("finalize response should populate a native finalize preview");
    assert_eq!(finalize.session_id, "session-1");
    assert_eq!(finalize.mode, "skip");
    assert_eq!(finalize.applied_at.as_deref(), Some("2026-06-29T10:00:00Z"));
    assert_eq!(finalize.feedback[0].score, "1");
    assert_eq!(finalize.feedback[0].note, "Timing issue note.");
    assert!(state.status.contains("Finalized review session as skip"));
}

#[test]
fn drafting_health_request_and_success_populate_backend_readiness() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::DraftCheckBackendStatus);

    assert!(
        state
            .review_backend_requests
            .iter()
            .any(|request| matches!(request, ReviewBackendRequest::BackendHealth))
    );

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            ReviewBackendRequest::BackendHealth,
            serde_json::json!({
                "ok": true,
                "service": "babel-review-backend",
                "backendVersion": "0.3.9",
                "testMode": false,
                "drafting": {
                    "provider": "openrouter",
                    "model": "openai/gpt-4o-mini",
                    "serviceTier": "auto",
                    "reasoningEffort": "low"
                },
                "openRouterCredits": {
                    "remaining_credits": 8
                }
            })
            .to_string(),
        ),
    );

    let health = state
        .native_capability_health
        .as_ref()
        .expect("backend health should be parsed");
    assert_eq!(health.service, "babel-review-backend");
    assert_eq!(health.backend_version.as_deref(), Some("0.3.9"));
    assert_eq!(
        health.openrouter_status.as_deref(),
        Some("credits remaining: 8")
    );
    assert_eq!(
        health.provider_diagnostics,
        vec![
            "provider: openrouter".to_string(),
            "model: openai/gpt-4o-mini".to_string(),
            "tier: auto".to_string(),
            "reasoning: low".to_string(),
        ]
    );
}

#[test]
fn drafting_response_previews_rows_and_applies_or_skips_individual_edits() {
    let mut state = stable_sample_state();
    let first_segment_id = state.project().unwrap().segments[0].id.clone();
    let second_segment_id = state.project().unwrap().segments[1].id.clone();
    let original_second = state
        .project()
        .unwrap()
        .segment(&second_segment_id)
        .unwrap()
        .text
        .clone();

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            ReviewBackendRequest::DraftGenerate(serde_json::json!({ "jobId": "draft-job" })),
            serde_json::json!({
                "draftRows": [
                    {
                        "rowId": first_segment_id,
                        "rewrittenText": "Preview replacement one.",
                        "status": "rewritten",
                        "warnings": []
                    },
                    {
                        "rowId": second_segment_id,
                        "rewrittenText": "Preview replacement two.",
                        "status": "rewritten",
                        "warnings": ["length_delta"]
                    }
                ],
                "summary": {
                    "totalRows": 2,
                    "rewrittenRows": 2,
                    "unchangedRows": 0,
                    "failedRows": 0,
                    "anomalyCounts": { "length_delta": 1 }
                },
                "generationMeta": {
                    "model": "openai/gpt-4o-mini",
                    "rulePackVersion": "ru-gold-2sp-v1",
                    "generatedAt": "2026-06-29T10:00:00Z"
                }
            })
            .to_string(),
        ),
    );

    assert_eq!(state.draft_row_previews.len(), 2);
    assert_eq!(state.draft_row_previews[0].row_id, first_segment_id);
    assert_eq!(
        state.draft_row_previews[0].rewritten_text,
        "Preview replacement one."
    );
    assert_eq!(state.draft_row_previews[1].warnings, vec!["length_delta"]);
    assert_eq!(
        state.draft_summary_preview.as_ref().unwrap().rewritten_rows,
        2
    );

    let _ = update(&mut state, Message::DraftSkipRow(second_segment_id.clone()));
    let _ = update(&mut state, Message::DraftApplyRow(first_segment_id.clone()));

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        "Preview replacement one."
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&second_segment_id)
            .unwrap()
            .text,
        original_second
    );
    assert!(state.draft_row_previews[0].applied);
    assert!(state.draft_row_previews[1].skipped);
    assert!(state.can_undo());
}

#[test]
fn accepted_redistribution_preview_applies_draft_allocations_with_undo() {
    let mut state = stable_sample_state();
    let first_segment_id = state.project().unwrap().segments[0].id.clone();
    let second_segment_id = state.project().unwrap().segments[1].id.clone();
    let original_first = state
        .project()
        .unwrap()
        .segment(&first_segment_id)
        .unwrap()
        .text
        .clone();

    let request = ReviewBackendRequest::DraftBrokerRedistributeText(serde_json::json!({
        "groups": [
            {
                "groupId": "speaker-group-1",
                "speakerKey": "speaker-1",
                "segments": [
                    { "id": first_segment_id, "index": 1, "speakerKey": "speaker-1", "startSeconds": 0.0, "endSeconds": 1.0, "text": original_first },
                    { "id": second_segment_id, "index": 2, "speakerKey": "speaker-1", "startSeconds": 1.0, "endSeconds": 2.0, "text": "two" }
                ],
                "fullText": "one two",
                "draftAllocations": [
                    { "segmentId": first_segment_id, "text": "Redistributed first." },
                    { "segmentId": second_segment_id, "text": "Redistributed second." }
                ]
            }
        ]
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "model": "openai/gpt-4o-mini",
                "results": [
                    {
                        "groupId": "speaker-group-1",
                        "ok": true,
                        "model": "openai/gpt-4o-mini",
                        "review": {
                            "acceptDraft": true,
                            "moves": [],
                            "notes": "Looks balanced."
                        }
                    }
                ]
            })
            .to_string(),
        ),
    );

    assert_eq!(state.draft_redistribution_previews.len(), 1);
    assert!(state.draft_redistribution_previews[0].accept_draft);
    assert_eq!(state.draft_redistribution_previews[0].allocations.len(), 2);

    let _ = update(
        &mut state,
        Message::DraftApplyRedistributionGroup("speaker-group-1".to_string()),
    );

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        "Redistributed first."
    );
    assert!(state.can_undo());

    let _ = update(&mut state, Message::Undo);

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        original_first
    );
}

#[test]
fn malformed_redistribution_move_surfaces_validation_copy_and_blocks_apply() {
    let mut state = stable_sample_state();
    let first_segment_id = state.project().unwrap().segments[0].id.clone();
    let second_segment_id = state.project().unwrap().segments[1].id.clone();
    let request = ReviewBackendRequest::DraftBrokerRedistributeText(serde_json::json!({
        "groups": [
            {
                "groupId": "speaker-group-1",
                "speakerKey": "speaker-1",
                "draftAllocations": [
                    { "segmentId": first_segment_id, "text": "Alpha. Beta." },
                    { "segmentId": second_segment_id, "text": "Gamma." }
                ]
            }
        ]
    }));

    let _ = update(
        &mut state,
        Message::ReviewBackendRequestSucceeded(
            request,
            serde_json::json!({
                "results": [
                    {
                        "groupId": "speaker-group-1",
                        "ok": true,
                        "review": {
                            "acceptDraft": false,
                            "moves": [
                                { "fromIndex": 1, "sentenceCount": 1 }
                            ],
                            "notes": "Move one sentence, but the model omitted the target."
                        }
                    }
                ]
            })
            .to_string(),
        ),
    );

    let preview = state
        .draft_redistribution_previews
        .first()
        .expect("redistribution response should populate preview");
    assert_eq!(preview.moves.len(), 0);
    assert_eq!(
        preview.error.as_deref(),
        Some("Invalid redistribution move 1: missing `toIndex`.")
    );

    let _ = update(
        &mut state,
        Message::DraftApplyRedistributionGroup("speaker-group-1".to_string()),
    );

    assert_eq!(
        state.status,
        "Apply redistribution blocked: Invalid redistribution move 1: missing `toIndex`."
    );
}

#[test]
fn model_move_redistribution_preview_applies_adjacent_sentence_moves_with_undo() {
    let mut state = stable_sample_state();
    let first_segment_id = state.project().unwrap().segments[0].id.clone();
    let second_segment_id = state.project().unwrap().segments[1].id.clone();
    let original_first = state
        .project()
        .unwrap()
        .segment(&first_segment_id)
        .unwrap()
        .text
        .clone();
    let original_second = state
        .project()
        .unwrap()
        .segment(&second_segment_id)
        .unwrap()
        .text
        .clone();
    state.draft_redistribution_previews = vec![DraftRedistributionPreview {
        group_id: "speaker-group-1".to_string(),
        model: Some("openai/gpt-4o-mini".to_string()),
        accept_draft: false,
        notes: Some("Move the last sentence down.".to_string()),
        moves: vec![DraftRedistributionMovePreview {
            from_index: 1,
            to_index: 2,
            sentence_count: 1,
        }],
        allocations: vec![
            DraftRedistributionAllocationPreview {
                segment_id: first_segment_id.clone(),
                text: "Alpha. Beta.".to_string(),
            },
            DraftRedistributionAllocationPreview {
                segment_id: second_segment_id.clone(),
                text: "Gamma.".to_string(),
            },
        ],
        error: None,
        applied: false,
    }];

    let _ = update(
        &mut state,
        Message::DraftApplyRedistributionGroup("speaker-group-1".to_string()),
    );

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        "Alpha."
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&second_segment_id)
            .unwrap()
            .text,
        "Beta. Gamma."
    );
    assert!(state.draft_redistribution_previews[0].applied);
    assert!(state.can_undo());

    let _ = update(&mut state, Message::Undo);

    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&first_segment_id)
            .unwrap()
            .text,
        original_first
    );
    assert_eq!(
        state
            .project()
            .unwrap()
            .segment(&second_segment_id)
            .unwrap()
            .text,
        original_second
    );
}

#[test]
fn native_auth_flow_blocks_service_dispatch() {
    let mut state = stable_sample_state();
    apply_startup_route_config(
        &mut state,
        StartupRouteConfig {
            review_action_id: Some("review-action-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );
    state.native_auth_flow = Some(NativeAuthFlowState::pending_code(
        "si_native".to_string(),
        Some("phone_native".to_string()),
    ));

    let _ = state.dispatch_new_babel_service_requests();

    assert!(state.dispatched_babel_service_requests.is_empty());
    assert!(
        state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadReviewAction(_)))
    );
}

#[test]
fn dashboard_project_refresh_dispatches_worker_get_projects_callback() {
    let payload = serde_json::json!([
        {
            "id": "project-1",
            "name": "Gold Transcription",
            "chunkedQueueId": "chunked-queue-1",
            "stitchedTxQueueId": "stitched-queue-1"
        }
    ])
    .to_string();

    assert_eq!(
        fixture_payload_message_for_request(
            BabelServiceRequest::LoadWorkerProjects,
            payload.clone()
        ),
        Some(Message::DashboardProjectsLoaded(payload))
    );

    let mut state = stable_sample_state();
    state.queue_dashboard_projects_refresh();
    assert!(
        state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::LoadWorkerProjects))
    );

    let _ = update(
        &mut state,
        Message::DashboardProjectsLoaded(
            serde_json::json!([
                {
                    "projectId": "0bf29f0c-371a-506a-8b77-5fffd14e5762",
                    "name": "RU-tx-gold",
                    "permissionStatus": "active",
                    "status": "active"
                },
                {
                    "projectId": "c07a40d3-61e4-4fc6-ace6-2b79b5e66575",
                    "name": "Russian-Podcast",
                    "permissionStatus": "active",
                    "status": "completed"
                }
            ])
            .to_string(),
        ),
    );

    assert_eq!(state.dashboard.projects.len(), 1);
    assert_eq!(
        state.dashboard.projects[0].name,
        "Russian Transcription - Gold"
    );
    assert_eq!(
        state.dashboard.projects[0].chunked_queue_id,
        "f95babcc-8681-521c-a39f-66454a16347f"
    );
    assert!(!state.dashboard.is_pending);
}

#[test]
fn dashboard_project_refresh_merges_discovered_queue_config_for_new_active_projects() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::DashboardProjectsLoaded(
            serde_json::json!({
                "projects": [
                    {
                        "projectId": "1c97f55a-3868-4861-9778-d39ee80cda3d",
                        "name": "RU-tx-gold-dg",
                        "permissionStatus": "active",
                        "status": "active"
                    }
                ],
                "projectConfigs": [
                    {
                        "projectId": "1c97f55a-3868-4861-9778-d39ee80cda3d",
                        "chunkedQueueId": "dg-chunked-queue",
                        "stitchedTxQueueId": "dg-stitched-queue"
                    }
                ]
            })
            .to_string(),
        ),
    );

    assert_eq!(state.dashboard.projects.len(), 1);
    assert_eq!(state.dashboard.projects[0].name, "RU-tx-gold-dg");
    assert_eq!(
        state.dashboard.projects[0].chunked_queue_id,
        "dg-chunked-queue"
    );
    assert_eq!(
        state.dashboard.projects[0].stitched_queue_id,
        "dg-stitched-queue"
    );
    assert_eq!(state.status, "Loaded 1 Babel project.");
}

#[test]
fn fixture_service_executor_maps_recovered_load_and_claim_requests_to_callbacks() {
    let payload = "{\"ok\":true}".to_string();

    assert_eq!(
        fixture_payload_message_for_request(
            BabelServiceRequest::LoadReviewAction(LoadReviewActionRequest {
                review_action_id: "review-action-url".to_string(),
            }),
            payload.clone(),
        ),
        Some(Message::LoadReviewActionSucceeded(payload.clone()))
    );
    assert_eq!(
        fixture_payload_message_for_request(
            BabelServiceRequest::LoadLatestReviewAction(LoadLatestReviewActionRequest {
                processed_transcription_id: "processed-url".to_string(),
                queue_id: "stitched-queue-url".to_string(),
            }),
            payload.clone(),
        ),
        Some(Message::LoadLatestReviewActionSucceeded(payload.clone()))
    );
    assert_eq!(
        fixture_payload_message_for_request(
            BabelServiceRequest::ClaimByProcessedTranscription(
                ClaimByProcessedTranscriptionRequest {
                    processed_transcription_id: "processed-url".to_string(),
                    queue_id: "stitched-queue-url".to_string(),
                },
            ),
            payload.clone(),
        ),
        Some(Message::ClaimByProcessedTranscriptionSucceeded(
            payload.clone()
        ))
    );
    assert_eq!(
        fixture_payload_message_for_request(
            BabelServiceRequest::ClaimNextFromQueue(ClaimNextFromQueueRequest {
                queue_id: "chunked-queue-url".to_string(),
            }),
            payload.clone(),
        ),
        Some(Message::ClaimNextFromQueueSucceeded(payload))
    );
}

#[test]
fn fixture_service_executor_dispatches_and_maps_recovered_mutation_requests_to_callbacks() {
    let mutation_requests = [
        BabelServiceRequest::SubmitReviewAction(SubmitReviewActionRequest {
            review_action_id: "review-action-42".to_string(),
            annotations: vec![serde_json::json!({ "id": "annotation-1" })],
            is_stitched_queue: false,
        }),
        BabelServiceRequest::DropWorkerAction(DropWorkerActionRequest {
            queue_id: "chunked-queue-url".to_string(),
        }),
        BabelServiceRequest::SkipReviewAction(SkipReviewActionRequest {
            review_action_id: "review-action-42".to_string(),
        }),
        BabelServiceRequest::AdminRateReviewAction(AdminRateReviewActionRequest {
            review_action_id: "review-action-42".to_string(),
            rating: 1,
            rating_reason: Some("wrong emotion".to_string()),
        }),
        BabelServiceRequest::SaveFeedbackDraft(FeedbackDraftSaveRequest {
            form_step_response_id: "form-step-response-1".to_string(),
            input_responses: vec![FeedbackInputResponse {
                form_input_id: "comment".to_string(),
                value: "Looks good".to_string(),
            }],
        }),
    ];
    assert!(
        mutation_requests
            .iter()
            .all(is_dispatchable_babel_service_request)
    );

    let submit_request = mutation_requests[0].clone();
    assert_eq!(
        fixture_payload_message_for_request(
            submit_request.clone(),
            r#"{"success":true,"linterErrors":[]}"#.to_string()
        ),
        Some(Message::SubmitReviewSucceeded)
    );
    assert_eq!(
        fixture_payload_message_for_request(
            submit_request,
            r#"{"success":true,"linterErrors":[{"id":"empty-content"}]}"#.to_string()
        ),
        Some(Message::SubmitReviewFailed(
            "Failed to pass linting. Please fix the errors and try again.".to_string()
        ))
    );
    assert_eq!(
        fixture_payload_message_for_request(
            mutation_requests[1].clone(),
            r#"{"success":true}"#.to_string()
        ),
        Some(Message::DropClaimSucceeded)
    );
    assert_eq!(
        fixture_payload_message_for_request(
            mutation_requests[2].clone(),
            r#"{"success":false,"message":"daily skip cap reached"}"#.to_string()
        ),
        Some(Message::SkipChunkFailed(
            "daily skip cap reached".to_string()
        ))
    );
    assert_eq!(
        fixture_payload_message_for_request(
            mutation_requests[3].clone(),
            r#"{"success":true}"#.to_string()
        ),
        Some(Message::AdminFlagSucceeded)
    );
    assert_eq!(
        fixture_payload_message_for_request(
            mutation_requests[4].clone(),
            r#"{"success":true,"updatedAt":"2026-06-23T10:00:00Z"}"#.to_string()
        ),
        Some(Message::FeedbackDraftSaved(
            "2026-06-23T10:00:00Z".to_string()
        ))
    );
    assert_eq!(
        fixture_payload_message_for_request(
            mutation_requests[4].clone(),
            r#"{"success":false,"error":"draft rejected"}"#.to_string()
        ),
        Some(Message::FeedbackDraftSaveFailed(
            "draft rejected".to_string()
        ))
    );
}

#[test]
fn service_command_envelopes_use_recovered_trpc_operation_names_and_payload_keys() {
    assert_eq!(
        request_envelope_for_service_command(&BabelServiceRequest::ClaimNextFromQueue(
            ClaimNextFromQueueRequest {
                queue_id: "chunked-queue-url".to_string(),
            },
        )),
        serde_json::json!({
            "operation": "transcriptions.claimNextReviewActionFromReviewQueue",
            "payload": { "queueId": "chunked-queue-url" },
        })
    );
    assert_eq!(
        request_envelope_for_service_command(&BabelServiceRequest::LoadLatestReviewAction(
            LoadLatestReviewActionRequest {
                processed_transcription_id: "processed-url".to_string(),
                queue_id: "stitched-queue-url".to_string(),
            },
        )),
        serde_json::json!({
            "operation": "transcriptions.getLatestReviewActionByProcessedTranscriptionId",
            "payload": {
                "processedTranscriptionId": "processed-url",
                "queueId": "stitched-queue-url",
            },
        })
    );
    assert_eq!(
        request_envelope_for_service_command(&BabelServiceRequest::SubmitReviewAction(
            SubmitReviewActionRequest {
                review_action_id: "review-action-42".to_string(),
                annotations: vec![serde_json::json!({ "id": "annotation-1" })],
                is_stitched_queue: true,
            },
        )),
        serde_json::json!({
            "operation": "transcriptions.submitTranscriptReviewAction",
            "payload": {
                "reviewActionId": "review-action-42",
                "annotations": [{ "id": "annotation-1" }],
                "isStitchedQueue": true,
            },
        })
    );
    assert_eq!(
        request_envelope_for_service_command(&BabelServiceRequest::AdminRateReviewAction(
            AdminRateReviewActionRequest {
                review_action_id: "review-action-42".to_string(),
                rating: 1,
                rating_reason: Some("wrong emotion".to_string()),
            },
        )),
        serde_json::json!({
            "operation": "transcriptions.adminRateTtsTranscription",
            "payload": {
                "reviewActionId": "review-action-42",
                "rating": 1,
                "ratingReason": "wrong emotion",
            },
        })
    );
    assert_eq!(
        request_envelope_for_service_command(&BabelServiceRequest::SaveFeedbackDraft(
            FeedbackDraftSaveRequest {
                form_step_response_id: "form-step-response-1".to_string(),
                input_responses: vec![FeedbackInputResponse {
                    form_input_id: "comment".to_string(),
                    value: "Looks good".to_string(),
                }],
            },
        )),
        serde_json::json!({
            "operation": "transcriptionFeedbackForm.saveDraft",
            "payload": {
                "formStepResponseId": "form-step-response-1",
                "inputResponses": [
                    { "formInputId": "comment", "value": "Looks good" },
                ],
            },
        })
    );
}

#[test]
fn startup_route_args_wire_task_lookup_and_processed_load_like_recovered_query() {
    let config = parse_startup_route_config([
        "studio-app",
        "babel://we-edit?processedTranscriptionId=processed-url&displayFeedback=true&taskLookup=true&stitchedQueueId=stitched-queue-url",
    ]);

    let mut task_lookup_state = stable_sample_state();
    apply_startup_route_config(&mut task_lookup_state, config.clone());

    let lookup = task_lookup_state.task_lookup.as_ref().unwrap();
    assert_eq!(lookup.processed_transcription_id, "processed-url");
    assert_eq!(
        task_lookup_state
            .project()
            .unwrap()
            .metadata
            .get("studio.babel.stitchedQueueId"),
        Some(&"stitched-queue-url".to_string())
    );
    assert!(task_lookup_state.babel_service_requests.is_empty());
    assert_eq!(task_lookup_state.status, "Task lookup route opened.");

    let mut processed_state = stable_sample_state();
    apply_startup_route_config(
        &mut processed_state,
        StartupRouteConfig {
            show_task_lookup_modal: false,
            ..config
        },
    );
    assert_eq!(
        processed_state.babel_service_requests.last(),
        Some(&BabelServiceRequest::LoadLatestReviewAction(
            LoadLatestReviewActionRequest {
                processed_transcription_id: "processed-url".to_string(),
                queue_id: "stitched-queue-url".to_string(),
            }
        ))
    );
    assert_eq!(
        processed_state
            .project()
            .unwrap()
            .metadata
            .get("studio.babel.displayFeedback"),
        Some(&"true".to_string())
    );

    let mut default_claim_state = stable_sample_state();
    apply_startup_route_config(
        &mut default_claim_state,
        StartupRouteConfig {
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );
    assert_eq!(
        default_claim_state.babel_service_requests.last(),
        Some(&BabelServiceRequest::ClaimNextFromQueue(
            ClaimNextFromQueueRequest {
                queue_id: "chunked-queue-url".to_string(),
            }
        ))
    );
}

#[test]
fn task_lookup_modal_wires_recovered_copy_and_trimmed_inputs() {
    let mut state = stable_sample_state();
    open_task_lookup_fixture(&mut state);

    let lookup = state.task_lookup.as_ref().unwrap();
    let model = lookup.select_action_model();
    assert_eq!(model.title, "Select Action");
    assert_eq!(
        model.description,
        "Claim a specific stitched transcription or view a transcription in read-only mode."
    );
    assert!(model.show_processed_transcription_input);

    let _ = update(
        &mut state,
        Message::TaskLookupProcessedTranscriptionChanged(" processed-1 ".to_string()),
    );
    assert_eq!(
        state
            .task_lookup
            .as_ref()
            .unwrap()
            .processed_transcription_id,
        "processed-1"
    );
    assert_eq!(
        state.task_lookup_last_effect,
        Some(TaskLookupEffect::SetProcessedTranscriptionId {
            value: "processed-1".to_string()
        })
    );

    let _ = update(
        &mut state,
        Message::TaskLookupReviewActionChanged(" action-1 ".to_string()),
    );
    assert_eq!(
        state.task_lookup.as_ref().unwrap().review_action_id,
        "action-1"
    );
    assert_eq!(
        state.task_lookup_last_effect,
        Some(TaskLookupEffect::SetReviewActionId {
            value: "action-1".to_string()
        })
    );
}

#[test]
fn task_lookup_modal_claim_and_read_only_effects_follow_recovered_priority() {
    let mut state = stable_sample_state();
    open_task_lookup_fixture(&mut state);
    install_babel_route_metadata(&mut state);
    let _ = update(
        &mut state,
        Message::TaskLookupProcessedTranscriptionChanged("processed-1".to_string()),
    );
    let _ = update(
        &mut state,
        Message::TaskLookupReviewActionChanged("action-1".to_string()),
    );

    let _ = update(&mut state, Message::TaskLookupClickClaimStitched);
    assert!(state.task_lookup.as_ref().unwrap().confirming_edit);
    assert_ne!(
        state.task_lookup_last_effect,
        Some(TaskLookupEffect::ClaimById)
    );

    let _ = update(&mut state, Message::TaskLookupCancelConfirmEdit);
    assert!(!state.task_lookup.as_ref().unwrap().confirming_edit);

    let _ = update(&mut state, Message::TaskLookupClickClaimStitched);
    let _ = update(&mut state, Message::TaskLookupConfirmEdit);
    assert!(!state.task_lookup.as_ref().unwrap().confirming_edit);
    assert_eq!(
        state.task_lookup_last_effect,
        Some(TaskLookupEffect::ClaimById)
    );
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::ClaimByProcessedTranscription(
            ClaimByProcessedTranscriptionRequest {
                processed_transcription_id: "processed-1".to_string(),
                queue_id: "stitched-queue-9".to_string(),
            }
        ))
    );

    let _ = update(&mut state, Message::TaskLookupClickReadOnly);
    assert_eq!(
        state.task_lookup_last_effect,
        Some(TaskLookupEffect::ViewReadOnlyByReviewActionId)
    );
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::LoadReviewAction(
            LoadReviewActionRequest {
                review_action_id: "action-1".to_string(),
            }
        ))
    );

    let _ = update(
        &mut state,
        Message::TaskLookupReviewActionChanged(String::new()),
    );
    let _ = update(&mut state, Message::TaskLookupClickReadOnly);
    assert_eq!(
        state.task_lookup_last_effect,
        Some(TaskLookupEffect::ViewReadOnlyByProcessedTranscriptionId)
    );
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::LoadLatestReviewAction(
            LoadLatestReviewActionRequest {
                processed_transcription_id: "processed-1".to_string(),
                queue_id: "stitched-queue-9".to_string(),
            }
        ))
    );
}

#[test]
fn task_lookup_modal_component_wires_recovered_select_and_confirm_actions() {
    let source = include_str!("components/task_lookup_dialog.rs");
    let model_source = include_str!("../editor_model/task_lookup.rs");

    assert!(source.contains("Select Action"));
    assert!(model_source.contains("Processed Transcription ID (For Stitched Queue)"));
    assert!(model_source.contains("Review Action ID (For Read-Only)"));
    assert!(model_source.contains("Confirm Edit"));
    assert!(source.contains("Message::TaskLookupClickClaimStitched"));
    assert!(source.contains("Message::TaskLookupClickReadOnly"));
}

#[test]
fn no_work_modal_is_non_dismissible_but_back_to_projects_closes_it() {
    let mut state = StudioState {
        no_work_modal: Some(build_no_work_modal_model(NoWorkModalInput {
            open: true,
            mode: "review".to_string(),
        })),
        ..stable_sample_state()
    };
    let project = state.project().unwrap().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);

    let _ = update(&mut state, Message::EscapePressed);

    assert!(state.no_work_modal.is_some());
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::BackToProjects);

    assert!(state.no_work_modal.is_none());
    assert_eq!(state.status, "Back to projects requested.");
}

#[test]
fn default_queue_claim_failure_opens_recovered_no_work_modal() {
    let mut state = stable_sample_state();
    apply_startup_route_config(
        &mut state,
        StartupRouteConfig {
            chunked_queue_id: Some("chunked-queue-url".to_string()),
            ..StartupRouteConfig::default()
        },
    );
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::ClaimNextFromQueue(
            ClaimNextFromQueueRequest {
                queue_id: "chunked-queue-url".to_string(),
            }
        ))
    );

    let _ = update(
        &mut state,
        Message::ClaimNextFromQueueFailed("No tasks available".to_string()),
    );

    assert!(state.babel_service_requests.is_empty());
    let modal = state.no_work_modal.as_ref().unwrap();
    assert!(modal.open);
    assert_eq!(
        modal.title,
        "Sorry, we weren't able to find a review job for you!"
    );
    assert_eq!(state.status, "No tasks available");
}

#[test]
fn no_work_modal_component_wires_recovered_back_to_projects_action() {
    let source = include_str!("components/no_work_dialog.rs");

    assert!(source.contains("NoWorkModalModel"));
    assert!(source.contains("Message::BackToProjects"));
    assert!(!source.contains("Cancel"));
}

#[test]
fn drop_claim_message_records_recovered_action_intent_without_dirtying_editor() {
    let mut state = stable_sample_state();
    install_babel_route_metadata(&mut state);

    let _ = update(&mut state, Message::DropClaim);

    assert!(state.drop_claim_is_pending);
    assert_eq!(state.status, "Dropping claim...");
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::DropWorkerAction(
            DropWorkerActionRequest {
                queue_id: "chunked-queue-7".to_string(),
            }
        ))
    );
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.dropClaim.dropped"),
        None
    );

    let _ = update(&mut state, Message::DropClaimSucceeded);

    assert!(!state.drop_claim_is_pending);
    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::DropWorkerAction(_)))
    );
    assert_eq!(state.status, "Claim dropped.");
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.dropClaim.dropped")
            .map(String::as_str),
        Some("true")
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn skip_chunk_dialog_records_recovered_babel_skip_intent_without_dirtying_editor() {
    let mut state = stable_sample_state();
    install_babel_route_metadata(&mut state);
    enable_optional_babel_review_controls(&mut state);

    let _ = update(&mut state, Message::RequestSkipChunk);
    assert!(state.show_skip_chunk_dialog);

    let _ = update(&mut state, Message::ConfirmSkipChunk);

    assert!(state.show_skip_chunk_dialog);
    assert!(state.skip_is_pending);
    assert_eq!(state.status, "Skipping chunk...");
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::SkipReviewAction(
            SkipReviewActionRequest {
                review_action_id: "review-action-42".to_string(),
            }
        ))
    );
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.skip.requested"),
        None
    );

    let _ = update(&mut state, Message::SkipChunkSucceeded);

    assert!(!state.show_skip_chunk_dialog);
    assert!(!state.skip_is_pending);
    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::SkipReviewAction(_)))
    );
    assert_eq!(state.status, "Skip requested.");
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.skip.requested")
            .map(String::as_str),
        Some("true")
    );
    assert!(!state.editor.as_ref().unwrap().is_dirty);
}

#[test]
fn admin_flag_dialog_records_recovered_babel_flag_payload() {
    let mut state = stable_sample_state();
    install_babel_route_metadata(&mut state);
    enable_optional_babel_review_controls(&mut state);

    let _ = update(&mut state, Message::RequestAdminFlag);
    let _ = update(
        &mut state,
        Message::AdminFlagReasonChanged("background noise".to_string()),
    );
    assert!(state.show_admin_flag_dialog);

    let _ = update(&mut state, Message::ConfirmAdminFlag);

    assert!(state.show_admin_flag_dialog);
    assert!(state.admin_flag_is_pending);
    assert_eq!(state.status, "Flagging recording...");
    assert_eq!(
        state.babel_service_requests.last(),
        Some(&BabelServiceRequest::AdminRateReviewAction(
            AdminRateReviewActionRequest {
                review_action_id: "review-action-42".to_string(),
                rating: 1,
                rating_reason: Some("background noise".to_string()),
            }
        ))
    );
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.adminFlag.rating"),
        None
    );

    let _ = update(&mut state, Message::AdminFlagSucceeded);

    assert!(!state.show_admin_flag_dialog);
    assert!(!state.admin_flag_is_pending);
    assert!(
        !state
            .babel_service_requests
            .iter()
            .any(|request| matches!(request, BabelServiceRequest::AdminRateReviewAction(_)))
    );
    assert_eq!(state.status, "Recording flagged.");
    let metadata = &state.workflow.as_ref().unwrap().project().metadata;
    assert_eq!(
        metadata
            .get("studio.babel.adminFlag.rating")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        metadata
            .get("studio.babel.adminFlag.reason")
            .map(String::as_str),
        Some("background noise")
    );
}

#[test]
fn admin_flag_close_paths_match_recovered_babel_reason_lifecycle() {
    let mut cancel_state = stable_sample_state();
    enable_optional_babel_review_controls(&mut cancel_state);
    let _ = update(&mut cancel_state, Message::RequestAdminFlag);
    let _ = update(
        &mut cancel_state,
        Message::AdminFlagReasonChanged("  wrong emotion  ".to_string()),
    );

    let _ = update(&mut cancel_state, Message::CancelAdminFlag);

    assert!(!cancel_state.show_admin_flag_dialog);
    assert_eq!(cancel_state.admin_flag_reason, "  wrong emotion  ");

    let mut escape_state = stable_sample_state();
    enable_optional_babel_review_controls(&mut escape_state);
    let _ = update(&mut escape_state, Message::RequestAdminFlag);
    let _ = update(
        &mut escape_state,
        Message::AdminFlagReasonChanged("  poor audio  ".to_string()),
    );

    let _ = update(&mut escape_state, Message::EscapePressed);

    assert!(!escape_state.show_admin_flag_dialog);
    assert_eq!(escape_state.admin_flag_reason, "");

    let mut pending_state = stable_sample_state();
    enable_optional_babel_review_controls(&mut pending_state);
    let _ = update(&mut pending_state, Message::RequestAdminFlag);
    let _ = update(
        &mut pending_state,
        Message::AdminFlagReasonChanged("  clipping  ".to_string()),
    );
    let _ = update(&mut pending_state, Message::ConfirmAdminFlag);

    let _ = update(&mut pending_state, Message::EscapePressed);

    assert!(pending_state.show_admin_flag_dialog);
    assert!(pending_state.admin_flag_is_pending);
    assert_eq!(pending_state.admin_flag_reason, "  clipping  ");
}

#[test]
fn submit_control_pending_dialog_inputs_match_recovered_disabled_noops() {
    let mut admin_state = stable_sample_state();
    install_babel_route_metadata(&mut admin_state);
    enable_optional_babel_review_controls(&mut admin_state);
    let _ = update(&mut admin_state, Message::RequestAdminFlag);
    let _ = update(
        &mut admin_state,
        Message::AdminFlagReasonChanged("  wrong emotion  ".to_string()),
    );
    let _ = update(&mut admin_state, Message::ConfirmAdminFlag);
    let admin_request_count = admin_state
        .babel_service_requests
        .iter()
        .filter(|request| matches!(request, BabelServiceRequest::AdminRateReviewAction(_)))
        .count();

    let _ = update(
        &mut admin_state,
        Message::AdminFlagReasonChanged("mutated while pending".to_string()),
    );
    let _ = update(&mut admin_state, Message::CancelAdminFlag);
    let _ = update(&mut admin_state, Message::ConfirmAdminFlag);

    assert!(admin_state.show_admin_flag_dialog);
    assert!(admin_state.admin_flag_is_pending);
    assert_eq!(admin_state.admin_flag_reason, "  wrong emotion  ");
    assert_eq!(
        admin_state
            .babel_service_requests
            .iter()
            .filter(|request| matches!(request, BabelServiceRequest::AdminRateReviewAction(_)))
            .count(),
        admin_request_count
    );

    let mut skip_state = stable_sample_state();
    install_babel_route_metadata(&mut skip_state);
    enable_optional_babel_review_controls(&mut skip_state);
    let _ = update(&mut skip_state, Message::RequestSkipChunk);
    let _ = update(&mut skip_state, Message::ConfirmSkipChunk);
    let skip_request_count = skip_state
        .babel_service_requests
        .iter()
        .filter(|request| matches!(request, BabelServiceRequest::SkipReviewAction(_)))
        .count();

    let _ = update(&mut skip_state, Message::CancelSkipChunk);
    let _ = update(&mut skip_state, Message::ConfirmSkipChunk);
    let _ = update(&mut skip_state, Message::RequestSkipChunk);

    assert!(skip_state.show_skip_chunk_dialog);
    assert!(skip_state.skip_is_pending);
    assert_eq!(
        skip_state
            .babel_service_requests
            .iter()
            .filter(|request| matches!(request, BabelServiceRequest::SkipReviewAction(_)))
            .count(),
        skip_request_count
    );
}

#[test]
fn submit_review_emits_recovered_babel_service_payload_without_auto_success() {
    let mut state = stable_sample_state();
    install_babel_route_metadata(&mut state);
    acknowledge_all_assertable_warnings(&mut state);

    let _ = update(&mut state, Message::RequestSubmitReview);
    let _ = update(&mut state, Message::SubmitReview);

    assert!(state.show_submit_dialog);
    assert!(state.submit_is_pending);
    assert_eq!(state.status, "Submitting review...");
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("babel.review_action.actionDecision"),
        None
    );
    match state.babel_service_requests.last() {
        Some(BabelServiceRequest::SubmitReviewAction(SubmitReviewActionRequest {
            review_action_id,
            annotations,
            is_stitched_queue,
        })) => {
            assert_eq!(review_action_id, "review-action-42");
            assert!(!annotations.is_empty());
            assert!(!is_stitched_queue);
        }
        other => panic!("expected submit review action request, got {other:?}"),
    }
}

#[test]
fn default_action_confirms_skip_and_admin_dialogs_with_pending_guards() {
    let mut state = stable_sample_state();
    enable_optional_babel_review_controls(&mut state);
    let _ = update(&mut state, Message::RequestSkipChunk);

    let _ = update(&mut state, Message::ActivateDefaultSurfaceAction);

    assert!(state.show_skip_chunk_dialog);
    assert!(state.skip_is_pending);
    assert_eq!(state.status, "Skipping chunk...");

    let _ = update(&mut state, Message::SkipChunkSucceeded);

    assert!(!state.show_skip_chunk_dialog);
    assert!(!state.skip_is_pending);
    assert_eq!(state.status, "Skip requested.");

    let _ = update(&mut state, Message::RequestAdminFlag);
    let _ = update(
        &mut state,
        Message::AdminFlagReasonChanged("  wrong emotion  ".to_string()),
    );

    let _ = update(&mut state, Message::ActivateDefaultSurfaceAction);

    assert!(state.show_admin_flag_dialog);
    assert!(state.admin_flag_is_pending);
    assert_eq!(state.status, "Flagging recording...");

    let _ = update(&mut state, Message::AdminFlagSucceeded);

    assert!(!state.show_admin_flag_dialog);
    assert!(!state.admin_flag_is_pending);
    assert_eq!(state.status, "Recording flagged.");

    let mut pending_state = stable_sample_state();
    enable_optional_babel_review_controls(&mut pending_state);
    let _ = update(&mut pending_state, Message::RequestSkipChunk);
    pending_state.skip_is_pending = true;
    let _ = update(&mut pending_state, Message::ActivateDefaultSurfaceAction);
    assert!(pending_state.show_skip_chunk_dialog);
    assert!(pending_state.skip_is_pending);

    let _ = update(&mut pending_state, Message::RequestAdminFlag);
    pending_state.admin_flag_is_pending = true;
    let _ = update(&mut pending_state, Message::ActivateDefaultSurfaceAction);
    assert!(pending_state.show_admin_flag_dialog);
    assert!(pending_state.admin_flag_is_pending);
}

#[test]
fn review_action_failures_keep_babel_dialog_state_for_retry() {
    let mut submit_state = stable_sample_state();
    acknowledge_all_assertable_warnings(&mut submit_state);
    let _ = update(&mut submit_state, Message::RequestSubmitReview);
    let _ = update(&mut submit_state, Message::SubmitReview);

    let _ = update(
        &mut submit_state,
        Message::SubmitReviewFailed("network unavailable".to_string()),
    );

    assert!(submit_state.show_submit_dialog);
    assert!(!submit_state.submit_is_pending);
    assert!(submit_state.submit_has_error);
    assert_eq!(submit_state.status, "Submit failed: network unavailable");
    let submit = submit_state.editor.as_ref().unwrap().submit_control(
        submit_state.project().unwrap(),
        submit_state.show_submit_dialog,
        submit_state.submit_is_pending,
        submit_state.submit_has_error,
    );
    assert_eq!(
        submit.dialog_description,
        "Failed to submit annotations. Please try again."
    );

    let mut skip_state = stable_sample_state();
    enable_optional_babel_review_controls(&mut skip_state);
    let _ = update(&mut skip_state, Message::RequestSkipChunk);
    let _ = update(&mut skip_state, Message::ConfirmSkipChunk);

    let _ = update(
        &mut skip_state,
        Message::SkipChunkFailed("daily skip cap reached".to_string()),
    );

    assert!(skip_state.show_skip_chunk_dialog);
    assert!(!skip_state.skip_is_pending);
    assert_eq!(skip_state.status, "Skip failed: daily skip cap reached");
    assert_eq!(
        skip_state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.skip.requested"),
        None
    );

    let mut admin_state = stable_sample_state();
    enable_optional_babel_review_controls(&mut admin_state);
    let _ = update(&mut admin_state, Message::RequestAdminFlag);
    let _ = update(
        &mut admin_state,
        Message::AdminFlagReasonChanged("  wrong script  ".to_string()),
    );
    let _ = update(&mut admin_state, Message::ConfirmAdminFlag);

    let _ = update(
        &mut admin_state,
        Message::AdminFlagFailed("admin endpoint rejected request".to_string()),
    );

    assert!(admin_state.show_admin_flag_dialog);
    assert!(!admin_state.admin_flag_is_pending);
    assert_eq!(
        admin_state.status,
        "Flag recording failed: admin endpoint rejected request"
    );
    assert_eq!(
        admin_state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.adminFlag.rating"),
        None
    );

    let mut drop_state = stable_sample_state();
    let _ = update(&mut drop_state, Message::DropClaim);

    let _ = update(
        &mut drop_state,
        Message::DropClaimFailed("claim already released".to_string()),
    );

    assert!(!drop_state.drop_claim_is_pending);
    assert_eq!(
        drop_state.status,
        "Drop claim failed: claim already released"
    );
    assert_eq!(
        drop_state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .metadata
            .get("studio.babel.dropClaim.dropped"),
        None
    );
}

#[test]
fn submit_flushes_pending_hidden_draft_and_blocks_lint_even_when_filtered() {
    let mut state = stable_sample_state();
    state.editor.as_mut().unwrap().track_filter =
        studio_app::TrackFilter::lane("track-speaker-2", "Speaker 2");

    state
        .segment_drafts
        .get_mut("annotation-1")
        .unwrap()
        .replace_text(String::new());
    state.mark_dirty();
    state.submit_review();

    assert_eq!(
        state.editor.as_ref().unwrap().review_state,
        ReviewState::Draft
    );
    assert!(state.status.contains("blocked"));
    assert_eq!(
        state
            .workflow
            .as_ref()
            .unwrap()
            .project()
            .segment("annotation-1")
            .unwrap()
            .text,
        ""
    );
}

#[test]
fn transport_track_filter_options_use_recovered_babel_lane_labels() {
    let mut state = stable_sample_state();
    let mut project = state.workflow.as_ref().unwrap().project().clone();
    project.lanes[1].label = "Host".to_string();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "loaded relabeled project".to_string(),
    );

    let project = state.workflow.as_ref().unwrap().project();
    let strip = state.editor.as_ref().unwrap().transport_strip(project);

    assert!(
        strip
            .track_filter_options
            .contains(&studio_app::TrackFilter::lane("track-speaker-2", "Host"))
    );
    assert!(
        !strip
            .track_filter_options
            .iter()
            .any(|option| option.label() == "Speaker 2")
    );
}

#[test]
fn deleting_selected_segment_clears_removed_action_state() {
    let mut state = stable_sample_state();
    state.select_segment("annotation-1");
    state.editor.as_mut().unwrap().open_actions_segment_id = Some("annotation-1".to_string());

    state.delete_segment("annotation-1");

    let editor = state.editor.as_ref().unwrap();
    assert_ne!(editor.selected_segment_id.as_deref(), Some("annotation-1"));
    assert!(editor.open_actions_segment_id.is_none());
}

#[test]
fn row_structure_action_create_closes_menu_and_selects_created_segment_like_browser_dropdown() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    state.with_editor_mut(|editor| {
        editor.open_actions_segment_id = Some("annotation-3".to_string());
        editor.viewport = TimeRange {
            start_seconds: 10.0,
            end_seconds: 15.0,
        };
        editor.playhead_seconds = 80.0;
    });
    let before_viewport = state.editor.as_ref().unwrap().viewport;

    let _ = update(
        &mut state,
        Message::CreateSegmentBefore("annotation-3".to_string()),
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count + 1
    );
    assert_eq!(
        editor.selected_segment_id.as_deref(),
        Some("annotation-3__studio_1")
    );
    assert!(editor.open_actions_segment_id.is_none());
    assert!(editor.is_dirty);
    assert_eq!(editor.playhead_seconds, 80.0);
    assert_eq!(editor.viewport, before_viewport);
}

#[test]
fn row_structure_action_merge_closes_menu_and_dirties_task_like_browser_dropdown() {
    let mut state = stable_sample_state();
    let original_count = state.workflow.as_ref().unwrap().project().segments.len();
    state.editor.as_mut().unwrap().open_actions_segment_id = Some("annotation-1".to_string());

    let _ = update(
        &mut state,
        Message::MergeSegmentBelow("annotation-1".to_string()),
    );

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(
        state.workflow.as_ref().unwrap().project().segments.len(),
        original_count - 1
    );
    assert!(editor.open_actions_segment_id.is_none());
    assert!(editor.is_dirty);
}

#[test]
fn hotkey_mapping_covers_transport_navigation_and_workflow_actions() {
    assert_eq!(
        hotkey_message(Key::Named(Named::Space), Modifiers::empty()),
        Some(Message::TogglePlayback)
    );
    assert_eq!(
        hotkey_message(Key::Named(Named::ArrowRight), Modifiers::empty()),
        Some(Message::NextSegment)
    );
    assert_eq!(
        hotkey_message(Key::Character("s".into()), Modifiers::COMMAND),
        None
    );
    assert_eq!(
        hotkey_message(Key::Character("e".into()), Modifiers::COMMAND),
        None
    );
    assert_eq!(
        hotkey_message(
            Key::Character("z".into()),
            Modifiers::COMMAND | Modifiers::SHIFT,
        ),
        Some(Message::Redo)
    );
    assert_eq!(
        hotkey_message(Key::Character("l".into()), Modifiers::empty()),
        Some(Message::ToggleLoop)
    );
    assert_eq!(
        hotkey_message(Key::Character("t".into()), Modifiers::empty()),
        Some(Message::ToggleOverlay)
    );
    assert_eq!(
        hotkey_message(Key::Character("j".into()), Modifiers::COMMAND),
        Some(Message::ToggleActiveWarningAssertion)
    );
    assert_eq!(
        hotkey_message(Key::Named(Named::Undo), Modifiers::empty()),
        Some(Message::Undo)
    );
    assert_eq!(
        hotkey_message(Key::Named(Named::Redo), Modifiers::empty()),
        Some(Message::Redo)
    );
    assert_eq!(
        hotkey_message(Key::Character("q".into()), Modifiers::empty()),
        Some(Message::AuditionKeyChanged(AuditionKey::BeforeStart, true))
    );
    assert_eq!(
        hotkey_message(Key::Character("w".into()), Modifiers::empty()),
        Some(Message::AuditionKeyChanged(AuditionKey::AfterStart, true))
    );
    assert_eq!(
        hotkey_message(Key::Character("e".into()), Modifiers::empty()),
        Some(Message::AuditionKeyChanged(AuditionKey::BeforeEnd, true))
    );
    assert_eq!(
        hotkey_message(Key::Character("r".into()), Modifiers::empty()),
        Some(Message::AuditionKeyChanged(AuditionKey::AfterEnd, true))
    );
    assert_eq!(
        hotkey_message(Key::Named(Named::Escape), Modifiers::empty()),
        Some(Message::EscapePressed)
    );
    assert_eq!(
        hotkey_message(Key::Named(Named::Enter), Modifiers::empty()),
        Some(Message::ActivateDefaultSurfaceAction)
    );
}

#[test]
fn physical_audition_release_tokens_ignore_unrelated_key_releases() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::AuditionKeyPressed {
            key: AuditionKey::BeforeStart,
            release_token: "Character(\"'\")".to_string(),
        },
    );
    let _ = update(
        &mut state,
        Message::AuditionKeyReleaseToken("Character(\"a\")".to_string()),
    );
    assert!(state.held_audition_keys.contains(&AuditionKey::BeforeStart));

    let _ = update(
        &mut state,
        Message::AuditionKeyReleaseToken("Character(\"'\")".to_string()),
    );

    assert!(state.held_audition_keys.is_empty());
}

#[test]
fn clearing_audition_keys_removes_all_physical_holds_and_release_tokens() {
    let mut state = stable_sample_state();

    let _ = update(
        &mut state,
        Message::AuditionKeyPressed {
            key: AuditionKey::BeforeStart,
            release_token: "Character(\"'\")".to_string(),
        },
    );
    let _ = update(
        &mut state,
        Message::AuditionKeyPressed {
            key: AuditionKey::AfterEnd,
            release_token: "Character(\"r\")".to_string(),
        },
    );
    assert_eq!(
        state.held_audition_keys,
        BTreeSet::from([AuditionKey::BeforeStart, AuditionKey::AfterEnd])
    );

    let _ = update(&mut state, Message::ClearAuditionKeys);
    let _ = update(
        &mut state,
        Message::AuditionKeyReleaseToken("Character(\"'\")".to_string()),
    );

    assert!(state.held_audition_keys.is_empty());
}

#[test]
fn escape_routes_to_create_region_dialog_before_timeline_loop() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(&mut state, Message::EscapePressed);

    assert!(state.create_region_dialog.is_none());
    assert!(state.editor.as_ref().unwrap().loop_enabled);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);

    assert!(state.editor.as_ref().unwrap().loop_enabled);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_none());
    assert!(
        !state
            .editor
            .as_ref()
            .unwrap()
            .loop_strip(&project)
            .has_range
    );
}

#[test]
fn escape_dismisses_babel_surface_stack_before_timeline_loop() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);
    state.show_submit_dialog = true;
    state.show_skip_chunk_dialog = true;
    state.show_admin_flag_dialog = true;
    state.show_hotkeys_dialog = true;
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);
    let _ = update(
        &mut state,
        Message::RequestEditAnnotation("annotation-1".to_string()),
    );

    let _ = update(&mut state, Message::EscapePressed);
    assert!(state.edit_annotation_dialog.is_none());
    assert!(state.create_region_dialog.is_some());
    assert!(state.show_hotkeys_dialog);
    assert!(state.show_admin_flag_dialog);
    assert!(state.show_skip_chunk_dialog);
    assert!(state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);
    assert!(state.create_region_dialog.is_none());
    assert!(state.show_hotkeys_dialog);
    assert!(state.show_admin_flag_dialog);
    assert!(state.show_skip_chunk_dialog);
    assert!(state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);
    assert!(!state.show_hotkeys_dialog);
    assert!(state.show_admin_flag_dialog);
    assert!(state.show_skip_chunk_dialog);
    assert!(state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);
    assert!(!state.show_admin_flag_dialog);
    assert!(state.show_skip_chunk_dialog);
    assert!(state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);
    assert!(!state.show_skip_chunk_dialog);
    assert!(state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);
    assert!(!state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    let _ = update(&mut state, Message::EscapePressed);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_none());
}

#[test]
fn escape_closes_native_capability_modal_before_timeline_loop() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);
    state.native_capability_panel_open = true;

    let _ = update(&mut state, Message::EscapePressed);

    assert!(!state.native_capability_panel_open);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());
}

#[test]
fn escape_closes_browser_find_before_page_drawers_like_native_browser_chrome() {
    let mut state = stable_sample_state();
    enable_feedback_panel_for_word_accuracy(&mut state);

    let _ = update(&mut state, Message::OpenFeedbackPanel);
    let _ = update(&mut state, Message::OpenFind);
    assert!(state.find_bar_open);
    assert!(
        state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.open)
    );

    let _ = update(&mut state, Message::EscapePressed);

    assert!(!state.find_bar_open);
    assert!(
        state
            .feedback_panel
            .as_ref()
            .is_some_and(|panel| panel.open)
    );
}

#[test]
fn escape_consumes_pending_babel_dialogs_without_clearing_loop_behind_them() {
    let mut state = stable_sample_state();
    let project = state.project().unwrap().clone();
    state
        .editor
        .as_mut()
        .unwrap()
        .set_loop_range_from_drag(&project, 3.5, 5.1);
    state.show_submit_dialog = true;
    state.submit_is_pending = true;

    let _ = update(&mut state, Message::EscapePressed);

    assert!(state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());

    state.submit_is_pending = false;
    let _ = update(&mut state, Message::EscapePressed);

    assert!(!state.show_submit_dialog);
    assert!(state.editor.as_ref().unwrap().stored_loop_range.is_some());
}

#[test]
fn captured_escape_routes_to_create_region_input_reset() {
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::Escape),
            Modifiers::empty(),
            Status::Captured,
        ),
        Some(Message::CreateRegionInputEscapePressed)
    );
}

#[test]
fn captured_escape_closes_browser_find_before_create_region_input_reset() {
    let mut state = stable_sample_state();
    open_create_region_dialog_from_waveform(&mut state, 12.0, 12.8);

    let _ = update(&mut state, Message::OpenFind);
    assert!(state.find_bar_open);
    assert!(state.create_region_dialog.is_some());

    let _ = update(&mut state, Message::CreateRegionInputEscapePressed);

    assert!(!state.find_bar_open);
    assert!(state.create_region_dialog.is_some());
}

#[test]
fn captured_command_enter_submits_create_region_dialog_like_recovered_babel() {
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::Enter),
            Modifiers::COMMAND,
            Status::Captured,
        ),
        Some(Message::SubmitCreateRegion)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::Enter),
            Modifiers::empty(),
            Status::Captured,
        ),
        None
    );
}

#[test]
fn transport_strip_matches_recovered_babel_action_row_control_set() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("TRANSPORT_TRACK_FILTER_WIDTH"));
    assert!(source.contains("Message::TrackFilterChanged"));
    assert!(source.contains("editor.transcript_visible_cells(project).track"));
    assert!(source.contains("if dependency.show_track_filter"));
    assert!(source.contains(".text_size(14)"));
    assert!(source.contains(".padding([5, 12])"));
    assert!(source.contains(".style(transport_track_filter_style)"));
    assert!(source.contains("TRANSPORT_ICON_PREVIOUS"));
    assert!(source.contains("Message::PreviousSegment"));
    assert!(source.contains("TRANSPORT_ICON_PLAY"));
    assert!(source.contains("Message::TogglePlayback"));
    assert!(source.contains("TRANSPORT_ICON_NEXT"));
    assert!(source.contains("Message::NextSegment"));
    assert!(source.contains("TRANSPORT_ICON_UNDO"));
    assert!(source.contains("Message::Undo"));
    assert!(source.contains("TRANSPORT_ICON_REDO"));
    assert!(source.contains("Message::Redo"));
    assert!(source.contains("view_problem_summary_group"));
    assert!(source.contains("Message::SaveProgress"));
    assert!(source.contains("Message::RequestSubmitReview"));

    assert!(!source.contains("TRANSPORT_ICON_ADD_SEGMENT"));
    assert!(!source.contains("TRANSPORT_ICON_ADD_BLANK"));
    assert!(!source.contains("\"Add segment at playhead\""));
    assert!(!source.contains("Message::CreateSegmentAtPlayhead"));
    assert!(!source.contains("\"Add blank segment\""));
    assert!(!source.contains("Message::CreateBlankSegment"));
    assert!(!source.contains("Message::ToggleOverlay"));
}

#[test]
fn transport_strip_gates_optional_babel_admin_and_skip_actions_by_task_metadata() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("admin_flag_control_available(project)"));
    assert!(source.contains("skip_chunk_control_available(project)"));
    assert!(source.contains("skip_chunk: Option<SkipChunkControlView>"));
    assert!(source.contains("admin_flag: Option<AdminFlagControlView>"));
    assert!(source.contains("if let Some(skip_chunk) = &dependency.skip_chunk"));
    assert!(source.contains("if let Some(admin_flag) = &dependency.admin_flag"));
    assert!(source.contains("Message::RequestAdminFlag"));
    assert!(source.contains("Message::RequestSkipChunk"));
}

#[test]
fn admin_flag_dialog_disables_reason_input_while_submitting_like_recovered_babel() {
    let source = include_str!("components/admin_flag_dialog.rs");

    assert!(source.contains(".on_input_maybe("));
    assert!(source.contains("!admin_flag.reason_disabled"));
    assert!(source.contains("then_some(Message::AdminFlagReasonChanged)"));
    assert!(!source.contains(".on_input(Message::AdminFlagReasonChanged)"));
}

#[test]
fn transport_strip_keeps_submit_block_copy_in_tooltip_not_inline_row() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("submit.trigger_tooltip_content"));
    assert!(source.contains("fixed_submit_button"));
    assert!(source.contains("transport_submit_button_style"));
    assert!(source.contains("!submit.trigger_disabled"));
    assert!(!source.contains("text(submit.trigger_tooltip_content.clone())"));
    assert!(!source.contains("submit_tooltip_color"));
}

#[test]
fn transport_strip_disables_recovered_babel_undo_redo_when_unavailable() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("state.transport_strip_snapshot_for_render(project, editor)"));
    assert!(source.contains("state.submit_control_snapshot_for_render(project, editor)"));
    assert!(source.contains("TRANSPORT_ICON_UNDO"));
    assert!(source.contains("\"Undo\""));
    assert!(source.contains("(!strip.undo_disabled).then_some(Message::Undo)"));
    assert!(source.contains("TRANSPORT_ICON_REDO"));
    assert!(source.contains("\"Redo\""));
    assert!(source.contains("(!strip.redo_disabled).then_some(Message::Redo)"));
}

#[test]
fn transport_strip_uses_recovered_babel_icon_action_buttons() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("transport_svg_icon_button"));
    assert!(source.contains("TRANSPORT_ICON_BUTTON_WIDTH"));
    assert!(source.contains("TRANSPORT_ICON_PREVIOUS"));
    assert!(source.contains("TRANSPORT_ICON_PLAY"));
    assert!(source.contains("TRANSPORT_ICON_REDO"));
    assert!(source.contains("transport_icon_button_style"));
    assert!(source.contains("strip.play_label == \"Pause\""));
    assert!(source.contains("tooltip("));
    assert!(!source.contains("const TRANSPORT_ICON_PREVIOUS: &str"));
    assert!(!source.contains("button(text(icon).size(16))"));
    assert!(!source.contains("button(text(\"|<\"))"));
    assert!(!source.contains("button(text(strip.play_label))"));
    assert!(!source.contains("button(text(\">|\"))"));
    assert!(!source.contains("button(text(\"Undo\"))"));
    assert!(!source.contains("button(text(\"Redo\"))"));
}

#[test]
fn transport_strip_reuses_static_svg_handles_across_play_pause_rebuilds() {
    let source = include_str!("components/transport_strip.rs");
    let button_fn = source
        .split("fn transport_svg_icon_button")
        .nth(1)
        .expect("transport icon button helper should exist");

    assert!(source.contains("LazyLock<SvgHandle>"));
    assert!(source.contains("TRANSPORT_ICON_PLAY_HANDLE"));
    assert!(source.contains("TRANSPORT_ICON_PAUSE_HANDLE"));
    assert!(source.contains("play_icon_handle("));
    assert!(
        !button_fn.contains("SvgHandle::from_memory"),
        "play/pause rebuilds should reuse prebuilt SVG handles instead of parsing icon bytes in the transport hot path"
    );
}

#[test]
fn top_toolbar_reuses_static_svg_handles_across_playhead_rebuilds() {
    let source = include_str!("components/toolbar.rs");
    let zoom_fn = source
        .split("fn zoom_svg_icon")
        .nth(1)
        .expect("toolbar zoom icon helper should exist");
    let labeled_button_fn = source
        .split("fn toolbar_labeled_icon_button")
        .nth(1)
        .expect("toolbar labeled icon helper should exist");

    assert!(source.contains("LazyLock<SvgHandle>"));
    assert!(source.contains("ZOOM_OUT_ICON_HANDLE"));
    assert!(source.contains("ZOOM_IN_ICON_HANDLE"));
    assert!(source.contains("COPY_SUPPORT_ID_ICON_HANDLE"));
    assert!(source.contains("FEEDBACK_ICON_HANDLE"));
    assert!(source.contains("HOTKEYS_ICON_HANDLE"));
    assert!(zoom_fn.contains("icon: SvgHandle"));
    assert!(labeled_button_fn.contains("icon: SvgHandle"));
    assert!(!zoom_fn.contains("SvgHandle::from_memory(icon)"));
    assert!(!labeled_button_fn.contains("SvgHandle::from_memory(icon)"));
}

#[test]
fn transport_strip_uses_recovered_babel_linter_summary_pill() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("view_problem_summary_group"));
    assert!(source.contains("TRANSPORT_PROBLEM_BUTTON_WIDTH"));
    assert!(source.contains("TRANSPORT_PROBLEM_ICON_SIZE"));
    assert!(source.contains("TRANSPORT_ICON_PROBLEM_PREVIOUS"));
    assert!(source.contains("TRANSPORT_ICON_PROBLEM_NEXT"));
    assert!(source.contains("problem_summary_group_style"));
    assert!(source.contains("problem_summary_nav_button_style"));
    assert!(source.contains("Message::NavigateProblemRows(-1)"));
    assert!(source.contains("Message::NavigateProblemRows(1)"));
    assert!(!source.contains("button(text(\"^\").size(12))"));
    assert!(!source.contains("button(text(\"v\").size(12))"));
}

#[test]
fn transport_strip_uses_fixed_babel_control_widths_to_prevent_wrapping() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("const TRANSPORT_TRACK_FILTER_WIDTH: f32 = 180.0;"));
    assert!(source.contains("const TRANSPORT_ICON_BUTTON_WIDTH: f32 = 42.0;"));
    assert!(source.contains("const TRANSPORT_BUTTON_HEIGHT: f32 = 32.0;"));
    assert!(source.contains("const TRANSPORT_CLUSTER_GAP: u16 = 8;"));
    assert!(source.contains("const REVIEW_ACTION_BUTTON_WIDTH: f32 = 118.0;"));
    assert!(source.contains("const REVIEW_SAVE_BUTTON_WIDTH: f32 = 112.0;"));
    assert!(source.contains("const REVIEW_SUBMIT_BUTTON_WIDTH: f32 = 132.0;"));
    assert!(source.contains("fixed_review_button"));
}

#[test]
fn transport_strip_uses_recovered_babel_compact_action_row_height() {
    let source = include_str!("components/transport_strip.rs");

    assert!(source.contains("const TRANSPORT_STRIP_HEIGHT: f32 = 58.0;"));
    assert!(source.contains(".height(Length::Fixed(TRANSPORT_STRIP_HEIGHT))"));
    assert!(source.contains(".height(Length::Fixed(TRANSPORT_RULE_HEIGHT))"));
    assert!(source.contains(".height(Length::Fixed(TRANSPORT_BUTTON_HEIGHT))"));
}

#[test]
fn timeline_lane_controls_use_recovered_babel_icon_buttons() {
    let source = include_str!("components/timeline/lane.rs");
    let builder_source = include_str!("../timeline_model/builder.rs");
    let drag_source = include_str!("components/drag_capture.rs");

    assert!(source.contains("lane_svg_icon_button"));
    assert!(source.contains("LANE_ICON_EYE_OFF"));
    assert!(source.contains("LANE_ICON_MIC"));
    assert!(source.contains("const LANE_ICON_BUTTON_WIDTH: f32 = 28.0;"));
    assert!(source.contains("const LANE_ICON_BUTTON_HEIGHT: f32 = 28.0;"));
    assert!(source.contains("const LANE_ICON_SIZE: f32 = 12.0;"));
    assert!(source.contains("LANE_EDGE_BORDER_WIDTH"));
    assert!(source.contains("lane_edge_border"));
    assert!(source.contains("const LANE_CONTROL_RAIL_WIDTH: f32 = 176.0;"));
    assert!(source.contains("LANE_VERTICAL_ZOOM_SLIDER_WIDTH"));
    assert!(source.contains("fn view_lane_control_rail"));
    assert!(source.contains("row![label, view_lane_vertical_zoom_slider(dependency)]"));
    assert!(source.contains("lane.viewport_state.container_height"));
    assert!(source.contains("lane.viewport_state.render_vertical_zoom_slider"));
    assert!(source.contains("lane.viewport_state.render_solo_controls"));
    assert!(source.contains("lane.chrome.collapse_tooltip.clone()"));
    assert!(source.contains("lane.chrome.solo_aria_label.clone()"));
    assert!(source.contains(".width(Length::Fixed(LANE_CONTROL_RAIL_WIDTH))"));
    assert!(source.contains("view_lane_vertical_zoom_slider"));
    assert!(source.contains("Message::LaneVerticalZoomSliderPointerMoved"));
    assert!(source.contains("Message::LaneVerticalZoomSliderPressed"));
    assert!(source.contains("Message::LaneVerticalZoomSliderReleased"));
    assert!(source.contains("view_vertical_drag_capture"));
    assert!(drag_source.contains("mouse::Interaction::Pointer"));
    assert!(source.contains("tooltip("));
    assert!(source.contains(".padding([8, 12])"));
    assert!(source.contains("text(dependency.label.clone()).size(14)"));
    assert!(builder_source.contains("\"Show track\""));
    assert!(builder_source.contains("\"Hide track\""));
    assert!(builder_source.contains("\"Solo track\""));
    assert!(builder_source.contains("\"Unsolo track\""));
    assert!(!source.contains(", slider,"));
    assert!(!source.contains(" slider("));
    assert!(!source.contains(".style(zoom_slider_style)"));
    assert!(!source.contains("format!(\"{:.0}x\", lane.vertical_zoom)"));
    assert!(!source.contains("const LANE_ICON_AUDIBLE: &str = \"o\""));
    assert!(!source.contains("LANE_ACCENT_WIDTH"));
    assert!(!source.contains("const LANE_LABEL_WIDTH"));
    assert!(!source.contains("let vertical_zoom_slider = view_lane_vertical_zoom_slider(&lane);"));
    assert!(!source.contains("vertical_zoom_slider,\r\n        waveform,"));
    assert!(!source.contains("vertical_zoom_slider,\n        waveform,"));
    assert!(!source.contains("continuous_waveform_style(&accent_token)"));
    assert!(!source.contains("button(text(icon).size(15))"));
    assert!(!source.contains("text(lane.label.clone()).size(15)"));
    assert!(!source.contains("const LANE_ICON_SIZE: f32 = 15.0;"));
    assert!(!source.contains("\"Expand track\""));
    assert!(!source.contains("\"Collapse track\""));
    assert!(!source.contains("\"Toggle track audio\""));
}

#[test]
fn playback_adjacent_controls_reuse_static_svg_handles() {
    let lane_source = include_str!("components/timeline/lane.rs");
    let action_source = include_str!("components/transcript_table/actions.rs");
    let row_source = include_str!("components/transcript_table/row.rs");
    let lane_button_fn = lane_source
        .split("fn lane_svg_icon_button")
        .nth(1)
        .expect("lane icon helper should exist");
    let action_trigger_fn = action_source
        .split("fn action_menu_svg_trigger")
        .nth(1)
        .expect("action trigger helper should exist")
        .split("fn view_action_popup_menu")
        .next()
        .expect("action trigger helper should precede popup menu");
    let action_icon_fn = action_source
        .split("fn view_action_button_icon")
        .nth(1)
        .expect("action item icon helper should exist")
        .split("fn action_message")
        .next()
        .expect("action item icon helper should precede action messages");
    let ack_button_fn = row_source
        .split("fn view_low_confidence_ack_button")
        .nth(1)
        .expect("low-confidence ack helper should exist")
        .split("fn word_diff_token_color")
        .next()
        .expect("ack helper should precede word diff colors");

    assert!(lane_source.contains("LANE_ICON_EYE_HANDLE"));
    assert!(lane_source.contains("LANE_ICON_EYE_OFF_HANDLE"));
    assert!(lane_source.contains("LANE_ICON_MIC_HANDLE"));
    assert!(lane_source.contains("LANE_ICON_MIC_OFF_HANDLE"));
    assert!(lane_button_fn.contains("icon: SvgHandle"));
    assert!(!lane_button_fn.contains("SvgHandle::from_memory(icon)"));
    assert!(action_source.contains("ACTION_MENU_ICON_HANDLE"));
    assert!(action_source.contains("ACTION_MENU_ICON_ADD_HANDLE"));
    assert!(action_source.contains("ACTION_MENU_ICON_MERGE_ABOVE_HANDLE"));
    assert!(action_source.contains("ACTION_MENU_ICON_MERGE_BELOW_HANDLE"));
    assert!(action_source.contains("ACTION_MENU_ICON_DELETE_HANDLE"));
    assert!(action_trigger_fn.contains("ACTION_MENU_ICON_HANDLE.clone()"));
    assert!(!action_trigger_fn.contains("SvgHandle::from_memory(ACTION_MENU_ICON)"));
    assert!(action_icon_fn.contains("SvgHandle"));
    assert!(!action_icon_fn.contains("SvgHandle::from_memory(icon)"));
    assert!(row_source.contains("LOW_CONFIDENCE_ACK_ICON_HANDLE"));
    assert!(ack_button_fn.contains("LOW_CONFIDENCE_ACK_ICON_HANDLE.clone()"));
    assert!(!ack_button_fn.contains("SvgHandle::from_memory(LOW_CONFIDENCE_ACK_ICON)"));
}

#[test]
fn timeline_lane_vertical_zoom_slider_thumb_tracks_recovered_babel_value() {
    let source = include_str!("components/timeline/lane.rs");

    assert!(source.contains("const LANE_VERTICAL_ZOOM_SLIDER_MIN: f32 = 0.3;"));
    assert!(source.contains("const LANE_VERTICAL_ZOOM_SLIDER_MAX: f32 = 1000.0;"));
    assert!(source.contains("lane_vertical_zoom_slider_top_fraction(dependency.vertical_zoom())"));
    assert!(source.contains("fn lane_vertical_zoom_slider_top_fraction"));
    assert!(source.contains("view_lane_vertical_zoom_slider_body("));
    assert!(source.contains("thumb_top_offset,"));
    assert!(source.contains("thumb_bottom_offset,"));
    assert!(source.contains("(height - LANE_VERTICAL_ZOOM_SLIDER_THUMB_SIZE).max(0.0)"));
    assert!(!source.contains("column![\n        Space::with_height(Fill),"));
    assert!(!source.contains("fn view_lane_vertical_zoom_slider_body<'a>()"));
}

#[test]
fn timeline_drag_controls_use_captured_drag_surface_after_pointer_leaves_bounds() {
    let toolbar_source = include_str!("components/toolbar.rs");
    let lane_source = include_str!("components/timeline/lane.rs");
    let drag_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/drag_capture.rs"
    ))
    .expect("drag capture component should exist");

    assert!(toolbar_source.contains("view_horizontal_drag_capture"));
    assert!(lane_source.contains("view_vertical_drag_capture"));
    assert!(drag_source.contains("state.dragging || hovered"));
    assert!(drag_source.contains("ButtonReleased(mouse::Button::Left)"));
    assert!(!toolbar_source.contains("mouse_area(container(content).width(Fill).height"));
    assert!(
        !lane_source
            .contains("mouse_area(\n            container(view_lane_vertical_zoom_slider_body")
    );
}

#[test]
fn timeline_lanes_use_recovered_babel_visual_stack_and_colors() {
    let state = stable_sample_state();
    let project = state.project().unwrap();
    let lanes = state.editor.as_ref().unwrap().timeline_lanes(project);
    let style_source = include_str!("styles.rs");
    let builder_source = include_str!("../timeline_model/builder.rs");

    assert_eq!(lanes[0].lane_id, "track-speaker-1");
    assert_eq!(lanes[0].label, "Speaker 1");
    assert_eq!(lanes[0].color_token, "blue");
    assert_eq!(lanes[1].lane_id, "track-speaker-2");
    assert_eq!(lanes[1].label, "Speaker 2");
    assert_eq!(lanes[1].color_token, "purple");

    assert!(style_source.contains("Color::from_rgb8(100, 181, 246)"));
    assert!(style_source.contains("Color::from_rgb8(176, 131, 255)"));
    assert!(style_source.contains("Color::from_rgb8(49, 160, 252)"));
    assert!(!style_source.contains("Color::from_rgb8(72, 166, 232)"));
    assert!(!style_source.contains("Color::from_rgb8(152, 106, 236)"));
    assert!(!style_source.contains("Color::from_rgb8(31, 170, 116)"));
    assert!(!style_source.contains("Color::from_rgb8(218, 160, 46)"));
    assert!(builder_source.contains("match index % 3"));
    assert!(!builder_source.contains("=> \"green\""));
    assert!(!builder_source.contains("=> \"amber\""));
}

#[test]
fn timeline_waveform_draws_flat_wavesurfer_region_overlays() {
    let source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let style_source = include_str!("styles.rs");

    assert!(source.contains("view_waveform_region_affordance_block"));
    assert!(source.contains("view_wavesurfer_line_waveform_window_canvas"));
    assert!(source.contains("view_wavesurfer_region_fill_window_canvas"));
    assert!(source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas"));
    assert!(source.contains("view_waveform_selected_region_border_canvas"));
    assert!(canvas_source.contains("fn draw_region_fills("));
    assert!(canvas_source.contains("struct SelectedRegionBorderCanvas"));
    assert!(canvas_source.contains("a: 0.25"));
    assert!(canvas_source.contains("region_border_color_for_token"));
    assert!(canvas_source.contains("with_width(2.0)"));
    assert!(style_source.contains("Color::from_rgb8(49, 160, 252)"));
    assert!(!source.contains("waveform_region_overlay_style"));
    assert!(!source.contains(".style(move |_| block_style(&color_token, is_selected))"));
    assert!(!source.contains("centered_peak_spacers"));
    assert!(!source.contains("top_spacer"));
    assert!(!source.contains("bottom_spacer"));
    assert!(!source.contains("Space::with_height(Length::Fixed(top_spacer))"));
    assert!(!source.contains("Space::with_height(Length::Fixed(bottom_spacer))"));
    assert!(!source.contains("scaled_peak_height"));
}

#[test]
fn timeline_waveform_region_overlays_render_recovered_babel_affordances() {
    let source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let style_source = include_str!("styles.rs");

    assert!(source.contains("const REGION_HANDLE_HIT_ZONE_WIDTH: f32 = 12.0;"));
    assert!(source.contains("const REGION_TIMESTAMP_TOOLTIP_TOP_OFFSET: f32 = -22.0;"));
    assert!(source.contains("const REGION_LABEL_TOOLTIP_WIDTH: f32 = 300.0;"));
    assert!(source.contains("const REGION_EDIT_PENCIL_ICON"));
    assert!(source.contains("view_region_boundary_timestamps"));
    assert!(source.contains("view_region_label_edit_tooltip"));
    assert!(source.contains("if !block.is_transcription"));
    assert!(source.contains(".on_press(Message::RequestEditAnnotation(block.segment_id.clone()))"));
    assert!(source.contains(".style(table_plain_button_style)"));
    assert!(source.contains("REGION_HANDLE_HIT_ZONE_WIDTH / 2.0"));
    assert!(canvas_source.contains("fn waveform_region_border_color(color_token: &str) -> Color"));
    assert!(canvas_source.contains(".with_width(WAVEFORM_REGION_BORDER_WIDTH)"));
    assert!(source.contains("format_mm_ss(region_start_seconds"));
    assert!(source.contains("format_mm_ss(region_end_seconds"));
    assert!(style_source.contains("waveform_region_timestamp_label_style"));
    assert!(style_source.contains("waveform_region_label_edit_tooltip_style"));
    assert!(style_source.contains("Color {"));
    assert!(style_source.contains("a: 0.90"));
    assert!(style_source.contains("Color::from_rgb8(15, 23, 42)"));
}

#[test]
fn timeline_waveform_region_affordance_reuses_static_pencil_svg_handle() {
    let source = include_str!("components/timeline/waveform.rs");
    let label_tooltip_fn = source
        .split("fn view_region_label_edit_tooltip")
        .nth(1)
        .expect("waveform region label edit tooltip helper should exist");

    assert!(source.contains("LazyLock<SvgHandle>"));
    assert!(source.contains("REGION_EDIT_PENCIL_ICON_HANDLE"));
    assert!(label_tooltip_fn.contains("REGION_EDIT_PENCIL_ICON_HANDLE.clone()"));
    assert!(!label_tooltip_fn.contains("SvgHandle::from_memory(REGION_EDIT_PENCIL_ICON)"));
}

#[test]
fn timeline_waveform_uses_dense_wavesurfer_canvas_without_widget_gutters() {
    let view_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let model_source = include_str!("../timeline_model/waveform.rs");

    assert!(model_source.contains("const WAVEFORM_PEAK_COUNT: usize = 96;"));
    assert!(model_source.contains("const TIMELINE_RENDER_PEAK_COUNT: usize = 1024;"));
    assert!(view_source.contains("view_wavesurfer_line_waveform"));
    assert!(canvas_source.contains("canvas::Path::new"));
    assert!(canvas_source.contains("frame.fill(&waveform_path"));
    assert!(!view_source.contains("Row::new().spacing(1).width(Fill)"));
    assert!(!view_source.contains(".spacing(1)\n        .width(Fill)"));
    assert!(!view_source.contains(".padding(2)"));
}

#[test]
fn timeline_synthetic_lanes_render_recovered_wavesurfer_segment_canvases() {
    let source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("const SYNTHETIC_REGION_PEAK_SCALE: f64 = 0.045;"));
    assert!(source.contains("view_lazy_synthetic_base_waveform"));
    assert!(source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas("));
    assert!(canvas_source.contains("struct SegmentWaveformsWithRegionFillCanvas"));
    assert!(canvas_source.contains("append_wavesurfer_segment_waveforms_f32("));
    assert!(canvas_source.contains("synthetic_waveform_block_widths("));
    assert!(source.contains("SYNTHETIC_REGION_PEAK_SCALE"));
    assert!(canvas_source.contains("synthetic_waveform_color(color_token)"));
    assert!(source.contains("view_lazy_region_affordance_overlay("));
    assert!(!source.contains("view_lazy_region_base_overlay("));
    assert!(!source.contains("view_synthetic_segment_waveform_block"));
    assert!(!source.contains("view_wavesurfer_line_waveform_f32("));
    assert!(!source.contains("synthetic_segment_peaks_for_canvas"));
    assert!(!source.contains(".map(|peak| f64::from(peak) * SYNTHETIC_REGION_PEAK_SCALE)"));
    assert!(!source.contains("Row::new().spacing(1).width(Fill)"));
    assert!(!source.contains("scaled_peak_height"));
    assert!(!source.contains("centered_peak_spacers"));
}

#[test]
fn timeline_synthetic_hit_testing_borrows_waveform_blocks_without_region_clone_list() {
    let source = include_str!("components/timeline/waveform.rs");

    assert!(source.contains("let hit_blocks = Arc::clone(&data.blocks);"));
    assert!(source.contains("synthetic_hit_test(&hit_blocks, viewport, fraction, width)"));
    assert!(source.contains("fn synthetic_hit_test(\n    blocks: &[WaveformBlock],"));
    assert!(!source.contains("struct SyntheticHitRegion"));
    assert!(!source.contains("fn synthetic_hit_regions"));
    assert!(!source.contains("let hit_regions = synthetic_hit_regions"));
}

#[test]
fn timeline_region_base_overlay_uses_cached_canvas_instead_of_per_segment_widgets() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(waveform_source.contains("struct SyntheticBaseWaveformDependency"));
    assert!(waveform_source.contains("struct EngineWaveformStripDependency"));
    assert!(waveform_source.contains("struct EngineRegionFillDependency"));
    assert!(waveform_source.contains("view_lazy_synthetic_base_waveform("));
    assert!(waveform_source.contains("view_lazy_engine_waveform_strip("));
    assert!(waveform_source.contains("view_lazy_engine_region_fill_strip("));
    assert!(waveform_source.contains("view_lazy_selected_region_border_overlay("));
    assert!(waveform_source.contains("view_lazy_region_affordance_overlay("));
    assert!(waveform_source.contains("view_wavesurfer_line_waveform_window_canvas("));
    assert!(waveform_source.contains("view_wavesurfer_region_fill_window_canvas("));
    assert!(waveform_source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas("));
    assert!(waveform_source.contains("view_waveform_selected_region_border_canvas("));
    assert!(canvas_source.contains("struct RegionFillCanvas"));
    assert!(canvas_source.contains("impl canvas::Program<Message> for RegionFillCanvas"));
    assert!(canvas_source.contains("struct SegmentWaveformsWithRegionFillCanvas"));
    assert!(
        canvas_source
            .contains("impl canvas::Program<Message> for SegmentWaveformsWithRegionFillCanvas")
    );
    assert!(canvas_source.contains("struct SelectedRegionBorderCanvas"));
    assert!(canvas_source.contains("impl canvas::Program<Message> for SelectedRegionBorderCanvas"));
    assert!(canvas_source.contains("frame.fill(&region_path"));
    assert!(canvas_source.contains("frame.stroke("));
    assert!(canvas_source.contains("synthetic_waveform_block_widths("));
    assert!(waveform_source.contains("pending_space_portions"));
    assert!(waveform_source.contains("view_waveform_region_affordance_block("));
    assert!(!waveform_source.contains("fn view_region_overlay<'a>("));
    assert!(!waveform_source.contains("row = row.push(view_waveform_region_overlay_block("));
}

#[test]
fn transcript_action_cell_uses_recovered_vertical_dots_trigger() {
    let source = include_str!("components/transcript_table/actions.rs");

    assert!(source.contains("ACTION_MENU_ICON"));
    assert!(source.contains("ACTION_MENU_TRIGGER_WIDTH"));
    assert!(source.contains("action_menu_svg_trigger"));
    assert!(source.contains("align_action_trigger"));
    assert!(source.contains("row![Space::with_width(Fill), icon]"));
    assert!(source.contains(".style(table_plain_button_style)"));
    assert!(source.contains(".on_press(Message::ToggleActions(segment_id))"));
    assert!(
        source.contains(".width(Length::Fixed(super::ACTION_COLUMN_WIDTH))"),
        "native action menu should keep the visual dots small but expose a browser-like column hit target"
    );
    assert!(source.contains(".height(Length::Fixed(ACTION_MENU_TRIGGER_WIDTH))"));
    assert!(
        !source.contains("tooltip("),
        "the native action trigger must remain a direct button; wrapping it in a tooltip can swallow pointer presses"
    );
    assert!(!source.contains("button(icon)\n        .style(outline_button_style)"));
    assert!(!source.contains("ACTION_MENU_TRIGGER: &str"));
    assert!(!source.contains("button(text(ACTION_MENU_TRIGGER)"));
    assert!(!source.contains("\"Close\""));
    assert!(!source.contains("\"...\""));
}

#[test]
fn transcript_table_reserves_scrollbar_gutter_after_actions() {
    let table_source = include_str!("components/transcript_table/mod.rs");
    let header_source = include_str!("components/transcript_table/header.rs");
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(table_source.contains("const SCROLLBAR_GUTTER_WIDTH: f32 = 16.0;"));
    assert!(header_source.contains("view_scrollbar_gutter()"));
    assert!(row_source.contains("view_scrollbar_gutter()"));
    assert!(row_source.contains("let action_cells = if row_view.visible_cells.actions"));
    assert!(
        row_source.contains("let selectable_body = mouse_area(container(row_content).width(Fill))")
    );
    assert!(row_source.contains("full_row = full_row.push(action_cell).push(scrollbar_gutter);"));
    assert!(!row_source.contains("mouse_area(row_container)"));
    assert!(
        row_source.matches("super::view_scrollbar_gutter()").count() >= 2,
        "normal and modified rows should reserve a click-safe gutter after the action cell"
    );
}

#[test]
fn transcript_action_cell_uses_recovered_babel_dropdown_content() {
    let source = include_str!("components/transcript_table/actions.rs");
    let style_source = include_str!("styles.rs");

    assert!(source.contains("const ACTION_MENU_POPUP_WIDTH: f32 = 192.0;"));
    assert!(source.contains("const ACTION_MENU_ITEM_ICON_SIZE: f32 = 12.0;"));
    assert!(source.contains("view_action_popup_menu"));
    assert!(source.contains("view_action_button_icon"));
    assert!(source.contains("ACTION_MENU_ICON_ADD"));
    assert!(source.contains("ACTION_MENU_ICON_MERGE_ABOVE"));
    assert!(source.contains("ACTION_MENU_ICON_MERGE_BELOW"));
    assert!(source.contains("ACTION_MENU_ICON_DELETE"));
    assert!(source.contains("action_menu_popup_style"));
    assert!(source.contains("action_menu_item_button_style"));
    assert!(source.contains("action_menu_delete_button_style"));
    assert!(source.contains("action_menu_separator_style"));
    assert!(
        source.contains("container(popup).align_right(Length::Fixed(ACTION_MENU_POPUP_WIDTH))")
    );
    assert!(source.contains(".width(Length::Fixed(ACTION_MENU_POPUP_WIDTH))"));
    assert!(!source.contains(".style(outline_button_style)"));

    assert!(style_source.contains("pub(crate) fn action_menu_popup_style"));
    assert!(style_source.contains("pub(crate) fn action_menu_item_button_style"));
    assert!(style_source.contains("pub(crate) fn action_menu_delete_button_style"));
    assert!(style_source.contains("pub(crate) fn action_menu_separator_style"));
    assert!(style_source.contains("Color::from_rgb8(220, 38, 38)"));
}

#[test]
fn transcript_table_uses_recovered_babel_compact_actions_column() {
    let source = include_str!("components/transcript_table/mod.rs");

    assert!(source.contains("const ACTION_COLUMN_WIDTH: f32 = 64.0;"));
    assert!(!source.contains("const ACTION_COLUMN_WIDTH: f32 = 118.0;"));
}

#[test]
fn transcript_workspace_uses_recovered_babel_sticky_header_shell() {
    let source = include_str!("components/transcript_table/mod.rs");

    assert!(source.contains("let mut rows = Column::new().spacing(0);"));
    assert!(source.contains("let transcript_layout = state.transcript_layout_for_render"));
    assert!(source.contains("let virtual_window = transcript_layout.virtual_window"));
    assert!(source.contains("Space::with_height(Length::Fixed("));
    assert!(source.contains(".skip(virtual_window.start_index)"));
    assert!(source.contains(".take(virtual_window.rendered_len())"));
    assert!(source.contains(".on_scroll(|viewport|"));
    assert!(source.contains("column!["));
    assert!(source.contains("header::view_table_header(visible_cells)"));
    assert!(source.contains("scrollable(rows)"));
    assert!(source.contains(".height(Fill)"));
    assert!(source.contains(".padding(12)"));
    assert!(!source.contains("scrollable(table).height(Fill)"));
    assert!(!source.contains(".padding(10)"));
}

#[test]
fn transcript_grid_uses_recovered_babel_cell_metrics_and_density() {
    let table_source = include_str!("components/transcript_table/mod.rs");
    let header_source = include_str!("components/transcript_table/header.rs");
    let row_source = include_str!("components/transcript_table/row.rs");
    let style_source = include_str!("styles.rs");

    assert!(table_source.contains("const ID_COLUMN_WIDTH: f32 = 48.0;"));
    assert!(table_source.contains("const SPEAKER_COLUMN_WIDTH: f32 = 96.0;"));
    assert!(table_source.contains("const TIME_COLUMN_WIDTH: f32 = 96.0;"));
    assert!(table_source.contains("const LINTER_COLUMN_WIDTH: f32 = 40.0;"));
    assert!(table_source.contains("const ACTION_COLUMN_WIDTH: f32 = 64.0;"));
    assert!(table_source.contains("const REVIEWER_COLUMN_WIDTH: f32 = 144.0;"));
    assert!(header_source.contains("Row::new().spacing(0).padding([4, 0])"));
    assert!(row_source.contains("Row::new().spacing(0)"));
    assert!(row_source.contains(".padding([1, 8])"));
    assert!(row_source.contains(".padding([5, 8])"));
    assert!(row_source.contains(".size(12)"));
    assert!(row_source.contains(".style(table_plain_text_editor_style)"));
    assert!(style_source.contains("pub(crate) fn table_plain_text_editor_style"));
    assert!(!row_source.contains("Row::new().spacing(8)"));
    assert!(!row_source.contains(".padding(8)"));
    assert!(
        !row_source.contains("container(full_row)\n        .width(Fill)\n        .height(Fill)")
    );
    assert!(!table_source.contains("const REVIEWER_COLUMN_WIDTH: f32 = 92.0;"));
}

#[test]
fn transcript_grid_uses_recovered_babel_plain_row_cells() {
    let state_source = include_str!("state.rs");
    let actions_source = include_str!("actions.rs");
    let row_source = include_str!("components/transcript_table/row.rs");
    let row_actions_source = include_str!("components/transcript_table/actions.rs");
    let lint_source = include_str!("components/transcript_table/lint.rs");
    let style_source = include_str!("styles.rs");

    assert!(row_source.contains("view_speaker_label"));
    assert!(state_source.contains("BABEL_TRACK_CELL_DOUBLE_CLICK_WINDOW"));
    assert!(state_source.contains("last_track_cell_press: Option<TrackCellPress>"));
    assert!(actions_source.contains("Message::SegmentTrackCellPressed(segment_id)"));
    assert!(row_source.contains("Message::SegmentTrackCellPressed"));
    assert!(row_source.contains("Message::SegmentTrackSelectOpenChanged"));
    assert!(row_source.contains("Message::SegmentTrackChanged"));
    assert!(row_source.contains("pick_list("));
    assert!(!row_source.contains(".on_press(Message::BeginSegmentTrackEdit"));
    assert!(row_actions_source.contains(".style(table_plain_button_style)"));
    assert!(lint_source.contains(".style(table_plain_button_style)"));
    assert!(row_source.contains(".style(table_plain_text_input_style)"));
    assert!(!row_source.contains("button(text(row_view.display_id.clone()))"));
    assert!(style_source.contains("pub(crate) fn table_plain_button_style"));
    assert!(style_source.contains("pub(crate) fn table_plain_text_input_style"));
}

#[test]
fn transcript_rows_use_recovered_babel_whole_row_click_and_hover_contract() {
    let state_source = include_str!("state.rs");
    let message_source = include_str!("message.rs");
    let actions_source = include_str!("actions.rs");
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(state_source.contains("transcript_row_hover_segment_id: Option<String>"));
    assert!(state_source.contains("last_transcript_row_press: Option<TranscriptRowPress>"));
    assert!(message_source.contains("TranscriptRowBodyPressed(String)"));
    assert!(message_source.contains("TranscriptRowHovered(String)"));
    assert!(message_source.contains("TranscriptRowExited(String)"));
    assert!(actions_source.contains("Message::TranscriptRowBodyPressed(segment_id)"));
    assert!(actions_source.contains("Message::TranscriptRowHovered(segment_id)"));
    assert!(actions_source.contains("Message::TranscriptRowExited(segment_id)"));
    assert!(
        row_source.contains("let selectable_body = mouse_area(container(row_content).width(Fill))")
    );
    assert!(
        row_source.contains(
            ".on_press(Message::TranscriptRowBodyPressed(\n            selectable_segment_id.clone(),\n        ))"
        )
    );
    assert!(
        row_source
            .contains(".on_enter(Message::TranscriptRowHovered(selectable_segment_id.clone()))")
    );
    assert!(row_source.contains(".on_exit(Message::TranscriptRowExited(selectable_segment_id))"));
    assert!(row_source.contains("full_row = full_row.push(action_cell).push(scrollbar_gutter);"));
    assert!(!row_source.contains("mouse_area(row_container)"));
    assert!(row_source.contains(".interaction(mouse::Interaction::Pointer)"));
    assert!(!row_source.contains("button(text(row_view.display_id.clone())"));
}

#[test]
fn transcript_rows_use_recovered_babel_neutral_and_playing_active_styles() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let style_source = include_str!("styles.rs");

    assert!(row_source.contains("transcript_row_style("));
    assert!(row_source.contains("let row_is_active = row_view.is_active || row_view.is_selected"));
    assert!(row_source.contains("row_is_active"));
    assert!(row_source.contains("row_view.is_selected"));
    assert!(row_source.contains("row_is_playing"));
    assert!(row_source.contains("row_is_hovered"));
    assert!(style_source.contains("pub(crate) fn transcript_row_style"));
    assert!(style_source.contains("Color::from_rgb8(250, 250, 250)"));
    assert!(style_source.contains("Color::from_rgb8(245, 245, 245)"));
    assert!(style_source.contains("Color::from_rgb8(240, 253, 244)"));
    assert!(style_source.contains("Color::from_rgb8(134, 239, 172)"));
    assert!(!style_source.contains("Color::from_rgb8(247, 249, 253)"));
    assert!(!style_source.contains("Color::from_rgb8(190, 205, 230)"));
}

#[test]
fn transcript_low_confidence_words_use_recovered_babel_inline_dotted_affordance() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let style_source = include_str!("styles.rs");

    assert!(row_source.contains("view_low_confidence_transcript_text"));
    assert!(row_source.contains("view_low_confidence_word_token"));
    assert!(row_source.contains("view_low_confidence_ack_button"));
    assert!(row_source.contains("low_confidence_word_dotted_style"));
    assert!(row_source.contains("low_confidence_ack_button_style"));
    assert!(row_source.contains("Message::AcknowledgeLowConfidenceWord"));
    assert!(row_source.contains("Message::SeekToTime(start_seconds)"));
    assert!(row_source.contains("LOW_CONFIDENCE_ACK_ICON"));
    assert!(!row_source.contains("row![text(\"Review\").size(11)]"));
    assert!(!row_source.contains("text(format!(\"{} ok\", word.text)).size(11)"));

    assert!(style_source.contains("pub(crate) fn low_confidence_text_shell_style"));
    assert!(style_source.contains("pub(crate) fn low_confidence_word_dotted_style"));
    assert!(style_source.contains("pub(crate) fn low_confidence_ack_button_style"));
    assert!(style_source.contains("Color::from_rgb8(245, 158, 11)"));
    assert!(style_source.contains("border::rounded(1)"));
    assert!(style_source.contains(".width(2)"));
}

#[test]
fn transcript_low_confidence_acknowledged_words_drop_recovered_babel_dotted_state() {
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(row_source.contains("view_plain_low_confidence_word_token"));
    assert!(row_source.contains("if is_acknowledged || is_read_only"));
    assert!(row_source.contains("return view_plain_low_confidence_word_token(word);"));
}

#[test]
fn transcript_diff_cells_use_recovered_babel_badges_and_word_chips() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let style_source = include_str!("styles.rs");

    assert!(row_source.contains("view_diff_badge"));
    assert!(row_source.contains("view_diff_word_chips"));
    assert!(row_source.contains("view_diff_word_chip"));
    assert!(row_source.contains("diff_change_badge_style"));
    assert!(row_source.contains("diff_no_change_badge_style"));
    assert!(row_source.contains("diff_new_badge_style"));
    assert!(row_source.contains("diff_word_added_chip_style"));
    assert!(row_source.contains("diff_word_removed_chip_style"));
    assert!(row_source.contains("TranscriptDiffCellKind::New"));
    assert!(!row_source.contains("rich_text(spans).size(11).width(Fill)"));
    assert!(!row_source.contains("text(diff.badge_label.clone())"));

    assert!(style_source.contains("pub(crate) fn diff_change_badge_style"));
    assert!(style_source.contains("pub(crate) fn diff_no_change_badge_style"));
    assert!(style_source.contains("pub(crate) fn diff_new_badge_style"));
    assert!(style_source.contains("pub(crate) fn diff_word_added_chip_style"));
    assert!(style_source.contains("pub(crate) fn diff_word_removed_chip_style"));
    assert!(style_source.contains("Color::from_rgb8(254, 252, 232)"));
    assert!(style_source.contains("Color::from_rgb8(219, 234, 254)"));
    assert!(style_source.contains("Color::from_rgb8(220, 252, 231)"));
    assert!(style_source.contains("Color::from_rgb8(254, 226, 226)"));
}

#[test]
fn top_toolbar_renders_recovered_babel_overview_waveform() {
    let source = include_str!("components/toolbar.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .expect("playhead line widget should exist");

    assert!(source.contains("OverviewWaveformSnapshot"));
    assert!(source.contains("state.overview_waveform_snapshot_for_render(project, editor)"));
    assert!(source.contains("view_overview_waveform"));
    assert!(source.contains("overview_playhead_percent("));
    assert!(source.contains("Message::OverviewPointerMoved"));
    assert!(source.contains("Message::OverviewPressed"));
    assert!(source.contains("Message::OverviewReleased"));
    assert!(source.contains("view_overview_window_quad("));
    assert!(source.contains("view_overview_playhead_canvas("));
    assert!(canvas_source.contains("struct OverviewWindowQuad"));
    assert!(!canvas_source.contains("struct OverviewWindowCanvas"));
    assert!(!canvas_source.contains("struct OverviewPlayheadCanvas"));
    assert!(canvas_source.contains("view_playhead_line(playhead_percent, 30.0, 2.0"));
    assert!(canvas_source.contains("renderer.fill_quad("));
    assert!(playhead_source.contains("renderer.fill_quad("));
    assert!(canvas_source.contains("Color {"));
    assert!(canvas_source.contains("a: 0.10"));
    assert!(canvas_source.contains("Color::from_rgb8(49, 160, 252)"));
}

#[test]
fn top_toolbar_gives_overview_the_recovered_babel_center_span() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains(
        ".push(view_timed(\n            \"overview-waveform\",\n            view_overview_waveform(\n                state.overview_waveform_snapshot_for_render(project, editor),\n                editor.playhead_seconds,\n            ),\n        ))\n        .push(toolbar_separator())"
    ));
    assert!(!source.contains("toolbar\n            .push(Space::with_width(Fill))"));
}

#[test]
fn top_toolbar_uses_recovered_babel_compact_band_height() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("const TOP_TOOLBAR_CONTROL_HEIGHT: f32 = 32.0;"));
    assert!(source.contains("const TOP_TOOLBAR_HEIGHT: f32 = 64.0;"));
    assert!(source.contains("const TOP_TOOLBAR_RULE_HEIGHT: f32 = 34.0;"));
    assert!(source.contains(".height(Length::Fixed(TOP_TOOLBAR_HEIGHT))"));
    assert!(source.contains(".height(Length::Fixed(TOP_TOOLBAR_RULE_HEIGHT))"));
    assert!(source.contains(".height(Length::Fixed(TOP_TOOLBAR_CONTROL_HEIGHT))"));
    assert!(!source.contains(".push(vertical_rule(1))\n        .push(speed)"));
}

#[test]
fn diff_toolbar_keeps_a_stable_native_control_band_height() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("view_diff_toolbar"));
    assert!(source.contains(".height(Length::Fixed(44.0))"));
}

#[test]
fn transcript_modified_diff_rows_render_without_segment_drafts() {
    let table_source = include_str!("components/transcript_table/mod.rs");
    let row_source = include_str!("components/transcript_table/row.rs");
    let style_source = include_str!("styles.rs");

    assert!(table_source.contains("row_view.row_kind != TranscriptRowKind::Annotation"));
    assert!(table_source.contains("view_modified_transcript_row(row_view)"));
    assert!(row_source.contains("pub(super) fn view_modified_transcript_row"));
    assert!(row_source.contains("TranscriptRowKind::ModifiedAdded"));
    assert!(table_source.contains("TranscriptRowKind::Annotation"));
    assert!(row_source.contains("TranscriptDiffCellKind::ModifiedAdded"));
    assert!(row_source.contains("TranscriptDiffCellKind::ModifiedDeleted"));
    assert!(row_source.contains("transcript_modified_row_style"));
    assert!(row_source.contains("text(row_view.text.clone())"));
    assert!(style_source.contains("pub(crate) fn transcript_modified_row_style"));
    assert!(style_source.contains("pub(crate) fn diff_added_badge_style"));
    assert!(style_source.contains("pub(crate) fn diff_deleted_badge_style"));
}

#[test]
fn top_toolbar_uses_recovered_babel_zoom_glyphs_and_neutral_slider() {
    let toolbar_source = include_str!("components/toolbar.rs");
    let style_source = include_str!("styles.rs");

    assert!(toolbar_source.contains("ZOOM_OUT_ICON"));
    assert!(toolbar_source.contains("ZOOM_IN_ICON"));
    assert!(toolbar_source.contains("zoom_svg_icon(ZOOM_OUT_ICON_HANDLE.clone())"));
    assert!(toolbar_source.contains("zoom_svg_icon(ZOOM_IN_ICON_HANDLE.clone())"));
    assert!(toolbar_source.contains(".style(zoom_slider_style)"));
    assert!(!toolbar_source.contains("text(\"-\").size(16)"));
    assert!(!toolbar_source.contains("text(\"+\").size(16)"));
    assert!(style_source.contains("pub(crate) fn zoom_slider_style"));
    assert!(style_source.contains("slider::Style"));
    assert!(style_source.contains("Color::from_rgb8(124, 124, 124)"));
}

#[test]
fn top_toolbar_recovered_actions_use_icons_like_babel() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("COPY_SUPPORT_ID_ICON"));
    assert!(source.contains("HOTKEYS_ICON"));
    assert!(source.contains("toolbar_labeled_icon_button("));
    assert!(source.contains("toolbar_outline_button_style"));
    assert!(source.contains(".padding([0, 8])"));
    assert!(source.contains(".height(Length::Fixed(TOP_TOOLBAR_CONTROL_HEIGHT))"));
    assert!(source.contains("editor.support_id_button(state.support_id_copied)"));
    assert!(source.contains("support_id.button_label"));
    assert!(source.contains("support_id.tooltip_content"));
    assert!(source.contains("build_toolbar_feedback_button("));
    assert!(source.contains("feedback_button.tooltip_content"));
    assert!(source.contains("Message::CopySupportId"));
    assert!(source.contains("Message::ShowHotkeys"));
    assert!(!source.contains("button(text(\"Copy Support ID\"))"));
    assert!(!source.contains("button(text(\"Hotkeys\"))"));
}

#[test]
fn top_toolbar_speed_select_matches_recovered_babel_h8_outline_control() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("toolbar_select_style"));
    assert!(source.contains(".text_size(14)"));
    assert!(source.contains(".padding([5, 10])"));
    assert!(source.contains(".style(toolbar_select_style)"));
    assert!(source.contains(".width(Length::Fixed(TOP_TOOLBAR_SPEED_WIDTH))"));
    assert!(source.contains("const TOP_TOOLBAR_SPEED_WIDTH: f32 = 78.0;"));
    assert!(source.contains("PickListStatus::Hovered | PickListStatus::Opened"));
    assert!(!source.contains(".width(Length::Fixed(78.0))"));
}

#[test]
fn diff_toolbar_uses_recovered_babel_switch_control() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("view_diff_toggle"));
    assert!(source.contains("DIFF_SWITCH_WIDTH"));
    assert!(source.contains("diff_switch_style"));
    assert!(!source.contains("let toggle_label"));
    assert!(!source.contains("button(text(toggle_label))"));
}

#[test]
fn diff_toolbar_hides_metrics_legend_when_diff_view_is_off() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("if !metrics.visible"));
    assert!(source.contains("Space::with_width(Length::Shrink).into()"));
    assert!(!source.contains("if !metrics.visible {\n        let mut legend"));
}

#[test]
fn diff_toolbar_surfaces_timestamp_metric_tooltip_on_avg_shift() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("metrics.timestamp_tooltip.take()"));
    assert!(source.contains("view_timestamp_metric_tooltip"));
    assert!(source.contains("tooltip.toggle_label"));
    assert!(source.contains("tooltip.descriptions"));
    assert!(source.contains("tooltip::Position::FollowCursor"));
}

#[test]
fn diff_toolbar_surfaces_annotation_metric_tooltips() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("view_annotation_metric(event)"));
    assert!(source.contains("view_annotation_metric(label_accuracy)"));
    assert!(source.contains("view_annotation_metric(intensity)"));
    assert!(source.contains("view_annotation_metric_tooltip"));
    assert!(source.contains("tooltip::Position::FollowCursor"));
}

#[test]
fn diff_toolbar_off_state_matches_recovered_babel_minimal_row() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("if toolbar.compare_select.visible && toolbar.diff_toggle.checked"));
    assert!(!source.contains("else if let Some(no_versions_text)"));
    assert!(
        !source.contains("controls = controls.push(text(no_versions_text.to_string()).size(12));")
    );
}

#[test]
fn editor_shell_uses_recovered_babel_page_chrome_and_panel_order() {
    let shell_source = include_str!("components/mod.rs");
    let toolbar_source = include_str!("components/toolbar.rs");
    let context_source = include_str!("components/context_panels.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let transport_source = include_str!("components/transport_strip.rs");
    let transcript_source = include_str!("components/transcript_table/mod.rs");
    let style_source = include_str!("styles.rs");

    assert!(shell_source.contains("const WORKBENCH_PADDING: u16 = 24;"));
    assert!(shell_source.contains("fn view_waveform_card"));
    assert!(shell_source.contains("fn view_transcript_card"));
    assert!(shell_source.contains("fn view_outside_dismiss_area"));
    assert!(shell_source.contains("let mut layout = column![]"));
    assert!(shell_source.contains("context_panels::view_context_panels(state, project)"));
    assert!(shell_source.contains("layout = layout.push(context_panels);"));
    assert!(shell_source.contains("layout = layout.push(waveform_card);"));
    assert!(context_source.contains(".width(Fill)"));
    assert!(!context_source.contains("CONTEXT_PANEL_WIDTH"));
    assert!(!context_source.contains("scrollable(panels).height(Fill)"));
    assert!(!context_source.contains("Length::Fixed(CONTEXT_PANEL_WIDTH)"));
    assert!(shell_source.contains("view_waveform_card(state, project, editor)"));
    assert!(shell_source.contains("view_transcript_card(state, project, editor)"));
    assert!(shell_source.contains(".spacing(8)"));
    assert!(shell_source.contains(".padding(WORKBENCH_PADDING)"));
    assert!(style_source.contains("pub(crate) fn babel_editor_card_style"));
    assert!(style_source.contains("pub(crate) fn babel_transcript_card_style"));
    assert!(shell_source.contains(".style(babel_editor_card_style)"));
    assert!(shell_source.contains(".style(app_background_style)"));
    assert!(style_source.contains("app_background_style(_theme: &Theme) -> container::Style {\n    container::Style::default().background(Color::WHITE)"));
    assert!(
        !style_source
            .contains("container::Style::default().background(Color::from_rgb8(246, 248, 251))")
    );
    assert!(!shell_source.contains(".style(panel_style)"));
    assert!(
        !shell_source.contains("transport_strip::view_transport_strip(state, project, editor)")
    );
    assert!(style_source.contains("pub(crate) fn editor_band_style"));
    assert!(toolbar_source.contains(".style(editor_band_style)"));
    assert!(!toolbar_source.contains(".style(panel_style)"));
    assert!(!toolbar_source.contains(".style(subtle_panel_style)"));
    assert!(timeline_source.contains(".style(editor_band_style)"));
    assert!(
        transcript_source.contains("transport_strip::view_transport_strip(state, project, editor)")
    );
    assert!(!transcript_source.contains("context_panels::view_context_panels(state, project)"));
    assert!(!transcript_source.contains("row![table, context_panels]"));
    assert!(transcript_source.contains(".style(babel_transcript_card_style)"));
    assert!(!transcript_source.contains(".style(editor_band_style)"));
    assert!(!transport_source.contains(".style(editor_band_style)"));
}

#[test]
fn app_window_defaults_to_recovered_babel_workbench_size() {
    let source = include_str!("mod.rs");

    assert!(source.contains("window::Settings"));
    assert!(source.contains("size: Size::new(1440.0, 900.0)"));
    assert!(source.contains("min_size: Some(Size::new(1200.0, 760.0))"));
}

#[test]
fn app_enables_antialiasing_for_native_canvas_rendering_smoothness() {
    let source = include_str!("mod.rs");

    assert!(source.contains(".antialiasing(true)"));
}

#[test]
fn workbench_shell_smoke_can_capture_default_recovered_layout() {
    let app_source = include_str!("mod.rs");
    let action_source = include_str!("actions.rs");
    let hotkey_source = include_str!("hotkeys.rs");
    let smoke_source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app/workbench_shell_smoke.rs"),
    )
    .expect("workbench shell smoke module should exist");

    assert!(app_source.contains("mod workbench_shell_smoke;"));
    assert!(hotkey_source.contains("workbench_shell_smoke_enabled"));
    assert!(hotkey_source.contains("|| workbench_shell_smoke_enabled()"));
    assert!(action_source.contains("workbench_shell_smoke_enabled()"));
    assert!(action_source.contains("workbench_shell_renderer_screenshot_task()"));
    assert!(smoke_source.contains("STUDIO_SMOKE_WORKBENCH_SHELL_ON_START"));
    assert!(smoke_source.contains("runtime-workbench-shell-smoke"));
    assert!(smoke_source.contains("workbench-shell-renderer.bmp"));
}

#[test]
fn transport_subscription_uses_sixteen_millisecond_cadence_for_smooth_playhead() {
    let hotkey_source = include_str!("hotkeys.rs");
    let action_source = include_str!("actions.rs");
    let message_source = include_str!("message.rs");

    assert!(hotkey_source.contains("TRANSPORT_TICK_MILLIS"));
    assert!(hotkey_source.contains("time::every(Duration::from_millis(TRANSPORT_TICK_MILLIS))"));
    assert!(
        !hotkey_source.contains("window::frames().map(Message::TransportTick)"),
        "transport playback should not force full-window redraws at high-refresh display cadence"
    );
    assert!(!hotkey_source.contains("Duration::from_millis(100)"));
    assert!(message_source.contains("TransportTick(Instant)"));
    assert!(action_source.contains("advance_transport_tick"));
}

#[test]
fn editor_autosave_subscription_uses_recovered_interval_and_save_tick() {
    let hotkey_source = include_str!("hotkeys.rs");
    let action_source = include_str!("actions.rs");
    let message_source = include_str!("message.rs");
    let io_source = include_str!("io_actions.rs");

    assert_eq!(AUTO_SAVE_TICK_MILLIS, 60_000);
    assert!(hotkey_source.contains("AUTO_SAVE_TICK_MILLIS"));
    assert!(hotkey_source.contains("STUDIO_AUTOSAVE_TICK_MILLIS"));
    assert!(hotkey_source.contains("auto_save_tick_millis()"));
    assert!(hotkey_source.contains("should_run_editor_auto_save_tick"));
    assert!(message_source.contains("AutoSaveTick"));
    assert!(action_source.contains("Message::AutoSaveTick"));
    assert!(action_source.contains("auto_save_progress"));
    assert!(io_source.contains("should_run_auto_save_tick(self.submit_is_pending)"));
    assert!(io_source.contains("STUDIO_TRACE_EDITOR_AUTOSAVE"));
    assert!(io_source.contains("editor_auto_save_runtime_trace_line"));
}

#[test]
fn app_keeps_iced_wgpu_renderer_enabled_for_native_canvas_acceleration() {
    let workspace_manifest = include_str!("../../../../Cargo.toml");
    let lockfile = include_str!("../../../../Cargo.lock");

    assert!(workspace_manifest.contains("iced = { version = \"0.13\""));
    assert!(workspace_manifest.contains("\"canvas\""));
    assert!(!workspace_manifest.contains("iced = { version = \"0.13\", default-features = false"));
    assert!(lockfile.contains("name = \"iced_wgpu\""));
}

#[test]
fn app_queries_runtime_graphics_backend_for_gpu_smoke_proof() {
    let workspace_manifest = include_str!("../../../../Cargo.toml");
    let app_source = include_str!("mod.rs");
    let message_source = include_str!("message.rs");
    let state_source = include_str!("state.rs");
    let actions_source = include_str!("actions.rs");

    assert!(workspace_manifest.contains("\"system\""));
    assert!(app_source.contains("STUDIO_LOG_RUNTIME_GRAPHICS"));
    assert!(app_source.contains("runtime_graphics_info_enabled()"));
    assert!(app_source.contains("if runtime_graphics_info_enabled()"));
    assert!(app_source.contains("iced::system::fetch_information()"));
    assert!(app_source.contains("Message::RuntimeGraphicsInfoLoaded"));
    assert!(message_source.contains("RuntimeGraphicsInfoLoaded"));
    assert!(state_source.contains("runtime_graphics_info"));
    assert!(actions_source.contains("RuntimeGraphicsInfoLoaded"));
    assert!(actions_source.contains("STUDIO_LOG_RUNTIME_GRAPHICS"));
}

#[test]
fn runtime_graphics_info_is_recorded_without_replacing_editor_status() {
    let mut state = stable_sample_state();
    let initial_status = state.status.clone();

    let _ = update(
        &mut state,
        Message::RuntimeGraphicsInfoLoaded(RuntimeGraphicsInfo {
            backend: "Vulkan".to_string(),
            adapter: "Discrete GPU".to_string(),
        }),
    );

    assert_eq!(state.status, initial_status);
    assert_eq!(
        state.runtime_graphics_info,
        Some(RuntimeGraphicsInfo {
            backend: "Vulkan".to_string(),
            adapter: "Discrete GPU".to_string(),
        })
    );
}

#[test]
fn loop_strip_matches_recovered_babel_single_row_visual_contract() {
    let source = include_str!("components/timeline/loop_strip.rs");

    assert!(source.contains("LOOP_STRIP_HEIGHT"));
    assert!(source.contains("LOOP_LABEL_WIDTH"));
    assert!(source.contains("format_loop_label"));
    assert!(source.contains("LoopStripClientMetrics::for_viewport"));
    assert!(source.contains("hit_test_client_x"));
    assert!(!source.contains("button(text(\"Loop\"))"));
    assert!(!source.contains("view_loop_playhead"));
    assert!(!source.contains("\"Off\".to_string()"));
    assert!(!source.contains("hit_test(viewport, (point.x / width).clamp(0.0, 1.0))"));
}

#[test]
fn timeline_ruler_uses_recovered_wavesurfer_absolute_canvas_notches() {
    let source = include_str!("components/timeline/ruler.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");

    assert!(source.contains("struct RulerCanvas"));
    assert!(source.contains("canvas::Program<Message> for RulerCanvas"));
    assert!(source.contains("const RULER_HEIGHT: f32 = 20.0;"));
    assert!(source.contains("pub(super) fn view_lane_time_ruler"));
    assert!(waveform_source.contains("ruler::view_lane_time_ruler(editor)"));
    assert!(
        waveform_source.contains("body_height: (lane.track_height - ruler::RULER_HEIGHT).max(1.0)")
    );
    assert!(source.contains("const RULER_TEXT_INDENT_PX: f32 = 3.0;"));
    assert!(source.contains("build_babel_timeline_ruler_for_viewport"));
    assert!(source.contains("let min_px_per_sec = editor.timeline_min_px_per_sec();"));
    assert!(source.contains("self.min_px_per_sec"));
    assert!(source.contains("notch.left_px as f32"));
    assert!(source.contains("frame.fill_text(canvas::Text"));
    assert!(source.contains("canvas::Path::line"));
    assert!(source.contains("TimelineRulerNotchKind::Primary"));
    assert!(source.contains("TimelineRulerNotchKind::Secondary"));
    assert!(source.contains("TimelineRulerNotchKind::Tick"));
    assert!(!timeline_source.contains("view_time_ruler(editor)"));
    assert!(!source.contains("RULER_LABEL_RAIL_WIDTH"));
    assert!(!source.contains("container(\"\")"));
    assert!(!source.contains("Row::new().spacing(2).width(Fill)"));
    assert!(!source.contains("Length::FillPortion(1)"));
}

#[test]
fn timeline_ruler_canvas_caches_static_geometry_across_playhead_ticks() {
    let source = include_str!("components/timeline/ruler.rs");

    assert!(source.contains("struct RulerCanvasState"));
    assert!(source.contains("cache: canvas::Cache"));
    assert!(source.contains("state.cache.draw("));
    assert!(source.contains("state.clear_if_key_changed(self.cache_key())"));
    assert!(source.contains("type State = RulerCanvasState"));
    assert!(source.contains("self.viewport.start_seconds.to_bits()"));
    assert!(source.contains("self.min_px_per_sec.to_bits()"));
    assert!(!source.contains("type State = ();"));
    assert!(!source.contains("let mut frame = canvas::Frame::new(renderer, bounds.size());"));
}

#[test]
fn timeline_ruler_uses_shared_canvas_cache_group_for_moving_viewport_redraws() {
    let source = include_str!("components/timeline/ruler.rs");

    assert!(source.contains("static RULER_CANVAS_CACHE_GROUP: LazyLock<canvas::Group>"));
    assert!(source.contains("canvas::Cache::with_group(*RULER_CANVAS_CACHE_GROUP)"));
    assert!(!source.contains("view_offset_clip("));
    assert!(!source.contains("strip_width_px"));
    assert!(!source.contains("offset_x_px"));
    assert!(
        !source.contains("cache: canvas::Cache::new()"),
        "moving viewport ruler redraws should use a bounded viewport canvas instead of laying out a full-duration strip"
    );
}

#[test]
fn timeline_waveform_uses_recovered_wavesurfer_flat_cursor_surface() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let style_source = include_str!("styles.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .expect("playhead line widget should exist");

    assert!(
        waveform_source.contains("body_height: (lane.track_height - ruler::RULER_HEIGHT).max(1.0)")
    );
    assert!(waveform_source.contains("ruler::view_lane_time_ruler(editor)"));
    assert!(waveform_source.contains("view_waveform_playhead_canvas"));
    assert!(waveform_source.contains("data.playhead_percent"));
    assert!(!canvas_source.contains("struct WaveformPlayheadCanvas"));
    assert!(canvas_source.contains("view_playhead_line(playhead_percent, height, 1.0"));
    assert!(playhead_source.contains("renderer.fill_quad("));
    assert!(canvas_source.contains("Color::from_rgb8(20, 20, 20)"));
    assert!(!waveform_source.contains("model.cursor.cursor_left_percent"));
    assert!(!waveform_source.contains("body_height: (track_height - 31.0)"));
    assert!(
        !waveform_source
            .contains(".padding(8)\n    .width(Fill)\n    .height(Length::Fixed(track_height))")
    );
    assert!(style_source.contains("pub(crate) fn waveform_backdrop_style"));
    assert!(style_source.contains("border::rounded(0)"));
    assert!(!style_source.contains("pub(crate) fn playhead_style"));
    assert!(!style_source.contains("border::rounded(4)\n                .width(1)\n                .color(Color::from_rgb8(210, 224, 244))"));
}

#[test]
fn timeline_waveform_uses_recovered_wavesurfer_filled_line_canvas() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(waveform_source.contains("vertical_scale: lane.vertical_zoom"));
    assert!(!waveform_source.contains("lane.vertical_zoom / 20.0"));
    assert!(waveform_source.contains("view_wavesurfer_line_waveform"));
    assert!(canvas_source.contains("struct WaveformCanvas"));
    assert!(canvas_source.contains("impl canvas::Program<Message> for WaveformCanvas"));
    assert!(canvas_source.contains("canvas::Path::new"));
    assert!(canvas_source.contains("builder.move_to"));
    assert!(canvas_source.contains("builder.line_to"));
    assert!(canvas_source.contains("builder.close()"));
    assert!(canvas_source.contains("frame.fill(&waveform_path"));
    assert!(canvas_source.contains("fn append_wavesurfer_line_points"));
    assert!(canvas_source.contains("canvas(WaveformCanvas"));
    assert!(!waveform_source.contains("let mut row = Row::new()\n        .spacing(0)\n        .width(Fill)\n        .height(Length::Fixed(metrics.body_height));\n    for peak in peaks"));
}

#[test]
fn timeline_waveform_hot_path_borrows_peak_data_without_render_model_clones() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(canvas_source.contains("enum WaveformPeaks"));
    assert!(canvas_source.contains("peaks: WaveformPeaks"));
    assert!(canvas_source.contains("F64(Arc<[f64]>)"));
    assert!(canvas_source.contains("struct SegmentWaveformsWithRegionFillCanvas"));
    assert!(canvas_source.contains("blocks: Arc<[WaveformBlock]>"));
    assert!(
        canvas_source
            .contains("pub(super) fn view_wavesurfer_segment_waveforms_with_region_fill_canvas")
    );
    assert!(canvas_source.contains("struct WaveformCanvasState"));
    assert!(canvas_source.contains("cache: canvas::Cache"));
    assert!(canvas_source.contains("state.cache.draw("));
    assert!(canvas_source.contains("state.clear_if_key_changed(self.cache_key())"));
    assert!(waveform_source.contains("struct EngineWaveformData"));
    assert!(waveform_source.contains("let hit_blocks = Arc::clone(&data.blocks);"));
    assert!(
        waveform_source.contains("synthetic_hit_test(&hit_blocks, viewport, fraction, hit_width)")
    );
    assert!(waveform_source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas("));
    assert!(waveform_source.contains("view_wavesurfer_line_waveform_window_canvas("));
    assert!(waveform_source.contains("view_wavesurfer_region_fill_window_canvas("));
    assert!(waveform_source.contains("Arc::clone(&channel.scroll_strip_peaks)"));
    assert!(waveform_source.contains("blocks: Arc::clone(&data.blocks)"));
    assert!(!canvas_source.contains("peaks: Vec<f64>"));
    assert!(!waveform_source.contains("let waveform_model = lane.waveform_model.clone();"));
    assert!(!waveform_source.contains("let blocks = lane.blocks.clone();"));
    assert!(!waveform_source.contains("let move_model = model.clone();"));
    assert!(!waveform_source.contains("view_continuous_waveform(model.clone()"));
    assert!(!waveform_source.contains(".map(|channel| channel.peaks.clone())"));
    assert!(!waveform_source.contains(".map(|channel| channel.visible_peaks.clone())"));
}

#[test]
fn decoded_engine_region_fill_uses_full_strip_blocks_not_visible_viewport_blocks() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let engine_body = waveform_source
        .split("fn view_engine_waveform_body")
        .nth(1)
        .expect("engine waveform body should exist");

    assert!(waveform_source.contains("strip_blocks: Arc<[WaveformBlock]>"));
    assert!(engine_body.contains("blocks: Arc::clone(&data.strip_blocks)"));
    assert!(
        !engine_body.contains("blocks: Arc::clone(&data.blocks),\r\n                    color_token: data.color_token.clone(),\r\n                    metrics: data.metrics,\r\n                    strip_width_px"),
        "decoded region fill should draw from full-lane strip blocks so canvas geometry stays cached across scrolling"
    );
}

#[test]
fn timeline_lane_builder_filters_visible_segments_before_waveform_region_rendering() {
    let source = include_str!("../timeline_model/builder.rs");

    assert!(source.contains("let mut visible_segments: Vec<(Cow"));
    assert!(
        source.contains("segment_with_preview(segment, lane.id.as_str(), state.segment_preview)")
    );
    assert!(source.contains("create_preview_segment_for_lane("));
    assert!(source.contains("state.viewport.intersects(segment.as_ref())"));
    assert!(source.contains("waveform.render_model("));
    assert!(source.contains("visible_segment_refs.as_slice()"));
    assert!(source.contains("let blocks = visible_segments"));
    assert!(source.contains("strip_blocks: Option<&'a BTreeMap<String, Arc<[WaveformBlock]>>>"));
    assert!(source.contains("strip_blocks,"));
    assert!(source.contains("WaveformBlock::for_segment_with_static_source("));
    assert!(!source.contains("waveform.render_model(&segments, state.viewport"));
}

#[test]
fn timeline_lane_builder_indexes_segments_by_lane_before_visible_filtering() {
    let source = include_str!("../timeline_model/builder.rs");

    assert!(source.contains("let mut segments_by_lane"));
    assert!(source.contains(".entry(segment.lane_id.clone())"));
    assert!(source.contains("segments_by_lane.values_mut()"));
    assert!(source.contains("prefix_max_end_seconds"));
    assert!(source.contains(
        ".candidate_segment_indices_for_lane(project, lane.id.as_str(), state.viewport)"
    ));
    assert!(!source.contains(".segments\n                .iter()\n                .filter(|segment| segment.lane_id == lane.id)"));
}

#[test]
fn app_timeline_rebuilds_reuse_cached_segment_index_while_viewport_moves() {
    let state_source = include_str!("state.rs");
    let view_data_source = include_str!("../editor_model/view_data.rs");

    assert!(state_source.contains("timeline_segment_index_cache"));
    assert!(state_source.contains("TimelineSegmentIndex::for_project(project)"));
    assert!(state_source.contains("editor.timeline_lanes_with_segment_index"));
    assert!(state_source.contains("self.transcript_rows_generation"));
    assert!(view_data_source.contains("timeline_lanes_with_segment_index"));
}

#[test]
fn moving_playback_reuses_cached_timeline_segment_index_across_viewport_misses() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(cache_sized_two_lane_project()),
        "Loaded large moving-viewport project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    let duration_seconds = TimeRange::for_project(&project).end_seconds + 1.0;
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.set_lane_waveform(
            "track-2",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.playhead_seconds = 30.0;
        editor.set_zoom(&project, 1.0);
        editor.toggle_playback();
    });

    let start_tick = Instant::now();
    for frame in 0..20 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }

    assert!(
        state.timeline_lanes_cache_miss_count() > 1,
        "the test should exercise moving viewport timeline rebuilds"
    );
    assert_eq!(
        state.timeline_segment_index_cache_miss_count(),
        1,
        "viewport-only timeline rebuilds should reuse the already sorted segment index"
    );
}

#[test]
fn moving_playback_reuses_cached_project_bounds_across_viewport_misses() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(cache_sized_two_lane_project()),
        "Loaded large moving-bounds project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    let duration_seconds = TimeRange::for_project(&project).end_seconds + 1.0;
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.set_lane_waveform(
            "track-2",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.playhead_seconds = 30.0;
        editor.set_zoom(&project, 1.0);
        editor.toggle_playback();
    });

    let start_tick = Instant::now();
    for frame in 0..20 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let editor = state.editor.as_ref().unwrap();
        let _ = state.timeline_lanes_snapshot_for_render(&project, editor);
    }

    assert!(
        state.timeline_lanes_cache_miss_count() > 1,
        "the test should exercise moving viewport timeline rebuilds"
    );
    assert_eq!(
        state.timeline_project_bounds_cache_miss_count(),
        1,
        "viewport-only timeline rebuilds should reuse project bounds instead of scanning every segment"
    );
}

#[test]
fn moving_playback_reuses_static_waveform_blocks_while_visible_segments_are_unchanged() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(cache_sized_two_lane_project()),
        "Loaded large moving-waveform-block project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    let duration_seconds = TimeRange::for_project(&project).end_seconds + 1.0;
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.set_lane_waveform(
            "track-2",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.playhead_seconds = 30.25;
        editor.set_zoom(&project, 1.0);
        editor.toggle_playback();
    });

    let initial_lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let initial_segment_ids: Vec<Vec<String>> = initial_lanes
        .iter()
        .map(|lane| {
            lane.blocks
                .iter()
                .map(|block| block.segment_id.clone())
                .collect()
        })
        .collect();
    let initial_peak_stats: Vec<_> = ["track-1", "track-2"]
        .into_iter()
        .map(|lane_id| {
            state
                .editor
                .as_ref()
                .unwrap()
                .lane_waveforms
                .get(lane_id)
                .unwrap()
                .segment_peak_cache_stats()
        })
        .collect();

    let start_tick = Instant::now();
    for frame in 0..8 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            state.timeline_lanes_snapshot_for_render(&project, editor)
        };
        let segment_ids: Vec<Vec<String>> = lanes
            .iter()
            .map(|lane| {
                lane.blocks
                    .iter()
                    .map(|block| block.segment_id.clone())
                    .collect()
            })
            .collect();
        assert_eq!(
            segment_ids, initial_segment_ids,
            "test setup should keep the same visible segment set while the viewport moves"
        );
    }

    assert!(
        state.timeline_lanes_cache_miss_count() > 1,
        "the test should exercise viewport-driven timeline lane rebuilds"
    );
    let final_peak_stats: Vec<_> = ["track-1", "track-2"]
        .into_iter()
        .map(|lane_id| {
            state
                .editor
                .as_ref()
                .unwrap()
                .lane_waveforms
                .get(lane_id)
                .unwrap()
                .segment_peak_cache_stats()
        })
        .collect();
    assert_eq!(
        final_peak_stats, initial_peak_stats,
        "moving playback should reuse cached static waveform block data instead of touching segment peak caches every frame"
    );
}

#[test]
fn moving_playback_viewport_reuses_decoded_timeline_peak_arc_while_visible_segments_unchanged() {
    let mut state = stable_sample_state();
    let project = cache_sized_two_lane_project();
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large moving-waveform-arc project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    let duration_seconds = TimeRange::for_project(&project).end_seconds + 1.0;
    state.with_editor_mut(|editor| {
        editor.set_lane_waveform(
            "track-1",
            LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
        );
        editor.playhead_seconds = 30.25;
        editor.set_zoom(&project, 1.0);
        editor.toggle_playback();
    });

    let initial_lanes = {
        let editor = state.editor.as_ref().unwrap();
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let initial_lane = initial_lanes
        .iter()
        .find(|lane| lane.lane_id == "track-1")
        .expect("track-1 should render");
    let initial_model = initial_lane
        .waveform_model
        .as_ref()
        .expect("decoded track should render a waveform model");
    let initial_segment_ids: Vec<_> = initial_lane
        .blocks
        .iter()
        .map(|block| block.segment_id.clone())
        .collect();
    let initial_visible_start = initial_model.visible_start_seconds;
    let initial_peaks = std::sync::Arc::clone(&initial_model.channels[0].peaks);
    let initial_visible_peaks = std::sync::Arc::clone(&initial_model.channels[0].visible_peaks);
    let initial_scroll_strip_peaks =
        std::sync::Arc::clone(&initial_model.channels[0].scroll_strip_peaks);
    let initial_render_stats = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .render_cache_stats();

    let start_tick = Instant::now();
    let mut final_visible_start = initial_visible_start;
    for frame in 0..8 {
        let _ = update(
            &mut state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame)),
        );
        let lanes = {
            let editor = state.editor.as_ref().unwrap();
            state.timeline_lanes_snapshot_for_render(&project, editor)
        };
        let lane = lanes
            .iter()
            .find(|lane| lane.lane_id == "track-1")
            .expect("track-1 should still render while the viewport follows playback");
        let model = lane
            .waveform_model
            .as_ref()
            .expect("decoded track should keep rendering while the viewport follows playback");
        let segment_ids: Vec<_> = lane
            .blocks
            .iter()
            .map(|block| block.segment_id.clone())
            .collect();
        assert_eq!(
            segment_ids, initial_segment_ids,
            "test setup should keep the same visible segment set while the viewport moves"
        );
        assert!(
            std::sync::Arc::ptr_eq(&initial_peaks, &model.channels[0].peaks),
            "moving viewport playback should reuse decoded timeline peak storage instead of allocating a new Arc each frame"
        );
        assert!(
            std::sync::Arc::ptr_eq(&initial_visible_peaks, &model.channels[0].visible_peaks),
            "the native offset-clipped WaveSurfer strip should not resample the unused visible_peaks buffer on every playback tick"
        );
        assert!(
            std::sync::Arc::ptr_eq(
                &initial_scroll_strip_peaks,
                &model.channels[0].scroll_strip_peaks
            ),
            "moving viewport playback should shift the cached WaveSurfer-style strip instead of replacing the expensive continuous waveform path every frame; frame={frame} initial_len={} next_len={} initial_strip={:.3}-{:.3} next_strip={:.3}-{:.3} visible={:.3}-{:.3}",
            initial_scroll_strip_peaks.len(),
            model.channels[0].scroll_strip_peaks.len(),
            initial_model.channels[0].scroll_strip_start_seconds,
            initial_model.channels[0].scroll_strip_end_seconds,
            model.channels[0].scroll_strip_start_seconds,
            model.channels[0].scroll_strip_end_seconds,
            model.visible_start_seconds,
            model.visible_end_seconds,
        );
        final_visible_start = model.visible_start_seconds;
    }

    assert!(
        final_visible_start > initial_visible_start,
        "the regression must exercise a real moving viewport, not just playhead-only ticks"
    );
    let final_render_stats = state
        .editor
        .as_ref()
        .unwrap()
        .lane_waveforms
        .get("track-1")
        .unwrap()
        .render_cache_stats();
    assert_eq!(
        final_render_stats.misses, initial_render_stats.misses,
        "moving playback should retarget the cached decoded timeline render model while visible segments and zoom are unchanged"
    );
    assert!(
        final_render_stats.hits > initial_render_stats.hits,
        "moving playback should record timeline render cache hits instead of rebuilding the model every frame"
    );
}

#[test]
fn lane_waveform_keeps_audio_strip_cache_independent_of_regions() {
    let source = include_str!("../timeline_model/waveform.rs");

    assert!(source.contains("let _ = segments;"));
    assert!(source.contains("WaveformRenderCacheKey::timeline(viewport)"));
    assert!(source.contains("with_visible_window_for_scroll_strip("));
    assert!(source.contains("&[],"));
    assert!(!source.contains("region_cache: RefCell<TimelineRegionCache>"));
    assert!(!source.contains("regions_for_segments(&self.source, segments)"));
    assert!(!source.contains("struct TimelineRegionCacheKey"));
    assert!(!source.contains("RegionsPlugin::new(self.source.duration()"));
}

#[test]
fn wavesurfer_canvas_streams_path_points_without_frame_point_vectors() {
    let source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("append_wavesurfer_line_points("));
    assert!(source.contains("builder: &mut canvas::path::Builder"));
    assert!(!source.contains("fn wavesurfer_line_points("));
    assert!(!source.contains("-> [Vec<Point>; 2]"));
    assert!(!source.contains("let mut top = vec!["));
    assert!(!source.contains("let mut bottom = vec!["));
}

#[test]
fn wavesurfer_canvas_streams_block_widths_without_frame_vec_allocation() {
    let source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("struct SyntheticWaveformBlockWidths<'a>"));
    assert!(source.contains("impl Iterator for SyntheticWaveformBlockWidths<'_>"));
    assert!(source.contains("struct OverviewBlockWidths<'a>"));
    assert!(source.contains("impl Iterator for OverviewBlockWidths<'_>"));
    assert!(
        !source.contains("-> Vec<SyntheticWaveformBlockWidth>"),
        "waveform canvas draws should stream block widths instead of allocating a Vec each frame"
    );
    assert!(!source.contains("let mut widths = Vec::with_capacity(blocks.len())"));
}

#[test]
fn wavesurfer_canvas_uses_shared_cache_group_for_moving_viewport_redraw_storage() {
    let source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("LazyLock<canvas::Group>"));
    assert!(source.contains("WAVEFORM_CANVAS_CACHE_GROUP"));
    assert!(source.contains("canvas::Group::unique"));
    assert!(source.contains("canvas::Cache::with_group(*WAVEFORM_CANVAS_CACHE_GROUP)"));
    assert!(
        !source.contains("cache: canvas::Cache::new()"),
        "moving waveform redraws should reuse backend cache storage instead of allocating isolated canvas caches"
    );
}

#[test]
fn wavesurfer_canvas_specializes_peak_sampling_outside_point_loop() {
    let source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("append_wavesurfer_line_points_f64("));
    assert!(source.contains("append_wavesurfer_line_points_f32_scaled_at("));
    assert!(source.contains("append_wavesurfer_line_points_with("));
    assert!(!source.contains("fn abs_clamped(&self, index: usize)"));
    assert!(!source.contains("peaks.abs_clamped(index)"));
}

#[test]
fn decoded_wavesurfer_visible_samples_are_reused_for_top_and_bottom_paths() {
    let source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("WaveformPeaks::F32"));
    assert!(source.contains("fn append_wavesurfer_line_points_f32("));
    assert!(
        source.contains("|index| peaks.get(index).copied().unwrap_or(0.0)"),
        "decoded waveform draws should reuse the pre-sampled visible peak values for both polarities"
    );
}

#[test]
fn decoded_wavesurfer_canvas_draws_presampled_visible_peaks_without_window_sampling() {
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let render_model_source = include_str!("../../../studio-waveform/src/render_model.rs");

    assert!(render_model_source.contains("visible_peaks: Arc<[f32]>"));
    assert!(
        !canvas_source.contains("fn decoded_window_peak_samples("),
        "decoded waveform sampling belongs in the render model cache path, not in canvas draw"
    );
    assert!(!canvas_source.contains("peak_window_abs_max("));
    assert!(canvas_source.contains("WaveformPeaks::F32"));
}

#[test]
fn decoded_timeline_waveform_draws_bounded_viewport_from_full_strip_cache() {
    let workspace_manifest = include_str!("../../../../Cargo.toml");
    let components_source = include_str!("components/mod.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(workspace_manifest.contains("\"advanced\""));
    assert!(!components_source.contains("mod offset_clip;"));
    assert!(waveform_source.contains("struct EngineWaveformStripDependency"));
    assert!(waveform_source.contains("struct EngineRegionFillDependency"));
    assert!(waveform_source.contains("view_wavesurfer_line_waveform_window_canvas("));
    assert!(waveform_source.contains("view_wavesurfer_region_fill_window_canvas("));
    assert!(waveform_source.contains("view_lazy_engine_waveform_strip("));
    assert!(waveform_source.contains("view_lazy_engine_region_fill_strip("));
    assert!(canvas_source.contains("struct WaveformWindowCanvas"));
    assert!(canvas_source.contains("struct PeakTimeWindow"));
    assert!(!waveform_source.contains("view_offset_clipped_waveform_strip("));
    assert!(!waveform_source.contains("view_offset_clipped_region_fill_strip("));
    assert!(!waveform_source.contains("strip_width_px"));
    assert!(
        !waveform_source.contains("view_lazy_engine_base_waveform("),
        "decoded playback should not combine the expensive continuous path with every-frame region fill invalidation"
    );
}

#[test]
fn moving_playhead_uses_direct_quad_widget_instead_of_canvas_geometry() {
    let components_source = include_str!("components/mod.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .unwrap_or_default();

    assert!(components_source.contains("mod playhead_line;"));
    assert!(canvas_source.contains("view_playhead_line("));
    assert!(!canvas_source.contains("struct OverviewPlayheadCanvas"));
    assert!(!canvas_source.contains("struct WaveformPlayheadCanvas"));
    assert!(playhead_source.contains("renderer.fill_quad("));
    assert!(
        !playhead_source.contains("canvas("),
        "moving playhead lines should bypass Canvas geometry allocation and draw a direct renderer quad"
    );
}

#[test]
fn timeline_waveform_static_layers_are_lazy_cached_across_playhead_ticks() {
    let waveform_source = include_str!("components/timeline/waveform.rs");

    assert!(waveform_source.contains("lazy,"));
    assert!(waveform_source.contains("struct SyntheticBaseWaveformDependency"));
    assert!(waveform_source.contains("struct EngineWaveformStripDependency"));
    assert!(waveform_source.contains("struct EngineRegionFillDependency"));
    assert!(waveform_source.contains("struct RegionAffordanceOverlayDependency"));
    assert!(waveform_source.contains("impl std::hash::Hash for SyntheticBaseWaveformDependency"));
    assert!(waveform_source.contains("impl std::hash::Hash for EngineWaveformStripDependency"));
    assert!(waveform_source.contains("impl std::hash::Hash for EngineRegionFillDependency"));
    assert!(waveform_source.contains("impl std::hash::Hash for RegionAffordanceOverlayDependency"));
    assert!(waveform_source.contains("view_lazy_synthetic_base_waveform("));
    assert!(waveform_source.contains("view_lazy_engine_waveform_strip("));
    assert!(waveform_source.contains("view_lazy_engine_region_fill_strip("));
    assert!(waveform_source.contains("view_lazy_region_affordance_overlay("));
    assert!(waveform_source.contains("Arc::as_ptr(&self.scroll_strip_peaks)"));
    assert!(waveform_source.contains("metrics.body_height.to_bits()"));
}

#[test]
fn timeline_region_fill_layer_stays_cached_when_playback_selection_changes() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let synthetic_base_dependency = waveform_source
        .split("struct SyntheticBaseWaveformDependency")
        .nth(1)
        .expect("timeline waveform should define a synthetic base dependency")
        .split("struct EngineWaveformStripDependency")
        .next()
        .expect("synthetic base dependency should precede engine base dependency");
    let engine_strip_dependency = waveform_source
        .split("struct EngineWaveformStripDependency")
        .nth(1)
        .expect("timeline waveform should define an engine strip dependency")
        .split("struct EngineRegionFillDependency")
        .next()
        .expect("engine strip dependency should precede engine region dependency");
    let engine_region_dependency = waveform_source
        .split("struct EngineRegionFillDependency")
        .nth(1)
        .expect("timeline waveform should define an engine region dependency")
        .split("struct SelectedRegionBorderDependency")
        .next()
        .expect("engine region dependency should precede selected border dependency");
    let synthetic_base_hash = waveform_source
        .split("impl std::hash::Hash for SyntheticBaseWaveformDependency")
        .nth(1)
        .expect("timeline waveform should hash synthetic base dependency")
        .split("impl std::hash::Hash for EngineWaveformStripDependency")
        .next()
        .expect("synthetic base hash should precede engine base hash");
    let engine_strip_hash = waveform_source
        .split("impl std::hash::Hash for EngineWaveformStripDependency")
        .nth(1)
        .expect("timeline waveform should hash engine strip dependency")
        .split("impl std::hash::Hash for EngineRegionFillDependency")
        .next()
        .expect("engine strip hash should precede engine region hash");
    let engine_region_hash = waveform_source
        .split("impl std::hash::Hash for EngineRegionFillDependency")
        .nth(1)
        .expect("timeline waveform should hash engine region dependency")
        .split("impl std::hash::Hash for SelectedRegionBorderDependency")
        .next()
        .expect("engine region hash should precede selected border hash");

    assert!(
        !synthetic_base_dependency.contains("selected_segment_id")
            && !engine_strip_dependency.contains("selected_segment_id")
            && !engine_region_dependency.contains("selected_segment_id"),
        "the heavy waveform/region base layer must not rebuild every time playback selects a new segment"
    );
    assert!(
        !synthetic_base_hash.contains("selected_segment_id")
            && !engine_strip_hash.contains("selected_segment_id")
            && !engine_region_hash.contains("selected_segment_id"),
        "lazy hashing for waveform/region fill must ignore playback selection changes"
    );
    assert!(waveform_source.contains("view_lazy_selected_region_border_overlay("));
    assert!(canvas_source.contains("view_wavesurfer_line_waveform_window_canvas"));
    assert!(canvas_source.contains("view_wavesurfer_region_fill_window_canvas"));
    assert!(canvas_source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas"));
    assert!(canvas_source.contains("view_waveform_selected_region_border_canvas"));
}

#[test]
fn selected_region_border_cache_ignores_rebuilt_block_arc_when_geometry_is_same() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let selected_dependency_hash = waveform_source
        .split("impl std::hash::Hash for SelectedRegionBorderDependency")
        .nth(1)
        .expect("timeline waveform should hash selected border dependency")
        .split("impl std::hash::Hash for RegionAffordanceOverlayDependency")
        .next()
        .expect("selected border hash should precede affordance hash");
    let selected_canvas_cache = canvas_source
        .split("impl SelectedRegionBorderCanvas")
        .nth(1)
        .expect("selected border canvas should define cache key")
        .split("impl canvas::Program<Message> for OverviewBlocksCanvas")
        .next()
        .expect("selected border cache key should precede overview canvas");

    assert!(
        !selected_dependency_hash.contains("Arc::as_ptr(&self.blocks)"),
        "selected region border should stay cached when moving viewport rebuilds an equivalent block array"
    );
    assert!(
        !selected_canvas_cache.contains("Arc::as_ptr(&self.blocks)"),
        "selected border canvas cache should use block geometry, not the transient Arc allocation"
    );
    assert!(selected_dependency_hash.contains("block.left_percent.to_bits()"));
    assert!(selected_canvas_cache.contains("block.left_percent.to_bits()"));
}

#[test]
fn synthetic_waveform_base_cache_ignores_rebuilt_block_arc_when_geometry_and_peaks_are_same() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let synthetic_base_hash = waveform_source
        .split("impl std::hash::Hash for SyntheticBaseWaveformDependency")
        .nth(1)
        .expect("timeline waveform should hash synthetic base dependency")
        .split("impl std::hash::Hash for EngineWaveformStripDependency")
        .next()
        .expect("synthetic base hash should precede engine base hash");
    let segment_canvas_cache = canvas_source
        .split("impl SegmentWaveformsWithRegionFillCanvas")
        .nth(1)
        .expect("segment waveform canvas should define cache key")
        .split("impl canvas::Program<Message> for SelectedRegionBorderCanvas")
        .next()
        .expect("segment waveform cache key should precede selected border canvas");

    assert!(
        !synthetic_base_hash.contains("Arc::as_ptr(&self.blocks)"),
        "synthetic waveform base should stay cached when the visible block array is rebuilt with equivalent geometry"
    );
    assert!(
        !segment_canvas_cache.contains("Arc::as_ptr(&self.blocks)"),
        "synthetic waveform canvas should key blocks by draw-affecting geometry and peak buffers, not the transient block array"
    );
    assert!(synthetic_base_hash.contains("block.left_percent.to_bits()"));
    assert!(synthetic_base_hash.contains("Arc::as_ptr(&block.peaks)"));
    assert!(segment_canvas_cache.contains("block.left_percent.to_bits()"));
    assert!(segment_canvas_cache.contains("Arc::as_ptr(&block.peaks)"));
}

#[test]
fn region_affordance_overlay_cache_ignores_rebuilt_block_arc_when_geometry_is_same() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let affordance_dependency_hash = waveform_source
        .split("impl std::hash::Hash for RegionAffordanceOverlayDependency")
        .nth(1)
        .expect("timeline waveform should hash affordance dependency")
        .split("#[derive(Clone, Debug, PartialEq)]")
        .next()
        .expect("affordance hash should precede hover overlay type");

    assert!(
        !affordance_dependency_hash.contains("Arc::as_ptr(&self.blocks)"),
        "selected/hovered affordances should stay cached when moving viewport rebuilds an equivalent block array"
    );
    assert!(affordance_dependency_hash.contains("block.left_percent.to_bits()"));
    assert!(affordance_dependency_hash.contains("block.width_percent.to_bits()"));
    assert!(affordance_dependency_hash.contains("block.tooltip.hash(state)"));
}

#[test]
fn timeline_waveform_skips_invisible_selection_and_affordance_layers_per_lane() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(waveform_source.contains("fn visible_segment_id_for_blocks("));
    assert!(
        waveform_source
            .contains("visible_segment_id_for_blocks(&data.blocks, data.selected_segment_id"),
        "timeline lanes should compute per-lane selected visibility before pushing overlays"
    );
    assert!(
        waveform_source.contains("if let Some(selected_segment_id) = selected_segment_id.clone()"),
        "timeline lanes should only push the selected border layer when the selected segment is visible in that lane"
    );
    assert!(
        waveform_source.contains(
            "if !data.suppress_region_affordances\n            && (selected_segment_id.is_some() || hovered_segment_id.is_some())"
        ),
        "timeline lanes should skip invisible affordance widget overlays when no selected or hovered region is visible"
    );
    assert!(
        !waveform_source.contains("selected_segment_id: data.selected_segment_id.clone()"),
        "unmatched lane selections should not invalidate every lane's overlay dependencies"
    );
    assert!(canvas_source.contains("selected_segment_id: String"));
    assert!(!canvas_source.contains("selected_segment_id: Option<String>"));
    assert!(!canvas_source.contains("let Some(selected_segment_id) = self.selected_segment_id"));
}

#[test]
fn timeline_waveform_preallocates_layer_stack_for_moving_playback() {
    let waveform_source = include_str!("components/timeline/waveform.rs");

    assert!(waveform_source.contains("const TIMELINE_WAVEFORM_LAYER_CAPACITY: usize = 11;"));
    assert!(
        waveform_source.contains("Vec::with_capacity(TIMELINE_WAVEFORM_LAYER_CAPACITY)"),
        "moving playback should not grow the timeline waveform layer Vec by repeated reallocations"
    );
    assert!(!waveform_source.contains("let mut layers = vec![view_lazy_synthetic_base_waveform"));
    assert!(!waveform_source.contains("let mut layers = vec![view_lazy_engine_waveform_strip"));
}

#[test]
fn timeline_waveform_moving_viewport_splits_cached_strip_from_region_fill() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(waveform_source.contains("struct EngineWaveformStripDependency"));
    assert!(waveform_source.contains("struct EngineRegionFillDependency"));
    assert!(waveform_source.contains("struct SyntheticBaseWaveformDependency"));
    assert!(waveform_source.contains("view_wavesurfer_line_waveform_window_canvas("));
    assert!(waveform_source.contains("view_wavesurfer_region_fill_window_canvas("));
    assert!(!waveform_source.contains("view_offset_clipped_waveform_strip("));
    assert!(!waveform_source.contains("view_offset_clipped_region_fill_strip("));
    assert!(waveform_source.contains("view_lazy_engine_waveform_strip("));
    assert!(waveform_source.contains("view_lazy_engine_region_fill_strip("));
    assert!(waveform_source.contains("view_lazy_synthetic_base_waveform("));
    assert!(canvas_source.contains("view_wavesurfer_line_waveform_window_canvas"));
    assert!(canvas_source.contains("view_wavesurfer_region_fill_window_canvas"));
    assert!(canvas_source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas"));
    assert!(canvas_source.contains("struct RegionFillCanvas"));
    assert!(canvas_source.contains("struct SegmentWaveformsWithRegionFillCanvas"));
    assert!(!waveform_source.contains("view_lazy_continuous_waveform("));
    assert!(!waveform_source.contains("view_lazy_synthetic_segment_waveforms("));
}

#[test]
fn timeline_waveform_playhead_uses_direct_quad_without_fillportion_row() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .expect("playhead line widget should exist");
    let style_source = include_str!("styles.rs");

    assert!(waveform_source.contains("view_waveform_playhead_canvas("));
    assert!(canvas_source.contains("pub(super) fn view_waveform_playhead_canvas"));
    assert!(canvas_source.contains("view_playhead_line(playhead_percent, height, 1.0"));
    assert!(!canvas_source.contains("struct WaveformPlayheadCanvas"));
    assert!(!canvas_source.contains("impl canvas::Program<Message> for WaveformPlayheadCanvas"));
    assert!(playhead_source.contains("renderer.fill_quad("));
    assert!(playhead_source.contains("fn playhead_pixel_x("));
    assert!(canvas_source.contains("Color::from_rgb8(20, 20, 20)"));
    assert!(!waveform_source.contains("fn view_playhead_cursor("));
    assert!(!waveform_source.contains("playhead_style"));
    assert!(!style_source.contains("pub(crate) fn playhead_style"));
}

#[test]
fn moving_playhead_direct_quad_draws_without_canvas_cache_churn() {
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .expect("playhead line widget should exist");

    assert!(
        playhead_source.contains("renderer.fill_quad("),
        "moving playheads should bypass canvas cache invalidation and draw their tiny rect directly"
    );
    assert!(!canvas_source.contains("struct OverviewPlayheadCanvas"));
    assert!(!canvas_source.contains("struct WaveformPlayheadCanvas"));
    assert!(!canvas_source.contains("fn draw_uncached_playhead_geometry("));
    assert!(!playhead_source.contains("canvas::Cache"));
    assert!(!playhead_source.contains("cache_key("));
}

#[test]
fn moving_playhead_uses_absolute_pixel_fraction_without_fillportion_quantization() {
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .expect("playhead line widget should exist");

    assert!(playhead_source.contains("fn playhead_pixel_x("));
    assert!(canvas_source.contains("view_playhead_line(playhead_percent, 30.0, 2.0"));
    assert!(canvas_source.contains("view_playhead_line(playhead_percent, height, 1.0"));
    assert!(
        !playhead_source.contains("portion(playhead_percent"),
        "moving playheads must use absolute pixel fractions; FillPortion-style rounding makes native playback look like a low-FPS jump"
    );
}

#[test]
fn timeline_view_uses_shared_lane_snapshot_across_playhead_ticks() {
    let state_source = include_str!("state.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let lane_source = include_str!("components/timeline/lane.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");

    assert!(state_source.contains("lanes: Arc<[TimelineLaneView]>"));
    assert!(state_source.contains("pub(super) fn timeline_lanes_snapshot_for_render"));
    assert!(state_source.contains("Arc::clone(&entry.lanes)"));
    assert!(!state_source.contains("timeline_lanes_with_playhead(&entry.lanes"));
    assert!(timeline_source.contains("state.timeline_lanes_snapshot_for_render"));
    assert!(timeline_source.contains("for lane_view in lane_views.iter()"));
    assert!(lane_source.contains("lane: &TimelineLaneView"));
    assert!(waveform_source.contains("lane: &TimelineLaneView"));
    assert!(waveform_source.contains("data.playhead_percent"));
    assert!(!waveform_source.contains("model.cursor.cursor_left_percent"));
}

#[test]
fn timeline_lane_control_rail_is_lazy_cached_across_playhead_ticks() {
    let lane_source = include_str!("components/timeline/lane.rs");

    assert!(lane_source.contains("lazy,"));
    assert!(lane_source.contains("struct LaneControlRailDependency"));
    assert!(lane_source.contains("impl std::hash::Hash for LaneControlRailDependency"));
    assert!(lane_source.contains("fn view_lazy_lane_control_rail("));
    assert!(lane_source.contains("lazy(dependency, view_lane_control_rail_content)"));
    assert!(lane_source.contains("track_height_bits"));
    assert!(lane_source.contains("vertical_zoom_bits"));
}

#[test]
fn top_toolbar_overview_uses_shared_snapshot_across_playhead_ticks() {
    let state_source = include_str!("state.rs");
    let toolbar_source = include_str!("components/toolbar.rs");

    assert!(state_source.contains("overview: Arc<OverviewWaveformView>"));
    assert!(state_source.contains("pub(super) struct OverviewWaveformSnapshot"));
    assert!(state_source.contains("pub(super) fn overview_waveform_snapshot_for_render"));
    assert!(state_source.contains("Arc::clone(&entry.overview)"));
    assert!(!state_source.contains("overview_waveform_with_playhead("));
    assert!(!state_source.contains("model.with_playhead_cursor(playhead_seconds, 2.0)"));
    assert!(toolbar_source.contains("state.overview_waveform_snapshot_for_render"));
    assert!(toolbar_source.contains("overview_playhead_percent("));
    assert!(!toolbar_source.contains("state.overview_waveform_for_render(project, editor)"));
}

#[test]
fn top_toolbar_overview_preallocates_layer_stack_for_moving_playback() {
    let toolbar_source = include_str!("components/toolbar.rs");

    assert!(toolbar_source.contains("const OVERVIEW_WAVEFORM_LAYER_CAPACITY: usize = 3;"));
    assert!(
        toolbar_source.contains("Vec::with_capacity(OVERVIEW_WAVEFORM_LAYER_CAPACITY)"),
        "moving playback should not grow the overview waveform layer Vec every frame"
    );
    assert!(!toolbar_source.contains("stack(vec![\n            view_overview_peaks("));
}

#[test]
fn transcript_table_hot_path_borrows_visible_row_models_without_cloning() {
    let table_source = include_str!("components/transcript_table/mod.rs");
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(table_source.contains("for row_view in row_views"));
    assert!(table_source.contains("view_transcript_row("));
    assert!(table_source.contains("view_modified_transcript_row(row_view)"));
    assert!(!table_source.contains(".take(virtual_window.rendered_len())\n            .cloned()"));
    assert!(row_source.contains("row_view: &TranscriptRowView"));
    assert!(row_source.contains("pub(super) fn view_modified_transcript_row("));
    assert!(row_source.contains("low_confidence_word_tokens(&row_view.word_views)"));
    assert!(!row_source.contains("row_view.word_views.clone()"));
}

#[test]
fn transcript_table_action_cells_are_lazy_cached_across_playback_ticks() {
    let actions_source = include_str!("components/transcript_table/actions.rs");

    assert!(actions_source.contains("lazy,"));
    assert!(actions_source.contains("struct ActionCellDependency"));
    assert!(actions_source.contains("impl std::hash::Hash for ActionCellDependency"));
    assert!(actions_source.contains("fn view_lazy_action_cell("));
    assert!(actions_source.contains("lazy(dependency, view_action_cell_content)"));
    assert!(actions_source.contains("menu: Option<TranscriptActionMenuView>"));
    assert!(
        actions_source
            .contains("is_open.then(|| editor.transcript_action_menu(project, row_view))")
    );
}

#[test]
fn transcript_table_lint_cells_are_lazy_cached_across_playback_ticks() {
    let lint_source = include_str!("components/transcript_table/lint.rs");

    assert!(lint_source.contains("lazy,"));
    assert!(lint_source.contains("struct LintCellDependency"));
    assert!(lint_source.contains("impl std::hash::Hash for LintCellDependency"));
    assert!(lint_source.contains("fn view_lazy_lint_cell("));
    assert!(lint_source.contains("lazy(dependency, view_lint_cell_content)"));
    assert!(lint_source.contains("hash_lint_severity("));
    assert!(lint_source.contains("can_toggle_assertion"));
}

#[test]
fn transcript_table_speaker_label_cells_are_lazy_cached_across_playback_ticks() {
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(row_source.contains("lazy,"));
    assert!(row_source.contains("struct SpeakerLabelDependency"));
    assert!(row_source.contains("impl std::hash::Hash for SpeakerLabelDependency"));
    assert!(row_source.contains("fn view_lazy_speaker_label("));
    assert!(row_source.contains("lazy(dependency, view_speaker_label_content)"));
    assert!(row_source.contains("speaker_label_color_for("));
}

#[test]
fn transcript_table_id_cells_are_lazy_cached_across_playback_ticks() {
    let row_source = include_str!("components/transcript_table/row.rs");

    assert!(row_source.contains("struct IdCellDependency"));
    assert!(row_source.contains("impl std::hash::Hash for IdCellDependency"));
    assert!(row_source.contains("fn view_lazy_id_cell("));
    assert!(row_source.contains("lazy(dependency, view_id_cell_content)"));
}

#[test]
fn timeline_waveform_regions_are_flat_wavesurfer_overlays_not_peak_bars() {
    let source = include_str!("components/timeline/waveform.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(source.contains("view_wavesurfer_line_waveform_window_canvas"));
    assert!(source.contains("view_wavesurfer_region_fill_window_canvas"));
    assert!(source.contains("view_wavesurfer_segment_waveforms_with_region_fill_canvas"));
    assert!(source.contains("view_waveform_selected_region_border_canvas"));
    assert!(source.contains("view_waveform_region_affordance_block"));
    assert!(canvas_source.contains("struct RegionFillCanvas"));
    assert!(canvas_source.contains("struct SegmentWaveformsWithRegionFillCanvas"));
    assert!(canvas_source.contains("struct SelectedRegionBorderCanvas"));
    assert!(!source.contains("let peak_values = block.peaks"));
    assert!(!source.contains("fn scaled_peak_height"));
    assert!(!source.contains("fn centered_peak_spacers"));
}

#[test]
fn timeline_waveform_region_base_canvas_draws_recovered_babel_edges() {
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(canvas_source.contains("border_color: waveform_region_border_color(color_token)"));
    assert!(canvas_source.contains("fn waveform_region_border_color(color_token: &str) -> Color"));
    assert!(canvas_source.contains("draw_region_fills("));
    assert!(canvas_source.contains("draw_region_fills_for_time_window("));
    assert!(
        canvas_source.contains("frame.stroke(\n            &region_path,"),
        "normal regions should always draw a solid Babel-style edge, not only a hover affordance"
    );
    assert!(canvas_source.contains(".with_width(WAVEFORM_REGION_BORDER_WIDTH)"));
    assert!(canvas_source.contains(".with_color(border_color)"));
    assert!(canvas_source.contains("hash_color(key, &self.border_color)"));
}

#[test]
fn timeline_waveform_resize_affordance_does_not_paint_extra_handle_pills() {
    let waveform_source = include_str!("components/timeline/waveform.rs");

    let affordance_block = waveform_source
        .split("fn view_waveform_region_affordance_block")
        .nth(1)
        .expect("timeline waveform should define region affordance block")
        .split("fn region_time_seconds")
        .next()
        .expect("affordance block should precede region time helper");

    assert!(
        !affordance_block.contains("view_region_resize_handles("),
        "resize hit testing should use the waveform hit model while the normal region edge is painted by the base canvas"
    );
    assert!(!waveform_source.contains("waveform_region_resize_handle_style"));
    assert!(!waveform_source.contains("REGION_HANDLE_LINE_WIDTH"));
    assert!(!waveform_source.contains("REGION_HANDLE_VERTICAL_INSET"));
}

#[test]
fn top_toolbar_overview_uses_recovered_wavesurfer_filled_line_canvas() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("view_wavesurfer_line_waveform"));
    assert!(source.contains("continuous_waveform_color"));
    assert!(!source.contains("Row::new()\n            .spacing(1)\n            .width(Fill)\n            .height(Length::Fixed(26.0));\n        for peak in peaks"));
}

#[test]
fn top_toolbar_synthetic_overview_blocks_use_cached_canvas_without_per_block_widgets() {
    let toolbar_source = include_str!("components/toolbar.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(toolbar_source.contains("view_overview_blocks_canvas("));
    assert!(canvas_source.contains("struct OverviewBlocksCanvas"));
    assert!(canvas_source.contains("impl canvas::Program<Message> for OverviewBlocksCanvas"));
    assert!(canvas_source.contains("frame.fill_rectangle("));
    assert!(canvas_source.contains("overview_block_widths("));
    assert!(toolbar_source.contains("Arc::clone(&overview.blocks)"));
    assert!(!toolbar_source.contains("fn view_overview_blocks(blocks: &[OverviewWaveformBlock])"));
    assert!(!toolbar_source.contains(
        "for block in blocks {\n        let gap = (block.left_percent - cursor).max(0.0);"
    ));
}

#[test]
fn top_toolbar_overview_window_and_playhead_use_separate_layers_without_fillportion_rows() {
    let toolbar_source = include_str!("components/toolbar.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");
    let playhead_source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/app/components/playhead_line.rs"
    ))
    .expect("playhead line widget should exist");
    let style_source = include_str!("styles.rs");

    assert!(toolbar_source.contains("view_overview_window_quad("));
    assert!(toolbar_source.contains("view_overview_playhead_canvas("));
    assert!(!toolbar_source.contains("view_overview_chrome_canvas("));
    assert!(canvas_source.contains("struct OverviewWindowQuad"));
    assert!(!canvas_source.contains("struct OverviewWindowCanvas"));
    assert!(!canvas_source.contains("struct OverviewPlayheadCanvas"));
    assert!(canvas_source.contains("impl Widget<Message, Theme, Renderer> for OverviewWindowQuad"));
    assert!(!canvas_source.contains("impl canvas::Program<Message> for OverviewWindowCanvas"));
    assert!(!canvas_source.contains("impl canvas::Program<Message> for OverviewPlayheadCanvas"));
    assert!(canvas_source.contains("overview_window_rect("));
    assert!(canvas_source.contains("view_playhead_line(playhead_percent, 30.0, 2.0"));
    assert!(playhead_source.contains("renderer.fill_quad("));
    assert!(playhead_source.contains("fn playhead_pixel_x("));
    assert!(canvas_source.contains("renderer.fill_quad("));
    assert!(canvas_source.contains("border::rounded(1)"));
    assert!(canvas_source.contains(".width(2)"));
    assert!(canvas_source.contains(".color(Color::from_rgb8(49, 160, 252))"));
    assert!(!canvas_source.contains("struct OverviewChromeCanvas"));
    assert!(!canvas_source.contains(
        "playhead_percent: f32,\n}\n\n#[derive(Clone, Debug)]\nstruct OverviewWindowCanvas"
    ));
    assert!(!toolbar_source.contains("fn view_overview_window("));
    assert!(!toolbar_source.contains("fn view_overview_playhead("));
    assert!(!toolbar_source.contains("overview_window_style"));
    assert!(!toolbar_source.contains("overview_playhead_style"));
    assert!(!style_source.contains("pub(crate) fn overview_window_style"));
    assert!(!style_source.contains("pub(crate) fn overview_playhead_style"));
}

#[test]
fn top_toolbar_overview_borrows_waveform_peaks_without_hot_path_clones() {
    let source = include_str!("components/toolbar.rs");

    assert!(source.contains("Arc::clone(&overview.lane_waveform_models)"));
    assert!(source.contains("view_overview_peaks("));
    assert!(source.contains("lazy,"));
    assert!(source.contains("struct OverviewWaveformPeaksDependency"));
    assert!(source.contains("impl std::hash::Hash for OverviewWaveformPeaksDependency"));
    assert!(source.contains("view_lazy_overview_waveform_peaks("));
    assert!(source.contains("Arc::as_ptr(&self.peaks)"));
    assert!(source.contains("let peaks = lane\n                .waveform_model"));
    assert!(source.contains("Arc::clone(&overview.blocks)"));
    assert!(source.contains("Arc::clone(&channel.peaks)"));
    assert!(!source.contains("overview.waveform_model.clone()"));
    assert!(!source.contains("overview.blocks.clone()"));
    assert!(!source.contains(".map(|channel| channel.peaks.clone())"));
}

#[test]
fn timeline_waveform_renders_recovered_wavesurfer_scrollbar_thumb() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let style_source = include_str!("styles.rs");
    let canvas_source = include_str!("components/wavesurfer_canvas.rs");

    assert!(waveform_source.contains("TimelineScrollbarView"));
    assert!(waveform_source.contains("view_waveform_scrollbar"));
    assert!(waveform_source.contains("SCROLLBAR_THUMB_HEIGHT"));
    assert!(waveform_source.contains("SCROLLBAR_BOTTOM_OFFSET"));
    assert!(waveform_source.contains("view_waveform_scrollbar_canvas("));
    assert!(canvas_source.contains("pub(super) fn view_waveform_scrollbar_canvas"));
    assert!(canvas_source.contains("struct WaveformScrollbarCanvas"));
    assert!(canvas_source.contains("impl canvas::Program<Message> for WaveformScrollbarCanvas"));
    assert!(canvas_source.contains("waveform_scrollbar_thumb_rect("));
    assert!(canvas_source.contains("Color::from_rgb8(116, 116, 116)"));
    assert!(waveform_source.contains("data.scrollbar"));
    assert!(waveform_source.contains("Message::TimelineScrollbarHovered"));
    assert!(waveform_source.contains("Message::TimelineScrollbarPressed"));
    assert!(waveform_source.contains("Message::TimelineScrollbarReleased"));
    assert!(waveform_source.contains(".interaction(mouse::Interaction::Pointer)"));
    assert!(!waveform_source.contains("fn view_scrollbar_thumb_row("));
    assert!(!waveform_source.contains("scrollbar_thumb_style"));
    assert!(!waveform_source.contains("Length::FillPortion(portion(thumb_width))"));
    assert!(!style_source.contains("pub(crate) fn scrollbar_thumb_style"));
}

#[test]
fn timeline_waveform_wires_browser_style_middle_mouse_autoscroll_handlers() {
    let message_source = include_str!("message.rs");
    let state_source = include_str!("state.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let lane_source = include_str!("components/timeline/lane.rs");
    let structure_source = include_str!("segment_actions/structure.rs");
    let hotkey_source = include_str!("hotkeys.rs");

    assert!(message_source.contains("TimelineAutoscrollPressed(String)"));
    assert!(message_source.contains("TimelineAutoscrollReleased"));
    assert!(state_source.contains("active_timeline_autoscroll"));
    assert!(state_source.contains("TimelineAutoscrollMode"));
    assert!(state_source.contains("advance_timeline_autoscroll"));
    assert!(state_source.contains("timeline_autoscroll_active"));
    assert!(state_source.contains("timeline_autoscroll_active_for_lane"));
    assert!(structure_source.contains("press_timeline_autoscroll"));
    assert!(structure_source.contains("release_timeline_autoscroll"));
    assert!(structure_source.contains("cancel_timeline_autoscroll"));
    assert!(structure_source.contains("update_timeline_autoscroll_target"));
    assert!(hotkey_source.contains("state.timeline_autoscroll_active()"));
    assert!(hotkey_source.contains("ButtonReleased(mouse::Button::Middle)"));
    assert!(
        waveform_source
            .matches(".on_middle_press(Message::TimelineAutoscrollPressed")
            .count()
            >= 2
    );
    assert!(
        waveform_source
            .matches(".on_middle_release(Message::TimelineAutoscrollReleased)")
            .count()
            >= 2
    );
    assert!(timeline_source.contains("timeline_autoscroll_active_for_lane"));
    assert!(lane_source.contains("timeline_autoscroll_active"));
    assert!(waveform_source.contains("autoscroll_interaction"));
    assert!(waveform_source.contains("mouse::Interaction::Grabbing"));
    assert!(waveform_source.contains("view_timeline_autoscroll_marker"));
    assert!(!message_source.contains("TimelineMiddleDragReleased"));
}

#[test]
fn timeline_waveform_uses_resize_cursor_on_region_edges() {
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");

    assert!(timeline_source.contains("target: hover.hit.target"));
    assert!(waveform_source.contains("target: WaveformHitTarget"));
    assert!(waveform_source.contains("fn waveform_interaction("));
    assert!(waveform_source.contains("WaveformHitTarget::RegionStartHandle"));
    assert!(waveform_source.contains("WaveformHitTarget::RegionEndHandle"));
    assert!(waveform_source.contains("mouse::Interaction::ResizingHorizontally"));
    assert!(
        waveform_source
            .matches("let interaction = waveform_interaction(")
            .count()
            >= 2
    );
    assert!(waveform_source.contains(".interaction(interaction)"));
}

#[test]
fn timeline_waveform_renders_recovered_edge_drag_magnifier() {
    let state_source = include_str!("state.rs");
    let waveform_source = include_str!("components/timeline/waveform.rs");
    let timeline_source = include_str!("components/timeline/mod.rs");
    let lane_source = include_str!("components/timeline/lane.rs");
    let style_source = include_str!("styles.rs");

    assert!(state_source.contains("WaveformMagnifierOverlay"));
    assert!(state_source.contains("waveform_magnifier_overlay_for_lane"));
    assert!(state_source.contains("WAVEFORM_MAGNIFIER_SCALE: f32 = 3.0"));
    assert!(state_source.contains("WAVEFORM_MAGNIFIER_WIDTH_PX: f32 = 180.0"));
    assert!(state_source.contains("WAVEFORM_MAGNIFIER_MAX_HEIGHT_PX: f32 = 150.0"));
    assert!(timeline_source.contains("waveform_magnifier_overlay_for_lane"));
    assert!(lane_source.contains("magnifier"));
    assert!(waveform_source.contains("WaveformMagnifierOverlay"));
    assert!(waveform_source.contains("view_waveform_magnifier"));
    assert!(waveform_source.contains("view_waveform_magnifier_center_line"));
    assert!(style_source.contains("waveform_magnifier_style"));
    assert!(style_source.contains("waveform_magnifier_center_line_style"));
}

#[test]
fn timeline_waveform_outlines_hovered_and_create_preview_regions() {
    let waveform_source = include_str!("components/timeline/waveform.rs");

    assert!(waveform_source.contains("const CREATE_PREVIEW_SEGMENT_ID"));
    assert!(waveform_source.contains("fn active_region_segment_id_for_blocks("));
    assert!(waveform_source.contains("visible_segment_id_for_blocks(blocks, hovered_segment_id)"));
    assert!(waveform_source.contains("create_preview_segment_id_for_blocks(blocks)"));
    assert!(waveform_source.contains("block.segment_id == CREATE_PREVIEW_SEGMENT_ID"));
    assert!(waveform_source.contains("active_region_segment_id_for_blocks("));
}

#[test]
fn hotkeys_dialog_documents_recovered_babel_overlay_controls() {
    let rows = super::components::hotkeys_dialog::recovered_hotkey_registry();
    let overlay = rows
        .iter()
        .find(|section| section.0 == "1-Second Overlay")
        .expect("Babel overlay section should exist");

    assert!(
        overlay
            .1
            .iter()
            .any(|row| row.1 == "Toggle 1-second overlay")
    );
    assert!(
        overlay
            .1
            .iter()
            .any(|row| row.1 == "Move overlay to check different positions")
    );
}

#[test]
fn hotkeys_dialog_matches_recovered_babel_section_order_and_key_labels() {
    let rows = super::components::hotkeys_dialog::recovered_hotkey_registry();
    let titles = rows.iter().map(|section| section.0).collect::<Vec<_>>();
    let overlay_index = titles
        .iter()
        .position(|title| *title == "1-Second Overlay")
        .expect("Babel labels the overlay section exactly");
    let loop_index = titles
        .iter()
        .position(|title| *title == "Loop Controls")
        .expect("Babel loop section should exist");

    assert!(overlay_index < loop_index);
    assert!(!titles.contains(&"Overlay Controls"));
    assert!(
        rows.iter()
            .flat_map(|section| section.1.iter())
            .any(|row| row.0 == vec!["\u{2190}"])
    );
    assert!(
        rows.iter()
            .flat_map(|section| section.1.iter())
            .any(|row| row.0 == vec!["\u{2192}"])
    );
}

#[test]
fn hotkeys_dialog_registry_matches_recovered_babel_hotkeys() {
    let registry = super::components::hotkeys_dialog::recovered_hotkey_registry();

    assert_eq!(
        registry,
        vec![
            (
                "Global Controls",
                vec![
                    (vec!["Space"], "Play/Pause"),
                    (vec!["\u{2190}"], "Seek To Previous Segment"),
                    (vec!["\u{2192}"], "Seek To Next Segment"),
                    (vec!["Cmd/Ctrl", "Z"], "Undo"),
                    (vec!["Cmd/Ctrl", "Shift", "Z"], "Redo"),
                    (vec!["Cmd/Ctrl", "F"], "Find in transcript"),
                    (vec!["F3", "Or", "Cmd/Ctrl", "G"], "Find next match",),
                    (
                        vec!["Shift", "F3", "Or", "Shift", "Cmd/Ctrl", "G"],
                        "Find previous match",
                    ),
                    (
                        vec!["Cmd/Ctrl", "J"],
                        "Confirm/undo warning on active segment",
                    ),
                    (vec!["Hold Shift", "Scroll"], "Side-scroll"),
                    (vec!["Pinch", "Or", "Cmd/Ctrl", "Scroll"], "Zoom in/out",),
                ],
            ),
            (
                "1-Second Overlay",
                vec![
                    (vec!["T"], "Toggle 1-second overlay"),
                    (vec!["Drag"], "Move overlay to check different positions"),
                ],
            ),
            (
                "Loop Controls",
                vec![
                    (vec!["Drag"], "Set loop region (on loop bar)"),
                    (vec!["L"], "Toggle loop on/off"),
                    (vec!["Escape"], "Clear loop region"),
                    (vec!["Double-click"], "Clear loop (on loop bar)"),
                ],
            ),
            (
                "Segment Controls (Click on segment)",
                vec![
                    (vec!["Click"], "Select segment"),
                    (vec!["Click", "Click"], "Play segment"),
                    (vec!["Hold Shift", "Click"], "Play segment and continue"),
                    (vec!["Cmd/Ctrl", "Click"], "Split segment at click position",),
                    (vec!["Hold Q", "Click"], "Play 500ms before segment start",),
                    (vec!["Hold W", "Click"], "Play first 500ms of segment"),
                    (vec!["Hold E", "Click"], "Play last 500ms of segment"),
                    (vec!["Hold R", "Click"], "Play 500ms after segment end"),
                ],
            ),
            (
                "Helper Extensions",
                vec![
                    (
                        vec!["Right Shift", "Left/Right"],
                        "Focus previous / next segment from start",
                    ),
                    (vec!["Alt", "1/2"], "Switch active speaker workflow lane",),
                    (vec!["Alt", "`"], "Reset speaker workflow lanes"),
                    (vec!["Alt", "X"], "Rewind playback 1000ms"),
                    (vec!["Shift", "1/2"], "Increase / decrease playback speed",),
                    (vec!["D"], "Delete current segment when not typing"),
                    (
                        vec!["Alt", "Shift", "Up/Down"],
                        "Merge with adjacent segment"
                    ),
                    (
                        vec!["Alt", "["],
                        "Move text before caret to previous segment",
                    ),
                    (vec!["Alt", "]"], "Move text after caret to next segment",),
                    (vec!["Escape"], "Pause and blur / resume and restore cursor",),
                    (
                        vec!["Alt", "A"],
                        "Auto-convert selected digits into SKAZ notation",
                    ),
                    (
                        vec!["Alt", "R"],
                        "Trim current segment to nearby visible audio",
                    ),
                    (
                        vec!["Alt", "Shift", "R"],
                        "Trim all visible segments to nearby visible audio",
                    ),
                    (
                        vec!["Alt", "Shift", "G"],
                        "Transcribe current empty segment with native drafting audio",
                    ),
                    (
                        vec!["Alt", "C"],
                        "Create empty segment around nearest uncovered speech near caret",
                    ),
                    (
                        vec!["Alt", "Shift", "S"],
                        "Split visible segments on silence runs over 1000ms, then trim all",
                    ),
                    (vec!["S"], "Commit active cut preview with smart text split",),
                    (
                        vec!["Shift", "S"],
                        "Commit active cut preview without smart text split",
                    ),
                    (vec!["L"], "Loop active cut preview range",),
                    (vec!["Alt", "F"], "Auto-fix lint issues in current row",),
                    (
                        vec!["Alt", "Shift", "F"],
                        "Auto-fix lint issues in all rows",
                    ),
                ],
            ),
        ]
    );
}

#[test]
fn hotkeys_dialog_reads_recovered_rows_from_capability_registry() {
    let registry_rows = studio_app::CapabilityRegistry::default().hotkey_sections();
    let dialog_rows = super::components::hotkeys_dialog::recovered_hotkey_registry();

    assert_eq!(dialog_rows, registry_rows);
}

#[test]
fn hotkeys_dialog_renders_ported_helper_extension_rows() {
    let rows = super::components::hotkeys_dialog::recovered_hotkey_registry();
    let helper = rows
        .iter()
        .find(|section| section.0 == "Helper Extensions")
        .expect("ported Helper hotkeys should be rendered");

    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "A"] && row.1.contains("SKAZ"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "["] && row.1.contains("previous segment"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "Shift", "F"] && row.1.contains("all rows"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["D"] && row.1.contains("Delete current segment"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Right Shift", "Left/Right"] && row.1.contains("from start"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Shift", "1/2"] && row.1.contains("playback speed"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "X"] && row.1.contains("1000ms"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "Shift", "R"] && row.1.contains("visible audio"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "Shift", "G"] && row.1.contains("drafting audio"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "C"] && row.1.contains("nearest uncovered speech"))
    );
    assert!(
        helper
            .1
            .iter()
            .any(|row| row.0 == vec!["Alt", "Shift", "S"] && row.1.contains("1000ms"))
    );
}

#[test]
fn native_capability_tools_open_as_real_modal_root() {
    let components_source = include_str!("components/mod.rs");

    assert!(
        components_source.contains("view_native_capability_modal_root"),
        "capabilities must render as a modal root layered over the workbench"
    );
    assert!(
        !components_source.contains(
            "layout = layout.push(view_outside_dismiss_area(\n            state,\n            native_capability_panel::view_native_capability_panel"
        ),
        "capabilities must not be pushed into the editor column before the waveform card"
    );
    assert!(
        !components_source.contains("view_native_capability_overlay"),
        "capabilities must not be implemented as an attached side overlay"
    );
}

#[test]
fn native_capability_modal_uses_web_level_dialog_layout() {
    let panel_source = include_str!("components/native_capability_panel.rs");

    assert!(
        panel_source.contains("NATIVE_CAPABILITY_MODAL_WIDTH")
            && panel_source.contains("NATIVE_CAPABILITY_MODAL_HEIGHT"),
        "capabilities need a stable modal surface with its own dimensions"
    );
    assert!(
        !panel_source.contains("NATIVE_CAPABILITY_DRAWER_WIDTH"),
        "capabilities must not be a drawer or attached panel"
    );
    assert!(
        panel_source.contains("scrollable("),
        "capability content must be bounded inside the modal and scroll independently"
    );
    assert!(
        panel_source.contains("view_reviewer_workspace_section")
            && panel_source.contains("view_gold_drafting_section"),
        "Reviewer and Gold Drafting need task-oriented modal sections, not one developer control matrix"
    );
    assert!(
        !panel_source.contains("let drafting = row!["),
        "Gold Drafting actions must not live in a single overflow-prone row"
    );
    assert!(
        !panel_source.contains("let reviewer_primary = row!["),
        "Reviewer actions must not live in a single overflow-prone row"
    );
}

#[test]
fn native_capability_modal_uses_tabs_for_distinct_extension_workspaces() {
    let state_source = include_str!("state.rs");
    let message_source = include_str!("message.rs");
    let actions_source = include_str!("actions.rs");
    let panel_source = include_str!("components/native_capability_panel.rs");

    assert!(
        state_source.contains("NativeCapabilityModalTab"),
        "modal needs first-class tab state instead of one long scrolling tool dump"
    );
    assert!(
        message_source.contains("NativeCapabilityModalTabChanged"),
        "modal tab changes must be real app messages"
    );
    assert!(
        actions_source.contains("NativeCapabilityModalTabChanged"),
        "modal tab messages must update state"
    );
    assert!(
        panel_source.contains("view_capability_modal_tabs"),
        "modal should render a visible tab strip for extension workspaces"
    );
    assert!(
        panel_source.contains("match active_tab"),
        "modal body should render one active workspace at a time"
    );
    assert!(
        !panel_source.contains(
            "view_reviewer_workspace_section(state),\n        view_gold_drafting_section(state),"
        ),
        "Reviewer and Gold Drafting must not be stacked in one long modal body"
    );
}

#[test]
fn dashboard_hides_raw_id_marks_and_moves_drafting_settings_home() {
    let dashboard_source = include_str!("components/dashboard.rs");
    let panel_source = include_str!("components/native_capability_panel.rs");

    assert!(
        !dashboard_source.contains("text(project.id.clone()).size(11)"),
        "dashboard project cards should not render raw project IDs under project names"
    );
    assert!(
        !dashboard_source.contains("view_task_entry_tools"),
        "dashboard should not keep the always-visible manual ID block at the bottom"
    );
    assert!(
        dashboard_source.contains("view_capability_settings"),
        "dashboard needs a first-class settings area for capability configuration"
    );
    for label in ["OpenRouter key", "Model", "Tier", "Reasoning"] {
        assert!(
            dashboard_source.contains(label),
            "capability setting `{label}` should live on the dashboard"
        );
        assert!(
            !panel_source.contains(label),
            "capability setting `{label}` should not stay inside the task modal"
        );
    }
}

#[test]
fn updater_defaults_to_alpha_feed_and_current_version() {
    let state = StudioState::default();

    assert_eq!(state.updater.feed_url, DEFAULT_UPDATE_FEED_URL);
    assert_eq!(state.updater.current_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        state.updater.build_commit,
        option_env!("STUDIO_GIT_COMMIT").unwrap_or("unknown")
    );
    assert!(matches!(state.updater.phase, UpdatePhase::Idle));
    assert_eq!(
        state.updater.status_label(),
        "Ready to check for alpha updates."
    );
    assert!(state.updater.can_check());
    assert!(!state.updater.can_install());
}

#[test]
fn update_check_messages_drive_dashboard_update_status() {
    let mut state = StudioState::default();

    let _ = update(&mut state, Message::CheckForUpdates);
    assert!(matches!(state.updater.phase, UpdatePhase::Checking));
    assert!(!state.updater.can_check());
    assert!(!state.updater.can_install());

    let _ = update(
        &mut state,
        Message::UpdateCheckFinished(Ok(UpdateCheckSummary::available("0.1.1"))),
    );
    assert_eq!(state.updater.status_label(), "Update 0.1.1 is available.");
    assert_eq!(state.updater.available_version.as_deref(), Some("0.1.1"));
    assert!(state.updater.can_check());
    assert!(state.updater.can_install());

    let _ = update(&mut state, Message::InstallAvailableUpdate);
    assert!(matches!(state.updater.phase, UpdatePhase::Installing));
    assert!(!state.updater.can_check());
    assert!(!state.updater.can_install());

    let _ = update(
        &mut state,
        Message::UpdateInstallFinished(Err("network down".to_string())),
    );
    assert_eq!(state.updater.status_label(), "Update failed: network down");
    assert!(state.updater.can_check());
    assert!(state.updater.can_install());

    let _ = update(
        &mut state,
        Message::UpdateCheckFinished(Ok(UpdateCheckSummary::up_to_date())),
    );
    assert_eq!(
        state.updater.status_label(),
        "You are on the latest alpha build."
    );
    assert!(state.updater.available_version.is_none());
    assert!(state.updater.can_check());
    assert!(!state.updater.can_install());
}

#[test]
fn dashboard_exposes_small_update_section() {
    let dashboard_source = include_str!("components/dashboard.rs");

    assert!(dashboard_source.contains("view_update_section"));
    for label in [
        "Updates",
        "Current version",
        "Build commit",
        "Update feed",
        "Check for updates",
        "Install update",
    ] {
        assert!(
            dashboard_source.contains(label),
            "dashboard update section should render `{label}`"
        );
    }
    assert!(dashboard_source.contains("Message::CheckForUpdates"));
    assert!(dashboard_source.contains("Message::InstallAvailableUpdate"));
}

#[test]
fn main_runs_velopack_hooks_before_auth_cli_routes() {
    let main_source = include_str!("../main.rs");
    let velopack_index = main_source
        .find("VelopackApp::build().run()")
        .expect("main should initialize Velopack hooks");
    let auth_cli_index = main_source
        .find("run_native_auth_cli_from_args")
        .expect("main should still support native auth CLI routes");

    assert!(
        velopack_index < auth_cli_index,
        "Velopack hooks must run before app-specific CLI parsing"
    );
}

#[test]
fn windows_alpha_release_scripts_use_velopack_and_ethernetservers() {
    let package_script = include_str!("../../../../tools/package-alpha-windows.ps1");
    let publish_script = include_str!("../../../../tools/publish-alpha-windows.ps1");
    let release_doc = include_str!("../../../../docs/release/windows-alpha.md");
    let workflow = include_str!("../../../../.github/workflows/windows-alpha.yml");

    assert!(package_script.contains("vpk pack"));
    assert!(package_script.contains("--channel"));
    assert!(package_script.contains("TranscriptionStudio.exe"));
    assert!(workflow.contains("$packageArgs = @{"));
    assert!(workflow.contains("Channel = $env:CHANNEL"));
    assert!(workflow.contains(".\\tools\\package-alpha-windows.ps1 @packageArgs"));
    assert!(!workflow.contains("$args = @("));
    assert!(!workflow.contains("$packageArgs = @("));
    assert!(publish_script.contains("$packageArgs = @{"));
    assert!(publish_script.contains("OutputDir = $releaseDir"));
    assert!(publish_script.contains("ssh ethernetservers"));
    assert!(publish_script.contains("scp"));
    assert!(publish_script.contains("/var/www/transcription-studio/alpha/win-x64"));
    assert!(release_doc.contains("STUDIO_UPDATE_FEED_URL"));
    assert!(release_doc.contains("releases.alpha.json"));
    assert!(release_doc.contains("TranscriptionStudio-alpha-Setup.exe"));
}

#[test]
fn reviewer_capability_is_gated_to_l2_review_tasks() {
    let mut state = StudioState::default();

    assert!(!state.reviewer_capability_available());
    let _ = update(&mut state, Message::ReviewerGenerateReview);
    assert!(state.review_backend_requests.is_empty());
    assert!(state.status.contains("L2"));

    state.workflow.as_mut().unwrap().merge_project_metadata([(
        "babel.extra.package".to_string(),
        r#"{"action_level":2}"#.to_string(),
    )]);

    assert!(state.reviewer_capability_available());
    let _ = update(&mut state, Message::ReviewerGenerateReview);
    assert!(
        state
            .review_backend_requests
            .iter()
            .any(|request| matches!(request, ReviewBackendRequest::ReviewGenerate(_)))
    );
}

#[test]
fn native_capability_modal_defaults_to_gold_drafting_when_reviewer_is_not_l2() {
    let mut state = StudioState::default();

    let _ = update(&mut state, Message::ToggleNativeCapabilityPanel);

    assert!(state.native_capability_panel_open);
    assert_eq!(
        state.native_capability_modal_tab,
        NativeCapabilityModalTab::GoldDrafting
    );
}

#[test]
fn capability_registry_documents_helper_text_move_hotkeys_without_rendering_disabled_ai() {
    let registry = studio_app::CapabilityRegistry::default();
    let helper = registry
        .descriptor(studio_app::CapabilityId::new("helper.text-move"))
        .expect("helper text move descriptor");
    let drafting = registry
        .descriptor(studio_app::CapabilityId::new("drafting.transcribe-segment"))
        .expect("drafting segment descriptor");

    assert_eq!(
        helper
            .hotkeys
            .iter()
            .map(|hotkey| (hotkey.keys, hotkey.description))
            .collect::<Vec<_>>(),
        vec![
            (
                &["Alt", "["][..],
                "Move text before caret to previous segment"
            ),
            (&["Alt", "]"][..], "Move text after caret to next segment"),
        ]
    );
    assert!(!registry.is_enabled(drafting.id));
    assert!(
        !registry
            .hotkey_sections()
            .iter()
            .any(|section| section.0.contains("Drafting"))
    );
}

#[test]
fn dashboard_exposes_babel_json_paths_without_native_studio_project_controls() {
    let source = include_str!("components/dashboard.rs");

    assert!(source.contains("Babel JSON"));
    assert!(source.contains("Import"));
    assert!(source.contains("Export"));
    assert!(!source.contains("DEFAULT_LOCAL_PROJECT_PATH"));
    assert!(!source.contains("LocalProjectPathChanged"));
    assert!(!source.contains("LoadLocalProject"));
    assert!(!source.contains("SaveLocalProject"));
    assert!(!source.contains("button(text(\"Load\"))"));
    assert!(!source.contains("button(text(\"Save\"))"));
}

#[test]
fn top_toolbar_exposes_back_to_dashboard_button() {
    let toolbar_source = include_str!("components/toolbar.rs");

    assert!(toolbar_source.contains("BACK_TO_DASHBOARD_ICON"));
    assert!(toolbar_source.contains("Message::BackToProjects"));
    assert!(toolbar_source.contains("\"Back\""));
}

#[test]
fn seek_to_time_message_moves_playhead_to_word_timestamp() {
    let mut state = stable_sample_state();

    let _ = update(&mut state, Message::SeekToTime(3.75));

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, 3.75);
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-2"));
}

#[test]
fn seek_to_time_message_moves_playhead_without_recentering_timeline_viewport() {
    let mut state = stable_sample_state();
    {
        let editor = state.editor.as_mut().unwrap();
        editor.viewport = TimeRange {
            start_seconds: 10.0,
            end_seconds: 15.0,
        };
    }
    let before = state.editor.as_ref().unwrap().viewport;

    let _ = update(&mut state, Message::SeekToTime(3.75));

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.playhead_seconds, 3.75);
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-2"));
    assert_eq!(editor.viewport, before);
}

#[test]
fn helper_focus_escape_workflow_matches_recovered_four_state_toggle() {
    let mut state = stable_sample_state();

    {
        let editor = state.editor.as_mut().unwrap();
        editor.selected_segment_id = Some("annotation-1".to_string());
        editor.playhead_seconds = 0.30;
        editor.is_playing = true;
    }
    let _ = update(
        &mut state,
        Message::TranscriptEditorEscapePressed("annotation-1".to_string()),
    );
    let editor = state.editor.as_ref().unwrap();
    assert!(!editor.is_playing, "focused + playing Esc pauses only");
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-1"));

    let _ = update(
        &mut state,
        Message::TranscriptEditorEscapePressed("annotation-1".to_string()),
    );
    let editor = state.editor.as_ref().unwrap();
    assert!(editor.is_playing, "focused + paused Esc blurs and resumes");
    assert_eq!(
        state
            .helper_focus_restore_pending
            .as_ref()
            .map(|restore| restore.segment_id.as_str()),
        Some("annotation-1")
    );

    {
        let editor = state.editor.as_mut().unwrap();
        editor.playhead_seconds = 3.75;
    }
    set_draft_text_with_cursor_at_end(&mut state, "annotation-2", "a bcdefghi");
    let _ = update(&mut state, Message::EscapePressed);
    let editor = state.editor.as_ref().unwrap();
    assert!(
        !editor.is_playing,
        "unfocused + playing Esc restores and pauses"
    );
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-2"));
    assert_eq!(
        state
            .helper_last_focus_restore
            .as_ref()
            .map(|restore| restore.segment_id.as_str()),
        Some("annotation-2")
    );
    assert_eq!(
        state
            .helper_last_focus_restore
            .as_ref()
            .map(|restore| restore.restore_offset),
        Some(2)
    );
    assert_eq!(
        state
            .segment_drafts
            .get("annotation-2")
            .unwrap()
            .text_content
            .cursor_position(),
        (0, 2)
    );

    let _ = update(&mut state, Message::EscapePressed);
    let editor = state.editor.as_ref().unwrap();
    assert!(
        editor.is_playing,
        "unfocused + paused Esc bootstraps and resumes"
    );
    assert_eq!(editor.selected_segment_id.as_deref(), Some("annotation-2"));
    assert_eq!(
        state
            .helper_focus_restore_pending
            .as_ref()
            .map(|restore| restore.synthetic),
        Some(true)
    );
}

#[test]
fn helper_focus_restore_targets_native_transcript_editor_widget() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let focusable_text_editor_source = include_str!("components/focusable_text_editor.rs");
    let actions_source = include_str!("actions.rs");

    assert!(row_source.contains("focusable_text_editor::transcript_text_editor_id"));
    assert!(row_source.contains("focusable_text_editor::view_focusable_text_editor"));
    assert!(focusable_text_editor_source.contains("operation.focusable(state, Some(self.id))"));
    assert!(actions_source.contains("focus_restored_transcript_editor"));
    assert!(actions_source.contains("helper_last_focus_restore"));
    assert!(actions_source.contains("focusable_text_editor::focus_transcript_text_editor"));
}

#[test]
fn helper_focus_pending_restore_renders_ghost_cursor_overlay() {
    let mut state = stable_sample_state();
    state.helper_focus_restore_pending = Some(HelperFocusRestore {
        segment_id: "annotation-2".to_string(),
        blur_playhead_seconds: 0.30,
        restore_playhead_seconds: 3.75,
        restore_offset: 2,
        synthetic: false,
    });
    set_draft_text_with_cursor_at_end(&mut state, "annotation-2", "abcdefghij");

    let ghost = state
        .helper_ghost_cursor_for_segment("annotation-2")
        .expect("pending helper restore should expose a ghost cursor");

    assert_eq!(ghost.segment_id, "annotation-2");
    assert_eq!(ghost.char_offset, 2);
    assert_eq!(ghost.before_cursor, "ab");
    assert_eq!(ghost.after_cursor, "cdefghij");
    assert!(
        state
            .helper_ghost_cursor_for_segment("annotation-1")
            .is_none(),
        "ghost cursor should only render on the pending restore row"
    );

    let row_source = include_str!("components/transcript_table/row.rs");
    assert!(row_source.contains("view_helper_ghost_cursor"));
    assert!(row_source.contains("state.helper_ghost_cursor_for_segment"));
    assert!(row_source.contains("helper_focus_restore_pending"));
    assert!(row_source.contains("stack!["));
    assert!(row_source.contains("ghost_cursor"));
    assert!(row_source.contains("transcript_text_wheel_overlay()"));
}

#[test]
fn helper_focus_ghost_cursor_uses_thin_extension_style_caret() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let ghost_source = row_source
        .split("fn view_helper_ghost_cursor")
        .nth(1)
        .and_then(|source| source.split("fn id_table_cell").next())
        .expect("ghost cursor renderer source should be present");

    assert!(ghost_source.contains("helper_ghost_cursor_color"));
    assert!(ghost_source.contains("span(\"\\u{200a}\")"));
    assert!(ghost_source.contains(".background(Background::Color(helper_ghost_cursor_color))"));
    assert!(!ghost_source.contains(".border("));
    assert!(!ghost_source.contains(".padding([0, 1])"));
}

#[test]
fn helper_focus_ghost_cursor_tracks_playhead_while_playing() {
    let mut state = stable_sample_state();
    set_draft_text_with_cursor_at_end(&mut state, "annotation-2", "abcdefghij");
    let segment = state
        .project()
        .unwrap()
        .segment("annotation-2")
        .unwrap()
        .clone();
    state.helper_focus_restore_pending = Some(HelperFocusRestore {
        segment_id: "annotation-2".to_string(),
        blur_playhead_seconds: segment.start_seconds,
        restore_playhead_seconds: segment.start_seconds,
        restore_offset: 0,
        synthetic: false,
    });
    {
        let editor = state.editor.as_mut().unwrap();
        editor.is_playing = true;
        editor.playhead_seconds = segment.start_seconds;
    }
    let first = state
        .helper_ghost_cursor_for_segment("annotation-2")
        .expect("playing restore should expose ghost cursor");

    state.editor.as_mut().unwrap().playhead_seconds =
        segment.start_seconds + (segment.end_seconds - segment.start_seconds) * 0.6;
    let second = state
        .helper_ghost_cursor_for_segment("annotation-2")
        .expect("playing restore should keep exposing ghost cursor");

    assert!(
        second.char_offset > first.char_offset,
        "playing ghost cursor should move with the live playhead"
    );
    assert!(second.before_cursor.chars().count() > first.before_cursor.chars().count());
}

#[test]
fn selected_segment_navigation_recenters_current_text_row_even_when_visible() {
    let mut state = stable_sample_state();
    let project = large_two_lane_project(80);
    let workflow = StudioWorkflow::from_project(project.clone());
    state.replace_workflow(
        workflow,
        "Loaded large project row-following regression fixture.".to_string(),
    );
    state.transcript_scroll_viewport_height = 360.0;
    {
        let editor = state.editor.as_mut().unwrap();
        editor.select_segment_without_seek(&project, "annotation-0018");
        editor.playhead_seconds = project
            .segment("annotation-0018")
            .expect("large project should include the selected row")
            .start_seconds;
    }
    let initial_offset = {
        let editor = state.editor.as_ref().unwrap();
        state
            .transcript_scroll_offset_for_segment(&project, editor, "annotation-0018")
            .expect("initial selected row should have a centered scroll target")
    };
    let expected_next_offset = {
        let editor = state.editor.as_ref().unwrap();
        state
            .transcript_scroll_offset_for_segment(&project, editor, "annotation-0019")
            .expect("next selected row should have a centered scroll target")
    };
    assert_ne!(initial_offset, expected_next_offset);
    state.transcript_scroll_offset_y = initial_offset;

    let _ = update(&mut state, Message::NextSegment);

    let editor = state.editor.as_ref().unwrap();
    assert_eq!(
        editor.selected_segment_id.as_deref(),
        Some("annotation-0019")
    );
    assert_eq!(state.transcript_scroll_offset_y, expected_next_offset);
}

#[test]
fn helper_ghost_cursor_pending_target_follows_segment_navigation() {
    let mut state = stable_sample_state();
    set_draft_text_with_cursor_at_end(&mut state, "annotation-2", "abcdefghij");
    let project = state.project().unwrap().clone();
    {
        let editor = state.editor.as_mut().unwrap();
        editor.select_segment(&project, "annotation-1");
    }
    let blur_playhead_seconds = project.segment("annotation-1").unwrap().start_seconds;
    state.helper_focus_restore_pending = Some(HelperFocusRestore {
        segment_id: "annotation-1".to_string(),
        blur_playhead_seconds,
        restore_playhead_seconds: blur_playhead_seconds,
        restore_offset: 0,
        synthetic: true,
    });

    let _ = update(&mut state, Message::NextSegment);

    assert_eq!(
        state
            .helper_focus_restore_pending
            .as_ref()
            .map(|restore| restore.segment_id.as_str()),
        Some("annotation-2")
    );
    assert!(
        state
            .helper_ghost_cursor_for_segment("annotation-2")
            .is_some(),
        "navigation should initialize the ghost cursor for the newly selected segment"
    );
}

#[test]
fn helper_focus_ghost_cursor_snaps_playback_offset_to_word_boundary() {
    let mut state = stable_sample_state();
    set_draft_text_with_cursor_at_end(&mut state, "annotation-2", "alpha beta gamma");
    let segment = state
        .project()
        .unwrap()
        .segment("annotation-2")
        .unwrap()
        .clone();
    state.helper_focus_restore_pending = Some(HelperFocusRestore {
        segment_id: "annotation-2".to_string(),
        blur_playhead_seconds: segment.start_seconds,
        restore_playhead_seconds: segment.start_seconds,
        restore_offset: 0,
        synthetic: false,
    });
    {
        let editor = state.editor.as_mut().unwrap();
        editor.is_playing = true;
        editor.playhead_seconds =
            segment.start_seconds + (segment.end_seconds - segment.start_seconds) * 0.5;
    }

    let ghost = state
        .helper_ghost_cursor_for_segment("annotation-2")
        .expect("playing restore should expose ghost cursor");

    assert_eq!(
        ghost.char_offset, 6,
        "raw proportional offset inside beta should snap to the nearest word boundary"
    );
}

#[test]
fn helper_focus_ghost_cursor_uses_text_layout_mirror_for_soft_wraps() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let state_source = include_str!("state.rs");

    assert!(row_source.contains("rich_text(["));
    assert!(row_source.contains("span(ghost.before_cursor"));
    assert!(row_source.contains("span(ghost.after_cursor"));
    assert!(
        !row_source.contains("Space::with_width(Length::Fixed(ghost.left_px))"),
        "ghost cursor positioning must not use fixed character-width offsets"
    );
    assert!(
        !state_source.contains("HELPER_GHOST_CURSOR_CHAR_WIDTH_PX"),
        "ghost cursor placement must not depend on a fake monospace character width"
    );
}

#[test]
fn helper_focus_ghost_cursor_marker_uses_visible_thin_caret_styling() {
    let row_source = include_str!("components/transcript_table/row.rs");
    let ghost_source = row_source
        .split("fn view_helper_ghost_cursor")
        .nth(1)
        .and_then(|source| source.split("fn id_table_cell").next())
        .expect("ghost cursor renderer source should be present");

    assert!(ghost_source.contains("helper_ghost_cursor_color"));
    assert!(ghost_source.contains("span(\"\\u{200a}\")"));
    assert!(ghost_source.contains(".background(Background::Color(helper_ghost_cursor_color))"));
    assert!(!ghost_source.contains("border::rounded(2)"));
    assert!(!ghost_source.contains(".border("));
    assert!(!ghost_source.contains(".padding([0, 1])"));
}

#[test]
fn helper_playback_workflow_hotkeys_match_recovered_helper_routes() {
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("!".into()),
            Physical::Code(Code::Digit1),
            Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::AdjustPlaybackSpeed(1))
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("@".into()),
            Physical::Code(Code::Digit2),
            Modifiers::SHIFT,
            Status::Ignored,
        ),
        Some(Message::AdjustPlaybackSpeed(-1))
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("!".into()),
            Physical::Code(Code::Digit1),
            Modifiers::SHIFT,
            Status::Captured,
        ),
        None,
        "Shift+1 should not steal shifted punctuation while typing"
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("x".into()),
            Physical::Code(Code::KeyX),
            Modifiers::ALT,
            Status::Ignored,
        ),
        Some(Message::RewindPlaybackOneSecond)
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("1".into()),
            Physical::Code(Code::Digit1),
            Modifiers::ALT,
            Status::Ignored,
        ),
        Some(Message::SwitchSpeakerWorkflowLane("Speaker 1".to_string()))
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("2".into()),
            Physical::Code(Code::Digit2),
            Modifiers::ALT,
            Status::Ignored,
        ),
        Some(Message::SwitchSpeakerWorkflowLane("Speaker 2".to_string()))
    );
    assert_eq!(
        hotkey_message_for_physical_status(
            Key::Character("`".into()),
            Physical::Code(Code::Backquote),
            Modifiers::ALT,
            Status::Ignored,
        ),
        Some(Message::ResetSpeakerWorkflow)
    );
}

#[test]
fn helper_playback_speed_rewind_and_speaker_workflow_hotkeys_update_native_editor() {
    let mut state = stable_sample_state();
    {
        let editor = state.editor.as_mut().unwrap();
        editor.speed = 1.0;
        editor.playhead_seconds = 3.75;
    }

    let _ = update(&mut state, Message::AdjustPlaybackSpeed(1));
    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.speed, 1.5);
    assert_eq!(editor.playhead_seconds, 3.75);

    let _ = update(&mut state, Message::AdjustPlaybackSpeed(-1));
    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.speed, 1.0);
    assert_eq!(editor.playhead_seconds, 3.75);

    let _ = update(&mut state, Message::RewindPlaybackOneSecond);
    assert_eq!(state.editor.as_ref().unwrap().playhead_seconds, 2.75);

    let _ = update(
        &mut state,
        Message::SwitchSpeakerWorkflowLane("Speaker 1".to_string()),
    );
    let project = state.project().unwrap();
    let speaker_one_id = project
        .lanes
        .iter()
        .find(|lane| lane.label == "Speaker 1")
        .unwrap()
        .id
        .clone();
    let speaker_two_id = project
        .lanes
        .iter()
        .find(|lane| lane.label == "Speaker 2")
        .unwrap()
        .id
        .clone();
    let editor = state.editor.as_ref().unwrap();
    assert!(matches!(
        &editor.track_filter,
        studio_app::TrackFilter::Lane { label, .. } if label == "Speaker 1"
    ));
    assert!(!editor.muted_lane_ids.contains(&speaker_one_id));
    assert!(editor.muted_lane_ids.contains(&speaker_two_id));
    assert!(!editor.collapsed_lane_ids.contains(&speaker_one_id));
    assert!(editor.collapsed_lane_ids.contains(&speaker_two_id));

    let _ = update(&mut state, Message::ResetSpeakerWorkflow);
    let editor = state.editor.as_ref().unwrap();
    assert_eq!(editor.track_filter, studio_app::TrackFilter::All);
    assert!(!editor.muted_lane_ids.contains(&speaker_one_id));
    assert!(!editor.muted_lane_ids.contains(&speaker_two_id));
    assert!(!editor.collapsed_lane_ids.contains(&speaker_one_id));
    assert!(!editor.collapsed_lane_ids.contains(&speaker_two_id));
}

#[test]
fn hotkey_mapping_preserves_text_input_editing() {
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::Space),
            Modifiers::empty(),
            Status::Captured
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::ArrowRight),
            Modifiers::empty(),
            Status::Captured
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("z".into()),
            Modifiers::COMMAND,
            Status::Captured
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("q".into()),
            Modifiers::empty(),
            Status::Captured
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("z".into()),
            Modifiers::COMMAND,
            Status::Captured
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::Undo),
            Modifiers::empty(),
            Status::Captured
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::Redo),
            Modifiers::empty(),
            Status::Captured
        ),
        None
    );
}

#[test]
fn hotkey_mapping_routes_plain_tab_to_browser_focus_traversal() {
    assert_eq!(
        hotkey_message_for_status(Key::Named(Named::Tab), Modifiers::empty(), Status::Ignored),
        Some(Message::FocusNext)
    );
    assert_eq!(
        hotkey_message_for_status(Key::Named(Named::Tab), Modifiers::SHIFT, Status::Ignored),
        Some(Message::FocusPrevious)
    );
    assert_eq!(
        hotkey_message_for_status(Key::Named(Named::Tab), Modifiers::COMMAND, Status::Ignored),
        None
    );
    assert_eq!(
        hotkey_message_for_status(Key::Named(Named::Tab), Modifiers::empty(), Status::Captured),
        None
    );
}

#[test]
fn hotkey_mapping_routes_browser_back_keys_outside_text_fields() {
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::BrowserBack),
            Modifiers::empty(),
            Status::Ignored
        ),
        Some(Message::BackToProjects)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::GoBack),
            Modifiers::empty(),
            Status::Ignored
        ),
        Some(Message::BackToProjects)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::ArrowLeft),
            Modifiers::ALT,
            Status::Ignored
        ),
        Some(Message::BackToProjects)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Named(Named::BrowserBack),
            Modifiers::empty(),
            Status::Captured
        ),
        None
    );
}

#[test]
fn hotkey_mapping_opens_browser_find_even_from_text_input() {
    for status in [Status::Ignored, Status::Captured] {
        assert_eq!(
            hotkey_message_for_status(Key::Character("f".into()), Modifiers::COMMAND, status),
            Some(Message::OpenFind)
        );
        assert_eq!(
            hotkey_message_for_status(Key::Character("\u{6}".into()), Modifiers::COMMAND, status),
            Some(Message::OpenFind)
        );
        assert_eq!(
            hotkey_message_for_status(Key::Named(Named::Find), Modifiers::empty(), status),
            Some(Message::OpenFind)
        );
    }
}

#[test]
fn hotkey_mapping_routes_browser_find_navigation_shortcuts() {
    for status in [Status::Ignored, Status::Captured] {
        assert_eq!(
            hotkey_message_for_status(Key::Named(Named::F3), Modifiers::empty(), status),
            Some(Message::FindNext)
        );
        assert_eq!(
            hotkey_message_for_status(Key::Named(Named::F3), Modifiers::SHIFT, status),
            Some(Message::FindPrevious)
        );
        assert_eq!(
            hotkey_message_for_status(Key::Character("g".into()), Modifiers::COMMAND, status),
            Some(Message::FindNext)
        );
        assert_eq!(
            hotkey_message_for_status(Key::Character("\u{7}".into()), Modifiers::COMMAND, status),
            Some(Message::FindNext)
        );
        assert_eq!(
            hotkey_message_for_status(
                Key::Character("g".into()),
                Modifiers::COMMAND | Modifiers::SHIFT,
                status
            ),
            Some(Message::FindPrevious)
        );
        assert_eq!(
            hotkey_message_for_status(
                Key::Character("\u{7}".into()),
                Modifiers::COMMAND | Modifiers::SHIFT,
                status
            ),
            Some(Message::FindPrevious)
        );
    }
}

#[test]
fn hotkey_mapping_allows_warning_assertion_inside_input_like_recovered_babel() {
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("j".into()),
            Modifiers::COMMAND,
            Status::Captured
        ),
        Some(Message::ToggleActiveWarningAssertion)
    );
    assert_eq!(
        hotkey_message_for_status(
            Key::Character("\n".into()),
            Modifiers::COMMAND,
            Status::Captured
        ),
        Some(Message::ToggleActiveWarningAssertion)
    );
}

#[test]
fn hotkey_mapping_ignores_repeated_keydown_like_recovered_babel() {
    assert_eq!(
        hotkey_message_for_repeat(
            Key::Named(Named::Space),
            Modifiers::empty(),
            Status::Ignored,
            true
        ),
        None
    );
    assert_eq!(
        hotkey_message_for_repeat(
            Key::Named(Named::Space),
            Modifiers::empty(),
            Status::Ignored,
            false
        ),
        Some(Message::TogglePlayback)
    );
    assert_eq!(
        hotkey_message_for_repeat(
            Key::Character("j".into()),
            Modifiers::COMMAND,
            Status::Ignored,
            true
        ),
        None
    );
}

fn drag_region_body(state: &mut StudioState, grab_time_seconds: f64, move_time_seconds: f64) {
    let _ = update(
        state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: grab_time_seconds,
            },
        ),
    );
    let _ = update(state, Message::WaveformHitPressed("track-1".to_string()));
    let _ = update(
        state,
        Message::WaveformHitHovered(
            "track-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::RegionBody,
                region_id: Some("annotation-1".to_string()),
                time_seconds: move_time_seconds,
            },
        ),
    );
    let _ = update(state, Message::WaveformHitReleased("track-1".to_string()));
}

fn open_create_region_dialog_from_waveform(
    state: &mut StudioState,
    start_seconds: f64,
    end_seconds: f64,
) {
    if let Some(editor) = state.editor.as_mut() {
        let viewport_start = start_seconds.min(end_seconds) - 1.0;
        let viewport_end = (start_seconds.max(end_seconds) + 1.0).max(viewport_start + 0.001);
        editor.viewport = TimeRange {
            start_seconds: viewport_start,
            end_seconds: viewport_end,
        };
    }

    let _ = update(
        state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: start_seconds,
            },
        ),
    );
    let _ = update(
        state,
        Message::WaveformHitPressed("track-speaker-1".to_string()),
    );
    let _ = update(
        state,
        Message::WaveformHitHovered(
            "track-speaker-1".to_string(),
            WaveformHit {
                target: WaveformHitTarget::Waveform,
                region_id: None,
                time_seconds: end_seconds,
            },
        ),
    );
    let _ = update(
        state,
        Message::WaveformHitReleased("track-speaker-1".to_string()),
    );
}

fn state_project_with_annotation_mode() -> Project {
    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("studio.babel.mode".to_string(), "annotation".to_string());
    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
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
            text: String::new(),
            metadata: Default::default(),
        }],
        metadata,
    }
}

fn imported_audio_state_for_audition(segment_start: f64, segment_end: f64) -> StudioState {
    let temp = tempfile::tempdir().expect("temp dir should be created");
    let audio_dir = temp.path().join("audio");
    std::fs::create_dir_all(&audio_dir).expect("audio dir should be created");
    let samples = vec![16_384; 64];
    std::fs::write(
        audio_dir.join("speaker_1.wav"),
        pcm16_wav_bytes(4, 1, &samples),
    )
    .expect("wav should be written");
    let import_path = temp.path().join("task.json");
    std::fs::write(
        &import_path,
        serde_json::json!({
            "task_id": "task-with-audition",
            "project_preset": "ru-demo",
            "speakers": [
                { "speaker_key": "speaker-1", "label": "Speaker 1" }
            ],
            "audio_tracks": [
                { "track_id": "track-1", "speaker_key": "speaker-1", "path": "audio/speaker_1.wav" }
            ],
            "annotations": [
                {
                    "annotation_id": "annotation-1",
                    "speaker_key": "speaker-1",
                    "track_id": "track-1",
                    "start_seconds": segment_start,
                    "end_seconds": segment_end,
                    "text": "hello"
                }
            ]
        })
        .to_string(),
    )
    .expect("task should be written");
    let mut state = StudioState {
        import_path: import_path.display().to_string(),
        ..stable_sample_state()
    };
    let _ = update(&mut state, Message::ImportBabelJson);
    state
}

fn acknowledge_all_assertable_warnings(state: &mut StudioState) {
    let warning_segment_ids: Vec<_> = {
        let project = state.workflow.as_ref().unwrap().project();
        let editor = state.editor.as_ref().unwrap();
        editor
            .transcript_rows(project)
            .into_iter()
            .filter(|row| row.lint.is_assertable_warning())
            .map(|row| row.segment_id)
            .collect()
    };
    assert!(!warning_segment_ids.is_empty());
    for segment_id in warning_segment_ids {
        let project = state.workflow.as_ref().unwrap().project().clone();
        let editor = state.editor.as_mut().unwrap();
        editor.select_segment(&project, &segment_id);
        assert!(editor.toggle_warning_assertion(&project));
    }
}

fn pcm16_wav_bytes(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_size = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data_size);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_size as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_size as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[test]
fn workbench_dialogs_render_as_centered_modal_layers_over_scrim() {
    let shell_source = include_str!("components/mod.rs");

    assert!(
        shell_source.contains("fn view_modal_layer"),
        "workbench dialogs need a shared centered modal layer helper"
    );
    for dialog in [
        "submit_dialog::view_submit_dialog",
        "skip_chunk_dialog::view_skip_chunk_dialog",
        "admin_flag_dialog::view_admin_flag_dialog",
        "hotkeys_dialog::view_hotkeys_dialog",
        "task_lookup_dialog::view_task_lookup_dialog",
        "edit_annotation_dialog::view_edit_annotation_dialog",
        "no_work_dialog::view_no_work_dialog",
        "exit_confirm_dialog::view_exit_confirm_dialog",
    ] {
        assert!(
            shell_source.contains(&format!("view_modal_layer(\n                {dialog}"))
                || shell_source.contains(&format!("view_modal_layer({dialog}")),
            "{dialog} must render inside the shared modal layer instead of the page column"
        );
        assert!(
            !shell_source.contains(&format!("layout = layout.push({dialog}")),
            "{dialog} must not be pushed into the editor column"
        );
    }
    assert!(
        shell_source
            .contains("view_modal_layer(no_work_dialog::view_no_work_dialog(no_work_modal), None)"),
        "no-work modal must not be dismissible from the scrim"
    );
}

#[test]
fn window_close_request_with_dirty_editor_opens_exit_confirm_dialog() {
    let mut state = stable_sample_state();
    state.editor.as_mut().unwrap().is_dirty = true;

    let _ = update(&mut state, Message::WindowCloseRequested);

    assert!(state.show_exit_confirm_dialog);

    let _ = update(&mut state, Message::EscapePressed);

    assert!(!state.show_exit_confirm_dialog);

    let _ = update(&mut state, Message::WindowCloseRequested);
    assert!(state.show_exit_confirm_dialog);

    let _ = update(&mut state, Message::CancelExit);
    assert!(!state.show_exit_confirm_dialog);
}

#[test]
fn window_close_request_with_clean_editor_skips_exit_confirm_dialog() {
    let mut state = stable_sample_state();
    state.editor.as_mut().unwrap().is_dirty = false;

    let _ = update(&mut state, Message::WindowCloseRequested);

    assert!(!state.show_exit_confirm_dialog);
}

#[test]
fn window_title_reflects_project_name_and_unsaved_changes() {
    let mut state = stable_sample_state();
    state.editor.as_mut().unwrap().is_dirty = false;
    let clean_title = super::window_title(&state);
    assert!(clean_title.ends_with("Transcription Studio"));
    assert!(!clean_title.starts_with('\u{25CF}'));

    state.editor.as_mut().unwrap().is_dirty = true;
    let dirty_title = super::window_title(&state);
    assert!(dirty_title.starts_with('\u{25CF}'));
    assert!(dirty_title.ends_with("Transcription Studio"));
}

#[test]
fn transcript_virtual_window_pins_out_of_window_row_as_exact_island() {
    let row_heights = vec![30.0; 1_200];
    let input = |pinned| super::transcript_virtualization::TranscriptVirtualizationInput {
        scroll_offset_y: 900.0,
        viewport_height: 600.0,
        overscan_height: 600.0,
        render_all_threshold: 100,
        pinned_row_index: pinned,
    };

    let window =
        super::transcript_virtualization::transcript_virtual_window(&row_heights, input(None));
    assert_eq!(window.pinned_row, None);

    let inside =
        super::transcript_virtualization::transcript_virtual_window(&row_heights, input(Some(30)));
    assert_eq!(inside.pinned_row, None, "in-window rows need no island");

    let after =
        super::transcript_virtualization::transcript_virtual_window(&row_heights, input(Some(500)));
    let pinned = after
        .pinned_row
        .expect("row after window must stay mounted");
    assert_eq!(pinned.index, 500);
    assert_eq!(
        pinned.leading_height + 30.0 + pinned.trailing_height,
        after.after_height,
        "island spacers must replace the after spacer exactly"
    );

    let before =
        super::transcript_virtualization::transcript_virtual_window(&row_heights, input(Some(5)));
    let pinned = before
        .pinned_row
        .expect("row before window must stay mounted");
    assert_eq!(pinned.index, 5);
    assert_eq!(
        pinned.leading_height + 30.0 + pinned.trailing_height,
        before.before_height,
        "island spacers must replace the before spacer exactly"
    );
}

#[test]
fn selected_row_stays_mounted_as_island_when_scrolled_away() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(virtualized_two_lane_project()),
        "Loaded large Babel-like project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    let far_segment_id = {
        let editor = state.editor.as_ref().unwrap();
        let rows = state.transcript_rows_snapshot_for_render(&project, editor);
        rows[240].segment_id.clone()
    };
    let _ = state.select_segment_without_seek(&far_segment_id);
    let _ = update(
        &mut state,
        Message::TranscriptScrolled {
            offset_y: 0.0,
            viewport_height: 600.0,
        },
    );

    let editor = state.editor.as_ref().unwrap();
    let rows = state.transcript_rows_snapshot_for_render(&project, editor);
    let layout = state.transcript_layout_for_render(&rows, editor);
    let pinned = layout
        .virtual_window
        .pinned_row
        .expect("selected row must stay mounted so focus operations can reach its editor");
    assert_eq!(rows[pinned.index].segment_id, far_segment_id);
    assert!(pinned.index >= layout.virtual_window.end_index);

    let total: f32 = layout.row_heights.iter().sum();
    let rendered: f32 = layout.row_heights
        [layout.virtual_window.start_index..layout.virtual_window.end_index]
        .iter()
        .sum();
    let island_height = layout.row_heights[pinned.index];
    let composed = layout.virtual_window.before_height
        + rendered
        + pinned.leading_height
        + island_height
        + pinned.trailing_height;
    assert!(
        (composed - total).abs() < 0.5,
        "island spacers must preserve total content height: composed={composed} total={total}"
    );
}

#[test]
fn playback_follow_scrolls_transcript_to_row_under_playhead() {
    let mut state = stable_sample_state();
    state.replace_workflow(
        StudioWorkflow::from_project(virtualized_two_lane_project()),
        "Loaded large Babel-like project.".to_string(),
    );
    let project = state.project().unwrap().clone();
    let far_segment_id = {
        let editor = state.editor.as_ref().unwrap();
        let rows = state.transcript_rows_snapshot_for_render(&project, editor);
        rows[200].segment_id.clone()
    };
    state.with_editor_mut(|editor| {
        editor.select_segment(&project, &far_segment_id);
        if !editor.is_playing {
            editor.toggle_playback();
        }
    });
    let _ = update(
        &mut state,
        Message::TranscriptScrolled {
            offset_y: 0.0,
            viewport_height: 600.0,
        },
    );
    assert_eq!(state.transcript_scroll_offset_y, 0.0);

    let start = Instant::now();
    let _ = update(&mut state, Message::TransportTick(start));

    assert_eq!(
        state.playback_followed_segment_id.as_deref(),
        Some(far_segment_id.as_str()),
        "playback must track the row under the playhead"
    );
    assert!(
        state.transcript_scroll_offset_y > 0.0,
        "playback must scroll the out-of-view current row into the viewport"
    );

    let followed_offset = state.transcript_scroll_offset_y;
    let _ = update(
        &mut state,
        Message::TransportTick(start + Duration::from_millis(16)),
    );
    assert_eq!(
        state.transcript_scroll_offset_y, followed_offset,
        "follow must not re-scroll while playback stays within the same row"
    );
}
