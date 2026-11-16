#![cfg(feature = "mline-pilot")]
use opencaddocument::{
    entities::{Line, MLine},
    Document, EntityType, Format, Ifcx, Vector3,
};

#[test]
fn paper_mlines_restore_and_retain_on_their_own_layouts() {
    use opencaddocument::ifccad::{
        encode_ifccad_cbor_document, encode_ifccad_document, load_ifccad_bytes,
        load_ifccad_cbor_bytes, IfccadLinePatternId, IfccadPreservationBinding,
        IfccadPreservationTarget,
    };
    use opencaddocument::objects::ObjectType;

    let mut source = Document::new();
    source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 1.0, 0.0),
        )))
        .unwrap();
    source.add_layout("Sheet B").unwrap();
    for (name, x) in [("Layout1", 2.0), ("Sheet B", 12.0)] {
        source
            .add_entity_to_layout(
                EntityType::MLine(MLine::from_points(&[
                    Vector3::new(x, 3.0, 0.0),
                    Vector3::new(x + 5.0, 3.0, 0.0),
                ])),
                name,
            )
            .unwrap();
        source
            .add_entity_to_layout(
                EntityType::Line(Line::from_points(
                    Vector3::new(x, 0.0, 0.0),
                    Vector3::new(x + 1.0, 0.0, 0.0),
                )),
                name,
            )
            .unwrap();
    }
    let mut drawing = Ifcx.export_ifccad(&source).unwrap().drawing;
    let find_sheet = |drawing: &opencaddocument::ifccad::IfccadDocument, name| {
        drawing
            .paper_layouts
            .iter()
            .position(|layout| layout.name == name)
            .unwrap()
    };
    assert_eq!(drawing.paper_layouts.len(), 2);
    let a = &drawing.paper_layouts[find_sheet(&drawing, "Layout1")];
    let b = &drawing.paper_layouts[find_sheet(&drawing, "Sheet B")];
    let (a_layout, b_layout) = (a.id, b.id);
    let a = a.entities[0]
        .as_opaque()
        .expect("Sheet A preserves the MLINE")
        .clone();
    let b_line = b.entities[1].id();
    let b = b.entities[0]
        .as_opaque()
        .expect("Sheet B preserves the MLINE")
        .clone();
    let missing = IfccadLinePatternId(250);
    drawing.id_counters.next_line_pattern_id = 251;
    let envelope = drawing.preservation.as_mut().unwrap();
    let b_record = envelope
        .records
        .iter_mut()
        .find(|r| r.id == b.preservation_record_id)
        .unwrap();
    b_record.bindings.push(IfccadPreservationBinding {
        slot: "root-linetype".into(),
        source_key: "missing-paper-pattern".into(),
        target: IfccadPreservationTarget::LinePattern(missing),
    });
    let b_record = b_record.clone();
    let b_backing = envelope
        .records
        .iter()
        .find(|r| Some(r.id) == b_record.storage_backing)
        .unwrap()
        .clone();
    let bytes = encode_ifccad_document(&drawing).unwrap();
    if let Some(dir) = std::env::var_os("IFCCAD_PAPER_MLINE_REVIEW_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("paper-mline-input.ifccad"), bytes.bytes()).unwrap();
    }
    let input = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(input.document()).unwrap();
    let blocked: Vec<_> = context.blocked_entities().collect();
    assert_eq!(blocked.len(), 1);
    let (id, reason) = blocked[0];
    assert_eq!(id, b.id);
    assert!(reason.contains("Missing required line pattern 250"));
    assert!(!imported.entity_handles.contains_key(&b.id));
    let a_handle = imported.entity_handles[&a.id];
    let owner = imported
        .document
        .get_entity(a_handle)
        .unwrap()
        .common()
        .owner_handle;
    assert!(imported
        .document
        .objects
        .values()
        .any(|object| matches!(object,
            ObjectType::Layout(layout) if layout.name == "Layout1" && layout.block_record == owner
        )));
    let Some(EntityType::Line(line)) = imported
        .document
        .get_entity_mut(imported.entity_handles[&b_line])
    else {
        panic!("missing native paper line");
    };
    line.start.y = 4.0;
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap()
        .drawing;
    let ifcx = encode_ifccad_document(&saved).unwrap();
    let cbor = encode_ifccad_cbor_document(&saved).unwrap();
    let ifcx = load_ifccad_bytes(ifcx.bytes(), Default::default()).unwrap();
    let cbor = load_ifccad_cbor_bytes(cbor.bytes(), Default::default()).unwrap();
    for saved in [ifcx.document(), cbor.document()] {
        let sheet_a = &saved.paper_layouts[find_sheet(saved, "Layout1")];
        let sheet_b = &saved.paper_layouts[find_sheet(saved, "Sheet B")];
        assert_eq!((sheet_a.id, sheet_b.id), (a_layout, b_layout));
        assert_eq!(sheet_a.entities.iter().map(|e| e.id()).next(), Some(a.id));
        assert_eq!(
            sheet_b.entities.iter().map(|e| e.id()).collect::<Vec<_>>(),
            [b.id, b_line]
        );
        assert_eq!(sheet_b.entities[0].as_opaque(), Some(&b));
        let saved_envelope = saved.preservation.as_ref().unwrap();
        assert!(saved_envelope.records.contains(&b_record));
        assert!(saved_envelope.records.contains(&b_backing));
        let (reopened, context) = Ifcx.import_ifccad_preserving(saved).unwrap();
        assert_eq!(
            context
                .blocked_entities()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            [b.id]
        );
        assert!(matches!(
            reopened.document.get_entity(reopened.entity_handles[&a.id]),
            Some(EntityType::MLine(_))
        ));
        assert!(
            matches!(reopened.document.get_entity(reopened.entity_handles[&b_line]), Some(EntityType::Line(line)) if line.start.y == 4.0)
        );
    }
}

