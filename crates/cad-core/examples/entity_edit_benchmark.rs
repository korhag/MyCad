//! Run with `cargo run -p cad-core --example entity_edit_benchmark --offline`.
//! Times ordinary appends and tail removals in a drawing with 300,000 entities.
//! Setup and index-consistency checks are outside the timed sections.

use cad_core::{BlockDefinition, Document, Entity, Geometry, Point3};
use std::time::Instant;

fn point() -> Entity {
    Entity::new(Geometry::Point {
        position: Point3::default(),
    })
}

fn main() {
    let mut document = Document::default();
    document.model_space = (0..100_000).map(|_| point()).collect();
    document.blocks.insert(
        "Library".into(),
        BlockDefinition::plain(
            "Library",
            Point3::default(),
            (0..200_000).map(|_| point()).collect(),
        ),
    );
    document.assign_missing_ids();
    let started = Instant::now();
    let mut inserted = Vec::with_capacity(100);
    for _ in 0..100 {
        inserted.push(document.add_entity(point()).id);
    }
    println!(
        "append_average_ms={:.3}",
        started.elapsed().as_secs_f64() * 10.0
    );
    assert!(document.entity_index_is_consistent());
    let started = Instant::now();
    for id in inserted.into_iter().rev() {
        document.remove_model_entity(id).expect("appended entity");
    }
    println!(
        "tail_remove_average_ms={:.3}",
        started.elapsed().as_secs_f64() * 10.0
    );
    assert_eq!(document.model_space.len(), 100_000);
    assert!(document.entity_index_is_consistent());
}
