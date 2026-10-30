#![cfg(feature = "ifcx")]

use ifccad::*;
use opencaddocument::entities::{Block, BlockEnd, Line};
use opencaddocument::tables::BlockRecord;
use opencaddocument::types::Vector3;
use opencaddocument::{Document, EntityType, Format, Ifcx};

fn frame(origin: [f64; 3]) -> IfccadPlacement {
    IfccadPlacement {
        origin,
        x_axis: [1., 0., 0.],
        y_axis: [0., 1., 0.],
    }
}

fn load(bytes: &[u8]) -> IfccadDocument {
    let drawing = load_ifccad_bytes(
        bytes,
        IfccadReadOptions {
            composition_policy: IfccadCompositionPolicy::RejectConflicts,
        },
    )
    .unwrap()
    .into_document();
    validate_ifccad_document(&drawing).unwrap();
    drawing
}

fn source() -> IfccadDocument {
    let mut drawing = load(include_bytes!("fixtures/dimension-graphics.ifcx"));
    let mut seen = false;
    drawing.model.entities.retain_mut(|entity| {
        if let IfccadEntityKind::Dimension(d) = &mut entity.as_native_mut().unwrap().kind {
            if seen {
                return false;
            }
            seen = true;
            d.placement = frame([16., 8., 0.]);
            d.graphics = None;
        }
        true
    });
    drawing
}

fn dimension(drawing: &IfccadDocument) -> &IfccadDimension {
    drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Dimension(d) => Some(d.as_ref()),
            _ => None,
        })
        .unwrap()
}

fn dimension_mut(drawing: &mut IfccadDocument) -> &mut IfccadDimension {
    drawing
        .model
        .entities
        .iter_mut()
        .find_map(|e| match &mut e.as_native_mut()?.kind {
            IfccadEntityKind::Dimension(d) => Some(d.as_mut()),
            _ => None,
        })
        .unwrap()
}

fn part(kind: IfccadDimensionGraphicsKind) -> IfccadDimensionGraphicsPart {
    IfccadDimensionGraphicsPart {
        kind,
        layer_id: None,
        visible: true,
        line_pattern_scale: 1.,
        purpose: IfccadDimensionGraphicsPurpose::Display,
        appearance: IfccadEntityAppearance {
            color: IfccadMode::ByLayer,
            opacity: IfccadMode::ByLayer,
            line_pattern: IfccadMode::ByLayer,
            line_weight: IfccadMode::ByLayer,
        },
    }
}

fn graphics(parts: Vec<IfccadDimensionGraphicsPart>) -> IfccadDimensionGraphics {
    IfccadDimensionGraphics {
        origin: IfccadDimensionGraphicsOrigin::SourceSnapshot,
        parts,
    }
}

fn read(drawing: &IfccadDocument) -> (Document, Vec<opencaddocument::Loss>) {
    let encoded = encode_ifccad_document(drawing).unwrap();
    Ifcx.read(encoded.bytes()).unwrap()
}

fn picture(document: &Document) -> &str {
    document
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().block_name.as_str()),
            _ => None,
        })
        .unwrap()
}

fn add_picture(document: &mut Document, parts: Vec<EntityType>) {
    let mut record = BlockRecord::new("*D1");
    record.flags.anonymous = true;
    record.handle = document.allocate_handle();
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    let mut start = Block::new("*D1", Vector3::new(0., 0., 0.));
    start.common.handle = record.block_entity_handle;
    start.common.owner_handle = record.handle;
    let mut end = BlockEnd::new();
    end.common.handle = record.block_end_handle;
    end.common.owner_handle = record.handle;
    document.block_records.add(record.clone()).unwrap();
    document.add_entity(EntityType::Block(start)).unwrap();
    document.add_entity(EntityType::BlockEnd(end)).unwrap();
    for mut part in parts {
        part.common_mut().owner_handle = record.handle;
        document.add_entity(part).unwrap();
    }
    let handle = document
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().common.handle),
            _ => None,
        })
        .unwrap();
    let EntityType::Dimension(d) = document.get_entity_mut(handle).unwrap() else {
        panic!()
    };
    d.base_mut().block_name = "*D1".into();
}

/// Writing succeeds and reports the dimension's picture as left out.
fn picture_dropped(document: &Document) -> bool {
    let (_, losses) = Ifcx.write(document).unwrap();
    losses.iter().any(|loss| loss.message.contains("graphics"))
}

fn line() -> EntityType {
    EntityType::Line(Line::from_points(
        Vector3::new(16., 9., 0.),
        Vector3::new(28., 9., 0.),
    ))
}

