//! Gold host API v1 payloads (`local://gold-host-contract`, Gold `src/backend/host-api.ts`) and
//! their mapping onto Studio's shared inference types.

use std::collections::HashMap;

use base64::Engine as _;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use studio_inference::{
    BackendStatus, DraftResponse, InferenceError, InferenceErrorKind, InferenceProgress, LaneAudio,
    OperationAvailability, PreservedRow, TimingResponse, TimingToken,
};

use crate::rpc::PageError;

/// Maps a host-page error onto the shared error kinds (contract "Rust mapping").
pub fn page_error(error: PageError) -> InferenceError {
    let kind = match error.code.as_deref() {
        Some("needs-models" | "needs-test") => InferenceErrorKind::NeedsModels,
        Some("no-webgpu") => InferenceErrorKind::NoWebGpu,
        Some("cancelled") => InferenceErrorKind::Cancelled,
        _ => InferenceErrorKind::Other,
    };
    InferenceError::new(kind, error.message)
}

/// The timing lane name of a lane: its speaker key, or its label when the key is blank.
pub fn lane_name(lane: &LaneAudio) -> &str {
    if lane.speaker_key.trim().is_empty() {
        &lane.label
    } else {
        &lane.speaker_key
    }
}

/// A lane's audio as the page fetches it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioRef {
    pub field_name: String,
    pub url: String,
}

fn audio_map(audio: &[AudioRef]) -> Value {
    Value::Object(
        audio
            .iter()
            .map(|audio| (audio.field_name.clone(), json!({ "url": audio.url })))
            .collect(),
    )
}

fn timing_lanes(lanes: &[LaneAudio], audio: &[AudioRef]) -> Value {
    lanes
        .iter()
        .zip(audio)
        .map(|(lane, audio)| json!({ "lane": lane_name(lane), "fieldName": audio.field_name }))
        .collect()
}

/// `timing(params, audio, emit)` arguments.
pub fn timing_args(
    request_id: &str,
    task_id: &str,
    lanes: &[LaneAudio],
    audio: &[AudioRef],
) -> Vec<Value> {
    vec![
        json!({ "requestId": request_id, "taskId": task_id, "lanes": timing_lanes(lanes, audio) }),
        audio_map(audio),
    ]
}

/// `draft(params, audio, emit)` arguments.
pub fn draft_args(
    request_id: &str,
    task_id: &str,
    timing: &TimingResponse,
    preserve_rows: Option<&[PreservedRow]>,
    lanes: &[LaneAudio],
    audio: &[AudioRef],
) -> Vec<Value> {
    let mut params = json!({
        "requestId": request_id,
        "taskId": task_id,
        "timing": timing,
        "lanes": timing_lanes(lanes, audio),
    });
    if let Some(rows) = preserve_rows {
        params["preserveRows"] = rows
            .iter()
            .map(|row| {
                json!({
                    "rowId": row.row_id,
                    "speakerKey": row.speaker_key,
                    "startSeconds": row.start_seconds,
                    "endSeconds": row.end_seconds,
                })
            })
            .collect();
    }
    vec![params, audio_map(audio)]
}

/// `enhance(params, audio, emit)` arguments.
pub fn enhance_args(
    request_id: &str,
    task_id: &str,
    lanes: &[LaneAudio],
    audio: &[AudioRef],
) -> Vec<Value> {
    let lane_params: Value = lanes
        .iter()
        .zip(audio)
        .map(|(lane, audio)| {
            json!({
                "laneId": lane.lane_id,
                "speakerKey": lane.speaker_key,
                "label": lane.label,
                "fieldName": audio.field_name,
            })
        })
        .collect();
    vec![
        json!({ "requestId": request_id, "taskId": task_id, "lanes": lane_params }),
        audio_map(audio),
    ]
}

/// `align(params, emit)` arguments.
pub fn align_args(request_id: &str, language: &str, audio: &AudioRef) -> Vec<Value> {
    vec![json!({ "requestId": request_id, "language": language, "audio": { "url": audio.url } })]
}

/// Parameters of methods that only take a request id (`installModels`, `testSample`, ...).
pub fn request_args(request_id: &str) -> Vec<Value> {
    vec![json!({ "requestId": request_id })]
}

