use studio_audio::{AudioDecodeError, AudioError, decode_audio_bytes, decode_audio_file};

// Original synthetic CC0 signals. See fixtures/generate.py for provenance and
// reproducible FFmpeg generation commands; no runtime encoder is needed.
const FORMATS: &[(&str, &[u8], bool)] = &[
    ("FLAC", include_bytes!("fixtures/tone.flac"), true),
    ("MP3", include_bytes!("fixtures/tone.mp3"), false),
    ("Vorbis", include_bytes!("fixtures/tone-vorbis.ogg"), false),
    ("Opus", include_bytes!("fixtures/tone-opus.ogg"), false),
    ("AAC/M4A", include_bytes!("fixtures/tone.m4a"), false),
    (
        "AAC/MP4 video",
        include_bytes!("fixtures/tone-video.mp4"),
        false,
    ),
    (
        "Vorbis/WebM video",
        include_bytes!("fixtures/tone-video.webm"),
        false,
    ),
    (
        "FLAC/Matroska video",
        include_bytes!("fixtures/tone-video.mkv"),
        true,
    ),
];

fn wav(format: u16, bits: u16, channels: u16, sample_rate: u32, data: &[u8]) -> Vec<u8> {
    let align = channels * (bits / 8);
    let padding = data.len() % 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data.len() as u32 + padding as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&format.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * u32::from(align)).to_le_bytes());
    bytes.extend_from_slice(&align.to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(data);
    bytes.resize(bytes.len() + padding, 0);
    bytes
}

#[test]
fn all_pcm16_values_preserve_existing_waveform_decoder_fidelity() {
    let samples: Vec<u8> = (i16::MIN..=i16::MAX).flat_map(i16::to_le_bytes).collect();
    let bytes = wav(1, 16, 2, 48_000, &samples);
    let expected = studio_waveform::decode_pcm16_wav(&bytes).unwrap();
    let decoded = decode_audio_bytes(&bytes).unwrap();
    assert_eq!(decoded, expected);
    assert_eq!(decoded.duration(), 32_768.0 / 48_000.0);
}

#[test]
fn integer_and_float_wav_preserve_channels_and_scale() {
    let cases = [
        (1, 8, vec![0, 128, 192, 64]),
        (
            1,
            16,
            [-32768_i16, 0, 16384, -16384]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect(),
        ),
        (1, 24, vec![0, 0, 128, 0, 0, 0, 0, 0, 64, 0, 0, 192]),
        (
            1,
            32,
            [i32::MIN, 0, 1 << 30, -(1 << 30)]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect(),
        ),
        (
            3,
            32,
            [-1_f32, 0.0, 0.5, -0.5]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
        ),
        (
            3,
            64,
            [-1_f64, 0.0, 0.5, -0.5]
                .into_iter()
                .flat_map(f64::to_le_bytes)
                .collect(),
        ),
    ];
    for (format, bits, samples) in cases {
        let audio = decode_audio_bytes(&wav(format, bits, 2, 8_000, &samples)).unwrap();
        assert_eq!(audio.sample_rate(), 8_000, "{format}/{bits}");
        assert_eq!(audio.number_of_channels(), 2, "{format}/{bits}");
        assert_eq!(audio.frame_count(), 2, "{format}/{bits}");
        assert_eq!(
            audio.channel_data(0).unwrap(),
            &[-1.0, 0.5],
            "{format}/{bits}"
        );
        assert_eq!(
            audio.channel_data(1).unwrap(),
            &[0.0, -0.5],
            "{format}/{bits}"
        );
        assert_eq!(audio.duration(), 2.0 / 8_000.0);
    }
}

#[test]
fn common_audio_and_audio_bearing_video_decode_without_filename_hints() {
    for &(name, bytes, lossless) in FORMATS {
        let audio = decode_audio_bytes(bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(audio.sample_rate(), 48_000, "{name}");
        assert_eq!(audio.number_of_channels(), 2, "{name}");
        if lossless || name == "Opus" || name == "MP3" {
            assert_eq!(audio.frame_count(), 12_000, "{name}");
        } else {
            // AAC may retain encoder priming/padding; no invented duration from metadata.
            assert!(
                (audio.duration() - 0.25).abs() < 0.05,
                "{name}: {}",
                audio.duration()
            );
        }
        assert_eq!(audio.duration(), audio.frame_count() as f64 / 48_000.0);
        for channel in 0..2 {
            let samples = audio.channel_data(channel).unwrap();
            assert!(samples.iter().all(|sample| sample.is_finite()), "{name}");
            let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
                / samples.len() as f32)
                .sqrt();
            assert!(
                (0.10..0.24).contains(&rms),
                "{name}, channel {channel}: RMS {rms}"
            );
            // Measure the intended frequency, independent of encoder delay/phase.
            let frequency = if channel == 0 { 440.0 } else { 880.0 };
            let (mut real, mut imaginary) = (0.0_f64, 0.0_f64);
            for (frame, sample) in samples.iter().enumerate() {
                let angle = std::f64::consts::TAU * frequency * frame as f64 / 48_000.0;
                real += f64::from(*sample) * angle.cos();
                imaginary += f64::from(*sample) * angle.sin();
            }
            let amplitude = 2.0 * real.hypot(imaginary) / samples.len() as f64;
            assert!(
                (0.16..0.31).contains(&amplitude),
                "{name}, channel {channel}: amplitude {amplitude}"
            );
        }
    }
}