#[test]
fn native_picture_is_attached_and_returns_with_its_scope_coordinates() {
    let mut drawing = source();
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(
        IfccadDimensionGraphicsKind::LineSegment {
            start: [0., 1., 0.],
            end: [12., 1., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        },
    )]));
    let (document, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    assert!(!picture(&document).is_empty());
    assert_eq!(document.audit().error_count(), 0);
    let members: Vec<_> = document.entities_in_block(picture(&document)).collect();
    assert_eq!(members.len(), 1);
    let EntityType::Line(line) = members[0] else {
        panic!()
    };
    assert_eq!(line.start, Vector3::new(16., 9., 0.));
    assert_eq!(line.end, Vector3::new(28., 9., 0.));
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let restored = load(&bytes);
    let d = dimension(&restored);
    let picture = d.graphics.as_ref().unwrap();
    assert_eq!(picture.parts.len(), 1);
    let IfccadDimensionGraphicsKind::LineSegment { start, end, .. } = picture.parts[0].kind else {
        panic!()
    };
    assert_eq!(start, [0., -299., 0.]);
    assert_eq!(end, [12., -299., 0.]);
    assert_eq!(d.placement.origin, [16., 308., 0.]);
    assert_eq!(restored.model.entities.len(), drawing.model.entities.len());
}

#[test]
fn linked_cad_picture_is_captured_and_recreated() {
    let (mut document, _) = read(&source());
    add_picture(&mut document, vec![line()]);
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let native = load(&bytes);
    assert_eq!(dimension(&native).graphics.as_ref().unwrap().parts.len(), 1);
    assert!(native.blocks.is_empty());
    let (restored, losses) = Ifcx.read(&bytes).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let EntityType::Line(restored_line) = restored
        .entities_in_block(picture(&restored))
        .next()
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(restored_line.start, Vector3::new(16., 9., 0.));
}

#[test]
fn absent_empty_and_generated_empty_remain_distinct() {
    let mut drawing = source();
    let (absent, losses) = read(&drawing);
    assert!(losses.is_empty());
    assert!(picture(&absent).is_empty());
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![]));
    let (empty, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    assert!(!picture(&empty).is_empty());
    let (bytes, losses) = Ifcx.write(&empty).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    assert!(dimension(&load(&bytes))
        .graphics
        .as_ref()
        .unwrap()
        .parts
        .is_empty());
    dimension_mut(&mut drawing)
        .graphics
        .as_mut()
        .unwrap()
        .origin = IfccadDimensionGraphicsOrigin::Generated;
    let (generated, losses) = read(&drawing);
    assert_eq!(losses.len(), 1, "{losses:?}");
    assert!(losses[0].message.contains("provenance"));
    assert!(!picture(&generated).is_empty());
}

#[test]
fn a_bad_sibling_omits_the_whole_picture() {
    let (mut document, _) = read(&source());
    let mut bad = Line::from_points(Vector3::new(0., 0., 0.), Vector3::new(1., 0., 0.));
    bad.common.shadow_flags = 1; // thickness is supported by the new profile.
    add_picture(&mut document, vec![line(), EntityType::Line(bad)]);
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(
        losses.iter().any(|l| l.message.contains("graphics")),
        "{losses:?}"
    );
    let restored = load(&bytes);
    assert!(dimension(&restored).graphics.is_none());
    assert!(restored.blocks.is_empty());
}

fn simple_parts(drawing: &IfccadDocument) -> Vec<IfccadDimensionGraphicsPart> {
    use ifccad::text::*;
    let p = frame([2., 4., 0.]);
    let q = IfccadFilledQuadrilateral {
        placement: p.clone(),
        corners: [[0., 0.], [2., 0.], [0., 1.], [0., 1.]],
        thickness: 0.,
    };
    let style_id = drawing.text_styles[0].id;
    vec![
        part(IfccadDimensionGraphicsKind::Point {
            placement: p.clone(),
            thickness: 0.,
        }),
        part(IfccadDimensionGraphicsKind::Circle {
            placement: p.clone(),
            radius: 1.5,
            thickness: 0.,
        }),
        part(IfccadDimensionGraphicsKind::Arc {
            placement: p.clone(),
            radius: 2.,
            start_parameter: 0.,
            sweep_parameter: 1.,
            thickness: 0.,
        }),
        part(IfccadDimensionGraphicsKind::Ellipse {
            placement: p.clone(),
            semi_major_radius: 2.,
            semi_minor_radius: 1.,
        }),
        part(IfccadDimensionGraphicsKind::EllipseArc {
            placement: p.clone(),
            semi_major_radius: 2.,
            semi_minor_radius: 1.,
            start_parameter: 0.,
            sweep_parameter: 1.,
        }),
        part(IfccadDimensionGraphicsKind::PlanarPolyline {
            placement: p.clone(),
            vertices: vec![[0., 0.], [2., 1.]],
            bulges: vec![0.5, 0.],
            closed: false,
            widths: IfccadPlanarPolylineWidths::Constant { width: 0.25 },
            line_pattern_generation: IfccadLinePatternGeneration::Continuous,
            thickness: 0.,
        }),
        part(IfccadDimensionGraphicsKind::SpatialPolyline {
            vertices: vec![[0., 0., 1.], [2., 1., 2.]],
            closed: false,
            line_pattern_generation: IfccadLinePatternGeneration::Continuous,
        }),
        part(IfccadDimensionGraphicsKind::Solid(q.clone())),
        part(IfccadDimensionGraphicsKind::Trace(q)),
        part(IfccadDimensionGraphicsKind::Text(IfccadText {
            style_id,
            placement: p.clone(),
            rotation: 0.,
            backward: false,
            upside_down: false,
            layout: TextLayout::Anchored {
                horizontal: TextHorizontalAlignment::Left,
                vertical: TextVerticalAlignment::Baseline,
                height: 2.,
                width_factor: 1.,
            },
            oblique_angle: 0.,
            thickness: 0.,
            content: vec![TextRun {
                text: "1200 mm".into(),
                underline: false,
                overline: false,
                strike_through: false,
            }],
        })),
        part(IfccadDimensionGraphicsKind::MText(Box::new(IfccadMText {
            style_id,
            placement: p,
            rotation: 0.,
            backward: false,
            upside_down: false,
            height: 2.,
            attachment: MTextAttachment::TopLeft,
            flow: MTextFlow::ByStyle,
            wrap_width: None,
            columns: None,
            background: None,
            character_format: Default::default(),
            paragraph_format: Default::default(),
            content: vec![MTextParagraph {
                inlines: vec![MTextInline::Run {
                    text: "1200 mm".into(),
                    character_format: Default::default(),
                }],
                ..Default::default()
            }],
        }))),
    ]
}

