//! Decode evaluation clips by their contents, not filename or a PCM16-only assumption.
use rodio::{Decoder, Source};
use std::io::Cursor;
use studio_waveform::DecodedAudio;

const MAX_SAMPLES: usize = 32_000_000;

pub(super) fn decode(mut bytes: Vec<u8>) -> Result<DecodedAudio, String> {
    normalize_streaming_wav(&mut bytes)?;
    // Do not silently accept a truncated RIFF file as a fully listened clip.
    if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WAVE" {
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as u64 + 8;
        if declared > bytes.len() as u64 {
            return Err("Audio file is incomplete. Retry downloading the clip.".into());
        }
    }
    let mut decoder = Decoder::try_from(Cursor::new(bytes)).map_err(|_| {
        "Cannot decode this audio file (unsupported or damaged). Your answers are retained."
            .to_string()
    })?;
    let channel_count = decoder.channels().get() as usize;
    let sample_rate = decoder.sample_rate().get();
    if channel_count > 8 || sample_rate > 384_000 {
        return Err("Audio exceeds supported channel or sample-rate limits.".into());
    }
    let expected_duration = decoder.total_duration();
    let mut channels = vec![Vec::new(); channel_count];
    let mut count = 0;
    while let Some(sample) = decoder.next() {
        if count >= MAX_SAMPLES {
            return Err("Decoded audio exceeds the memory limit.".into());
        }
        if !sample.is_finite() {
            return Err("Audio contains invalid samples.".into());
        }
        channels[count % channel_count].push(sample as f32);
        count += 1;
    }
    if count == 0 || count % channel_count != 0 {
        return Err("Audio contains no complete playable frames.".into());
    }
    let audio = DecodedAudio::from_f32_channels(sample_rate, channels)?;
    if expected_duration.is_some_and(|expected| audio.duration() + 0.25 < expected.as_secs_f64()) {
        return Err(
            "Audio decoding stopped before the end of the clip. Retry downloading it.".into(),
        );
    }
    Ok(audio)
}

// Some streaming WAV writers cannot seek back to write lengths and use MAX.
// Normalize only uncompressed, frame-aligned data extending to EOF. A stream
// with no declared length cannot reveal truncation on a whole-frame boundary.
fn normalize_streaming_wav(bytes: &mut [u8]) -> Result<(), String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Ok(());
    }
    let incomplete = || "Audio file is incomplete. Retry downloading the clip.".to_string();
    let riff_size = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let end = if riff_size == u32::MAX {
        bytes.len()
    } else {
        let end = riff_size as u64 + 8;
        if end > bytes.len() as u64 || end < 12 {
            return Err(incomplete());
        }
        end as usize
    };
    let mut offset = 12;
    let mut pcm_align = None;
    while offset < end {
        if end - offset < 8 {
            return Err(incomplete());
        }
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
        let start = offset + 8;
        if size == u32::MAX {
            if &bytes[offset..offset + 4] != b"data" {
                return Err("Unsupported unknown-length WAV chunk.".into());
            }
            let align = pcm_align.ok_or("Unsupported unknown-length WAV format.")?;
            let actual = end - start;
            if actual == 0 || actual % align != 0 {
                return Err(incomplete());
            }
            let actual = u32::try_from(actual).map_err(|_| incomplete())?;
            bytes[offset + 4..offset + 8].copy_from_slice(&actual.to_le_bytes());
            break;
        }
        let chunk_end = start.checked_add(size as usize).ok_or_else(incomplete)?;
        if chunk_end > end {
            return Err(incomplete());
        }
        if &bytes[offset..offset + 4] == b"fmt " && size >= 16 {
            let format = u16::from_le_bytes(bytes[start..start + 2].try_into().unwrap());
            let channels =
                u16::from_le_bytes(bytes[start + 2..start + 4].try_into().unwrap()) as usize;
            let align =
                u16::from_le_bytes(bytes[start + 12..start + 14].try_into().unwrap()) as usize;
            let bits =
                u16::from_le_bytes(bytes[start + 14..start + 16].try_into().unwrap()) as usize;
            if matches!(format, 1 | 3)
                && channels > 0
                && bits > 0
                && bits % 8 == 0
                && align == channels * (bits / 8)
            {
                pcm_align = Some(align);
            }
        }
        offset = chunk_end + (size as usize % 2);
    }
    if riff_size == u32::MAX {
        let actual = u32::try_from(bytes.len() - 8).map_err(|_| incomplete())?;
        bytes[4..8].copy_from_slice(&actual.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav(format: u16, bits: u16, channels: u16, data: &[u8]) -> Vec<u8> {
        let align = channels * (bits / 8);
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&format.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&48000u32.to_le_bytes());
        out.extend_from_slice(&(48000u32 * align as u32).to_le_bytes());
        out.extend_from_slice(&align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }
    #[test]
    fn pcm16_stereo_keeps_channel_order() {
        let data: Vec<u8> = [16384i16, -16384, 8192, -8192]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let audio = decode(wav(1, 16, 2, &data)).unwrap();
        assert_eq!(audio.sample_rate(), 48000);
        assert_eq!(audio.channel_data(0).unwrap(), &[0.5, 0.25]);
        assert_eq!(audio.channel_data(1).unwrap(), &[-0.5, -0.25]);
    }
    #[test]
    fn pcm24_previously_rejected_is_decoded() {
        let bytes = wav(1, 24, 1, &[0, 0, 64, 0, 0, 192]);
        assert!(studio_waveform::decode_pcm16_wav(&bytes).is_err());
        let audio = decode(bytes).unwrap();
        assert_eq!(audio.channel_data(0).unwrap(), &[0.5, -0.5]);
    }
    #[test]
    fn float32_previously_rejected_is_decoded() {
        let data: Vec<u8> = [0.25f32, -0.5]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let bytes = wav(3, 32, 1, &data);
        assert!(studio_waveform::decode_pcm16_wav(&bytes).is_err());
        assert_eq!(
            decode(bytes).unwrap().channel_data(0).unwrap(),
            &[0.25, -0.5]
        );
    }
    #[test]
    fn pcm32_is_decoded() {
        let data: Vec<u8> = [1073741824i32, -1073741824]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(
            decode(wav(1, 32, 1, &data))
                .unwrap()
                .channel_data(0)
                .unwrap(),
            &[0.5, -0.5]
        );
    }
    #[test]
    fn rejects_truncated_empty_and_non_audio_responses() {
        let mut bytes = wav(1, 16, 1, &[0, 64, 0, 192]);
        bytes.pop();
        assert!(decode(bytes).is_err());
        assert!(decode(wav(1, 16, 1, &[])).is_err());
        assert!(decode(b"<html>Access denied</html>".to_vec()).is_err());
    }

    #[test]
    fn streaming_pcm16_lengths_are_normalized() {
        let mut bytes = wav(1, 16, 1, &[0, 64, 0, 192]);
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(
            decode(bytes.clone()).unwrap().channel_data(0).unwrap(),
            &[0.5, -0.5]
        );
        bytes.pop();
        assert!(decode(bytes).is_err());
    }

    #[test]
    fn streaming_riff_still_rejects_truncated_known_data() {
        let mut bytes = wav(1, 16, 1, &[0, 64, 0, 192]);
        bytes[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        bytes.truncate(bytes.len() - 2);
        assert!(decode(bytes).is_err());
    }

    #[test]
    fn unknown_metadata_size_is_not_treated_as_audio() {
        let mut bytes = wav(1, 16, 1, &[0, 64, 0, 192]);
        bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(bytes).is_err());
    }
}
