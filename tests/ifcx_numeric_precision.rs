#![cfg(feature = "ifcx")]

use ifccad::geometry_kernel::hatch::{HatchBoundary2, HatchDash, HatchEdge2, HatchFill};
use ifccad::plot_kernel::{MediaUnit, PlotPlacement};
use ifccad::text::{MTextInline, TextLayout, TextLineSpacing};
use ifccad::*;
use opencaddocument::entities::hatch::{
    BoundaryEdge, BoundaryPath, CircularArcEdge, Hatch, HatchPattern, HatchPatternLine, LineEdge,
};
use opencaddocument::entities::{Ellipse, LineSpacingStyle, MText, Text, TextHorizontalAlignment};
use opencaddocument::types::Vector2;
use opencaddocument::types::Vector3;
use opencaddocument::{Document, EntityType, Format, Ifcx};
use std::f64::consts::TAU;

fn strict(bytes: &[u8]) -> IfccadDocument {
    let d = load_ifccad_bytes(
        bytes,
        IfccadReadOptions {
            composition_policy: IfccadCompositionPolicy::RejectConflicts,
        },
    )
    .unwrap()
    .into_document();
    validate_ifccad_document(&d).unwrap();
    d
}
fn native(cad: &Document) -> IfccadDocument {
    let (bytes, losses) = Ifcx.write(cad).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    strict(&bytes)
}
fn read(source: &IfccadDocument) -> Document {
    let bytes = encode_ifccad_document(source).unwrap();
    strict(bytes.bytes());
    let (cad, losses) = Ifcx.read(bytes.bytes()).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    cad
}
fn mtext_source() -> Document {
    let mut cad = Document::new();
    let mut m = MText::new();
    m.value = "Precision".into();
    m.height = 2.123456789012345;
    m.line_spacing_style = LineSpacingStyle::Exactly;
    m.line_spacing_factor = 1.123456789012345;
    cad.add_entity(EntityType::MText(Box::new(m))).unwrap();
    cad
}

#[test]
fn derived_ellipse_axes_are_not_quantized() {
    let mut cad = Document::new();
    let major = 1_000_000_000.1234567_f64;
    cad.add_entity(EntityType::Ellipse(Ellipse::from_center_axes(
        Vector3::new(0., 0., 0.),
        Vector3::new(major, 0., 0.),
        0.6,
    )))
    .unwrap();
    let source = native(&cad);
    let IfccadEntityKind::Ellipse {
        semi_major_radius,
        semi_minor_radius,
        ..
    } = source.model.entities[0].as_native().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(semi_major_radius.to_bits(), major.hypot(0.).to_bits());
    assert_eq!(semi_minor_radius.to_bits(), (major * 0.6).to_bits());
}

#[test]
fn derived_aligned_text_length_is_not_quantized() {
    let mut cad = Document::new();
    let mut t = Text::new();
    t.value = "Aligned".into();
    t.horizontal_alignment = TextHorizontalAlignment::Aligned;
    t.insertion_point = Vector3::new(0.123456789012345, 0.234567890123456, 0.);
    t.alignment_point = Some(Vector3::new(12.3456789012345, 9.87654321098765, 0.));
    let delta = t.alignment_point.unwrap() - t.insertion_point;
    let expected = delta.x.hypot(delta.y);
    cad.add_entity(EntityType::Text(t)).unwrap();
    let source = native(&cad);
    let IfccadEntityKind::Text(t) = &source.model.entities[0].as_native().unwrap().kind else {
        panic!()
    };
    let TextLayout::Aligned { length, .. } = t.layout else {
        panic!()
    };
    assert_eq!(length.to_bits(), expected.to_bits());
}

#[test]
fn absolute_mtext_spacing_keeps_available_precision() {
    let cad = mtext_source();
    let source = native(&cad);
    let IfccadEntityKind::MText(m) = &source.model.entities[0].as_native().unwrap().kind else {
        panic!()
    };
    let Some(TextLineSpacing::Exact { distance }) = m.paragraph_format.line_spacing else {
        panic!()
    };
    let expected = 1.123456789012345 * (5. / 3.) * 2.123456789012345;
    assert_eq!(distance.to_bits(), f64::to_bits(expected));
}