#[test]
fn primitive_text_and_width_families_transfer_as_one_complete_picture() {
    let mut drawing = source();
    let parts = simple_parts(&drawing);
    let count = parts.len();
    dimension_mut(&mut drawing).graphics = Some(graphics(parts));
    let (document, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    assert_eq!(
        document.entities_in_block(picture(&document)).count(),
        count
    );
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let native = load(&bytes);
    assert_eq!(
        dimension(&native).graphics.as_ref().unwrap().parts.len(),
        count
    );
    let (again, losses) = Ifcx.read(&bytes).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    assert_eq!(again.entities_in_block(picture(&again)).count(), count);
}

#[test]
fn five_thick_primitive_families_survive_saved_picture_file_readback() {
    let mut drawing = source();
    let mut parts = simple_parts(&drawing);
    parts.push(part(IfccadDimensionGraphicsKind::LineSegment {
        start: [0., 0., 0.],
        end: [2., 1., 0.],
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    }));
    for p in &mut parts {
        match &mut p.kind {
            IfccadDimensionGraphicsKind::Point { thickness, .. }
            | IfccadDimensionGraphicsKind::Circle { thickness, .. }
            | IfccadDimensionGraphicsKind::Arc { thickness, .. }
            | IfccadDimensionGraphicsKind::PlanarPolyline { thickness, .. } => *thickness = -1.25,
            IfccadDimensionGraphicsKind::LineSegment {
                thickness,
                extrusion_direction,
                ..
            } => {
                *thickness = -1.25;
                *extrusion_direction = [0., -0.6, 0.8];
            }
            _ => {}
        }
    }
    dimension_mut(&mut drawing).graphics = Some(graphics(parts));
    let (cad, _) = read(&drawing);
    for format in [&opencaddocument::Dxf as &dyn Format, &opencaddocument::Dwg] {
        let bytes = format.write(&cad).unwrap().0;
        let read = format.read(&bytes).unwrap().0;
        let bytes = Ifcx.write(&read).unwrap().0;
        let native = load(&bytes);
        let graphic = dimension(&native)
            .graphics
            .as_ref()
            .expect("whole picture retained");
        let hs: Vec<_> = graphic
            .parts
            .iter()
            .filter_map(|p| match &p.kind {
                IfccadDimensionGraphicsKind::Point { thickness, .. }
                | IfccadDimensionGraphicsKind::Circle { thickness, .. }
                | IfccadDimensionGraphicsKind::Arc { thickness, .. }
                | IfccadDimensionGraphicsKind::PlanarPolyline { thickness, .. }
                | IfccadDimensionGraphicsKind::LineSegment { thickness, .. } => Some(*thickness),
                _ => None,
            })
            .collect();
        assert_eq!(hs, vec![-1.25; 5], "{}", format.name());
    }
}

#[test]
fn physical_dxf_and_dwg_readback_keeps_the_picture() {
    let (mut document, _) = read(&source());
    add_picture(&mut document, vec![line()]);
    for format in [&opencaddocument::Dxf as &dyn Format, &opencaddocument::Dwg] {
        let (bytes, losses) = format.write(&document).unwrap();
        assert!(losses.is_empty());
        let (cad, losses) = format.read(&bytes).unwrap();
        assert!(losses.is_empty());
        let (bytes, losses) = Ifcx.write(&cad).unwrap();
        assert!(losses.is_empty(), "{}: {losses:?}", format.name());
        let native = load(&bytes);
        assert_eq!(dimension(&native).graphics.as_ref().unwrap().parts.len(), 1);
        let (again, losses) = Ifcx.read(&bytes).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let EntityType::Line(line) = again.entities_in_block(picture(&again)).next().unwrap()
        else {
            panic!()
        };
        assert_eq!(line.start, Vector3::new(16., 9., 0.));
    }
}

#[test]
fn source_helpers_and_explicit_layer_zero_are_never_silently_changed() {
    for explicit in [false, true] {
        let mut drawing = source();
        let mut helper = part(IfccadDimensionGraphicsKind::Point {
            placement: frame([1., 2., 0.]),
            thickness: 0.,
        });
        helper.purpose = IfccadDimensionGraphicsPurpose::ReferencePoint;
        helper.layer_id = explicit.then_some(0);
        dimension_mut(&mut drawing).graphics = Some(graphics(vec![
            part(IfccadDimensionGraphicsKind::LineSegment {
                start: [0., 0., 0.],
                end: [1., 0., 0.],
                thickness: 0.,
                extrusion_direction: [0., 0., 1.],
            }),
            helper,
        ]));
        let (document, losses) = read(&drawing);
        assert_eq!(losses.len(), 1, "{losses:?}");
        assert!(picture(&document).is_empty());
        assert!(!document
            .block_records
            .iter()
            .any(|b| b.name.starts_with("*D")));
    }
}

#[test]
fn translated_and_rotated_dimension_frames_keep_owner_geometry() {
    for tilted in [false, true] {
        let mut drawing = source();
        let d = dimension_mut(&mut drawing);
        if tilted {
            d.placement.x_axis = [0., 1., 0.];
            d.placement.y_axis = [-1., 0., 0.];
        } else {
            d.placement.origin = [0.2, 0., 0.];
        }
        d.graphics = Some(graphics(vec![part(
            IfccadDimensionGraphicsKind::LineSegment {
                start: [0.1, 0., 0.],
                end: [1., 0., 0.],
                thickness: 0.,
                extrusion_direction: [0., 0., 1.],
            },
        )]));
        let (document, losses) = read(&drawing);
        assert!(!picture(&document).is_empty(), "{losses:?}");
        let EntityType::Line(line) = document
            .entities_in_block(picture(&document))
            .next()
            .unwrap()
        else {
            panic!()
        };
        let expected = if tilted {
            [16., 8.1, 0.]
        } else {
            [0.3, 0., 0.]
        };
        for (a, b) in [line.start.x, line.start.y, line.start.z]
            .into_iter()
            .zip(expected)
        {
            assert!((a - b).abs() <= 1e-7, "{line:?}");
        }
    }
}

#[test]
fn arbitrary_picture_member_xdata_is_a_located_metadata_loss() {
    use opencaddocument::acadrust::xdata::{ExtendedDataRecord, XDataValue};
    let (mut document, _) = read(&source());
    let mut member = line();
    let mut record = ExtendedDataRecord::new("SYNTHETIC_LINKS");
    record.add_value(XDataValue::String("meaningful-source-reference".into()));
    member.common_mut().extended_data.add_record(record);
    add_picture(&mut document, vec![member]);
    let original = document.clone();
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert_eq!(document, original);
    assert!(dimension(&load(&bytes)).graphics.is_some(), "{losses:?}");
    assert!(
        losses.iter().any(|l| l.message.contains("graphics")
            && l.message.contains("p0")
            && l.message.contains("SYNTHETIC_LINKS")
            && l.message.contains("XDATA")),
        "{losses:?}"
    );
}

#[test]
fn canvas_background_mtext_retains_layout_and_diagnoses_dormant_colour() {
    use opencaddocument::acadrust::entities::{LineSpacingStyle, MText};
    use opencaddocument::acadrust::types::Color;
    for canvas in [true, false] {
        let (mut document, _) = read(&source());
        let mut text = MText::new();
        text.value = "1234".into();
        text.insertion_point = Vector3::new(16.1, 8.2, 0.);
        text.height = 125.;
        text.rectangle_width = 731.25;
        text.line_spacing_factor = 1.1425788497217069;
        text.line_spacing_style = LineSpacingStyle::Exactly;
        text.background_fill_flags = if canvas { 3 } else { 1 };
        text.background_color = Color::Rgb {
            r: 200,
            g: 200,
            b: 200,
        };
        add_picture(&mut document, vec![EntityType::MText(Box::new(text))]);
        let (bytes, losses) = Ifcx.write(&document).unwrap();
        assert!(dimension(&load(&bytes)).graphics.is_some(), "{losses:?}");
        let (back, _) = Ifcx.read(&bytes).unwrap();
        let EntityType::MText(t) = back.entities_in_block(picture(&back)).next().unwrap() else {
            panic!()
        };
        assert!((t.line_spacing_factor - 1.1425788497217069).abs() * t.height < 1e-7);
        assert_eq!(t.background_fill_flags, if canvas { 3 } else { 1 });
        if canvas {
            assert!(
                losses
                    .iter()
                    .any(|l| l.message.contains("background colour")),
                "{losses:?}"
            );
        } else {
            assert_eq!(
                t.background_color,
                Color::Rgb {
                    r: 200,
                    g: 200,
                    b: 200
                }
            );
        }
    }
}

#[test]
fn rotated_mtext_picture_keeps_direction_anchor_and_native_dimension() {
    use opencaddocument::acadrust::entities::MText;
    for angle in [std::f64::consts::FRAC_PI_2, -0.37, 2.6] {
        let (mut document, _) = read(&source());
        let mut text = MText::new();
        text.value = "rotated synthetic dimension".into();
        text.height = 2.5;
        text.rectangle_width = 40.;
        text.insertion_point = Vector3::new(12345.123456789, -456.987654321, 3.);
        text.rotation = if angle == 2.6 { 0. } else { angle };
        text.dwg_x_direction = Some(Vector3::new(angle.cos(), angle.sin(), 0.));
        add_picture(
            &mut document,
            vec![EntityType::MText(Box::new(text.clone()))],
        );
        let (bytes, losses) = Ifcx.write(&document).unwrap();
        assert!(dimension(&load(&bytes)).graphics.is_some(), "{losses:?}");
        if angle == 2.6 {
            assert!(
                losses
                    .iter()
                    .any(|l| l.message.contains("inactive MText angle")),
                "{losses:?}"
            );
        }
        let (back, losses) = Ifcx.read(&bytes).unwrap();
        assert!(!picture(&back).is_empty(), "{losses:?}");
        let EntityType::MText(t) = back.entities_in_block(picture(&back)).next().unwrap() else {
            panic!()
        };
        assert!((t.insertion_point - text.insertion_point).length() < 1e-7);
        let direction = t
            .dwg_x_direction
            .unwrap_or_else(|| Vector3::new(t.rotation.cos(), t.rotation.sin(), 0.));
        assert!((direction - text.dwg_x_direction.unwrap()).length() < 1e-12);
    }
}

#[test]
fn finite_translation_rounding_keeps_the_picture() {
    let mut drawing = source();
    dimension_mut(&mut drawing).placement.origin = [1e16, 0., 0.];
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(
        IfccadDimensionGraphicsKind::LineSegment {
            start: [0.1, 0., 0.],
            end: [1., 0., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        },
    )]));
    let (document, losses) = read(&drawing);
    assert!(!picture(&document).is_empty(), "{losses:?}");
    assert!(
        losses
            .iter()
            .any(|l| l.message.contains("retained with numerical rounding")),
        "{losses:?}"
    );
}