#[test]
fn nested_block_mlines_restore_and_retain_in_their_definition() {
    use opencaddocument::entities::Insert;
    use opencaddocument::ifccad::{
        encode_ifccad_cbor_document, encode_ifccad_document, load_ifccad_bytes,
        load_ifccad_cbor_bytes, IfccadLinePatternId, IfccadPreservationBinding,
        IfccadPreservationTarget,
    };
    use opencaddocument::tables::BlockRecord;
    use opencaddocument::Handle;

    let mut source = Document::new();
    source.header.insertion_units = 4;
    let [inner, outer] = ["Inner", "Outer"].map(|name| {
        let mut block = BlockRecord::new(name);
        block.handle = source.allocate_handle();
        block.block_entity_handle = source.allocate_handle();
        block.block_end_handle = source.allocate_handle();
        let owner = block.handle;
        source.block_records.add(block).unwrap();
        owner
    });
    for x in [2.0, 12.0] {
        let mut mline =
            MLine::from_points(&[Vector3::new(x, 3.0, 0.0), Vector3::new(x + 5.0, 3.0, 0.0)]);
        mline.common.owner_handle = inner;
        mline.common.linetype = "ByBlock".into();
        source.add_entity(EntityType::MLine(mline)).unwrap();
    }
    let mut line = Line::from_points(Vector3::ZERO, Vector3::new(1.0, 0.0, 0.0));
    line.common.owner_handle = inner;
    source.add_entity(EntityType::Line(line)).unwrap();
    let mut nested = Insert::new("Inner", Vector3::new(4.0, 5.0, 0.0));
    nested.common.owner_handle = outer;
    source.add_entity(EntityType::Insert(nested)).unwrap();
    for x in [50.0, 60.0] {
        source
            .add_entity(EntityType::Insert(Insert::new(
                "Outer",
                Vector3::new(x, 10.0, 0.0),
            )))
            .unwrap();
    }

    let mut drawing = Ifcx.export_ifccad(&source).unwrap().drawing;
    let block_index = |drawing: &opencaddocument::ifccad::IfccadDocument, name| {
        drawing
            .blocks
            .iter()
            .position(|block| block.name == name)
            .unwrap()
    };
    let inner = &drawing.blocks[block_index(&drawing, "Inner")];
    let inner_id = inner.id;
    let outer_id = drawing.blocks[block_index(&drawing, "Outer")].id;
    let a = inner.entities[0]
        .as_opaque()
        .expect("inner block preserves MLINE A")
        .clone();
    let b = inner.entities[1]
        .as_opaque()
        .expect("inner block preserves MLINE B")
        .clone();
    let line_id = inner.entities[2].id();
    let moved = drawing.model.entities[0].id();
    let stationary = drawing.model.entities[1].id();
    let missing = IfccadLinePatternId(250);
    drawing.id_counters.next_line_pattern_id = 251;
    let envelope = drawing.preservation.as_mut().unwrap();
    let record = envelope
        .records
        .iter_mut()
        .find(|r| r.id == b.preservation_record_id)
        .unwrap();
    record.bindings.push(IfccadPreservationBinding {
        slot: "root-linetype".into(),
        source_key: "missing-block-pattern".into(),
        target: IfccadPreservationTarget::LinePattern(missing),
    });
    let b_record = record.clone();
    let b_backing = envelope
        .records
        .iter()
        .find(|r| Some(r.id) == b_record.storage_backing)
        .unwrap()
        .clone();
    assert_eq!(b_backing.payload.as_ref().unwrap().version, 3);
    assert_eq!(
        envelope
            .records
            .iter()
            .find(|r| r.id == a.preservation_record_id)
            .unwrap()
            .storage_backing,
        Some(b_backing.id)
    );
    let input_bytes = encode_ifccad_document(&drawing).unwrap();
    if let Some(dir) = std::env::var_os("IFCCAD_BLOCK_MLINE_REVIEW_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("block-mline-input.ifccad"), input_bytes.bytes()).unwrap();
    }
    let input = load_ifccad_bytes(input_bytes.bytes(), Default::default()).unwrap();
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(input.document()).unwrap();
    assert!(!imported.entity_handles.contains_key(&b.id));
    assert_eq!(
        context
            .blocked_entities()
            .map(|(id, _)| id)
            .collect::<Vec<_>>(),
        [b.id]
    );
    let edit_insert = |doc: &mut Document, handle: Handle| {
        let Some(EntityType::Insert(insert)) = doc.get_entity_mut(handle) else {
            panic!("missing outer placement");
        };
        insert.insert_point = Vector3::new(35.0, -15.0, 4.0);
        insert.rotation = std::f64::consts::FRAC_PI_3;
    };
    edit_insert(&mut imported.document, imported.entity_handles[&moved]);
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap()
        .drawing;
    let ifcx = encode_ifccad_document(&saved).unwrap();
    let ifcx = load_ifccad_bytes(ifcx.bytes(), Default::default()).unwrap();
    let cbor = encode_ifccad_cbor_document(&saved).unwrap();
    let cbor = load_ifccad_cbor_bytes(cbor.bytes(), Default::default()).unwrap();
    for saved in [ifcx.document(), cbor.document()] {
        let inner = &saved.blocks[block_index(saved, "Inner")];
        let outer = &saved.blocks[block_index(saved, "Outer")];
        assert_eq!((inner.id, outer.id), (inner_id, outer_id));
        assert_eq!(
            inner.entities.iter().map(|e| e.id()).collect::<Vec<_>>(),
            [a.id, b.id, line_id]
        );
        assert_eq!(inner.entities[1].as_opaque(), Some(&b));
        assert!(outer.entities.iter().all(|e| e.as_opaque().is_none()));
        let envelope = saved.preservation.as_ref().unwrap();
        assert!(envelope.records.contains(&b_record));
        assert!(envelope.records.contains(&b_backing));
        let (reopened, context) = Ifcx.import_ifccad_preserving(saved).unwrap();
        assert_eq!(
            context
                .blocked_entities()
                .map(|(id, _)| id)
                .collect::<Vec<_>>(),
            [b.id]
        );
        let Some(EntityType::MLine(line)) =
            reopened.document.get_entity(reopened.entity_handles[&a.id])
        else {
            panic!("missing restored MLINE A");
        };
        assert_eq!(
            line.common.owner_handle,
            reopened.document.block_records.get("Inner").unwrap().handle
        );
        assert_eq!(line.vertices[0].position, Vector3::new(2.0, 3.0, 0.0));
        assert!(line.common.linetype.eq_ignore_ascii_case("ByBlock"));
        for (id, point, rotation) in [
            (
                moved,
                Vector3::new(35.0, -15.0, 4.0),
                std::f64::consts::FRAC_PI_3,
            ),
            (stationary, Vector3::new(60.0, 10.0, 0.0), 0.0),
        ] {
            let Some(EntityType::Insert(insert)) =
                reopened.document.get_entity(reopened.entity_handles[&id])
            else {
                panic!("missing outer placement");
            };
            assert_eq!(insert.block_name, "Outer");
            assert_eq!(insert.insert_point, point);
            assert!((insert.rotation - rotation).abs() < 1e-12);
        }
    }
}

