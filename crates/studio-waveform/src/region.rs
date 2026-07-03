#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub id: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub total_duration_seconds: f64,
    pub drag: bool,
    pub resize: bool,
    pub resize_start: bool,
    pub resize_end: bool,
    pub color: String,
    pub content: Option<String>,
    pub min_length_seconds: f64,
    pub max_length_seconds: f64,
    pub channel_index: Option<usize>,
    pub number_of_channels: usize,
    updating_side: Option<RegionUpdateSide>,
}

impl Region {
    pub fn new(
        id: impl Into<String>,
        start_seconds: f64,
        end_seconds: f64,
        total_duration_seconds: f64,
    ) -> Result<Self, String> {
        if !total_duration_seconds.is_finite() || total_duration_seconds <= 0.0 {
            return Err("region total duration must be greater than zero".to_string());
        }

        let start_seconds = clamp_position(start_seconds, total_duration_seconds);
        let end_seconds = clamp_position(end_seconds, total_duration_seconds);
        let (start_seconds, end_seconds) = if start_seconds <= end_seconds {
            (start_seconds, end_seconds)
        } else {
            (end_seconds, start_seconds)
        };

        Ok(Self::from_parts(
            id.into(),
            start_seconds,
            end_seconds,
            total_duration_seconds,
            None,
            0,
            RegionParams::default_for_internal(),
        ))
    }

    pub fn from_params(
        params: RegionParams,
        total_duration_seconds: f64,
        number_of_channels: usize,
    ) -> Result<Self, String> {
        if !total_duration_seconds.is_finite() || total_duration_seconds <= 0.0 {
            return Err("region total duration must be greater than zero".to_string());
        }

        let start_seconds = clamp_position(params.start_seconds, total_duration_seconds);
        let end_seconds = clamp_position(
            params.end_seconds.unwrap_or(params.start_seconds),
            total_duration_seconds,
        );
        Ok(Self::from_parts(
            params.id.clone(),
            start_seconds,
            end_seconds,
            total_duration_seconds,
            params.channel_index,
            number_of_channels,
            params,
        ))
    }

    pub fn geometry(&self) -> RegionGeometry {
        let left_percent = self.start_seconds / self.total_duration_seconds * 100.0;
        let right_percent =
            (self.total_duration_seconds - self.end_seconds) / self.total_duration_seconds * 100.0;
        let (top_percent, height_percent) = match (self.channel_index, self.number_of_channels) {
            (Some(channel_index), channels) if channels > 0 && channel_index < channels => {
                let height = 100.0 / channels as f64;
                (height * channel_index as f64, height)
            }
            _ => (0.0, 100.0),
        };

        RegionGeometry {
            left_percent,
            right_percent,
            top_percent,
            height_percent,
            is_marker: self.start_seconds == self.end_seconds,
        }
    }

    pub fn update_by_pixels(
        &mut self,
        delta_px: f64,
        parent_width_px: f64,
        side: Option<RegionUpdateSide>,
        start_time_seconds: Option<f64>,
    ) -> Option<RegionUpdate> {
        if parent_width_px <= 0.0 || !parent_width_px.is_finite() {
            return None;
        }
        if side.is_none() && !self.drag {
            return None;
        }
        if side.is_some() && !self.resize {
            return None;
        }
        if side == Some(RegionUpdateSide::Start) && !self.resize_start {
            return None;
        }
        if side == Some(RegionUpdateSide::End) && !self.resize_end {
            return None;
        }

        let delta_seconds =
            finite_or_zero(delta_px) / parent_width_px * self.total_duration_seconds;
        let mut start = if side.is_some() && side != Some(RegionUpdateSide::Start) {
            self.start_seconds
        } else {
            self.start_seconds + delta_seconds
        };
        let mut end = if side.is_some() && side != Some(RegionUpdateSide::End) {
            self.end_seconds
        } else {
            self.end_seconds + delta_seconds
        };
        let has_start_time = start_time_seconds.is_some();
        if has_start_time && self.updating_side.is_some() && self.updating_side != side {
            if self.updating_side == Some(RegionUpdateSide::Start) {
                start = start_time_seconds.unwrap_or(start);
            } else {
                end = start_time_seconds.unwrap_or(end);
            }
        }

        start = start.clamp(0.0, self.total_duration_seconds);
        end = end.clamp(0.0, self.total_duration_seconds);

        let duration = end - start;
        self.updating_side = side;
        let length_is_valid =
            duration >= self.min_length_seconds && duration <= self.max_length_seconds;
        if start <= end && (length_is_valid || has_start_time) {
            self.start_seconds = start;
            self.end_seconds = end;
            Some(RegionUpdate {
                id: self.id.clone(),
                side,
                start_seconds: self.start_seconds,
                end_seconds: self.end_seconds,
            })
        } else {
            None
        }
    }

