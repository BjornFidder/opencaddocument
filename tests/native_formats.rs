//! DWG and DXF carry a document through a write and a read unchanged, and
//! report no losses, since they are the model's own formats.

use opencaddocument::entities::{Circle, Line};
use opencaddocument::types::Vector3;
use opencaddocument::{Document, Dwg, Dxf, EntityType, Format};

fn drawing() -> Document {
    let mut document = Document::new();
    document
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 50.0, 0.0),
        )))
        .unwrap();
    let mut circle = Circle::new();
    circle.center = Vector3::new(30.0, 5.0, 0.0);
    circle.radius = 4.0;
    document.add_entity(EntityType::Circle(circle)).unwrap();
    document
}

fn geometry(document: &Document) -> Vec<String> {
    let mut out: Vec<String> = document
        .entities()
        .filter_map(|entity| match entity {
            EntityType::Line(l) => Some(format!("line {:?} {:?}", l.start, l.end)),
            EntityType::Circle(c) => Some(format!("circle {:?} {}", c.center, c.radius)),
            _ => None,
        })
        .collect();
    out.sort();
    out
}

fn round_trip(format: &dyn Format) {
    let original = drawing();
    let (bytes, losses) = format.write(&original).unwrap();
    assert!(losses.is_empty(), "{} write: {losses:?}", format.name());
    let (read, losses) = format.read(&bytes).unwrap();
    assert!(losses.is_empty(), "{} read: {losses:?}", format.name());
    assert_eq!(geometry(&read), geometry(&original), "{}", format.name());
}

#[test]
fn dwg_round_trip_keeps_the_drawing() {
    round_trip(&Dwg);
}

#[test]
fn dxf_round_trip_keeps_the_drawing() {
    round_trip(&Dxf);
}
