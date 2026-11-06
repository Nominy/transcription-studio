use std::collections::BTreeMap;

use studio_core::{Project, Segment};

#[derive(Clone, Copy, Debug, Default)]
pub struct RowAvailability {
    pub can_insert_above: bool,
    pub can_insert_below: bool,
    pub can_merge_above: bool,
    pub can_merge_below: bool,
}

pub fn compute_row_availability(project: &Project) -> BTreeMap<String, RowAvailability> {
    let mut by_lane: BTreeMap<&str, Vec<&Segment>> = BTreeMap::new();
    for segment in &project.segments {
        by_lane.entry(&segment.lane_id).or_default().push(segment);
    }

    let mut availability = BTreeMap::new();
    for lane_segments in by_lane.values_mut() {
        lane_segments.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));
        let lane_len = lane_segments.len();
        for (index, segment) in lane_segments.iter().enumerate() {
            availability.insert(
                segment.id.clone(),
                RowAvailability {
                    can_insert_above: true,
                    can_insert_below: true,
                    can_merge_above: index > 0,
                    can_merge_below: index + 1 < lane_len,
                },
            );
        }
    }

    availability
}
