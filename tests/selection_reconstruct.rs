#![cfg(feature = "selection")]

use std::collections::BTreeMap;

use opencaddocument::selection::*;
use opencaddocument::{Document, EntityType, Handle, Line, Vector3};

fn selection() -> DocumentSelection {
    DocumentSelection::capture(&Document::new(), &CaptureRequest::new("source", vec![]))
        .unwrap()
        .selection
}

fn request(document: &Document) -> ReconstructionRequest {
    ReconstructionRequest {
        model_space: document.header.model_space_block_handle,
        coordinates: CoordinatePolicy::DrawingCoordinates,
        roots: vec![],
        resources: BTreeMap::new(),
        external: vec![],
    }
}

#[test]
fn empty_request_is_independent_noop() {
    let target = Document::new();
    let mut result = selection()
        .reconstruct_available(&target, &request(&target))
        .unwrap();
    assert!(result.roots.is_empty());
    assert!(result.resources.is_empty());
    assert_eq!(result.document.next_handle(), target.next_handle());
    assert_eq!(
        result.document.header.handle_seed,
        target.header.handle_seed
    );
    result.document.header.insertion_units = 4;
    assert_ne!(
        result.document.header.insertion_units,
        target.header.insertion_units
    );
}

fn root(handle: Handle) -> RootReconstruction {
    RootReconstruction {
        root: SelectionRecordKey {
            kind: SelectionRecordKind::Entity,
            handle,
        },
        transform: SelectionPlacement::Identity,
        order: SelectionDrawOrder::Append,
        layer: None,
        linetype: None,
        appearance: RootAppearanceOverrides::default(),
    }
}

#[test]
fn rejects_unknown_root() {
    let target = Document::new();
    let mut input = request(&target);
    input.roots.push(root(Handle::new(800)));
    assert!(matches!(
        selection().reconstruct_available(&target, &input),
        Err(ReconstructionError::InvalidRequest(_))
    ));
}

#[test]
fn rejects_inconsistent_model_space() {
    let mut source = Document::new();
    let line = source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 0.0, 0.0),
        )))
        .unwrap();
    let mut capture = CaptureRequest::new("source", vec![line]);
    capture.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: source.layers.get("0").unwrap().handle,
    });
    let selection = DocumentSelection::capture(&source, &capture)
        .unwrap()
        .selection;
    let mut target = Document::new();
    let mut input = request(&target);
    input.roots.push(root(line));
    target.header.model_space_block_handle = Handle::new(800);
    assert!(matches!(
        selection.reconstruct_available(&target, &input),
        Err(ReconstructionError::UnsafeTarget(_))
    ));
}

fn capture_external_layer(doc: &Document, roots: Vec<Handle>) -> DocumentSelection {
    let mut request = CaptureRequest::new("source", roots);
    request.external.insert(SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: doc.layers.get("0").unwrap().handle,
    });
    DocumentSelection::capture(doc, &request).unwrap().selection
}

fn multiline(doc: &mut Document, x: f64) -> Handle {
    doc.add_entity(EntityType::MLine(
        opencaddocument::entities::MLine::from_points(&[
            Vector3::new(x, 0.0, 0.0),
            Vector3::new(x + 2.0, 0.0, 0.0),
        ]),
    ))
    .unwrap()
}

