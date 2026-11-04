//! Flip and visibility evaluation benchmark. Run from the repository root:
//! `cargo run -p cad-core --example dynamic_actions_benchmark --offline`
//!
//! Applies a reflection and an active visibility group to 10,000 line members.
//! Every trial starts with an empty cache and checks the reflected endpoint and
//! visibility. This measures evaluation, not viewport rendering or import.

use cad_core::dynamic_model::{
    ParameterCondition, ReflectionBehavior, TextReflectPolicy, VisibilityGroup,
};
use cad_core::{
    evaluate_definition, BlockDefinition, BooleanParameter, Document, DynamicDefinition, Entity,
    EvaluationCache, EvaluationRequest, Geometry, ParameterDef, Point2, Point3,
};
use std::time::Instant;
fn main() {
    let mut document = Document::default();
    let parameter = document.allocate_parameter_id();
    let members: Vec<_> = (0..10_000)
        .map(|index| {
            let mut entity = Entity::new(Geometry::Line {
                start: Point3::from_xy(index as f64, 0.0),
                end: Point3::from_xy(index as f64, 5.0),
            });
            entity.id = document.allocate_id();
            entity
        })
        .collect();
    let ids: Vec<_> = members.iter().map(|entity| entity.id).collect();
    let mut block = BlockDefinition::plain("ManyMembers", Point3::default(), members);
    block.dynamic = Some(DynamicDefinition {
        parameters: vec![ParameterDef::boolean(
            parameter,
            "Enabled",
            BooleanParameter {
                default: true,
                ..Default::default()
            },
        )],
        reflections: vec![ReflectionBehavior {
            id: document.allocate_action_id(),
            name: None,
            members: ids.clone(),
            axis_a: Point2::new(0.0, 0.0),
            axis_b: Point2::new(0.0, 1.0),
            condition: ParameterCondition::Boolean {
                parameter,
                state: true,
            },
            text_policy: TextReflectPolicy::KeepReadable,
        }],
        visibility: vec![VisibilityGroup {
            id: document.allocate_action_id(),
            name: "Shown".into(),
            members: ids,
            conditions: vec![ParameterCondition::Boolean {
                parameter,
                state: true,
            }],
        }],
        ..Default::default()
    });
    document.replace_block_definition(block);
    let block = document.block_by_name("ManyMembers").unwrap();
    for trial in 0..3 {
        let started = Instant::now();
        let result = evaluate_definition(
            &document,
            block,
            None,
            &mut EvaluationCache::default(),
            EvaluationRequest {
                generation: document.content_generation(),
            },
        )
        .unwrap();
        let elapsed = started.elapsed();
        let Geometry::Line { start, .. } = result.entities[9999].geometry else {
            panic!("line");
        };
        assert_eq!(start.x, -9999.0);
        assert!(result.entities.iter().all(|entity| entity.visible));
        println!(
            "trial={trial} elapsed_ms={:.3}",
            elapsed.as_secs_f64() * 1000.0
        );
    }
}