#[test]
fn unsupported_visual_common_state_still_drops_atomically() {
    let (mut document, _) = read(&source());
    let mut member = line();
    member.common_mut().shadow_flags = 1;
    add_picture(&mut document, vec![line(), member]);
    assert!(picture_dropped(&document));
}

#[test]
fn dangling_picture_links_and_nonfinite_picture_coordinates_leave_the_picture_out() {
    let (mut document, _) = read(&source());
    let mut bad = Line::from_points(Vector3::new(f64::NAN, 0., 0.), Vector3::new(1., 0., 0.));
    bad.common.layer = "0".into();
    add_picture(&mut document, vec![EntityType::Line(bad)]);
    assert!(picture_dropped(&document));
    let handle = document
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().common.handle),
            _ => None,
        })
        .unwrap();
    let EntityType::Dimension(d) = document.get_entity_mut(handle).unwrap() else {
        panic!()
    };
    d.base_mut().block_name = "Missing".into();
    assert!(picture_dropped(&document));
}

fn marker(drawing: &mut IfccadDocument, kind: IfccadEntityKind) -> u64 {
    let id = drawing.id_counters.allocate_entity_id().unwrap();
    let mut member = drawing.model.entities[0].as_native().unwrap().clone();
    member.id = id;
    member.layer_id = 0;
    member.kind = kind;
    member.appearance = part(IfccadDimensionGraphicsKind::Point {
        placement: frame([0.; 3]),
        thickness: 0.,
    })
    .appearance;
    drawing.blocks.push(IfccadBlockDefinition {
        id: 1,
        name: "Arrow".into(),
        description: String::new(),
        anonymous: false,
        explodable: true,
        uniform_scaling: false,
        base_point: [0.; 3],
        insertion_unit: drawing.length_unit.clone(),
        bounds: None,
        bounds_quality: None,
        entities: vec![IfccadEntity::Native(member)],
    });
    drawing.id_counters.next_block_id = 2;
    1
}

