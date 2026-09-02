use crate::tessellate_document;
use cad_core::{CadColor, Document, Entity, Geometry, Layer, Point3};

#[test]
fn tessellates_line_into_two_vertices() {
    let mut document = Document::default();
    document.layers.insert(
        "0".into(),
        Layer {
            name: "0".into(),
            visible: true,
            frozen: false,
            color: CadColor::Aci(7),
            linetype: "CONTINUOUS".into(),
        },
    );
    document.model_space.push(Entity::new(Geometry::Line {
        start: Point3::from_xy(0.0, 0.0),
        end: Point3::from_xy(10.0, 0.0),
    }));
    let list = tessellate_document(&document);
    assert_eq!(list.line_count(), 1);
}
