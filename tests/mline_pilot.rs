#![cfg(feature = "mline-pilot")]
use opencaddocument::{
    entities::{Line, MLine},
    Document, EntityType, Format, Ifcx, Vector3,
};

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
    assert_eq!(drawing.preservation.as_ref().unwrap().version, 1);
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
    let record = &drawing.preservation.as_ref().unwrap().records[0];
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
        &reopened.document().preservation.as_ref().unwrap().records[0],
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
    let original_record = &original.preservation.as_ref().unwrap().records[0];
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
        drawing.preservation.as_ref().unwrap().records.as_slice(),
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
    bound.preservation.as_mut().unwrap().records[0]
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
    assert_ne!(exported_pattern, pattern_id);
    assert_eq!(
        saved.preservation.as_ref().unwrap().records[0].bindings[0].target,
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
        saved.preservation.as_ref().unwrap().records[0].bindings[0].target,
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
        let record = &mut envelope.records[1];
        if detached {
            record.subject = None;
            drawing.model.entities.remove(1);
        } else {
            record.payload.schema = "foreign.example".into();
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

    let mut source = Document::new();
    source.header.insertion_units = 4;
    let mut layer = Layer::new("B layer");
    layer.handle = source.allocate_handle();
    let layer_handle = layer.handle;
    source.layers.add(layer).unwrap();
    let mut pattern = LineType::new("B appearance");
    pattern.handle = source.allocate_handle();
    source.line_types.add(pattern).unwrap();
    for x in [2.0, 12.0] {
        let mut mline =
            MLine::from_points(&[Vector3::new(x, 3.0, 0.0), Vector3::new(x + 3.0, 3.0, 0.0)]);
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
    let missing = IfccadLinePatternId(drawing.id_counters.next_line_pattern_id);
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
    let bytes = encode_ifccad_document(&drawing).unwrap();
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

    let mut check = |document: &Document, live: bool| {
        let result = Ifcx
            .export_ifccad_preserving(document, &mut context)
            .unwrap();
        let bytes = encode_ifccad_document(&result.drawing).unwrap();
        let file = load_ifccad_bytes(bytes.bytes(), Default::default()).unwrap();
        let saved = file.document();
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
            .any(|pattern| pattern.name == "Unrelated"));
        assert!(saved
            .line_patterns
            .iter()
            .all(|pattern| pattern.id != missing));
        assert!(saved.id_counters.next_line_pattern_id > missing.0);
        let (reopened, context) = Ifcx.import_ifccad_preserving(saved).unwrap();
        assert_eq!(context.blocked_entities().count(), 1);
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
    assert!(second.next_entity_id > first.next_entity_id);
    assert!(second.next_preservation_record_id >= first.next_preservation_record_id);
}
