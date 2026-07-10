mod actions;
mod annotation_smoke;
mod audio_backend;
mod babel_audio;
mod browser_behavior_smoke;
mod browser_input;
mod components;
mod context_panels;
mod create_region;
mod diff_smoke;
mod drafts;
mod edit_annotation;
mod feedback_panel;
mod find;
mod formatting;
mod helper_lint_autofix;
mod helper_text;
mod helper_timeline;
mod hotkeys;
mod io_actions;
mod keybinds;
mod large_project_smoke;
mod message;
mod native_auth;
mod native_auth_cli;
mod native_auth_flow;
mod native_capabilities;
mod navigation;
mod panel_persistence;
mod platform_paths;
mod recovery;
mod renderer_capture;
mod review_backend_client;
mod segment_actions;
mod service_callbacks;
mod service_executor;
mod startup_route;
mod state;
mod styles;
mod subaccounts;
mod tag_suggestions;
mod task_lookup;
mod time_edit;
mod transcript_virtualization;
mod updater;
mod workbench_shell_smoke;

use iced::{Size, window};

use actions::update;
use annotation_smoke::{annotation_mutation_smoke_enabled, run_annotation_mutation_smoke};
use browser_behavior_smoke::{browser_behavior_smoke_enabled, run_browser_behavior_smoke};
use components::view;
use create_region::CreateRegionDialog;
use diff_smoke::{diff_workbench_smoke_mode, run_diff_workbench_smoke};
use drafts::SegmentDraft;
use edit_annotation::EditAnnotationDialog;
use feedback_panel::FeedbackPanelState;
use large_project_smoke::{large_project_smoke_enabled, run_large_project_smoke};
use message::{AuditionKey, Message, RuntimeGraphicsInfo};
use native_auth_flow::{NativeAuthFlowState, NativeAuthStartup};
use startup_route::{apply_startup_route_config, parse_startup_route_config};
use state::{
    DEFAULT_EXPORT_PATH, DEFAULT_IMPORT_PATH, StudioState,
    startup_playback_smoke_delay_frames_from_env, startup_playback_smoke_enabled,
};
use time_edit::TimeField;

const RUNTIME_GRAPHICS_INFO_ENV: &str = "STUDIO_LOG_RUNTIME_GRAPHICS";

pub fn run_native_auth_cli_from_args(
    args: impl IntoIterator<Item = impl AsRef<str>>,
) -> Option<Result<(), String>> {
    native_auth_cli::run_native_auth_cli_from_args(args)
}

pub fn run() -> iced::Result {
    let startup_route = parse_startup_route_config(std::env::args());
    let load_bundled_sample_audio = startup_route == Default::default();
    iced::application(window_title, update, view)
        .antialiasing(true)
        .subscription(hotkeys::subscription)
        .theme(|_| styles::studio_theme())
        .window(window::Settings {
            size: Size::new(1440.0, 900.0),
            min_size: Some(Size::new(1200.0, 760.0)),
            exit_on_close_request: false,
            ..window::Settings::default()
        })
        .run_with(move || {
            let mut state = StudioState::default();
            apply_startup_route_config(&mut state, startup_route);
            state.open_native_auth_gate_from_store();
            let diff_smoke_mode = diff_workbench_smoke_mode();
            if annotation_mutation_smoke_enabled() {
                run_annotation_mutation_smoke(&mut state);
            }
            if let Some(diff_smoke_mode) = diff_smoke_mode {
                run_diff_workbench_smoke(&mut state, diff_smoke_mode);
            }
            if large_project_smoke_enabled() {
                run_large_project_smoke(&mut state);
            }
            if browser_behavior_smoke_enabled() {
                run_browser_behavior_smoke(&mut state);
            }
            if load_bundled_sample_audio && startup_playback_smoke_enabled() {
                if let Some(delay_frames) = startup_playback_smoke_delay_frames_from_env() {
                    state.load_bundled_sample_waveforms_for_runtime();
                    state.schedule_startup_playback_smoke_after_frames(delay_frames);
                } else {
                    state.start_recovered_audio_playback_smoke_for_runtime();
                }
            } else if load_bundled_sample_audio {
                state.load_bundled_sample_waveforms_for_runtime();
            }
            let mut startup_tasks = vec![
                state.dispatch_new_babel_service_requests(),
                state.dispatch_new_review_backend_requests(),
            ];
            if runtime_graphics_info_enabled() {
                startup_tasks.push(iced::system::fetch_information().map(|information| {
                    Message::RuntimeGraphicsInfoLoaded(RuntimeGraphicsInfo {
                        backend: information.graphics_backend,
                        adapter: information.graphics_adapter,
                    })
                }));
            }
            let task = iced::Task::batch(startup_tasks);
            (state, task)
        })
}

fn runtime_graphics_info_enabled() -> bool {
    std::env::var_os(RUNTIME_GRAPHICS_INFO_ENV).is_some()
}

fn window_title(state: &StudioState) -> String {
    const APP_TITLE: &str = "Transcription Studio";
    if state.show_dashboard() {
        return APP_TITLE.to_string();
    }
    let Some(project) = state.project() else {
        return APP_TITLE.to_string();
    };
    let unsaved_marker = if state.editor.as_ref().is_some_and(|editor| editor.is_dirty) {
        "● "
    } else {
        ""
    };
    let project_name = project.name.trim();
    if project_name.is_empty() {
        format!("{unsaved_marker}{APP_TITLE}")
    } else {
        format!("{unsaved_marker}{project_name} — {APP_TITLE}")
    }
}

#[cfg(test)]
mod tests;