#[test]
fn mline_pilot_roundtrips_existing_ifccad_envelope() {
    let mut source = Document::new();
    source.header.insertion_units = 4;
    source
        .add_entity(EntityType::MLine(MLine::from_points(&[
            Vector3::new(2.0, 3.0, 0.0),
            Vector3::new(5.0, 3.0, 0.0),
        ])))
        .unwrap();
    source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 1.0, 0.0),
        )))
        .unwrap();
    let (bytes, losses) = Ifcx.write(&source).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let file = opencaddocument::ifccad::load_ifccad_bytes(&bytes, Default::default()).unwrap();
    let drawing = file.document();
    assert!(drawing.model.entities[0].as_opaque().is_some());
    assert_eq!(drawing.preservation.as_ref().unwrap().version, 2);
    let out = Ifcx.import_ifccad(drawing).unwrap();
    assert!(out.losses.is_empty(), "{:?}", out.losses);
    assert!(matches!(
        out.document
            .get_entity(out.entity_handles[&drawing.model.entities[0].id()]),
        Some(EntityType::MLine(_))
    ));
    let (reopened, _) = Ifcx.write(&out.document).unwrap();
    let file = opencaddocument::ifccad::load_ifccad_bytes(&reopened, Default::default()).unwrap();
    assert!(file.document().model.entities[0].as_opaque().is_some());
}

