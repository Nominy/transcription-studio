use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;
use studio_app::{LaneWaveform, StudioWorkflow, TimeRange};
use studio_core::{AudioLane, Project, Segment, Speaker};
use studio_waveform::DecodedAudio;

use super::actions::update;
use super::renderer_capture::renderer_screenshot_task;
use super::{Message, StudioState};

const LARGE_PROJECT_SMOKE_ENV: &str = "STUDIO_SMOKE_LARGE_PROJECT_ON_START";
const LARGE_PROJECT_SYNTHETIC_WAVEFORMS_ENV: &str =
    "STUDIO_SMOKE_LARGE_PROJECT_SYNTHETIC_WAVEFORMS";
const LARGE_PROJECT_KEEP_PLAYING_ENV: &str = "STUDIO_SMOKE_LARGE_PROJECT_KEEP_PLAYING";
const LARGE_PROJECT_LIVE_PAN_ENV: &str = "STUDIO_SMOKE_LARGE_PROJECT_LIVE_PAN";
const LARGE_PROJECT_LIVE_TRANSCRIPT_SCROLL_ENV: &str =
    "STUDIO_SMOKE_LARGE_PROJECT_LIVE_TRANSCRIPT_SCROLL";
const LARGE_PROJECT_SEGMENT_COUNT: usize = 1_200;
const LARGE_PROJECT_FRAME_COUNT: usize = 4;
const LARGE_PROJECT_INTERACTION_FRAME_COUNT: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LargeProjectSmokeReport {
    pub(super) rows: usize,
    pub(super) rendered_rows: usize,
    pub(super) lanes: usize,
    pub(super) frames: usize,
    pub(super) cache_misses: usize,
    pub(super) layout_cache_misses: usize,
    pub(super) timeline_cache_misses: usize,
    pub(super) overview_cache_misses: usize,
    pub(super) transport_cache_misses: usize,
    pub(super) submit_cache_misses: usize,
    pub(super) view_build_frames: usize,
    pub(super) view_build_micros_total: u128,
    pub(super) view_build_micros_max: u128,
    pub(super) interaction_frames: usize,
    pub(super) interaction_view_build_micros_total: u128,
    pub(super) interaction_view_build_micros_max: u128,
    pub(super) interaction_timeline_cache_misses: usize,
    pub(super) interaction_overview_cache_misses: usize,
    pub(super) interaction_waveform_cache_hits: usize,
    pub(super) interaction_waveform_cache_misses: usize,
    pub(super) peak_cache_counts: Vec<(String, usize)>,
    pub(super) decoded_waveforms: bool,
    pub(super) stable_rows: bool,
    pub(super) stable_lanes: bool,
    pub(super) stable_overview: bool,
    pub(super) stable_transport: bool,
    pub(super) stable_submit: bool,
}

impl LargeProjectSmokeReport {
    pub(super) fn runtime_trace_line(&self) -> String {
        format!(
            "Transcription Studio large project smoke: rows={} rendered_rows={} lanes={} frames={} decoded_waveforms={} cache_misses={} layout_cache_misses={} timeline_cache_misses={} overview_cache_misses={} transport_cache_misses={} submit_cache_misses={} view_build_frames={} view_build_micros_total={} view_build_micros_max={} interaction_frames={} interaction_view_build_micros_total={} interaction_view_build_micros_max={} interaction_timeline_cache_misses={} interaction_overview_cache_misses={} interaction_waveform_cache_hits={} interaction_waveform_cache_misses={} peak_caches={} stable_rows={} stable_lanes={} stable_overview={} stable_transport={} stable_submit={}",
            self.rows,
            self.rendered_rows,
            self.lanes,
            self.frames,
            self.decoded_waveforms,
            self.cache_misses,
            self.layout_cache_misses,
            self.timeline_cache_misses,
            self.overview_cache_misses,
            self.transport_cache_misses,
            self.submit_cache_misses,
            self.view_build_frames,
            self.view_build_micros_total,
            self.view_build_micros_max,
            self.interaction_frames,
            self.interaction_view_build_micros_total,
            self.interaction_view_build_micros_max,
            self.interaction_timeline_cache_misses,
            self.interaction_overview_cache_misses,
            self.interaction_waveform_cache_hits,
            self.interaction_waveform_cache_misses,
            self.peak_cache_counts
                .iter()
                .map(|(lane_id, count)| format!("{lane_id}:{count}"))
                .collect::<Vec<_>>()
                .join(","),
            self.stable_rows,
            self.stable_lanes,
            self.stable_overview,
            self.stable_transport,
            self.stable_submit
        )
    }
}

