//! Reversible document transactions for editor commands.

use cad_core::{BlockDefinition, Document, Entity, EntitySpace};

// ------------------------------------------------------------
// Enum: Edit
// Purpose: One reversible mutation of an entity container or block definition.
// ------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum Edit {
    InsertEntity {
        space: EntitySpace,
        index: usize,
        entity: Entity,
    },
    RemoveEntity {
        space: EntitySpace,
        index: usize,
        entity: Entity,
    },
    ReplaceEntity {
        space: EntitySpace,
        index: usize,
        before: Entity,
        after: Entity,
    },
    ReplaceBlockDefinition {
        name: String,
        before: Option<BlockDefinition>,
        after: Option<BlockDefinition>,
    },
    RenameBlock {
        before: String,
        after: String,
    },
}

impl Edit {
    #[cfg(test)]
    pub fn insert_model(index: usize, entity: Entity) -> Self {
        Self::InsertEntity {
            space: EntitySpace::ModelSpace,
            index,
            entity,
        }
    }

    #[cfg(test)]
    pub fn remove_model(index: usize, entity: Entity) -> Self {
        Self::RemoveEntity {
            space: EntitySpace::ModelSpace,
            index,
            entity,
        }
    }

    #[cfg(test)]
    pub fn replace_model(index: usize, before: Entity, after: Entity) -> Self {
        Self::ReplaceEntity {
            space: EntitySpace::ModelSpace,
            index,
            before,
            after,
        }
    }

    pub fn apply(&self, document: &mut Document) {
        match self {
            Self::InsertEntity {
                space,
                index,
                entity,
            } => {
                if let Some(inserted) = document.insert_entity(space, *index, entity.clone()) {
                    if space.is_model() {
                        document
                            .diagnostics
                            .bump_entity(inserted.geometry.type_name());
                        document.diagnostics.object_count =
                            document.diagnostics.object_count.saturating_add(1);
                    }
                }
            }
            Self::RemoveEntity { space, entity, .. } => {
                if document.remove_entity_from(space, entity.id).is_some() && space.is_model() {
                    decrement_entity_count(document, entity.geometry.type_name());
                    document.diagnostics.object_count =
                        document.diagnostics.object_count.saturating_sub(1);
                }
            }
            Self::ReplaceEntity { space, after, .. } => {
                let _ = document.replace_entity_in(space, after.id, after.clone());
            }
            Self::ReplaceBlockDefinition { name, after, .. } => match after {
                Some(definition) => {
                    document.replace_block_definition(definition.clone());
                }
                None => {
                    let _ = document.remove_block_definition(name);
                }
            },
            Self::RenameBlock { before, after } => {
                let _ = document.rename_block(before, after);
            }
        }
    }

    pub fn invert(&self) -> Self {
        match self {
            Self::InsertEntity {
                space,
                index,
                entity,
            } => Self::RemoveEntity {
                space: space.clone(),
                index: *index,
                entity: entity.clone(),
            },
            Self::RemoveEntity {
                space,
                index,
                entity,
            } => Self::InsertEntity {
                space: space.clone(),
                index: *index,
                entity: entity.clone(),
            },
            Self::ReplaceEntity {
                space,
                index,
                before,
                after,
            } => Self::ReplaceEntity {
                space: space.clone(),
                index: *index,
                before: after.clone(),
                after: before.clone(),
            },
            Self::ReplaceBlockDefinition {
                name,
                before,
                after,
            } => Self::ReplaceBlockDefinition {
                name: name.clone(),
                before: after.clone(),
                after: before.clone(),
            },
            Self::RenameBlock { before, after } => Self::RenameBlock {
                before: after.clone(),
                after: before.clone(),
            },
        }
    }

