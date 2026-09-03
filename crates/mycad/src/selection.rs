//! Top-level entity selection scoped to the loaded document.

use std::collections::HashSet;

use cad_core::{Extents2, Point2};
use cad_render::{hit_test, DisplayList, SelectBoxMode, DEFAULT_PICK_TOLERANCE_PX};
use cad_viewport::Camera2;

// ------------------------------------------------------------
// Type: Selection
// Purpose: Ordered set of top-level model-space entity indices.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    indices: Vec<usize>,
    members: HashSet<usize>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.indices.clear();
        self.members.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    pub fn len(&self) -> usize {
        self.indices.len()
    }

    pub fn indices(&self) -> &[usize] {
        &self.indices
    }

    #[allow(dead_code)]
    pub fn contains(&self, index: usize) -> bool {
        self.members.contains(&index)
    }

    pub fn replace(&mut self, index: usize) {
        self.indices.clear();
        self.members.clear();
        self.indices.push(index);
        self.members.insert(index);
    }

    pub fn toggle(&mut self, index: usize) {
        if self.members.remove(&index) {
            self.indices.retain(|existing| *existing != index);
        } else {
            self.members.insert(index);
            self.indices.push(index);
        }
    }

    pub fn replace_all(&mut self, indices: impl IntoIterator<Item = usize>) {
        self.indices.clear();
        self.members.clear();
        for index in indices {
            if self.members.insert(index) {
                self.indices.push(index);
            }
        }
    }

    pub fn toggle_all(&mut self, indices: impl IntoIterator<Item = usize>) {
        let mut odd = HashSet::new();
        for index in indices {
            if !odd.remove(&index) {
                odd.insert(index);
            }
        }
        if odd.is_empty() {
            return;
        }
        let mut add = Vec::new();
        let mut remove = HashSet::new();
        for index in odd {
            if self.members.contains(&index) {
                remove.insert(index);
            } else {
                add.push(index);
            }
        }
        if !remove.is_empty() {
            self.indices.retain(|index| !remove.contains(index));
            self.members.retain(|index| !remove.contains(index));
        }
        for index in add {
            if self.members.insert(index) {
                self.indices.push(index);
            }
        }
    }

    pub fn commit_box(&mut self, candidates: &[usize], toggle: bool) {
        if toggle {
            self.toggle_all(candidates.iter().copied());
        } else {
            self.replace_all(candidates.iter().copied());
        }
    }

    #[allow(dead_code)]
    pub fn retain_valid(&mut self, entity_count: usize) {
        self.indices.retain(|index| *index < entity_count);
        self.members.retain(|index| *index < entity_count);
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
) -> Option<usize> {
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
) -> (SelectBoxMode, Vec<usize>) {
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
    out: &mut Vec<usize>,
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

    #[test]
    fn replace_clears_previous_selection() {
        let mut selection = Selection::default();
        selection.replace(2);
        selection.replace(5);
        assert_eq!(selection.indices(), &[5]);
    }

    #[test]
    fn toggle_adds_and_removes() {
        let mut selection = Selection::default();
        selection.toggle(1);
        selection.toggle(3);
        selection.toggle(1);
        assert_eq!(selection.indices(), &[3]);
    }

    #[test]
    fn clear_empties_selection() {
        let mut selection = Selection::default();
        selection.replace(0);
        selection.clear();
        assert!(selection.is_empty());
    }

    #[test]
    fn retain_valid_drops_stale_indices() {
        let mut selection = Selection::default();
        selection.toggle(0);
        selection.toggle(4);
        selection.retain_valid(2);
        assert_eq!(selection.indices(), &[0]);
    }

    #[test]
    fn replace_all_replaces_and_dedupes() {
        let mut selection = Selection::default();
        selection.replace(9);
        selection.replace_all([1, 2, 1]);
        assert_eq!(selection.indices(), &[1, 2]);
        assert!(selection.contains(1));
        assert!(!selection.contains(9));
    }

    #[test]
    fn toggle_all_adds_and_removes_candidates() {
        let mut selection = Selection::default();
        selection.replace_all([1, 2]);
        selection.toggle_all([2, 3]);
        assert_eq!(selection.indices(), &[1, 3]);
    }

    #[test]
    fn commit_box_replace_and_toggle() {
        let mut selection = Selection::default();
        selection.replace(4);
        selection.commit_box(&[1, 2], false);
        assert_eq!(selection.indices(), &[1, 2]);
        selection.commit_box(&[2, 5], true);
        assert_eq!(selection.indices(), &[1, 5]);
    }

    #[test]
    fn bulk_commit_stays_near_linear() {
        let mut selection = Selection::default();
        let many: Vec<usize> = (0..20_000).collect();
        selection.replace_all(many.iter().copied());
        assert_eq!(selection.len(), 20_000);
        selection.toggle_all(0..10_000);
        assert_eq!(selection.len(), 10_000);
        assert_eq!(selection.indices()[0], 10_000);
        assert!(selection.contains(19_999));
        assert!(!selection.contains(0));
    }
}