fn custom_mline_source() -> Document {
    use opencaddocument::objects::{MLineStyle, MLineStyleElement, ObjectType, Scale};
    use opencaddocument::tables::{LineType, LineTypeElement};
    use opencaddocument::Color;

    let mut source = Document::new();
    source.header.insertion_units = 4;
    // Match the active annotation-scale state Studio writes on an ordinary save.
    let scale_dict = source.ensure_subdictionary("ACAD_SCALELIST");
    let mut scale = Scale::new("1:1", 1.0, 1.0);
    scale.handle = source.allocate_handle();
    scale.owner_handle = scale_dict;
    let Some(ObjectType::Dictionary(dictionary)) = source.objects.get_mut(&scale_dict) else {
        panic!("missing source scale dictionary");
    };
    dictionary.add_entry("*A1", scale.handle);
    source
        .objects
        .insert(scale.handle, ObjectType::Scale(scale));
    assert!(source.set_drawing_variable("CANNOSCALE", "1:1"));
    let mut pattern = LineType::new("Wall dash");
    pattern.handle = source.allocate_handle();
    pattern.add_element(LineTypeElement::dash(0.75));
    pattern.add_element(LineTypeElement::space(0.5));
    pattern.pattern_length = 1.25;
    source.line_types.add(pattern).unwrap();

    let mut style = MLineStyle::new("Wall edges");
    style.handle = source.allocate_handle();
    style.owner = source.header.acad_mlinestyle_dict_handle;
    style.elements = vec![
        MLineStyleElement::full(-0.3, Color::ByLayer, "Wall dash"),
        MLineStyleElement::full(0.4, Color::ByLayer, "Wall dash"),
    ];
    let mut root = MLine::from_points(&[Vector3::ZERO, Vector3::new(4.0, 0.0, 0.0)]);
    root.style_handle = Some(style.handle);
    root.style_name.clone_from(&style.name);
    let Some(ObjectType::Dictionary(dictionary)) = source.objects.get_mut(&style.owner) else {
        panic!("missing source MLINESTYLE dictionary");
    };
    dictionary.add_entry(&style.name, style.handle);
    source
        .objects
        .insert(style.handle, ObjectType::MLineStyle(style));
    source.add_entity(EntityType::MLine(root)).unwrap();
    source
}

#[test]
fn shared_selection_restores_both_live_roots_with_one_style() {
    use opencaddocument::objects::ObjectType;
    let mut source = custom_mline_source();
    let handle = source
        .model_space_entities()
        .next()
        .unwrap()
        .common()
        .handle;
    let Some(EntityType::MLine(mut second)) = source.get_entity(handle).cloned() else {
        panic!("expected the first MLINE");
    };
    second.common.handle = Default::default();
    second.translate(Vector3::new(10.0, 0.0, 0.0));
    source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 1.0, 0.0),
        )))
        .unwrap();
    source.add_entity(EntityType::MLine(second)).unwrap();
    let drawing = Ifcx.export_ifccad(&source).unwrap().drawing;
    let opened = Ifcx.import_ifccad(&drawing).unwrap();
    let style_handles: Vec<_> = opened
        .document
        .model_space_entities()
        .filter_map(|e| {
            if let EntityType::MLine(mline) = e {
                mline.style_handle
            } else {
                None
            }
        })
        .collect();
    assert_eq!(style_handles.len(), 2);
    assert_eq!(style_handles[0], style_handles[1]);
    assert!(
        matches!(opened.document.objects.get(&style_handles[0]), Some(ObjectType::MLineStyle(style)) if style.name == "Wall edges")
    );
    let roundtrip = Ifcx.export_ifccad(&opened.document).unwrap().drawing;
    assert!(roundtrip.model.entities[0].as_opaque().is_some());
    assert!(roundtrip.model.entities[1].as_native().is_some());
    assert!(roundtrip.model.entities[2].as_opaque().is_some());
}

