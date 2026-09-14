use crate::ids::ProjectId;
use crate::model::Timeline;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub timeline: Timeline,
}

impl Project {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: ProjectId::new(),
            name: name.into(),
            timeline: Timeline::default(),
        }
    }
}
