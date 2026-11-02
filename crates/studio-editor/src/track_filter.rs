#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackFilter {
    All,
    Lane { id: String, label: String },
}

impl TrackFilter {
    pub fn lane(id: impl Into<String>, label: impl Into<String>) -> Self {
        TrackFilter::Lane {
            id: id.into(),
            label: label.into(),
        }
    }

    pub fn label(&self) -> String {
        match self {
            TrackFilter::All => "All Tracks".to_string(),
            TrackFilter::Lane { label, .. } => label.clone(),
        }
    }

    pub fn matches_lane(&self, lane_id: &str) -> bool {
        match self {
            TrackFilter::All => true,
            TrackFilter::Lane { id, .. } => id == lane_id,
        }
    }
}

impl std::fmt::Display for TrackFilter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label())
    }
}
