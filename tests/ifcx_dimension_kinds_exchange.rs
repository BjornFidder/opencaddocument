#![cfg(feature = "ifcx")]
#[path = "../examples/support/dimension_cases.rs"]
mod cases;
use ifccad::{dimension::*, *};
use opencaddocument::{Document, DwgWriter, DxfWriter, Ifcx};

fn position(p: [f64; 3], frame: &IfccadPlacement) -> [f64; 3] {
    let x = frame.x_axis;
    let y = frame.y_axis;
    let z = [
        x[1] * y[2] - x[2] * y[1],
        x[2] * y[0] - x[0] * y[2],
        x[0] * y[1] - x[1] * y[0],
    ];
    std::array::from_fn(|i| frame.origin[i] + p[0] * x[i] + p[1] * y[i] + p[2] * z[i])
}

fn physical(document: &Document, transport: &str) -> Document {
    let bytes = match transport {
        "dwg" => DwgWriter::write_to_vec(document).unwrap(),
        "binary" => DxfWriter::new_binary(document).write_to_vec().unwrap(),
        _ => DxfWriter::new(document).write_to_vec().unwrap(),
    };
    if transport == "dwg" {
        assert_eq!(&bytes[..6], b"AC1032");
    }
    let outcome = if transport == "dwg" {
        opencaddocument::DwgReader::from_stream(std::io::Cursor::new(bytes))
            .read_with_stats()
            .unwrap()
    } else {
        opencaddocument::DxfReader::from_reader(std::io::Cursor::new(bytes))
            .unwrap()
            .read_with_stats()
            .unwrap()
    };
    assert_eq!(
        outcome.stats.recovered_errors, 0,
        "{transport}: {:?}",
        outcome.stats
    );
    assert_eq!(
        outcome.stats.skipped_source_records, 0,
        "{transport}: {:?}",
        outcome.stats
    );
    assert!(
        outcome.stats.stream_completed,
        "{transport}: {:?}",
        outcome.stats
    );
    outcome.document
}

