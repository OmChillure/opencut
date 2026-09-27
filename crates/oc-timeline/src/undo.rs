use crate::model::Timeline;
use serde::{Deserialize, Serialize};

/// One step in the Undo History. The snapshot is the timeline *before* the named change.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UndoEntry {
    pub label: String,
    pub timeline: Timeline,
}

/// Snapshot of the timeline taken before an edit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Edit {
    pub timeline: Timeline,
}

/// Named command stack, same idea as Kdenlive's Undo History.
/// Capped so a long session does not keep every timeline forever.
const CAP: usize = 40;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UndoStack {
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
}

impl UndoStack {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn checkpoint(&mut self, before: Timeline) {
        self.checkpoint_named(before, "Edit");
    }

    pub fn checkpoint_named(&mut self, before: Timeline, label: impl Into<String>) {
        self.undo.push(UndoEntry {
            label: label.into(),
            timeline: before,
        });
        if self.undo.len() > CAP {
            let drop_n = self.undo.len() - CAP;
            self.undo.drain(0..drop_n);
        }
        self.redo.clear();
    }

    /// Set the label of the change that was just checkpointed.
    pub fn label_last(&mut self, label: impl Into<String>) {
        if let Some(entry) = self.undo.last_mut() {
            entry.label = label.into();
        }
    }

    pub fn undo(&mut self, timeline: &mut Timeline) -> bool {
        let Some(prev) = self.undo.pop() else {
            return false;
        };
        self.redo.push(UndoEntry {
            label: prev.label.clone(),
            timeline: timeline.clone(),
        });
        *timeline = prev.timeline;
        true
    }

    pub fn redo(&mut self, timeline: &mut Timeline) -> bool {
        let Some(next) = self.redo.pop() else {
            return false;
        };
        self.undo.push(UndoEntry {
            label: next.label.clone(),
            timeline: timeline.clone(),
        });
        *timeline = next.timeline;
        true
    }

    /// Jump to the state after `keep` undone steps. `keep == 0` is the oldest snapshot.
    /// `keep == depth()` is the current timeline.
    pub fn jump(&mut self, timeline: &mut Timeline, keep: usize) -> bool {
        let keep = keep.min(self.undo.len() + self.redo.len());
        let mut changed = false;
        while self.undo.len() > keep {
            if !self.undo(timeline) {
                break;
            }
            changed = true;
        }
        while self.undo.len() < keep {
            if !self.redo(timeline) {
                break;
            }
            changed = true;
        }
        changed
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.undo.len()
    }

    /// Labels of the changes that can still be undone, oldest first.
    #[must_use]
    pub fn labels(&self) -> Vec<String> {
        self.undo.iter().map(|e| e.label.clone()).collect()
    }

    /// Labels waiting on redo, nearest first.
    #[must_use]
    pub fn redo_labels(&self) -> Vec<String> {
        self.redo.iter().rev().map(|e| e.label.clone()).collect()
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}
