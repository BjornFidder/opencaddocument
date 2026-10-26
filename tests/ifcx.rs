//! IFCX through ifccad: what maps carries through a read and a write, and
//! everything else is reported as a loss.
#![cfg(feature = "ifcx")]

use opencaddocument::entities::{
    Arc, Circle, Ellipse, Insert, Line, LwPolyline, LwVertex, MText, Point, Polyline3D, Solid, Text,
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
            EntityType::MText(m) => format!(
                "mtext {:?} {:?} {} {} {:?}",
                m.value, m.insertion_point, m.height, m.rectangle_width, m.attachment_point
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

/// The text around the first place `a` and `b` differ.
fn difference(a: &str, b: &str) -> String {
    let at = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count();
    let window = |text: &str| text.chars().skip(at.saturating_sub(150)).take(300).collect::<String>();
    format!("\n  got: …{}…\n want: …{}…", window(a), window(b))
}

/// Each layout's name, sheet and plot settings and entities, in tab order.
fn sheets(document: &Document) -> Vec<String> {
    use opencaddocument::objects::ObjectType;
    let mut layouts: Vec<_> = document
        .objects
        .values()
        .filter_map(|object| match object {
            ObjectType::Layout(layout) => Some(layout),
            _ => None,
        })
        .collect();
    layouts.sort_by_key(|layout| layout.tab_order);
    layouts
        .into_iter()
        .map(|layout| {
            let record = document.block_records.iter().find(|r| r.handle == layout.block_record);
            let mut entities: Vec<String> = record
                .map(|record| {
                    document
                        .entities_in_block(&record.name)
                        .map(|entity| format!("{entity:?}"))
                        .collect()
                })
                .unwrap_or_default();
            entities.sort();
            rounded(format!(
                "{} {} {}x{} {} {} {:?} {:?}",
                layout.name,
                layout.tab_order,
                layout.paper_width,
                layout.paper_height,
                layout.plot_type,
                layout.plot_scale_factor,
                layout.plot_flags,
                entities
            ))
        })
        .collect()
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

/// `text` with every number rounded to nine significant digits and handle
/// values dropped (a reader allocates them in its own order).
fn rounded(text: String) -> String {
    let text = text
        .split("Handle(")
        .enumerate()
        .map(|(index, part)| match index {
            0 => part.to_string(),
            _ => format!("Handle(#{}", part.trim_start_matches(|c: char| c.is_ascii_digit())),
        })
        .collect::<String>();
    let mut out = String::new();
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut String| {
        match number.parse::<f64>() {
            Ok(value) if number.contains('.') => out.push_str(&format!("{value:.8e}")),
            _ => out.push_str(number),
        }
        number.clear();
    };
    for c in text.chars() {
        let continues = c.is_ascii_digit()
            || c == '.'
            || (c == '-' && (number.is_empty() || number.ends_with('e')))
            || (c == 'e' && !number.is_empty());
        if continues {
            number.push(c);
        } else {
            flush(&mut number, &mut out);
            out.push(c);
        }
    }
    flush(&mut number, &mut out);
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

    let mut mtext = MText::new();
    mtext.value = "{\\fArial|b1|i0;Bold} plain \\C1;red\\P\\pxqc;Centre 25%%d \\S1/2; {\\H2x;big}".to_string();
    mtext.insertion_point = Vector3::new(0.0, 40.0, 0.0);
    mtext.height = 2.5;
    mtext.rectangle_width = 30.0;
    add(&mut document, EntityType::MText(Box::new(mtext)));

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
        let converted = ["circle", "ellipse", "mtext"].iter().any(|kind| got.starts_with(kind))
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

#[test]
fn complex_line_patterns_round_trip() {
    let (document, _) = Ifcx.read(include_bytes!("fixtures/complex-line-patterns.ifcx")).unwrap();
    let audit = document.audit();
    assert_eq!(audit.error_count(), 0, "{audit:#?}");
    let complex = document
        .line_types
        .iter()
        .filter(|linetype| linetype.elements.iter().any(|e| e.complex.is_some()))
        .count();
    assert!(complex > 0, "no complex linetype was read");

    let (first, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (again, _) = Ifcx.read(&first).unwrap();
    let patterns = |document: &Document| -> Vec<String> {
        let mut out: Vec<_> = document
            .line_types
            .iter()
            .map(|linetype| {
                let elements: Vec<_> = linetype
                    .elements
                    .iter()
                    .map(|e| format!("{} {:?}", e.length, e.complex.as_ref().map(|c| (&c.content, c.scale, c.rotation, c.offset))))
                    .collect();
                format!("{} {:?}", linetype.name, elements)
            })
            .collect();
        out.sort();
        out
    };
    assert_eq!(patterns(&again), patterns(&document));
    let (second, _) = Ifcx.write(&again).unwrap();
    assert_eq!(without_timestamp(&second), without_timestamp(&first));
}

/// Every fixture: reads cleanly, writes without new losses, and its bytes
/// settle after the first save.
#[test]
fn fixtures_settle_after_the_first_save() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "ifcx"))
        .collect();
    names.sort();
    for path in names {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let (document, _) = Ifcx.read(&std::fs::read(&path).unwrap()).unwrap();
        let audit = document.audit();
        assert_eq!(audit.error_count(), 0, "{name}: {audit:#?}");
        let (first, losses) = Ifcx.write(&document).unwrap();
        assert!(losses.is_empty(), "{name}: {losses:?}");
        let (again, _) = Ifcx.read(&first).unwrap();
        // The first save settles converted lengths to about 1e-12.
        let settled = |document: &Document| geometry(document).into_iter().map(rounded).collect::<Vec<_>>();
        assert_eq!(settled(&again), settled(&document), "{name}");
        let (a, b) = (sheets(&again), sheets(&document));
        assert!(a == b, "{name}: {}", difference(&a.join("\n"), &b.join("\n")));
        // What IFCX gave is a sound DWG drawing.
        let (dwg, _) = opencaddocument::Dwg.write(&document).unwrap();
        let (from_dwg, _) = opencaddocument::Dwg.read(&dwg).unwrap();
        assert_eq!(from_dwg.audit().error_count(), 0, "{name}");
        assert_eq!(sheets(&from_dwg).len(), sheets(&document).len(), "{name}");
        let (second, _) = Ifcx.write(&again).unwrap();
        assert_eq!(without_timestamp(&second), without_timestamp(&first), "{name}");
    }
}

#[test]
fn a_patterned_hatch_round_trips() {
    use opencaddocument::entities::hatch::{
        BoundaryEdge, BoundaryPath, CircularArcEdge, Hatch, HatchPattern, HatchPatternLine,
        LineEdge,
    };
    let mut document = Document::new();
    let mut hatch = Hatch::new();
    hatch.is_solid = false;
    hatch.common.linetype = "ByLayer".to_string();
    hatch.pattern = HatchPattern::new("ANSI37");
    hatch.pattern_angle = 0.3;
    hatch.pattern_scale = 2.5;
    // Lines as a CAD stores them: turned and scaled already.
    for angle in [0.3, 0.3 + std::f64::consts::FRAC_PI_2] {
        let (sin, cos) = angle.sin_cos();
        hatch.pattern.lines.push(HatchPatternLine {
            angle,
            base_point: Vector2::new(0.0, 0.0),
            offset: Vector2::new(-sin * 0.3125, cos * 0.3125),
            dash_lengths: vec![0.625, -0.3125, 0.0, -0.3125],
        });
    }
    let mut outer = BoundaryPath::new();
    outer.edges.push(BoundaryEdge::Line(LineEdge { start: Vector2::new(0.0, 0.0), end: Vector2::new(10.0, 0.0) }));
    outer.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
        center: Vector2::new(10.0, 5.0),
        radius: 5.0,
        // Clockwise: stored angles are the mathematical ones negated, so
        // this runs from (10, 0) through (5, 5) to (10, 10).
        start_angle: std::f64::consts::FRAC_PI_2,
        end_angle: 3.0 * std::f64::consts::FRAC_PI_2,
        counter_clockwise: false,
    }));
    outer.edges.push(BoundaryEdge::Line(LineEdge { start: Vector2::new(10.0, 10.0), end: Vector2::new(0.0, 0.0) }));
    hatch.paths.push(outer);
    let mut hole = BoundaryPath::new();
    hole.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
        center: Vector2::new(5.0, 3.0),
        radius: 1.0,
        start_angle: 0.0,
        end_angle: std::f64::consts::TAU,
        counter_clockwise: true,
    }));
    hatch.paths.push(hole);
    document.add_entity(EntityType::Hatch(Box::new(hatch))).unwrap();

    let (first, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (read, losses) = Ifcx.read(&first).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let settled = |document: &Document| geometry(document).into_iter().map(rounded).collect::<Vec<_>>();
    assert_eq!(settled(&read), settled(&document));
    let (second, _) = Ifcx.write(&read).unwrap();
    assert_eq!(without_timestamp(&second), without_timestamp(&first));
}

