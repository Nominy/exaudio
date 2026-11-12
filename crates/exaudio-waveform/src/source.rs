use std::sync::Arc;

use crate::{DecodedAudio, PeakExportError, export_peaks, export_range_peaks, peaks::bucket_max};

#[derive(Clone, Debug, PartialEq)]
pub enum WaveformSource {
    Decoded(Arc<DecodedAudio>),
    Peaks(Arc<PrecomputedPeaks>),
}

impl WaveformSource {
    pub fn from_decoded(decoded: DecodedAudio) -> Self {
        Self::Decoded(Arc::new(decoded))
    }

    pub fn from_precomputed_peaks(
        peaks: Vec<Vec<f64>>,
        duration_seconds: f64,
    ) -> Result<Self, String> {
        Ok(Self::Peaks(Arc::new(PrecomputedPeaks::new(
            peaks,
            duration_seconds,
        )?)))
    }

    pub fn duration(&self) -> f64 {
        match self {
            Self::Decoded(decoded) => decoded.duration(),
            Self::Peaks(peaks) => peaks.duration_seconds,
        }
    }

    pub fn number_of_channels(&self) -> usize {
        match self {
            Self::Decoded(decoded) => decoded.number_of_channels(),
            Self::Peaks(peaks) => peaks.channels.len(),
        }
    }

    pub fn export_peaks(
        &self,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        match self {
            Self::Decoded(decoded) => export_peaks(decoded, channels, max_length, precision),
            Self::Peaks(peaks) => peaks.export_peaks(channels, max_length, precision),
        }
    }

    pub fn export_range_peaks(
        &self,
        start_seconds: f64,
        end_seconds: f64,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        match self {
            Self::Decoded(decoded) => export_range_peaks(
                decoded,
                start_seconds,
                end_seconds,
                channels,
                max_length,
                precision,
            ),
            Self::Peaks(peaks) => peaks.export_range_peaks(
                start_seconds,
                end_seconds,
                channels,
                max_length,
                precision,
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PrecomputedPeaks {
    channels: Vec<Vec<f64>>,
    duration_seconds: f64,
}

impl PrecomputedPeaks {
    pub fn new(channels: Vec<Vec<f64>>, duration_seconds: f64) -> Result<Self, String> {
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err("precomputed peak duration must be greater than zero".to_string());
        }
        if channels.is_empty() {
            return Err("precomputed peaks must contain at least one channel".to_string());
        }

        let peak_count = channels[0].len();
        if peak_count == 0 {
            return Err("precomputed peak channels cannot be empty".to_string());
        }
        if channels.iter().any(|channel| channel.len() != peak_count) {
            return Err("precomputed peak channels must have the same length".to_string());
        }

        Ok(Self {
            channels,
            duration_seconds,
        })
    }

    fn export_peaks(
        &self,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        if channels == 0 {
            return Err(PeakExportError::NoChannelsRequested);
        }
        if max_length == 0 {
            return Err(PeakExportError::EmptyExport);
        }
        if !precision.is_finite() || precision <= 0.0 {
            return Err(PeakExportError::InvalidPrecision);
        }

        Ok(self
            .channels
            .iter()
            .take(channels.min(self.channels.len()))
            .map(|channel| bucket_max(channel, max_length, precision))
            .collect())
    }

    fn export_range_peaks(
        &self,
        start_seconds: f64,
        end_seconds: f64,
        channels: usize,
        max_length: usize,
        precision: f64,
    ) -> Result<Vec<Vec<f64>>, PeakExportError> {
        if !start_seconds.is_finite() || !end_seconds.is_finite() || start_seconds >= end_seconds {
            return Err(PeakExportError::InvalidRange);
        }
        let start_seconds = start_seconds.clamp(0.0, self.duration_seconds);
        let end_seconds = end_seconds.clamp(0.0, self.duration_seconds);
        if start_seconds >= end_seconds {
            return Err(PeakExportError::InvalidRange);
        }

        let start_index = (start_seconds / self.duration_seconds * self.channels[0].len() as f64)
            .floor() as usize;
        let end_index =
            (end_seconds / self.duration_seconds * self.channels[0].len() as f64).ceil() as usize;
        Ok(self
            .channels
            .iter()
            .take(channels.min(self.channels.len()))
            .map(|channel| {
                let start_index = start_index.min(channel.len());
                let end_index = end_index.min(channel.len());
                bucket_max(&channel[start_index..end_index], max_length, precision)
            })
            .collect())
    }
}
