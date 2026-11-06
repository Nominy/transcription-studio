use studio_core::Project;

const META_SHOW_REVIEWERS: &str = "studio.babel.diff.showReviewers";
const META_REVIEWER_INTERVALS: &str = "studio.babel.diff.reviewerIntervals";

use studio_editor::transcript_model::TranscriptReviewerView;

#[derive(Clone, Debug, Default)]
pub(super) struct ReviewerLookup {
    pub show_reviewer: bool,
    intervals: Vec<ReviewerInterval>,
}

impl ReviewerLookup {
    pub fn for_project(project: &Project) -> Self {
        let show_reviewer = metadata_bool(project, META_SHOW_REVIEWERS);
        let mut intervals = parse_reviewer_intervals(project);
        intervals.sort_by(|left, right| left.start_seconds.total_cmp(&right.start_seconds));

        Self {
            show_reviewer,
            intervals,
        }
    }

    pub fn reviewer_at(&self, time_seconds: f64) -> Option<TranscriptReviewerView> {
        if !self.show_reviewer {
            return None;
        }

        self.intervals
            .iter()
            .find(|interval| {
                time_seconds >= interval.start_seconds && time_seconds < interval.end_seconds
            })
            .map(|interval| interval.reviewer.view())
    }
}

#[derive(Clone, Debug)]
struct ReviewerInterval {
    start_seconds: f64,
    end_seconds: f64,
    reviewer: ReviewerIdentity,
}

#[derive(Clone, Debug)]
struct ReviewerIdentity {
    worker_id: Option<String>,
    level: String,
    username: Option<String>,
}

impl ReviewerIdentity {
    fn view(&self) -> TranscriptReviewerView {
        TranscriptReviewerView {
            label: format!(
                "L{}: {}",
                self.level,
                self.username.as_deref().unwrap_or("\u{2014}")
            ),
            color_hue: self.worker_id.as_deref().map(reviewer_hue),
        }
    }
}

fn metadata_bool(project: &Project, key: &str) -> bool {
    project
        .metadata
        .get(key)
        .is_some_and(|value| value == "true")
}

fn parse_reviewer_intervals(project: &Project) -> Vec<ReviewerInterval> {
    let Some(raw) = project.metadata.get(META_REVIEWER_INTERVALS) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    let Some(intervals) = value.as_array() else {
        return Vec::new();
    };

    intervals
        .iter()
        .filter_map(parse_reviewer_interval)
        .collect()
}

fn parse_reviewer_interval(value: &serde_json::Value) -> Option<ReviewerInterval> {
    let start_seconds = number_field(value, &["startOffsetInSeconds", "start_seconds"])?;
    let end_seconds = number_field(value, &["endOffsetInSeconds", "end_seconds"])?;
    let reviewer_value = value
        .get("lastReviewer")
        .or_else(|| value.get("last_reviewer"))?;
    let level =
        display_field(reviewer_value, &["level"]).unwrap_or_else(|| "undefined".to_string());

    Some(ReviewerInterval {
        start_seconds,
        end_seconds,
        reviewer: ReviewerIdentity {
            worker_id: string_field(reviewer_value, &["workerId", "worker_id"]),
            level,
            username: string_field(
                reviewer_value,
                &["discourseUsername", "discourse_username", "username"],
            ),
        },
    })
}

fn number_field(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_f64))
}

fn string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    })
}

fn display_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        if let Some(text) = field.as_str() {
            return Some(text.to_string());
        }
        if let Some(number) = field.as_i64() {
            return Some(number.to_string());
        }
        field.as_u64().map(|number| number.to_string())
    })
}

fn reviewer_hue(worker_id: &str) -> u16 {
    let mut hash: i32 = 0;
    for unit in worker_id.encode_utf16() {
        hash = hash
            .wrapping_shl(5)
            .wrapping_sub(hash)
            .wrapping_add(i32::from(unit));
    }
    (i64::from(hash).abs() % 360) as u16
}

#[cfg(test)]
mod tests {
    #[test]
    fn reviewer_hue_is_stable_per_worker() {
        assert_eq!(super::reviewer_hue("worker-alpha"), 49);
        assert_eq!(super::reviewer_hue("worker-beta"), 287);
    }
}
