use super::formatting::{format_seconds_for_edit, parse_seconds};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TimeField {
    Start,
    End,
}

impl TimeField {
    pub(super) fn value_from(self, start_seconds: f64, end_seconds: f64) -> f64 {
        match self {
            Self::Start => start_seconds,
            Self::End => end_seconds,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::End => "end",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct TimeEditState {
    pub(super) active_field: Option<TimeField>,
    pub(super) draft_value: String,
    pub(super) error: Option<String>,
    pub(super) should_focus_and_select: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct TimeRangeEdit {
    pub(super) start_seconds: f64,
    pub(super) end_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum TimeValidationResult {
    Valid(TimeRangeEdit),
    Invalid(String),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum CommitTimeCellEditResult {
    Ignored,
    Handled {
        active_field: Option<TimeField>,
        draft_value: String,
        error: Option<String>,
        should_call_on_time_change: bool,
        annotation_id: String,
        next_times: Option<TimeRangeEdit>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct CommitTimeCellEditInput {
    pub(super) active_field: Option<TimeField>,
    pub(super) field: TimeField,
    pub(super) draft_value: String,
    pub(super) value: f64,
    pub(super) start_seconds: f64,
    pub(super) end_seconds: f64,
    pub(super) annotation_id: String,
    pub(super) from_blur: bool,
}

pub(super) fn start_time_cell_edit(field: TimeField, value: f64) -> TimeEditState {
    TimeEditState {
        active_field: Some(field),
        draft_value: format_recovered_time(value),
        error: None,
        should_focus_and_select: true,
    }
}

pub(super) fn commit_time_cell_edit(input: CommitTimeCellEditInput) -> CommitTimeCellEditResult {
    if input.active_field != Some(input.field) {
        return CommitTimeCellEditResult::Ignored;
    }

    match validate_time_edit(
        input.field,
        &input.draft_value,
        TimeRangeEdit {
            start_seconds: input.start_seconds,
            end_seconds: input.end_seconds,
        },
    ) {
        TimeValidationResult::Invalid(message) => CommitTimeCellEditResult::Handled {
            active_field: if input.from_blur {
                None
            } else {
                Some(input.field)
            },
            draft_value: if input.from_blur {
                format_recovered_time(input.value)
            } else {
                input.draft_value
            },
            error: if input.from_blur { None } else { Some(message) },
            should_call_on_time_change: false,
            annotation_id: input.annotation_id,
            next_times: None,
        },
        TimeValidationResult::Valid(next_times) => CommitTimeCellEditResult::Handled {
            active_field: None,
            draft_value: input.draft_value,
            error: None,
            should_call_on_time_change: true,
            annotation_id: input.annotation_id,
            next_times: Some(next_times),
        },
    }
}

pub(super) fn validate_time_edit(
    field: TimeField,
    value: &str,
    range: TimeRangeEdit,
) -> TimeValidationResult {
    let parsed = match parse_recovered_time(value) {
        Some(value) => value,
        None => return TimeValidationResult::Invalid("Invalid format".to_string()),
    };

    let start_seconds = if field == TimeField::Start {
        parsed
    } else {
        range.start_seconds
    };
    let end_seconds = if field == TimeField::End {
        parsed
    } else {
        range.end_seconds
    };

    if start_seconds < 0.0 || end_seconds < 0.0 {
        return TimeValidationResult::Invalid("Timestamp must be >= 00:00.00".to_string());
    }

    if start_seconds >= end_seconds {
        return TimeValidationResult::Invalid(match field {
            TimeField::Start => "Start must be earlier than end".to_string(),
            TimeField::End => "End must be later than start".to_string(),
        });
    }

    TimeValidationResult::Valid(TimeRangeEdit {
        start_seconds,
        end_seconds,
    })
}

pub(super) fn format_recovered_time(value: f64) -> String {
    if !value.is_finite() {
        return "--:--.--".to_string();
    }

    format_seconds_for_edit(value)
}

fn parse_recovered_time(value: &str) -> Option<f64> {
    parse_seconds("Time", value).ok()
}
