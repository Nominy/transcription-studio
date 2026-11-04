//! Lane audio decoding. Decoding is blocking CPU/IO work: call these from a
//! background executor and hand the result to [`crate::AudioEngine::insert_lane`].

use std::path::{Path, PathBuf};

use studio_core::AudioLane;
use studio_waveform::{DecodedAudio, decode_pcm16_wav};

use crate::AudioError;

/// Decodes an in-memory audio file. Supports PCM16 WAV, the only format the
/// Babel audio pipeline stages locally.
pub fn decode_audio_bytes(bytes: &[u8]) -> Result<DecodedAudio, AudioError> {
    decode_pcm16_wav(bytes).map_err(AudioError::Decode)
}

/// Reads and decodes an audio file from disk (see [`decode_audio_bytes`]).
pub fn decode_audio_file(path: impl AsRef<Path>) -> Result<DecodedAudio, AudioError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| AudioError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    decode_audio_bytes(&bytes)
}

/// Local files that may hold a lane's audio, in lookup order: the lane's
/// `source_path` (relative paths resolve against `base`), then the
/// `speaker_<n>.wav` / `audio/speaker_<n>.wav` naming used by exported task
/// folders for `speaker-<n>` speakers.
pub fn audio_source_candidates(base: &Path, source_path: &str, speaker_id: &str) -> Vec<PathBuf> {
    let source = Path::new(source_path);
    let mut candidates = vec![if source.is_absolute() {
        source.to_path_buf()
    } else {
        base.join(source)
    }];
    if let Some(speaker_number) = speaker_id.strip_prefix("speaker-") {
        let local_name = format!("speaker_{speaker_number}.wav");
        for candidate in [base.join(&local_name), base.join("audio").join(&local_name)] {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

/// A lane's decoded audio and the file it was decoded from (the original
/// bytes inference backends receive).
#[derive(Debug)]
pub struct LaneAudioFile {
    pub path: PathBuf,
    pub audio: DecodedAudio,
}

/// Decodes the first readable, decodable candidate for `lane` (see
/// [`audio_source_candidates`]). Missing files are skipped; when no candidate
/// decodes, the last decode error is returned, else `LaneAudioNotFound`.
pub fn load_lane_audio(base: &Path, lane: &AudioLane) -> Result<LaneAudioFile, AudioError> {
    let mut decode_error = None;
    for path in audio_source_candidates(base, &lane.source_path, &lane.speaker_id) {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        match decode_audio_bytes(&bytes) {
            Ok(audio) => return Ok(LaneAudioFile { path, audio }),
            Err(error) => decode_error = Some(error),
        }
    }
    Err(
        decode_error.unwrap_or_else(|| AudioError::LaneAudioNotFound {
            lane_id: lane.id.clone(),
        }),
    )
}

#[cfg(test)]
pub(crate) fn pcm16_wav_bytes(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let block_align = channels * 2;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(block_align)).to_le_bytes());
    bytes.extend_from_slice(&block_align.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(source_path: &str, speaker_id: &str) -> AudioLane {
        AudioLane {
            id: "track-1".to_string(),
            speaker_id: speaker_id.to_string(),
            label: "Speaker 1".to_string(),
            source_path: source_path.to_string(),
        }
    }

    #[test]
    fn decodes_pcm16_wav_bytes_into_f32_channels() {
        let audio = decode_audio_bytes(&pcm16_wav_bytes(4, 2, &[16_384, -16_384, 8_192, 0]))
            .expect("pcm16 wav decodes");

        assert_eq!(audio.sample_rate(), 4);
        assert_eq!(audio.number_of_channels(), 2);
        assert_eq!(audio.channel_data(0), Some(&[0.5, 0.25][..]));
        assert_eq!(audio.channel_data(1), Some(&[-0.5, 0.0][..]));
    }

    #[test]
    fn rejects_non_wav_bytes() {
        assert!(matches!(
            decode_audio_bytes(b"ID3 not a wav"),
            Err(AudioError::Decode(_))
        ));
    }

    #[test]
    fn missing_file_reports_read_error_with_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let path = temp.path().join("missing.wav");
        match decode_audio_file(&path) {
            Err(AudioError::Read { path: reported, .. }) => assert_eq!(reported, path),
            other => panic!("expected read error, got {other:?}"),
        }
    }

    #[test]
    fn candidates_resolve_relative_source_then_speaker_fallbacks_without_duplicates() {
        let base = Path::new("/task");
        assert_eq!(
            audio_source_candidates(base, "audio/speaker_2.wav", "speaker-2"),
            vec![base.join("audio/speaker_2.wav"), base.join("speaker_2.wav")]
        );
        assert_eq!(
            audio_source_candidates(base, "s3://bucket/a.wav", "narrator"),
            vec![base.join("s3://bucket/a.wav")]
        );
    }

    #[test]
    fn load_lane_audio_falls_back_to_speaker_file_when_source_is_remote() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path().join("speaker_1.wav"),
            pcm16_wav_bytes(4, 1, &[8_192; 4]),
        )
        .expect("write wav");

        let LaneAudioFile { path, audio } =
            load_lane_audio(temp.path(), &lane("s3://bucket/recording.wav", "speaker-1"))
                .expect("fallback speaker file decodes");

        assert_eq!(path, temp.path().join("speaker_1.wav"));
        assert_eq!(audio.frame_count(), 4);
        assert_eq!(audio.channel_data(0), Some(&[0.25; 4][..]));
    }

    #[test]
    fn load_lane_audio_reports_decode_error_or_not_found() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert!(matches!(
            load_lane_audio(temp.path(), &lane("audio/a.wav", "speaker-1")),
            Err(AudioError::LaneAudioNotFound { lane_id }) if lane_id == "track-1"
        ));

        std::fs::write(temp.path().join("a.wav"), b"not audio").expect("write file");
        assert!(matches!(
            load_lane_audio(temp.path(), &lane("a.wav", "narrator")),
            Err(AudioError::Decode(_))
        ));
    }
}
