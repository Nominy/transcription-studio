use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct ReviewEventThrottleInput<'a> {
    pub enabled: bool,
    pub review_action_id: &'a str,
    pub event_type: &'a str,
    pub now_ms: i64,
    pub last_emitted_at_by_key: &'a BTreeMap<String, i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewEventThrottleResult {
    pub should_emit: bool,
    pub key: String,
    pub next_last_emitted_at: Option<i64>,
}

pub fn compute_review_event_throttle(
    input: ReviewEventThrottleInput<'_>,
) -> ReviewEventThrottleResult {
    let key = format!("{}:{}", input.review_action_id, input.event_type);
    let previous = input.last_emitted_at_by_key.get(&key).copied();

    if !input.enabled {
        return ReviewEventThrottleResult {
            should_emit: false,
            key,
            next_last_emitted_at: previous,
        };
    }

    if previous.is_some_and(|previous| input.now_ms - previous < 20_000) {
        return ReviewEventThrottleResult {
            should_emit: false,
            key,
            next_last_emitted_at: previous,
        };
    }

    ReviewEventThrottleResult {
        should_emit: true,
        key,
        next_last_emitted_at: Some(input.now_ms),
    }
}