#[test]
fn mixed_roots_keep_current_layers_and_prune_unused_dependencies() {
    use opencaddocument::objects::ObjectType;
    use opencaddocument::tables::Layer;

    let mut source = Document::new();
    let a = multiline(&mut source, 10.0);
    let b = multiline(&mut source, 20.0);
    let c = multiline(&mut source, 30.0);
    let unused = multiline(&mut source, 40.0);
    let selection = capture_external_layer(&source, vec![a, b, c, unused]);
    let style = SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: source.header.current_multiline_style_handle,
    };
    let mut target = Document::new();
    let mut layer = Layer::new("Current");
    layer.handle = target.allocate_handle();
    let alternate = layer.handle;
    target.layers.add(layer).unwrap();
    let standard = target.header.current_multiline_style_handle;
    let zero = target.layers.get("0").unwrap().handle;
    let mut input = request(&target);
    input.roots = vec![root(a), root(b), root(c)];
    input.roots[0].layer = Some(zero);
    input.roots[1].layer = Some(alternate);
    input.roots[1].transform = SelectionPlacement::Translate(Vector3::new(5.0, 2.0, 0.0));
    input.resources.insert(
        style,
        ResourceAction::Create {
            name: "Restored Walls".into(),
        },
    );
    let out = selection.reconstruct_available(&target, &input).unwrap();
    assert_eq!(out.document.entity_count(), 2);
    assert_eq!(target.entity_count(), 0);
    assert_eq!(out.resources.len(), 1);
    let style_handle = out.resources[&style];
    assert_ne!(style_handle, standard);
    let style_dict = out.document.header.acad_mlinestyle_dict_handle;
    let ObjectType::Dictionary(dict) = &out.document.objects[&style_dict] else {
        panic!()
    };
    assert_eq!(dict.get("Standard"), Some(standard));
    assert_eq!(dict.get("Restored Walls"), Some(style_handle));
    for (result, expected, x) in [
        (&out.roots[0], zero, 10.0),
        (&out.roots[1], alternate, 25.0),
    ] {
        let RootReconstructionStatus::Applied { entity, layer } = result.status else {
            panic!("expected Applied, got {:?}", result.status);
        };
        assert_eq!(layer, expected);
        let EntityType::MLine(v) = out.document.get_entity(entity).unwrap() else {
            panic!()
        };
        assert_eq!(v.vertices[0].position.x, x);
        assert_eq!(v.common.layer_handle, Some(expected));
        assert_eq!(v.style_handle, Some(style_handle));
        assert_eq!(v.style_name, "Restored Walls");
    }
    assert!(
        matches!(&out.roots[2].status, RootReconstructionStatus::Blocked { reasons }
        if reasons[0].kind == ReconstructionBlockerKind::MissingBinding)
    );
}

#[test]
fn unsafe_target_exhaustion_does_not_change_floors() {
    use opencaddocument::HandleReservationError;

    let mut source = Document::new();
    let a = source
        .add_entity(EntityType::Line(Line::from_points(
            Vector3::ZERO,
            Vector3::new(1.0, 0.0, 0.0),
        )))
        .unwrap();
    let selection = capture_external_layer(&source, vec![a]);
    let mut target = Document::new();
    let mut input = request(&target);
    input.roots.push(root(a));
    input.roots[0].layer = Some(target.layers.get("0").unwrap().handle);
    let floor = target.next_handle();
    target.header.handle_seed = u64::MAX;
    assert!(matches!(
        selection.reconstruct_available(&target, &input),
        Err(ReconstructionError::Reservation(
            HandleReservationError::Exhausted
        ))
    ));
    assert_eq!(target.entity_count(), 0);
    assert_eq!(target.next_handle(), floor);
    assert_eq!(target.header.handle_seed, u64::MAX);
}

fn line_in(doc: &mut Document) -> Handle {
    doc.add_entity(EntityType::Line(Line::from_points(
        Vector3::ZERO,
        Vector3::new(1.0, 0.0, 0.0),
    )))
    .unwrap()
}

