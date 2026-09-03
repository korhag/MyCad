//! Top-level entity selection scoped to the loaded document.

use std::collections::HashSet;

use cad_core::{Document, EntityId, Extents2, Point2};
use cad_render::{hit_test, DisplayList, SelectBoxMode, DEFAULT_PICK_TOLERANCE_PX};
use cad_viewport::Camera2;

// ------------------------------------------------------------
// Type: Selection
// Purpose: Ordered set of top-level model-space entity IDs.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    ids: Vec<EntityId>,
    members: HashSet<EntityId>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.ids.clear();
        self.members.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn ids(&self) -> &[EntityId] {
        &self.ids
    }

    #[allow(dead_code)]
    pub fn contains(&self, id: EntityId) -> bool {
        self.members.contains(&id)
    }

    pub fn replace(&mut self, id: EntityId) {
        self.ids.clear();
        self.members.clear();
        self.ids.push(id);
        self.members.insert(id);
    }

    pub fn toggle(&mut self, id: EntityId) {
        if self.members.remove(&id) {
            self.ids.retain(|existing| *existing != id);
        } else {
            self.members.insert(id);
            self.ids.push(id);
        }
    }

    pub fn replace_all(&mut self, ids: impl IntoIterator<Item = EntityId>) {
        self.ids.clear();
        self.members.clear();
        for id in ids {
            if self.members.insert(id) {
                self.ids.push(id);
            }
        }
    }

    pub fn toggle_all(&mut self, ids: impl IntoIterator<Item = EntityId>) {
        let mut odd = HashSet::new();
        for id in ids {
            if !odd.remove(&id) {
                odd.insert(id);
            }
        }
        if odd.is_empty() {
            return;
        }
        let mut add = Vec::new();
        let mut remove = HashSet::new();
        for id in odd {
            if self.members.contains(&id) {
                remove.insert(id);
            } else {
                add.push(id);
            }
        }
        if !remove.is_empty() {
            self.ids.retain(|id| !remove.contains(id));
            self.members.retain(|id| !remove.contains(id));
        }
        for id in add {
            if self.members.insert(id) {
                self.ids.push(id);
            }
        }
    }

    pub fn commit_box(&mut self, candidates: &[EntityId], toggle: bool) {
        if toggle {
            self.toggle_all(candidates.iter().copied());
        } else {
            self.replace_all(candidates.iter().copied());
        }
    }

    pub fn retain_valid(&mut self, document: &Document) {
        self.ids.retain(|id| document.entity_by_id(*id).is_some());
        self.members
            .retain(|id| document.entity_by_id(*id).is_some());
    }

    pub fn shared_layer(&self, document: &Document) -> Option<String> {
        let mut layer = None;
        for id in &self.ids {
            let entity_layer = document.entity_by_id(*id)?.layer.clone();
            match &layer {
                None => layer = Some(entity_layer),
                Some(existing) if existing != &entity_layer => return None,
                Some(_) => {}
            }
        }
        layer
    }
}

// ------------------------------------------------------------
// Function: pick_entity
// Purpose: Hit-test the pointer against tessellated pick geometry.
// ------------------------------------------------------------
pub fn pick_entity(
    display: &DisplayList,
    camera: &Camera2,
    screen: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
) -> Option<EntityId> {
    hit_test(
        &display.picks,
        camera,
        screen,
        viewport_origin,
        viewport_size,
        DEFAULT_PICK_TOLERANCE_PX,
    )
}

#[allow(dead_code)]
pub fn box_pick_entities(
    display: &DisplayList,
    camera: &Camera2,
    start: Point2,
    current: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
) -> (SelectBoxMode, Vec<EntityId>) {
    let mut out = Vec::new();
    let mode = box_pick_entities_into(
        display,
        camera,
        start,
        current,
        viewport_origin,
        viewport_size,
        &mut out,
    );
    (mode, out)
}

pub fn box_pick_entities_into(
    display: &DisplayList,
    camera: &Camera2,
    start: Point2,
    current: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
    out: &mut Vec<EntityId>,
) -> SelectBoxMode {
    let mode = SelectBoxMode::from_screen_drag(start, current);
    let a = camera.screen_to_world(start, viewport_origin, viewport_size);
    let b = camera.screen_to_world(current, viewport_origin, viewport_size);
    let region = Extents2::from_corners(a, b);
    display.box_select_into(region, mode, out);
    mode
}

#[cfg(test)]
mod tests {
    use super::*;
    use cad_core::{Entity, Geometry, Point3};

    fn id(n: u64) -> EntityId {
        EntityId(n)
    }

    #[test]
    fn replace_clears_previous_selection() {
        let mut selection = Selection::default();
        selection.replace(id(2));
        selection.replace(id(5));
        assert_eq!(selection.ids(), &[id(5)]);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut selection = Selection::default();
        selection.toggle(id(1));
        selection.toggle(id(3));
        selection.toggle(id(1));
        assert_eq!(selection.ids(), &[id(3)]);
    }

    #[test]
    fn clear_empties_selection() {
        let mut selection = Selection::default();
        selection.replace(id(0));
        selection.clear();
        assert!(selection.is_empty());
    }

    #[test]
    fn retain_valid_drops_removed_ids() {
        let mut document = Document::default();
        let first = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(0.0, 0.0),
            end: Point3::from_xy(1.0, 0.0),
        }));
        let second = document.add_entity(Entity::new(Geometry::Line {
            start: Point3::from_xy(1.0, 0.0),
            end: Point3::from_xy(2.0, 0.0),
        }));
        let mut selection = Selection::default();
        selection.toggle(first.id);
        selection.toggle(second.id);
        document.remove_model_entity(second.id);
        selection.retain_valid(&document);
        assert_eq!(selection.ids(), &[first.id]);
    }

    #[test]
    fn replace_all_replaces_and_dedupes() {
        let mut selection = Selection::default();
        selection.replace(id(9));
        selection.replace_all([id(1), id(2), id(1)]);
        assert_eq!(selection.ids(), &[id(1), id(2)]);
        assert!(selection.contains(id(1)));
        assert!(!selection.contains(id(9)));
    }

    #[test]
    fn toggle_all_adds_and_removes_candidates() {
        let mut selection = Selection::default();
        selection.replace_all([id(1), id(2)]);
        selection.toggle_all([id(2), id(3)]);
        assert_eq!(selection.ids(), &[id(1), id(3)]);
    }

    #[test]
    fn commit_box_replace_and_toggle() {
        let mut selection = Selection::default();
        selection.replace(id(4));
        selection.commit_box(&[id(1), id(2)], false);
        assert_eq!(selection.ids(), &[id(1), id(2)]);
        selection.commit_box(&[id(2), id(5)], true);
        assert_eq!(selection.ids(), &[id(1), id(5)]);
    }

    #[test]
    fn bulk_commit_stays_near_linear() {
        let mut selection = Selection::default();
        let many: Vec<EntityId> = (1..20_001).map(EntityId).collect();
        selection.replace_all(many.iter().copied());
        assert_eq!(selection.len(), 20_000);
        selection.toggle_all((1..10_001).map(EntityId));
        assert_eq!(selection.len(), 10_000);
        assert_eq!(selection.ids()[0], id(10_001));
        assert!(selection.contains(id(20_000)));
        assert!(!selection.contains(id(1)));
    }
}
