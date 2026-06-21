use studio_app::{load_sample_project, timeline_lane_rows, transport_status};

#[test]
fn sample_project_loads_two_lanes_and_three_segments() {
    let project = load_sample_project().expect("sample project should load");

    assert_eq!(project.lanes.len(), 2);
    assert_eq!(project.segments.len(), 3);
}

#[test]
fn timeline_lane_rows_group_segments_by_lane() {
    let project = load_sample_project().expect("sample project should load");

    let rows = timeline_lane_rows(&project);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].lane.id, "track-speaker-1");
    assert_eq!(rows[0].lane.label, "Speaker 1");
    assert_eq!(rows[0].segments.len(), 2);
    assert_eq!(rows[1].lane.id, "track-speaker-2");
    assert_eq!(rows[1].lane.label, "Speaker 2");
    assert_eq!(rows[1].segments.len(), 1);
}

#[test]
fn transport_status_includes_project_name_and_segment_count() {
    let project = load_sample_project().expect("sample project should load");

    let status = transport_status(&project);

    assert!(status.contains("Babel task babel-task-ru-demo"));
    assert!(status.contains("3 segments"));
}
