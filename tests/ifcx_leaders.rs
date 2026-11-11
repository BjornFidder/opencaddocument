//! Leader-family conversion through the public production route.
#![cfg(feature = "ifcx")]
use opencaddocument::entities::{Leader, Tolerance};
use opencaddocument::ifccad::{self, IfccadEntity, IfccadEntityKind};
use opencaddocument::types::Vector3;
use opencaddocument::{Document, EntityType, Ifcx};

fn physical(document: &Document, format: &str) -> Document {
    let bytes = match format {
        "dwg" => opencaddocument::DwgWriter::write_to_vec(document).unwrap(),
        "binary" => opencaddocument::DxfWriter::new_binary(document)
            .write_to_vec()
            .unwrap(),
        _ => opencaddocument::DxfWriter::new(document)
            .write_to_vec()
            .unwrap(),
    };
    let outcome = if format == "dwg" {
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
        "{format}: {:?}",
        outcome.stats
    );
    assert_eq!(
        outcome.stats.skipped_source_records, 0,
        "{format}: {:?}",
        outcome.stats
    );
    assert!(outcome.stats.stream_completed);
    outcome.document
}

#[test]
fn shared_mtext_and_attributed_insert_targets_survive_the_serialized_ifcx_route() {
    use opencaddocument::entities::{
        AttributeDefinition, AttributeEntity, Insert, LeaderCreationType, MText,
    };
    let mut source = Document::new();
    let mut text = MText::new();
    text.value = "Shared annotation".into();
    text.height = 2.5;
    let text = source
        .add_entity(EntityType::MText(Box::new(text)))
        .unwrap();
    let mut block = opencaddocument::tables::BlockRecord::new("ANNOTATION_TARGET");
    block.handle = source.allocate_handle();
    block.block_entity_handle = source.allocate_handle();
    block.block_end_handle = source.allocate_handle();
    let owner = block.handle;
    source.block_records.add(block).unwrap();
    let mut definition = AttributeDefinition::new("ID".into(), "Identity".into(), "default".into());
    definition.common.owner_handle = owner;
    source
        .add_entity(EntityType::AttributeDefinition(definition))
        .unwrap();
    let mut insert = Insert::new("ANNOTATION_TARGET", Vector3::new(20., 10., 0.));
    insert
        .attributes
        .push(AttributeEntity::new("ID".into(), "B-42".into()));
    let insert = source.add_entity(EntityType::Insert(insert)).unwrap();
    for (target, kind) in [
        (text, LeaderCreationType::WithText),
        (insert, LeaderCreationType::WithBlock),
    ] {
        for x in [0., 5.] {
            source
                .add_entity(EntityType::Leader(
                    Leader::two_point(Vector3::new(x, 0., 0.), Vector3::new(20., 10., 0.))
                        .with_creation_type(kind)
                        .with_annotation(target),
                ))
                .unwrap();
        }
    }
    let exported = Ifcx.export_ifccad(&source).unwrap();
    let bytes = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
    let strict = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let imported = Ifcx.import_ifccad(strict.document()).unwrap();
    for format in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, format);
        let mut targets = std::collections::HashMap::new();
        for entity in cad.model_space_entities() {
            if let EntityType::Leader(leader) = entity {
                *targets.entry(leader.annotation_handle).or_insert(0) += 1;
            }
        }
        assert_eq!(
            targets.len(),
            2,
            "{format}: no duplicated annotation targets"
        );
        assert!(targets.values().all(|count| *count == 2));
        for target in targets.keys() {
            match cad.get_entity(*target).unwrap() {
                EntityType::MText(text) => assert_eq!(text.value, "Shared annotation"),
                EntityType::Insert(insert) => {
                    assert_eq!(insert.block_name, "ANNOTATION_TARGET");
                    assert_eq!(insert.attributes.len(), 1);
                    assert_eq!(insert.attributes[0].value, "B-42");
                }
                other => panic!("{format}: wrong target {}", other.as_entity().entity_type()),
            }
        }
        let returned = Ifcx.export_ifccad(&cad).unwrap();
        let bytes = ifccad::encode_ifccad_document(&returned.drawing).unwrap();
        ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}

