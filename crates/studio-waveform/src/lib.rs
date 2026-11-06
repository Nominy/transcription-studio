//! Native Wavesurfer-shaped waveform core for Exaudio.

mod audio;
mod cache;
mod line;
mod peaks;
mod region;
mod render_model;
mod renderer;
mod source;
mod viewport;
mod wav;
mod wavesurfer;

pub use audio::DecodedAudio;
pub use cache::PeakCache;
pub use line::{
    CanvasLayout, DISPLAY_SAMPLE_RATE, DisplaySignal, LineCanvasColumns, LineColumn, LineFrame,
    LineFrameCanvas, LineView, LineWaveform, js_round,
};
pub use peaks::{PeakExportError, export_peaks, export_range_peaks};
pub use region::{
    Region, RegionGeometry, RegionParams, RegionUpdate, RegionUpdateSide, RegionVirtualVisibility,
    RegionsPlugin,
};
pub use render_model::{
    WaveformChannelRender, WaveformHit, WaveformHitTarget, WaveformHitTester, WaveformRegionRender,
    WaveformRenderModel, WaveformRenderOptions,
};
pub use renderer::{ProgressGeometry, RendererLayout, RendererOptions, ScrollWindow};
pub use source::{PrecomputedPeaks, WaveformSource};
pub use viewport::WaveViewport;
pub use wav::{WavDecodeError, decode_pcm16_wav};
pub use wavesurfer::{WaveSurfer, WaveSurferEvent, WaveSurferOptions};
