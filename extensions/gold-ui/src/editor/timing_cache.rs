//! Gold word timings of a task: the backend's `TimingResponse` (reused by later drafts and
//! segment transcriptions), its conversion into the editor's per-lane word clocks, and a small
//! per-task file cache so timings survive reopening the task.
//!
//! A cached response is only reused for exactly the lanes it was computed from, identified by
//! the SHA-256 of each lane's Original WAV bytes.

use std::path::PathBuf;

use exaudio_gold::protocol::lane_name;
use serde::{Deserialize, Serialize};
use studio_audio::sha256_hex;
use studio_core::timing::{LaneTimings, TimingToken as WordToken};
use studio_inference::{LaneAudio, TimingResponse};
use studio_services::paths::app_support_dir;

/// Folder under the app support dir holding one file per task.
const CACHE_DIR_NAME: &str = "word-timings";

/// One lane a timing run used: its project lane id and the digest of its Original WAV.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimedLane {
    pub lane_id: String,
    pub wav_sha256: String,
}

/// The lanes of `lanes` in order, keyed by their WAV digests.
pub fn timed_lanes(lanes: &[LaneAudio]) -> Vec<TimedLane> {
    lanes
        .iter()
        .map(|lane| TimedLane {
            lane_id: lane.lane_id.clone(),
            wav_sha256: sha256_hex(&lane.wav),
        })
        .collect()
}

/// A completed timing run and the exact lanes it timed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedTiming {
    pub lanes: Vec<TimedLane>,
    pub timing: TimingResponse,
}

impl SavedTiming {
    /// Whether this run timed exactly `lanes` (same lanes, same audio, same order).
    pub fn matches(&self, lanes: &[TimedLane]) -> bool {
        self.lanes == lanes
    }
}

/// Editor word clocks of every timed lane. Tracks are named by the lane's timing name
/// (speaker key, else label); tracks of unknown lanes are ignored.
pub fn lane_timings(timing: &TimingResponse, lanes: &[LaneAudio]) -> Vec<LaneTimings> {
    timing
        .tracks
        .iter()
        .filter_map(|track| {
            let lane = lanes.iter().find(|lane| lane_name(lane) == track.lane)?;
            Some(LaneTimings {
                lane_id: lane.lane_id.clone(),
                source_id: track.pcm_sha256.clone(),
                tokens: track
                    .tokens
                    .iter()
                    .map(|token| WordToken {
                        text: token.text.clone(),
                        start_seconds: token.start_seconds,
                        end_seconds: token.end_seconds,
                    })
                    .collect(),
            })
        })
        .collect()
}

/// Per-task cache files under `dir`.
#[derive(Debug, Clone)]
pub struct TimingCache {
    dir: PathBuf,
}

impl TimingCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn for_app() -> Self {
        Self::new(app_support_dir().join(CACHE_DIR_NAME))
    }

    fn path(&self, task_id: &str) -> PathBuf {
        let name: String = task_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join(format!("{name}.json"))
    }

    /// The task's saved run; a missing or unreadable file is `None`.
    pub fn load(&self, task_id: &str) -> Option<SavedTiming> {
        let source = std::fs::read(self.path(task_id)).ok()?;
        serde_json::from_slice(&source).ok()
    }

    /// Replaces the task's saved run (written to a temp file, then renamed).
    pub fn store(&self, task_id: &str, saved: &SavedTiming) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.path(task_id);
        let temp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(saved).map_err(std::io::Error::other)?;
        std::fs::write(&temp, bytes)?;
        std::fs::rename(&temp, &path)
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use serde_json::json;
    use studio_inference::{TimingToken, TimingTrack};

    use super::*;

    fn lane(id: &str, speaker: &str, wav: &'static [u8]) -> LaneAudio {
        LaneAudio {
            lane_id: id.to_string(),
            speaker_key: speaker.to_string(),
            label: format!("Track {id}"),
            wav: Bytes::from_static(wav),
        }
    }

    fn response(task_id: &str) -> TimingResponse {
        TimingResponse {
            task_id: task_id.to_string(),
            tracks: vec![TimingTrack {
                lane: "spk-b".to_string(),
                pcm_sha256: "pcm".to_string(),
                sample_rate: 16_000,
                tokens: vec![TimingToken {
                    id: "t0".to_string(),
                    text: "привет".to_string(),
                    start_seconds: 0.5,
                    end_seconds: 0.9,
                }],
                segments: Vec::new(),
                e2e: Some(json!({ "opaque": true })),
            }],
            summary: json!({}),
            models: json!({}),
        }
    }

    #[test]
    fn tracks_map_to_lanes_by_timing_name() {
        let lanes = [lane("a", "spk-a", b"a"), lane("b", "spk-b", b"b")];
        let timings = lane_timings(&response("task"), &lanes);
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].lane_id, "b");
        assert_eq!(timings[0].tokens[0].text, "привет");
    }

    #[test]
    fn cache_round_trips_and_only_matches_the_same_audio() {
        let dir = std::env::temp_dir().join(format!("studio-timing-cache-{}", std::process::id()));
        let cache = TimingCache::new(dir.clone());
        let lanes = [lane("a", "spk-a", b"a"), lane("b", "spk-b", b"b")];
        let saved = SavedTiming {
            lanes: timed_lanes(&lanes),
            timing: response("task/1"),
        };
        cache.store("task/1", &saved).unwrap();
        let loaded = cache.load("task/1").unwrap();
        assert_eq!(loaded, saved);
        assert!(loaded.matches(&timed_lanes(&lanes)));
        let changed = [lane("a", "spk-a", b"a"), lane("b", "spk-b", b"other")];
        assert!(!loaded.matches(&timed_lanes(&changed)));
        assert!(cache.load("task/2").is_none());
        std::fs::remove_dir_all(dir).ok();
    }
}