pub(super) fn large_project_smoke_enabled() -> bool {
    std::env::var(LARGE_PROJECT_SMOKE_ENV).is_ok_and(|value| value != "0")
}

fn large_project_synthetic_waveforms_enabled() -> bool {
    std::env::var(LARGE_PROJECT_SYNTHETIC_WAVEFORMS_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn large_project_keep_playing_enabled() -> bool {
    std::env::var(LARGE_PROJECT_KEEP_PLAYING_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn large_project_live_pan_enabled() -> bool {
    std::env::var(LARGE_PROJECT_LIVE_PAN_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn large_project_live_transcript_scroll_enabled() -> bool {
    std::env::var(LARGE_PROJECT_LIVE_TRANSCRIPT_SCROLL_ENV).is_ok_and(|value| value != "0")
}

pub(super) fn large_project_renderer_screenshot_task() -> Task<Message> {
    let directory = if large_project_synthetic_waveforms_enabled() {
        "runtime-large-project-synthetic-smoke"
    } else {
        "runtime-large-project-smoke"
    };
    renderer_screenshot_task(
        PathBuf::from("target")
            .join(directory)
            .join("large-project-renderer.bmp"),
    )
}

pub(super) fn run_large_project_smoke(state: &mut StudioState) -> LargeProjectSmokeReport {
    run_large_project_smoke_with_waveforms(state, !large_project_synthetic_waveforms_enabled())
}

#[cfg(test)]
pub(super) fn run_large_project_synthetic_waveform_smoke(
    state: &mut StudioState,
) -> LargeProjectSmokeReport {
    run_large_project_smoke_with_waveforms(state, false)
}

fn run_large_project_smoke_with_waveforms(
    state: &mut StudioState,
    decoded_waveforms: bool,
) -> LargeProjectSmokeReport {
    let project = large_two_lane_project(LARGE_PROJECT_SEGMENT_COUNT);
    let duration_seconds = TimeRange::for_project(&project).end_seconds + 1.0;
    state.replace_workflow(
        StudioWorkflow::from_project(project),
        "Loaded large Babel-like smoke project.".to_string(),
    );
    state.with_editor_mut(|editor| {
        if decoded_waveforms {
            editor.set_lane_waveform(
                "track-1",
                LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
            );
            editor.set_lane_waveform(
                "track-2",
                LaneWaveform::from_decoded(decoded_audio_for_duration(duration_seconds)),
            );
        }
        editor.toggle_playback();
    });

    let project = state
        .project()
        .expect("large smoke project should remain loaded")
        .clone();
    let first_rows = {
        let editor = state
            .editor
            .as_ref()
            .expect("large smoke editor should remain loaded");
        state.transcript_rows_snapshot_for_render(&project, editor)
    };
    let rendered_rows = {
        let editor = state
            .editor
            .as_ref()
            .expect("large smoke editor should remain loaded");
        state
            .transcript_layout_for_render(&first_rows, editor)
            .virtual_window
            .rendered_len()
    };
    let first_overview = {
        let editor = state
            .editor
            .as_ref()
            .expect("large smoke editor should remain loaded");
        state.overview_waveform_snapshot_for_render(&project, editor)
    };
    let (lane_count, first_transport, first_submit) = {
        let editor = state
            .editor
            .as_ref()
            .expect("large smoke editor should remain loaded");
        let first_transport = state.transport_strip_snapshot_for_render(&project, editor);
        let first_submit = state.submit_control_snapshot_for_render(&project, editor);
        let lane_count = state
            .timeline_lanes_snapshot_for_render(&project, editor)
            .len();
        (lane_count, first_transport, first_submit)
    };
    let first_lanes = {
        let editor = state
            .editor
            .as_ref()
            .expect("large smoke editor should remain loaded");
        state.timeline_lanes_snapshot_for_render(&project, editor)
    };
    let start_tick = Instant::now();
    let mut stable_rows = true;
    let mut stable_lanes = true;
    let mut stable_overview = true;
    let mut stable_transport = true;
    let mut stable_submit = true;
    let mut view_build_micros_total = 0;
    let mut view_build_micros_max = 0;

    for frame in 0..LARGE_PROJECT_FRAME_COUNT {
        let _ = update(
            state,
            Message::TransportTick(start_tick + Duration::from_millis(16 * frame as u64)),
        );
        let editor = state
            .editor
            .as_ref()
            .expect("large smoke editor should remain loaded");
        let rows = state.transcript_rows_snapshot_for_render(&project, editor);
        stable_rows &= std::sync::Arc::ptr_eq(&first_rows, &rows);
        let _ = state.transcript_layout_for_render(&rows, editor);
        let overview = state.overview_waveform_snapshot_for_render(&project, editor);
        stable_overview &= Arc::ptr_eq(&first_overview.overview, &overview.overview);
        let transport = state.transport_strip_snapshot_for_render(&project, editor);
        stable_transport &= Arc::ptr_eq(&first_transport, &transport);
        let submit = state.submit_control_snapshot_for_render(&project, editor);
        stable_submit &= Arc::ptr_eq(&first_submit, &submit);
        let lanes = state.timeline_lanes_snapshot_for_render(&project, editor);
        stable_lanes &= Arc::ptr_eq(&first_lanes, &lanes);
        let view_build_start = Instant::now();
        std::hint::black_box(super::components::view(state));
        let view_build_micros = view_build_start.elapsed().as_micros();
        view_build_micros_total += view_build_micros;
        view_build_micros_max = view_build_micros_max.max(view_build_micros);
    }
    let passive_cache_misses = state.transcript_rows_cache_miss_count();
    let passive_layout_cache_misses = state.transcript_layout_cache_miss_count();
    let passive_timeline_cache_misses = state.timeline_lanes_cache_miss_count();
    let passive_overview_cache_misses = state.overview_waveform_cache_miss_count();
    let passive_transport_cache_misses = state.transport_strip_cache_miss_count();
    let passive_submit_cache_misses = state.submit_control_cache_miss_count();
    let interaction = run_large_project_interaction_smoke(state, &project);
    if !large_project_keep_playing_enabled() {
        state.with_editor_mut(|editor| {
            if editor.is_playing {
                editor.toggle_playback();
            }
        });
    } else {
        state.reset_frame_cadence_probe_from_env();
        state.enable_large_project_live_pan_smoke();
        state.enable_large_project_live_transcript_scroll_smoke();
    }

    let editor = state
        .editor
        .as_ref()
        .expect("large smoke editor should remain loaded");
    let mut peak_cache_counts = ["track-1", "track-2"]
        .into_iter()
        .filter_map(|lane_id| {
            editor
                .lane_waveforms
                .get(lane_id)
                .map(|waveform| (lane_id.to_string(), waveform.cached_peak_resolution_count()))
        })
        .collect::<Vec<_>>();
    peak_cache_counts.sort_by(|left, right| left.0.cmp(&right.0));

    let report = LargeProjectSmokeReport {
        rows: first_rows.len(),
        rendered_rows,
        lanes: lane_count,
        frames: LARGE_PROJECT_FRAME_COUNT,
        cache_misses: passive_cache_misses,
        layout_cache_misses: passive_layout_cache_misses,
        timeline_cache_misses: passive_timeline_cache_misses,
        overview_cache_misses: passive_overview_cache_misses,
        transport_cache_misses: passive_transport_cache_misses,
        submit_cache_misses: passive_submit_cache_misses,
        view_build_frames: LARGE_PROJECT_FRAME_COUNT,
        view_build_micros_total,
        view_build_micros_max,
        interaction_frames: interaction.frames,
        interaction_view_build_micros_total: interaction.view_build_micros_total,
        interaction_view_build_micros_max: interaction.view_build_micros_max,
        interaction_timeline_cache_misses: interaction.timeline_cache_misses,
        interaction_overview_cache_misses: interaction.overview_cache_misses,
        interaction_waveform_cache_hits: interaction.waveform_cache_hits,
        interaction_waveform_cache_misses: interaction.waveform_cache_misses,
        peak_cache_counts,
        decoded_waveforms,
        stable_rows,
        stable_lanes,
        stable_overview,
        stable_transport,
        stable_submit,
    };
    eprintln!("{}", report.runtime_trace_line());
    report
}

#[derive(Debug, Default)]
struct LargeProjectInteractionSmokeReport {
    frames: usize,
    view_build_micros_total: u128,
    view_build_micros_max: u128,
    timeline_cache_misses: usize,
    overview_cache_misses: usize,
    waveform_cache_hits: usize,
    waveform_cache_misses: usize,
}

fn run_large_project_interaction_smoke(
    state: &mut StudioState,
    project: &Project,
) -> LargeProjectInteractionSmokeReport {
    let Some(editor) = state.editor.as_ref() else {
        return LargeProjectInteractionSmokeReport::default();
    };
    let original_zoom = editor.zoom;
    let original_viewport = editor.viewport;
    let original_playhead_seconds = editor.playhead_seconds;
    let original_selected_segment_id = editor.selected_segment_id.clone();
    let original_timeline_misses = state.timeline_lanes_cache_miss_count();
    let original_overview_misses = state.overview_waveform_cache_miss_count();
    let (original_waveform_hits, original_waveform_misses) = waveform_render_cache_totals(state);

    state.with_editor_mut(|editor| {
        editor.seek_to_time(project, 22.0);
        editor.set_zoom(project, 1.0);
    });

    let mut view_build_micros_total = 0;
    let mut view_build_micros_max = 0;
    for frame in 0..LARGE_PROJECT_INTERACTION_FRAME_COUNT {
        let delta_x = if frame % 2 == 0 { 1.0 } else { -1.0 };
        let _ = update(
            state,
            Message::TimelineWheelScrolled {
                delta_x,
                delta_y: 0.0,
            },
        );
        let view_build_start = Instant::now();
        std::hint::black_box(super::components::view(state));
        let view_build_micros = view_build_start.elapsed().as_micros();
        view_build_micros_total += view_build_micros;
        view_build_micros_max = view_build_micros_max.max(view_build_micros);
    }

    let timeline_cache_misses = state
        .timeline_lanes_cache_miss_count()
        .saturating_sub(original_timeline_misses);
    let overview_cache_misses = state
        .overview_waveform_cache_miss_count()
        .saturating_sub(original_overview_misses);
    let (waveform_hits, waveform_misses) = waveform_render_cache_totals(state);

    state.with_editor_mut(|editor| {
        editor.zoom = original_zoom;
        editor.viewport = original_viewport;
        editor.playhead_seconds = original_playhead_seconds;
        editor.selected_segment_id = original_selected_segment_id;
    });

    LargeProjectInteractionSmokeReport {
        frames: LARGE_PROJECT_INTERACTION_FRAME_COUNT,
        view_build_micros_total,
        view_build_micros_max,
        timeline_cache_misses,
        overview_cache_misses,
        waveform_cache_hits: waveform_hits.saturating_sub(original_waveform_hits),
        waveform_cache_misses: waveform_misses.saturating_sub(original_waveform_misses),
    }
}

fn waveform_render_cache_totals(state: &StudioState) -> (usize, usize) {
    state
        .editor
        .as_ref()
        .map(|editor| {
            editor
                .lane_waveforms
                .values()
                .map(|waveform| waveform.render_cache_stats())
                .fold((0, 0), |(hits, misses), stats| {
                    (hits + stats.hits, misses + stats.misses)
                })
        })
        .unwrap_or((0, 0))
}

fn large_two_lane_project(segment_count: usize) -> Project {
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
        metadata: BTreeMap::new(),
    });

    for index in 1..segment_count {
        let is_first_lane = index % 2 == 0;
        let lane_id = if is_first_lane { "track-1" } else { "track-2" };
        let speaker_id = if is_first_lane {
            "speaker-1"
        } else {
            "speaker-2"
        };
        let start_seconds = 20.0 + index as f64;
        segments.push(Segment {
            id: format!("annotation-{index:04}"),
            source_annotation_id: Some(format!("annotation-{index:04}")),
            speaker_id: speaker_id.to_string(),
            lane_id: lane_id.to_string(),
            start_seconds,
            end_seconds: start_seconds + 0.72,
            text: format!("large project row {index} with unique transcript text"),
            metadata: BTreeMap::new(),
        });
    }

    Project {
        id: "large-project-smoke".to_string(),
        name: "Large Babel Project Smoke".to_string(),
        speakers,
        lanes,
        segments,
        metadata: BTreeMap::new(),
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