#[test]
fn missing_style_dependency_is_diagnosed_and_retained() {
    use opencaddocument::ifccad::{
        encode_ifccad_document, load_ifccad_bytes, IfccadPreservationDependencyCoverage,
    };

    let mut source = custom_mline_source();
    source.line_types.remove("Wall dash").unwrap();
    let handle = source
        .model_space_entities()
        .next()
        .unwrap()
        .common()
        .handle;
    let (bytes, losses) = Ifcx.write(&source).unwrap();
    assert!(
        losses.iter().any(|loss| {
            loss.handle == Some(handle)
                && loss.message.contains("StyleElementLineType")
                && loss.message.contains("dependency is missing")
        }),
        "{losses:?}"
    );

    let file = load_ifccad_bytes(&bytes, Default::default()).unwrap();
    let drawing = file.document();
    let record = &drawing.preservation.as_ref().unwrap().records[1];
    assert_eq!(
        record.dependency_coverage,
        IfccadPreservationDependencyCoverage::Unknown
    );
    let opaque_id = drawing.model.entities[0].id();
    let (opened, mut context) = Ifcx.import_ifccad_preserving(drawing).unwrap();
    assert!(context
        .blocked_entities()
        .any(|(id, reason)| { id == opaque_id && reason.contains("unresolved capture gap") }));
    assert!(!opened.entity_handles.contains_key(&opaque_id));

    let saved = Ifcx
        .export_ifccad_preserving(&opened.document, &mut context)
        .unwrap();
    let bytes = encode_ifccad_document(&saved.drawing).unwrap();
    let reopened = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    assert_eq!(
        &reopened.document().preservation.as_ref().unwrap().records[1],
        record
    );
    let (_, context) = Ifcx.import_ifccad_preserving(reopened.document()).unwrap();
    assert_eq!(context.blocked_entities().next().unwrap().0, opaque_id);
}

#[test]
fn custom_mline_style_and_pattern_reopen_from_snapshot() {
    use opencaddocument::ifccad::{encode_ifccad_document, load_ifccad_bytes};
    use opencaddocument::objects::ObjectType;

    let mut source = custom_mline_source();
    source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 1.0, 0.0),
        )))
        .unwrap();

    let exported = Ifcx.export_ifccad(&source).unwrap();
    assert!(exported.losses.is_empty(), "{:?}", exported.losses);
    let mut drawing = exported.drawing;
    drawing
        .line_patterns
        .retain(|pattern| pattern.name != "Wall dash");
    let bytes = encode_ifccad_document(&drawing).unwrap();
    let file = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    assert!(file
        .document()
        .line_patterns
        .iter()
        .all(|pattern| pattern.name != "Wall dash"));

    let check = |document: &Document| {
        let root = document
            .entities()
            .find_map(|entity| match entity {
                EntityType::MLine(root) => Some(root),
                _ => None,
            })
            .expect("restored MLINE");
        let handle = root.style_handle.unwrap();
        let Some(ObjectType::MLineStyle(style)) = document.objects.get(&handle) else {
            panic!("missing restored MLINESTYLE");
        };
        assert_eq!(style.name, "Wall edges");
        assert_eq!(root.style_name, style.name);
        let Some(ObjectType::Dictionary(dictionary)) = document.objects.get(&style.owner) else {
            panic!("missing restored MLINESTYLE dictionary");
        };
        assert_eq!(dictionary.get(&style.name), Some(handle));
        assert_eq!(style.elements.len(), 2);
        assert_eq!(style.elements[0].offset, -0.3);
        assert_eq!(style.elements[1].offset, 0.4);
        assert_eq!(style.elements[0].linetype, style.elements[1].linetype);
        let pattern = document
            .line_types
            .get(&style.elements[0].linetype)
            .unwrap();
        assert_eq!(pattern.pattern_length, 1.25);
        assert_eq!(
            pattern
                .elements
                .iter()
                .map(|element| element.length)
                .collect::<Vec<_>>(),
            [0.75, -0.5]
        );
        assert_eq!(
            document
                .line_types
                .iter()
                .filter(|entry| entry.elements == pattern.elements)
                .count(),
            1
        );
    };
    let imported = Ifcx.import_ifccad(file.document()).unwrap();
    assert!(imported.losses.is_empty(), "{:?}", imported.losses);
    check(&imported.document);
    let (saved, losses) = Ifcx.write(&imported.document).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let (reopened, losses) = Ifcx.read(&saved).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    check(&reopened);
}