#[test]
fn multiple_groups_keep_minimal_paths_overrides_and_all_break_segments() {
    use opencaddocument::entities::{
        multileader::{
            LeaderLineBreakInfo, LeaderLinePropertyOverrideFlags, MultiLeaderPathType,
            StartEndPointPair,
        },
        MultiLeader,
    };
    let mut source = Document::new();
    let mut leader = MultiLeader::with_text("", Vector3::new(20., 10., 0.), vec![Vector3::ZERO]);
    leader.text_height = 2.5;
    leader.context.text_height = 2.5;
    leader.context.leader_roots[0].leader_index = 47;
    leader.context.leader_roots[0].lines[0].index = 901;
    for (index, path) in [
        (1200, MultiLeaderPathType::Spline),
        (2500, MultiLeaderPathType::Invisible),
    ] {
        let root = leader.add_leader_root();
        root.leader_index = index + 10;
        root.connection_point = Vector3::new(20., 10., 0.);
        let line = root.create_line(vec![
            Vector3::new(1., 2., 0.),
            Vector3::new(5., 6., 0.),
            Vector3::new(10., 8., 0.),
        ]);
        line.index = index;
        line.path_type = path;
        line.arrowhead_size = 6.;
        line.override_flags = LeaderLinePropertyOverrideFlags::PATH_TYPE
            | LeaderLinePropertyOverrideFlags::ARROWHEAD_SIZE;
        line.break_infos = vec![
            LeaderLineBreakInfo {
                segment_index: 0,
                break_points: vec![StartEndPointPair::new(
                    Vector3::new(2., 3., 0.),
                    Vector3::new(3., 4., 0.),
                )],
            },
            LeaderLineBreakInfo {
                segment_index: 1,
                break_points: vec![StartEndPointPair::new(
                    Vector3::new(6., 6.4, 0.),
                    Vector3::new(7., 6.8, 0.),
                )],
            },
        ];
        line.break_info_count = 2;
        line.segment_index = 0;
        line.break_points = line.break_infos[0].break_points.clone();
        root.create_line(vec![Vector3::new(4., 1., 0.)]).index = index + 1;
    }
    source
        .add_entity(EntityType::MultiLeader(Box::new(leader)))
        .unwrap();
    let exported = Ifcx.export_ifccad(&source).unwrap();
    let expected = exported
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::MultiLeader(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(expected.presentation.groups.len(), 3);
    assert_eq!(expected.presentation.groups[0].lines[0].vertices.len(), 1);
    let bytes = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
    let strict = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let imported = Ifcx.import_ifccad(strict.document()).unwrap();
    for format in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, format);
        let returned = Ifcx.export_ifccad(&cad).unwrap();
        let actual = returned
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::MultiLeader(v) => Some(v),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            actual.presentation.groups, expected.presentation.groups,
            "{format}"
        );
        assert_eq!(
            actual.presentation.content, expected.presentation.content,
            "{format}: empty owned text remains content"
        );
        let bytes = ifccad::encode_ifccad_document(&returned.drawing).unwrap();
        ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}

