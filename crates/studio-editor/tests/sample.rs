use studio_editor::load_sample_project;

#[test]
fn native_sample_loads_two_lanes_without_platform_metadata() {
    let project = load_sample_project().expect("sample project should load");

    assert_eq!(
        project
            .lanes
            .iter()
            .map(|lane| lane.id.as_str())
            .collect::<Vec<_>>(),
        vec!["track-speaker-1", "track-speaker-2"]
    );
    assert_eq!(project.segments.len(), 24);
    assert!(project.metadata.is_empty());
    assert!(
        project
            .segments
            .iter()
            .all(|segment| segment.metadata.is_empty())
    );
}