#[test]
fn custom_mline_blocked_by_circle_survives_edit_and_reopen() {
    use opencaddocument::entities::Circle;
    use opencaddocument::ifccad::{
        encode_ifccad_document, load_ifccad_bytes, IfccadPreservationBinding,
        IfccadPreservationTarget,
    };

    let mut source = custom_mline_source();
    let center = Vector3::new(2.0, 3.0, 0.0);
    source
        .add_entity(EntityType::Circle(Circle::from_center_radius(center, 2.0)))
        .unwrap();
    let (bytes, losses) = Ifcx.write(&source).unwrap();
    assert!(losses.is_empty(), "{losses:?}");
    let file = load_ifccad_bytes(&bytes, Default::default()).unwrap();
    let original = file.document();
    let opaque = original.model.entities[0].as_opaque().unwrap();
    let circle_id = original.model.entities[1].id();
    let original_record = &original.preservation.as_ref().unwrap().records[1];
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(original).unwrap();
    let blocked: Vec<_> = context.blocked_entities().collect();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].0, opaque.id);
    assert!(blocked[0].1.contains("qualified name inventory"));
    assert!(!imported.entity_handles.contains_key(&opaque.id));
    assert!(
        Ifcx.read(&bytes).is_err(),
        "context-free open would lose the root"
    );
    let circle_handle = imported.entity_handles[&circle_id];
    let Some(EntityType::Circle(circle)) = imported.document.get_entity_mut(circle_handle) else {
        panic!("missing editable native circle");
    };
    circle.radius = 4.0;

    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap();
    assert!(saved.losses.is_empty(), "{:?}", saved.losses);
    let bytes = encode_ifccad_document(&saved.drawing).unwrap();
    let file = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let drawing = file.document();
    assert_eq!(drawing.model.entities[0].id(), opaque.id);
    assert_eq!(
        &drawing.preservation.as_ref().unwrap().records[1..],
        std::slice::from_ref(original_record)
    );
    let (reopened, context) = Ifcx.import_ifccad_preserving(drawing).unwrap();
    assert_eq!(context.blocked_entities().count(), 1);
    assert!(matches!(
        reopened.document.get_entity(reopened.entity_handles[&drawing.model.entities[1].id()]),
        Some(EntityType::Circle(circle)) if circle.center == center && circle.radius == 4.0
    ));

    // A root that blocks in D3 can still have a live soft binding. It should
    // follow the resource while present and stay missing after deletion.
    let mut bound = original.clone();
    let pattern_id = bound
        .line_patterns
        .iter()
        .find(|p| p.name == "Wall dash")
        .unwrap()
        .id;
    bound.preservation.as_mut().unwrap().records[1]
        .bindings
        .push(IfccadPreservationBinding {
            slot: "root-linetype".into(),
            source_key: "source-pattern".into(),
            target: IfccadPreservationTarget::LinePattern(pattern_id),
        });
    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(&bound).unwrap();
    assert_eq!(context.blocked_entities().count(), 1);
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap()
        .drawing;
    let exported_pattern = saved
        .line_patterns
        .iter()
        .find(|p| p.name == "Wall dash")
        .unwrap()
        .id;
    assert_eq!(exported_pattern, pattern_id);
    assert_eq!(
        saved.preservation.as_ref().unwrap().records[1].bindings[0].target,
        IfccadPreservationTarget::LinePattern(exported_pattern)
    );

    let mut replacement = imported.document.line_types.remove("Wall dash").unwrap();
    replacement.handle = imported.document.allocate_handle();
    imported.document.line_types.add(replacement).unwrap();
    let saved = Ifcx
        .export_ifccad_preserving(&imported.document, &mut context)
        .unwrap()
        .drawing;
    assert!(saved.line_patterns.iter().all(|p| p.id != pattern_id));
    assert_eq!(
        saved.preservation.as_ref().unwrap().records[1].bindings[0].target,
        IfccadPreservationTarget::LinePattern(pattern_id)
    );
}