#[test]
fn reservation_skips_stale_floor_dangling_reference_and_redo_member() {
    use opencaddocument::objects::{Dictionary, ObjectType, XRecord};

    let mut source = Document::new();
    let root_handle = line_in(&mut source);
    let selection = capture_external_layer(&source, vec![root_handle]);
    let mut target = Document::new();
    let old = line_in(&mut target);
    let saved = target.remove_entity_arc(old).unwrap();
    let xrecord_handle = target.allocate_handle();
    let high = Handle::new((1 << 53) + 11);
    let mut xrecord = XRecord::new();
    xrecord.handle = xrecord_handle;
    xrecord.owner = target.header.named_objects_dict_handle;
    xrecord.add_handle(340, high);
    target
        .objects
        .insert(xrecord_handle, ObjectType::XRecord(xrecord));
    target
        .named_objects_dictionary_mut()
        .unwrap()
        .add_entry("Keep pointer", xrecord_handle);
    let floor = target.next_handle();
    let occupied = Handle::new(floor);
    let mut dictionary = Dictionary::new();
    dictionary.handle = occupied;
    dictionary.owner = target.header.named_objects_dict_handle;
    target
        .objects
        .insert(occupied, ObjectType::Dictionary(dictionary));
    target
        .named_objects_dictionary_mut()
        .unwrap()
        .add_entry("Occupied", occupied);

    let mut input = request(&target);
    input.roots.push(root(root_handle));
    input.roots[0].layer = Some(target.layers.get("0").unwrap().handle);
    let mut out = selection.reconstruct_available(&target, &input).unwrap();
    let RootReconstructionStatus::Applied { entity, .. } = out.roots[0].status else {
        panic!()
    };
    assert_eq!(entity, Handle::new(high.value() + 1));
    assert_eq!(out.document.next_handle(), high.value() + 2);
    assert_eq!(target.next_handle(), floor);
    assert!(out.document.get_entity(high).is_none());
    assert!(!out.document.objects.contains_key(&high));
    assert_eq!(out.document.objects[&occupied], target.objects[&occupied]);
    assert_eq!(
        out.document.objects[&xrecord_handle],
        target.objects[&xrecord_handle]
    );
    assert_eq!(out.document.restore_entity_arc(saved), Some(old));
    let new = out.document.remove_entity_arc(entity).unwrap();
    assert_eq!(out.document.restore_entity_arc(new), Some(entity));
    let block = out.document.block_records.get("*Model_Space").unwrap();
    assert_eq!(block.entity_handles, [old, entity]);
    assert_eq!(target.entity_count(), 0);
}

#[test]
fn raw_sort_table_is_not_removed_to_make_reservation_succeed() {
    use opencaddocument::objects::{ObjectType, SortEntitiesTable};
    use opencaddocument::HandleReservationError;

    let mut source = Document::new();
    let h = line_in(&mut source);
    let selection = capture_external_layer(&source, vec![h]);
    let mut target = Document::new();
    let a = line_in(&mut target);
    let model = target.header.model_space_block_handle;
    let ext = target.ensure_extension_dictionary(model);
    let key = target.allocate_handle();
    let mut sort = SortEntitiesTable::for_block(model);
    sort.handle = key;
    sort.owner_handle = ext;
    sort.add_entry(a, Handle::new(1));
    let raw = vec![(331, format!("{:X}", a.value()))];
    sort.raw_dxf_codes = Some(raw.clone());
    target
        .objects
        .insert(key, ObjectType::SortEntitiesTable(sort));
    let Some(ObjectType::Dictionary(dict)) = target.objects.get_mut(&ext) else {
        panic!()
    };
    dict.add_entry(SortEntitiesTable::DICTIONARY_KEY, key);
    let mut input = request(&target);
    let mut add = root(h);
    add.layer = Some(target.layers.get("0").unwrap().handle);
    add.order = SelectionDrawOrder::Before(a);
    input.roots.push(add);
    let floor = target.next_handle();
    assert!(matches!(selection.reconstruct_available(&target, &input),
        Err(ReconstructionError::Reservation(HandleReservationError::UninspectableData { field_path, .. }))
        if field_path == format!("objects[{key}].sort_entities.raw_dxf_codes")));
    let ObjectType::SortEntitiesTable(v) = &target.objects[&key] else {
        panic!()
    };
    assert_eq!(v.raw_dxf_codes.as_ref(), Some(&raw));
    assert_eq!(target.next_handle(), floor);
}

#[test]
fn create_cannot_activate_dangling_style_name() {
    use opencaddocument::entities::MLine;
    let mut source = Document::new();
    let a = multiline(&mut source, 10.0);
    let selection = capture_external_layer(&source, vec![a]);
    let key = SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: source.header.current_multiline_style_handle,
    };
    let mut target = Document::new();
    let mut existing = MLine::from_points(&[Vector3::ZERO, Vector3::new(2.0, 0.0, 0.0)]);
    existing.style_name = "DANGLING".into();
    existing.style_handle = Some(Handle::new(1000));
    target.add_entity(EntityType::MLine(existing)).unwrap();
    let mut input = request(&target);
    let mut add = root(a);
    add.layer = Some(target.layers.get("0").unwrap().handle);
    input.roots.push(add);
    input.resources.insert(
        key,
        ResourceAction::Create {
            name: "dangling".into(),
        },
    );
    let result = selection.reconstruct_available(&target, &input).unwrap();
    assert!(
        matches!(&result.roots[0].status, RootReconstructionStatus::Blocked { reasons }
        if reasons[0].kind == ReconstructionBlockerKind::NameConflict)
    );
    assert!(result.resources.is_empty());
    assert_eq!(result.document.next_handle(), target.next_handle());
}

