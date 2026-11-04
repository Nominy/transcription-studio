//! Waveforms of what each lane plays while enhanced (denoised) audio is installed.
//!
//! A lane with enhanced audio and a non-zero enhanced ratio is painted from
//! [`blend_lane_audio`] — the playback mixer's own per-sample blend of its original and
//! enhanced PCM — instead of its original. Lane mute, solo and volume do not change the
//! picture, as before. [`EditorModel`](super::model::EditorModel) builds blends off the UI
//! thread, one per lane at a time (the newest wanted ratio wins; detents the slider passed
//! meanwhile are never built), and the previously painted waveform stays until its
//! replacement is ready. Built blends are kept per ratio in a byte-bounded LRU, so returning
//! to a ratio swaps instantly.

use std::collections::HashMap;

use studio_audio::{DecodedAudio, blend_lane_audio};
use studio_editor::timeline_model::LaneWaveform;

/// Retained bytes of the blends kept but not painted, all lanes together. The most recently
/// built blend is kept even when it alone exceeds the budget.
const CACHE_BYTE_BUDGET: usize = 256 * 1024 * 1024;

/// One lane's blend of its current original and enhanced audio at one enhanced ratio.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlendKey {
    pub lane_id: String,
    /// [`BlendedWaveforms::sources`] when the blend was requested.
    sources: u64,
    ratio_bits: u32,
}

impl BlendKey {
    pub fn ratio(&self) -> f32 {
        f32::from_bits(self.ratio_bits)
    }
}

/// What a lane should paint now.
#[derive(Debug)]
pub enum Selection {
    /// It already paints the wanted waveform.
    Unchanged,
    /// Paint this blend (`None`: the original); the replaced blend goes back through
    /// [`BlendedWaveforms::store`] under `replaced`.
    Paint {
        waveform: Option<Box<LaneWaveform>>,
        replaced: Option<BlendKey>,
    },
    /// The wanted blend is not built; keep painting the current waveform meanwhile.
    Missing,
}

/// Blend bookkeeping: which blend each lane paints and the built blends not painted.
#[derive(Debug)]
pub struct BlendedWaveforms {
    /// Bumped whenever a lane's original or enhanced audio changes; older blends are stale.
    sources: u64,
    /// Blend painted per lane (lanes painting their original are absent).
    painted: HashMap<String, BlendKey>,
    /// Built blends not painted with their retained bytes, least recently used first.
    cached: Vec<(BlendKey, LaneWaveform, usize)>,
    cached_bytes: usize,
    budget_bytes: usize,
}

impl Default for BlendedWaveforms {
    fn default() -> Self {
        Self {
            sources: 0,
            painted: HashMap::new(),
            cached: Vec::new(),
            cached_bytes: 0,
            budget_bytes: CACHE_BYTE_BUDGET,
        }
    }
}

impl BlendedWaveforms {
    /// Marks every built blend stale (originals or enhanced audio changed).
    pub fn sources_changed(&mut self) {
        self.sources = self.sources.wrapping_add(1);
        self.cached.clear();
        self.cached_bytes = 0;
    }

    /// The blend a lane plays at `enhanced_ratio` of `has_enhanced` audio, or `None` when it
    /// plays its original only.
    pub fn wanted(
        &self,
        lane_id: &str,
        has_enhanced: bool,
        enhanced_ratio: f32,
    ) -> Option<BlendKey> {
        (has_enhanced && enhanced_ratio > 0.0).then(|| BlendKey {
            lane_id: lane_id.to_owned(),
            sources: self.sources,
            ratio_bits: enhanced_ratio.to_bits(),
        })
    }

    /// Chooses what `lane_id` paints when it plays `wanted`, taking a built blend out of the
    /// cache. A missing blend leaves the current painting (and bookkeeping) as is.
    pub fn select(&mut self, lane_id: &str, wanted: Option<BlendKey>) -> Selection {
        if self.painted.get(lane_id) == wanted.as_ref() {
            return Selection::Unchanged;
        }
        let waveform = match &wanted {
            None => None,
            Some(key) => {
                let Some(index) = self.cached.iter().position(|(cached, ..)| cached == key) else {
                    return Selection::Missing;
                };
                let (_, waveform, bytes) = self.cached.remove(index);
                self.cached_bytes -= bytes;
                Some(Box::new(waveform))
            }
        };
        let replaced = match wanted {
            Some(key) => self.painted.insert(lane_id.to_owned(), key),
            None => self.painted.remove(lane_id),
        };
        Selection::Paint { waveform, replaced }
    }

