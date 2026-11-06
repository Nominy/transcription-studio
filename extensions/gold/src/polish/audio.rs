//! Drafting audio cues: lane audio encoded as PCM16 WAV multipart tracks, plus the
//! "audio capture needs review" guard shown before a generation with missing lanes.

use anyhow::{Result, bail};
use serde_json::json;
use studio_core::{AudioLane, Project};
use studio_waveform::DecodedAudio;

use super::model::AudioTrack;

/// How many lanes have decoded audio loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAudioCueStatus {
    pub loaded_tracks: usize,
    pub total_lanes: usize,
    /// Lane label (or id when blank) for each lane without audio.
    pub missing_lane_labels: Vec<String>,
    pub missing_lane_sources: Vec<String>,
}

/// Blocking prompt before generating with incomplete lane audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftAudioGuard {
    pub expected_speaker_lanes: usize,
    pub captured_speaker_lanes: usize,
    pub captured_tracks: usize,
    /// "Use available audio" is offered only when at least one lane is loaded.
    pub can_use_partial_audio: bool,
    pub message: String,
}

/// How a row generation treats lane audio (the guard's three choices).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftAudioMode {
    /// Show [`DraftAudioGuard`] when some lanes lack audio.
    Guard,
    /// Send whatever lanes are loaded (requires at least one).
    UseAvailableAudio,
    /// Send no audio.
    TextOnly,
}

pub fn draft_audio_cue_status<'a>(
    project: &'a Project,
    decoded_audio: impl Fn(&str) -> Option<&'a DecodedAudio>,
) -> DraftAudioCueStatus {
    let mut status = DraftAudioCueStatus {
        loaded_tracks: 0,
        total_lanes: project.lanes.len(),
        missing_lane_labels: Vec::new(),
        missing_lane_sources: Vec::new(),
    };
    for lane in &project.lanes {
        if decoded_audio(&lane.id).is_some() {
            status.loaded_tracks += 1;
        } else {
            status.missing_lane_labels.push(lane_label(lane));
            status.missing_lane_sources.push(lane.source_path.clone());
        }
    }
    status
}

/// The guard to show for `status` and the tracks that would be sent, or `None` when every lane
/// is loaded (or the project has no lanes).
pub fn draft_audio_guard(
    status: &DraftAudioCueStatus,
    captured_tracks: usize,
) -> Option<DraftAudioGuard> {
    if status.total_lanes == 0 || status.loaded_tracks >= status.total_lanes {
        return None;
    }
    let problem = if status.loaded_tracks == 0 {
        format!(
            "No speaker audio is loaded for {} speaker tracks.",
            status.total_lanes
        )
    } else {
        format!(
            "Only {} of {} speaker tracks have audio loaded.",
            status.loaded_tracks, status.total_lanes
        )
    };
    Some(DraftAudioGuard {
        expected_speaker_lanes: status.total_lanes,
        captured_speaker_lanes: status.loaded_tracks,
        captured_tracks,
        can_use_partial_audio: status.loaded_tracks > 0,
        message: format!("{problem} The draft may miss laughter or other sounds if it starts now."),
    })
}

/// Tracks to send for a row generation in `mode`, or the guard to show instead: `Guard` stops
/// while lanes are missing, `UseAvailableAudio` needs at least one track, `TextOnly` sends none.
pub fn resolve_draft_audio(
    mode: DraftAudioMode,
    status: &DraftAudioCueStatus,
    tracks: Vec<AudioTrack>,
) -> Result<Vec<AudioTrack>, DraftAudioGuard> {
    match mode {
        DraftAudioMode::TextOnly => Ok(Vec::new()),
        DraftAudioMode::Guard => match draft_audio_guard(status, tracks.len()) {
            Some(guard) => Err(guard),
            None => Ok(tracks),
        },
        DraftAudioMode::UseAvailableAudio if tracks.is_empty() => Err(draft_audio_guard(status, 0)
            .unwrap_or_else(|| DraftAudioGuard {
                expected_speaker_lanes: status.total_lanes,
                captured_speaker_lanes: 0,
                captured_tracks: 0,
                can_use_partial_audio: false,
                message: "No speaker audio was found.".to_string(),
            })),
        DraftAudioMode::UseAvailableAudio => Ok(tracks),
    }
}

/// One `audio/wav` track per lane with decoded audio; metadata carries `speakerKey`/`trackLabel`.
pub fn broker_audio_tracks<'a>(
    project: &'a Project,
    decoded_audio: impl Fn(&str) -> Option<&'a DecodedAudio>,
) -> Vec<AudioTrack> {
    project
        .lanes
        .iter()
        .filter_map(|lane| {
            let bytes = encode_pcm16_wav(decoded_audio(&lane.id)?).ok()?;
            Some(AudioTrack {
                track_id: lane.id.clone(),
                file_name: format!("{}.wav", safe_audio_file_stem(&lane.id)),
                mime_type: "audio/wav".to_string(),
                metadata: json!({ "speakerKey": lane.speaker_id, "trackLabel": lane.label }),
                bytes: bytes.into(),
            })
        })
        .collect()
}

/// Interleaved little-endian PCM16 WAV (samples clamped to [-1, 1]).
pub fn encode_pcm16_wav(audio: &DecodedAudio) -> Result<Vec<u8>> {
    let channels = audio.number_of_channels();
    if channels == 0 || channels > usize::from(u16::MAX) {
        bail!("unsupported channel count: {channels}");
    }
    let frame_count = audio.frame_count();
    let sample_rate = audio.sample_rate();
    let data_len = frame_count
        .checked_mul(channels * 2)
        .filter(|len| *len <= u32::MAX as usize - 36)
        .ok_or_else(|| anyhow::anyhow!("audio is too large to encode as WAV"))?;
    let byte_rate = sample_rate
        .checked_mul(channels as u32 * 2)
        .ok_or_else(|| anyhow::anyhow!("audio sample rate is too large"))?;
    let channel_data = (0..channels)
        .map(|channel| {
            audio
                .channel_data(channel)
                .ok_or_else(|| anyhow::anyhow!("missing decoded audio channel: {channel}"))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut bytes = Vec::with_capacity(44 + data_len);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&(channels as u16).to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&((channels * 2) as u16).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data_len as u32).to_le_bytes());
    for frame in 0..frame_count {
        for channel in &channel_data {
            let sample = channel
                .get(frame)
                .copied()
                .unwrap_or_default()
                .clamp(-1.0, 1.0);
            let scaled = if sample < 0.0 {
                sample * 32768.0
            } else {
                sample * 32767.0
            };
            bytes.extend_from_slice(&(scaled.round() as i16).to_le_bytes());
        }
    }
    Ok(bytes)
}

fn lane_label(lane: &AudioLane) -> String {
    let label = lane.label.trim();
    if label.is_empty() {
        lane.id.clone()
    } else {
        label.to_string()
    }
}

fn safe_audio_file_stem(value: &str) -> String {
    let stem = value
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    if stem.is_empty() {
        "audio".to_string()
    } else {
        stem
    }
}