#[test]
fn style_reuse_compares_mapped_pattern_and_offsets() {
    use opencaddocument::objects::ObjectType;
    use opencaddocument::tables::{LineType, LineTypeElement};

    fn pattern(doc: &mut Document, name: &str) -> Handle {
        let mut value = LineType::new(name);
        value.handle = doc.allocate_handle();
        value.pattern_length = 3.0;
        value.elements = vec![LineTypeElement::dash(2.0), LineTypeElement::space(1.0)];
        let h = value.handle;
        doc.line_types.add(value).unwrap();
        h
    }
    fn set_pattern(doc: &mut Document, name: &str) {
        let style = doc.header.current_multiline_style_handle;
        let Some(ObjectType::MLineStyle(v)) = doc.objects.get_mut(&style) else {
            panic!()
        };
        v.elements[0].linetype = name.into();
    }
    let mut source = Document::new();
    let source_pattern = pattern(&mut source, "Source Dash");
    set_pattern(&mut source, "Source Dash");
    let a = multiline(&mut source, 10.0);
    let selection = capture_external_layer(&source, vec![a]);
    let style = SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: source.header.current_multiline_style_handle,
    };
    let pattern_key = SelectionRecordKey {
        kind: SelectionRecordKind::LineType,
        handle: source_pattern,
    };
    let mut target = Document::new();
    let current_pattern = pattern(&mut target, "Current Dash");
    set_pattern(&mut target, "Current Dash");
    let current_style = target.header.current_multiline_style_handle;
    let mut input = request(&target);
    let mut add = root(a);
    add.layer = Some(target.layers.get("0").unwrap().handle);
    input.roots.push(add);
    input
        .resources
        .insert(pattern_key, ResourceAction::Reuse(current_pattern));
    input
        .resources
        .insert(style, ResourceAction::Reuse(current_style));
    let out = selection.reconstruct_available(&target, &input).unwrap();
    assert!(matches!(
        out.roots[0].status,
        RootReconstructionStatus::Applied { .. }
    ));
    assert_eq!(
        out.resources,
        BTreeMap::from([(style, current_style), (pattern_key, current_pattern)])
    );
    assert_eq!(out.document.next_handle(), target.next_handle() + 1);

    let Some(ObjectType::MLineStyle(v)) = target.objects.get_mut(&current_style) else {
        panic!()
    };
    v.elements[0].offset += 0.1;
    let out = selection.reconstruct_available(&target, &input).unwrap();
    assert!(
        matches!(&out.roots[0].status, RootReconstructionStatus::Blocked { reasons }
        if reasons[0].kind == ReconstructionBlockerKind::IncompatibleResource)
    );
    assert!(out.resources.is_empty());
    assert_eq!(out.document.next_handle(), target.next_handle());
}

#[test]
fn conflicting_create_names_block_all_dependents_in_either_order() {
    use opencaddocument::entities::MLine;
    use opencaddocument::objects::{MLineStyle, ObjectType};

    let mut source = Document::new();
    let a = multiline(&mut source, 10.0);
    let first_style = source.header.current_multiline_style_handle;
    let second_style = source.allocate_handle();
    let dictionary = source.header.acad_mlinestyle_dict_handle;
    let mut other = MLineStyle::standard();
    other.name = "Alternate".into();
    other.handle = second_style;
    other.owner = dictionary;
    source
        .objects
        .insert(second_style, ObjectType::MLineStyle(other));
    let Some(ObjectType::Dictionary(v)) = source.objects.get_mut(&dictionary) else {
        panic!()
    };
    v.add_entry("Alternate", second_style);
    let mut mline = MLine::from_points(&[Vector3::ZERO, Vector3::new(2.0, 0.0, 0.0)]);
    mline.style_name = "Alternate".into();
    mline.style_handle = Some(second_style);
    let b = source.add_entity(EntityType::MLine(mline)).unwrap();
    let selection = capture_external_layer(&source, vec![a, b]);
    let target = Document::new();
    let mut input = request(&target);
    input.roots = vec![root(a), root(b)];
    for r in &mut input.roots {
        r.layer = Some(target.layers.get("0").unwrap().handle);
    }
    for (h, name) in [(first_style, "Claimed"), (second_style, "CLAIMED")] {
        input.resources.insert(
            SelectionRecordKey {
                kind: SelectionRecordKind::Object,
                handle: h,
            },
            ResourceAction::Create { name: name.into() },
        );
    }
    for _ in 0..2 {
        let out = selection.reconstruct_available(&target, &input).unwrap();
        assert!(out.roots.iter().all(
            |r| matches!(&r.status, RootReconstructionStatus::Blocked { reasons }
            if reasons[0].kind == ReconstructionBlockerKind::NameConflict)
        ));
        assert!(out.resources.is_empty());
        assert_eq!(out.document.next_handle(), target.next_handle());
        input.roots.reverse();
    }
}