#[test]
fn a_sheet_with_viewports_round_trips() {
    use opencaddocument::entities::Viewport;
    use opencaddocument::objects::ObjectType;
    use opencaddocument::tables::Layer;
    let mut document = Document::new();
    let mut hidden = Layer::new("Hidden");
    hidden.handle = document.allocate_handle();
    document.layers.add(hidden).unwrap();
    let hidden = document.layers.get("Hidden").unwrap().handle;
    let sheet = document
        .objects
        .values_mut()
        .find_map(|object| match object {
            ObjectType::Layout(layout) if layout.name == "Layout1" => Some(layout),
            _ => None,
        })
        .unwrap();
    sheet.paper_width = 420.0;
    sheet.paper_height = 297.0;
    sheet.paper_size = "ISO_A3_(420.00_x_297.00_MM)".to_string();
    sheet.plot_paper_units = 1;
    sheet.plot_type = 5;
    sheet.plot_margin_left = 7.5;
    sheet.plot_margin_bottom = 20.0;
    sheet.plot_margin_right = 7.5;
    sheet.plot_margin_top = 20.0;
    sheet.plot_scale_numerator = 1.0;
    sheet.plot_scale_denominator = 1.0;
    sheet.plot_scale_factor = 1.0;
    sheet.plot_flags.use_standard_scale = false;
    sheet.plot_flags.print_lineweights = true;
    let mut sheet_viewport = Viewport::new();
    sheet_viewport.id = 1;
    document.add_entity_to_layout(EntityType::Viewport(Box::new(sheet_viewport)), "Layout1").unwrap();
    let mut frame = LwPolyline::new();
    for (x, y) in [(20.0, 20.0), (200.0, 20.0), (200.0, 150.0), (20.0, 150.0)] {
        frame.vertices.push(LwVertex::new(Vector2::new(x, y)));
    }
    frame.is_closed = true;
    let frame = document.add_entity_to_layout(EntityType::LwPolyline(frame), "Layout1").unwrap();
    let mut viewport = Viewport::new();
    viewport.id = 2;
    viewport.center = Vector3::new(110.0, 85.0, 0.0);
    viewport.width = 180.0;
    viewport.height = 130.0;
    viewport.view_center = Vector3::new(500.0, 300.0, 0.0);
    viewport.view_height = 1300.0;
    viewport.frozen_layers = vec![hidden];
    viewport.status.non_rectangular_clipping = true;
    viewport.clip_boundary_handle = frame;
    document.add_entity_to_layout(EntityType::Viewport(Box::new(viewport)), "Layout1").unwrap();

    let (first, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (read, losses) = Ifcx.read(&first).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    assert_eq!(read.audit().error_count(), 0);
    let sheet = sheets(&read).join("\n");
    assert!(sheet.contains("420x297") && sheet.contains("print_lineweights: true"), "{sheet}");
    let viewport = read
        .entities()
        .find_map(|entity| match entity {
            EntityType::Viewport(v) if v.id == 2 => Some(v.clone()),
            _ => None,
        })
        .expect("model viewport");
    assert_eq!(viewport.frozen_layers, vec![read.layers.get("Hidden").unwrap().handle]);
    assert!(matches!(read.get_entity(viewport.clip_boundary_handle), Some(EntityType::LwPolyline(_))));
    assert_eq!((viewport.view_height, viewport.width), (1300.0, 180.0));
    let (second, _) = Ifcx.write(&read).unwrap();
    assert_eq!(without_timestamp(&second), without_timestamp(&first));
}
