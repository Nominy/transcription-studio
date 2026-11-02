mod history;
mod io;
mod segments;

use std::{cell::RefCell, cmp::Ordering};

use studio_core::{CommandHistory, Project};
use time::OffsetDateTime;

#[derive(Clone, Debug)]
pub struct StudioWorkflow {
    project: Project,
    history: CommandHistory,
    pending_boundary_edit_segment_id: RefCell<Option<String>>,
}

impl StudioWorkflow {
    pub fn from_project(project: Project) -> Self {
        Self {
            project,
            history: CommandHistory::default(),
            pending_boundary_edit_segment_id: RefCell::new(None),
        }
    }

    fn next_derived_segment_id(&self, base_segment_id: &str) -> String {
        for index in 1.. {
            let candidate = format!("{base_segment_id}__studio_{index}");
            if !self.project.has_segment(&candidate) {
                return candidate;
            }
        }

        unreachable!("failed to allocate derived segment id")
    }
}

fn compare_seconds(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

fn current_babel_timestamp() -> String {
    let now = OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        now.year(),
        u8::from(now.month()),
        now.day(),
        now.hour(),
        now.minute(),
        now.second(),
        now.nanosecond() / 1_000_000
    )
}