#[test]
fn frame_and_separate_mtext_rotation_keep_available_precision() {
    let mut source = native(&mtext_source());
    let IfccadEntityKind::MText(m) = &mut source.model.entities[0].as_native_mut().unwrap().kind
    else {
        panic!()
    };
    let (s, c) = 0.371234567890123_f64.sin_cos();
    m.placement.x_axis = [c, s, 0.];
    m.placement.y_axis = [-s, c, 0.];
    m.rotation = 0.231234567890123;
    let expected = s.atan2(c) + m.rotation;
    let cad = read(&source);
    let EntityType::MText(m) = cad.model_space_entities().next().unwrap() else {
        panic!()
    };
    assert_eq!(m.rotation.to_bits(), expected.to_bits());
}

#[test]
fn inline_oblique_angle_uses_roundtrip_decimal_degrees() {
    let mut source = native(&mtext_source());
    let angle = 0.123456789012345_f64;
    let IfccadEntityKind::MText(m) = &mut source.model.entities[0].as_native_mut().unwrap().kind
    else {
        panic!()
    };
    let MTextInline::Run {
        character_format, ..
    } = &mut m.content[0].inlines[0]
    else {
        panic!()
    };
    character_format.oblique_angle = Some(angle);
    let cad = read(&source);
    let EntityType::MText(m) = cad.model_space_entities().next().unwrap() else {
        panic!()
    };
    assert!(
        m.value.contains(&format!("\\Q{};", angle.to_degrees())),
        "{}",
        m.value
    );
    let back = native(&cad);
    let IfccadEntityKind::MText(m) = &back.model.entities[0].as_native().unwrap().kind else {
        panic!()
    };
    let MTextInline::Run {
        character_format, ..
    } = &m.content[0].inlines[0]
    else {
        panic!()
    };
    assert_eq!(
        character_format.oblique_angle.unwrap().to_bits(),
        angle.to_degrees().to_radians().to_bits()
    );
}

#[test]
fn hatch_tiny_and_nearly_full_sweeps_keep_direction() {
    for sweep in [4e-13, TAU - 4e-13] {
        for ccw in [true, false] {
            let mut hatch = Hatch::new();
            let mut path = BoundaryPath::new();
            path.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
                center: Vector2::new(0., 0.),
                radius: 1.,
                start_angle: 0.,
                end_angle: sweep,
                counter_clockwise: ccw,
            }));
            let angle = if ccw { sweep } else { -sweep };
            path.edges.push(BoundaryEdge::Line(LineEdge {
                start: Vector2::new(angle.cos(), angle.sin()),
                end: Vector2::new(1., 0.),
            }));
            hatch.paths.push(path);
            let mut cad = Document::new();
            cad.add_entity(EntityType::Hatch(Box::new(hatch))).unwrap();
            let source = native(&cad);
            let IfccadEntityKind::Hatch(h) = &source.model.entities[0].as_native().unwrap().kind
            else {
                panic!()
            };
            let HatchBoundary2::Edges(edges) = &h.loops[0].boundary else {
                panic!()
            };
            let HatchEdge2::CircularArc {
                sweep_parameter, ..
            } = edges[0]
            else {
                panic!()
            };
            assert_eq!(sweep_parameter.to_bits(), angle.to_bits());
            assert_ne!(sweep_parameter.abs(), TAU);
            let back = read(&source);
            let EntityType::Hatch(h) = back.model_space_entities().next().unwrap() else {
                panic!()
            };
            let BoundaryEdge::CircularArc(a) = &h.paths[0].edges[0] else {
                panic!()
            };
            assert_eq!(a.counter_clockwise, ccw);
            assert_eq!((a.end_angle - a.start_angle).to_bits(), sweep.to_bits());
        }
    }
    let mut hatch = Hatch::new();
    let mut path = BoundaryPath::new();
    path.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
        center: Vector2::new(0., 0.),
        radius: 1.,
        start_angle: 0.,
        end_angle: 0.,
        counter_clockwise: true,
    }));
    hatch.paths.push(path);
    let mut cad = Document::new();
    cad.add_entity(EntityType::Hatch(Box::new(hatch))).unwrap();
    let source = native(&cad);
    let IfccadEntityKind::Hatch(h) = &source.model.entities[0].as_native().unwrap().kind else {
        panic!()
    };
    assert!(matches!(h.loops[0].boundary, HatchBoundary2::Circle { .. }));
}

