use crate::model::Timeline;
use serde::{Deserialize, Serialize};

/// Snapshot of the timeline taken before an edit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edit {
    pub timeline: Timeline,
}

#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    undo: Vec<Timeline>,
    redo: Vec<Timeline>,
}

impl UndoStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn checkpoint(&mut self, before: Timeline) {
        self.undo.push(before);
        self.redo.clear();
    }

    pub fn undo(&mut self, timeline: &mut Timeline) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(timeline.clone());
        *timeline = prev;
        true
    }

    pub fn redo(&mut self, timeline: &mut Timeline) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(timeline.clone());
        *timeline = next;
        true
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}