    fn is_rename(&self) -> bool {
        matches!(self, Self::RenameBlock { .. })
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

    pub fn is_rename_only(&self) -> bool {
        !self.edits.is_empty() && self.edits.iter().all(Edit::is_rename)
    }

    pub fn rename_edits(&self) -> Vec<(String, String)> {
        self.edits
            .iter()
            .filter_map(|edit| match edit {
                Edit::RenameBlock { before, after } => Some((before.clone(), after.clone())),
                _ => None,
            })
            .collect()
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

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn peek_undo(&self) -> Option<&Transaction> {
        self.undo.last()
    }

    pub fn peek_redo(&self) -> Option<&Transaction> {
        self.redo.last()
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

    #[cfg(test)]
    pub fn pop_last_open_edit(&mut self) -> Option<Edit> {
        self.open.as_mut()?.edits.pop()
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

    pub fn undo_beyond(&mut self, document: &mut Document, min_len: usize) -> bool {
        if self.undo.len() <= min_len {
            return false;
        }
        self.undo(document)
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

    /// Merge geometry since `since` into one undo step.
    /// Explicit block renames stay as their own commands so Save does not
    /// bundle a tree rename with in-session drawing edits.
    pub fn collapse_since(&mut self, since: usize) -> bool {
        self.commit_open();
        if self.undo.len() <= since {
            return false;
        }
        let rest: Vec<Transaction> = self.undo.drain(since..).collect();
        let removed = rest.len() as u64;
        let mut rename_edits = Vec::new();
        let mut other_edits = Vec::new();
        for transaction in rest {
            for edit in transaction.edits {
                if edit.is_rename() {
                    rename_edits.push(edit);
                } else {
                    other_edits.push(edit);
                }
            }
        }
        self.revision = self.revision.wrapping_sub(removed);
        let mut changed = false;
        for edit in rename_edits {
            self.undo.push(Transaction { edits: vec![edit] });
            self.revision = self.revision.wrapping_add(1);
            changed = true;
        }
        if !other_edits.is_empty() {
            self.undo.push(Transaction { edits: other_edits });
            self.revision = self.revision.wrapping_add(1);
            changed = true;
        }
        self.redo.clear();
        changed
    }

    /// Invert geometry since `since` and drop it so discarded block edits
    /// cannot be redone. Explicit `RenameBlock` commands are inverted with
    /// the rest, then re-applied so a later tree rename survives Discard.
    pub fn discard_since(&mut self, document: &mut Document, since: usize) {
        self.commit_open();
        if self.undo.len() <= since {
            self.redo.clear();
            return;
        }
        let rest: Vec<Transaction> = self.undo.drain(since..).collect();
        let mut kept_renames = Vec::new();
        for transaction in rest.into_iter().rev() {
            let keep = transaction.is_rename_only();
            transaction.invert().apply(document);
            self.revision = self.revision.wrapping_sub(1);
            if keep {
                kept_renames.push(transaction);
            }
        }
        kept_renames.reverse();
        for transaction in kept_renames {
            transaction.apply(document);
            self.undo.push(transaction);
            self.revision = self.revision.wrapping_add(1);
        }
        self.redo.clear();
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
    use cad_core::{
        create_block_from_entities, default_extrusion, Entity, EntitySpace, Geometry, Point2,
        Point3,
    };

    fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Entity {
        Entity::new(Geometry::Line {
            start: Point3::from_xy(x0, y0),
            end: Point3::from_xy(x1, y1),
        })
    }

    fn circle(x: f64, y: f64, radius: f64) -> Entity {
        Entity::new(Geometry::Circle {
            center: Point3::from_xy(x, y),
            radius,
            extrusion: default_extrusion(),
        })
    }

    #[test]
    fn grouped_line_segments_undo_as_one_transaction() {
        let mut document = Document::default();
        let mut history = History::default();
        history.begin();
        let first = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        history.record(Edit::insert_model(0, first.clone()));
        let second = document.add_entity(line(1.0, 0.0, 2.0, 0.0));
        history.record(Edit::insert_model(1, second.clone()));
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
    fn popping_last_open_edit_undoes_one_segment() {
        let mut document = Document::default();
        let mut history = History::default();
        history.begin();
        let first = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        history.record(Edit::insert_model(0, first.clone()));
        let second = document.add_entity(line(1.0, 0.0, 2.0, 0.0));
        history.record(Edit::insert_model(1, second));
        let popped = history.pop_last_open_edit().expect("open edit");
        popped.invert().apply(&mut document);
        assert_eq!(document.model_space.len(), 1);
        assert_eq!(document.model_space[0].id, first.id);
        history.commit_open();
        assert!(history.can_undo());
    }

    #[test]
    fn undoing_to_loaded_revision_clears_dirty() {
        let mut document = Document::default();
        let mut history = History::default();
        history.mark_clean();
        let entity = document.add_entity(line(0.0, 0.0, 3.0, 0.0));
        history.record(Edit::insert_model(0, entity));
        assert!(history.is_dirty());
        history.undo(&mut document);
        assert!(!history.is_dirty());
        history.redo(&mut document);
        assert!(history.is_dirty());
    }

    #[test]
    fn grouped_replaces_undo_as_one_transaction() {
        use cad_core::{transform_entity, EntityTransform};
        let mut document = Document::default();
        let mut history = History::default();
        let first = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        let second = document.add_entity(line(2.0, 0.0, 3.0, 0.0));
        history.begin();
        let xf = EntityTransform::Translate { dx: 5.0, dy: 0.0 };
        for entity in [first.clone(), second.clone()] {
            let after = transform_entity(&entity, xf).expect("move");
            let index = document.entity_index(entity.id).unwrap();
            let _ = document.replace_model_entity(entity.id, after.clone());
            history.record(Edit::replace_model(index, entity, after));
        }
        history.commit_open();
        history.undo(&mut document);
        assert_eq!(document.model_space.len(), 2);
        match &document.model_space[0].geometry {
            Geometry::Line { start, .. } => assert!((start.x).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
        history.redo(&mut document);
        match &document.model_space[0].geometry {
            Geometry::Line { start, .. } => assert!((start.x - 5.0).abs() < 1e-12),
            other => panic!("{other:?}"),
        }
        assert_eq!(document.model_space[0].id, first.id);
        assert_eq!(document.model_space[1].id, second.id);
    }

    #[test]
    fn erase_undo_restores_ids_and_order() {
        let mut document = Document::default();
        let mut history = History::default();
        let first = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        let second = document.add_entity(line(2.0, 0.0, 3.0, 0.0));
        history.begin();
        history.record(Edit::remove_model(1, second.clone()));
        history.record(Edit::remove_model(0, first.clone()));
        Edit::remove_model(1, second.clone()).apply(&mut document);
        Edit::remove_model(0, first.clone()).apply(&mut document);
        history.commit_open();
        assert!(document.model_space.is_empty());
        history.undo(&mut document);
        assert_eq!(document.model_space.len(), 2);
        assert_eq!(document.model_space[0].id, first.id);
        assert_eq!(document.model_space[1].id, second.id);
        history.redo(&mut document);
        assert!(document.model_space.is_empty());
    }

    #[test]
    fn copy_inserts_keep_new_ids_on_undo_redo() {
        let mut document = Document::default();
        let mut history = History::default();
        let source = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        history.begin();
        let mut copy = source.clone();
        copy.id = cad_core::EntityId::UNASSIGNED;
        let copy = document.add_entity(copy);
        history.record(Edit::insert_model(1, copy.clone()));
        history.commit_open();
        assert_ne!(copy.id, source.id);
        assert_eq!(copy.layer, source.layer);
        history.undo(&mut document);
        assert_eq!(document.model_space.len(), 1);
        assert_eq!(document.model_space[0].id, source.id);
        history.redo(&mut document);
        assert_eq!(document.model_space.len(), 2);
        assert_eq!(document.model_space[1].id, copy.id);
    }

    #[test]
    fn create_block_undo_restores_source_entities() {
        let mut document = Document::default();
        let mut history = History::default();
        let a = document.add_entity(line(0.0, 0.0, 10.0, 0.0));
        let b = document.add_entity(circle(5.0, 0.0, 2.0));
        history.mark_clean();
        history.begin();
        let result = create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[a.id, b.id],
            "TestBlock",
            Point2::new(5.0, 0.0),
            true,
        )
        .expect("create");
        for (space, index, entity) in &result.removed {
            history.record(Edit::RemoveEntity {
                space: space.clone(),
                index: *index,
                entity: entity.clone(),
            });
        }
        history.record(Edit::ReplaceBlockDefinition {
            name: result.name.clone(),
            before: None,
            after: Some(result.definition.clone()),
        });
        if let Some((space, index, entity)) = result.insert {
            history.record(Edit::InsertEntity {
                space,
                index,
                entity,
            });
        }
        history.commit_open();
        assert_eq!(document.model_space.len(), 1);
        assert!(matches!(
            document.model_space[0].geometry,
            Geometry::Insert { .. }
        ));
        assert_eq!(
            document.block_by_name("TestBlock").unwrap().entities.len(),
            2
        );
        history.undo(&mut document);
        assert_eq!(document.model_space.len(), 2);
        assert!(document.block_by_name("TestBlock").is_none());
        assert_eq!(document.model_space[0].id, a.id);
        assert_eq!(document.model_space[1].id, b.id);
        assert!(!history.is_dirty());
        history.redo(&mut document);
        assert_eq!(document.model_space.len(), 1);
        assert!(document.block_by_name("TestBlock").is_some());
    }

    #[test]
    fn block_space_edits_undo_without_touching_model_space() {
        let mut document = Document::default();
        let mut history = History::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let space = EntitySpace::Block("Motor".into());
        history.begin();
        let extra = document
            .add_entity_to(&space, line(2.0, 0.0, 3.0, 0.0))
            .unwrap();
        history.record(Edit::InsertEntity {
            space: space.clone(),
            index: 1,
            entity: extra.clone(),
        });
        history.commit_open();
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 2);
        history.undo(&mut document);
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 1);
        assert_eq!(document.model_space.len(), 1);
        history.redo(&mut document);
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 2);
    }

    #[test]
    fn collapse_since_makes_session_edits_one_undo() {
        use cad_core::{membership_matrix, transfer_entity, Transform2};
        let mut document = Document::default();
        let mut history = History::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let extra = document.add_entity(line(4.0, 0.0, 5.0, 0.0));
        let extra_id = extra.id;
        let mark = history.undo_len();
        history.begin();
        let transferred = transfer_entity(
            &mut document,
            extra_id,
            &EntitySpace::Block("Motor".into()),
            membership_matrix(Transform2::identity(), Transform2::identity()).unwrap(),
        )
        .unwrap();
        history.record(Edit::RemoveEntity {
            space: transferred.source,
            index: transferred.source_index,
            entity: transferred.before,
        });
        history.record(Edit::InsertEntity {
            space: transferred.dest,
            index: transferred.dest_index,
            entity: transferred.after,
        });
        history.commit_open();
        history.begin();
        let space = EntitySpace::Block("Motor".into());
        let drawn = document
            .add_entity_to(&space, line(8.0, 0.0, 9.0, 0.0))
            .unwrap();
        history.record(Edit::InsertEntity {
            space,
            index: 2,
            entity: drawn,
        });
        history.commit_open();
        assert_eq!(history.undo_len(), mark + 2);
        assert!(history.collapse_since(mark));
        assert_eq!(history.undo_len(), mark + 1);
        assert_eq!(document.model_space.len(), 1);
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 3);
        history.undo(&mut document);
        assert_eq!(document.model_space.len(), 2);
        assert_eq!(document.model_space[1].id, extra_id);
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 1);
        history.redo(&mut document);
        assert!(document
            .entity_by_id_in(&EntitySpace::ModelSpace, extra_id)
            .is_none());
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 3);
    }

    #[test]
    fn discard_since_restores_membership() {
        use cad_core::{membership_matrix, transfer_entity, Transform2};
        let mut document = Document::default();
        let mut history = History::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let extra = document.add_entity(line(4.0, 0.0, 5.0, 0.0));
        let extra_id = extra.id;
        let mark = history.undo_len();
        history.begin();
        let transferred = transfer_entity(
            &mut document,
            extra_id,
            &EntitySpace::Block("Motor".into()),
            membership_matrix(Transform2::identity(), Transform2::identity()).unwrap(),
        )
        .unwrap();
        history.record(Edit::RemoveEntity {
            space: transferred.source,
            index: transferred.source_index,
            entity: transferred.before,
        });
        history.record(Edit::InsertEntity {
            space: transferred.dest,
            index: transferred.dest_index,
            entity: transferred.after,
        });
        history.commit_open();
        history.discard_since(&mut document, mark);
        assert_eq!(document.model_space.len(), 2);
        assert_eq!(document.model_space[1].id, extra_id);
        assert_eq!(document.block_by_name("Motor").unwrap().entities.len(), 1);
        assert!(!history.can_redo());
    }

    #[test]
    fn rename_block_undo_restores_name_and_insert_ids() {
        let mut document = Document::default();
        let mut history = History::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let insert_id = document.model_space[0].id;
        history.begin();
        document.rename_block("Motor", "Motor Drive").unwrap();
        history.record(Edit::RenameBlock {
            before: "Motor".into(),
            after: "Motor Drive".into(),
        });
        history.commit_open();
        assert!(document.block_by_name("Motor Drive").is_some());
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor Drive"),
            other => panic!("{other:?}"),
        }
        assert_eq!(document.model_space[0].id, insert_id);
        assert!(history.peek_undo().is_some_and(Transaction::is_rename_only));
        history.undo(&mut document);
        assert!(document.block_by_name("Motor").is_some());
        match &document.model_space[0].geometry {
            Geometry::Insert { block_name, .. } => assert_eq!(block_name, "Motor"),
            other => panic!("{other:?}"),
        }
        assert_eq!(document.model_space[0].id, insert_id);
        history.redo(&mut document);
        assert!(document.block_by_name("Motor Drive").is_some());
        assert_eq!(document.model_space[0].id, insert_id);
    }

    #[test]
    fn discard_keeps_explicit_rename() {
        let mut document = Document::default();
        let mut history = History::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let mark = history.undo_len();
        history.begin();
        let extra = document
            .add_entity_to(
                &EntitySpace::Block("Motor".into()),
                line(4.0, 0.0, 5.0, 0.0),
            )
            .unwrap();
        history.record(Edit::InsertEntity {
            space: EntitySpace::Block("Motor".into()),
            index: 1,
            entity: extra,
        });
        history.commit_open();
        history.begin();
        document.rename_block("Motor", "Drive").unwrap();
        history.record(Edit::RenameBlock {
            before: "Motor".into(),
            after: "Drive".into(),
        });
        history.commit_open();
        history.discard_since(&mut document, mark);
        assert!(document.block_by_name("Drive").is_some());
        assert!(document.block_by_name("Motor").is_none());
        assert_eq!(document.block_by_name("Drive").unwrap().entities.len(), 1);
        assert!(history.can_undo());
        history.undo(&mut document);
        assert!(document.block_by_name("Motor").is_some());
    }

    #[test]
    fn collapse_peels_rename_out_of_session_geometry() {
        let mut document = Document::default();
        let mut history = History::default();
        let member = document.add_entity(line(0.0, 0.0, 1.0, 0.0));
        create_block_from_entities(
            &mut document,
            &EntitySpace::ModelSpace,
            &[member.id],
            "Motor",
            Point2::new(0.0, 0.0),
            true,
        )
        .unwrap();
        let mark = history.undo_len();
        history.begin();
        document.rename_block("Motor", "Drive").unwrap();
        history.record(Edit::RenameBlock {
            before: "Motor".into(),
            after: "Drive".into(),
        });
        history.commit_open();
        history.begin();
        let extra = document
            .add_entity_to(
                &EntitySpace::Block("Drive".into()),
                line(4.0, 0.0, 5.0, 0.0),
            )
            .unwrap();
        history.record(Edit::InsertEntity {
            space: EntitySpace::Block("Drive".into()),
            index: 1,
            entity: extra,
        });
        history.commit_open();
        assert!(history.collapse_since(mark));
        assert_eq!(history.undo_len(), mark + 2);
        assert_eq!(document.block_by_name("Drive").unwrap().entities.len(), 2);
        history.undo(&mut document);
        assert_eq!(document.block_by_name("Drive").unwrap().entities.len(), 1);
        history.undo(&mut document);
        assert!(document.block_by_name("Motor").is_some());
    }
}