#[test]
fn block_labels_styles_and_physical_transports_keep_direct_binding() {
    use opencaddocument::entities::{
        multileader::BlockAttribute, AttributeDefinition, MultiLeader,
    };
    use opencaddocument::objects::{MultiLeaderStyle, ObjectType};
    let mut document = Document::new();
    let mut block = opencaddocument::tables::BlockRecord::new("Leader label");
    block.handle = document.allocate_handle();
    block.block_entity_handle = document.allocate_handle();
    block.block_end_handle = document.allocate_handle();
    let block_handle = block.handle;
    document.block_records.add(block).unwrap();
    let mut definition = AttributeDefinition::new("ID".into(), "Identity".into(), "default".into());
    definition.common.owner_handle = block_handle;
    let definition = document
        .add_entity(EntityType::AttributeDefinition(definition))
        .unwrap();
    let mut style = MultiLeaderStyle::new("Custom leaders");
    style.handle = document.allocate_handle();
    let style_handle = style.handle;
    style.scale_factor = 2.5;
    style.description = "independent style".into();
    document
        .objects
        .insert(style_handle, ObjectType::MultiLeaderStyle(style));
    let mut value = MultiLeader::new();
    value.set_block_content(block_handle, Vector3::new(10., 20., 0.));
    value.style_handle = Some(style_handle);
    value.context.block_content_scale = Vector3::new(-2., 3., 1.);
    value.context.block_rotation = 0.375;
    value.context.transform_matrix[12] = 123.;
    value.block_attributes.push(BlockAttribute {
        attribute_definition_handle: Some(definition),
        index: 17,
        width: 2.5,
        text: "L-42".into(),
    });
    let leader = document
        .add_entity(EntityType::MultiLeader(Box::new(value)))
        .unwrap();
    let native = Ifcx.export_ifccad(&document).unwrap();
    let id = native.entity_ids[&leader];
    let value = native
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| e.as_native().filter(|n| n.id == id))
        .unwrap();
    let IfccadEntityKind::MultiLeader(value) = &value.kind else {
        panic!()
    };
    assert_eq!(
        value.block_labels[0].definition_id,
        Some(native.entity_ids[&definition])
    );
    assert!(value.style_id.is_some());
    let bytes = ifccad::encode_ifccad_document(&native.drawing).unwrap();
    let strict = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let imported = Ifcx.import_ifccad(strict.document()).unwrap();
    for format in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, format);
        let value = cad
            .model_space_entities()
            .find_map(|e| match e {
                EntityType::MultiLeader(v) => Some(v),
                _ => None,
            })
            .unwrap();
        assert_eq!(value.block_attributes[0].text, "L-42", "{format}");
        let target = value.block_attributes[0]
            .attribute_definition_handle
            .unwrap();
        assert!(
            matches!(
                cad.get_entity(target),
                Some(EntityType::AttributeDefinition(_))
            ),
            "{format}"
        );
        assert_eq!(
            value.context.block_content_scale,
            Vector3::new(-2., 3., 1.),
            "{format}"
        );
        assert_eq!(value.context.transform_matrix[12], 123., "{format}");
        let ObjectType::MultiLeaderStyle(style) =
            cad.objects.get(&value.style_handle.unwrap()).unwrap()
        else {
            panic!()
        };
        assert_eq!(style.name, "Custom leaders", "{format}");
        assert_eq!(style.description, "independent style", "{format}");
        assert_eq!(style.scale_factor, 2.5, "{format}");
        let returned = Ifcx.export_ifccad(&cad).unwrap();
        let bytes = ifccad::encode_ifccad_document(&returned.drawing).unwrap();
        ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}

