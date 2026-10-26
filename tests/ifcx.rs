//! IFCX through ifccad: what maps carries through a read and a write, and
//! everything else is reported as a loss.
#![cfg(feature = "ifcx")]

use opencaddocument::entities::{Arc, Circle, Line, LwPolyline, LwVertex, Point, Text};
use opencaddocument::types::{Vector2, Vector3};
use opencaddocument::{Document, EntityType, Format, Ifcx};

const HELLO: &[u8] = include_bytes!("fixtures/hello-cad.ifcx");

fn property(document: &Document, name: &str) -> Option<String> {
    let properties = &document.summary_info.custom_properties;
    properties.iter().find(|(key, _)| key == name).map(|(_, value)| value.clone())
}

fn geometry(document: &Document) -> Vec<String> {
    let mut out: Vec<String> = document
        .model_space_entities()
        .map(|entity| match entity {
            EntityType::Line(l) => format!("line {:?} {:?} {}", l.start, l.end, l.common.layer),
            EntityType::Circle(c) => format!("circle {:?} {:?} {}", c.center, c.normal, c.radius),
            EntityType::Arc(a) => format!(
                "arc {:?} {:?} {} {} {}",
                a.center, a.normal, a.radius, a.start_angle, a.end_angle
            ),
            EntityType::Point(p) => format!("point {:?}", p.location),
            EntityType::LwPolyline(p) => format!(
                "polyline {:?} {} {} {:?}",
                p.vertices
                    .iter()
                    .map(|v| (v.location.x, v.location.y, v.bulge, v.start_width, v.end_width))
                    .collect::<Vec<_>>(),
                p.is_closed,
                p.elevation,
                p.normal
            ),
            other => format!("{other:?}"),
        })
        .collect();
    out.sort();
    out
}

/// Bytes with the write timestamp blanked, which is the only part allowed to
/// change between saves.
fn without_timestamp(bytes: &[u8]) -> String {
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let start = text.find("\"timestamp\"").unwrap();
    let end = start + text[start..].find(',').unwrap_or(text.len() - start);
    format!("{}{}", &text[..start], &text[end..])
}

#[test]
fn ifccad_fixture_reads_and_keeps_its_identity() {
    let (document, losses) = Ifcx.read(HELLO).unwrap();
    let audit = document.audit();
    assert_eq!(audit.error_count(), 0, "{audit:#?}");
    let kinds: Vec<_> = geometry(&document)
        .iter()
        .map(|g| g.split(' ').next().unwrap().to_string())
        .collect();
    assert_eq!(kinds, ["circle", "line", "polyline"], "{:#?}", geometry(&document));
    let messages: Vec<_> = losses.iter().map(|loss| loss.message.as_str()).collect();
    assert!(messages.iter().any(|m| m.starts_with("BlockInstance")), "{messages:?}");
    assert!(messages.iter().any(|m| m.starts_with("block definitions")), "{messages:?}");
    assert_eq!(property(&document, "IFCX id").as_deref(), Some("ifccad-experimental-hello"));
    assert_eq!(property(&document, "IFCX drawing id").as_deref(), Some("1"));
    assert_eq!(document.header.insertion_units, 5, "cm");

    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (again, losses) = Ifcx.read(&bytes).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    assert_eq!(geometry(&again), geometry(&document));
    assert_eq!(property(&again, "IFCX id"), property(&document, "IFCX id"));
    assert_eq!(again.header.insertion_units, 5);
}

#[test]
fn a_drawing_round_trips_and_is_byte_stable_after_the_first_save() {
    let mut document = Document::new();
    document.header.insertion_units = 4;
    let add = |document: &mut Document, entity| {
        document.add_entity(entity).unwrap();
    };
    add(
        &mut document,
        EntityType::Line(Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 50.0, 0.0))),
    );
    let mut circle = Circle::new();
    circle.center = Vector3::new(30.0, 5.0, 2.0);
    circle.normal = Vector3::new(0.3, -0.2, 0.9).normalize();
    circle.radius = 4.0;
    add(&mut document, EntityType::Circle(circle));
    let mut arc = Arc::new();
    arc.center = Vector3::new(-10.0, 20.0, 0.0);
    arc.radius = 7.5;
    arc.start_angle = 0.5;
    arc.end_angle = 2.0;
    add(&mut document, EntityType::Arc(arc));
    let mut point = Point::new();
    point.location = Vector3::new(1.0, 2.0, 3.0);
    add(&mut document, EntityType::Point(point));
    let mut polyline = LwPolyline::new();
    for (x, y, bulge) in [(0.0, 0.0, 0.0), (10.0, 0.0, 0.5), (10.0, 10.0, 0.0)] {
        let mut vertex = LwVertex::new(Vector2::new(x, y));
        vertex.bulge = bulge;
        polyline.vertices.push(vertex);
    }
    polyline.is_closed = true;
    polyline.elevation = 2.5;
    add(&mut document, EntityType::LwPolyline(polyline));
    add(&mut document, EntityType::Text(Text::new()));

    let (first, losses) = Ifcx.write(&document).unwrap();
    let messages: Vec<_> = losses.iter().map(|loss| loss.message.as_str()).collect();
    assert_eq!(messages, ["TEXT: not converted"], "{messages:?}");

    let (read, losses) = Ifcx.read(&first).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let audit = read.audit();
    assert_eq!(audit.error_count(), 0, "{audit:#?}");
    let expected: Vec<_> = geometry(&document).into_iter().filter(|g| !g.starts_with("Text")).collect();
    let got = geometry(&read);
    assert_eq!(got.len(), expected.len(), "{got:#?}");
    // The tilted circle goes through a placement conversion; the rest is exact.
    for (got, expected) in got.iter().zip(&expected) {
        if !got.starts_with("circle") {
            assert_eq!(got, expected);
        }
    }

    let (second, _) = Ifcx.write(&read).unwrap();
    let (read_again, _) = Ifcx.read(&second).unwrap();
    let (third, _) = Ifcx.write(&read_again).unwrap();
    assert_eq!(without_timestamp(&second), without_timestamp(&third));
}