#[test]
fn creates_layer_and_pattern_without_activating_a_dangling_name() {
    use opencaddocument::tables::{Layer, LineType, LineTypeElement};

    let mut source = Document::new();
    let mut pattern = LineType::new("Source Dash");
    pattern.handle = source.allocate_handle();
    pattern.pattern_length = 3.0;
    pattern.elements = vec![LineTypeElement::dash(2.0), LineTypeElement::space(1.0)];
    let pattern_key = SelectionRecordKey {
        kind: SelectionRecordKind::LineType,
        handle: pattern.handle,
    };
    source.line_types.add(pattern).unwrap();
    let mut layer = Layer::new("Source Walls");
    layer.handle = source.allocate_handle();
    layer.line_type = "Source Dash".into();
    let layer_key = SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: layer.handle,
    };
    source.layers.add(layer).unwrap();
    let a = line_in(&mut source);
    let common = source.get_entity_mut(a).unwrap().common_mut();
    common.layer = "Source Walls".into();
    common.layer_handle = Some(layer_key.handle);
    let selection = DocumentSelection::capture(&source, &CaptureRequest::new("source", vec![a]))
        .unwrap()
        .selection;

    let mut target = Document::new();
    let native = line_in(&mut target);
    let mut input = request(&target);
    input.roots.push(root(a));
    input.resources.insert(
        pattern_key,
        ResourceAction::Create {
            name: "Restored Dash".into(),
        },
    );
    input.resources.insert(
        layer_key,
        ResourceAction::Create {
            name: "Restored Walls".into(),
        },
    );
    let out = selection.reconstruct_available(&target, &input).unwrap();
    assert!(
        matches!(out.roots[0].status, RootReconstructionStatus::Applied { layer, .. }
        if layer == out.resources[&layer_key])
    );
    let created_layer = out.document.layers.get("Restored Walls").unwrap();
    assert_eq!(created_layer.line_type, "Restored Dash");
    assert_eq!(
        out.document.line_types.get("Restored Dash").unwrap().handle,
        out.resources[&pattern_key]
    );
    assert_eq!(out.document.next_handle(), target.next_handle() + 3);

    for (layer, pattern) in [("Restored Walls", "ByLayer"), ("0", "Restored Dash")] {
        let common = target.get_entity_mut(native).unwrap().common_mut();
        common.layer = layer.into();
        common.linetype = pattern.into();
        let blocked = selection.reconstruct_available(&target, &input).unwrap();
        assert!(
            matches!(&blocked.roots[0].status, RootReconstructionStatus::Blocked { reasons }
            if reasons[0].kind == ReconstructionBlockerKind::NameConflict)
        );
        assert!(blocked.resources.is_empty());
        assert_eq!(blocked.document.next_handle(), target.next_handle());
    }
}

