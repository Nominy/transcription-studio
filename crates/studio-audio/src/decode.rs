//! Lane audio decoding. Decoding is blocking CPU/IO work: call these from a
//! background executor and hand the result to [`crate::AudioEngine::insert_lane`].

use std::{
    fs::File,
    io::Cursor,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use studio_core::AudioLane;
use studio_waveform::DecodedAudio;
use symphonia::core::{
    audio::{Audio, GenericAudioBufferRef, conv::FromSample, sample::Sample},
    codecs::{audio::AudioDecoderOptions, registry::CodecRegistry},
    errors::Error as MediaError,
    formats::{TrackType, probe::Hint},
    io::{MediaSource, MediaSourceStream},
};
use symphonia_adapter_libopus::OpusDecoder;

use crate::{AudioDecodeError, AudioError};

/// Decodes audio without depending on a filename or a platform-specific backend.
///
/// Supported: 8/16/24/32-bit integer and 32/64-bit float PCM WAV, MP3, FLAC,
/// Ogg Vorbis, mono/stereo Opus (mapping family 0), and AAC-LC in M4A/MP4
/// or ADTS. MP4/MOV and
/// Matroska/WebM may contain video: only the default audio track is decoded,
/// and it must use one of these codecs. Video is never decoded. DRM, HE-AAC,
/// AC-3/E-AC-3, DTS, AVI, MPEG-TS, and multichannel Opus are not supported.
///
/// Returns planar f32 PCM at the decoded rate and channel count; no resampling,
/// downmixing or normalization is performed. Duration is the decoded frame
/// count divided by the sample rate. Container/codec padding is removed where
/// supported by the decoder (AAC/MP4 edit lists are not guaranteed gapless).
/// Decoder-reported corrupt packets fail the entire decode. Truncation is
/// rejected when the container reports an incomplete packet or an expected
/// frame count; a clean packet boundary in a stream with no declared length
/// cannot always be distinguished from an intentionally shorter recording.
pub fn decode_audio_bytes(bytes: &[u8]) -> Result<DecodedAudio, AudioError> {
    decode_source(Box::new(Cursor::new(bytes)), &Hint::new())
}

/// Decodes directly from a seekable file with bounded input buffering. Only the
/// output PCM is retained in memory, not a second copy of the encoded file.
pub fn decode_audio_file(path: impl AsRef<Path>) -> Result<DecodedAudio, AudioError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| AudioError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    decode_source(Box::new(file), &hint)
}

static CODECS: LazyLock<CodecRegistry> = LazyLock::new(|| {
    let mut registry = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut registry);
    registry.register_audio_decoder::<OpusDecoder>();
    registry
});

fn media_error(error: MediaError) -> AudioError {
    let error = match error {
        MediaError::Unsupported(reason) => AudioDecodeError::Unsupported(reason.into()),
        MediaError::ResetRequired => AudioDecodeError::Unsupported(
            "chained streams or changing codec parameters are not supported".into(),
        ),
        MediaError::IoError(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
            AudioDecodeError::Truncated(error.to_string())
        }
        MediaError::IoError(error) => AudioDecodeError::Io(error),
        error => AudioDecodeError::Invalid(error.to_string()),
    };
    AudioError::Decode(error)
}

fn invalid(reason: impl Into<String>) -> AudioError {
    AudioError::Decode(AudioDecodeError::Invalid(reason.into()))
}

