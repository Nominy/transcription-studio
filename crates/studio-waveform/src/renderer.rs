#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererOptions {
    pub parent_width_px: f64,
    pub pixel_ratio: f64,
}

impl Default for RendererOptions {
    fn default() -> Self {
        Self {
            parent_width_px: 1.0,
            pixel_ratio: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RendererLayout {
    pub scroll_width_px: f64,
    pub wrapper_width_px: f64,
    pub is_scrollable: bool,
    pub uses_parent_width: bool,
}

impl RendererLayout {
    pub(crate) fn from_wavesurfer_rules(
        duration_seconds: f64,
        min_px_per_sec: f64,
        parent_width_px: f64,
        fill_parent: bool,
        pixel_ratio: f64,
    ) -> Self {
        let duration_seconds = finite_non_negative(duration_seconds);
        let min_px_per_sec = finite_non_negative(min_px_per_sec);
        let parent_width_px = finite_non_negative(parent_width_px);
        let pixel_ratio = finite_positive(pixel_ratio);

        let scroll_width_px = (duration_seconds * min_px_per_sec).ceil();
        let is_scrollable = scroll_width_px > parent_width_px;
        let uses_parent_width = fill_parent && !is_scrollable;
        let wrapper_width_px = if uses_parent_width {
            parent_width_px
        } else {
            scroll_width_px
        } * pixel_ratio;

        Self {
            scroll_width_px,
            wrapper_width_px,
            is_scrollable,
            uses_parent_width,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressGeometry {
    pub progress_percent: f64,
    pub cursor_left_percent: f64,
    pub cursor_translate_x_px: f64,
}

impl ProgressGeometry {
    pub(crate) fn from_wavesurfer_rules(
        current_seconds: f64,
        duration_seconds: f64,
        cursor_width_px: f64,
    ) -> Self {
        let duration_seconds = finite_positive(duration_seconds);
        let ratio = (finite_non_negative(current_seconds) / duration_seconds).clamp(0.0, 1.0);
        let progress_percent = ratio * 100.0;

        Self {
            progress_percent,
            cursor_left_percent: progress_percent,
            cursor_translate_x_px: -ratio * finite_non_negative(cursor_width_px),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollWindow {
    pub visible_start_seconds: f64,
    pub visible_end_seconds: f64,
    pub scroll_left_px: f64,
    pub scroll_right_px: f64,
}

impl ScrollWindow {
    pub(crate) fn from_wavesurfer_rules(
        scroll_left_px: f64,
        client_width_px: f64,
        scroll_width_px: f64,
        duration_seconds: f64,
    ) -> Self {
        let scroll_width_px = finite_positive(scroll_width_px);
        let duration_seconds = finite_non_negative(duration_seconds);
        let scroll_left_px = finite_non_negative(scroll_left_px);
        let client_width_px = finite_non_negative(client_width_px);
        let scroll_right_px = scroll_left_px + client_width_px;
        let visible_start_seconds =
            (scroll_left_px / scroll_width_px * duration_seconds).clamp(0.0, duration_seconds);
        let visible_end_seconds = (scroll_right_px / scroll_width_px * duration_seconds)
            .clamp(visible_start_seconds, duration_seconds);

        Self {
            visible_start_seconds,
            visible_end_seconds,
            scroll_left_px,
            scroll_right_px,
        }
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
