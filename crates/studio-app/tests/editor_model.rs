use studio_app::{EditorSession, LintSeverity, ReviewState, TrackFilter, load_sample_project};
use studio_core::{AudioLane, Project, Segment, Speaker};

fn sample_project() -> Project {
    load_sample_project().expect("sample project should load")
}

fn project_with_segments(segments: Vec<Segment>) -> Project {
    Project {
        id: "project".to_string(),
        name: "Project".to_string(),
        speakers: vec![Speaker {
            id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
        }],
        lanes: vec![AudioLane {
            id: "track-1".to_string(),
            speaker_id: "speaker-1".to_string(),
            label: "Speaker 1".to_string(),
            source_path: "audio/speaker-1.wav".to_string(),
        }],
        segments,
        metadata: Default::default(),
    }
}

fn segment(id: &str, start_seconds: f64, end_seconds: f64, text: &str) -> Segment {
    Segment {
        id: id.to_string(),
        source_annotation_id: Some(id.to_string()),
        speaker_id: "speaker-1".to_string(),
        lane_id: "track-1".to_string(),
        start_seconds,
        end_seconds,
        text: text.to_string(),
    }
}

#[test]
fn editor_session_defaults_to_babel_workspace_state() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    assert_eq!(session.speed, 1.0);
    assert_eq!(session.zoom, 1.0);
    assert!(!session.is_playing);
    assert!(!session.diff_view);
    assert!(!session.loop_enabled);
    assert_eq!(session.track_filter, TrackFilter::All);
    assert_eq!(session.review_state, ReviewState::Draft);
    assert_eq!(session.selected_segment_id.as_deref(), Some("annotation-1"));
    assert_eq!(session.viewport.start_seconds, 0.0);
    assert_eq!(session.viewport.end_seconds, 7.2);
}

#[test]
fn track_filter_options_include_all_tracks_and_each_lane() {
    let project = sample_project();
    let options = EditorSession::track_filter_options(&project);

    assert_eq!(options[0], TrackFilter::All);
    assert_eq!(
        options.iter().map(TrackFilter::label).collect::<Vec<_>>(),
        vec!["All Tracks", "Speaker 1", "Speaker 2"]
    );
}

#[test]
fn transcript_rows_respect_selected_track_filter_and_format_times() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.track_filter = TrackFilter::Lane("track-speaker-2".to_string());

    let rows = session.transcript_rows(&project);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].display_id, "02");
    assert_eq!(rows[0].speaker_label, "Speaker 2");
    assert_eq!(rows[0].start_time, "00:03.50");
    assert_eq!(rows[0].end_time, "00:05.10");
    assert_eq!(rows[0].lint.severity, LintSeverity::Pass);
}

#[test]
fn timeline_lanes_include_deterministic_waveform_blocks_and_colors() {
    let project = sample_project();
    let session = EditorSession::for_project(&project);

    let lanes = session.timeline_lanes(&project);

    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0].label, "Speaker 1");
    assert_eq!(lanes[0].color_token, "purple");
    assert_eq!(lanes[0].blocks.len(), 2);
    assert_eq!(lanes[1].color_token, "blue");
    assert_eq!(lanes[1].blocks.len(), 1);
    assert!(lanes[0].blocks[0].left_percent > 0.0);
    assert!(lanes[0].blocks[0].width_percent > 0.0);
    assert_eq!(lanes[0].blocks[0].peaks.len(), 28);
    assert_eq!(lanes[0].blocks[0].peaks, lanes[0].blocks[0].peaks);
}

#[test]
fn linter_reports_empty_text_invalid_timing_missing_lane_and_overlap() {
    let mut project = project_with_segments(vec![
        segment("one", 1.0, 3.0, ""),
        segment("two", 2.5, 4.0, "overlap"),
        segment("three", 5.0, 4.0, "bad timing"),
        segment("four", 6.0, 7.0, "missing lane"),
    ]);
    project.segments[3].lane_id = "missing".to_string();
    let session = EditorSession::for_project(&project);

    let rows = session.transcript_rows(&project);

    assert_eq!(rows[0].lint.severity, LintSeverity::Error);
    assert!(rows[0].lint.message.contains("text is empty"));
    assert_eq!(rows[1].lint.severity, LintSeverity::Error);
    assert!(rows[1].lint.message.contains("overlaps"));
    assert_eq!(rows[2].lint.severity, LintSeverity::Error);
    assert!(rows[2].lint.message.contains("start before end"));
    assert_eq!(rows[3].lint.severity, LintSeverity::Error);
    assert!(rows[3].lint.message.contains("unknown track"));
}

#[test]
fn submit_review_is_blocked_by_lint_errors_and_marks_clean_project_submitted() {
    let project = sample_project();
    let mut session = EditorSession::for_project(&project);
    session.mark_dirty();
    session.mark_saved();

    session
        .submit_review(&project)
        .expect("clean project should submit");
    assert_eq!(session.review_state, ReviewState::Submitted);

    let bad_project = project_with_segments(vec![segment("bad", 1.0, 2.0, "")]);
    let mut bad_session = EditorSession::for_project(&bad_project);
    let error = bad_session
        .submit_review(&bad_project)
        .expect_err("lint errors should block submit");
    assert!(error.contains("1 blocking"));
    assert_eq!(bad_session.review_state, ReviewState::Draft);
}
