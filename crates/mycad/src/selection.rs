//! Top-level entity selection scoped to the loaded document.

use cad_core::{Extents2, Point2};
use cad_render::{box_select, hit_test, DisplayList, SelectBoxMode, DEFAULT_PICK_TOLERANCE_PX};
use cad_viewport::Camera2;

// ------------------------------------------------------------
// Type: Selection
// Purpose: Ordered set of top-level model-space entity indices.
// ------------------------------------------------------------
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    indices: Vec<usize>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.indices.clear();
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
        self.indices.contains(&index)
    }

    pub fn replace(&mut self, index: usize) {
        self.indices.clear();
        self.indices.push(index);
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(pos) = self.indices.iter().position(|existing| *existing == index) {
            self.indices.remove(pos);
        } else {
            self.indices.push(index);
        }
    }

    pub fn replace_all(&mut self, indices: impl IntoIterator<Item = usize>) {
        self.indices.clear();
        for index in indices {
            if !self.indices.contains(&index) {
                self.indices.push(index);
            }
        }
    }

    pub fn toggle_all(&mut self, indices: impl IntoIterator<Item = usize>) {
        for index in indices {
            self.toggle(index);
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

pub fn box_pick_entities(
    display: &DisplayList,
    camera: &Camera2,
    start: Point2,
    current: Point2,
    viewport_origin: Point2,
    viewport_size: Point2,
) -> (SelectBoxMode, Vec<usize>) {
    let mode = SelectBoxMode::from_screen_drag(start, current);
    let a = camera.screen_to_world(start, viewport_origin, viewport_size);
    let b = camera.screen_to_world(current, viewport_origin, viewport_size);
    let region = Extents2::from_corners(a, b);
    (mode, box_select(&display.picks, region, mode))
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
}
