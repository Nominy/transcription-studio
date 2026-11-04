use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct DecodedAudio {
    sample_rate: u32,
    channels: Vec<Arc<[f32]>>,
    frame_count: usize,
    duration_seconds: f64,
}

impl DecodedAudio {
    pub fn new(sample_rate: u32, channels: Vec<Vec<f64>>) -> Result<Self, String> {
        let channels = channels
            .into_iter()
            .map(|channel| channel.into_iter().map(|sample| sample as f32).collect())
            .collect();
        Self::from_f32_channels(sample_rate, channels)
    }

    pub fn from_f32_channels(sample_rate: u32, channels: Vec<Vec<f32>>) -> Result<Self, String> {
        Self::from_shared_channels(
            sample_rate,
            channels.into_iter().map(Arc::<[f32]>::from).collect(),
        )
    }

    /// Wraps already shared channel buffers without copying them.
    pub fn from_shared_channels(
        sample_rate: u32,
        channels: Vec<Arc<[f32]>>,
    ) -> Result<Self, String> {
        if sample_rate == 0 {
            return Err("sample rate must be greater than zero".to_string());
        }
        if channels.is_empty() {
            return Err("decoded audio must contain at least one channel".to_string());
        }

        let frame_count = channels[0].len();
        if frame_count == 0 {
            return Err("decoded audio channels cannot be empty".to_string());
        }
        if channels.iter().any(|channel| channel.len() != frame_count) {
            return Err("decoded audio channels must have the same frame count".to_string());
        }

        Ok(Self {
            sample_rate,
            channels,
            frame_count,
            duration_seconds: frame_count as f64 / sample_rate as f64,
        })
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn number_of_channels(&self) -> usize {
        self.channels.len()
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn duration(&self) -> f64 {
        self.duration_seconds
    }

    pub fn channel_data(&self, channel_index: usize) -> Option<&[f32]> {
        self.channels.get(channel_index).map(Arc::as_ref)
    }

    pub fn channel_data_shared(&self, channel_index: usize) -> Option<Arc<[f32]>> {
        self.channels.get(channel_index).map(Arc::clone)
    }
}

#[cfg(test)]
mod tests {
    use super::DecodedAudio;

    #[test]
    fn decoded_audio_uses_f32_sample_storage_for_native_runtime_footprint() {
        let audio = DecodedAudio::new(48_000, vec![vec![0.25, -0.5]]).expect("valid decoded audio");
        let sample = audio
            .channel_data(0)
            .and_then(|channel| channel.first())
            .expect("decoded channel sample");

        assert_eq!(std::mem::size_of_val(sample), std::mem::size_of::<f32>());
    }
}