#[test]
fn wide_marker_resources_transfer_completely_and_remain_shared() {
    let mut drawing = source();
    let definition_id = marker(
        &mut drawing,
        IfccadEntityKind::PlanarPolyline {
            placement: frame([0.; 3]),
            vertices: vec![[0., 0.], [2., 1.]],
            bulges: vec![0., 0.],
            closed: false,
            line_pattern_generation: IfccadLinePatternGeneration::Continuous,
            widths: IfccadPlanarPolylineWidths::Constant { width: 0.25 },
            thickness: 0.,
        },
    );
    let a = part(IfccadDimensionGraphicsKind::BlockInstance {
        definition_id,
        transform: IfccadBlockTransform {
            placement: frame([2., 4., 0.]),
            rotation: 0.,
            scale: [2., 2., 1.],
        },
    });
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![a.clone(), a]));
    let (document, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let restored = load(&bytes);
    assert_eq!(restored.blocks.len(), 1);
    let IfccadEntityKind::PlanarPolyline { widths, .. } =
        &restored.blocks[0].entities[0].as_native().unwrap().kind
    else {
        panic!()
    };
    assert_eq!(
        *widths,
        IfccadPlanarPolylineWidths::Constant { width: 0.25 }
    );
    let parts = &dimension(&restored).graphics.as_ref().unwrap().parts;
    assert_eq!(parts.len(), 2);
    let IfccadDimensionGraphicsKind::BlockInstance {
        definition_id: a, ..
    } = parts[0].kind
    else {
        panic!()
    };
    let IfccadDimensionGraphicsKind::BlockInstance {
        definition_id: b, ..
    } = parts[1].kind
    else {
        panic!()
    };
    assert_eq!(a, b);
}