    /// Keeps a built or no longer painted blend as the most recently used, evicting the least
    /// recently used ones over budget. Stale blends are dropped.
    pub fn store(&mut self, key: BlendKey, waveform: LaneWaveform) {
        if key.sources != self.sources || self.cached.iter().any(|(cached, ..)| *cached == key) {
            return;
        }
        let bytes = waveform.retained_bytes();
        self.cached.push((key, waveform, bytes));
        self.cached_bytes += bytes;
        while self.cached_bytes > self.budget_bytes && self.cached.len() > 1 {
            let (_, _, bytes) = self.cached.remove(0);
            self.cached_bytes -= bytes;
        }
    }

    #[cfg(test)]
    fn cached_ratios(&self, lane_id: &str) -> Vec<f32> {
        self.cached
            .iter()
            .filter(|(key, ..)| key.lane_id == lane_id)
            .map(|(key, ..)| key.ratio())
            .collect()
    }
}

/// Builds the waveform a lane paints for `key`: the display signal of what it plays. `None`
/// when the enhanced audio cannot play beside the original. O(frames); run off the UI thread.
pub fn build_blend(
    key: &BlendKey,
    original: &DecodedAudio,
    enhanced: &DecodedAudio,
) -> Option<LaneWaveform> {
    blend_lane_audio(original, enhanced, key.ratio())
        .map(|audio| LaneWaveform::display_only(&audio))
}

#[cfg(test)]
mod tests {
    use std::f32::consts::TAU;

    use super::*;

    const RATE: u32 = 16_000;
    const FRAMES: usize = 3 * RATE as usize;

    fn audio(samples: impl Fn(usize) -> f32) -> DecodedAudio {
        DecodedAudio::from_f32_channels(RATE, vec![(0..FRAMES).map(samples).collect()]).unwrap()
    }

    /// Speech-like bursts over a hum, and a quieter "denoised" version with different peaks.
    fn original() -> DecodedAudio {
        audio(|frame| {
            let t = frame as f32 / RATE as f32;
            let burst = if (t * 2.0).fract() < 0.4 { 0.7 } else { 0.05 };
            burst * (TAU * 220.0 * t).sin() + 0.2 * (TAU * 50.0 * t).sin()
        })
    }

    fn enhanced() -> DecodedAudio {
        audio(|frame| {
            let t = frame as f32 / RATE as f32;
            let burst = if (t * 2.0).fract() < 0.4 { 0.6 } else { 0.0 };
            burst * (TAU * 220.0 * t + 0.3).sin()
        })
    }

    fn key(ratio: f32) -> BlendKey {
        BlendKey {
            lane_id: "lane".to_owned(),
            sources: 0,
            ratio_bits: ratio.to_bits(),
        }
    }

    /// The mixer's formula evaluated directly, sample by sample.
    fn brute_force_blend(
        original: &DecodedAudio,
        enhanced: &DecodedAudio,
        ratio: f32,
    ) -> DecodedAudio {
        let original = original.channel_data(0).unwrap();
        let enhanced = enhanced.channel_data(0).unwrap();
        audio(|frame| {
            original[frame].clamp(-1.0, 1.0) * (1.0 - ratio)
                + enhanced[frame].clamp(-1.0, 1.0) * ratio
        })
    }

    /// Asserts both waveforms paint the same: the timeline's display signal (equality) and
    /// the overview's peaks.
    fn assert_paints_like(actual: &LaneWaveform, expected: &LaneWaveform, context: &str) {
        assert!(actual == expected, "{context}: display signals differ");
        assert_eq!(
            actual.overview_render_model(0.0),
            expected.overview_render_model(0.0),
            "{context}: overview peaks differ"
        );
    }

    #[test]
    fn blended_peaks_match_a_brute_force_mix_at_every_slider_step() {
        let (original, enhanced) = (original(), enhanced());
        for percent in (10..=90_u8).step_by(10) {
            let ratio = f32::from(percent) / 100.0;
            let blended = build_blend(&key(ratio), &original, &enhanced).unwrap();
            let reference =
                LaneWaveform::display_only(&brute_force_blend(&original, &enhanced, ratio));
            assert_paints_like(&blended, &reference, &format!("{percent}%"));
        }
    }

    #[test]
    fn blended_peaks_are_not_a_blend_of_peaks() {
        // Opposite-phase audio cancels at 50%: the blend is silent although both sources peak
        // high, so blending their peaks would paint a loud lane.
        let original = original();
        let inverted = audio(|frame| -original.channel_data(0).unwrap()[frame]);
        let silence = LaneWaveform::display_only(&audio(|_| 0.0));

        let blended = build_blend(&key(0.5), &original, &inverted).unwrap();

        assert_paints_like(&blended, &silence, "cancelling blend");
        let original = LaneWaveform::display_only(&original);
        assert!(original != silence);
        assert_ne!(
            original.overview_render_model(0.0),
            silence.overview_render_model(0.0)
        );
    }

