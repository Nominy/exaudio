use crate::{RendererLayout, ScrollWindow};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveViewport {
    duration_seconds: f64,
    parent_width_px: f64,
    min_px_per_sec: f64,
    fill_parent: bool,
    pixel_ratio: f64,
}

impl WaveViewport {
    pub fn new(
        duration_seconds: f64,
        parent_width_px: f64,
        min_px_per_sec: f64,
        fill_parent: bool,
        pixel_ratio: f64,
    ) -> Self {
        Self {
            duration_seconds: finite_non_negative(duration_seconds),
            parent_width_px: finite_non_negative(parent_width_px),
            min_px_per_sec: finite_non_negative(min_px_per_sec),
            fill_parent,
            pixel_ratio: finite_positive(pixel_ratio),
        }
    }

    pub fn layout(&self) -> RendererLayout {
        RendererLayout::from_wavesurfer_rules(
            self.duration_seconds,
            self.min_px_per_sec,
            self.parent_width_px,
            self.fill_parent,
            self.pixel_ratio,
        )
    }

    /// The window at the start of the content: the viewport does not scroll.
    pub fn scroll_window(&self) -> ScrollWindow {
        ScrollWindow::from_wavesurfer_rules(
            0.0,
            self.parent_width_px,
            self.layout().wrapper_width_px,
            self.duration_seconds,
        )
    }
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn finite_positive(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}