#[test]
fn defaulted_width_metadata_is_reported_for_direct_parts_and_marker_members() {
    for nested in [false, true] {
        let mut drawing = source();
        let kind = IfccadEntityKind::PlanarPolyline {
            placement: frame([0.; 3]),
            vertices: vec![[0., 0.], [2., 1.]],
            bulges: vec![0., 0.],
            closed: false,
            line_pattern_generation: IfccadLinePatternGeneration::Continuous,
            widths: IfccadPlanarPolylineWidths::Defaulted {
                default_start_width: 0.25,
                default_end_width: 0.5,
                overrides: vec![Default::default(), Default::default()],
            },
            thickness: 0.,
        };
        let kind = if nested {
            let definition_id = marker(&mut drawing, kind);
            IfccadDimensionGraphicsKind::BlockInstance {
                definition_id,
                transform: IfccadBlockTransform {
                    placement: frame([0.; 3]),
                    rotation: 0.,
                    scale: [1.; 3],
                },
            }
        } else {
            IfccadDimensionGraphicsKind::try_from(kind).unwrap()
        };
        dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(kind)]));
        let (document, losses) = read(&drawing);
        assert!(!picture(&document).is_empty(), "{losses:?}");
        assert!(
            losses
                .iter()
                .any(|l| l.message.contains("width inheritance")),
            "nested={nested}: {losses:?}"
        );
    }
}

#[test]
fn reference_points_on_defpoints_keep_their_role_and_stored_position() {
    let mut drawing = source();
    let mut layer = drawing.layers[0].clone();
    layer.id = 3;
    layer.name = "Defpoints".into();
    drawing.layers.push(layer);
    drawing.id_counters.next_layer_id = 4;
    let mut helper = part(IfccadDimensionGraphicsKind::Point {
        placement: frame([100., 200., 0.]),
        thickness: 0.,
    });
    helper.layer_id = Some(3);
    helper.visible = false;
    helper.purpose = IfccadDimensionGraphicsPurpose::ReferencePoint;
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![helper]));
    let (document, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let native = load(&bytes);
    let p = &dimension(&native).graphics.as_ref().unwrap().parts[0];
    assert_eq!(p.purpose, IfccadDimensionGraphicsPurpose::ReferencePoint);
    assert!(!p.visible);
    let IfccadDimensionGraphicsKind::Point { placement, .. } = &p.kind else {
        panic!()
    };
    assert_eq!(placement.origin, [100., -100., 0.]);
}

#[test]
fn an_ordinary_insert_of_a_picture_keeps_its_regular_definition() {
    let (mut document, _) = read(&source());
    add_picture(&mut document, vec![line()]);
    let insert = opencaddocument::entities::Insert::new("*D1", Vector3::new(0., 0., 0.));
    document.add_entity(EntityType::Insert(insert)).unwrap();
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let restored = load(&bytes);
    assert_eq!(restored.blocks.len(), 1);
    assert_eq!(restored.blocks[0].entities.len(), 1);
    assert!(dimension(&restored).graphics.is_some());
}

#[test]
fn root_picture_metadata_is_not_silently_discarded() {
    let (mut document, _) = read(&source());
    add_picture(&mut document, vec![line()]);
    document.block_records.get_mut("*D1").unwrap().description = "Authored description".into();
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(
        losses.iter().any(|l| l.message.contains("graphics")),
        "{losses:?}"
    );
    assert!(dimension(&load(&bytes)).graphics.is_none());
}

#[test]
fn nonfinite_late_sibling_cannot_be_hidden_by_an_unsupported_earlier_sibling() {
    let (mut document, _) = read(&source());
    let mut unsupported = Line::from_points(Vector3::new(0., 0., 0.), Vector3::new(1., 0., 0.));
    unsupported.thickness = 1.;
    let invalid = Line::from_points(
        Vector3::new(f64::INFINITY, 0., 0.),
        Vector3::new(1., 0., 0.),
    );
    add_picture(
        &mut document,
        vec![EntityType::Line(unsupported), EntityType::Line(invalid)],
    );
    assert!(picture_dropped(&document));
}

