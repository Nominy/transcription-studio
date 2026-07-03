use studio_core::{Project, Segment};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeRange {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

impl TimeRange {
    pub fn for_project(project: &Project) -> Self {
        let max_end = project
            .segments
            .iter()
            .map(|segment| segment.end_seconds)
            .filter(|seconds| seconds.is_finite())
            .fold(0.0, f64::max);
        Self {
            start_seconds: 0.0,
            end_seconds: max_end,
        }
    }

    pub fn duration_seconds(self) -> f64 {
        (self.end_seconds - self.start_seconds).max(0.001)
    }

    pub(super) fn intersects(self, segment: &Segment) -> bool {
        segment.end_seconds > self.start_seconds && segment.start_seconds < self.end_seconds
    }
}