#[test]
fn file_and_memory_paths_decode_identically_even_with_wrong_extension() {
    let temp = tempfile::tempdir().unwrap();
    for &(name, bytes, _) in FORMATS {
        let path = temp.path().join("media.wav");
        std::fs::write(&path, bytes).unwrap();
        assert_eq!(
            decode_audio_file(&path).unwrap(),
            decode_audio_bytes(bytes).unwrap(),
            "{name}"
        );
    }
}

#[test]
fn invalid_truncated_and_empty_audio_are_errors_not_partial_success() {
    for bytes in [
        &b""[..],
        &b"not media"[..],
        &b"RIFF\xff\xff\xff\xffWAVE"[..],
    ] {
        assert!(matches!(
            decode_audio_bytes(bytes),
            Err(AudioError::Decode(_))
        ));
    }
    let complete = wav(1, 16, 1, 8_000, &[0; 160]);
    for length in [12, 36, 43, 44, complete.len() - 2] {
        assert!(
            decode_audio_bytes(&complete[..length]).is_err(),
            "WAV prefix {length}"
        );
    }
    assert!(decode_audio_bytes(&wav(1, 16, 1, 8_000, &[])).is_err());
    for &(name, bytes, _) in FORMATS {
        assert!(
            decode_audio_bytes(&bytes[..bytes.len() / 2]).is_err(),
            "truncated {name}"
        );
    }
}

#[test]
fn unsupported_codec_and_video_only_are_distinct_from_file_open_errors() {
    let unsupported_wav = wav(0xffff, 16, 1, 8_000, &[0; 160]);
    assert!(matches!(
        decode_audio_bytes(&unsupported_wav),
        Err(AudioError::Decode(AudioDecodeError::Unsupported(_)))
    ));
    assert!(matches!(
        decode_audio_bytes(include_bytes!("fixtures/video-only.mp4")),
        Err(AudioError::Decode(AudioDecodeError::Unsupported(_)))
    ));
    assert!(decode_audio_bytes(include_bytes!("fixtures/encrypted.m4a")).is_err());
}

#[test]
fn corrupt_flac_checksum_is_not_silently_accepted() {
    let mut bytes = include_bytes!("fixtures/tone.flac").to_vec();
    // Native FLAC's first metadata block is STREAMINFO. Its MD5 starts at
    // byte 26 (4-byte magic + 4-byte block header + 18-byte stream fields).
    bytes[26] ^= 0x80;
    assert!(matches!(
        decode_audio_bytes(&bytes),
        Err(AudioError::Decode(AudioDecodeError::Invalid(_)))
    ));
}

fn opus_header_variant(pre_skip: Option<u16>, gain: i16) -> Vec<u8> {
    let mut bytes = include_bytes!("fixtures/tone-opus.ogg").to_vec();
    let segments = usize::from(bytes[26]);
    let header = 27 + segments;
    let page_end = header
        + bytes[27..header]
            .iter()
            .map(|size| usize::from(*size))
            .sum::<usize>();
    assert_eq!(&bytes[header..header + 8], b"OpusHead");
    if let Some(pre_skip) = pre_skip {
        bytes[header + 10..header + 12].copy_from_slice(&pre_skip.to_le_bytes());
    }
    bytes[header + 16..header + 18].copy_from_slice(&gain.to_le_bytes());
    bytes[22..26].fill(0);
    let mut crc = 0_u32;
    for byte in &bytes[..page_end] {
        crc ^= u32::from(*byte) << 24;
        for _ in 0..8 {
            crc = (crc << 1)
                ^ if crc & 0x8000_0000 != 0 {
                    0x04c1_1db7
                } else {
                    0
                };
        }
    }
    bytes[22..26].copy_from_slice(&crc.to_le_bytes());
    bytes
}

#[test]
fn opus_header_gain_and_multi_packet_pre_skip_are_applied() {
    let baseline = decode_audio_bytes(include_bytes!("fixtures/tone-opus.ogg")).unwrap();
    let quieter = decode_audio_bytes(&opus_header_variant(None, -6 * 256)).unwrap();
    let gain = 10.0_f32.powf(-6.0 / 20.0);
    assert_eq!(baseline.frame_count(), quieter.frame_count());
    for channel in 0..2 {
        for (original, quiet) in baseline
            .channel_data(channel)
            .unwrap()
            .iter()
            .zip(quieter.channel_data(channel).unwrap())
        {
            assert!((original * gain - quiet).abs() < 0.000001);
        }
    }
    // Original libopus pre-skip is 312 frames. Increasing it by 1920 frames
    // deliberately spans multiple 20 ms packets without changing the payload.
    let shifted = decode_audio_bytes(&opus_header_variant(Some(312 + 1920), 0)).unwrap();
    assert_eq!(shifted.frame_count(), baseline.frame_count() - 1920);
    for channel in 0..2 {
        assert_eq!(
            shifted.channel_data(channel).unwrap(),
            &baseline.channel_data(channel).unwrap()[1920..],
        );
    }
}