#[test]
fn identity_and_translation_preserve_mline_vectors_and_scalar_bits() {
    use opencaddocument::objects::ObjectType;
    let mut source = Document::new();
    let handle = multiline(&mut source, 10.0);
    let Some(EntityType::MLine(v)) = source.get_entity_mut(handle) else {
        panic!()
    };
    v.vertices[0].position.z = -0.0;
    v.vertices[0].miter = Vector3::new(0.0, 2.0, -0.0);
    v.vertices[0].segments[0].parameters = vec![-0.0, 3.0];
    v.common.color_name = Some("Old color".into());
    let selection = capture_external_layer(&source, vec![handle]);
    let target = Document::new();
    let style = SelectionRecordKey {
        kind: SelectionRecordKind::Object,
        handle: source.header.current_multiline_style_handle,
    };
    let mut input = request(&target);
    let mut add = root(handle);
    add.layer = Some(target.layers.get("0").unwrap().handle);
    add.appearance.color = Some(opencaddocument::Color::ByBlock);
    input.roots.push(add);
    input.resources.insert(
        style,
        ResourceAction::Reuse(target.header.current_multiline_style_handle),
    );
    for (transform, x) in [
        (SelectionPlacement::Identity, 10.0),
        (
            SelectionPlacement::Translate(Vector3::new(3.0, 2.0, 0.0)),
            13.0,
        ),
    ] {
        input.roots[0].transform = transform;
        let out = selection.reconstruct_available(&target, &input).unwrap();
        let RootReconstructionStatus::Applied { entity, .. } = out.roots[0].status else {
            panic!("{:?}", out.roots[0].status);
        };
        let EntityType::MLine(v) = out.document.get_entity(entity).unwrap() else {
            panic!()
        };
        let vertex = &v.vertices[0];
        assert_eq!(vertex.position.x, x);
        if matches!(transform, SelectionPlacement::Identity) {
            assert_eq!(vertex.position.z.to_bits(), (-0.0_f64).to_bits());
        }
        assert_eq!(vertex.miter.y, 2.0);
        assert_eq!(vertex.miter.z.to_bits(), (-0.0_f64).to_bits());
        assert_eq!(
            vertex.segments[0].parameters[0].to_bits(),
            (-0.0_f64).to_bits()
        );
        assert_eq!(vertex.segments[0].parameters[1], 3.0);
        assert!(v.common.color_name.is_none());
        assert!(matches!(
            out.document.objects[&out.resources[&style]],
            ObjectType::MLineStyle(_)
        ));
    }
    input.roots[0].transform = SelectionPlacement::Translate(Vector3::new(f64::INFINITY, 0.0, 0.0));
    let out = selection.reconstruct_available(&target, &input).unwrap();
    assert!(
        matches!(&out.roots[0].status, RootReconstructionStatus::Blocked { reasons }
        if reasons[0].kind == ReconstructionBlockerKind::InvalidPlacement)
    );
}

#[test]
fn rejects_created_layer_name_outside_dwg_text_length() {
    use opencaddocument::tables::Layer;
    use opencaddocument::DxfVersion;

    let mut source = Document::new();
    let mut layer = Layer::new("Source walls");
    layer.line_type = "ByLayer".into();
    layer.handle = source.allocate_handle();
    let layer_key = SelectionRecordKey {
        kind: SelectionRecordKind::Layer,
        handle: layer.handle,
    };
    source.layers.add(layer).unwrap();
    let mut line = Line::from_points(Vector3::ZERO, Vector3::new(1.0, 0.0, 0.0));
    line.common.layer = "Source walls".into();
    line.common.layer_handle = Some(layer_key.handle);
    let handle = source.add_entity(EntityType::Line(line)).unwrap();
    let selection =
        DocumentSelection::capture(&source, &CaptureRequest::new("source", vec![handle]))
            .unwrap()
            .selection;

    let names = [
        (DxfVersion::AC1032, "A".repeat(65_536)),
        // An unmappable supplementary character uses two CIF escapes in legacy DWG.
        (DxfVersion::AC1018, "\u{1F9EA}".repeat(3_000)),
    ];
    for (version, name) in names {
        let target = Document::with_version(version);
        let mut input = request(&target);
        input.roots.push(root(handle));
        input
            .resources
            .insert(layer_key, ResourceAction::Create { name });
        let outcome = selection.reconstruct_available(&target, &input).unwrap();
        assert!(
            matches!(
                &outcome.roots[0].status,
                RootReconstructionStatus::Blocked { reasons }
                    if reasons[0].kind == ReconstructionBlockerKind::NameConflict
            ),
            "{version:?} accepted a name that cannot fit in the DWG text field"
        );
        assert!(outcome.resources.is_empty());
        assert_eq!(outcome.document, target);
    }
}
