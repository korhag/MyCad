//! Repeated dynamic-block materialization benchmark. Run from the repository root:
//! `cargo run -p cad-core --example dynamic_materialize_benchmark --offline`
//!
//! Measures 1,000 identical configured INSERTs of a 2,000-line block with one Move
//! action. Trial 0 starts with an empty evaluation cache; trials 1–2 reuse it.
//! Setup, geometry checks and result destruction are outside the timed section.
//! Compare builds with the same profile; this measures materialization, not rendering.

use cad_core::block::identity_insert;
use cad_core::document::{BlockDefinition, Document};
use cad_core::dynamic::{
    BehaviorKind, CompositionRule, DynamicBehavior, DynamicDefinition, FollowRole, GeometryTarget,
    InstanceConfiguration, NumericParameter, ParameterDef, ParameterValue,
};
use cad_core::entity::{Entity, Geometry};
use cad_core::evaluate::{materialize_evaluated, EvaluationCache, EvaluationRequest};
use cad_core::geom::{Point2, Point3};
use std::time::Instant;

fn main() {
    let mut document = Document::default();
    let parameter = document.allocate_parameter_id();
    let members: Vec<_> = (0..2_000)
        .map(|index| {
            let mut entity = Entity::new(Geometry::Line {
                start: Point3::from_xy(index as f64, 0.0),
                end: Point3::from_xy(index as f64, 10.0),
            });
            entity.id = document.allocate_id();
            entity
        })
        .collect();
    let target = members[0].id;
    let mut block = BlockDefinition::plain("LibraryPart", Point3::from_xy(0.0, 0.0), members);
    block.dynamic = Some(DynamicDefinition {
        parameters: vec![ParameterDef::number(
            parameter,
            "Length",
            NumericParameter::length(800.0),
        )],
        behaviors: vec![DynamicBehavior {
            id: document.allocate_action_id(),
            kind: BehaviorKind::Move,
            parameter,
            targets: vec![GeometryTarget::Entity(target)],
            local_direction: Point2::new(1.0, 0.0),
            reference_value: 800.0,
            multiplier: 1.0,
            composition: CompositionRule::Additive,
            follow: FollowRole::Second,
            name: None,
        }],
        ..Default::default()
    });
    document.replace_block_definition(block);
    for index in 0..1_000 {
        let mut entity = Entity::new(identity_insert(
            "LibraryPart".to_owned(),
            Point3::from_xy(index as f64, 0.0),
        ));
        let mut config = InstanceConfiguration::default();
        config.set(parameter, ParameterValue::Number(1000.0));
        entity.geometry.set_insert_configuration(Some(config));
        document.model_space.push(entity);
    }
    document.assign_missing_ids();
    let request = EvaluationRequest {
        generation: document.content_generation(),
    };
    let mut cache = EvaluationCache::default();
    for trial in 0..3 {
        let started = Instant::now();
        let evaluated = materialize_evaluated(&document, &mut cache, request).unwrap();
        let elapsed = started.elapsed();
        assert_eq!(evaluated.model_space.len(), 1_000);
        assert_eq!(evaluated.blocks.len(), 2);
        let generated = evaluated.model_space[0]
            .geometry
            .insert_block_name()
            .unwrap();
        let generated_block = evaluated.block_by_name(generated).unwrap();
        assert_eq!(generated_block.entities.len(), 2_000);
        let Geometry::Line { start, .. } = generated_block.entities[0].geometry else {
            panic!("line");
        };
        assert_eq!(start.x, 200.0);
        println!(
            "trial={trial} elapsed_ms={:.3} cache_entries={}",
            elapsed.as_secs_f64() * 1000.0,
            cache.len()
        );
        std::hint::black_box(evaluated);
    }
}