#[test]
fn hatch_pattern_factorization_keeps_direct_arithmetic_and_dash_meaning() {
    let mut hatch = Hatch::new();
    hatch.is_solid = false;
    hatch.pattern = HatchPattern::new("Precision");
    hatch.pattern_angle = 0.123456789012345;
    hatch.pattern_scale = 1.23456789012345;
    let line = HatchPatternLine {
        angle: 0.567890123456789,
        base_point: Vector2::new(1.23456789012345, 2.34567890123456),
        offset: Vector2::new(3.45678901234567, 4.56789012345678),
        dash_lengths: vec![0.123456789012345, -0.234567890123456, 0.],
    };
    hatch.pattern.lines.push(line.clone());
    let mut path = BoundaryPath::new();
    path.edges.push(BoundaryEdge::CircularArc(CircularArcEdge {
        center: Vector2::new(0., 0.),
        radius: 10.,
        start_angle: 0.,
        end_angle: TAU,
        counter_clockwise: true,
    }));
    hatch.paths.push(path);
    let rotation = hatch.pattern_angle;
    let scale = hatch.pattern_scale;
    let mut cad = Document::new();
    cad.add_entity(EntityType::Hatch(Box::new(hatch))).unwrap();
    let source = native(&cad);
    let IfccadEntityKind::Hatch(h) = &source.model.entities[0].as_native().unwrap().kind else {
        panic!()
    };
    let HatchFill::LinePattern(pattern) = &h.fill else {
        panic!()
    };
    let family = &pattern.families[0];
    assert_eq!(family.angle.to_bits(), (line.angle - rotation).to_bits());
    let (s, c) = (-rotation).sin_cos();
    assert_eq!(
        family.base_point.map(f64::to_bits),
        [
            (line.base_point.x * c - line.base_point.y * s) / scale,
            (line.base_point.x * s + line.base_point.y * c) / scale
        ]
        .map(f64::to_bits)
    );
    assert_eq!(
        family.dashes,
        vec![
            HatchDash::Dash {
                length: line.dash_lengths[0] / scale
            },
            HatchDash::Gap {
                length: -line.dash_lengths[1] / scale
            },
            HatchDash::Dot
        ]
    );
    assert_eq!(
        h.classic_settings.as_ref().unwrap().rotation.to_bits(),
        rotation.to_bits()
    );
}

#[test]
fn plot_media_product_does_not_drop_mantissa_bits() {
    let mut source = strict(include_bytes!("fixtures/layout-plot-inch.ifcx"));
    let sheet = &mut source.paper_layouts[0];
    sheet.settings.plot_settings = None;
    let media = sheet.settings.media.as_mut().unwrap();
    media.unit = MediaUnit::from_token("in").unwrap();
    media.width = 11.123456789012345;
    media.height = 8.765432109876543;
    let expected = [media.width * 25.4, media.height * 25.4];
    let name = sheet.name.clone();
    let cad = read(&source);
    let sheet = cad
        .objects
        .values()
        .find_map(|o| match o {
            opencaddocument::objects::ObjectType::Layout(l) if l.name == name => Some(l),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        [sheet.paper_width, sheet.paper_height].map(f64::to_bits),
        expected.map(f64::to_bits)
    );
}

#[test]
fn plot_origin_division_does_not_drop_mantissa_bits() {
    let mut cad = read(&strict(include_bytes!("fixtures/layout-plot-inch.ifcx")));
    let sheet = cad
        .objects
        .values_mut()
        .find_map(|o| match o {
            opencaddocument::objects::ObjectType::Layout(l) if l.name != "Model" => Some(l),
            _ => None,
        })
        .unwrap();
    sheet.plot_paper_units = 0;
    sheet.plot_flags.plot_centered = false;
    sheet.plot_origin_x = 1.123456789012345;
    sheet.plot_origin_y = 2.234567890123456;
    let expected = [sheet.plot_origin_x / 25.4, sheet.plot_origin_y / 25.4];
    let source = native(&cad);
    let plot = source.paper_layouts[0]
        .settings
        .plot_settings
        .as_ref()
        .unwrap();
    let PlotPlacement::Offset { x, y, .. } = plot.mapping.placement else {
        panic!()
    };
    assert_eq!([x, y].map(f64::to_bits), expected.map(f64::to_bits));
}
