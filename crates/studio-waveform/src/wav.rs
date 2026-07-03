use crate::DecodedAudio;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WavDecodeError {
    InvalidHeader,
    MissingFormatChunk,
    MissingDataChunk,
    UnsupportedFormat,
    MalformedData,
}

pub fn decode_pcm16_wav(bytes: &[u8]) -> Result<DecodedAudio, WavDecodeError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavDecodeError::InvalidHeader);
    }

    let mut offset = 12;
    let mut format = None;
    let mut data = None;

    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset + 4];
        let size = read_u32_le(bytes, offset + 4)? as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .ok_or(WavDecodeError::MalformedData)?;
        if end > bytes.len() {
            return Err(WavDecodeError::MalformedData);
        }

        match id {
            b"fmt " => {
                if size < 16 {
                    return Err(WavDecodeError::MalformedData);
                }
                format = Some(WavFormat {
                    audio_format: read_u16_le(bytes, start)?,
                    channels: read_u16_le(bytes, start + 2)?,
                    sample_rate: read_u32_le(bytes, start + 4)?,
                    block_align: read_u16_le(bytes, start + 12)?,
                    bits_per_sample: read_u16_le(bytes, start + 14)?,
                });
            }
            b"data" => data = Some((start, size)),
            _ => {}
        }

        offset = end + size % 2;
    }

    let format = format.ok_or(WavDecodeError::MissingFormatChunk)?;
    let (data_offset, data_size) = data.ok_or(WavDecodeError::MissingDataChunk)?;

    if format.audio_format != 1
        || format.channels == 0
        || format.bits_per_sample != 16
        || format.block_align != format.channels * 2
        || format.sample_rate == 0
    {
        return Err(WavDecodeError::UnsupportedFormat);
    }

    let frame_size = format.block_align as usize;
    if data_size % frame_size != 0 {
        return Err(WavDecodeError::MalformedData);
    }

    let channel_count = format.channels as usize;
    let frame_count = data_size / frame_size;
    let mut channels = vec![Vec::with_capacity(frame_count); channel_count];

    for frame_index in 0..frame_count {
        let frame_offset = data_offset + frame_index * frame_size;
        for (channel_index, channel) in channels.iter_mut().enumerate() {
            let sample_offset = frame_offset + channel_index * 2;
            let sample = i16::from_le_bytes([bytes[sample_offset], bytes[sample_offset + 1]]);
            channel.push(sample as f32 / 32768.0);
        }
    }

    DecodedAudio::from_f32_channels(format.sample_rate, channels)
        .map_err(|_| WavDecodeError::MalformedData)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WavFormat {
    audio_format: u16,
    channels: u16,
    sample_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, WavDecodeError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(WavDecodeError::MalformedData)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, WavDecodeError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(WavDecodeError::MalformedData)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}
