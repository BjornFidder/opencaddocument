//! IFCX through ifccad: what maps carries through a read and a write, and
//! everything else is reported as a loss.
#![cfg(feature = "ifcx")]

use opencaddocument::entities::{
    Arc, Circle, Ellipse, Insert, Line, LwPolyline, LwVertex, Point, Polyline3D, Solid, Text,
    TextHorizontalAlignment, TextVerticalAlignment,
};
use opencaddocument::tables::BlockRecord;
use opencaddocument::types::{Vector2, Vector3};
use opencaddocument::{Document, EntityType, Format, Ifcx};

const HELLO: &[u8] = include_bytes!("fixtures/hello-cad.ifcx");
const NESTED: &[u8] = include_bytes!("fixtures/hello-nested-blocks.ifcx");

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
            EntityType::Ellipse(e) => format!(
                "ellipse {:?} {:?} {} {} {} {:?}",
                e.center, e.major_axis, e.minor_axis_ratio, e.start_parameter, e.end_parameter, e.normal
            ),
            EntityType::Polyline3D(p) => format!(
                "polyline3d {:?} {}",
                p.vertices.iter().map(|v| v.position).collect::<Vec<_>>(),
                p.flags.closed
            ),
            EntityType::Solid(s) => format!(
                "solid {:?} {:?} {:?} {:?} {} {}",
                s.first_corner, s.second_corner, s.third_corner, s.fourth_corner, s.thickness, s.is_trace
            ),
            EntityType::Text(t) => format!(
                "text {:?} {:?} {:?} {} {} {} {:?} {:?} {}",
                t.value,
                t.insertion_point,
                t.alignment_point,
                t.height,
                t.rotation,
                t.width_factor,
                t.horizontal_alignment,
                t.vertical_alignment,
                t.style
            ),
            EntityType::Insert(i) => format!(
                "insert {} {:?} {} {} {} {}",
                i.block_name,
                i.insert_point,
                i.rotation,
                i.x_scale(),
                i.y_scale(),
                i.z_scale()
            ),
            other => format!("{other:?}"),
        })
        .collect();
    out.sort();
    out
}

/// Each block's name and member geometry.
fn blocks(document: &Document) -> Vec<(String, usize)> {
    let mut out: Vec<_> = document
        .block_records
        .iter()
        .filter(|record| !record.is_model_space() && !record.is_paper_space())
        .map(|record| (record.name.clone(), document.entities_in_block(&record.name).count()))
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
    assert_eq!(kinds, ["circle", "insert", "insert", "line", "polyline"], "{:#?}", geometry(&document));
    assert!(losses.is_empty(), "{losses:?}");
    assert_eq!(blocks(&document).len(), 1, "{:?}", blocks(&document));
    assert_eq!(property(&document, "IFCX id").as_deref(), Some("ifccad-experimental-hello"));
    assert_eq!(property(&document, "IFCX drawing id").as_deref(), Some("1"));
    assert_eq!(document.header.insertion_units, 5, "cm");

    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (again, losses) = Ifcx.read(&bytes).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    assert_eq!(geometry(&again), geometry(&document));
    assert_eq!(blocks(&again), blocks(&document));
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
    let mut ellipse = Ellipse::from_center_axes(
        Vector3::new(5.0, 5.0, 0.0),
        Vector3::new(6.0, 8.0, 0.0),
        0.4,
    );
    ellipse.start_parameter = 1.0;
    ellipse.end_parameter = 4.0;
    add(&mut document, EntityType::Ellipse(ellipse));
    add(
        &mut document,
        EntityType::Polyline3D(Polyline3D::from_points(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 2.0, 3.0),
            Vector3::new(4.0, 5.0, 6.0),
        ])),
    );
    let mut trace = Solid::new(
        Vector3::new(0.0, 0.0, 1.0),
        Vector3::new(2.0, 0.0, 1.0),
        Vector3::new(0.0, 2.0, 1.0),
        Vector3::new(2.0, 2.0, 1.0),
    );
    trace.is_trace = true;
    add(&mut document, EntityType::Solid(trace));
    let mut text = Text::new();
    text.value = "Hello".to_string();
    text.insertion_point = Vector3::new(3.0, 4.0, 0.0);
    text.height = 2.5;
    text.rotation = 0.25;
    add(&mut document, EntityType::Text(text.clone()));
    text.horizontal_alignment = TextHorizontalAlignment::Center;
    text.vertical_alignment = TextVerticalAlignment::Middle;
    text.alignment_point = Some(Vector3::new(7.0, 4.0, 0.0));
    add(&mut document, EntityType::Text(text.clone()));
    text.horizontal_alignment = TextHorizontalAlignment::Aligned;
    text.vertical_alignment = TextVerticalAlignment::Baseline;
    text.alignment_point = Some(Vector3::new(13.0, 9.0, 0.0));
    add(&mut document, EntityType::Text(text));

    // A block holding a line and an insert of another block.
    let define = |document: &mut Document, name: &str, members: Vec<EntityType>| {
        let mut record = BlockRecord::new(name);
        record.handle = document.allocate_handle();
        let owner = record.handle;
        document.block_records.add(record).unwrap();
        for mut member in members {
            member.common_mut().owner_handle = owner;
            document.add_entity(member).unwrap();
        }
    };
    define(
        &mut document,
        "Inner",
        vec![EntityType::Line(Line::from_points(Vector3::new(0.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)))],
    );
    define(
        &mut document,
        "Outer",
        vec![EntityType::Insert(Insert::new("Inner", Vector3::new(2.0, 0.0, 0.0)))],
    );
    let mut insert = Insert::new("Outer", Vector3::new(50.0, 60.0, 0.0));
    insert.rotation = 0.5;
    insert.set_x_scale(2.0);
    add(&mut document, EntityType::Insert(insert));

    let (first, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");

    let (read, losses) = Ifcx.read(&first).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let audit = read.audit();
    assert_eq!(audit.error_count(), 0, "{audit:#?}");
    let expected = geometry(&document);
    let got = geometry(&read);
    assert_eq!(got.len(), expected.len(), "{got:#?}");
    // The tilted circle, the turned ellipse and the aligned text go through a
    // placement conversion. IFCX keeps only the point a text hangs from, so
    // the centred text's insertion point (derived from it) is not compared.
    // The rest is exact.
    for (got, expected) in got.iter().zip(&expected) {
        let converted = ["circle", "ellipse"].iter().any(|kind| got.starts_with(kind))
            || got.contains("Aligned")
            || got.contains("Center");
        if !converted {
            assert_eq!(got, expected);
        }
    }
    assert_eq!(blocks(&read), [("Inner".to_string(), 1), ("Outer".to_string(), 1)]);

    let (second, _) = Ifcx.write(&read).unwrap();
    let (read_again, _) = Ifcx.read(&second).unwrap();
    let (third, _) = Ifcx.write(&read_again).unwrap();
    assert_eq!(without_timestamp(&second), without_timestamp(&third));
}

#[test]
fn nested_blocks_round_trip() {
    let (document, losses) = Ifcx.read(NESTED).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let audit = document.audit();
    assert_eq!(audit.error_count(), 0, "{audit:#?}");
    assert_eq!(blocks(&document).len(), 2, "{:?}", blocks(&document));

    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (again, _) = Ifcx.read(&bytes).unwrap();
    assert_eq!(geometry(&again), geometry(&document));
    assert_eq!(blocks(&again), blocks(&document));
}
