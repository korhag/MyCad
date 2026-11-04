//! Shared nested-block dependency benchmark. Run from the repository root:
//! `cargo run -p cad-core --example dynamic_dependencies_benchmark --offline`
//!
//! Twelve stored definitions reference the next level three times each. The
//! evaluator only needs eleven unique dependency revisions. Every trial starts
//! with an empty evaluation cache; this measures evaluation/key construction,
//! not drawing expansion, import, or rendering.

use cad_core::{
    evaluate_definition, identity_insert, BlockDefinition, Document, DynamicDefinition, Entity,
    EvaluationCache, EvaluationRequest, Geometry, Point3,
};
use std::time::Instant;
fn main() {
    let mut document = Document::default();
    for index in (0..12).rev() {
        let entities = if index == 11 {
            vec![Entity::new(Geometry::Point {
                position: Point3::default(),
            })]
        } else {
            (0..3)
                .map(|_| {
                    Entity::new(identity_insert(
                        format!("Shared{}", index + 1),
                        Point3::default(),
                    ))
                })
                .collect()
        };
        let mut block =
            BlockDefinition::plain(format!("Shared{index}"), Point3::default(), entities);
        if index == 0 {
            block.dynamic = Some(DynamicDefinition::default());
        }
        document.replace_block_definition(block);
    }
    document.assign_missing_ids();
    let definition = document.block_by_name("Shared0").unwrap();
    let request = EvaluationRequest {
        generation: document.content_generation(),
    };
    for trial in 0..3 {
        let started = Instant::now();
        let result = evaluate_definition(
            &document,
            definition,
            None,
            &mut EvaluationCache::default(),
            request,
        )
        .unwrap();
        assert_eq!(result.key.nested_revisions.len(), 11);
        println!(
            "trial={trial} elapsed_ms={:.3}",
            started.elapsed().as_secs_f64() * 1000.0
        );
        std::hint::black_box(result);
    }
}
