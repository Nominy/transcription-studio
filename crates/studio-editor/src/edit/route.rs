//! Startup route parsing: CLI flags (`--review-action-id X`, `--display-feedback`, ...) and
//! `?query` strings select which task the app opens and how.

/// Project metadata keys the route writes (shared with the Babel service layer).
pub const META_REVIEW_ACTION_ID: &str = "studio.babel.reviewActionId";
pub const META_ACTIVE_QUEUE_ID: &str = "studio.babel.activeQueueId";
pub const META_CHUNKED_QUEUE_ID: &str = "studio.babel.chunkedQueueId";
pub const META_STITCHED_QUEUE_ID: &str = "studio.babel.stitchedQueueId";
pub const META_DISPLAY_FEEDBACK: &str = "studio.babel.displayFeedback";
pub const META_FEEDBACK_INITIAL_OPEN: &str = "studio.babel.feedback.initialOpen";
pub const META_TASK_LOOKUP_OPEN: &str = "studio.babel.taskLookup.open";
pub const META_TASK_LOOKUP_PROCESSED_ID: &str = "studio.babel.taskLookup.processedTranscriptionId";
pub const META_TASK_LOOKUP_REVIEW_ACTION_ID: &str = "studio.babel.taskLookup.reviewActionId";
pub const META_TASK_LOOKUP_SHOW_STITCHED: &str = "studio.babel.taskLookup.showStitchedOption";

/// Parsed startup route.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartupRoute {
    pub review_action_id: Option<String>,
    pub processed_transcription_id: Option<String>,
    pub display_feedback: bool,
    pub show_task_lookup: bool,
    pub chunked_queue_id: Option<String>,
    pub stitched_queue_id: Option<String>,
}

/// What the app should load for a route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteLoad {
    /// Open the task-lookup dialog (no load).
    OpenTaskLookup,
    /// Load a review action by id.
    ReviewAction { review_action_id: String },
    /// Load the latest review action of a processed transcription in a stitched queue.
    LatestReviewAction {
        processed_transcription_id: String,
        queue_id: String,
    },
    /// Claim the next task from a chunked queue.
    ClaimNextFromQueue { queue_id: String },
}

impl StartupRoute {
    /// Parses process arguments. Accepts `--key value`, `--key=value`, bare boolean flags
    /// (`--display-feedback`, `--task-lookup`, `--show-task-lookup[-modal]`) and any argument
    /// containing `?` as a query string (`app://open?reviewActionId=...`). Keys are matched
    /// case-insensitively ignoring `-`/`_`; values are percent-decoded and trimmed.
    pub fn parse_args(args: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let mut route = Self::default();
        let mut pending_key: Option<String> = None;
        for arg in args {
            let arg = arg.as_ref();
            if let Some(key) = pending_key.take() {
                route.set(&key, arg);
                continue;
            }
            if let Some(query_start) = arg.find('?') {
                route.merge_query(&arg[query_start + 1..]);
                continue;
            }
            let Some(flag) = arg.strip_prefix("--") else {
                continue;
            };
            if let Some((key, value)) = flag.split_once('=') {
                route.set(key, value);
            } else if is_bool_flag(flag) {
                route.set(flag, "true");
            } else {
                pending_key = Some(flag.to_string());
            }
        }
        route
    }

    /// Parses a `key=value&key2` query string (leading `?` optional).
    pub fn parse_query(query: &str) -> Self {
        let mut route = Self::default();
        route.merge_query(query.strip_prefix('?').unwrap_or(query));
        route
    }

    /// Whether the route carries anything to apply.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Metadata entries to merge into the project before loading.
    pub fn metadata_entries(&self) -> Vec<(String, String)> {
        let mut entries = Vec::new();
        let mut push = |key: &str, value: &str| entries.push((key.to_string(), value.to_string()));
        if let Some(id) = &self.review_action_id {
            push(META_REVIEW_ACTION_ID, id);
            push(META_TASK_LOOKUP_REVIEW_ACTION_ID, id);
        }
        if let Some(id) = &self.processed_transcription_id {
            push(META_TASK_LOOKUP_PROCESSED_ID, id);
        }
        if let Some(id) = &self.chunked_queue_id {
            push(META_CHUNKED_QUEUE_ID, id);
            push(META_ACTIVE_QUEUE_ID, id);
        }
        if let Some(id) = &self.stitched_queue_id {
            push(META_STITCHED_QUEUE_ID, id);
        }
        if self.display_feedback {
            push(META_DISPLAY_FEEDBACK, "true");
            push(META_FEEDBACK_INITIAL_OPEN, "true");
        }
        if self.show_task_lookup {
            push(META_TASK_LOOKUP_OPEN, "true");
            push(
                META_TASK_LOOKUP_SHOW_STITCHED,
                if self.stitched_queue_id.is_some() {
                    "true"
                } else {
                    "false"
                },
            );
        }
        entries
    }

    /// Load to perform, by precedence: task lookup > review action > processed transcription
    /// (needs a stitched queue) > chunked-queue claim.
    pub fn load(&self) -> Option<RouteLoad> {
        if self.show_task_lookup {
            return Some(RouteLoad::OpenTaskLookup);
        }
        if let Some(review_action_id) = &self.review_action_id {
            return Some(RouteLoad::ReviewAction {
                review_action_id: review_action_id.clone(),
            });
        }
        if let (Some(processed_transcription_id), Some(queue_id)) =
            (&self.processed_transcription_id, &self.stitched_queue_id)
        {
            return Some(RouteLoad::LatestReviewAction {
                processed_transcription_id: processed_transcription_id.clone(),
                queue_id: queue_id.clone(),
            });
        }
        self.chunked_queue_id
            .as_ref()
            .map(|queue_id| RouteLoad::ClaimNextFromQueue {
                queue_id: queue_id.clone(),
            })
    }

    fn merge_query(&mut self, query: &str) {
        for pair in query.split('&').filter(|pair| !pair.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, "true"));
            self.set(key, value);
        }
    }

    fn set(&mut self, key: &str, value: &str) {
        let value = decode_query_component(value).trim().to_string();
        let nonempty = |value: String| (!value.is_empty()).then_some(value);
        match normalize_key(key).as_str() {
            "reviewactionid" => self.review_action_id = nonempty(value),
            "processedtranscriptionid" => self.processed_transcription_id = nonempty(value),
            "displayfeedback" => self.display_feedback = parse_bool(&value),
            "tasklookup" | "showtasklookup" | "showtasklookupmodal" => {
                self.show_task_lookup = parse_bool(&value)
            }
            "chunkedqueueid" => self.chunked_queue_id = nonempty(value),
            "stitchedqueueid" | "stitchedtxqueueid" => self.stitched_queue_id = nonempty(value),
            _ => {}
        }
    }
}

/// Lowercases and strips `-`/`_` (`--Review_Action-Id` → `reviewactionid`).
pub fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| *c != '-' && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_bool_flag(flag: &str) -> bool {
    matches!(
        normalize_key(flag).as_str(),
        "displayfeedback" | "tasklookup" | "showtasklookup" | "showtasklookupmodal"
    )
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "" | "true" | "1" | "yes" | "on"
    )
}

/// Percent/plus decoding; malformed escapes are kept literally; invalid UTF-8 is replaced.
fn decode_query_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(index + 1..index + 3)
                    .filter(|pair| pair.iter().all(u8::is_ascii_hexdigit))
                    .and_then(|pair| std::str::from_utf8(pair).ok())
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok());
                if let Some(byte) = hex {
                    decoded.push(byte);
                    index += 2;
                } else {
                    decoded.push(b'%');
                }
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}