#[test]
fn rejects_other_preservation_before_an_edit_can_drop_it() {
    use opencaddocument::ifccad::{encode_ifccad_document, load_ifccad_bytes};

    let mut source = custom_mline_source();
    source
        .add_entity(EntityType::MLine(MLine::from_points(&[
            Vector3::new(12.0, 3.0, 0.0),
            Vector3::new(15.0, 3.0, 0.0),
        ])))
        .unwrap();
    let original = Ifcx.export_ifccad(&source).unwrap().drawing;
    for detached in [false, true] {
        let mut drawing = original.clone();
        let envelope = drawing.preservation.as_mut().unwrap();
        let inline = envelope.records[0].payload.clone();
        let record = &mut envelope.records[1];
        if detached {
            record.subject = None;
            record.storage_backing = None;
            record.payload = inline;
            drawing.model.entities.remove(0);
        } else {
            envelope.records[0].payload.as_mut().unwrap().schema = "foreign.example".into();
        }
        let bytes = encode_ifccad_document(&drawing).unwrap();
        let file = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
        let Err(err) = Ifcx.import_ifccad_preserving(file.document()) else {
            panic!("editable import accepted an unsupported preservation record");
        };
        assert!(
            err.to_string().contains("unsupported preservation"),
            "{err}"
        );
        assert!(Ifcx.read(bytes.bytes()).is_err());
    }
}