#[test]
fn complete_picture_has_actual_ascii_binary_and_dwg_readback() {
    use std::io::Cursor;
    let mut drawing = source();
    let mut parts = simple_parts(&drawing);
    if let IfccadDimensionGraphicsKind::MText(mtext) = &mut parts.last_mut().unwrap().kind {
        use ifccad::text::*;
        mtext.content[0].inlines.push(MTextInline::Stack(TextStack {
            upper: "1".into(),
            lower: "2".into(),
            ..Default::default()
        }));
    }
    let count = parts.len();
    dimension_mut(&mut drawing).graphics = Some(graphics(parts));
    let (document, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    let variants = [
        (
            "ASCII",
            opencaddocument::DxfWriter::new(&document)
                .write_to_vec()
                .unwrap(),
        ),
        (
            "binary",
            opencaddocument::DxfWriter::new_binary(&document)
                .write_to_vec()
                .unwrap(),
        ),
        (
            "DWG",
            opencaddocument::DwgWriter::write_to_vec(&document).unwrap(),
        ),
    ];
    for (name, bytes) in variants {
        let cad: Document = if name == "DWG" {
            opencaddocument::DwgReader::from_stream(Cursor::new(bytes))
                .read()
                .unwrap()
        } else {
            opencaddocument::DxfReader::from_reader(Cursor::new(bytes))
                .unwrap()
                .read()
                .unwrap()
        };
        assert_eq!(cad.audit().error_count(), 0, "{name}");
        let (bytes, losses) = Ifcx.write(&cad).unwrap();
        assert!(losses.is_empty(), "{name}: {losses:?}");
        let native = load(&bytes);
        assert_eq!(
            dimension(&native).graphics.as_ref().unwrap().parts.len(),
            count,
            "{name}"
        );
    }
}

#[test]
fn positive_pictures_in_paper_and_block_owners_keep_scope_positions() {
    for block in [false, true] {
        let mut drawing = source();
        dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(
            IfccadDimensionGraphicsKind::LineSegment {
                start: [0., 1., 0.],
                end: [12., 1., 0.],
                thickness: 0.,
                extrusion_direction: [0., 0., 1.],
            },
        )]));
        let members = std::mem::take(&mut drawing.model.entities);
        if block {
            drawing.blocks.push(IfccadBlockDefinition {
                id: 1,
                name: "Owner".into(),
                description: String::new(),
                anonymous: false,
                explodable: true,
                uniform_scaling: false,
                base_point: [0.; 3],
                insertion_unit: drawing.length_unit.clone(),
                bounds: None,
                bounds_quality: None,
                entities: members,
            });
            drawing.id_counters.next_block_id = 2;
        } else {
            drawing.paper_layouts.push(IfccadPaperLayout {
                id: 2,
                name: "Sheet".into(),
                tab_index: 1,
                settings: Default::default(),
                canvas: None,
                bounds: None,
                bounds_quality: None,
                entities: members,
            });
            drawing.id_counters.next_layout_id = 3;
        }
        let (document, losses) = read(&drawing);
        assert!(losses.is_empty(), "{losses:?}");
        let (bytes, losses) = Ifcx.write(&document).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        let native = load(&bytes);
        let entities = if block {
            &native.blocks[0].entities
        } else {
            &native.paper_layouts[0].entities
        };
        let d = entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::Dimension(d) => Some(d),
                _ => None,
            })
            .unwrap();
        let IfccadDimensionGraphicsKind::LineSegment { start, .. } =
            d.graphics.as_ref().unwrap().parts[0].kind
        else {
            panic!()
        };
        assert_eq!(
            [
                start[0] + d.placement.origin[0],
                start[1] + d.placement.origin[1],
                start[2] + d.placement.origin[2]
            ],
            [16., 9., 0.]
        );
    }
}

#[test]
fn unresolved_picture_layer_handles_and_nonfinite_clone_state_leave_the_picture_out() {
    let (mut document, _) = read(&source());
    add_picture(&mut document, vec![line()]);
    let member = document
        .entities_in_block("*D1")
        .next()
        .unwrap()
        .common()
        .handle;
    document
        .get_entity_mut(member)
        .unwrap()
        .common_mut()
        .layer_handle = Some(opencaddocument::Handle::new(0xffff_ffff));
    assert!(picture_dropped(&document));
    document
        .get_entity_mut(member)
        .unwrap()
        .common_mut()
        .layer_handle = None;
    let dim = document
        .model_space_entities()
        .find_map(|e| match e {
            EntityType::Dimension(d) => Some(d.base().common.handle),
            _ => None,
        })
        .unwrap();
    let EntityType::Dimension(d) = document.get_entity_mut(dim).unwrap() else {
        panic!()
    };
    d.base_mut().insertion_rotation = f64::NAN;
    assert!(picture_dropped(&document));
}

#[test]
fn failing_marker_members_leave_no_partial_native_resource_or_picture() {
    let mut drawing = source();
    let definition_id = marker(
        &mut drawing,
        IfccadEntityKind::LineSegment {
            start: [0., 0., 0.],
            end: [2., 1., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        },
    );
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(
        IfccadDimensionGraphicsKind::BlockInstance {
            definition_id,
            transform: IfccadBlockTransform {
                placement: frame([0.; 3]),
                rotation: 0.,
                scale: [1.; 3],
            },
        },
    )]));
    let (mut document, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    document
        .block_records
        .get_mut("Arrow")
        .unwrap()
        .flags
        .anonymous = true;
    let leaf = document
        .entities_in_block("Arrow")
        .next()
        .unwrap()
        .common()
        .handle;
    let EntityType::Line(line) = document.get_entity_mut(leaf).unwrap() else {
        panic!()
    };
    line.common.shadow_flags = 1; // unsupported common metadata.
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(
        losses.iter().any(|l| l.message.contains("graphics")),
        "{losses:?}"
    );
    let native = load(&bytes);
    assert!(native.blocks.is_empty());
    assert!(dimension(&native).graphics.is_none());
}