#[test]
fn ordinary_reference_anchors_and_measures_survive_four_physical_cycles() {
    // Independent reference cases: radial 3-4-5, diameter twice that chord,
    // perpendicular unit rays, positive X ordinate 3, radius-five quarter arc.
    let measures = [
        5.,
        10.,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_2,
        3.,
        5. * std::f64::consts::FRAC_PI_2,
        5.,
    ];
    for (definition, expected) in cases::definitions().into_iter().zip(measures) {
        let source = cases::drawing(definition.clone());
        let source_dimension = cases::dimension(&source);
        for transport in ["ascii", "binary", "dwg"] {
            let mut drawing = source.clone();
            for cycle in 0..4 {
                let imported = Ifcx.import_ifccad(&drawing).unwrap();
                assert!(
                    imported.losses.is_empty(),
                    "{transport} cycle {cycle}: {:?}",
                    imported.losses
                );
                let cad = physical(&imported.document, transport);
                let returned = Ifcx.export_ifccad(&cad).unwrap();
                assert!(
                    returned.losses.is_empty(),
                    "{transport} cycle {cycle}: {:?}; dimensions {:?}",
                    returned.losses,
                    cad.model_space_entities()
                        .filter_map(|e| match e {
                            opencaddocument::EntityType::Dimension(d) =>
                                Some(d.base().block_name.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                );
                validate_ifccad_document(&returned.drawing).unwrap();
                let actual = cases::dimension(&returned.drawing);
                assert_eq!(actual.definition.kind(), definition.kind());
                // Named bounds for ONLY these axis-aligned integer cases in
                // [-10,10]: 2,4,8,16 ULP at extent 10 for coordinate residuals.
                // No general placement or conversion tolerance is implied.
                let ulp = f64::from_bits(10_f64.to_bits() + 1) - 10.;
                let bound = ulp * f64::from(2_u32 << cycle);
                for ((role, p), (actual_role, q)) in definition
                    .measurement_anchors()
                    .into_iter()
                    .zip(actual.definition.measurement_anchors())
                {
                    assert_eq!(role, actual_role);
                    let p = position(p, &source_dimension.placement);
                    let q = position(q, &actual.placement);
                    for axis in 0..3 {
                        assert!(
                            (p[axis] - q[axis]).abs() <= bound,
                            "{:?} {role:?} {transport} cycle {cycle}: {p:?} -> {q:?}",
                            definition.kind()
                        );
                    }
                }
                let measured = match evaluate_dimension(&actual.definition).unwrap() {
                    DimensionMeasurement::Angle(x) | DimensionMeasurement::Distance(x) => x,
                };
                assert!(
                    measured.to_bits().abs_diff(expected.to_bits()) <= 2,
                    "{:?} {transport} cycle {cycle}: {expected} -> {measured}",
                    definition.kind()
                );
                assert_eq!(actual.label, source_dimension.label);
                assert!(actual.associations.is_empty());
                assert!(actual.graphics.is_none());
                drawing = returned.drawing;
            }
        }
    }
}

#[test]
fn nine_kind_gallery_retains_each_independent_picture_through_actual_files() {
    let source = cases::gallery();
    let imported = Ifcx.import_ifccad(&source).unwrap();
    assert_eq!(
        imported
            .losses
            .iter()
            .filter(|loss| loss.message.contains("graphics provenance (Generated)"))
            .count(),
        9
    );
    assert!(
        imported.losses.iter().all(
            |loss| loss.message.contains("graphics provenance (Generated)")
                || loss
                    .message
                    .contains("graphics retained with numerical rounding")
        ),
        "{:?}",
        imported.losses
    );
    for transport in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, transport);
        let returned = Ifcx.export_ifccad(&cad).unwrap();
        validate_ifccad_document(&returned.drawing).unwrap();
        let dimensions: Vec<_> = returned
            .drawing
            .model
            .entities
            .iter()
            .filter_map(IfccadEntity::as_native)
            .filter_map(|e| match &e.kind {
                IfccadEntityKind::Dimension(d) => Some(d.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(dimensions.len(), 9, "{transport}: {:?}", returned.losses);
        for original in source
            .model
            .entities
            .iter()
            .filter_map(IfccadEntity::as_native)
            .filter_map(|e| match &e.kind {
                IfccadEntityKind::Dimension(d) => Some(d.as_ref()),
                _ => None,
            })
        {
            let actual = dimensions
                .iter()
                .find(|d| d.definition.kind() == original.definition.kind())
                .unwrap();
            let picture = actual.graphics.as_ref().unwrap_or_else(|| {
                panic!(
                    "{:?} {transport}: {:?}",
                    original.definition.kind(),
                    returned.losses
                )
            });
            assert_eq!(
                picture.parts.len(),
                original.graphics.as_ref().unwrap().parts.len()
            );
            assert_eq!(
                picture.origin,
                IfccadDimensionGraphicsOrigin::SourceSnapshot
            );
            assert_eq!(
                evaluate_dimension(&actual.definition).unwrap(),
                evaluate_dimension(&original.definition).unwrap()
            );
        }
    }
}

#[test]
fn positive_major_and_full_arc_spans_keep_their_selected_measure() {
    for sweep in [
        DimensionArcSpan::Partial {
            radians: 3. * std::f64::consts::FRAC_PI_2,
        },
        DimensionArcSpan::Full {
            traversal: DimensionTraversal::Counterclockwise,
        },
    ] {
        let definition = DimensionDefinition::ArcLength {
            center: [0.; 3],
            radius: 100.,
            start_parameter: 0.,
            sweep: sweep.clone(),
            dimension_arc_point: [120., 120.],
            leader: None,
        };
        let source = cases::drawing(definition.clone());
        let imported = Ifcx.import_ifccad(&source).unwrap();
        assert!(imported.losses.is_empty(), "{:?}", imported.losses);
        for transport in ["ascii", "binary", "dwg"] {
            let cad = physical(&imported.document, transport);
            let returned = Ifcx.export_ifccad(&cad).unwrap();
            assert!(
                returned.losses.is_empty(),
                "{transport}: {:?}",
                returned.losses
            );
            let actual = cases::dimension(&returned.drawing);
            assert_eq!(
                evaluate_dimension(&actual.definition).unwrap(),
                evaluate_dimension(&definition).unwrap()
            );
            assert!(
                matches!(&actual.definition,DimensionDefinition::ArcLength{sweep:actual_sweep,..} if *actual_sweep==sweep)
            );
        }
    }
}

#[test]
fn unsupported_modes_are_diagnosed_instead_of_exporting_another_measure() {
    let cases = [
        DimensionDefinition::Ordinate {
            origin: [0.; 3],
            feature_point: [-3., 4., 0.],
            axis_angle: 0.,
            leader_end: [10., 10.],
            value_mode: DimensionOrdinateMode::Signed,
        },
        DimensionDefinition::ArcLength {
            center: [0.; 3],
            radius: 5.,
            start_parameter: 0.,
            sweep: DimensionArcSpan::Partial {
                radians: -std::f64::consts::FRAC_PI_2,
            },
            dimension_arc_point: [6., 6.],
            leader: None,
        },
        DimensionDefinition::ArcLength {
            center: [0.; 3],
            radius: 5.,
            start_parameter: 1e100,
            sweep: DimensionArcSpan::Partial {
                radians: std::f64::consts::FRAC_PI_2,
            },
            dimension_arc_point: [6., 6.],
            leader: None,
        },
    ];
    for definition in cases {
        let source = cases::drawing(definition);
        let imported = Ifcx.import_ifccad(&source).unwrap();
        assert!(imported
            .document
            .entities()
            .all(|e| !matches!(e, opencaddocument::EntityType::Dimension(_))));
        assert!(
            imported
                .losses
                .iter()
                .any(|loss| loss.message.contains("Dimension")),
            "{:?}",
            imported.losses
        );
    }
}

#[test]
fn line_endpoint_identity_survives_source_edits_and_physical_files() {
    use opencaddocument::objects::{AssociativeData, ObjectType};
    use opencaddocument::EntityType;
    // The same source is intentionally used for both roles. Without explicit
    // endpoint identity Studio resolves both default markers to its start.
    let definition = DimensionDefinition::Linear {
        first_point: [0., 0., 0.],
        second_point: [18., 12., 0.],
        direction_angle: 0.,
        dimension_line_point: [0., 21.],
        extension_direction_angle: None,
    };
    let mut source = cases::drawing(definition);
    let source_id = source
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_native)
        .find(|e| matches!(e.kind, IfccadEntityKind::LineSegment { .. }))
        .unwrap()
        .id;
    for entity in &mut source.model.entities {
        let Some(entity) = entity.as_native_mut() else {
            continue;
        };
        if entity.id == source_id {
            if let IfccadEntityKind::LineSegment { start, end, .. } = &mut entity.kind {
                *start = [0., 0., 0.];
                *end = [18., 12., 0.];
            }
        }
        if let IfccadEntityKind::Dimension(d) = &mut entity.kind {
            d.associations = vec![
                IfccadDimensionAssociation {
                    role: DimensionRole::FirstPoint,
                    source_entity_id: source_id,
                    selector: DimensionPointSelector::Start,
                },
                IfccadDimensionAssociation {
                    role: DimensionRole::SecondPoint,
                    source_entity_id: source_id,
                    selector: DimensionPointSelector::End,
                },
            ];
        }
    }
    recompute_ifccad_document_bounds(&mut source).unwrap();
    validate_ifccad_document(&source).unwrap();
    let imported = Ifcx.import_ifccad(&source).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    fn check(document: &Document) {
        let association = document
            .objects
            .values()
            .find_map(|object| match object {
                ObjectType::Associative(object) => match &object.data {
                    AssociativeData::DimensionAssociation(a) => Some(a),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        let start = &association.references[0][0];
        let end = &association.references[1][0];
        assert_eq!((start.main_subent_type, start.main_gs_marker), (1, 0));
        assert_eq!(
            (end.main_subent_type, end.main_gs_marker),
            (1, 1),
            "end must not resolve to the start after source edits"
        );
        assert_eq!(start.xrefs, end.xrefs);
        // Reproduce the resolver's vertex identity without comparing handles.
        let Some(EntityType::Line(line)) = document.get_entity(start.xrefs[0]) else {
            panic!("line source")
        };
        let points = [line.start, line.end];
        assert_ne!(
            points[start.main_gs_marker as usize],
            points[end.main_gs_marker as usize]
        );
        let exported = Ifcx.export_ifccad(document).unwrap();
        validate_ifccad_document(&exported.drawing).unwrap();
        let actual = cases::dimension(&exported.drawing);
        assert_eq!(
            actual
                .associations
                .iter()
                .map(|a| (a.role, a.selector))
                .collect::<Vec<_>>(),
            vec![
                (DimensionRole::FirstPoint, DimensionPointSelector::Start),
                (DimensionRole::SecondPoint, DimensionPointSelector::End),
            ],
            "endpoint identity must survive even when the stored snap point is stale"
        );
    }
    check(&imported.document);
    // Keep the coordinate-based unspecified legacy reference readable, while
    // refusing unsupported or contradictory subentity encodings.
    for (subentity, marker, osnap, supported) in [
        (0, 0, 1, true),
        (0, 1, 1, false),
        (1, 2, 1, false),
        (2, 1, 1, false),
        (1, 1, 13, false),
    ] {
        let mut variant = imported.document.clone();
        for object in variant.objects.values_mut() {
            if let ObjectType::Associative(object) = object {
                if let AssociativeData::DimensionAssociation(a) = &mut object.data {
                    let end = &mut a.references[1][0];
                    end.main_subent_type = subentity;
                    end.main_gs_marker = marker;
                    end.osnap_type = osnap;
                }
            }
        }
        let returned = Ifcx.export_ifccad(&variant).unwrap();
        let associations = &cases::dimension(&returned.drawing).associations;
        if supported {
            assert_eq!(associations.len(), 2);
            assert_eq!(associations[1].selector, DimensionPointSelector::End);
        } else {
            assert_eq!(associations.len(), 1);
            assert!(!returned.losses.is_empty());
        }
    }
    for transport in ["ascii", "binary", "dwg"] {
        check(&physical(&imported.document, transport));
        let mut edited = physical(&imported.document, transport);
        let handle = edited
            .objects
            .values()
            .find_map(|object| match object {
                ObjectType::Associative(object) => match &object.data {
                    AssociativeData::DimensionAssociation(a) => Some(a.references[0][0].xrefs[0]),
                    _ => None,
                },
                _ => None,
            })
            .unwrap();
        let Some(EntityType::Line(line)) = edited.get_entity_mut(handle) else {
            panic!("line source")
        };
        line.start.x += 3.;
        line.end.x += 6.;
        // DIMASSOC snap coordinates retain the pre-edit values. The explicit
        // endpoint identity must remain usable both before and after saving.
        check(&edited);
        check(&physical(&edited, transport));
    }
}

#[test]
fn arc_sources_use_center_slot_one_and_angle_endpoints_two_and_three() {
    // Bricsys TD_Mgd 25.3 DimAssocPointType: CenterPoint=1,
    // AngLineStart=2, AngLineEnd=3. Slot zero is LeaderPoint, not ArcStart.
    use opencaddocument::objects::{AssociativeData, ObjectType};
    let mut source = cases::drawing(cases::definitions().remove(5));
    let source_id = source
        .model
        .entities
        .iter()
        .filter_map(IfccadEntity::as_native)
        .find(|e| matches!(e.kind, IfccadEntityKind::LineSegment { .. }))
        .unwrap()
        .id;
    for entity in &mut source.model.entities {
        if let Some(IfccadNativeEntity {
            kind: IfccadEntityKind::Dimension(d),
            ..
        }) = entity.as_native_mut()
        {
            d.associations = vec![
                IfccadDimensionAssociation {
                    role: DimensionRole::Center,
                    source_entity_id: source_id,
                    selector: DimensionPointSelector::Start,
                },
                IfccadDimensionAssociation {
                    role: DimensionRole::ArcStart,
                    source_entity_id: source_id,
                    selector: DimensionPointSelector::Start,
                },
                IfccadDimensionAssociation {
                    role: DimensionRole::ArcEnd,
                    source_entity_id: source_id,
                    selector: DimensionPointSelector::End,
                },
            ];
        }
    }
    validate_ifccad_document(&source).unwrap();
    let imported = Ifcx.import_ifccad(&source).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    let association = imported
        .document
        .objects
        .values()
        .find_map(|o| match o {
            ObjectType::Associative(o) => match &o.data {
                AssociativeData::DimensionAssociation(a) => Some(a),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    assert_eq!(association.associativity, 14);
    assert!(association.references[0].is_empty());
    for slot in 1..4 {
        assert_eq!(association.references[slot].len(), 1);
    }
    for transport in ["ascii", "binary", "dwg"] {
        let returned = Ifcx
            .export_ifccad(&physical(&imported.document, transport))
            .unwrap();
        let actual = cases::dimension(&returned.drawing);
        assert_eq!(
            actual
                .associations
                .iter()
                .map(|a| a.role)
                .collect::<Vec<_>>(),
            vec![
                DimensionRole::Center,
                DimensionRole::ArcStart,
                DimensionRole::ArcEnd
            ]
        );
        assert_eq!(
            evaluate_dimension(&actual.definition).unwrap(),
            DimensionMeasurement::Distance(5. * std::f64::consts::FRAC_PI_2)
        );
    }
}

#[test]
fn all_nine_definitions_and_pictures_keep_paper_and_nested_block_ownership() {
    for unit in ["mm", "m", "in"] {
        for paper in [true, false] {
            let mut source = cases::gallery();
            source.length_unit = unit.into();
            let template = source.model.entities[0].as_native().unwrap().clone();
            let entities = std::mem::take(&mut source.model.entities);
            if paper {
                source.paper_layouts.push(IfccadPaperLayout {
                    id: 2,
                    name: "Dimensions".into(),
                    tab_index: 1,
                    settings: Default::default(),
                    canvas: None,
                    bounds: None,
                    bounds_quality: None,
                    entities,
                });
                source.id_counters.next_layout_id = 3;
            } else {
                source.blocks.push(IfccadBlockDefinition {
                    id: 1,
                    name: "DimensionCases".into(),
                    description: String::new(),
                    anonymous: false,
                    explodable: true,
                    uniform_scaling: false,
                    base_point: [0.; 3],
                    insertion_unit: unit.into(),
                    bounds: None,
                    bounds_quality: None,
                    entities,
                });
                let plane = |origin| IfccadPlacement {
                    origin,
                    x_axis: [1., 0., 0.],
                    y_axis: [0., 1., 0.],
                };
                let mut nested = template.clone();
                nested.id = 100;
                nested.kind = IfccadEntityKind::BlockInstance {
                    definition_id: 1,
                    transform: IfccadBlockTransform {
                        placement: plane([10., 20., 3.]),
                        rotation: 0.,
                        scale: [-2., 0.5, 1.],
                    },
                };
                source.blocks.push(IfccadBlockDefinition {
                    id: 2,
                    name: "NestedCases".into(),
                    description: String::new(),
                    anonymous: false,
                    explodable: true,
                    uniform_scaling: false,
                    base_point: [0.; 3],
                    insertion_unit: unit.into(),
                    bounds: None,
                    bounds_quality: None,
                    entities: vec![IfccadEntity::Native(nested)],
                });
                let mut outer = template;
                outer.id = 101;
                outer.kind = IfccadEntityKind::BlockInstance {
                    definition_id: 2,
                    transform: IfccadBlockTransform {
                        placement: plane([100., 200., 5.]),
                        rotation: 0.,
                        scale: [-1., 2., 1.],
                    },
                };
                source.model.entities.push(IfccadEntity::Native(outer));
                source.id_counters.next_block_id = 3;
            }
            source.model.bounds = None;
            source.model.bounds_quality = None;
            validate_ifccad_document(&source).unwrap();
            let imported = Ifcx.import_ifccad(&source).unwrap();
            for transport in ["ascii", "binary", "dwg"] {
                let returned = Ifcx
                    .export_ifccad(&physical(&imported.document, transport))
                    .unwrap();
                validate_ifccad_document(&returned.drawing).unwrap();
                let entities = if paper {
                    &returned
                        .drawing
                        .paper_layouts
                        .iter()
                        .find(|p| p.name == "Dimensions")
                        .unwrap()
                        .entities
                } else {
                    &returned
                        .drawing
                        .blocks
                        .iter()
                        .find(|b| b.name == "DimensionCases")
                        .unwrap()
                        .entities
                };
                let dims: Vec<_> = entities
                    .iter()
                    .filter_map(IfccadEntity::as_native)
                    .filter_map(|e| match &e.kind {
                        IfccadEntityKind::Dimension(d) => Some(d.as_ref()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    dims.len(),
                    9,
                    "{unit} {paper} {transport}: {:?}",
                    returned.losses
                );
                assert!(
                    dims.iter()
                        .all(|d| d.graphics.as_ref().is_some_and(|g| !g.parts.is_empty())),
                    "{unit} {paper} {transport}: {:?}",
                    returned.losses
                );
                if !paper {
                    let nested = returned
                        .drawing
                        .blocks
                        .iter()
                        .find(|b| b.name == "NestedCases")
                        .unwrap();
                    let IfccadEntityKind::BlockInstance { transform, .. } =
                        &nested.entities[0].as_native().unwrap().kind
                    else {
                        panic!()
                    };
                    assert_eq!(transform.scale, [-2., 0.5, 1.]);
                    assert_eq!(transform.placement.origin, [10., 20., 3.]);
                }
            }
        }
    }
}

#[test]
fn new_kind_equal_parent_override_presence_is_diagnosed_both_ways() {
    use opencaddocument::xdata::{ExtendedDataRecord, XDataValue};
    let mut source = cases::drawing(cases::definitions().remove(0));
    let precision = source.dimension_styles[0]
        .settings
        .numbers
        .primary
        .precision;
    for entity in &mut source.model.entities {
        if let Some(IfccadNativeEntity {
            kind: IfccadEntityKind::Dimension(d),
            ..
        }) = entity.as_native_mut()
        {
            d.overrides.numbers.primary.precision = Some(precision);
        }
    }
    let imported = Ifcx.import_ifccad(&source).unwrap();
    assert!(
        imported
            .losses
            .iter()
            .any(|loss| loss.message.contains("authored override presence")),
        "{:?}",
        imported.losses
    );
    let mut cad = imported.document;
    let handle = cad
        .model_space_entities()
        .find_map(|e| match e {
            opencaddocument::EntityType::Dimension(d) => Some(d.base().common.handle),
            _ => None,
        })
        .unwrap();
    let opencaddocument::EntityType::Dimension(d) = cad.get_entity_mut(handle).unwrap() else {
        panic!()
    };
    let mut record = ExtendedDataRecord::new("ACAD");
    for value in [
        XDataValue::String("DSTYLE".into()),
        XDataValue::ControlString("{".into()),
        XDataValue::Integer16(271),
        XDataValue::Integer16(i16::from(precision)),
        XDataValue::ControlString("}".into()),
    ] {
        record.add_value(value);
    }
    d.base_mut().common.extended_data.upsert_record(record);
    let returned = Ifcx.export_ifccad(&cad).unwrap();
    assert!(
        returned
            .losses
            .iter()
            .any(|loss| loss.message.contains("authored override presence")),
        "{:?}",
        returned.losses
    );
    assert_eq!(
        evaluate_dimension(&cases::dimension(&returned.drawing).definition).unwrap(),
        DimensionMeasurement::Distance(5.)
    );
}

#[test]
fn angular_file_measurement_cache_is_radians_from_the_stored_definition() {
    for definition in cases::definitions().into_iter().filter(|d| {
        matches!(
            d,
            DimensionDefinition::AngularTwoLines { .. }
                | DimensionDefinition::AngularThreePoints { .. }
        )
    }) {
        let imported = Ifcx.import_ifccad(&cases::drawing(definition)).unwrap();
        let dimension = imported
            .document
            .model_space_entities()
            .find_map(|e| match e {
                opencaddocument::EntityType::Dimension(d) => Some(d.as_ref()),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            dimension.base().actual_measurement,
            std::f64::consts::FRAC_PI_2
        );
    }
}
