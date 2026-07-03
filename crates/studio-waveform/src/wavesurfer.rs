use crate::{DecodedAudio, ProgressGeometry, RendererLayout, RendererOptions, ScrollWindow};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WaveSurferOptions {
    pub min_px_per_sec: f64,
    pub fill_parent: bool,
    pub interact: bool,
    pub auto_scroll: bool,
    pub auto_center: bool,
}

impl Default for WaveSurferOptions {
    fn default() -> Self {
        Self {
            min_px_per_sec: 0.0,
            fill_parent: true,
            interact: true,
            auto_scroll: true,
            auto_center: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WaveSurferEvent {
    TimeUpdate(f64),
    Interaction(f64),
    Click { relative_x: f64, relative_y: f64 },
    DoubleClick { relative_x: f64, relative_y: f64 },
    DragStart(f64),
    Drag(f64),
    DragEnd(f64),
    Scroll(ScrollWindow),
    Resize(f64),
    Zoom(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct WaveSurfer {
    decoded: DecodedAudio,
    options: WaveSurferOptions,
    renderer_options: RendererOptions,
    scroll_px: f64,
    current_time_seconds: f64,
}

impl WaveSurfer {
    pub fn from_decoded(
        decoded: DecodedAudio,
        options: WaveSurferOptions,
        renderer_options: RendererOptions,
    ) -> Self {
        Self {
            decoded,
            options,
            renderer_options,
            scroll_px: 0.0,
            current_time_seconds: 0.0,
        }
    }

    pub fn options(&self) -> WaveSurferOptions {
        self.options
    }

    pub fn get_duration(&self) -> f64 {
        self.decoded.duration()
    }

    pub fn get_width(&self) -> f64 {
        self.renderer_options.parent_width_px.max(0.0)
    }

    pub fn get_scroll(&self) -> f64 {
        self.scroll_px
    }

    pub fn get_current_time(&self) -> f64 {
        self.current_time_seconds
    }

    pub fn get_decoded_data(&self) -> &DecodedAudio {
        &self.decoded
    }

    pub fn set_scroll(&mut self, scroll_px: f64) {
        self.scroll_px = if scroll_px.is_finite() {
            scroll_px.clamp(0.0, self.max_scroll_px())
        } else {
            0.0
        };
    }

    pub fn set_scroll_time(&mut self, time_seconds: f64) -> ScrollWindow {
        let duration = self.get_duration();
        let percentage = if duration > 0.0 {
            finite_non_negative(time_seconds) / duration
        } else {
            0.0
        };
        let scroll_start = self.layout().wrapper_width_px * percentage;
        self.set_scroll(scroll_start);
        self.scroll_window()
    }

    pub fn zoom(&mut self, min_px_per_sec: f64) -> WaveSurferEvent {
        self.options.min_px_per_sec = if min_px_per_sec.is_finite() {
            min_px_per_sec.max(0.0)
        } else {
            0.0
        };
        self.set_scroll(self.scroll_px);
        WaveSurferEvent::Zoom(self.options.min_px_per_sec)
    }

    pub fn resize_parent_width(&mut self, parent_width_px: f64) -> WaveSurferEvent {
        self.renderer_options.parent_width_px = finite_non_negative(parent_width_px);
        self.set_scroll(self.scroll_px);
        WaveSurferEvent::Resize(self.renderer_options.parent_width_px)
    }

    pub fn toggle_interaction(&mut self, is_interactive: bool) {
        self.options.interact = is_interactive;
    }

    pub fn set_time(&mut self, time_seconds: f64) -> WaveSurferEvent {
        self.current_time_seconds =
            finite_non_negative(time_seconds).clamp(0.0, self.get_duration());
        WaveSurferEvent::TimeUpdate(self.current_time_seconds)
    }

    pub fn update_progress(&mut self, time_seconds: f64, is_playing: bool) -> Vec<WaveSurferEvent> {
        let time_update = self.set_time(time_seconds);
        let mut events = vec![time_update];
        if let Some(scroll_window) = self.render_progress(self.current_progress(), is_playing) {
            events.push(WaveSurferEvent::Scroll(scroll_window));
        }
        events
    }

    pub fn render_progress(&mut self, progress: f64, is_playing: bool) -> Option<ScrollWindow> {
        if !self.layout().is_scrollable || !self.options.auto_scroll || self.get_duration() <= 0.0 {
            return None;
        }

        self.scroll_into_view(clamp_to_unit(progress), is_playing)
    }

    pub fn seek_to(&mut self, progress: f64) -> WaveSurferEvent {
        let progress = if progress.is_finite() { progress } else { 0.0 };
        self.set_time(self.get_duration() * progress)
    }

    pub fn click(&mut self, relative_x: f64, relative_y: f64) -> Vec<WaveSurferEvent> {
        if !self.options.interact {
            return Vec::new();
        }

        let time_update = self.seek_to(relative_x);
        vec![
            time_update,
            WaveSurferEvent::Interaction(finite_non_negative(relative_x) * self.get_duration()),
            WaveSurferEvent::Click {
                relative_x,
                relative_y,
            },
        ]
    }

    pub fn double_click(&self, relative_x: f64, relative_y: f64) -> WaveSurferEvent {
        WaveSurferEvent::DoubleClick {
            relative_x,
            relative_y,
        }
    }

    pub fn drag_start(&self, relative_x: f64) -> WaveSurferEvent {
        WaveSurferEvent::DragStart(clamp_to_unit(relative_x))
    }

    pub fn drag(&mut self, relative_x: f64) -> Vec<WaveSurferEvent> {
        if !self.options.interact {
            return Vec::new();
        }

        let relative_x = clamp_to_unit(relative_x);
        let time_update = self.seek_to(relative_x);
        vec![
            time_update,
            WaveSurferEvent::Interaction(relative_x * self.get_duration()),
            WaveSurferEvent::Drag(relative_x),
        ]
    }

    pub fn drag_end(&self, relative_x: f64) -> WaveSurferEvent {
        WaveSurferEvent::DragEnd(clamp_to_unit(relative_x))
    }

    pub fn layout(&self) -> RendererLayout {
        RendererLayout::from_wavesurfer_rules(
            self.get_duration(),
            self.options.min_px_per_sec,
            self.renderer_options.parent_width_px,
            self.options.fill_parent,
            self.renderer_options.pixel_ratio,
        )
    }

    pub fn progress_geometry(
        &self,
        current_seconds: f64,
        cursor_width_px: f64,
    ) -> ProgressGeometry {
        ProgressGeometry::from_wavesurfer_rules(
            current_seconds,
            self.get_duration(),
            cursor_width_px,
        )
    }

    pub fn current_progress_geometry(&self, cursor_width_px: f64) -> ProgressGeometry {
        self.progress_geometry(self.current_time_seconds, cursor_width_px)
    }

    pub fn scroll_window(&self) -> ScrollWindow {
        ScrollWindow::from_wavesurfer_rules(
            self.scroll_px,
            self.get_width(),
            self.layout().wrapper_width_px,
            self.get_duration(),
        )
    }

    pub fn current_progress(&self) -> f64 {
        let duration = self.get_duration();
        if duration > 0.0 {
            (self.current_time_seconds / duration).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub fn seconds_to_content_px(&self, seconds: f64) -> f64 {
        let layout = self.layout();
        if layout.wrapper_width_px <= 0.0 {
            return 0.0;
        }

        let seconds = if seconds.is_finite() {
            seconds.clamp(0.0, self.get_duration())
        } else {
            0.0
        };
        seconds / self.get_duration() * layout.wrapper_width_px
    }

    pub fn seconds_to_viewport_px(&self, seconds: f64) -> f64 {
        self.seconds_to_content_px(seconds) - self.scroll_px
    }

    pub fn viewport_px_to_seconds(&self, viewport_px: f64) -> f64 {
        let layout = self.layout();
        if layout.wrapper_width_px <= 0.0 {
            return 0.0;
        }

        let viewport_px = if viewport_px.is_finite() {
            viewport_px
        } else {
            0.0
        };
        let content_px = (self.scroll_px + viewport_px).clamp(0.0, layout.wrapper_width_px);
        content_px / layout.wrapper_width_px * self.get_duration()
    }

    pub fn render_window_samples(
        &self,
        offset_px: f64,
        clamped_width_px: f64,
        channels: usize,
    ) -> Vec<Vec<f64>> {
        if channels == 0 {
            return Vec::new();
        }

        let layout = self.layout();
        let total_width = layout.wrapper_width_px;
        if total_width <= 0.0 {
            return Vec::new();
        }

        let channel_count = channels.min(self.decoded.number_of_channels());
        (0..channel_count)
            .filter_map(|channel_index| {
                let channel = self.decoded.channel_data(channel_index)?;
                let start = ((finite_non_negative(offset_px) / total_width) * channel.len() as f64)
                    .floor() as usize;
                let end = (((finite_non_negative(offset_px)
                    + finite_non_negative(clamped_width_px))
                    / total_width)
                    * channel.len() as f64)
                    .floor() as usize;
                let start = start.min(channel.len());
                let end = end.min(channel.len());
                Some(
                    channel[start..end]
                        .iter()
                        .copied()
                        .map(sample_to_render_f64)
                        .collect(),
                )
            })
            .collect()
    }

    fn max_scroll_px(&self) -> f64 {
        (self.layout().wrapper_width_px - self.get_width()).max(0.0)
    }

    fn scroll_into_view(&mut self, progress: f64, is_playing: bool) -> Option<ScrollWindow> {
        let layout = self.layout();
        let scroll_width = layout.wrapper_width_px;
        let client_width = self.get_width();
        if scroll_width <= 0.0 || client_width <= 0.0 {
            return None;
        }

        let scroll_left = self.scroll_px;
        let cursor_px = progress * scroll_width;
        let scroll_right = scroll_left + client_width;
        let half_width = client_width / 2.0;
        let mut next_scroll = scroll_left;

        if cursor_px < scroll_left || cursor_px > scroll_right {
            next_scroll = cursor_px
                - if self.options.auto_center {
                    half_width
                } else {
                    0.0
                };
        }

        let after_center_delta = cursor_px - scroll_left - half_width;
        if is_playing && self.options.auto_center && after_center_delta > 0.0 {
            next_scroll += after_center_delta;
        }

        self.set_scroll(next_scroll);
        if (self.scroll_px - scroll_left).abs() > f64::EPSILON {
            Some(self.scroll_window())
        } else {
            None
        }
    }
}

fn sample_to_render_f64(sample: f32) -> f64 {
    const RENDER_SAMPLE_PRECISION: f64 = 1_000_000.0;
    let sample = f64::from(sample);
    (sample * RENDER_SAMPLE_PRECISION).round() / RENDER_SAMPLE_PRECISION
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn clamp_to_unit(value: f64) -> f64 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
