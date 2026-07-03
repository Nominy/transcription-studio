use crate::{RendererLayout, ScrollWindow};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveViewport {
    duration_seconds: f64,
    parent_width_px: f64,
    min_px_per_sec: f64,
    fill_parent: bool,
    pixel_ratio: f64,
    scroll_px: f64,
}

impl WaveViewport {
    pub fn new(
        duration_seconds: f64,
        parent_width_px: f64,
        min_px_per_sec: f64,
        fill_parent: bool,
        pixel_ratio: f64,
    ) -> Self {
        let mut viewport = Self {
            duration_seconds: finite_non_negative(duration_seconds),
            parent_width_px: finite_non_negative(parent_width_px),
            min_px_per_sec: finite_non_negative(min_px_per_sec),
            fill_parent,
            pixel_ratio: finite_positive(pixel_ratio),
            scroll_px: 0.0,
        };
        viewport.set_scroll(0.0);
        viewport
    }

    pub fn min_px_per_sec(&self) -> f64 {
        self.min_px_per_sec
    }

    pub fn get_scroll(&self) -> f64 {
        self.scroll_px
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

    pub fn scroll_window(&self) -> ScrollWindow {
        ScrollWindow::from_wavesurfer_rules(
            self.scroll_px,
            self.parent_width_px,
            self.layout().wrapper_width_px,
            self.duration_seconds,
        )
    }

    pub fn set_scroll(&mut self, scroll_px: f64) {
        self.scroll_px = finite_non_negative(scroll_px).clamp(0.0, self.max_scroll_px());
    }

    pub fn set_scroll_time(&mut self, time_seconds: f64) -> ScrollWindow {
        let percentage = if self.duration_seconds > 0.0 {
            finite_non_negative(time_seconds) / self.duration_seconds
        } else {
            0.0
        };
        self.set_scroll(self.layout().wrapper_width_px * percentage);
        self.scroll_window()
    }

    pub fn zoom_at(&mut self, anchor_viewport_px: f64, next_min_px_per_sec: f64) {
        let anchor_viewport_px = finite_non_negative(anchor_viewport_px);
        let anchor_seconds = self.viewport_px_to_seconds(anchor_viewport_px);
        self.min_px_per_sec = finite_non_negative(next_min_px_per_sec);
        let content_px = self.seconds_to_content_px(anchor_seconds);
        self.set_scroll(content_px - anchor_viewport_px);
    }

    pub fn seconds_to_content_px(&self, seconds: f64) -> f64 {
        let width = self.layout().wrapper_width_px;
        if width <= 0.0 || self.duration_seconds <= 0.0 {
            return 0.0;
        }

        finite_non_negative(seconds).clamp(0.0, self.duration_seconds) / self.duration_seconds
            * width
    }

    pub fn seconds_to_viewport_px(&self, seconds: f64) -> f64 {
        self.seconds_to_content_px(seconds) - self.scroll_px
    }

    pub fn viewport_px_to_seconds(&self, viewport_px: f64) -> f64 {
        let width = self.layout().wrapper_width_px;
        if width <= 0.0 {
            return 0.0;
        }

        let content_px = (self.scroll_px + finite_non_negative(viewport_px)).clamp(0.0, width);
        content_px / width * self.duration_seconds
    }

    fn max_scroll_px(&self) -> f64 {
        (self.layout().wrapper_width_px - self.parent_width_px).max(0.0)
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