#[test]
fn independent_cycles_preserve_the_first_transferred_scope_positions() {
    let mut drawing = source();
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(
        IfccadDimensionGraphicsKind::LineSegment {
            start: [0., 1., 0.],
            end: [12., 1., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        },
    )]));
    for _ in 0..3 {
        let (cad, losses) = read(&drawing);
        assert!(losses.is_empty(), "{losses:?}");
        let (bytes, losses) = Ifcx.write(&cad).unwrap();
        assert!(losses.is_empty(), "{losses:?}");
        drawing = load(&bytes);
        let d = dimension(&drawing);
        let IfccadDimensionGraphicsKind::LineSegment { start, end, .. } =
            d.graphics.as_ref().unwrap().parts[0].kind
        else {
            panic!()
        };
        assert_eq!(
            [
                start[0] + d.placement.origin[0],
                start[1] + d.placement.origin[1],
                start[2] + d.placement.origin[2]
            ],
            [16., 9., 0.]
        );
        assert_eq!(
            [
                end[0] + d.placement.origin[0],
                end[1] + d.placement.origin[1],
                end[2] + d.placement.origin[2]
            ],
            [28., 9., 0.]
        );
    }
}

#[test]
fn explicit_layer_zero_factoring_is_reported_even_when_current_owner_matches() {
    let mut drawing = source();
    let d = drawing
        .model
        .entities
        .iter_mut()
        .find(|e| matches!(&e.as_native().unwrap().kind, IfccadEntityKind::Dimension(_)))
        .unwrap()
        .as_native_mut()
        .unwrap();
    d.layer_id = 0;
    let IfccadEntityKind::Dimension(d) = &mut d.kind else {
        panic!()
    };
    let mut p = part(IfccadDimensionGraphicsKind::LineSegment {
        start: [0., 0., 0.],
        end: [1., 0., 0.],
        thickness: 0.,
        extrusion_direction: [0., 0., 1.],
    });
    p.layer_id = Some(0);
    d.graphics = Some(graphics(vec![p]));
    let (cad, losses) = read(&drawing);
    assert!(!picture(&cad).is_empty());
    assert!(
        losses.iter().any(|l| l.message.contains("layer binding")),
        "{losses:?}"
    );
}

#[test]
fn an_external_draw_order_table_is_reported_instead_of_losing_its_order() {
    use opencaddocument::objects::{ObjectType, SortEntitiesTable};
    let (mut cad, _) = read(&source());
    add_picture(&mut cad, vec![line()]);
    let root = cad.block_records.get("*D1").unwrap().handle;
    let member = cad.entities_in_block("*D1").next().unwrap().common().handle;
    let mut order = SortEntitiesTable::for_block(root);
    order.handle = cad.allocate_handle();
    order.add_entry(member, opencaddocument::Handle::new(1234));
    cad.objects
        .insert(order.handle, ObjectType::SortEntitiesTable(order));
    let (bytes, losses) = Ifcx.write(&cad).unwrap();
    assert!(
        losses.iter().any(|l| l.message.contains("draw order")),
        "{losses:?}"
    );
    assert!(dimension(&load(&bytes)).graphics.is_none());
}

#[test]
fn unrepresented_marker_record_metadata_is_diagnosed_atomically() {
    let mut drawing = source();
    let definition_id = marker(
        &mut drawing,
        IfccadEntityKind::LineSegment {
            start: [0.; 3],
            end: [1., 0., 0.],
            thickness: 0.,
            extrusion_direction: [0., 0., 1.],
        },
    );
    dimension_mut(&mut drawing).graphics = Some(graphics(vec![part(
        IfccadDimensionGraphicsKind::BlockInstance {
            definition_id,
            transform: IfccadBlockTransform {
                placement: frame([0.; 3]),
                rotation: 0.,
                scale: [1.; 3],
            },
        },
    )]));
    let (mut cad, losses) = read(&drawing);
    assert!(losses.is_empty(), "{losses:?}");
    let record = cad.block_records.get_mut("Arrow").unwrap();
    record.flags.anonymous = true;
    record.preview_data = vec![1, 2, 3];
    let (bytes, losses) = Ifcx.write(&cad).unwrap();
    assert!(
        losses.iter().any(|l| l.message.contains("graphics")),
        "{losses:?}"
    );
    let native = load(&bytes);
    assert!(dimension(&native).graphics.is_none());
    assert!(native.blocks.is_empty());
}

#[test]
fn picture_header_metadata_is_diagnosed() {
    let (mut document, _) = read(&source());
    add_picture(&mut document, vec![line()]);
    let start = document
        .block_records
        .get("*D1")
        .unwrap()
        .block_entity_handle;
    let EntityType::Block(header) = document.get_entity_mut(start).unwrap() else {
        panic!()
    };
    header.description = "header-only source metadata".into();
    let (bytes, losses) = Ifcx.write(&document).unwrap();
    assert!(dimension(&load(&bytes)).graphics.is_none());
    assert!(losses
        .iter()
        .any(|loss| loss.message.contains("header metadata")));
}

#[test]
fn mistyped_picture_headers_and_missing_member_handles_leave_the_picture_out() {
    for header in [false, true] {
        let (mut document, _) = read(&source());
        add_picture(&mut document, vec![line()]);
        let record = document.block_records.get_mut("*D1").unwrap();
        let missing = opencaddocument::types::Handle::new(0xabcdef);
        if header {
            record.block_end_handle = record.entity_handles[0];
        } else {
            record.entity_handles.push(missing);
        }
        assert!(picture_dropped(&document), "header={header}");
    }
}