#[test]
fn independent_mleader_scale_presentation_and_oblique_content() {
    use ifccad::*;
    let mut document = Document::new();
    let mut value = opencaddocument::entities::MultiLeader::with_text(
        "oblique",
        Vector3::new(10., 8., 5.),
        vec![Vector3::ZERO, Vector3::new(10., 8., 5.)],
    );
    value.context.base_point = Vector3::new(3., 4., 5.);
    value.context.base_direction = Vector3::UNIT_Y;
    value.context.base_vertical = Vector3::UNIT_Z;
    value.context.text_normal = Vector3::UNIT_X;
    value.context.text_direction = Vector3::UNIT_Y;
    value.context.text_rotation = 0.375;
    let source = document
        .add_entity(EntityType::MultiLeader(Box::new(value)))
        .unwrap();
    let mut native = Ifcx.export_ifccad(&document).unwrap();
    let id = native.entity_ids[&source];
    native
        .drawing
        .annotation_scales
        .push(IfccadAnnotationScale {
            id: IfccadAnnotationScaleId(27),
            name: "leader scale".into(),
            paper_units: 1.,
            drawing_units: 50.,
            unit_scale: false,
        });
    native.drawing.id_counters.next_annotation_scale_id = 28;
    let IfccadEntityKind::MultiLeader(value) = &mut native
        .drawing
        .model
        .entities
        .iter_mut()
        .find_map(|e| match e {
            IfccadEntity::Native(n) if n.id == id => Some(n),
            _ => None,
        })
        .unwrap()
        .kind
    else {
        panic!()
    };
    assert_eq!(
        value
            .presentation
            .content
            .m_text
            .as_ref()
            .unwrap()
            .text
            .placement
            .origin,
        [4., 0., 7.],
        "owned text placement must be local to the presentation frame"
    );
    let mut presentation = value.presentation.clone();
    presentation.scale_factor = 7.;
    presentation.groups[0].lines[0].vertices[0] = [2., 3., 4.];
    let key = IfccadAnnotationContextKey {
        scale_id: IfccadAnnotationScaleId(27),
    };
    value.annotation = Some(IfccadAnnotation {
        mode: IfccadAnnotationMode::Enabled,
        orientation: None,
        reference: Some(key),
        contexts: vec![IfccadAnnotationContext {
            key,
            presentation: Some(presentation),
        }],
    });
    let imported = Ifcx.import_ifccad(&native.drawing).unwrap();
    let returned = Ifcx.export_ifccad(&imported.document).unwrap();
    let value = returned
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::MultiLeader(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(value.presentation.scale_factor, 1.);
    assert_eq!(
        value.annotation.as_ref().unwrap().contexts[0]
            .presentation
            .as_ref()
            .unwrap()
            .scale_factor,
        7.
    );
    assert_eq!(
        value
            .presentation
            .content
            .m_text
            .as_ref()
            .unwrap()
            .state
            .stored_rotation,
        Some(0.375)
    );
    for format in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, format);
        let returned = Ifcx.export_ifccad(&cad).unwrap();
        let value = returned
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::MultiLeader(v) => Some(v),
                _ => None,
            })
            .unwrap();
        assert_eq!(value.presentation.scale_factor, 1., "{format}");
        assert_eq!(
            value.annotation.as_ref().unwrap().contexts[0]
                .presentation
                .as_ref()
                .unwrap()
                .scale_factor,
            7.,
            "{format}"
        );
        assert_eq!(
            value
                .presentation
                .content
                .m_text
                .as_ref()
                .unwrap()
                .state
                .stored_rotation,
            Some(0.375),
            "{format}"
        );
    }
}

#[test]
fn unsupported_snapshot_is_dropped_as_a_loss() {
    let mut document = Document::new();
    let mut value = opencaddocument::entities::MultiLeader::with_text(
        "saved",
        Vector3::ZERO,
        vec![Vector3::ZERO, Vector3::UNIT_X],
    );
    value.common.graphic_data = Some(
        opencaddocument::CheckedProxyGraphics {
            records: vec![
                opencaddocument::CheckedProxyGraphicRecord::Command(
                    opencaddocument::ProxyGraphicCommand::Polyline(vec![
                        Vector3::ZERO,
                        Vector3::UNIT_X,
                    ]),
                ),
                opencaddocument::CheckedProxyGraphicRecord::Unknown {
                    record_type: 999,
                    data: vec![0; 4],
                },
            ],
        }
        .encode_checked()
        .unwrap(),
    );
    document
        .add_entity(EntityType::MultiLeader(Box::new(value)))
        .unwrap();
    // The leader is still written; only its saved picture is dropped,
    // as for a dimension's.
    let outcome = Ifcx.export_ifccad(&document).unwrap();
    assert!(
        outcome
            .losses
            .iter()
            .any(|loss| loss.message.contains("unsupported proxy record 999")),
        "{:?}",
        outcome.losses
    );
}

