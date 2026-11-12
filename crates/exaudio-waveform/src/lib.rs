//! Waveform peaks, line rendering and overview render model for Exaudio.

mod audio;
mod cache;
mod line;
mod peaks;
mod render_model;
mod renderer;
mod source;
mod viewport;
mod wav;

pub use audio::DecodedAudio;
pub use cache::PeakCache;
pub use line::{
    CanvasLayout, DISPLAY_SAMPLE_RATE, DisplaySignal, LineCanvasColumns, LineColumn, LineFrame,
    LineFrameCanvas, LineView, LineWaveform, js_round,
};
pub use peaks::{PeakExportError, export_peaks, export_range_peaks};
pub use render_model::{
    WaveformChannelRender, WaveformHit, WaveformHitTarget, WaveformRenderModel,
    WaveformRenderOptions,
};
pub use renderer::{ProgressGeometry, RendererLayout, ScrollWindow};
pub use source::{PrecomputedPeaks, WaveformSource};
pub use viewport::WaveViewport;
pub use wav::{WavDecodeError, decode_pcm16_wav};
