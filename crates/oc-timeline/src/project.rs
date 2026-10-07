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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_project_starts_on_a_landscape_timeline() {
        let project = Project::new("Reel");
        assert_eq!(project.name, "Reel");
        assert_eq!(project.timeline.width, 1920);
        assert_eq!(project.timeline.height, 1080);
        assert_eq!(project.timeline.tracks.len(), 3);
    }
}
