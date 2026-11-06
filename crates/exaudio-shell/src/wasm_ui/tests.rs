use super::*;
use core::prelude::v1::test;
fn task(id: u64) -> w::Progress {
    w::Progress {
        task_id: id,
        label: "Task".into(),
        completed: 0,
        total: Some(100),
    }
}
#[test]
fn progress_has_an_aggregate_owner_quota_and_reclaims_completed_tasks() {
    let mut owner = OwnerUi::default();
    for id in 0..64 {
        owner.update_progress(task(id)).unwrap();
    }
    assert!(owner.update_progress(task(64)).is_err());
    assert_eq!(owner.progress.len(), 64);
    owner
        .update_progress(w::Progress {
            completed: 100,
            ..task(0)
        })
        .unwrap();
    assert_eq!(owner.progress.len(), 63);
    owner.update_progress(task(64)).unwrap();
    assert_eq!(owner.progress.len(), 64);
    assert!(
        owner
            .update_progress(w::Progress {
                label: "x".repeat(4097),
                ..task(1)
            })
            .is_err()
    );
}
fn node(id: &str, parent: Option<u32>) -> w::UiNode {
    w::UiNode {
        id: id.into(),
        parent,
        widget: w::Widget::Stack,
        config: w::WidgetConfig::Plain,
        text: String::new(),
        value: String::new(),
        enabled: true,
        command: None,
    }
}
#[test]
fn typed_tree_rejects_duplicate_ids_cycles_and_deep_nesting() {
    assert!(
        validate(&w::UiTree {
            contribution: "panel".into(),
            nodes: vec![node("same", None), node("same", Some(0))]
        })
        .is_err()
    );
    assert!(
        validate(&w::UiTree {
            contribution: "panel".into(),
            nodes: vec![node("cycle", Some(0))]
        })
        .is_err()
    );
    let nodes = (0..34)
        .map(|index| {
            node(
                &index.to_string(),
                if index == 0 { None } else { Some(index - 1) },
            )
        })
        .collect();
    assert!(
        validate(&w::UiTree {
            contribution: "panel".into(),
            nodes
        })
        .is_err()
    );
}

#[test]
fn tab_switches_show_only_the_selected_subtree_and_preserve_other_roots() {
    let mut tabs = node("tabs", None);
    tabs.widget = w::Widget::Tabs;
    tabs.config = w::WidgetConfig::Choices(w::ChoiceConfig {
        options: vec![
            w::Choice {
                key: "a".into(),
                label: "A".into(),
            },
            w::Choice {
                key: "b".into(),
                label: "B".into(),
            },
        ],
        selected: Some("a".into()),
    });
    let tree = w::UiTree {
        contribution: "panel".into(),
        nodes: vec![
            tabs,
            node("a", Some(0)),
            node("a-child", Some(1)),
            node("b", Some(0)),
            node("b-child", Some(3)),
            node("other-root", None),
        ],
    };
    validate(&tree).unwrap();
    let mut values = BTreeMap::new();
    assert_eq!(
        visible_nodes(&tree, &values),
        vec![true, true, true, false, false, true]
    );
    values.insert("tabs".into(), "b".into());
    assert_eq!(
        visible_nodes(&tree, &values),
        vec![true, false, false, true, true, true]
    );
    values.insert("tabs".into(), "unavailable".into());
    assert_eq!(
        visible_nodes(&tree, &values),
        vec![true, false, false, false, false, true]
    );
    values.insert("tabs".into(), String::new());
    assert_eq!(
        visible_nodes(&tree, &values),
        vec![true, true, true, false, false, true]
    );
}
#[test]
fn overlapping_decoration_ranges_become_disjoint_shaping_runs() {
    let ranges = disjoint_highlights(vec![
        (1..5, HighlightStyle::default()),
        (3..7, HighlightStyle::default()),
        (2..4, HighlightStyle::default()),
    ]);
    assert_eq!(
        ranges
            .iter()
            .map(|(range, _)| range.clone())
            .collect::<Vec<_>>(),
        vec![1..2, 2..3, 3..4, 4..5, 5..7]
    );
    assert!(
        ranges
            .windows(2)
            .all(|pair| pair[0].0.end <= pair[1].0.start)
    );
}
#[test]
fn numeric_controls_reject_values_that_overflow_native_slider_coordinates() {
    let mut value = node("slider", None);
    value.widget = w::Widget::Slider;
    value.config = w::WidgetConfig::Numeric(w::NumberConfig {
        min: 0.,
        max: f64::MAX,
        step: 1.,
    });
    assert!(
        validate(&w::UiTree {
            contribution: "panel".into(),
            nodes: vec![value]
        })
        .is_err()
    );
}
