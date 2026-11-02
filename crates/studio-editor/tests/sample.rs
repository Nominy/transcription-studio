use studio_editor::load_sample_project;

#[test]
fn sample_project_loads_two_lanes_and_rich_babel_segments() {
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
}