/// `HostStatus` from `status()`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub host_api_version: u64,
    pub release: String,
    pub model_base_url: String,
    pub models: ModelsStatus,
    pub download_bytes: Option<u64>,
    pub webgpu: WebGpuStatus,
    pub enhancement_model: EnhancementModel,
    pub ready: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ModelsState {
    NotInstalled,
    Downloading,
    Ready,
    Error,
    Stopped,
}

/// `HostStatus.models`, also the result of `installModels` / `removeModels`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsStatus {
    pub state: ModelsState,
    pub completed_bytes: u64,
    pub total_bytes: u64,
    #[serde(default)]
    pub current_path: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    pub tested: bool,
    #[serde(default)]
    pub installed_release_id: Option<String>,
    pub outdated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebGpuStatus {
    pub available: bool,
    pub shader_f16: bool,
    #[serde(default)]
    pub adapter: Option<GpuAdapter>,
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GpuAdapter {
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnhancementModel {
    pub id: String,
    pub sha256: String,
}

/// The shared readiness view of a host status.
pub fn backend_status(status: &HostStatus) -> BackendStatus {
    if !status.webgpu.available || !status.webgpu.shader_f16 {
        let message = match &status.webgpu.reason {
            Some(reason) if !reason.trim().is_empty() => reason.clone(),
            _ if status.webgpu.available => {
                "This computer's graphics card lacks WebGPU shader-f16, which local AI needs."
                    .to_string()
            }
            _ => "WebGPU is unavailable on this computer.".to_string(),
        };
        return BackendStatus::Unavailable {
            kind: InferenceErrorKind::NoWebGpu,
            message,
        };
    }
    if status.ready {
        return BackendStatus::Ready {
            release: status.release.clone(),
            operations: OperationAvailability {
                timing: true,
                draft: true,
                enhance: true,
                align: true,
            },
        };
    }
    BackendStatus::NeedsModels {
        release: status.release.clone(),
        download_bytes: status.download_bytes.unwrap_or(status.models.total_bytes),
        outdated: status.models.outdated,
    }
}

/// `testSample` result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleResult {
    pub text: String,
    pub duration_seconds: f64,
    pub tokens: Vec<SampleToken>,
    pub tested: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SampleToken {
    pub text: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// `align` result.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlignResult {
    pub words: Vec<SampleToken>,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    pub receipt: AlignReceipt,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct AlignReceipt {
    pub model: String,
}

impl AlignResult {
    pub fn tokens(&self) -> Vec<TimingToken> {
        self.words
            .iter()
            .enumerate()
            .map(|(index, word)| TimingToken {
                id: format!("w{index}"),
                text: word.text.clone(),
                start_seconds: word.start_seconds,
                end_seconds: word.end_seconds,
            })
            .collect()
    }
}

/// `enhance` result.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhanceResult {
    pub model: String,
    pub lanes: Vec<EnhancedLaneMetadata>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedLaneMetadata {
    pub lane_id: String,
    pub total_bytes: u64,
}

/// Decodes a method result into `T`.
pub fn decode<T: for<'de> Deserialize<'de>>(
    method: &str,
    value: Value,
) -> Result<T, InferenceError> {
    serde_json::from_value(value).map_err(|error| {
        InferenceError::other(format!(
            "The AI engine returned an invalid {method} result. ({error})"
        ))
    })
}

pub fn decode_timing(value: Value) -> Result<TimingResponse, InferenceError> {
    decode("timing", value)
}

pub fn decode_draft(value: Value) -> Result<DraftResponse, InferenceError> {
    decode("draft", value)
}

/// One event the page emitted, interpreted.
#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Progress(InferenceProgress),
    Audio {
        lane_id: String,
        offset: u64,
        total_bytes: u64,
        data: Vec<u8>,
    },
}

/// Interprets one emitted event; `Ok(None)` for event kinds Studio does not use.
pub fn decode_event(event: &Value) -> Result<Option<HostEvent>, String> {
    if let Some(progress) = event.get("progress") {
        let field = |name: &str| progress.get(name).and_then(Value::as_u64);
        let phase = progress
            .get("phase")
            .and_then(Value::as_str)
            .ok_or("progress event without phase")?;
        let detail = progress
            .get("lane")
            .and_then(Value::as_str)
            .map(str::to_string);
        return Ok(Some(HostEvent::Progress(InferenceProgress::Stage {
            stage: phase.to_string(),
            completed: field("completed").unwrap_or(0),
            total: field("total"),
            detail,
            clip: field("clip"),
        })));
    }
    if let Some(download) = event.get("download") {
        let field = |name: &str| download.get(name).and_then(Value::as_u64);
        return Ok(Some(HostEvent::Progress(InferenceProgress::Download {
            bytes: field("completedBytes").ok_or("download event without completedBytes")?,
            total: field("totalBytes").ok_or("download event without totalBytes")?,
        })));
    }
    if let Some(enhancement) = event.get("enhancement") {
        let field = |name: &str| enhancement.get(name).and_then(Value::as_u64);
        let phase = enhancement
            .get("phase")
            .and_then(Value::as_str)
            .ok_or("enhancement event without phase")?;
        return Ok(Some(HostEvent::Progress(InferenceProgress::Stage {
            stage: phase.to_string(),
            completed: field("completedChunks").unwrap_or(0),
            total: field("totalChunks"),
            detail: enhancement
                .get("laneId")
                .and_then(Value::as_str)
                .map(str::to_string),
            clip: None,
        })));
    }
    if let Some(audio) = event.get("audio") {
        let lane_id = audio
            .get("laneId")
            .and_then(Value::as_str)
            .ok_or("audio event without laneId")?;
        let offset = audio
            .get("offset")
            .and_then(Value::as_u64)
            .ok_or("audio event without offset")?;
        let total_bytes = audio
            .get("totalBytes")
            .and_then(Value::as_u64)
            .ok_or("audio event without totalBytes")?;
        let data = audio
            .get("data")
            .and_then(Value::as_str)
            .ok_or("audio event without data")?;
        let data = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| format!("audio event data is not base64: {error}"))?;
        return Ok(Some(HostEvent::Audio {
            lane_id: lane_id.to_string(),
            offset,
            total_bytes,
            data,
        }));
    }
    if let Some(stage) = event.get("stage").and_then(Value::as_str) {
        return Ok(Some(HostEvent::Progress(InferenceProgress::Stage {
            stage: stage.to_string(),
            completed: 0,
            total: None,
            detail: None,
            clip: None,
        })));
    }
    Ok(None)
}

/// Reassembles streamed enhancement WAVs; chunks must arrive in order per lane.
#[derive(Debug, Default)]
pub struct AudioAssembler {
    lanes: HashMap<String, (u64, Vec<u8>)>,
    error: Option<String>,
}

impl AudioAssembler {
    pub fn push(&mut self, lane_id: &str, offset: u64, total_bytes: u64, data: &[u8]) {
        if self.error.is_some() {
            return;
        }
        let (total, bytes) = self
            .lanes
            .entry(lane_id.to_string())
            .or_insert_with(|| (total_bytes, Vec::with_capacity(total_bytes as usize)));
        if *total != total_bytes || bytes.len() as u64 != offset {
            self.error = Some(format!(
                "audio for lane {lane_id} arrived out of order (offset {offset}, have {} of {total} bytes)",
                bytes.len()
            ));
            return;
        }
        if offset + data.len() as u64 > total_bytes {
            self.error = Some(format!(
                "audio for lane {lane_id} exceeds {total_bytes} bytes"
            ));
            return;
        }
        bytes.extend_from_slice(data);
    }

    pub fn record_error(&mut self, error: String) {
        self.error.get_or_insert(error);
    }

    /// Takes lane `lane_id`'s WAV, which must be exactly `total_bytes` long.
    pub fn take(&mut self, lane_id: &str, total_bytes: u64) -> Result<Bytes, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        let (_, bytes) = self
            .lanes
            .remove(lane_id)
            .ok_or_else(|| format!("no audio arrived for lane {lane_id}"))?;
        if bytes.len() as u64 != total_bytes {
            return Err(format!(
                "audio for lane {lane_id} is {} of {total_bytes} bytes",
                bytes.len()
            ));
        }
        Ok(Bytes::from(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(lane_id: &str, speaker_key: &str, label: &str) -> LaneAudio {
        LaneAudio {
            lane_id: lane_id.to_string(),
            speaker_key: speaker_key.to_string(),
            label: label.to_string(),
            wav: Bytes::from_static(b"RIFF"),
        }
    }

    fn audio(field: &str) -> AudioRef {
        AudioRef {
            field_name: field.to_string(),
            url: format!("https://studio-audio.example/{field}.wav"),
        }
    }

    fn host_status(webgpu: Value, models: Value, ready: bool) -> HostStatus {
        serde_json::from_value(json!({
            "hostApiVersion": 1, "release": "rel-9", "modelBaseUrl": "https://m/",
            "models": models, "downloadBytes": 2_600_000_000u64, "webgpu": webgpu,
            "enhancementModel": { "id": "zip-1", "sha256": "ab" }, "ready": ready
        }))
        .unwrap()
    }

    fn models(state: &str, tested: bool, outdated: bool) -> Value {
        json!({ "state": state, "completedBytes": 0, "totalBytes": 10, "tested": tested, "outdated": outdated })
    }

    #[test]
    fn error_codes_map_to_kinds() {
        let kind = |code: Option<&str>| {
            page_error(PageError {
                code: code.map(str::to_string),
                message: "m".to_string(),
            })
            .kind
        };
        assert_eq!(kind(Some("needs-models")), InferenceErrorKind::NeedsModels);
        assert_eq!(kind(Some("needs-test")), InferenceErrorKind::NeedsModels);
        assert_eq!(kind(Some("no-webgpu")), InferenceErrorKind::NoWebGpu);
        assert_eq!(kind(Some("cancelled")), InferenceErrorKind::Cancelled);
        assert_eq!(kind(Some("audio-unavailable")), InferenceErrorKind::Other);
        assert_eq!(kind(None), InferenceErrorKind::Other);
    }

    #[test]
    fn status_requires_shader_f16() {
        let status = host_status(
            json!({ "available": true, "shaderF16": false }),
            models("ready", true, false),
            false,
        );
        assert!(matches!(
            backend_status(&status),
            BackendStatus::Unavailable {
                kind: InferenceErrorKind::NoWebGpu,
                ..
            }
        ));
        let status = host_status(
            json!({ "available": false, "shaderF16": false, "reason": "No adapter." }),
            models("ready", true, false),
            false,
        );
        assert_eq!(
            backend_status(&status),
            BackendStatus::Unavailable {
                kind: InferenceErrorKind::NoWebGpu,
                message: "No adapter.".to_string()
            }
        );
    }

    #[test]
    fn status_ready_and_needs_models() {
        let gpu = json!({ "available": true, "shaderF16": true });
        let ready = host_status(gpu.clone(), models("ready", true, false), true);
        assert!(
            matches!(backend_status(&ready), BackendStatus::Ready { release, .. } if release == "rel-9")
        );
        let outdated = host_status(gpu.clone(), models("not-installed", false, true), false);
        assert_eq!(
            backend_status(&outdated),
            BackendStatus::NeedsModels {
                release: "rel-9".to_string(),
                download_bytes: 2_600_000_000,
                outdated: true
            }
        );
        let untested = host_status(gpu, models("ready", false, false), false);
        assert!(matches!(
            backend_status(&untested),
            BackendStatus::NeedsModels { .. }
        ));
    }

    #[test]
    fn timing_lanes_fall_back_to_label() {
        let lanes = [lane("t1", "speaker_1", "A"), lane("t2", " ", "Guest")];
        let args = timing_args("r1", "task", &lanes, &[audio("lane0"), audio("lane1")]);
        assert_eq!(
            args[0],
            json!({ "requestId": "r1", "taskId": "task", "lanes": [
                { "lane": "speaker_1", "fieldName": "lane0" },
                { "lane": "Guest", "fieldName": "lane1" }
            ] })
        );
        assert_eq!(
            args[1],
            json!({
                "lane0": { "url": "https://studio-audio.example/lane0.wav" },
                "lane1": { "url": "https://studio-audio.example/lane1.wav" }
            })
        );
    }

    #[test]
    fn draft_sends_only_contract_fields_of_preserved_rows() {
        let timing = TimingResponse {
            task_id: "task".to_string(),
            tracks: Vec::new(),
            summary: json!({}),
            models: json!({}),
        };
        let row = PreservedRow {
            row_id: "row-1".to_string(),
            speaker_key: "speaker_1".to_string(),
            start_seconds: 1.5,
            end_seconds: 2.5,
            text: "old".to_string(),
            index: 4,
        };
        let lanes = [lane("t1", "speaker_1", "A")];
        let args = draft_args(
            "r2",
            "task",
            &timing,
            Some(&[row]),
            &lanes,
            &[audio("lane0")],
        );
        assert_eq!(
            args[0]["preserveRows"],
            json!([{ "rowId": "row-1", "speakerKey": "speaker_1", "startSeconds": 1.5, "endSeconds": 2.5 }])
        );
        assert_eq!(args[0]["timing"]["taskId"], "task");
        let without = draft_args("r3", "task", &timing, None, &lanes, &[audio("lane0")]);
        assert!(without[0].get("preserveRows").is_none());
    }

    #[test]
    fn events_map_to_progress() {
        let event = json!({ "progress": { "phase": "row-llm", "completed": 3, "total": 9, "lane": "speaker_1", "clip": 2 } });
        assert_eq!(
            decode_event(&event),
            Ok(Some(HostEvent::Progress(InferenceProgress::Stage {
                stage: "row-llm".to_string(),
                completed: 3,
                total: Some(9),
                detail: Some("speaker_1".to_string()),
                clip: Some(2)
            })))
        );
        let event = json!({ "download": { "phase": "downloading", "completedBytes": 5, "totalBytes": 50, "fileIndex": 0, "fileCount": 3 } });
        assert_eq!(
            decode_event(&event),
            Ok(Some(HostEvent::Progress(InferenceProgress::Download {
                bytes: 5,
                total: 50
            })))
        );
        let event = json!({ "stage": "adapting" });
        assert!(matches!(
            decode_event(&event),
            Ok(Some(HostEvent::Progress(InferenceProgress::Stage { stage, total: None, .. }))) if stage == "adapting"
        ));
        assert_eq!(decode_event(&json!({ "other": 1 })), Ok(None));
        assert!(decode_event(&json!({ "download": { "completedBytes": 1 } })).is_err());
    }

    #[test]
    fn audio_chunks_reassemble_in_order() {
        let first = base64::engine::general_purpose::STANDARD.encode(b"RIFF");
        let event =
            json!({ "audio": { "laneId": "t1", "offset": 0, "totalBytes": 6, "data": first } });
        let Ok(Some(HostEvent::Audio {
            lane_id,
            offset,
            total_bytes,
            data,
        })) = decode_event(&event)
        else {
            panic!("expected audio");
        };
        let mut assembler = AudioAssembler::default();
        assembler.push(&lane_id, offset, total_bytes, &data);
        assembler.push("t1", 4, 6, b"ab");
        assert_eq!(
            assembler.take("t1", 6).unwrap(),
            Bytes::from_static(b"RIFFab")
        );
    }

    #[test]
    fn audio_gaps_and_short_lanes_fail() {
        let mut assembler = AudioAssembler::default();
        assembler.push("t1", 0, 6, b"RIFF");
        assert!(assembler.take("t1", 6).unwrap_err().contains("4 of 6"));

        let mut assembler = AudioAssembler::default();
        assembler.push("t1", 0, 6, b"RI");
        assembler.push("t1", 4, 6, b"ab");
        assert!(
            assembler
                .take("t1", 6)
                .unwrap_err()
                .contains("out of order")
        );

        let mut assembler = AudioAssembler::default();
        assert!(assembler.take("t1", 6).unwrap_err().contains("no audio"));
    }

    #[test]
    fn align_words_become_tokens() {
        let result: AlignResult = decode(
            "align",
            json!({ "words": [{ "text": "да", "startSeconds": 0.1, "endSeconds": 0.3 }],
                    "durationSeconds": 2.0, "receipt": { "provider": "local", "model": "rel-9" } }),
        )
        .unwrap();
        assert_eq!(result.tokens()[0].id, "w0");
        assert_eq!(result.duration_seconds, Some(2.0));
        assert_eq!(result.receipt.model, "rel-9");
    }
}