#[test]
fn blocked_mline_survives_live_edit_and_delete() {
    use opencaddocument::ifccad::{
        encode_ifccad_document, load_ifccad_bytes, IfccadLinePatternId, IfccadMode,
        IfccadPreservationBinding, IfccadPreservationTarget,
    };
    use opencaddocument::tables::{Layer, LineType};

    let mut source = custom_mline_source();
    let handle = source
        .model_space_entities()
        .next()
        .unwrap()
        .common()
        .handle;
    let Some(EntityType::MLine(template)) = source.get_entity(handle).cloned() else {
        panic!("missing template")
    };
    source.remove_entity(handle).unwrap();
    let mut layer = Layer::new("B layer");
    layer.handle = source.allocate_handle();
    let layer_handle = layer.handle;
    source.layers.add(layer).unwrap();
    let mut pattern = LineType::new("B appearance");
    pattern.handle = source.allocate_handle();
    source.line_types.add(pattern).unwrap();
    for x in [2.0, 12.0] {
        let mut mline = template.clone();
        mline.common.handle = Default::default();
        mline.translate(Vector3::new(x, 3.0, 0.0));
        if x == 12.0 {
            mline.common.layer = "B layer".into();
            mline.common.layer_handle = Some(layer_handle);
        }
        source.add_entity(EntityType::MLine(mline)).unwrap();
    }
    source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 1.0, 0.0),
        )))
        .unwrap();
    let mut drawing = Ifcx.export_ifccad(&source).unwrap().drawing;
    let a = drawing.model.entities[0].id();
    let pattern = drawing
        .line_patterns
        .iter()
        .find(|p| p.name == "B appearance")
        .unwrap()
        .id;
    let opencaddocument::ifccad::IfccadEntity::Opaque(b) = &mut drawing.model.entities[1] else {
        panic!("missing opaque B");
    };
    b.appearance.as_mut().unwrap().appearance.line_pattern = IfccadMode::Explicit(pattern);
    let b = b.clone();
    let missing = IfccadLinePatternId(250);
    drawing.id_counters.next_line_pattern_id = 251;
    let record = drawing
        .preservation
        .as_mut()
        .unwrap()
        .records
        .iter_mut()
        .find(|record| record.id == b.preservation_record_id)
        .unwrap();
    record.bindings.push(IfccadPreservationBinding {
        slot: "root-linetype".into(),
        source_key: "original-root-linetype".into(),
        target: IfccadPreservationTarget::LinePattern(missing),
    });
    let original = record.clone();
    let original_backing = drawing
        .preservation
        .as_ref()
        .unwrap()
        .records
        .iter()
        .find(|record| Some(record.id) == original.storage_backing)
        .unwrap()
        .clone();
    assert_eq!(drawing.preservation.as_ref().unwrap().records.len(), 3);
    let captured = opencaddocument::selection_storage::decode(
        &original_backing.payload.as_ref().unwrap().bytes,
    )
    .unwrap();
    assert_eq!(captured.roots().len(), 2);
    drop(source);
    let bytes = encode_ifccad_document(&drawing).unwrap();
    let cbor_input = opencaddocument::ifccad::encode_ifccad_cbor_document(&drawing).unwrap();
    let review = std::env::var_os("IFCCAD_MLINE_REVIEW_DIR").map(std::path::PathBuf::from);
    if let Some(dir) = &review {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("mline-shared-input.ifcx"), bytes.bytes()).unwrap();
        std::fs::write(
            dir.join("mline-shared-input.ifccad.cbor"),
            cbor_input.bytes(),
        )
        .unwrap();
    }
    let file = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
    let drawing = file.document();
    assert!(Ifcx.import_ifccad(drawing).is_err());

    let (mut imported, mut context) = Ifcx.import_ifccad_preserving(drawing).unwrap();
    let handle = imported.entity_handles[&a];
    assert!(!imported.entity_handles.contains_key(&b.id));
    let blocked: Vec<_> = context.blocked_entities().collect();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].0, b.id);
    assert!(blocked[0].1.contains("Missing required line pattern"));
    let Some(EntityType::MLine(mline)) = imported.document.get_entity_mut(handle) else {
        panic!("missing live MLINE");
    };
    mline.translate(Vector3::new(10.0, 4.0, 0.0));
    let mut unrelated = LineType::new("Unrelated");
    unrelated.handle = imported.document.allocate_handle();
    imported.document.line_types.add(unrelated).unwrap();

    let native_handle = imported.entity_handles[&drawing.model.entities[2].id()];
    let Some(EntityType::Line(line)) = imported.document.get_entity_mut(native_handle) else {
        panic!("missing native line")
    };
    line.end = Vector3::new(9.0, 8.0, 0.0);
    let mut check = |document: &Document, live: bool| {
        let result = Ifcx
            .export_ifccad_preserving(document, &mut context)
            .unwrap();
        let bytes = encode_ifccad_document(&result.drawing).unwrap();
        let file = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
        let saved = file.document();
        let cbor = opencaddocument::ifccad::encode_ifccad_cbor_document(saved).unwrap();
        if let Some(dir) = review.as_ref().filter(|_| !live) {
            std::fs::write(dir.join("mline-shared-after-edit.ifcx"), bytes.bytes()).unwrap();
            std::fs::write(
                dir.join("mline-shared-after-edit.ifccad.cbor"),
                cbor.bytes(),
            )
            .unwrap();
        }
        let cbor_read =
            opencaddocument::ifccad::load_ifccad_cbor_bytes(cbor.bytes(), Default::default())
                .unwrap();
        assert_eq!(cbor_read.document(), saved);
        let records = &saved.preservation.as_ref().unwrap().records;
        assert_eq!(
            records.iter().find(|r| r.id == original_backing.id),
            Some(&original_backing)
        );
        assert_eq!(
            records.iter().filter(|r| r.payload.is_some()).count(),
            if live { 2 } else { 1 }
        );
        assert_eq!(saved.model.entities.len(), if live { 3 } else { 2 });
        let preserved = saved.model.entities[usize::from(live)].as_opaque().unwrap();
        assert_eq!(preserved.id, b.id);
        assert_eq!(preserved.preservation_record_id, b.preservation_record_id);
        assert!(saved.model.entities.last().unwrap().as_native().is_some());
        assert!(saved
            .layers
            .iter()
            .any(|layer| Some(layer.id) == preserved.layer_id && layer.name == "B layer"));
        let appearance = &preserved.appearance.as_ref().unwrap().appearance;
        assert!(saved
            .line_patterns
            .iter()
            .any(
                |pattern| appearance.line_pattern == IfccadMode::Explicit(pattern.id)
                    && pattern.name == "B appearance"
            ));
        assert_eq!(
            saved
                .preservation
                .as_ref()
                .unwrap()
                .records
                .iter()
                .find(|record| record.id == b.preservation_record_id),
            Some(&original)
        );
        assert!(saved
            .line_patterns
            .iter()
            .any(|pattern| pattern.name == "Unrelated" && pattern.id.0 > missing.0));
        assert!(saved
            .line_patterns
            .iter()
            .all(|pattern| pattern.id != missing));
        assert!(saved.id_counters.next_line_pattern_id > missing.0);
        let (reopened, context) = Ifcx.import_ifccad_preserving(cbor_read.document()).unwrap();
        assert_eq!(context.blocked_entities().count(), 1);
        let native = reopened.entity_handles[&drawing.model.entities[2].id()];
        assert!(
            matches!(reopened.document.get_entity(native), Some(EntityType::Line(line)) if line.end == Vector3::new(9.0, 8.0, 0.0))
        );
        let points: Vec<_> = reopened
            .document
            .entities()
            .filter_map(|entity| match entity {
                EntityType::MLine(mline) => Some(mline.vertices[0].position),
                _ => None,
            })
            .collect();
        assert_eq!(
            points,
            if live {
                vec![Vector3::new(12.0, 7.0, 0.0)]
            } else {
                vec![]
            }
        );
        saved.id_counters
    };
    let first = check(&imported.document, true);
    imported.document.remove_entity(handle).unwrap();
    let second = check(&imported.document, false);
    assert!(second.next_entity_id >= first.next_entity_id);
    assert!(second.next_preservation_record_id >= first.next_preservation_record_id);
}