fn decode_source(
    source: Box<dyn MediaSource + '_>,
    hint: &Hint,
) -> Result<DecodedAudio, AudioError> {
    let stream = MediaSourceStream::new(source, Default::default());
    let mut format = symphonia::default::get_probe()
        .probe(hint, stream, Default::default(), Default::default())
        .map_err(media_error)?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| {
        AudioError::Decode(AudioDecodeError::Unsupported("no audio track".into()))
    })?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|params| params.audio())
        .ok_or_else(|| {
            AudioError::Decode(AudioDecodeError::Unsupported("unknown audio codec".into()))
        })?;
    let (opus_gain, opus_pre_skip) =
        if params.codec == symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS {
            let header = params
                .extra_data
                .as_deref()
                .ok_or_else(|| invalid("missing OpusHead"))?;
            if header.len() < 19 || !header.starts_with(b"OpusHead") {
                return Err(invalid("invalid OpusHead"));
            }
            if header[18] != 0 {
                return Err(AudioError::Decode(AudioDecodeError::Unsupported(
                    "only Opus channel mapping family 0 is supported".into(),
                )));
            }
            // libopus does not apply the Ogg identification header's Q7.8 dB gain.
            let gain = i16::from_le_bytes([header[16], header[17]]);
            (
                10.0_f32.powf(f32::from(gain) / (256.0 * 20.0)),
                u64::from(u16::from_le_bytes([header[10], header[11]])),
            )
        } else {
            (1.0, 0)
        };
    let track_id = track.id;
    // Symphonia's Ogg Opus frame count is the final granule position, which
    // includes OpusHead pre-skip. We remove those samples below.
    let expected_frames = track
        .num_frames
        .map(|frames| frames.saturating_sub(opus_pre_skip));
    // The adapter only handles pre-skip within the first packet. Clear its
    // copy of that field so we can also handle valid multi-packet pre-skip.
    let opus_params = (opus_pre_skip != 0).then(|| {
        let mut params = params.clone();
        if let Some(header) = params.extra_data.as_mut() {
            header[10..12].fill(0);
        }
        params
    });
    let mut remaining_skip = opus_pre_skip as usize;
    let mut decoder = CODECS
        .make_audio_decoder(
            opus_params.as_ref().unwrap_or(params),
            &AudioDecoderOptions::default().verify(true),
        )
        .map_err(media_error)?;
    let mut spec = None;
    let mut channels = Vec::new();
    while let Some(packet) = format.next_packet().map_err(media_error)? {
        if packet.track_id != track_id {
            continue;
        }
        let audio = decoder.decode(&packet).map_err(media_error)?;
        if let Some(spec) = &spec {
            if audio.spec() != spec {
                return Err(AudioError::Decode(AudioDecodeError::Unsupported(
                    "sample rate or channel layout changes within the audio track".into(),
                )));
            }
        } else {
            if audio.spec().rate() == 0 || audio.num_planes() == 0 {
                return Err(invalid("zero sample rate or channel count"));
            }
            spec = Some(audio.spec().clone());
            channels = (0..audio.num_planes()).map(|_| Vec::new()).collect();
        }
        let skip = remaining_skip.min(audio.frames());
        remaining_skip -= skip;
        append_audio(audio, &mut channels, skip);
    }
    if decoder.finalize().verify_ok == Some(false) {
        return Err(invalid("decoded audio checksum mismatch"));
    }
    let spec = spec.ok_or_else(|| invalid("audio track contains no packets"))?;
    let frames = channels[0].len();
    if frames == 0 {
        return Err(invalid("audio track contains no playable samples"));
    }
    if let Some(expected) = expected_frames.filter(|expected| (frames as u64) < *expected) {
        return Err(AudioError::Decode(AudioDecodeError::Truncated(format!(
            "expected {expected} frames, decoded {frames}",
        ))));
    }
    if opus_gain != 1.0 {
        for sample in channels.iter_mut().flatten() {
            *sample *= opus_gain;
        }
    }
    DecodedAudio::from_f32_channels(spec.rate(), channels).map_err(invalid)
}

fn append_planes<S: Sample>(audio: &impl Audio<S>, channels: &mut [Vec<f32>], skip: usize)
where
    f32: FromSample<S>,
{
    for (source, target) in audio.iter_planes().zip(channels) {
        target.extend(source[skip..].iter().copied().map(f32::from_sample));
    }
}

fn append_audio(audio: GenericAudioBufferRef<'_>, channels: &mut [Vec<f32>], skip: usize) {
    match audio {
        GenericAudioBufferRef::U8(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::U16(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::U24(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::U32(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::S8(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::S16(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::S24(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::S32(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::F32(audio) => append_planes(audio, channels, skip),
        GenericAudioBufferRef::F64(audio) => append_planes(audio, channels, skip),
    }
}

/// A lane's decoded audio and the file it was decoded from (the original
/// bytes inference backends receive).
#[derive(Debug)]
pub struct LaneAudioFile {
    pub path: PathBuf,
    pub audio: DecodedAudio,
}

/// Decodes the lane's explicit source; relative paths resolve against `base`.
/// Platform-specific filename conventions belong to the opening mode.
pub fn load_lane_audio(base: &Path, lane: &AudioLane) -> Result<LaneAudioFile, AudioError> {
    load_lane_audio_candidates(&lane.id, std::iter::once(base.join(&lane.source_path)))
}

/// Decodes the first readable candidate supplied by a mode. Missing files are skipped;
/// when none decodes, return the last decode error or `LaneAudioNotFound`.
pub fn load_lane_audio_candidates(
    lane_id: &str,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Result<LaneAudioFile, AudioError> {
    let mut decode_error = None;
    for path in candidates {
        match decode_audio_file(&path) {
            Err(AudioError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                continue;
            }
            Ok(audio) => return Ok(LaneAudioFile { path, audio }),
            Err(error) => decode_error = Some(error),
        }
    }
    Err(
        decode_error.unwrap_or_else(|| AudioError::LaneAudioNotFound {
            lane_id: lane_id.to_owned(),
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
    fn rejects_invalid_audio_bytes() {
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
    fn load_lane_audio_requires_explicit_platform_candidates() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path().join("speaker_1.wav"),
            pcm16_wav_bytes(4, 1, &[8_192; 4]),
        )
        .expect("write wav");

        let lane = lane("missing-source.wav", "speaker-1");
        assert!(matches!(
            load_lane_audio(temp.path(), &lane),
            Err(AudioError::LaneAudioNotFound { .. })
        ));
        let LaneAudioFile { path, audio } = load_lane_audio_candidates(
            &lane.id,
            [
                temp.path().join("missing-source.wav"),
                temp.path().join("speaker_1.wav"),
            ],
        )
        .expect("explicit candidate decodes");

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
