use rayon::prelude::*;
use std::sync::Arc;

use crate::{
    PeakCache, PeakExportError, ProgressGeometry, RendererLayout, ScrollWindow, WaveViewport,
    WaveformSource,
};

const MAX_CACHED_RENDER_PEAKS: usize = 1_048_576;
const PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN: usize = 16_384;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveformRenderOptions {
    pub channels: usize,
    pub peak_count: usize,
    pub precision: f64,
    pub cursor_width_px: f64,
}

impl Default for WaveformRenderOptions {
    fn default() -> Self {
        Self {
            channels: 1,
            peak_count: 256,
            precision: 10_000.0,
            cursor_width_px: 1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformRenderModel {
    pub layout: RendererLayout,
    pub visible_start_seconds: f64,
    pub visible_end_seconds: f64,
    pub cursor: ProgressGeometry,
    pub channels: Arc<[WaveformChannelRender]>,
}

impl WaveformRenderModel {
    pub fn from_source(
        source: &WaveformSource,
        viewport: WaveViewport,
        current_seconds: f64,
        options: WaveformRenderOptions,
    ) -> Result<Self, PeakExportError> {
        let scroll_window = viewport.scroll_window();
        let peaks = source.export_range_peaks(
            scroll_window.visible_start_seconds,
            scroll_window.visible_end_seconds,
            options.channels,
            options.peak_count,
            options.precision,
        )?;
        Self::from_render_parts(
            viewport.layout(),
            scroll_window,
            peaks,
            current_seconds,
            options,
        )
    }

    pub fn from_peak_cache(
        peak_cache: &mut PeakCache,
        viewport: WaveViewport,
        current_seconds: f64,
        options: WaveformRenderOptions,
    ) -> Result<Self, PeakExportError> {
        let scroll_window = viewport.scroll_window();
        let resolution_length =
            cached_render_peak_resolution(viewport.layout(), options.peak_count);
        let peaks = peak_cache.visible_peaks_from_cached_resolution(
            scroll_window.visible_start_seconds,
            scroll_window.visible_end_seconds,
            options.channels,
            options.peak_count,
            options.precision,
            resolution_length,
        )?;
        Self::from_render_parts(
            viewport.layout(),
            scroll_window,
            peaks,
            current_seconds,
            options,
        )
    }

    fn from_render_parts(
        layout: RendererLayout,
        scroll_window: ScrollWindow,
        peaks: Vec<Vec<f64>>,
        current_seconds: f64,
        options: WaveformRenderOptions,
    ) -> Result<Self, PeakExportError> {
        let visible_start_seconds = scroll_window.visible_start_seconds;
        let visible_end_seconds = scroll_window.visible_end_seconds;
        let visible_duration_seconds = (visible_end_seconds - visible_start_seconds).max(0.001);
        let cursor = ProgressGeometry::from_wavesurfer_rules(
            current_seconds - visible_start_seconds,
            visible_duration_seconds,
            options.cursor_width_px,
        );

        Ok(Self {
            layout,
            visible_start_seconds,
            visible_end_seconds,
            cursor,
            channels: peaks
                .into_iter()
                .enumerate()
                .map(|(channel_index, peaks)| {
                    WaveformChannelRender::from_window(
                        channel_index,
                        Arc::from(peaks),
                        visible_start_seconds,
                        visible_end_seconds,
                        options.peak_count,
                    )
                })
                .collect::<Vec<_>>()
                .into(),
        })
    }

    pub fn with_playhead_cursor(mut self, current_seconds: f64, cursor_width_px: f64) -> Self {
        let visible_duration_seconds =
            (self.visible_end_seconds - self.visible_start_seconds).max(0.001);
        self.cursor = ProgressGeometry::from_wavesurfer_rules(
            current_seconds - self.visible_start_seconds,
            visible_duration_seconds,
            cursor_width_px,
        );
        self
    }
}

fn cached_render_peak_resolution(layout: RendererLayout, peak_count: usize) -> usize {
    let wrapper_width = if layout.wrapper_width_px.is_finite() {
        layout.wrapper_width_px.ceil().max(peak_count as f64)
    } else {
        peak_count as f64
    };
    (wrapper_width as usize).clamp(peak_count.max(1), MAX_CACHED_RENDER_PEAKS)
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformChannelRender {
    pub channel_index: usize,
    pub peaks: Arc<[f64]>,
    pub visible_peaks: Arc<[f32]>,
}

impl WaveformChannelRender {
    fn from_window(
        channel_index: usize,
        peaks: Arc<[f64]>,
        visible_start_seconds: f64,
        visible_end_seconds: f64,
        visible_peak_count: usize,
    ) -> Self {
        let visible_peaks = sample_visible_peaks(
            &peaks,
            visible_start_seconds,
            visible_end_seconds,
            visible_start_seconds,
            visible_end_seconds,
            visible_peak_count,
        );

        Self {
            channel_index,
            peaks,
            visible_peaks,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveformHit {
    pub target: WaveformHitTarget,
    pub region_id: Option<String>,
    pub time_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveformHitTarget {
    Waveform,
    RegionBody,
    RegionStartHandle,
    RegionEndHandle,
}

fn sample_visible_peaks(
    peaks: &[f64],
    peak_start_seconds: f64,
    peak_end_seconds: f64,
    visible_start_seconds: f64,
    visible_end_seconds: f64,
    sample_count: usize,
) -> Arc<[f32]> {
    let sample_count = sample_count.max(1);
    let visible_duration = (visible_end_seconds - visible_start_seconds).max(f64::EPSILON);
    let sample_peak = |index: usize| {
        sample_visible_peak(
            peaks,
            peak_start_seconds,
            peak_end_seconds,
            visible_start_seconds,
            visible_duration,
            sample_count,
            index,
        )
    };
    let visible_peaks = if sample_count >= PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN {
        (0..sample_count)
            .into_par_iter()
            .map(sample_peak)
            .collect::<Vec<_>>()
    } else {
        (0..sample_count).map(sample_peak).collect::<Vec<_>>()
    };
    Arc::from(visible_peaks)
}

fn sample_visible_peak(
    peaks: &[f64],
    peak_start_seconds: f64,
    peak_end_seconds: f64,
    visible_start_seconds: f64,
    visible_duration: f64,
    sample_count: usize,
    index: usize,
) -> f32 {
    let start_ratio = index as f64 / sample_count as f64;
    let end_ratio = (index + 1) as f64 / sample_count as f64;
    let sample_start = visible_start_seconds + visible_duration * start_ratio;
    let sample_end = visible_start_seconds + visible_duration * end_ratio.min(1.0);
    peak_window_abs_max(
        peaks,
        peak_start_seconds,
        peak_end_seconds,
        sample_start,
        sample_end,
    )
}

fn peak_window_abs_max(
    peaks: &[f64],
    peak_start_seconds: f64,
    peak_end_seconds: f64,
    sample_start_seconds: f64,
    sample_end_seconds: f64,
) -> f32 {
    if peaks.is_empty() || peak_start_seconds >= peak_end_seconds {
        return 0.0;
    }

    let peak_duration = peak_end_seconds - peak_start_seconds;
    let start_ratio = ((sample_start_seconds - peak_start_seconds) / peak_duration).clamp(0.0, 1.0);
    let end_ratio = ((sample_end_seconds - peak_start_seconds) / peak_duration).clamp(0.0, 1.0);
    let start_index = (start_ratio * peaks.len() as f64).floor() as usize;
    let mut end_index = (end_ratio * peaks.len() as f64).ceil() as usize;
    let start_index = start_index.min(peaks.len());
    end_index = end_index.min(peaks.len());
    if end_index <= start_index && start_index < peaks.len() {
        end_index = start_index + 1;
    }

    peaks[start_index..end_index]
        .iter()
        .map(|peak| peak.abs().clamp(0.0, 1.0) as f32)
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    #[test]
    fn visible_peak_sampling_uses_parallel_iterator_for_large_windows() {
        let source = include_str!("render_model.rs");
        let imports_and_constants = source
            .split("#[derive(Clone, Copy, Debug, PartialEq)]")
            .next()
            .expect("render model options should follow imports and constants");
        let sampling_helper = source
            .split("fn sample_visible_peaks(")
            .nth(1)
            .expect("peak sampling helper should exist")
            .split("\nfn sample_visible_peak(")
            .next()
            .expect("single-peak sampler should follow peak sampling helper");

        assert!(imports_and_constants.contains("use rayon::prelude::*;"));
        assert!(imports_and_constants.contains("const PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN: usize"));
        assert!(sampling_helper.contains("sample_count >= PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN"));
        assert!(
            sampling_helper.contains("into_par_iter()"),
            "large visible windows reach PARALLEL_SAMPLE_VISIBLE_PEAKS_MIN and must fan bucket sampling across CPU cores"
        );
    }
}
