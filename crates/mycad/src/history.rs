//! Reversible document transactions for editor commands.

use cad_core::{Document, Entity};

// ------------------------------------------------------------
// Enum: Edit
// Purpose: One reversible model-space mutation, retaining draw order.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum Edit {
    Insert {
        index: usize,
        entity: Entity,
    },
    Remove {
        index: usize,
        entity: Entity,
    },
    Replace {
        index: usize,
        before: Entity,
        after: Entity,
    },
}

impl Edit {
    pub fn apply(&self, document: &mut Document) {
        match self {
            Self::Insert { index, entity } => {
                document.insert_model_entity(*index, entity.clone());
                document
                    .diagnostics
                    .bump_entity(entity.geometry.type_name());
                document.diagnostics.object_count =
                    document.diagnostics.object_count.saturating_add(1);
            }
            Self::Remove { entity, .. } => {
                if document.remove_model_entity(entity.id).is_some() {
                    decrement_entity_count(document, entity.geometry.type_name());
                    document.diagnostics.object_count =
                        document.diagnostics.object_count.saturating_sub(1);
                }
            }
            Self::Replace { after, .. } => {
                let _ = document.replace_model_entity(after.id, after.clone());
            }
        }
    }

    pub fn invert(&self) -> Self {
        match self {
            Self::Insert { index, entity } => Self::Remove {
                index: *index,
                entity: entity.clone(),
            },
            Self::Remove { index, entity } => Self::Insert {
                index: *index,
                entity: entity.clone(),
            },
            Self::Replace {
                index,
                before,
                after,
            } => Self::Replace {
                index: *index,
                before: after.clone(),
                after: before.clone(),
            },
        }
    }
}

// ------------------------------------------------------------
// Type: Transaction
// Purpose: One undoable command, possibly spanning several edits.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct Transaction {
    pub edits: Vec<Edit>,
}

impl Transaction {
    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    pub fn apply(&self, document: &mut Document) {
        for edit in &self.edits {
            edit.apply(document);
        }
    }

    pub fn invert(&self) -> Self {
        Self {
            edits: self.edits.iter().rev().map(Edit::invert).collect(),
        }
    }
}

// ------------------------------------------------------------
// Type: History
// Purpose: Undo/redo stack with a clean revision for dirty tracking.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Transaction>,
    redo: Vec<Transaction>,
    open: Option<Transaction>,
    revision: u64,
    clean_revision: u64,
}

impl History {
    pub fn is_dirty(&self) -> bool {
        self.revision != self.clean_revision
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn mark_clean(&mut self) {
        self.clean_revision = self.revision;
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.open = None;
        self.revision = 0;
        self.mark_clean();
    }

    pub fn begin(&mut self) {
        self.commit_open();
        self.open = Some(Transaction::default());
    }

    pub fn record(&mut self, edit: Edit) {
        match &mut self.open {
            Some(open) => open.edits.push(edit),
            None => {
                self.open = Some(Transaction { edits: vec![edit] });
                self.commit_open();
            }
        }
    }

    pub fn commit_open(&mut self) {
        if let Some(open) = self.open.take() {
            if !open.is_empty() {
                self.undo.push(open);
                self.redo.clear();
                self.revision = self.revision.wrapping_add(1);
            }
        }
    }

    pub fn undo(&mut self, document: &mut Document) -> bool {
        self.commit_open();
        let Some(transaction) = self.undo.pop() else {
            return false;
        };
        transaction.invert().apply(document);
        self.redo.push(transaction);
        self.revision = self.revision.wrapping_sub(1);
        true
    }

    pub fn redo(&mut self, document: &mut Document) -> bool {
        self.commit_open();
        let Some(transaction) = self.redo.pop() else {
            return false;
        };
        transaction.apply(document);
        self.undo.push(transaction);
        self.revision = self.revision.wrapping_add(1);
        true
    }
}

fn decrement_entity_count(document: &mut Document, type_name: &str) {
    if let Some(count) = document.diagnostics.entity_counts.get_mut(type_name) {
        *count = count.saturating_sub(1);
        if *count == 0 {
            document.diagnostics.entity_counts.remove(type_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{Entity, Geometry, Point3};

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    #[test]
    fn grouped_line_segments_undo_as_one_transaction() {
        let mut document = Document::default();
        let mut history = History::default();
        history.begin();
        let first = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        history.record(Edit::Insert {
            index: 0,
            entity: first.clone(),
        });
        let second = document.add_entity(line(1.0, 0.0, 2.0, 0.0));
        history.record(Edit::Insert {
            index: 1,
            entity: second.clone(),
        });
        history.commit_open();
        assert!(history.is_dirty());
        assert_eq!(document.model_space.len(), 2);
        assert!(history.undo(&mut document));
        assert!(document.model_space.is_empty());
        assert!(!history.is_dirty());
        assert!(history.redo(&mut document));
        assert_eq!(document.model_space.len(), 2);
        assert_eq!(document.model_space[0].id, first.id);
        assert_eq!(document.model_space[1].id, second.id);
        assert!(history.is_dirty());
    }

    #[test]
    fn empty_command_does_not_create_undo_entry() {
        let mut history = History::default();
        history.begin();
        history.commit_open();
        assert!(!history.can_undo());
        assert!(!history.is_dirty());
    }

    #[test]
    fn undoing_to_loaded_revision_clears_dirty() {
        let mut document = Document::default();
        let mut history = History::default();
        history.mark_clean();
        let entity = document.add_entity(line(0.0, 0.0, 3.0, 0.0));
        history.record(Edit::Insert { index: 0, entity });
        assert!(history.is_dirty());
        history.undo(&mut document);
        assert!(!history.is_dirty());
        history.redo(&mut document);
        assert!(history.is_dirty());
    }
}