#[test]
fn owned_tolerance_is_native_but_physical_interpretation_remains_explicit() {
    use ifccad::*;
    let mut document = Document::new();
    document
        .add_entity(EntityType::MultiLeader(Box::new(
            opencaddocument::entities::MultiLeader::with_text(
                "prototype",
                Vector3::ZERO,
                vec![Vector3::ZERO, Vector3::UNIT_X],
            ),
        )))
        .unwrap();
    let mut native = Ifcx.export_ifccad(&document).unwrap().drawing;
    let style = native.dimension_styles[0].id;
    let value = native
        .model
        .entities
        .iter_mut()
        .find_map(|e| match e {
            IfccadEntity::Native(n) => match &mut n.kind {
                IfccadEntityKind::MultiLeader(v) => Some(v),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    let placement = value
        .presentation
        .content
        .m_text
        .take()
        .unwrap()
        .text
        .placement;
    value.settings.common.content_kind = IfccadMultiLeaderContentKind::Tolerance;
    value.presentation.content.tolerance = Some(Box::new(IfccadMultiLeaderToleranceContent {
        placement,
        dimension_style: style,
        overrides: Default::default(),
        notation: IfccadToleranceNotation::CadFeatureControlTextV1 {
            code: "{\\Fgdt;j}%%v0.1%%vA".into(),
        },
    }));
    let encoded = encode_ifccad_document(&native).unwrap();
    load_ifccad_bytes(encoded.bytes(), Default::default()).unwrap();
    assert!(Ifcx
        .import_ifccad(&native)
        .unwrap_err()
        .to_string()
        .contains("unqualified"));
}

#[test]
fn native_picture_does_not_require_binding_unused_cad_defaults() {
    let mut document = Document::new();
    let mut value = opencaddocument::entities::MultiLeader::with_text(
        "renamed layer",
        Vector3::ZERO,
        vec![Vector3::ZERO, Vector3::UNIT_X],
    );
    value.common.graphic_data = Some(
        opencaddocument::CheckedProxyGraphics {
            records: vec![opencaddocument::CheckedProxyGraphicRecord::Command(
                opencaddocument::ProxyGraphicCommand::Polyline(vec![
                    Vector3::ZERO,
                    Vector3::UNIT_X,
                ]),
            )],
        }
        .encode_checked()
        .unwrap(),
    );
    document
        .add_entity(EntityType::MultiLeader(Box::new(value)))
        .unwrap();
    let mut native = Ifcx.export_ifccad(&document).unwrap().drawing;
    for layer in &mut native.layers {
        layer.name = format!("Native layer {}", layer.id);
    }
    let imported = Ifcx.import_ifccad(&native).unwrap();
    assert!(imported
        .document
        .model_space_entities()
        .any(|e| e.common().graphic_data.is_some()));
}

#[test]
fn leader_in_arrow_definition_can_reference_a_later_dimension_style() {
    let mut document = Document::new();
    let mut record = opencaddocument::tables::BlockRecord::new("Leader arrow");
    record.handle = document.allocate_handle();
    record.block_entity_handle = document.allocate_handle();
    record.block_end_handle = document.allocate_handle();
    let owner = record.handle;
    document.block_records.add(record).unwrap();
    let mut style = opencaddocument::tables::DimStyle::new("Zed");
    style.handle = document.allocate_handle();
    document.dim_styles.add(style).unwrap();
    let mut leader = Leader::two_point(Vector3::ZERO, Vector3::UNIT_X)
        .with_creation_type(opencaddocument::entities::LeaderCreationType::NoAnnotation)
        .with_dimension_style("Zed");
    leader.common.owner_handle = owner;
    let handle = document.add_entity(EntityType::Leader(leader)).unwrap();
    document.dim_styles.get_mut("Standard").unwrap().dimblk = owner;
    let native = Ifcx.export_ifccad(&document).unwrap();
    assert!(
        native.entity_ids.contains_key(&handle),
        "block members may not disappear during style registration"
    );
}

#[test]
fn explicit_equal_leader_style_overrides_keep_presence() {
    use opencaddocument::xdata::{ExtendedDataRecord, XDataValue};
    let mut document = Document::new();
    let height = document.dim_styles.get("Standard").unwrap().dimtxt;
    let mut value = Leader::two_point(Vector3::ZERO, Vector3::UNIT_X)
        .with_creation_type(opencaddocument::entities::LeaderCreationType::NoAnnotation);
    let mut record = ExtendedDataRecord::new("ACAD");
    record.values = vec![
        XDataValue::String("DSTYLE".into()),
        XDataValue::ControlString("{".into()),
        XDataValue::Integer16(140),
        XDataValue::Real(height),
        XDataValue::Integer16(341),
        XDataValue::Handle(opencaddocument::Handle::NULL),
        XDataValue::ControlString("}".into()),
    ];
    value.common.extended_data.upsert_record(record);
    document.add_entity(EntityType::Leader(value)).unwrap();
    let native = Ifcx.export_ifccad(&document).unwrap();
    let value = native
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Leader(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(value.overrides.text.height, Some(height));
    assert_eq!(
        value.arrow,
        Some(ifccad::IfccadLeaderArrow::DefaultClosedFilled)
    );
    let imported = Ifcx.import_ifccad(&native.drawing).unwrap();
    let returned = Ifcx.export_ifccad(&imported.document).unwrap();
    let value = returned
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::Leader(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(value.overrides.text.height, Some(height));
    assert_eq!(
        value.arrow,
        Some(ifccad::IfccadLeaderArrow::DefaultClosedFilled)
    );
}

#[test]
fn absent_arrow_is_distinct_from_a_custom_block_named_none() {
    let mut document = Document::new();
    document
        .add_entity(EntityType::MultiLeader(Box::new(
            opencaddocument::entities::MultiLeader::with_text(
                "no arrow",
                Vector3::UNIT_X,
                vec![Vector3::ZERO, Vector3::UNIT_X],
            ),
        )))
        .unwrap();
    let mut native = Ifcx.export_ifccad(&document).unwrap().drawing;
    let value = native
        .model
        .entities
        .iter_mut()
        .find_map(|e| match e {
            IfccadEntity::Native(n) => match &mut n.kind {
                IfccadEntityKind::MultiLeader(v) => Some(v),
                _ => None,
            },
            _ => None,
        })
        .unwrap();
    value.settings.common.line.arrow = ifccad::IfccadLeaderArrow::None;
    let imported = Ifcx.import_ifccad(&native).unwrap();
    let returned = Ifcx.export_ifccad(&imported.document).unwrap();
    let value = returned
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::MultiLeader(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        value.settings.common.line.arrow,
        ifccad::IfccadLeaderArrow::None
    );
    let mut document = Document::new();
    let mut block = opencaddocument::tables::BlockRecord::new("_NONE");
    block.handle = document.allocate_handle();
    block.block_entity_handle = document.allocate_handle();
    block.block_end_handle = document.allocate_handle();
    let handle = block.handle;
    document.block_records.add(block).unwrap();
    let mut line = opencaddocument::entities::Line::from_points(Vector3::ZERO, Vector3::UNIT_X);
    line.common.owner_handle = handle;
    document.add_entity(EntityType::Line(line)).unwrap();
    let mut value = opencaddocument::entities::MultiLeader::with_text(
        "custom",
        Vector3::UNIT_X,
        vec![Vector3::ZERO, Vector3::UNIT_X],
    );
    value.arrowhead_handle = Some(handle);
    document
        .add_entity(EntityType::MultiLeader(Box::new(value)))
        .unwrap();
    let native = Ifcx.export_ifccad(&document).unwrap().drawing;
    let value = native
        .model
        .entities
        .iter()
        .find_map(|e| match &e.as_native()?.kind {
            IfccadEntityKind::MultiLeader(v) => Some(v),
            _ => None,
        })
        .unwrap();
    assert!(matches!(
        value.settings.common.line.arrow,
        ifccad::IfccadLeaderArrow::Symbol { .. }
    ));
}

#[test]
fn classic_leader_keeps_ordinary_tolerance_target() {
    let mut document = Document::new();
    let tolerance = document
        .add_entity(EntityType::Tolerance(Tolerance::with_text(
            Vector3::new(10., 8., 0.),
            "{\\Fgdt;j}%%v0.1%%vA",
        )))
        .unwrap();
    let leader = Leader::two_point(Vector3::new(0., 0., 0.), Vector3::new(10., 8., 0.))
        .with_annotation(tolerance)
        .with_creation_type(opencaddocument::entities::LeaderCreationType::WithTolerance);
    let handle = document.add_entity(EntityType::Leader(leader)).unwrap();
    let exported = Ifcx.export_ifccad(&document).unwrap();
    let id = exported.entity_ids[&handle];
    let entity = exported
        .drawing
        .model
        .entities
        .iter()
        .find(|e| matches!(e, IfccadEntity::Native(n) if n.id == id))
        .unwrap();
    let IfccadEntity::Native(entity) = entity else {
        panic!()
    };
    let IfccadEntityKind::Leader(value) = &entity.kind else {
        panic!("leader must be native")
    };
    assert_eq!(
        value.annotation_entity,
        Some(exported.entity_ids[&tolerance])
    );
    let bytes = ifccad::encode_ifccad_document(&exported.drawing).unwrap();
    let strict = ifccad::load_ifccad_bytes(bytes.bytes(), Default::default())
        .unwrap()
        .into_document();
    let imported = Ifcx.import_ifccad(&strict).unwrap();
    let EntityType::Leader(value) = imported
        .document
        .get_entity(imported.entity_handles[&id])
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(
        value.annotation_handle,
        imported.entity_handles[&exported.entity_ids[&tolerance]]
    );
    for format in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, format);
        let value = cad
            .model_space_entities()
            .find_map(|e| match e {
                EntityType::Leader(v) => Some(v),
                _ => None,
            })
            .unwrap();
        let Some(EntityType::Tolerance(target)) = cad.get_entity(value.annotation_handle) else {
            panic!("{format}: missing ordinary tolerance target")
        };
        assert_eq!(target.text, "{\\Fgdt;j}%%v0.1%%vA", "{format}");
        let native = Ifcx.export_ifccad(&cad).unwrap();
        let bytes = ifccad::encode_ifccad_document(&native.drawing).unwrap();
        ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}

#[test]
fn multileader_keeps_owned_text_and_branch_indices() {
    let mut document = Document::new();
    let mut value = opencaddocument::entities::MultiLeader::with_text(
        "Leader text",
        Vector3::new(10., 8., 0.),
        vec![Vector3::ZERO, Vector3::new(10., 8., 0.)],
    );
    value.context.leader_roots[0].leader_index = 47;
    value.context.leader_roots[0].lines[0].index = 901;
    value.common.graphic_data = Some(
        opencaddocument::CheckedProxyGraphics {
            records: vec![opencaddocument::CheckedProxyGraphicRecord::Command(
                opencaddocument::ProxyGraphicCommand::Circle {
                    center: Vector3::ZERO,
                    radius: 2.,
                    normal: Vector3::UNIT_Z,
                },
            )],
        }
        .encode_checked()
        .unwrap(),
    );
    let handle = document
        .add_entity(EntityType::MultiLeader(Box::new(value)))
        .unwrap();
    let exported = Ifcx.export_ifccad(&document).unwrap();
    let id = exported.entity_ids[&handle];
    let native = exported
        .drawing
        .model
        .entities
        .iter()
        .find_map(|e| e.as_native().filter(|n| n.id == id))
        .unwrap();
    let IfccadEntityKind::MultiLeader(value) = &native.kind else {
        panic!("MLeader must be native")
    };
    assert_eq!(value.presentation.groups[0].source_index, Some(47));
    assert_eq!(
        value.presentation.groups[0].lines[0].source_index,
        Some(901)
    );
    assert!(value.presentation.content.m_text.is_some());
    assert!(value.graphics.is_some());
    let imported = Ifcx.import_ifccad(&exported.drawing).unwrap();
    let EntityType::MultiLeader(value) = imported
        .document
        .get_entity(imported.entity_handles[&id])
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(value.context.text_string, "Leader text");
    assert_eq!(value.context.leader_roots[0].lines[0].index, 901);
    assert!(value.common.graphic_data.is_some());
    for format in ["ascii", "binary", "dwg"] {
        let cad = physical(&imported.document, format);
        let returned = Ifcx.export_ifccad(&cad).unwrap();
        let value = returned
            .drawing
            .model
            .entities
            .iter()
            .find_map(|e| match &e.as_native()?.kind {
                IfccadEntityKind::MultiLeader(v) => Some(v),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            value.presentation.groups[0].lines[0].source_index,
            Some(901),
            "{format}"
        );
        assert_eq!(
            value.graphics.as_ref().unwrap().commands.len(),
            1,
            "{format}"
        );
        let bytes = ifccad::encode_ifccad_document(&returned.drawing).unwrap();
        ifccad::load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    }
}