    pub fn end_update(&mut self) {
        self.updating_side = None;
    }

    fn from_parts(
        id: String,
        start_seconds: f64,
        end_seconds: f64,
        total_duration_seconds: f64,
        channel_index: Option<usize>,
        number_of_channels: usize,
        params: RegionParams,
    ) -> Self {
        let (start_seconds, end_seconds) = if start_seconds <= end_seconds {
            (start_seconds, end_seconds)
        } else {
            (end_seconds, start_seconds)
        };

        Self {
            id,
            start_seconds,
            end_seconds,
            total_duration_seconds,
            drag: params.drag,
            resize: params.resize,
            resize_start: params.resize_start,
            resize_end: params.resize_end,
            color: params.color,
            content: params.content,
            min_length_seconds: params.min_length_seconds,
            max_length_seconds: params.max_length_seconds,
            channel_index,
            number_of_channels,
            updating_side: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionGeometry {
    pub left_percent: f64,
    pub right_percent: f64,
    pub top_percent: f64,
    pub height_percent: f64,
    pub is_marker: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionParams {
    pub id: String,
    pub start_seconds: f64,
    pub end_seconds: Option<f64>,
    pub drag: bool,
    pub resize: bool,
    pub resize_start: bool,
    pub resize_end: bool,
    pub color: String,
    pub content: Option<String>,
    pub min_length_seconds: f64,
    pub max_length_seconds: f64,
    pub channel_index: Option<usize>,
}

impl RegionParams {
    pub fn new(id: impl Into<String>, start_seconds: f64, end_seconds: Option<f64>) -> Self {
        Self {
            id: id.into(),
            start_seconds,
            end_seconds,
            drag: true,
            resize: true,
            resize_start: true,
            resize_end: true,
            color: "rgba(0, 0, 0, 0.1)".to_string(),
            content: None,
            min_length_seconds: 0.0,
            max_length_seconds: f64::INFINITY,
            channel_index: None,
        }
    }

    pub fn with_min_length(mut self, min_length_seconds: f64) -> Self {
        self.min_length_seconds = finite_or_zero(min_length_seconds).max(0.0);
        self
    }

    pub fn with_max_length(mut self, max_length_seconds: f64) -> Self {
        self.max_length_seconds = if max_length_seconds.is_finite() && max_length_seconds >= 0.0 {
            max_length_seconds
        } else {
            f64::INFINITY
        };
        self
    }

    pub fn with_channel_index(mut self, channel_index: usize) -> Self {
        self.channel_index = Some(channel_index);
        self
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }

    fn default_for_internal() -> Self {
        Self::new("", 0.0, Some(0.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionUpdateSide {
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionUpdate {
    pub id: String,
    pub side: Option<RegionUpdateSide>,
    pub start_seconds: f64,
    pub end_seconds: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RegionVirtualVisibility {
    pub start_px: f64,
    pub end_px: f64,
    pub is_visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegionsPlugin {
    total_duration_seconds: f64,
    number_of_channels: usize,
    regions: Vec<Region>,
}

impl RegionsPlugin {
    pub fn new(total_duration_seconds: f64, number_of_channels: usize) -> Result<Self, String> {
        if !total_duration_seconds.is_finite() || total_duration_seconds <= 0.0 {
            return Err("regions total duration must be greater than zero".to_string());
        }

        Ok(Self {
            total_duration_seconds,
            number_of_channels,
            regions: Vec::new(),
        })
    }

    pub fn get_regions(&self) -> &[Region] {
        &self.regions
    }

    pub fn region(&self, id: &str) -> Option<&Region> {
        self.regions.iter().find(|region| region.id == id)
    }

    pub fn region_mut(&mut self, id: &str) -> Option<&mut Region> {
        self.regions.iter_mut().find(|region| region.id == id)
    }

    pub fn add_region(&mut self, params: RegionParams) -> Result<Region, String> {
        let region =
            Region::from_params(params, self.total_duration_seconds, self.number_of_channels)?;
        self.regions.push(region.clone());
        Ok(region)
    }

    pub fn update_region_by_pixels(
        &mut self,
        id: &str,
        delta_px: f64,
        parent_width_px: f64,
        side: Option<RegionUpdateSide>,
        start_time_seconds: Option<f64>,
    ) -> Option<RegionUpdate> {
        self.region_mut(id)?
            .update_by_pixels(delta_px, parent_width_px, side, start_time_seconds)
    }

    pub fn remove_region(&mut self, id: &str) -> Option<Region> {
        let index = self.regions.iter().position(|region| region.id == id)?;
        Some(self.regions.remove(index))
    }

    pub fn clear_regions(&mut self) {
        self.regions.clear();
    }

    pub fn regions_at_time(&self, seconds: f64) -> Vec<&Region> {
        self.regions
            .iter()
            .filter(|region| {
                let region_end = if region.end_seconds == region.start_seconds {
                    region.start_seconds + 0.05
                } else {
                    region.end_seconds
                };
                region.start_seconds <= seconds && region_end >= seconds
            })
            .collect()
    }

    pub fn virtual_visibility(
        &self,
        id: &str,
        scroll_px: f64,
        viewport_width_px: f64,
        container_width_px: f64,
    ) -> Option<RegionVirtualVisibility> {
        let region = self.region(id)?;
        let (start_px, end_px) = self.region_virtual_pixels(region, container_width_px)?;
        let scroll_px = finite_non_negative(scroll_px);
        let viewport_width_px = finite_non_negative(viewport_width_px);

        Some(RegionVirtualVisibility {
            start_px,
            end_px,
            is_visible: end_px > scroll_px && start_px < scroll_px + viewport_width_px,
        })
    }

    pub fn adjust_scroll_for_region(
        &self,
        id: &str,
        scroll_px: f64,
        viewport_width_px: f64,
        container_width_px: f64,
    ) -> Option<f64> {
        let region = self.region(id)?;
        let (left_px, right_px) = self.region_virtual_pixels(region, container_width_px)?;
        let viewport_width_px = finite_non_negative(viewport_width_px);
        let container_width_px = finite_non_negative(container_width_px);
        let max_scroll = (container_width_px - viewport_width_px).max(0.0);
        let mut next_scroll = finite_non_negative(scroll_px).clamp(0.0, max_scroll);

        let relative_left = left_px - next_scroll;
        let relative_right = right_px - next_scroll;
        if relative_left < 0.0 {
            next_scroll += relative_left;
        } else if relative_right > viewport_width_px {
            next_scroll += relative_right - viewport_width_px;
        }

        Some(next_scroll.clamp(0.0, max_scroll))
    }

    fn region_virtual_pixels(
        &self,
        region: &Region,
        container_width_px: f64,
    ) -> Option<(f64, f64)> {
        if self.total_duration_seconds <= 0.0 {
            return None;
        }

        let container_width_px = finite_non_negative(container_width_px);
        let start_px = (region.start_seconds / self.total_duration_seconds * container_width_px)
            .round()
            .clamp(0.0, container_width_px);
        let width_px = ((region.end_seconds - region.start_seconds) / self.total_duration_seconds
            * container_width_px)
            .round()
            .max(1.0);
        Some((
            start_px,
            (start_px + width_px).clamp(start_px, container_width_px),
        ))
    }
}

fn clamp_position(position: f64, total_duration_seconds: f64) -> f64 {
    if position.is_finite() {
        position.clamp(0.0, total_duration_seconds)
    } else {
        0.0
    }
}

fn finite_or_zero(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}