    #[test]
    fn full_original_and_full_enhanced_blends_paint_their_source() {
        let (original, enhanced) = (original(), enhanced());

        let none = build_blend(&key(0.0), &original, &enhanced).unwrap();
        let full = build_blend(&key(1.0), &original, &enhanced).unwrap();

        assert_paints_like(&none, &LaneWaveform::display_only(&original), "0%");
        assert_paints_like(&full, &LaneWaveform::display_only(&enhanced), "100%");
        // A lane at 0% paints its original waveform itself, which paints the same.
        assert_eq!(BlendedWaveforms::default().wanted("lane", true, 0.0), None);
        assert_eq!(BlendedWaveforms::default().wanted("lane", false, 0.5), None);
        let painted_original = LaneWaveform::from_decoded(original);
        assert_eq!(
            none.overview_render_model(0.0),
            painted_original.overview_render_model(0.0)
        );
        assert_eq!(
            none.line().signal().samples_shared(),
            painted_original.line().signal().samples_shared()
        );
    }

    fn blend(ratio: f32) -> (BlendKey, LaneWaveform) {
        let key = key(ratio);
        let waveform = build_blend(&key, &original(), &enhanced()).unwrap();
        (key, waveform)
    }

    #[test]
    fn painted_blends_swap_with_cached_ones_and_wait_for_missing_ones() {
        let mut blends = BlendedWaveforms::default();
        let (half_key, half) = blend(0.5);
        let (most_key, most) = blend(0.9);
        blends.store(half_key.clone(), half);

        // Built: painted at once.
        let Selection::Paint { waveform, replaced } = blends.select("lane", Some(half_key.clone()))
        else {
            panic!("a built blend paints");
        };
        assert!(waveform.is_some() && replaced.is_none());
        assert!(matches!(
            blends.select("lane", Some(half_key.clone())),
            Selection::Unchanged
        ));

        // Not built yet: the 50% blend stays painted.
        assert!(matches!(
            blends.select("lane", Some(most_key.clone())),
            Selection::Missing
        ));
        blends.store(most_key.clone(), most);
        let Selection::Paint { replaced, .. } = blends.select("lane", Some(most_key)) else {
            panic!("the finished blend paints");
        };
        assert_eq!(replaced, Some(half_key.clone()));

        // Back to the original; the replaced blend is returned for reuse.
        let Selection::Paint { waveform, replaced } = blends.select("lane", None) else {
            panic!("the original paints");
        };
        assert!(waveform.is_none());
        assert_eq!(replaced.map(|key| key.ratio()), Some(0.9));
    }

    #[test]
    fn changed_sources_make_built_blends_stale() {
        let mut blends = BlendedWaveforms::default();
        let (stale_key, stale) = blend(0.5);
        blends.sources_changed();

        blends.store(stale_key.clone(), stale);

        assert!(blends.cached_ratios("lane").is_empty());
        let wanted = blends.wanted("lane", true, 0.5);
        assert_ne!(wanted.as_ref(), Some(&stale_key));
        assert!(matches!(blends.select("lane", wanted), Selection::Missing));
    }

    #[test]
    fn the_cache_evicts_least_recently_used_blends_over_budget() {
        let (half_key, half) = blend(0.5);
        let bytes = half.retained_bytes();
        assert!(bytes > 0);
        let mut blends = BlendedWaveforms {
            budget_bytes: 2 * bytes,
            ..BlendedWaveforms::default()
        };
        let (more_key, more) = blend(0.6);
        blends.store(half_key.clone(), half);
        blends.store(more_key, more);
        // Painting 50% and returning to the original makes 50% the most recently used.
        let Selection::Paint { waveform, .. } = blends.select("lane", Some(half_key.clone()))
        else {
            panic!("the 50% blend is built");
        };
        let Selection::Paint { replaced, .. } = blends.select("lane", None) else {
            panic!("the original paints");
        };
        blends.store(replaced.unwrap(), *waveform.unwrap());
        let (most_key, most) = blend(0.7);
        blends.store(most_key, most);

        assert_eq!(blends.cached_ratios("lane"), vec![0.5, 0.7]);
        assert!(blends.cached_bytes <= blends.budget_bytes);

        // A blend larger than the whole budget is still kept, alone.
        blends.budget_bytes = 1;
        let (last_key, last) = blend(0.8);
        blends.store(last_key, last);
        assert_eq!(blends.cached_ratios("lane"), vec![0.8]);
    }
}
